// SPDX-License-Identifier: BSD-2-Clause
//! libwolwebengine (ADR 0042): the C boundary over the pinned Rust browser engine, declared in
//! `crates/webengine/engine/include/wolwebengine.h`. This file is not a module of `webengine`: the
//! `webengine-wolos` package builds it as the `wolwebengine` cdylib inside the engine's workspace
//! (`crates/webengine/engine/Cargo.toml`), and `webengine` loads the result at run time. It is
//! counted by the unsafe ratchet under `webengine` all the same.
//!
//! What it adds to the engine, and nothing else: the origin gate (every navigation and every
//! resource load asks the host's `allow`; a refusal is a network error, a refused navigation does
//! not happen), the profile directory as the engine's only place for files, the page's features as
//! preferences, the host's clipboard and waker, and a synchronous screenshot for the goldens.
//!
//! A fault in the engine stays on this side ([`boundary`]): no export unwinds (a panic is an error
//! return, or a log line where the call returns nothing), the waker stops when the engine is
//! freed, the engine's shutdown is watched, and the one-engine slot comes back with the engine.
mod boundary;

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_void};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use dpi::PhysicalSize;
use raw_window_handle::{
    DisplayHandle, RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
    WindowHandle,
};
use servo::{
    ClipboardDelegate, Code, CompositionEvent, CompositionState, DevicePoint, EventLoopWaker,
    ImeEvent, InputEvent, Key, KeyState, KeyboardEvent, LoadStatus, Location, Modifiers,
    MouseButton, MouseButtonAction, MouseButtonEvent, MouseLeftViewportEvent, MouseMoveEvent,
    NavigationRequest, Opts, Preferences, RenderingContext, Servo, ServoBuilder,
    SoftwareRenderingContext, StringRequest, Theme, WebResourceLoad, WebResourceResponse, WebView,
    WebViewBuilder, WebViewDelegate, WheelDelta, WheelEvent, WheelMode, WindowRenderingContext,
};
use url::Url;

const ABI: u32 = 1;
const VERSION: &CStr = c"servo 0.7.0 (aac43a3f31a259f04a574f5ec4e959c943ad7cc7)";

pub const WWE_MULTIPROCESS: u32 = 1;
pub const WWE_SANDBOX: u32 = 2;
pub const WWE_FEATURE_WEBGL2: u32 = 1;
pub const WWE_FEATURE_WEBGPU: u32 = 2;
pub const WWE_FEATURE_SERVICE_WORKER: u32 = 4;
pub const WWE_FEATURE_INDEXEDDB: u32 = 8;
pub const WWE_FEATURE_NOTIFICATIONS: u32 = 16;
pub const WWE_FEATURE_WEBRTC: u32 = 32;
const LOAD_NAVIGATION: u32 = 0;
const LOAD_RESOURCE: u32 = 1;

// SAFETY: the host's callbacks, called with its `ctx` (header contract).
type AllowFn = unsafe extern "C" fn(*mut c_void, *const u8, usize, u32) -> bool;
type WakeFn = unsafe extern "C" fn(*mut c_void);
type ClipGetFn = unsafe extern "C" fn(*mut c_void, *mut u8, usize) -> isize;
type ClipSetFn = unsafe extern "C" fn(*mut c_void, *const u8, usize);

#[repr(C)]
pub struct Config {
    abi: u32,
    flags: u32,
    features: u32,
    heap_mb: u32,
    profile_dir: *const c_char,
    user_agent: *const c_char,
    allow: Option<AllowFn>,
    wake: Option<WakeFn>,
    clip_get: Option<ClipGetFn>,
    clip_set: Option<ClipSetFn>,
    ctx: *mut c_void,
}

#[repr(C)]
pub struct Target {
    kind: u32,
    width: u32,
    height: u32,
    scale: f32,
    wl_display: *mut c_void,
    wl_surface: *mut c_void,
}

#[repr(C)]
pub struct Input {
    kind: u32,
    button: u32,
    pressed: u32,
    mods: u32,
    repeat: u32,
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    key: *const c_char,
    code: *const c_char,
}

#[repr(C)]
pub struct State {
    load: u32,
    crashed: u32,
    frames: u64,
}

#[repr(C)]
pub struct Image {
    width: u32,
    height: u32,
    rgba: *mut u8,
}

/// The host's callbacks and the context they get, called on the engine thread only (the waker,
/// which any thread calls, is [`Waker`]).
#[derive(Clone, Copy)]
struct Host {
    allow: AllowFn,
    clip_get: Option<ClipGetFn>,
    clip_set: Option<ClipSetFn>,
    ctx: *mut c_void,
}

impl Host {
    fn allows(&self, url: &Url, kind: u32) -> bool {
        let s = url.as_str();
        // SAFETY: the host's function and context (Config contract); s is valid for the call.
        unsafe { (self.allow)(self.ctx, s.as_ptr(), s.len(), kind) }
    }
}

/// The waker crosses threads: the host promised `wake` may be called from any thread. Every copy
/// the engine's threads hold calls through one [`boundary::Revocable`], which `wwe_engine_free`
/// revokes once the engine has shut down: a thread the engine leaves behind cannot wake the host's
/// context after the host has freed it.
#[derive(Clone)]
struct Waker(boundary::Revocable<(WakeFn, usize)>);
impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }
    fn wake(&self) {
        self.0.call(|(wake, ctx)| {
            // SAFETY: Config contract: `wake` is callable from any thread with `ctx` until
            // wwe_engine_free returns, and the gate is revoked before it does.
            unsafe { wake(std::ptr::with_exposed_provenance_mut(ctx)) }
        });
    }
}

/// How long `wwe_engine_free` waits for the engine to shut down before it ends the content
/// processes (a wedged one never answers the engine's exit).
const SHUTDOWN: Duration = Duration::from_secs(5);

pub struct Engine {
    servo: Servo,
    host: Host,
    waker: Option<boundary::Revocable<(WakeFn, usize)>>,
    /// Last: the slot comes back once the engine is gone.
    _slot: boundary::Claim<'static>,
}

#[derive(Default)]
struct ViewState {
    load: Cell<u32>,
    crashed: Cell<u32>,
    frames: Cell<u64>,
}

struct Delegate {
    host: Host,
    state: Rc<ViewState>,
}

impl WebViewDelegate for Delegate {
    // A refused navigation must not stall the page: at the pin, denying a frame's first
    // navigation leaves the parent waiting for that frame's load event forever (no `load`, no
    // screenshot). So a refused http(s) navigation is let through here (the host has been told:
    // its `allow` said no) and stopped as a load below, where the engine says whether it is the
    // page or a frame. Other refused schemes are denied outright.
    fn request_navigation(&self, _webview: WebView, request: NavigationRequest) {
        let web = matches!(request.url.scheme(), "http" | "https");
        if self.host.allows(&request.url, LOAD_NAVIGATION) || web {
            request.allow();
        } else {
            request.deny();
        }
    }

    // A refused load: for the page itself `204 No Content`, which ends a navigation with the page
    // left as it was (HTML: a 204 or 205 response aborts the navigation); for anything else an
    // empty `403 Forbidden`, so a frame gets an empty document and fires `load` (a cancelled load
    // stalls the parent's `load` like a denied navigation at the pin).
    fn load_web_resource(&self, _webview: WebView, load: WebResourceLoad) {
        if self.host.allows(&load.request().url, LOAD_RESOURCE) {
            return;
        }
        let url = load.request().url.clone();
        let status = if load.request().is_for_main_frame {
            http::StatusCode::NO_CONTENT
        } else {
            http::StatusCode::FORBIDDEN
        };
        load.intercept(WebResourceResponse::new(url).status_code(status))
            .finish();
    }

    fn notify_new_frame_ready(&self, webview: WebView) {
        self.state.frames.set(self.state.frames.get() + 1);
        webview.paint();
    }

    fn notify_load_status_changed(&self, _webview: WebView, status: LoadStatus) {
        self.state.load.set(match status {
            LoadStatus::Started => 1,
            LoadStatus::HeadParsed => 2,
            LoadStatus::Complete => 3,
        });
    }

    fn notify_crashed(&self, _webview: WebView, reason: String, _backtrace: Option<String>) {
        log::error!("wolwebengine: content crashed: {reason}");
        self.state.crashed.set(self.state.crashed.get() + 1);
    }
}

struct Clipboard(Host);
impl ClipboardDelegate for Clipboard {
    fn get_text(&self, _webview: WebView, request: StringRequest) {
        let Some(get) = self.0.clip_get else {
            request.failure("no clipboard".into());
            return;
        };
        let mut buf = vec![0u8; 4096];
        // SAFETY: host contract; buf is writable for its length.
        let mut n = unsafe { get(self.0.ctx, buf.as_mut_ptr(), buf.len()) };
        if let Ok(want) = usize::try_from(n)
            && want > buf.len()
        {
            buf.resize(want, 0);
            // SAFETY: as above, with the length the host asked for.
            n = unsafe { get(self.0.ctx, buf.as_mut_ptr(), buf.len()) };
        }
        match usize::try_from(n) {
            Ok(n) if n <= buf.len() => {
                buf.truncate(n);
                request.success(String::from_utf8_lossy(&buf).into_owned());
            }
            _ => request.failure("clipboard read failed".into()),
        }
    }

    fn set_text(&self, _webview: WebView, text: String) {
        if let Some(set) = self.0.clip_set {
            // SAFETY: host contract; text is valid for the call.
            unsafe { set(self.0.ctx, text.as_ptr(), text.len()) }
        }
    }
}

pub struct View {
    webview: WebView,
    context: Rc<dyn RenderingContext>,
    state: Rc<ViewState>,
    engine: *mut Engine,
}

/// One engine per process at a time (the engine keeps process-wide state).
static ENGINE_SLOT: boundary::Slot = boundary::Slot::new();

/// Runs an export's body; a panic is logged instead of unwinding into the host.
fn guarded(what: &str, f: impl FnOnce()) {
    if let Err(m) = boundary::catch(f) {
        log::error!("wolwebengine: {what}: {m}");
    }
}

/// The engine's shutdown is late: its content processes (children of this process started with
/// `--content-process`) are ended, so the engine sees them gone and finishes.
fn end_content_processes() {
    let kids = boundary::content_children(Path::new("/proc"), std::process::id());
    log::error!(
        "wolwebengine: the engine has not shut down in {} s; ending {} content process(es)",
        SHUTDOWN.as_secs(),
        kids.len()
    );
    for kid in kids
        .into_iter()
        .filter_map(|k| libc::pid_t::try_from(k).ok())
    {
        // SAFETY: kill takes a pid and a signal number and touches no memory of ours; the pid is
        // an unreaped child of this process (listed in its children just above).
        unsafe { libc::kill(kid, libc::SIGKILL) };
    }
}

fn put_err(err: *mut c_char, len: usize, msg: &str) {
    if err.is_null() || len == 0 {
        return;
    }
    let n = msg.len().min(len - 1);
    // SAFETY: the caller gave `len` writable bytes at `err` (header contract).
    unsafe {
        std::ptr::copy_nonoverlapping(msg.as_ptr(), err.cast::<u8>(), n);
        *err.add(n) = 0;
    }
}

/// # Safety
/// `p` is NULL or a NUL-terminated string.
unsafe fn opt_str(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: contract above.
    Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

fn prefs(config: &Config) -> Preferences {
    let mut p = Preferences::default();
    let on = |bit: u32| config.features & bit != 0;
    p.dom_webgl2_enabled = on(WWE_FEATURE_WEBGL2);
    p.dom_webgpu_enabled = on(WWE_FEATURE_WEBGPU);
    p.dom_serviceworker_enabled = on(WWE_FEATURE_SERVICE_WORKER);
    p.dom_indexeddb_enabled = on(WWE_FEATURE_INDEXEDDB);
    p.dom_notification_enabled = on(WWE_FEATURE_NOTIFICATIONS);
    p.dom_webrtc_enabled = on(WWE_FEATURE_WEBRTC);
    if config.heap_mb > 0 {
        p.js_mem_max = i64::from(config.heap_mb);
    }
    // No proxy from the environment: the origin gate is the only way out.
    p.network_http_proxy_uri = String::new();
    p.network_https_proxy_uri = String::new();
    p
}

// SAFETY: an exported symbol name (no_mangle); no arguments, no state.
#[unsafe(no_mangle)]
pub extern "C" fn wwe_abi_version() -> u32 {
    ABI
}

// SAFETY: an exported symbol name (no_mangle); returns a static string.
#[unsafe(no_mangle)]
pub extern "C" fn wwe_engine_version() -> *const c_char {
    VERSION.as_ptr()
}

/// # Safety
/// `config` is NULL or valid (header contract); `err` has `errlen` writable bytes or is NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_engine_new(
    config: *const Config,
    err: *mut c_char,
    errlen: usize,
) -> *mut Engine {
    // SAFETY: contract above.
    let Some(config) = (unsafe { config.as_ref() }) else {
        put_err(err, errlen, "config is NULL");
        return std::ptr::null_mut();
    };
    if config.abi != ABI {
        put_err(
            err,
            errlen,
            &format!("ABI {} asked, {ABI} built", config.abi),
        );
        return std::ptr::null_mut();
    }
    let Some(allow) = config.allow else {
        put_err(err, errlen, "allow is required");
        return std::ptr::null_mut();
    };
    // SAFETY: contract above.
    let Some(profile) = (unsafe { opt_str(config.profile_dir) }) else {
        put_err(err, errlen, "profile_dir is required");
        return std::ptr::null_mut();
    };
    let profile = PathBuf::from(profile);
    if !profile.is_absolute() {
        put_err(err, errlen, "profile_dir must be absolute");
        return std::ptr::null_mut();
    }
    // SAFETY: contract above.
    let user_agent = unsafe { opt_str(config.user_agent) };
    let waker = config
        .wake
        .map(|wake| boundary::Revocable::new((wake, config.ctx.expose_provenance())));
    let made = boundary::catch(|| {
        // Dropped on every way out but the engine's own, which keeps it until it is freed.
        let slot = ENGINE_SLOT
            .claim()
            .ok_or("one engine per process at a time")?;
        let mut prefs = prefs(config);
        if let Some(ua) = user_agent {
            prefs.user_agent = ua;
        }
        prefs.network_http_disk_cache = profile.join("cache").to_string_lossy().into_owned();
        let mut opts = Opts::default();
        opts.config_dir = Some(profile);
        opts.multiprocess = config.flags & WWE_MULTIPROCESS != 0;
        opts.sandbox = opts.multiprocess && config.flags & WWE_SANDBOX != 0;
        let host = Host {
            allow,
            clip_get: config.clip_get,
            clip_set: config.clip_set,
            ctx: config.ctx,
        };
        let mut builder = ServoBuilder::default().opts(opts).preferences(prefs);
        if let Some(w) = &waker {
            builder = builder.event_loop_waker(Box::new(Waker(w.clone())));
        }
        let servo = builder.build();
        Ok::<_, &str>(Box::into_raw(Box::new(Engine {
            servo,
            host,
            waker: waker.clone(),
            _slot: slot,
        })))
    });
    let failed = match made {
        Ok(Ok(engine)) => return engine,
        Ok(Err(m)) => m.to_owned(),
        Err(m) => m,
    };
    // A half-built engine may have handed the waker to threads: none of them wakes the host's
    // context, which the host frees on this error.
    if let Some(w) = &waker {
        w.revoke();
    }
    put_err(err, errlen, &failed);
    std::ptr::null_mut()
}

/// Shuts the engine down (it ends its content processes) and frees it. The engine waits for its
/// content processes to answer; one that does not is ended after [`SHUTDOWN`], logged. When this
/// returns no callback is called again (the waker is revoked) and a new engine may be made.
///
/// # Safety
/// `engine` is NULL or from `wwe_engine_new`, with every view of it freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_engine_free(engine: *mut Engine) {
    if engine.is_null() {
        return;
    }
    // SAFETY: contract above.
    let engine = unsafe { Box::from_raw(engine) };
    let Engine {
        servo,
        waker,
        _slot: slot,
        ..
    } = *engine;
    let shutdown =
        boundary::catch(|| boundary::bounded(SHUTDOWN, move || drop(servo), end_content_processes));
    match shutdown {
        Ok(((), false)) => {}
        Ok(((), true)) => log::error!(
            "wolwebengine: the engine shut down late, after its content processes were ended"
        ),
        Err(m) => log::error!("wolwebengine: shutdown: {m}"),
    }
    if let Some(w) = waker {
        w.revoke();
    }
    drop(slot);
}

/// # Safety
/// `engine` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_engine_spin(engine: *mut Engine) {
    // SAFETY: contract above.
    if let Some(e) = unsafe { engine.as_ref() } {
        guarded("spin", || e.servo.spin_event_loop());
    }
}

fn context(t: &Target) -> Result<Rc<dyn RenderingContext>, String> {
    let size = PhysicalSize::new(t.width.max(1), t.height.max(1));
    match t.kind {
        0 => SoftwareRenderingContext::new(size)
            .map(|c| Rc::new(c) as Rc<dyn RenderingContext>)
            .map_err(|e| format!("software context: {e:?}")),
        1 => {
            let display = std::ptr::NonNull::new(t.wl_display).ok_or("wl_display is NULL")?;
            let surface = std::ptr::NonNull::new(t.wl_surface).ok_or("wl_surface is NULL")?;
            let raw_display = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display));
            let raw_window = RawWindowHandle::Wayland(WaylandWindowHandle::new(surface));
            // SAFETY: the wolkit surface node keeps the display and the subsurface alive for
            // as long as the view (the host frees the view first: header contract).
            let (d, w) = unsafe {
                (
                    DisplayHandle::borrow_raw(raw_display),
                    WindowHandle::borrow_raw(raw_window),
                )
            };
            WindowRenderingContext::new(d, w, size)
                .map(|c| Rc::new(c) as Rc<dyn RenderingContext>)
                .map_err(|e| format!("wayland context: {e:?}"))
        }
        k => Err(format!("unknown target kind {k}")),
    }
}

/// # Safety
/// `engine` is live; `target` is valid; `url` is a NUL-terminated string; `err` as in
/// `wwe_engine_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_view_new(
    engine: *mut Engine,
    target: *const Target,
    url: *const c_char,
    err: *mut c_char,
    errlen: usize,
) -> *mut View {
    // SAFETY: contract above.
    let (Some(e), Some(t)) = (unsafe { engine.as_ref() }, unsafe { target.as_ref() }) else {
        put_err(err, errlen, "engine or target is NULL");
        return std::ptr::null_mut();
    };
    // SAFETY: contract above.
    let Some(url) = (unsafe { opt_str(url) }) else {
        put_err(err, errlen, "url is NULL");
        return std::ptr::null_mut();
    };
    let url = match Url::parse(&url) {
        Ok(u) => u,
        Err(x) => {
            put_err(err, errlen, &format!("url: {x}"));
            return std::ptr::null_mut();
        }
    };
    if !e.host.allows(&url, LOAD_NAVIGATION) {
        put_err(err, errlen, "the start url is not an allowed origin");
        return std::ptr::null_mut();
    }
    let made = boundary::catch(|| {
        let context = context(t)?;
        context
            .make_current()
            .map_err(|x| format!("make_current: {x:?}"))?;
        let state = Rc::new(ViewState::default());
        let delegate = Rc::new(Delegate {
            host: e.host,
            state: state.clone(),
        });
        let scale = if t.scale > 0.0 { t.scale } else { 1.0 };
        let webview = WebViewBuilder::new(&e.servo, context.clone())
            .delegate(delegate)
            .clipboard_delegate(Rc::new(Clipboard(e.host)))
            .hidpi_scale_factor(euclid::Scale::new(scale))
            .url(url)
            .build();
        webview.show();
        webview.set_focused(true);
        Ok::<_, String>(Box::into_raw(Box::new(View {
            webview,
            context,
            state,
            engine,
        })))
    });
    match made.and_then(|r| r) {
        Ok(view) => view,
        Err(x) => {
            put_err(err, errlen, &x);
            std::ptr::null_mut()
        }
    }
}

/// # Safety
/// `view` is NULL or from `wwe_view_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_view_free(view: *mut View) {
    if !view.is_null() {
        // SAFETY: contract above.
        let view = unsafe { Box::from_raw(view) };
        guarded("view free", move || drop(view));
    }
}

/// # Safety
/// `view` is live; `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_view_state(view: *const View, out: *mut State) {
    // SAFETY: contract above.
    if let (Some(v), Some(o)) = unsafe { (view.as_ref(), out.as_mut()) } {
        *o = State {
            load: v.state.load.get(),
            crashed: v.state.crashed.get(),
            frames: v.state.frames.get(),
        };
    }
}

/// # Safety
/// `view` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_view_resize(view: *mut View, width: u32, height: u32, scale: f32) {
    // SAFETY: contract above.
    if let Some(v) = unsafe { view.as_ref() } {
        guarded("resize", || {
            v.webview
                .resize(PhysicalSize::new(width.max(1), height.max(1)));
            if scale > 0.0 {
                v.webview.set_hidpi_scale_factor(euclid::Scale::new(scale));
            }
        });
    }
}

fn mods(m: u32) -> Modifiers {
    let mut out = Modifiers::empty();
    for (bit, m2) in [
        (1, Modifiers::SHIFT),
        (2, Modifiers::CONTROL),
        (4, Modifiers::ALT),
        (8, Modifiers::META),
    ] {
        if m & bit != 0 {
            out |= m2;
        }
    }
    out
}

fn input(ev: &Input) -> Option<InputEvent> {
    let point = DevicePoint::new(ev.x, ev.y).into();
    // SAFETY: Input contract: key and code are NULL or NUL-terminated.
    let (key, code) = unsafe { (opt_str(ev.key), opt_str(ev.code)) };
    Some(match ev.kind {
        1 => InputEvent::MouseMove(MouseMoveEvent::new(point)),
        2 => InputEvent::MouseButton(MouseButtonEvent::new(
            if ev.pressed != 0 {
                MouseButtonAction::Down
            } else {
                MouseButtonAction::Up
            },
            match ev.button {
                0 => MouseButton::Primary,
                1 => MouseButton::Auxiliary,
                2 => MouseButton::Secondary,
                n => MouseButton::Other(u16::try_from(n).unwrap_or(u16::MAX)),
            },
            point,
        )),
        3 => InputEvent::Wheel(WheelEvent::new(
            WheelDelta {
                x: f64::from(ev.dx),
                y: f64::from(ev.dy),
                z: 0.0,
                mode: WheelMode::DeltaPixel,
            },
            point,
        )),
        4 => {
            let key: Key = key?.parse().ok()?;
            let code: Code = code
                .and_then(|c| c.parse().ok())
                .unwrap_or(Code::Unidentified);
            let mut k = KeyboardEvent::new_without_event(
                if ev.pressed != 0 {
                    KeyState::Down
                } else {
                    KeyState::Up
                },
                key,
                code,
                Location::Standard,
                mods(ev.mods),
                ev.repeat != 0,
                false,
            );
            k.event.repeat = ev.repeat != 0;
            InputEvent::Keyboard(k)
        }
        5 | 6 => InputEvent::Ime(ImeEvent::Composition(CompositionEvent {
            state: if ev.kind == 5 {
                CompositionState::Update
            } else {
                CompositionState::End
            },
            data: key.unwrap_or_default(),
        })),
        7 => InputEvent::Ime(ImeEvent::Dismissed),
        8 => InputEvent::MouseLeftViewport(MouseLeftViewportEvent::default()),
        _ => return None,
    })
}

/// # Safety
/// `view` is live; `event` is valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_view_input(view: *mut View, event: *const Input) {
    // SAFETY: contract above.
    if let (Some(v), Some(ev)) = unsafe { (view.as_ref(), event.as_ref()) } {
        guarded("input", || {
            if let Some(e) = input(ev) {
                v.webview.notify_input_event(e);
            }
        });
    }
}

/// # Safety
/// `view` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_view_focus(view: *mut View, focused: bool) {
    // SAFETY: contract above.
    if let Some(v) = unsafe { view.as_ref() } {
        guarded("focus", || v.webview.set_focused(focused));
    }
}

/// # Safety
/// `view` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_view_visible(view: *mut View, visible: bool) {
    // SAFETY: contract above.
    if let Some(v) = unsafe { view.as_ref() } {
        guarded("visible", || {
            if visible {
                v.webview.show();
            } else {
                v.webview.hide();
            }
        });
    }
}

/// # Safety
/// `view` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_view_theme(view: *mut View, dark: bool) {
    // SAFETY: contract above.
    if let Some(v) = unsafe { view.as_ref() } {
        guarded("theme", || {
            v.webview
                .notify_theme_change(if dark { Theme::Dark } else { Theme::Light });
        });
    }
}

/// # Safety
/// `view` is live; `out` is writable; `err` as in `wwe_engine_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_view_screenshot(
    view: *mut View,
    timeout_ms: u32,
    out: *mut Image,
    err: *mut c_char,
    errlen: usize,
) -> i32 {
    // SAFETY: contract above.
    let (Some(v), Some(out)) = (unsafe { (view.as_ref(), out.as_mut()) }) else {
        put_err(err, errlen, "view or out is NULL");
        return -1;
    };
    // SAFETY: the view's engine outlives the view (header contract).
    let Some(engine) = (unsafe { v.engine.as_ref() }) else {
        put_err(err, errlen, "engine is gone");
        return -1;
    };
    match boundary::catch(|| screenshot(v, engine, timeout_ms, out)) {
        Ok(Ok(())) => 0,
        Ok(Err(e)) | Err(e) => {
            put_err(err, errlen, &e);
            -1
        }
    }
}

/// The view's next up-to-date rendering into `out` (`wwe_view_screenshot`).
fn screenshot(v: &View, engine: &Engine, timeout_ms: u32, out: &mut Image) -> Result<(), String> {
    let got: Rc<RefCell<Option<Result<servo::RgbaImage, String>>>> = Rc::default();
    let slot = got.clone();
    v.webview.take_screenshot(None, move |r| {
        *slot.borrow_mut() = Some(r.map_err(|e| format!("{e:?}")));
    });
    let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
    while got.borrow().is_none() {
        if Instant::now() > deadline {
            return Err("screenshot timed out".into());
        }
        engine.servo.spin_event_loop();
        std::thread::sleep(Duration::from_millis(1));
    }
    let _ = &v.context;
    match got.borrow_mut().take() {
        Some(Ok(img)) => {
            let (w, h) = img.dimensions();
            // Owned by the caller until wwe_image_free gives it back to a Box.
            let px = Box::into_raw(img.into_raw().into_boxed_slice());
            *out = Image {
                width: w,
                height: h,
                rgba: px.cast::<u8>(),
            };
            Ok(())
        }
        Some(Err(e)) => Err(e),
        None => Err("no screenshot".into()),
    }
}

/// # Safety
/// `image` is NULL or was filled by `wwe_view_screenshot` and not freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_image_free(image: *mut Image) {
    // SAFETY: contract above.
    if let Some(i) = unsafe { image.as_mut() }
        && !i.rgba.is_null()
    {
        let px = |v: u32| usize::try_from(v).unwrap_or(0);
        let n = px(i.width) * px(i.height) * 4;
        // SAFETY: made from a boxed slice of exactly n bytes in wwe_view_screenshot.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(i.rgba, n)) });
        i.rgba = std::ptr::null_mut();
    }
}

/// # Safety
/// `token` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_run_content_process(token: *const c_char) {
    // SAFETY: contract above.
    let token = unsafe { opt_str(token) }.unwrap_or_default();
    guarded("content process", move || servo::run_content_process(token));
}
