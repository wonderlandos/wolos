// SPDX-License-Identifier: BSD-2-Clause
//! [`WebView`]: one web app's page, made from its [`Policy`] and painted into a wolkit surface node
//! or headless into memory. One per process at a time: the engine allows one instance per process
//! at a time, which is the process model anyway (one app per host process, [`crate::process`]).
use crate::ffi::{self, EngineH, Lib, ViewH};
use crate::input::{Bridged, Input};
use crate::policy::Policy;
use crate::sandbox::{Gate, Load};
use std::cell::RefCell;
use std::ffi::{CString, c_char, c_void};
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `libwolwebengine.so.1` is missing or of another ABI.
    Library(String),
    /// The policy or the app cannot be used as given.
    Policy(String),
    /// The engine refused (its message).
    Engine(String),
    Timeout(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Library(m) => write!(f, "web engine library: {m}"),
            Error::Policy(m) => write!(f, "web app policy: {m}"),
            Error::Engine(m) => write!(f, "web engine: {m}"),
            Error::Timeout(what) => write!(f, "web engine: {what} timed out"),
        }
    }
}

impl std::error::Error for Error {}

/// The system clipboard as the host has it (wolkit's, once it offers one to apps).
pub trait Clipboard {
    fn get(&self) -> Option<String>;
    fn set(&self, text: &str);
}

/// Who the page belongs to.
pub struct App {
    /// `org.wolos.webapp.<slug>` ([`crate::process::app_id`]).
    pub id: String,
    /// The engine's only place for files: storage, cookies, cache (absolute). The host makes it
    /// (`wol_core::webapps::profile_dir`).
    pub profile_dir: PathBuf,
    /// The page in its own content process (the default), or in this process (goldens and
    /// probes, where a child process adds nothing).
    pub content_process: bool,
    pub clipboard: Option<Box<dyn Clipboard>>,
}

impl App {
    pub fn new(id: &str, profile_dir: PathBuf) -> App {
        App {
            id: id.to_owned(),
            profile_dir,
            content_process: true,
            clipboard: None,
        }
    }
}

/// Where the view paints.
pub enum Target {
    /// Into memory, read back with [`WebView::screenshot`].
    Headless { width: u32, height: u32 },
    /// Into a wolkit `surface` node's Wayland subsurface.
    Surface {
        display: *mut c_void,
        surface: *mut c_void,
        width: u32,
        height: u32,
        scale: f32,
    },
}

impl Target {
    /// The subsurface wolkit made for a `surface` node (`Engine::create_subsurface`).
    ///
    /// # Safety
    /// The subsurface (and its window) must outlive the [`WebView`] made with this target: drop
    /// the view before `Engine::destroy_subsurface`.
    pub unsafe fn surface(sub: &wolkit::window::Subsurface) -> Target {
        Target::Surface {
            display: sub.display,
            surface: sub.surface,
            width: sub.width,
            height: sub.height,
            scale: sub.scale,
        }
    }
}

/// A screenshot: `width * height` RGBA pixels, rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// What the engine's callbacks reach. Boxed, so its address is stable while the engine lives.
struct Shared {
    gate: Gate,
    refused: RefCell<Vec<String>>,
    refused_loads: std::cell::Cell<u64>,
    clipboard: Option<Box<dyn Clipboard>>,
    /// The only field the waker touches (it runs on the engine's threads).
    woken: AtomicBool,
}

// SAFETY: called by the engine on the view's thread with the `Shared` this view boxed as `ctx`.
unsafe extern "C" fn allow_cb(ctx: *mut c_void, url: *const u8, len: usize, kind: u32) -> bool {
    // SAFETY: ctx is the Shared this view boxed; the engine calls allow on the view's thread.
    let s = unsafe { &*ctx.cast::<Shared>() };
    // SAFETY: the engine passes len valid bytes.
    let bytes = unsafe { ffi::bytes_in(url, len) };
    let Ok(url) = std::str::from_utf8(bytes) else {
        return false;
    };
    let load = if kind == ffi::LOAD_NAVIGATION {
        Load::Navigation
    } else {
        Load::Resource
    };
    let ok = s.gate.allows(url, load);
    if !ok {
        match load {
            Load::Navigation => s.refused.borrow_mut().push(url.to_owned()),
            Load::Resource => s.refused_loads.set(s.refused_loads.get() + 1),
        }
    }
    ok
}

// SAFETY: called from any engine thread; touches only the atomic field of `ctx`.
unsafe extern "C" fn wake_cb(ctx: *mut c_void) {
    // SAFETY: only the atomic field is touched, through a raw place (any thread).
    let woken = unsafe { &(*ctx.cast::<Shared>()).woken };
    woken.store(true, Ordering::Release);
}

// SAFETY: as allow_cb; `buf` has `cap` writable bytes.
unsafe extern "C" fn clip_get_cb(ctx: *mut c_void, buf: *mut u8, cap: usize) -> isize {
    // SAFETY: as allow_cb.
    let s = unsafe { &*ctx.cast::<Shared>() };
    let Some(text) = s.clipboard.as_ref().and_then(|c| c.get()) else {
        return -1;
    };
    let n = text.len().min(cap);
    // SAFETY: the engine gave cap writable bytes.
    unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), buf, n) };
    isize::try_from(text.len()).unwrap_or(-1)
}

// SAFETY: as allow_cb; `text` has `len` readable bytes.
unsafe extern "C" fn clip_set_cb(ctx: *mut c_void, text: *const u8, len: usize) {
    // SAFETY: as allow_cb; the engine passes len valid bytes.
    let (s, bytes) = unsafe { (&*ctx.cast::<Shared>(), ffi::bytes_in(text, len)) };
    if let (Some(c), Ok(t)) = (s.clipboard.as_ref(), std::str::from_utf8(bytes)) {
        c.set(t);
    }
}

pub struct WebView {
    lib: &'static Lib,
    engine: *mut EngineH,
    view: *mut ViewH,
    throttle: bool,
    // Last: the engine is freed (in Drop) before what its callbacks point at.
    shared: Box<Shared>,
}

const ERR: usize = 512;

fn take_err(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c.to_ne_bytes()[0])
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The engine flags of an app: a content process is always a confined one ([`ffi::SANDBOX`]; the
/// library refuses the one without the other, and confines the process itself before the engine
/// runs it, `src/engine/confine.rs`), and an app without a content process has nothing to confine.
pub(crate) fn engine_flags(content_process: bool) -> u32 {
    if content_process {
        ffi::MULTIPROCESS | ffi::SANDBOX
    } else {
        0
    }
}

impl WebView {
    /// The app's page at `url`, under `policy`. The start URL must pass the policy's gate.
    pub fn new(policy: &Policy, app: App, target: Target, url: &str) -> Result<WebView, Error> {
        if !app.profile_dir.is_absolute() {
            return Err(Error::Policy(
                "the profile directory must be absolute".into(),
            ));
        }
        let gate = Gate::new(policy, &app.profile_dir).map_err(Error::Policy)?;
        if !gate.allows(url, Load::Navigation) {
            return Err(Error::Policy(format!("{url} is outside the app's origins")));
        }
        std::fs::create_dir_all(&app.profile_dir)
            .map_err(|e| Error::Policy(format!("{}: {e}", app.profile_dir.display())))?;
        let lib = ffi::lib().map_err(Error::Library)?;
        let has_clip = app.clipboard.is_some();
        let shared = Box::new(Shared {
            gate,
            refused: RefCell::new(Vec::new()),
            refused_loads: std::cell::Cell::new(0),
            clipboard: app.clipboard,
            woken: AtomicBool::new(false),
        });
        let profile = CString::new(app.profile_dir.to_string_lossy().into_owned())
            .map_err(|_| Error::Policy("the profile directory has a NUL".into()))?;
        let curl = CString::new(url).map_err(|_| Error::Policy("the URL has a NUL".into()))?;
        let flags = engine_flags(app.content_process);
        let ctx = (&raw const *shared).cast_mut().cast::<c_void>();
        let config = ffi::Config {
            abi: ffi::ABI,
            flags,
            features: policy.feature_bits(),
            heap_mb: policy.heap_mb.unwrap_or(0),
            profile_dir: profile.as_ptr(),
            user_agent: std::ptr::null(),
            allow: Some(allow_cb),
            wake: Some(wake_cb),
            clip_get: if has_clip { Some(clip_get_cb) } else { None },
            clip_set: if has_clip { Some(clip_set_cb) } else { None },
            ctx,
        };
        let mut err: [c_char; ERR] = [0; ERR];
        // SAFETY: config and its strings are valid for the call; ctx outlives the engine (Drop).
        let engine = unsafe { (lib.engine_new)(&config, err.as_mut_ptr(), ERR) };
        if engine.is_null() {
            return Err(Error::Engine(take_err(&err)));
        }
        let t = match target {
            Target::Headless { width, height } => ffi::TargetC {
                kind: ffi::TARGET_HEADLESS,
                width,
                height,
                scale: 1.0,
                wl_display: std::ptr::null_mut(),
                wl_surface: std::ptr::null_mut(),
            },
            Target::Surface {
                display,
                surface,
                width,
                height,
                scale,
            } => ffi::TargetC {
                kind: ffi::TARGET_WAYLAND,
                width,
                height,
                scale,
                wl_display: display,
                wl_surface: surface,
            },
        };
        // SAFETY: engine is live; t and curl are valid for the call.
        let view = unsafe { (lib.view_new)(engine, &t, curl.as_ptr(), err.as_mut_ptr(), ERR) };
        if view.is_null() {
            // SAFETY: engine has no view. Freeing it gives the library's one-engine slot back,
            // so a later WebView::new in this process can make one again.
            unsafe { (lib.engine_free)(engine) };
            return Err(Error::Engine(take_err(&err)));
        }
        Ok(WebView {
            lib,
            engine,
            view,
            throttle: policy.background_throttle,
            shared,
        })
    }

    /// Runs what the engine has pending; call when [`WebView::take_woken`] is true or on the
    /// window's frame tick.
    pub fn spin(&self) {
        // SAFETY: engine is live (self).
        unsafe { (self.lib.engine_spin)(self.engine) };
    }

    /// Whether the engine asked to be spun since the last call.
    pub fn take_woken(&self) -> bool {
        self.shared.woken.swap(false, Ordering::AcqRel)
    }

    pub fn state(&self) -> ffi::State {
        let mut s = ffi::State::default();
        // SAFETY: view is live; s is writable.
        unsafe { (self.lib.view_state)(self.view, &mut s) };
        s
    }

    /// Spins the engine until `done` holds (true) or `timeout` passes (false).
    pub fn spin_until(&self, timeout: Duration, mut done: impl FnMut(&WebView) -> bool) -> bool {
        let end = Instant::now() + timeout;
        while !done(self) {
            if Instant::now() > end {
                return false;
            }
            self.spin();
            std::thread::sleep(Duration::from_millis(1));
        }
        true
    }

    /// Spins until the page and its subresources have loaded, or its content crashed (an error at
    /// once: the page will not load).
    pub fn wait_loaded(&self, timeout: Duration) -> Result<(), Error> {
        let mut outcome = None;
        self.spin_until(timeout, |v| {
            outcome = load_outcome(&v.state());
            outcome.is_some()
        });
        outcome.unwrap_or(Err(Error::Timeout("page load")))
    }

    /// The page as painted, once rendering is up to date.
    pub fn screenshot(&self, timeout: Duration) -> Result<Image, Error> {
        let mut img = ffi::ImageC {
            width: 0,
            height: 0,
            rgba: std::ptr::null_mut(),
        };
        let mut err: [c_char; ERR] = [0; ERR];
        let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: view is live; img and err are writable.
        let rc =
            unsafe { (self.lib.view_screenshot)(self.view, ms, &mut img, err.as_mut_ptr(), ERR) };
        if rc != 0 || img.rgba.is_null() {
            return Err(Error::Engine(take_err(&err)));
        }
        let px = |v: u32| usize::try_from(v).unwrap_or(0);
        let n = px(img.width) * px(img.height) * 4;
        // SAFETY: the library filled n bytes (header); copied before it is freed.
        let rgba = unsafe { ffi::bytes_in(img.rgba, n) }.to_vec();
        // SAFETY: filled by view_screenshot above, freed once.
        unsafe { (self.lib.image_free)(&mut img) };
        Ok(Image {
            width: img.width,
            height: img.height,
            rgba,
        })
    }

    pub fn resize(&self, width: u32, height: u32, scale: f32) {
        // SAFETY: view is live.
        unsafe { (self.lib.view_resize)(self.view, width, height, scale) };
    }

    pub fn set_focused(&self, focused: bool) {
        // SAFETY: view is live.
        unsafe { (self.lib.view_focus)(self.view, focused) };
    }

    /// A hidden window's page is throttled by the engine, unless the policy says otherwise.
    pub fn set_visible(&self, visible: bool) {
        if visible || self.throttle {
            // SAFETY: view is live.
            unsafe { (self.lib.view_visible)(self.view, visible) };
        }
    }

    pub fn set_dark(&self, dark: bool) {
        // SAFETY: view is live.
        unsafe { (self.lib.view_theme)(self.view, dark) };
    }

    pub fn input(&self, ev: &Input) {
        let mut c = ffi::InputC {
            kind: 0,
            button: 0,
            pressed: 0,
            mods: 0,
            repeat: 0,
            x: 0.0,
            y: 0.0,
            dx: 0.0,
            dy: 0.0,
            key: std::ptr::null(),
            code: std::ptr::null(),
        };
        let (mut key, mut code) = (None, None);
        match ev {
            Input::MouseMove { x, y } => (c.kind, c.x, c.y) = (ffi::INPUT_MOUSE_MOVE, *x, *y),
            Input::MouseButton {
                button,
                pressed,
                x,
                y,
            } => {
                (c.kind, c.button, c.pressed, c.x, c.y) = (
                    ffi::INPUT_MOUSE_BUTTON,
                    *button,
                    u32::from(*pressed),
                    *x,
                    *y,
                )
            }
            Input::Wheel { x, y, dx, dy } => {
                (c.kind, c.x, c.y, c.dx, c.dy) = (ffi::INPUT_WHEEL, *x, *y, *dx, *dy)
            }
            Input::Key {
                key: k,
                code: cd,
                pressed,
                mods,
                repeat,
            } => {
                (c.kind, c.pressed, c.mods, c.repeat) = (
                    ffi::INPUT_KEY,
                    u32::from(*pressed),
                    *mods,
                    u32::from(*repeat),
                );
                key = CString::new(k.as_str()).ok();
                code = cd.as_deref().and_then(|s| CString::new(s).ok());
            }
            Input::ImePreedit(t) | Input::ImeCommit(t) => {
                c.kind = if matches!(ev, Input::ImePreedit(_)) {
                    ffi::INPUT_IME_PREEDIT
                } else {
                    ffi::INPUT_IME_COMMIT
                };
                key = CString::new(t.as_str()).ok();
            }
            Input::ImeDismiss => c.kind = ffi::INPUT_IME_DISMISS,
            Input::MouseLeave => c.kind = ffi::INPUT_MOUSE_LEAVE,
        }
        c.key = key.as_ref().map_or(std::ptr::null(), |s| s.as_ptr());
        c.code = code.as_ref().map_or(std::ptr::null(), |s| s.as_ptr());
        // SAFETY: view is live; c and its strings are valid for the call.
        unsafe { (self.lib.view_input)(self.view, &c) };
    }

    /// Applies what [`crate::input::bridge`] made of a wolkit record.
    pub fn apply(&self, b: &Bridged) {
        match b {
            Bridged::Input(i) => self.input(i),
            Bridged::Focus(f) => self.set_focused(*f),
            Bridged::Resize {
                width,
                height,
                scale,
            } => self.resize(*width, *height, *scale),
            Bridged::Theme { dark } => self.set_dark(*dark),
        }
    }

    /// Navigations the gate refused since the last call (the host offers them in the browser).
    pub fn take_refused(&self) -> Vec<String> {
        std::mem::take(&mut *self.shared.refused.borrow_mut())
    }

    /// Resource loads (subresources, a frame's document, sockets) the gate has refused so far.
    pub fn refused_loads(&self) -> u64 {
        self.shared.refused_loads.get()
    }
}

/// What [`WebView::wait_loaded`] waits for, from the view's state: a crash ends the wait as an
/// error (no load follows one), a complete load as success, anything else goes on.
fn load_outcome(s: &ffi::State) -> Option<Result<(), Error>> {
    if s.crashed > 0 {
        Some(Err(Error::Engine(
            "the page's content process crashed".into(),
        )))
    } else if s.load == ffi::LOAD_COMPLETE {
        Some(Ok(()))
    } else {
        None
    }
}

// Releases the engine's view, then the engine (which ends its content process, at most a few
// seconds late: the library ends one that does not answer, and gives the one-engine slot back);
// the callback context `shared` is dropped after, and the library calls no callback, the waker
// included, once engine_free has returned.
impl Drop for WebView {
    fn drop(&mut self) {
        // SAFETY: both are live and freed once, the view first; `shared` is dropped after this.
        unsafe {
            (self.lib.view_free)(self.view);
            (self.lib.engine_free)(self.engine);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(load: u32, crashed: u32) -> ffi::State {
        ffi::State {
            load,
            crashed,
            frames: 0,
        }
    }

    #[test]
    fn a_crash_ends_the_wait_for_the_load_at_once() {
        assert_eq!(load_outcome(&state(1, 0)), None, "still loading");
        assert_eq!(load_outcome(&state(ffi::LOAD_COMPLETE, 0)), Some(Ok(())));
        for load in [0, 1, 2, ffi::LOAD_COMPLETE] {
            assert!(
                matches!(load_outcome(&state(load, 1)), Some(Err(Error::Engine(_)))),
                "load {load}"
            );
        }
    }

    /// A content process is a confined one: the flag that makes the library confine it is on
    /// whenever the flag that starts it is, and never without it.
    #[test]
    fn a_content_process_is_always_a_confined_one() {
        let flags = engine_flags(true);
        assert_ne!(flags & ffi::MULTIPROCESS, 0);
        assert_ne!(flags & ffi::SANDBOX, 0, "the content process is confined");
        assert_eq!(
            engine_flags(false),
            0,
            "no content process, nothing to confine"
        );
        // The default app has a content process (process.rs, plan-wave3 WA-3).
        let app = App::new("org.wolos.webapp.t", std::path::PathBuf::from("/tmp/x"));
        assert_ne!(engine_flags(app.content_process) & ffi::SANDBOX, 0);
    }
}
