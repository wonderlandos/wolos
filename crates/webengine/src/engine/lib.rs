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
//! preferences, the host's clipboard and waker, a synchronous screenshot for the goldens, (WWE_1.1,
//! ADR 0048) a message channel between the app's own page and the host ([`channel`]), and (WWE_1.2,
//! ADR 0052) the `fed:` scheme served by the host, with `http(s)://` Federate names opened as
//! `fed://` ([`scheme`]).
//!
//! A fault in the engine stays on this side ([`boundary`]): no export unwinds (a panic is an error
//! return, or a log line where the call returns nothing), the waker stops when the engine is
//! freed, the engine's shutdown is watched, and the one-engine slot comes back with the engine.
mod boundary;
mod channel;
mod confine;
mod frame;
mod scheme;
mod tls;

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_void};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use dpi::PhysicalSize;
use raw_window_handle::{
    DisplayHandle, RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
    WindowHandle,
};
use servo::protocol_handler::{
    DoneChannel, FetchContext, HttpStatus, ProtocolHandler, ProtocolRegistry, Request,
    ResourceFetchTiming, Response, ResponseBody,
};
use servo::{
    ClipboardDelegate, Code, CompositionEvent, CompositionState, ConsoleLogLevel, DevicePoint,
    EventLoopWaker, ImeEvent, InputEvent, JSValue, Key, KeyState, KeyboardEvent, LoadStatus,
    Location, Modifiers, MouseButton, MouseButtonAction, MouseButtonEvent, MouseLeftViewportEvent,
    MouseMoveEvent, NavigationRequest, Opts, Preferences, RenderingContext, Servo, ServoBuilder,
    SoftwareRenderingContext, StringRequest, Theme, UserContentManager, UserScript,
    WebResourceLoad, WebResourceResponse, WebView, WebViewBuilder, WebViewDelegate, WheelDelta,
    WheelEvent, WheelMode, WindowRenderingContext,
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
// SAFETY: `wwe_message_fn`: the host's callback for a page message, called with its `ctx`.
type MessageFn = unsafe extern "C" fn(*mut c_void, *const u8, usize);

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

/// What `wwe_engine_set_host_channel` was given (WWE_1.1), shared by the views made after it.
struct HostChannel {
    /// `file:///dir/`: the folder whose top-level documents get the channel.
    origin: String,
    init: String,
    /// Made per engine ([`channel::secret`]); only the injected script and the delegate know it.
    secret: String,
    on_message: MessageFn,
    ctx: *mut c_void,
}

impl HostChannel {
    /// A console line of the page: a message for the host when it carries the secret.
    fn console(&self, level: &ConsoleLogLevel, line: &str) {
        if !matches!(level, ConsoleLogLevel::Debug) {
            return;
        }
        if let Some(text) = channel::from_console(&self.secret, line) {
            // SAFETY: host contract (`wwe_message_fn`): callable on the engine thread with `ctx`
            // until wwe_engine_free returns; the views, and so this, are gone before that.
            unsafe { (self.on_message)(self.ctx, text.as_ptr(), text.len()) }
        }
    }
}

pub struct Engine {
    servo: Servo,
    host: Host,
    channel: RefCell<Option<Rc<HostChannel>>>,
    /// WWE_1.2: the host's `fed:` handler, registered with the engine at build, filled later.
    scheme: Arc<scheme::Slot>,
    waker: Option<boundary::Revocable<(WakeFn, usize)>>,
    /// Last: the slot comes back once the engine is gone.
    _slot: boundary::Claim<'static>,
}

/// A page load the host spelled as `fed://` (see `load_web_resource`), waiting to start.
struct Deferred {
    webview: WebView,
    to: Url,
    retry: scheme::Retry,
}

thread_local! {
    /// Mapped page loads not started yet. Only the engine thread touches the list.
    static DEFERRED: RefCell<Vec<Deferred>> = const { RefCell::new(Vec::new()) };
    /// http(s) frame and subresource loads of Federate-TLD hosts, held until the `fed:` chain has
    /// answered (see `load_web_resource`). Only the engine thread touches it.
    static SERVED: RefCell<scheme::Outbox<Held>> = RefCell::new(scheme::Outbox::default());
}

/// A web load held for the `fed:` chain. Never let go unanswered: dropping it (a panic, the
/// engine going away) answers it with a refusal, because an unanswered web load proceeds to the
/// network, and a Federate name never does.
struct Held(Option<WebResourceLoad>);

// A held load dropped unanswered (the engine going away, a view closed) is refused, never left to
// proceed to the network: a Federate name is `fed://` only.
impl Drop for Held {
    fn drop(&mut self) {
        if let Some(load) = self.0.take() {
            refuse(load, http::StatusCode::SERVICE_UNAVAILABLE);
        }
    }
}

/// Answers `load` with `status` and the page that says Federate names are `fed://` only.
fn refuse(load: WebResourceLoad, status: http::StatusCode) {
    let body = scheme::refused_fed_body(load.request().url.as_str());
    answer(load, status, "text/html; charset=utf-8", body.into_bytes());
}

/// Answers `load` (never the network) with `status`, the content type and the body.
fn answer(load: WebResourceLoad, status: http::StatusCode, mime: &str, body: Vec<u8>) {
    let mut headers = http::HeaderMap::new();
    if let Ok(v) = http::HeaderValue::from_str(mime) {
        headers.insert(http::header::CONTENT_TYPE, v);
    }
    let url = load.request().url.clone();
    let mut sending = load.intercept(
        WebResourceResponse::new(url)
            .status_code(status)
            .headers(headers),
    );
    if !body.is_empty() {
        sending.send_body_data(body);
    }
    sending.finish();
}

/// Wakes the host when a held load is answered ([`scheme::Pending::watch`]).
struct HostWake(std::sync::Mutex<Waker>);
impl std::task::Wake for HostWake {
    fn wake(self: Arc<Self>) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .wake();
    }
}

/// Sends the held loads whose reply is in.
fn answer_served() {
    let ready = SERVED.with(|s| {
        let mut s = s.borrow_mut();
        if s.is_empty() {
            return Vec::new();
        }
        s.ready()
    });
    for (mut held, reply) in ready {
        if let Some(load) = held.0.take() {
            let status =
                http::StatusCode::from_u16(reply.status).unwrap_or(http::StatusCode::BAD_GATEWAY);
            answer(load, status, &reply.mime, reply.body);
        }
    }
}

/// The engine is going away: every held load is refused, none is left to proceed.
fn refuse_served() {
    let all = SERVED.with(|s| s.borrow_mut().abandon());
    drop(all);
}
/// How many entries [`DEFERRED`] holds, for the thread that wakes the host while it waits.
static DEFERRED_N: AtomicUsize = AtomicUsize::new(0);
/// Documents (pages and frames) the `fed:` handler was asked for. A mapped load that took shows here.
static FED_DOCUMENTS: AtomicU64 = AtomicU64::new(0);
/// A mapped load is tried every `RETRY_EVERY`, at most `RETRY_MAX` times (5 s), until it takes.
const RETRY_EVERY: Duration = Duration::from_millis(100);
const RETRY_MAX: u32 = 50;

/// Starts the mapped loads that can start. The engine drops `webview.load` while the navigation it
/// replaces still has a pending change (`Constellation::load_url`: "id that sent load msg is being
/// changed already; abort"), and the 204 that ends that navigation removes it a moment later, by a
/// message nobody reports. So a load is tried again until the `fed:` handler is asked for a
/// document; a second try while the first is pending is dropped the same way, so it is harmless.
fn start_deferred() {
    answer_served();
    DEFERRED.with(|d| {
        let mut d = d.borrow_mut();
        d.retain_mut(|x| {
            match x
                .retry
                .step(FED_DOCUMENTS.load(Ordering::Relaxed), Instant::now())
            {
                scheme::Step::Try => {
                    x.webview.load(x.to.clone());
                    true
                }
                scheme::Step::Wait => true,
                scheme::Step::Done => false,
            }
        });
        DEFERRED_N.store(d.len(), Ordering::Relaxed);
    });
}

/// Wakes the host every `RETRY_EVERY` while a mapped load waits (a host that sleeps until woken
/// would not call `wwe_engine_spin` again).
fn nudge(waker: Waker) {
    let spare = waker.clone();
    let started = std::thread::Builder::new()
        .name("wwe-fed-retry".into())
        .spawn(move || {
            for _ in 0..=RETRY_MAX {
                std::thread::sleep(RETRY_EVERY);
                if DEFERRED_N.load(Ordering::Relaxed) == 0 {
                    return;
                }
                waker.wake();
            }
        });
    if let Err(e) = started {
        // A refused thread (TasksMax, RLIMIT_NPROC; RH-224) is logged, and the host is woken once
        // so its spin takes the next try itself; the load's own retry budget ([`RETRY_MAX`]) ends
        // it either way.
        log::error!("wolwebengine: the retry thread did not start: {e}");
        spare.wake();
    }
}

#[derive(Default)]
struct ViewState {
    load: Cell<u32>,
    crashed: Cell<u32>,
    frames: Cell<u64>,
}

struct Delegate {
    /// The context the view paints into; each new frame is presented through it ([`frame`]).
    context: Rc<dyn RenderingContext>,
    host: Host,
    state: Rc<ViewState>,
    channel: Option<Rc<HostChannel>>,
    scheme: Arc<scheme::Slot>,
    /// Wakes the host (see [`nudge`]).
    waker: Option<Waker>,
}

/// The engine's protocol handler for [`scheme::SCHEME`]: each load runs the host's handler on its
/// own thread ([`scheme::spawn`]) and becomes the response, status and content type the host gave.
struct HostScheme(Arc<scheme::Slot>);

impl ProtocolHandler for HostScheme {
    // The trait's own signature at the pin (`FutureResponse<'a>`, a private alias spelled out).
    fn load<'a>(
        &'a self,
        request: &'a mut Request,
        _done_chan: &mut DoneChannel,
        _context: &FetchContext,
    ) -> Pin<Box<dyn Future<Output = Response> + Send + 'a>> {
        let url = request.current_url();
        let timing = ResourceFetchTiming::new(request.timing_type());
        // A document may be larger than what it loads (the host caps the two differently).
        let kind = if request.is_navigation_request() {
            FED_DOCUMENTS.fetch_add(1, Ordering::Relaxed);
            scheme::LOAD_NAVIGATION
        } else {
            scheme::LOAD_RESOURCE
        };
        let pending = scheme::spawn(self.0.clone(), url.as_str().to_owned(), kind);
        Box::pin(async move {
            let reply = pending.await;
            let mut response = Response::new(url, timing);
            *response.body.lock() = ResponseBody::Done(reply.body);
            if let Ok(v) = http::HeaderValue::from_str(&reply.mime) {
                response.headers.insert(http::header::CONTENT_TYPE, v);
            }
            let reason = http::StatusCode::from_u16(reply.status)
                .ok()
                .and_then(|c| c.canonical_reason())
                .unwrap_or("");
            response.status = HttpStatus::new_raw(reply.status, reason.as_bytes().to_vec());
            response
        })
    }

    // Its pages fetch their own files (`fetch("/index.en.md")`) like any page's.
    fn is_fetchable(&self) -> bool {
        true
    }

    // Every byte is verified against signatures and content addresses before it is served.
    fn is_secure(&self) -> bool {
        true
    }
}

impl Delegate {
    /// The page load `load` of an http(s) Federate name ends with `204` and loads `to` (`fed://`)
    /// instead, which then passes the gate like any other navigation.
    fn defer_load(&self, webview: WebView, load: WebResourceLoad, to: Url) {
        let url = load.request().url.clone();
        load.intercept(WebResourceResponse::new(url).status_code(http::StatusCode::NO_CONTENT))
            .finish();
        // Not `webview.load(to)` here: it is dropped while this navigation is pending (see
        // [`start_deferred`]; measured on the forge guest, ENGINE-FED). It starts from the
        // engine's spin ([`wwe_engine_spin`]).
        let first = DEFERRED.with(|d| {
            let mut d = d.borrow_mut();
            d.push(Deferred {
                webview,
                to,
                retry: scheme::Retry::new(
                    RETRY_EVERY,
                    RETRY_MAX,
                    FED_DOCUMENTS.load(Ordering::Relaxed),
                    Instant::now(),
                ),
            });
            DEFERRED_N.store(d.len(), Ordering::Relaxed);
            d.len() == 1
        });
        if let (true, Some(w)) = (first, &self.waker) {
            nudge(w.clone());
        }
    }
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
    //
    // WWE_1.2, exclusive Federate names: a request to an http(s) URL the host spells as `fed://`
    // (the network's compatibility spelling, `http://home.fed`, any host under a Federate TLD) is
    // never sent to the network. A page load ends with `204` and loads that address instead; a
    // frame or a subresource is held and answered from the `fed:` chain ([`SERVED`]); one the gate
    // refuses is answered with a refusal. All pass the gate as the `fed://` address.
    fn load_web_resource(&self, webview: WebView, load: WebResourceLoad) {
        let main = load.request().is_for_main_frame;
        // A Federate name is `fed://` only (lead rule 2026-11-05, ADR 0052): a request to an
        // http(s) host under a Federate TLD is answered from the chain or refused here, for the
        // page, its frames and everything it loads. It never reaches the network, and a failed
        // `fed:` load is the chain's own error page, never a fallback to http(s).
        if let Some(fed) = self.scheme.map(load.request().url.as_str()) {
            let to = Url::parse(&fed).ok();
            if main {
                if let Some(to) = to.filter(|to| self.host.allows(to, LOAD_NAVIGATION)) {
                    self.defer_load(webview, load, to);
                    return;
                }
                // Not the app's to open: the page stays as it was (as every refused page load).
                let url = load.request().url.clone();
                load.intercept(
                    WebResourceResponse::new(url).status_code(http::StatusCode::NO_CONTENT),
                )
                .finish();
                return;
            }
            let kind = scheme::load_kind(&format!("{:?}", load.request().destination));
            // A web page may not read Federate names (probe, fingerprint): only a frame, or a
            // request of a `fed://` page, is answered.
            let referrer = load.request().referrer_url.as_ref().map(Url::to_string);
            // The referrer alone loses a secure Federate page asking for its own file by the
            // http spelling (no referrer on secure to insecure): the view's document counts too.
            let document = webview.url().map(|u| u.as_str().to_owned());
            let asked_by_fed = scheme::fed_request_allowed(
                kind,
                scheme::requester_is_fed(referrer.as_deref(), document.as_deref(), |r| {
                    self.scheme.map(r)
                }),
            );
            match to.filter(|to| asked_by_fed && self.host.allows(to, kind)) {
                Some(to) => {
                    let pending = scheme::spawn(self.scheme.clone(), to.as_str().to_owned(), kind);
                    if let Some(w) = &self.waker {
                        pending.watch(&std::task::Waker::from(Arc::new(HostWake(
                            std::sync::Mutex::new(w.clone()),
                        ))));
                    }
                    SERVED.with(|s| s.borrow_mut().add(Held(Some(load)), pending));
                }
                None => refuse(load, http::StatusCode::FORBIDDEN),
            }
            return;
        }
        if self.host.allows(&load.request().url, LOAD_RESOURCE) {
            return;
        }
        let url = load.request().url.clone();
        let status = if main {
            http::StatusCode::NO_CONTENT
        } else {
            http::StatusCode::FORBIDDEN
        };
        load.intercept(WebResourceResponse::new(url).status_code(status))
            .finish();
    }

    fn notify_new_frame_ready(&self, webview: WebView) {
        frame::ready(
            &self.state.frames,
            &Painted {
                webview: &webview,
                context: &*self.context,
            },
        );
    }

    fn notify_load_status_changed(&self, _webview: WebView, status: LoadStatus) {
        self.state.load.set(match status {
            LoadStatus::Started => 1,
            LoadStatus::HeadParsed => 2,
            LoadStatus::Complete => 3,
        });
    }

    // The page's messages to the host (`window.__wolosHost.post`, channel.rs).
    fn show_console_message(&self, _webview: WebView, level: ConsoleLogLevel, message: String) {
        if let Some(c) = &self.channel {
            c.console(&level, &message);
        }
    }

    fn notify_crashed(&self, _webview: WebView, reason: String, _backtrace: Option<String>) {
        log::error!("wolwebengine: content crashed: {reason}");
        self.state.crashed.set(self.state.crashed.get() + 1);
    }
}

/// One view and its context, for [`frame::ready`].
struct Painted<'a> {
    webview: &'a WebView,
    context: &'a dyn RenderingContext,
}
impl frame::Frame for Painted<'_> {
    fn make_current(&self) {
        if let Err(e) = self.context.make_current() {
            log::error!("wolwebengine: make_current: {e:?}");
        }
    }
    fn paint(&self) {
        self.webview.paint();
    }
    fn present(&self) {
        self.context.present();
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
    /// The channel this view was made with, if the host set one first.
    channel: Option<Rc<HostChannel>>,
    /// Keeps the injected script alive as long as the view.
    _user_content: Option<Rc<UserContentManager>>,
}

/// One engine per process at a time (the engine keeps process-wide state).
static ENGINE_SLOT: boundary::Slot = boundary::Slot::new();

/// Runs an export's body; a panic is logged instead of unwinding into the host.
fn guarded(what: &str, f: impl FnOnce()) {
    if let Err(m) = boundary::catch(f) {
        log::error!("wolwebengine: {what}: {m}");
    }
}

/// A line on stderr for the moments before the engine has a logger.
fn stderr_line(msg: &str) {
    use std::io::Write;
    // Nothing to do when stderr itself is gone.
    let _ = writeln!(std::io::stderr(), "{msg}");
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
    // Content processes run confined or not at all (confine.rs): the host says it knows
    // (WWE_SANDBOX), and this machine must be able to do it, so the failure is an error here and
    // not a content process that dies while the engine waits for it.
    if config.flags & WWE_MULTIPROCESS != 0 {
        if config.flags & WWE_SANDBOX == 0 {
            put_err(
                err,
                errlen,
                "a content process runs confined only: WWE_MULTIPROCESS needs WWE_SANDBOX",
            );
            return std::ptr::null_mut();
        }
        if let Err(e) = confine::preflight() {
            put_err(err, errlen, &e);
            return std::ptr::null_mut();
        }
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
        // Not the engine's own sandbox: at the pin it forks its jail from a multi-threaded host,
        // needs an unprivileged user namespace and a chroot, kills the child on a 2015 allowlist of
        // 21 system calls, never reaps it, and no page loads. WWE_SANDBOX is ours: the content
        // process confines itself in `wwe_run_content_process` (confine.rs), checked above.
        opts.sandbox = false;
        let host = Host {
            allow,
            clip_get: config.clip_get,
            clip_set: config.clip_set,
            ctx: config.ctx,
        };
        // Before the engine reads the process's default TLS provider (its connector falls back
        // to the classical-first default when none is installed; ADR 0055).
        install_tls_provider();
        // WWE_1.2: `fed:` is the host's (ADR 0052); until wwe_engine_set_scheme its loads say so.
        let scheme = Arc::new(scheme::Slot::default());
        let mut protocols = ProtocolRegistry::default();
        protocols
            .register(scheme::SCHEME, HostScheme(scheme.clone()))
            .map_err(|_| "the fed: scheme cannot be registered")?;
        let mut builder = ServoBuilder::default()
            .opts(opts)
            .preferences(prefs)
            .protocol_registry(protocols);
        if let Some(w) = &waker {
            builder = builder.event_loop_waker(Box::new(Waker(w.clone())));
        }
        let servo = builder.build();
        Ok::<_, &str>(Box::into_raw(Box::new(Engine {
            servo,
            host,
            channel: RefCell::new(None),
            scheme,
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

/// The process's rustls provider (the engine's aws-lc-rs one) with the hybrid post-quantum group
/// X25519MLKEM768 first and X25519 second ([`tls::order`]). Installed once per process; a second
/// engine finds it in place. Logged when the provider has no hybrid group.
fn install_tls_provider() {
    let mut p = rustls::crypto::aws_lc_rs::default_provider();
    p.kx_groups = tls::order(p.kx_groups, |g| u16::from(g.name()));
    if p.kx_groups.first().map(|g| u16::from(g.name())) != Some(tls::X25519MLKEM768) {
        log::warn!(
            "wolwebengine: the TLS provider has no X25519MLKEM768; connections stay classical"
        );
    }
    // Err: a provider is already installed (an earlier engine in this process installed this one).
    let _ = p.install_default();
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
        scheme,
        _slot: slot,
        ..
    } = *engine;
    // Held Federate loads are refused now: none is left to proceed to the network.
    refuse_served();
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
    // A fed: load in progress is waited for; none calls the host after this.
    scheme.revoke();
    drop(slot);
}

/// # Safety
/// `engine` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_engine_spin(engine: *mut Engine) {
    // SAFETY: contract above.
    if let Some(e) = unsafe { engine.as_ref() } {
        guarded("spin", || {
            e.servo.spin_event_loop();
            start_deferred();
        });
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
        let channel = e.channel.borrow().clone();
        let delegate = Rc::new(Delegate {
            context: context.clone(),
            host: e.host,
            state: state.clone(),
            channel: channel.clone(),
            scheme: e.scheme.clone(),
            waker: e.waker.clone().map(Waker),
        });
        let scale = if t.scale > 0.0 { t.scale } else { 1.0 };
        let mut builder = WebViewBuilder::new(&e.servo, context.clone())
            .delegate(delegate)
            .clipboard_delegate(Rc::new(Clipboard(e.host)))
            .hidpi_scale_factor(euclid::Scale::new(scale))
            .url(url);
        // The channel script is in the view's user content from its first document on.
        let user_content = channel.as_ref().map(|c| {
            let m = Rc::new(UserContentManager::new(&e.servo));
            let script = channel::bootstrap(&c.origin, &c.init, &c.secret);
            m.add_script(Rc::new(UserScript::new(script, None)));
            m
        });
        if let Some(m) = &user_content {
            builder = builder.user_content_manager(m.clone());
        }
        let webview = builder.build();
        webview.show();
        webview.set_focused(true);
        Ok::<_, String>(Box::into_raw(Box::new(View {
            webview,
            context,
            state,
            engine,
            channel,
            _user_content: user_content,
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
        guarded("view free", move || {
            // A mapped load still waiting holds a handle on a view that is going away.
            DEFERRED.with(|d| d.borrow_mut().clear());
            DEFERRED_N.store(0, Ordering::Relaxed);
            drop(view);
        });
    }
}

/// Shows the frame painted last (`wwe_view_present`, WWE_1.3): the delegate paints into the
/// rendering context when a frame is ready, and a window's surface shows what the context
/// presents.
///
/// # Safety
/// `view` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_view_present(view: *mut View) {
    // SAFETY: contract above.
    if let Some(v) = unsafe { view.as_ref() } {
        guarded("present", || v.context.present());
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

/// WWE_1.1. For the views made after this call: `window.__wolosHost` (`init_json` parsed as its
/// `init`, `post(text)`, `onmessage`) in each top-level document inside `origin`, before the page's
/// scripts; `post` calls `on_message` on the engine thread.
///
/// # Safety
/// `engine` is live; `origin` and `init_json` are NUL-terminated strings; `on_message` is callable
/// with `ctx` until `wwe_engine_free` returns; `err` as in `wwe_engine_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_engine_set_host_channel(
    engine: *mut Engine,
    origin: *const c_char,
    init_json: *const c_char,
    on_message: Option<MessageFn>,
    ctx: *mut c_void,
    err: *mut c_char,
    errlen: usize,
) -> i32 {
    // SAFETY: contract above.
    let (e, origin, init) = unsafe { (engine.as_ref(), opt_str(origin), opt_str(init_json)) };
    let Some(e) = e else {
        put_err(err, errlen, "engine is NULL");
        return -1;
    };
    let (Some(origin), Some(init), Some(on_message)) = (origin, init, on_message) else {
        put_err(err, errlen, "origin, init_json and on_message are required");
        return -1;
    };
    if !channel::valid_origin(&origin) {
        put_err(err, errlen, "the origin must be a file:///dir/ folder");
        return -1;
    }
    if init.len() > channel::MAX_MESSAGE {
        put_err(err, errlen, "init_json is over 64 KiB");
        return -1;
    }
    let secret = match channel::secret() {
        Ok(s) => s,
        Err(m) => {
            put_err(err, errlen, &m);
            return -1;
        }
    };
    *e.channel.borrow_mut() = Some(Rc::new(HostChannel {
        origin,
        init,
        secret,
        on_message,
        ctx,
    }));
    0
}

/// WWE_1.2 (ADR 0052). The host serves `scheme` (only `fed` at this build): each load calls `load`
/// on a thread of its own, and before each page load of an http(s) URL `map` (optional) may spell
/// it as a `fed://` address to load instead.
///
/// # Safety
/// `engine` is live; `scheme` is a NUL-terminated string; `load` and `map` are callable with
/// `ctx` from any thread until `wwe_engine_free` returns; `err` as in `wwe_engine_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_engine_set_scheme(
    engine: *mut Engine,
    scheme_name: *const c_char,
    load: Option<scheme::LoadFn>,
    map: Option<scheme::MapFn>,
    ctx: *mut c_void,
    err: *mut c_char,
    errlen: usize,
) -> i32 {
    // SAFETY: contract above.
    let (e, name) = unsafe { (engine.as_ref(), opt_str(scheme_name)) };
    let Some(e) = e else {
        put_err(err, errlen, "engine is NULL");
        return -1;
    };
    let (Some(name), Some(load)) = (name, load) else {
        put_err(err, errlen, "scheme and load are required");
        return -1;
    };
    if name != scheme::SCHEME {
        put_err(err, errlen, "this build serves the fed scheme only");
        return -1;
    }
    e.scheme.set(scheme::Host {
        load,
        map,
        ctx: ctx.expose_provenance(),
    });
    0
}

/// WWE_1.1. Calls `window.__wolosHost.onmessage(text)` in the view's top-level document when it has
/// the channel: 0 when the call was handed to the document, -1 when it has none (no channel, the
/// document is elsewhere, or the text is over 64 KiB or not UTF-8).
///
/// # Safety
/// `view` is live; `text` has `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_view_post_message(
    view: *mut View,
    text: *const u8,
    len: usize,
) -> i32 {
    // SAFETY: contract above (`text` is read only when it is not NULL and not empty).
    #[allow(clippy::disallowed_methods)] // the boundary slice clippy.toml names
    let (v, bytes) = unsafe {
        (
            view.as_ref(),
            if text.is_null() || len == 0 {
                &[][..]
            } else {
                std::slice::from_raw_parts(text, len)
            },
        )
    };
    let Some(v) = v else {
        return -1;
    };
    let Some(c) = &v.channel else {
        return -1;
    };
    let Some(text) = std::str::from_utf8(bytes)
        .ok()
        .filter(|t| t.len() <= channel::MAX_MESSAGE)
    else {
        return -1;
    };
    match boundary::catch(|| {
        let here = v
            .webview
            .url()
            .is_some_and(|u| channel::inside(&c.origin, u.as_str()));
        if here {
            v.webview.evaluate_javascript(channel::delivery(text), |r| {
                if matches!(r, Ok(JSValue::Boolean(false)) | Err(_)) {
                    log::debug!("wolwebengine: a host message found no handler");
                }
            });
        }
        here
    }) {
        Ok(true) => 0,
        _ => -1,
    }
}

/// # Safety
/// `token` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wwe_run_content_process(token: *const c_char) {
    // SAFETY: contract above.
    let token = unsafe { opt_str(token) }.unwrap_or_default();
    // Before the engine reads anything or starts a thread. A process that cannot be confined does
    // not run the page: this returns, the host's `main` returns and the process ends.
    match confine::enter(&token) {
        Ok(done) => {
            log::debug!("wolwebengine: content process confined: {done:?}");
            if !done.unix_resolve {
                // No logger yet (the engine installs it after the handshake).
                stderr_line(
                    "wolwebengine: this kernel cannot tell pathname unix sockets apart (Landlock \
                     RESOLVE_UNIX): the content process can reach the user's sockets",
                );
            }
        }
        Err(e) => {
            stderr_line(&format!(
                "wolwebengine: the content process cannot be confined: {e}"
            ));
            return;
        }
    }
    guarded("content process", move || servo::run_content_process(token));
}
