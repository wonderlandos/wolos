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

/// A host's callback for the texts a page posts.
pub type OnMessage = Box<dyn FnMut(&str)>;

/// The page-to-host channel of a system web app (WWE_1.1, ADR 0048): `window.__wolosHost` in the
/// app's own documents, and the host's callback for what the page posts.
pub struct Channel {
    /// `file:///dir/` of the policy: the documents that get the channel.
    pub origin: String,
    /// The `init` record the page reads before its scripts run (JSON text).
    pub init: String,
    /// Called on the engine's thread (inside [`WebView::spin`]) with each text the page posts.
    pub on_message: OnMessage,
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
    /// Set for a system web app; see [`WebView::has_channel`] for a library that cannot carry it.
    pub channel: Option<Channel>,
    /// Called from any of the engine's threads when it wants [`WebView::spin`] to run. A host that
    /// waits on a descriptor (the Wolkit engine's `dispatch(-1)`) sets this to the call that ends
    /// the wait, and sleeps until then instead of polling; without it the host must spin on a timer.
    pub on_wake: Option<WakeFn>,
}

/// What a host runs when the engine wants to be spun (any thread).
pub type WakeFn = Box<dyn Fn() + Send + Sync>;

impl App {
    pub fn new(id: &str, profile_dir: PathBuf) -> App {
        App {
            id: id.to_owned(),
            profile_dir,
            content_process: true,
            clipboard: None,
            channel: None,
            on_wake: None,
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
    /// The page's messages (WWE_1.1); taken while it runs, so a reentrant post is dropped.
    on_message: RefCell<Option<OnMessage>>,
    /// The only field the waker touches (it runs on the engine's threads).
    woken: AtomicBool,
    /// The host's wake-up, set once before the engine exists and never changed: any thread reads it.
    on_wake: Option<WakeFn>,
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

// SAFETY: WWE_1.1, as allow_cb: called on the view's thread with the `Shared` boxed as `ctx`; `text`
// has `len` readable bytes.
unsafe extern "C" fn message_cb(ctx: *mut c_void, text: *const u8, len: usize) {
    // SAFETY: ctx is the Shared this view boxed (it outlives the engine); the engine passes len
    // valid bytes.
    let (s, bytes) = unsafe { (&*ctx.cast::<Shared>(), ffi::bytes_in(text, len)) };
    deliver(s, bytes);
}

/// One page message to the host's callback: UTF-8 only, and not while the callback is running.
fn deliver(s: &Shared, bytes: &[u8]) {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let Ok(mut slot) = s.on_message.try_borrow_mut() else {
        return;
    };
    if let Some(f) = slot.as_mut() {
        f(text);
    }
}

// SAFETY: called from any engine thread; touches only the two fields of `ctx` that are made for it
// (the atomic and the host's `Send + Sync` wake-up), through raw places.
unsafe extern "C" fn wake_cb(ctx: *mut c_void) {
    let shared = ctx.cast::<Shared>();
    // SAFETY: only those two fields are borrowed, not the struct: `woken` is atomic, and `on_wake` is
    // written before the engine exists and read-only afterwards, so a shared reference to it is sound
    // on any thread (it is `Send + Sync`).
    let (woken, on_wake) = unsafe { (&(*shared).woken, &(*shared).on_wake) };
    woken.store(true, Ordering::Release);
    if let Some(wake) = on_wake {
        wake();
    }
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
    /// The library carried the channel (WWE_1.1) and the page has `window.__wolosHost`.
    channel: bool,
    // Last: the engine is freed (in Drop) before what its callbacks point at.
    shared: Box<Shared>,
}

const ERR: usize = 512;

fn cstring(s: &str, what: &str) -> Result<CString, Error> {
    CString::new(s).map_err(|_| Error::Policy(format!("{what} has a NUL")))
}

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

/// The start URL the view opens: `url`, or its `fed://` spelling when `map` has one, and in both
/// cases only if the policy's gate lets that address be navigated to.
fn checked_start(
    gate: &Gate,
    url: &str,
    map: impl Fn(&str) -> Option<String>,
) -> Result<String, Error> {
    let opens = map(url).unwrap_or_else(|| url.to_owned());
    if gate.allows(&opens, Load::Navigation) {
        Ok(opens)
    } else {
        Err(Error::Policy(format!("{url} is outside the app's origins")))
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
        // A start URL the network spells `fed://` (`http://home.fed`) opens as that address, so the
        // policy names `fed://name.tld` and nothing about the web (ADR 0052).
        let start = checked_start(&gate, url, crate::fed::map_http)?;
        let url = start.as_str();
        std::fs::create_dir_all(&app.profile_dir)
            .map_err(|e| Error::Policy(format!("{}: {e}", app.profile_dir.display())))?;
        let lib = ffi::lib().map_err(Error::Library)?;
        let has_clip = app.clipboard.is_some();
        let shared = Box::new(Shared {
            gate,
            refused: RefCell::new(Vec::new()),
            refused_loads: std::cell::Cell::new(0),
            clipboard: app.clipboard,
            on_message: RefCell::new(None),
            woken: AtomicBool::new(false),
            on_wake: app.on_wake,
        });
        let profile = CString::new(app.profile_dir.to_string_lossy().into_owned())
            .map_err(|_| Error::Policy("the profile directory has a NUL".into()))?;
        let curl = CString::new(url).map_err(|_| Error::Policy("the URL has a NUL".into()))?;
        let strings = match &app.channel {
            Some(c) => Some((
                cstring(&c.origin, "the channel origin")?,
                cstring(&c.init, "the channel init")?,
            )),
            None => None,
        };
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
        // WWE_1.1: the page's channel to the host, for the views made after this call. A library of
        // the earlier build has no such call: the app then runs in development mode (`has_channel`).
        let mut channel = false;
        let mut failed = None;
        if let (Some(ch), Some((o, i)), Some(set)) = (app.channel, strings, lib.set_host_channel) {
            *shared.on_message.borrow_mut() = Some(ch.on_message);
            // SAFETY: engine is live; o and i are valid for the call; message_cb and ctx live until
            // engine_free (Drop), and ctx is the boxed Shared.
            let rc = unsafe {
                set(
                    engine,
                    o.as_ptr(),
                    i.as_ptr(),
                    Some(message_cb),
                    ctx,
                    err.as_mut_ptr(),
                    ERR,
                )
            };
            if rc == 0 {
                channel = true;
            } else {
                failed = Some(Error::Engine(take_err(&err)));
            }
        }
        // WWE_1.2 (ADR 0052): Federate Network addresses are served natively, always: there is no
        // setting. A library of an earlier build has no such call and loads no `fed:` address.
        if failed.is_none()
            && let Some(set) = lib.set_scheme
        {
            // SAFETY: engine is live; the scheme name is static; the callbacks are plain functions
            // and take no context, so nothing they use can be freed before the engine.
            let rc = unsafe {
                set(
                    engine,
                    crate::fed::SCHEME.as_ptr(),
                    Some(crate::fed::load_cb),
                    Some(crate::fed::map_cb),
                    std::ptr::null_mut(),
                    err.as_mut_ptr(),
                    ERR,
                )
            };
            if rc != 0 {
                failed = Some(Error::Engine(take_err(&err)));
            }
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
        let view = if failed.is_some() {
            std::ptr::null_mut()
        } else {
            // SAFETY: engine is live; t and curl are valid for the call.
            unsafe { (lib.view_new)(engine, &t, curl.as_ptr(), err.as_mut_ptr(), ERR) }
        };
        if view.is_null() {
            // SAFETY: engine has no view. Freeing it gives the library's one-engine slot back,
            // so a later WebView::new in this process can make one again.
            unsafe { (lib.engine_free)(engine) };
            return Err(failed.unwrap_or_else(|| Error::Engine(take_err(&err))));
        }
        Ok(WebView {
            lib,
            engine,
            view,
            throttle: policy.background_throttle,
            channel,
            shared,
        })
    }

    /// Whether the page has `window.__wolosHost`: the app asked for a channel and the library
    /// (WWE_1.1) carries it. False means development mode only.
    pub fn has_channel(&self) -> bool {
        self.channel
    }

    /// Hands `text` to the page's `window.__wolosHost.onmessage`. False when there is no channel,
    /// the document is not the app's, or the text is over [`crate::bridge::MAX_MESSAGE`].
    pub fn post_message(&self, text: &str) -> bool {
        let Some(post) = self.lib.post_message.filter(|_| self.channel) else {
            return false;
        };
        if text.len() > crate::bridge::MAX_MESSAGE {
            return false;
        }
        // SAFETY: view is live; text is valid for the call.
        unsafe { post(self.view, text.as_ptr(), text.len()) == 0 }
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

    fn gate_of(origins: &[&str]) -> Gate {
        let p = Policy {
            origins: origins.iter().map(|s| (*s).to_owned()).collect(),
            ..Policy::default()
        };
        Gate::new(
            &p,
            std::path::Path::new("/home/a/.local/share/wolos/profiles/x"),
        )
        .unwrap()
    }

    /// The network's own mapping without a network: the 23 known TLDs, nothing held.
    fn map(url: &str) -> Option<String> {
        struct NoNet;
        impl federate::Source for NoNet {
            fn get(&self, _: &str, _: u64) -> Result<Vec<u8>, String> {
                Err("no network in this test".into())
            }
        }
        federate::Resolver::new(Box::new(NoNet), federate::ROOT_KEY).map_http(url)
    }

    #[test]
    fn a_mapped_start_url_is_allowed_when_the_policy_has_the_fed_origin() {
        let g = gate_of(&["fed://home.fed"]);
        assert_eq!(
            checked_start(&g, "http://home.fed/", map).unwrap(),
            "fed://home.fed"
        );
        assert_eq!(
            checked_start(&g, "fed://home.fed/", map).unwrap(),
            "fed://home.fed/"
        );
    }

    #[test]
    fn a_mapped_start_url_is_denied_when_the_policy_has_neither_spelling() {
        let g = gate_of(&["fed://lobo.pagina", "https://example.com"]);
        assert!(matches!(
            checked_start(&g, "http://home.fed/", map),
            Err(Error::Policy(m)) if m.contains("http://home.fed/")
        ));
        let none = gate_of(&[]);
        assert!(checked_start(&none, "http://home.fed/", map).is_err());
    }

    #[test]
    fn a_non_network_http_host_is_left_alone() {
        // `.com` is not one of the network's TLDs: no mapping, and the policy decides as before.
        let g = gate_of(&["https://example.com", "fed://home.fed"]);
        assert_eq!(map("https://example.com/"), None);
        assert_eq!(
            checked_start(&g, "https://example.com/", map).unwrap(),
            "https://example.com/"
        );
        assert!(checked_start(&g, "https://other.com/", map).is_err());
    }

    /// Exclusive Federate names: no start URL under a Federate TLD stays http(s), whether it is a
    /// valid name or not; what the mapping cannot make a name of the gate refuses.
    #[test]
    fn a_federate_tld_start_url_is_never_left_as_http() {
        let g = gate_of(&["fed://home.fed", "https://example.com"]);
        for url in [
            "http://www.home.fed/",
            "https://home.fed@evil.fed/",
            "http://lobo.pagina:8080/",
        ] {
            let opens = map(url).expect(url);
            assert!(opens.starts_with("fed://"), "{url} -> {opens}");
            assert!(checked_start(&g, url, map).is_err(), "{url}");
        }
        assert_eq!(
            checked_start(&g, "https://HOME.fed./x", map).unwrap(),
            "fed://home.fed/x"
        );
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
