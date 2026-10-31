//! `wol_context`: one per app (or per thread that drives libwol). Owns the connections, the event
//! queue and the wake-up fd. Reader threads only queue; callbacks run only inside
//! `wol_context_dispatch`, on the thread that calls it, so any language's event loop can drive
//! libwol by polling `wol_context_get_fd` and calling dispatch when it is readable.
use crate::client;
use crate::error::{Error, Result, Status, UserData, guard, input, need, wol_error, wol_status};
use crate::kit::{self, wol_widget, wol_window};
use crate::notify::{self, wol_notification};
use crate::string::{borrow, into_raw, release, retain, wol_string};
use shared::events::Event;
use shared::view::{FromHost, ToHost};
use std::collections::{HashMap, VecDeque};
use std::ffi::{c_char, c_int, c_void};
use std::io::BufReader;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

/// Topics `wol_subscribe` accepts: the public `shared::events` names. Account events
/// (notifications) are for the shell only.
pub const TOPICS: [&str; 6] = [
    "active-window",
    "open-window",
    "close-window",
    "workspace",
    "theme",
    "job",
];
pub const MAX_APP_ID: usize = 128;

pub type wol_event_fn = Option<unsafe extern "C" fn(*const wol_event, UserData)>;

/// Reverse-DNS app id: `org.example.hello` (the Kit host checks the same rule).
pub fn is_app_id(s: &str) -> bool {
    s.len() <= MAX_APP_ID && shared::view::is_app_id(s)
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub struct wol_event {
    topic: Arc<wol_string>,
    json: Arc<wol_string>,
}

impl wol_event {
    pub fn new(topic: &str, json: &str) -> Self {
        Self {
            topic: wol_string::new(topic),
            json: wol_string::new(json),
        }
    }
}

pub(crate) enum Incoming {
    Event(Arc<wol_event>),
    Kit(FromHost),
}

/// Shared with reader threads: they never see the context itself.
struct Mailbox {
    queue: Mutex<VecDeque<Incoming>>,
    wake_w: OwnedFd,
}

impl Mailbox {
    fn push(&self, m: Incoming) {
        lock(&self.queue).push_back(m);
        // Non-blocking: a full pipe already means "readable".
        // SAFETY: writing one byte from a live buffer to our own pipe.
        unsafe { libc::write(self.wake_w.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
    }
}

pub struct wol_subscription {
    topic: String,
    callback: unsafe extern "C" fn(*const wol_event, UserData),
    user_data: usize,
    active: AtomicBool,
}

pub struct wol_context {
    app_id: String,
    config_home: PathBuf,
    wold: PathBuf,
    kit_path: Option<PathBuf>,
    mailbox: Arc<Mailbox>,
    wake_r: OwnedFd,
    subs: Mutex<Vec<Arc<wol_subscription>>>,
    bus: Mutex<Option<UnixStream>>,
    kit: Mutex<Option<UnixStream>>,
    widgets: Mutex<HashMap<u32, Weak<wol_widget>>>,
    windows: Mutex<HashMap<u32, Weak<wol_window>>>,
    notes: Mutex<HashMap<u32, Weak<wol_notification>>>,
    search: Mutex<Weak<crate::search::wol_search_provider>>,
    endpoints: Mutex<HashMap<String, Weak<crate::message::wol_endpoint>>>,
    next_id: AtomicU32,
}

impl Drop for wol_context {
    fn drop(&mut self) {
        // Ends the reader threads.
        for s in [lock(&self.bus).take(), lock(&self.kit).take()]
            .into_iter()
            .flatten()
        {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

fn pipe() -> Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as c_int; 2];
    // SAFETY: pipe2 writes two fds into the array.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
        return Err(Error::new(
            Status::Internal,
            "libwol could not make its wake-up pipe.",
        ));
    }
    // SAFETY: both fds are fresh and owned by us from here.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

fn spawn_reader(stream: UnixStream, mailbox: Arc<Mailbox>, parse: fn(&str) -> Option<Incoming>) {
    std::thread::spawn(move || {
        let mut r = BufReader::new(stream);
        while let Some(line) = client::read_line(&mut r) {
            if let Some(m) = parse(&line) {
                mailbox.push(m);
            }
        }
    });
}

fn bus_line(line: &str) -> Option<Incoming> {
    let e = Event::parse(line)?;
    Some(Incoming::Event(Arc::new(wol_event::new(
        e.topic(),
        line.trim(),
    ))))
}

fn kit_line(line: &str) -> Option<Incoming> {
    FromHost::parse(line).map(Incoming::Kit)
}

impl wol_context {
    pub fn new(app_id: &str) -> Result<Self> {
        if !is_app_id(app_id) {
            return Err(Error::invalid(
                "An app id is reverse-DNS, like org.example.hello (letters, digits, '.', '-', '_').",
            ));
        }
        let (wake_r, wake_w) = pipe()?;
        Ok(Self {
            app_id: app_id.into(),
            config_home: crate::system::config_home(),
            wold: client::wold_socket(),
            kit_path: client::kit_socket(),
            mailbox: Arc::new(Mailbox {
                queue: Mutex::default(),
                wake_w,
            }),
            wake_r,
            subs: Mutex::default(),
            bus: Mutex::default(),
            kit: Mutex::default(),
            widgets: Mutex::default(),
            windows: Mutex::default(),
            notes: Mutex::default(),
            search: Mutex::default(),
            endpoints: Mutex::default(),
            next_id: AtomicU32::new(1),
        })
    }

    pub fn config_home(&self) -> &Path {
        &self.config_home
    }

    pub fn wold(&self) -> &Path {
        &self.wold
    }

    pub(crate) fn next_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn register(&self, w: &Arc<wol_widget>) {
        let mut m = lock(&self.widgets);
        m.retain(|_, w| w.strong_count() > 0);
        m.insert(w.id(), Arc::downgrade(w));
    }

    pub(crate) fn register_window(&self, w: &Arc<wol_window>) {
        let mut m = lock(&self.windows);
        m.retain(|_, w| w.strong_count() > 0);
        m.insert(w.id(), Arc::downgrade(w));
    }

    /// `id` is wold's notification id.
    pub(crate) fn register_note(&self, id: u32, n: &Arc<wol_notification>) {
        let mut m = lock(&self.notes);
        m.retain(|_, n| n.strong_count() > 0);
        m.insert(id, Arc::downgrade(n));
    }

    pub(crate) fn search_provider(&self) -> Option<Arc<crate::search::wol_search_provider>> {
        lock(&self.search).upgrade()
    }

    pub(crate) fn set_search_provider(&self, p: &Arc<crate::search::wol_search_provider>) {
        *lock(&self.search) = Arc::downgrade(p);
    }

    pub(crate) fn register_endpoint(&self, name: String, e: &Arc<crate::message::wol_endpoint>) {
        let mut m = lock(&self.endpoints);
        m.retain(|_, w| w.strong_count() > 0);
        m.insert(name, Arc::downgrade(e));
    }

    pub(crate) fn ensure_bus(&self) -> Result<()> {
        let mut bus = lock(&self.bus);
        if bus.is_none() {
            let s = client::subscribe(&self.wold).map_err(|_| {
                Error::new(
                    Status::Unavailable,
                    "The WolOS system service is not running.",
                )
            })?;
            spawn_reader(
                s.try_clone()
                    .map_err(|_| Error::new(Status::Internal, "libwol could not read events."))?,
                self.mailbox.clone(),
                bus_line,
            );
            *bus = Some(s);
        }
        Ok(())
    }

    /// One line to the Kit host, connecting first if needed.
    pub(crate) fn send_kit(&self, line: &str) -> Result<()> {
        use std::io::Write;
        let gone = || {
            Error::new(
                Status::Unavailable,
                "The WolOS desktop is not running, so the window cannot open.",
            )
        };
        let mut kit = lock(&self.kit);
        if kit.is_none() {
            let path = self.kit_path.as_ref().ok_or_else(gone)?;
            let s = UnixStream::connect(path).map_err(|_| gone())?;
            let _ = s.set_write_timeout(Some(client::TIMEOUT));
            let hello = ToHost::Hello {
                app: self.app_id.clone(),
            };
            (&s).write_all(hello.to_line().as_bytes())
                .map_err(|_| gone())?;
            spawn_reader(
                s.try_clone().map_err(|_| gone())?,
                self.mailbox.clone(),
                kit_line,
            );
            *kit = Some(s);
        }
        let ok = kit
            .as_mut()
            .is_some_and(|s| s.write_all(line.as_bytes()).is_ok());
        if !ok {
            *kit = None;
            return Err(gone());
        }
        Ok(())
    }

    pub fn subscribe(
        &self,
        topic: &str,
        callback: wol_event_fn,
        user_data: UserData,
    ) -> Result<Arc<wol_subscription>> {
        let callback =
            callback.ok_or_else(|| Error::invalid("A subscription needs a callback."))?;
        if !TOPICS.contains(&topic) {
            return Err(Error::invalid(format!(
                "Unknown topic. Topics: {}.",
                TOPICS.join(", ")
            )));
        }
        self.ensure_bus()?;
        let s = Arc::new(wol_subscription {
            topic: topic.into(),
            callback,
            user_data: user_data as usize,
            active: AtomicBool::new(true),
        });
        lock(&self.subs).push(s.clone());
        Ok(s)
    }

    /// Wait up to `timeout_ms` (-1 = forever) for something to arrive, then run its callbacks.
    pub fn dispatch(&self, timeout_ms: i32) -> Result<()> {
        if timeout_ms < -1 {
            return Err(Error::invalid(
                "The timeout is -1 (wait) or at least 0 milliseconds.",
            ));
        }
        let mut p = libc::pollfd {
            fd: self.wake_r.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        unsafe { libc::poll(&mut p, 1, timeout_ms) };
        let mut sink = [0u8; 256];
        // SAFETY: reading into a live buffer from our non-blocking pipe until it is empty.
        while unsafe {
            libc::read(
                self.wake_r.as_raw_fd(),
                sink.as_mut_ptr().cast(),
                sink.len(),
            )
        } > 0
        {}
        let items: Vec<Incoming> = lock(&self.mailbox.queue).drain(..).collect();
        for item in items {
            match item {
                Incoming::Event(e) if e.topic.as_str() == "search-query" => {
                    if let (Some(Event::SearchQuery { id, text, .. }), Some(p)) =
                        (Event::parse(e.json.as_str()), self.search_provider())
                    {
                        crate::search::asked(&p, id, &text);
                    }
                }
                Incoming::Event(e) if e.topic.as_str() == "message" => {
                    if let Some(Event::Message {
                        id,
                        from,
                        name,
                        body,
                        ..
                    }) = Event::parse(e.json.as_str())
                    {
                        let to = lock(&self.endpoints).get(&name).and_then(Weak::upgrade);
                        if let Some(to) = to {
                            crate::message::delivered(&to, id, &from, &body);
                        }
                    }
                }
                Incoming::Event(e) if e.topic.as_str() == "search-open" => {
                    if let (Some(Event::SearchOpen { key, .. }), Some(p)) =
                        (Event::parse(e.json.as_str()), self.search_provider())
                    {
                        crate::search::opened(&p, &key);
                    }
                }
                Incoming::Event(e) if e.topic.as_str() == "notification-action" => {
                    if let Some(Event::NotificationAction { id, key, .. }) =
                        Event::parse(e.json.as_str())
                    {
                        let n = lock(&self.notes).get(&id).and_then(Weak::upgrade);
                        if let Some(n) = n {
                            notify::pressed(&n, &key);
                        }
                    }
                }
                Incoming::Event(e) => {
                    let subs: Vec<Arc<wol_subscription>> = {
                        let mut all = lock(&self.subs);
                        all.retain(|s| s.active.load(Ordering::Relaxed));
                        all.iter()
                            .filter(|s| s.topic == e.topic.as_str())
                            .cloned()
                            .collect()
                    };
                    for s in subs {
                        if s.active.load(Ordering::Relaxed) {
                            // SAFETY: the app's callback with its own user data, on its thread.
                            unsafe { (s.callback)(Arc::as_ptr(&e), s.user_data as UserData) };
                        }
                    }
                }
                Incoming::Kit(FromHost::Closed { window }) => {
                    let w = lock(&self.windows).get(&window).and_then(Weak::upgrade);
                    if let Some(w) = w {
                        kit::closed(&w);
                    }
                }
                Incoming::Kit(FromHost::Chose {
                    window,
                    source,
                    key,
                }) => {
                    let w = lock(&self.windows).get(&window).and_then(Weak::upgrade);
                    if let Some(w) = w {
                        kit::chosen(&w, source, &key);
                    }
                }
                Incoming::Kit(e) => {
                    let id = match &e {
                        FromHost::Clicked { widget, .. }
                        | FromHost::Edited { widget, .. }
                        | FromHost::Toggled { widget, .. }
                        | FromHost::Slid { widget, .. } => *widget,
                        FromHost::Closed { .. } | FromHost::Chose { .. } => continue,
                    };
                    let w = lock(&self.widgets).get(&id).and_then(Weak::upgrade);
                    if let Some(w) = w {
                        kit::from_host(&w, &e);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn fd(&self) -> c_int {
        self.wake_r.as_raw_fd()
    }
}

/// # Safety
/// `app_id` points at `len` bytes; `out` and `error` are writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_context_create(
    app_id: *const c_char,
    len: usize,
    out: *mut *mut wol_context,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let id = input(app_id, len, MAX_APP_ID, "The app id")?;
        let ctx = wol_context::new(id)?;
        unsafe { *out = into_raw(Arc::new(ctx)) };
        Ok(())
    })
}

/// # Safety
/// `context` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_context_retain(context: *mut wol_context) -> *mut wol_context {
    unsafe { retain(context) }
}

/// # Safety
/// `context` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_context_release(context: *mut wol_context) {
    unsafe { release(context) }
}

/// # Safety
/// `context` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_context_get_fd(context: *const wol_context) -> c_int {
    unsafe { borrow(context) }.map_or(-1, wol_context::fd)
}

/// # Safety
/// `context` is live; `error` is writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_context_dispatch(
    context: *mut wol_context,
    timeout_ms: i32,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || unsafe { borrow(context) }?.dispatch(timeout_ms))
}

/// # Safety
/// `context` is live; `topic` points at `len` bytes; `out` and `error` are writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_subscribe(
    context: *mut wol_context,
    topic: *const c_char,
    len: usize,
    callback: wol_event_fn,
    user_data: *mut c_void,
    out: *mut *mut wol_subscription,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let topic = input(topic, len, 64, "The topic")?;
        let s = ctx.subscribe(topic, callback, user_data)?;
        unsafe { *out = into_raw(s) };
        Ok(())
    })
}

/// # Safety
/// `subscription` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_subscription_cancel(subscription: *mut wol_subscription) {
    if let Ok(s) = unsafe { borrow(subscription) } {
        s.active.store(false, Ordering::Relaxed);
    }
}

/// # Safety
/// `subscription` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_subscription_release(subscription: *mut wol_subscription) {
    unsafe { release(subscription) }
}

/// # Safety
/// `event` is NULL or live. Borrowed from `event`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_event_get_topic(event: *const wol_event) -> *const wol_string {
    unsafe { event.as_ref() }.map_or(std::ptr::null(), |e| Arc::as_ptr(&e.topic))
}

/// # Safety
/// `event` is NULL or live. Borrowed from `event`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_event_get_json(event: *const wol_event) -> *const wol_string {
    unsafe { event.as_ref() }.map_or(std::ptr::null(), |e| Arc::as_ptr(&e.json))
}

/// # Safety
/// `event` is NULL or live (inside a callback, or retained).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_event_retain(event: *const wol_event) -> *const wol_event {
    unsafe { retain(event.cast_mut()) }.cast_const()
}

/// # Safety
/// `event` is NULL or a reference the caller retained.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_event_release(event: *const wol_event) {
    unsafe { release(event.cast_mut()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_id_table() {
        for (id, ok) in [
            ("org.example.hello", true),
            ("io.wolos.Files-2_b", true),
            ("hello", false),
            ("", false),
            (".org.x", false),
            ("org..x", false),
            ("org.x/../y", false),
            ("org.ex ample", false),
        ] {
            assert_eq!(is_app_id(id), ok, "{id}");
        }
        assert!(!is_app_id(&format!("a.{}", "b".repeat(MAX_APP_ID))));
    }

    #[test]
    fn dispatch_runs_callbacks_on_the_calling_thread_only() {
        use std::sync::atomic::AtomicUsize;
        static SEEN: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn on(e: *const wol_event, ud: UserData) {
            let topic = unsafe { (*wol_event_get_topic(e)).as_str().to_owned() };
            assert_eq!(topic, "theme");
            assert_eq!(ud as usize, 7);
            SEEN.fetch_add(1, Ordering::Relaxed);
        }
        let ctx = wol_context::new("org.example.t").unwrap();
        let s = Arc::new(wol_subscription {
            topic: "theme".into(),
            callback: on,
            user_data: 7,
            active: AtomicBool::new(true),
        });
        lock(&ctx.subs).push(s.clone());
        ctx.mailbox
            .push(Incoming::Event(Arc::new(wol_event::new("theme", "{}"))));
        ctx.mailbox
            .push(Incoming::Event(Arc::new(wol_event::new("workspace", "{}"))));
        assert_eq!(
            SEEN.load(Ordering::Relaxed),
            0,
            "nothing runs before dispatch"
        );
        ctx.dispatch(0).unwrap();
        assert_eq!(SEEN.load(Ordering::Relaxed), 1);
        s.active.store(false, Ordering::Relaxed);
        ctx.mailbox
            .push(Incoming::Event(Arc::new(wol_event::new("theme", "{}"))));
        ctx.dispatch(0).unwrap();
        assert_eq!(SEEN.load(Ordering::Relaxed), 1, "cancelled");
        assert!(lock(&ctx.subs).is_empty(), "pruned");
        assert!(ctx.dispatch(-2).is_err());
    }

    #[test]
    fn unknown_topic_and_missing_service() {
        unsafe extern "C" fn on(_: *const wol_event, _: UserData) {}
        let mut ctx = wol_context::new("org.example.t").unwrap();
        ctx.wold = "/nonexistent/wold.sock".into();
        let e = ctx
            .subscribe("notification", Some(on), std::ptr::null_mut())
            .err()
            .unwrap();
        assert_eq!(e.status, Status::InvalidArgument);
        let e = ctx
            .subscribe("theme", None, std::ptr::null_mut())
            .err()
            .unwrap();
        assert_eq!(e.status, Status::InvalidArgument);
        let e = ctx
            .subscribe("theme", Some(on), std::ptr::null_mut())
            .err()
            .unwrap();
        assert_eq!(e.status, Status::Unavailable);
    }
}
