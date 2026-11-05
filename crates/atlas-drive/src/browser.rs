// SPDX-License-Identifier: GPL-2.0-or-later
//! The browser end: launch Atlas with `--remote-debugging-pipe`, multiplex CDP over it, keep the
//! tabs. The pipe is the browser's fds 3 (commands in) and 4 (events and replies out). std cannot
//! place a child's descriptors, so the child is `sh -c 'exec "$0" "$@" 3<&0 4>&1 0</dev/null
//! 1>/dev/null'` with the two pipes as its stdin/stdout: the shell moves them to 3 and 4 and execs
//! the browser, which keeps the crate free of `unsafe` (ratchet row 0). Never a TCP port.
use crate::cdp::{Framer, MAX_BROWSER_MSG, to_pipe};
use crate::page::Page;
use crate::util::{DriveError, Res, Ring, err};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// A reply is Ok(result) or Err((code, message)).
pub type Reply = Result<Value, (i64, String)>;
type Callback = Box<dyn FnOnce(Reply) + Send>;

/// Network bodies kept by the browser for `Network.getResponseBody`, in total (design: 32 MiB).
pub const NETWORK_BODIES_MAX: u64 = 32 * 1024 * 1024;
/// Default time any single CDP call may take.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);

pub enum Waiter {
    Chan(SyncSender<Reply>),
    Raw(Arc<dyn RawSink>, Value),
    Call(Callback),
}

/// Where a pass-through reply or event goes (a client connection).
pub trait RawSink: Send + Sync {
    /// One complete JSON line, no newline. False when the client is gone or too slow.
    fn send(&self, line: String) -> bool;
}

/// A raw event subscriber with filters (`cdp-events`).
pub struct Sub {
    pub sink: Arc<dyn RawSink>,
    pub patterns: Vec<String>,
    pub session: Option<String>,
    pub sent: AtomicU64,
    pub limit: u64,
    pub done: (Mutex<bool>, Condvar),
    pub all_pages: bool,
}

impl Sub {
    fn wants(&self, session: Option<&str>, method: &str) -> bool {
        if let Some(want) = &self.session
            && session != Some(want.as_str())
        {
            return false;
        }
        self.patterns.iter().any(|p| crate::util::glob(p, method))
    }
    pub fn finish(&self) {
        *self.done.0.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.done.1.notify_all();
    }
}

#[derive(Debug, Clone)]
pub struct Download {
    pub guid: String,
    pub url: String,
    pub name: String,
    pub state: String,
    pub received: f64,
    pub total: f64,
}

#[derive(Default)]
struct Tabs {
    list: Vec<Arc<Page>>,
    current: usize,
    attaching: usize,
}

pub struct Inner {
    writer: Mutex<Box<dyn Write + Send>>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, Waiter>>,
    tabs: Mutex<Tabs>,
    tabs_cv: Condvar,
    subs: Mutex<Vec<Arc<Sub>>>,
    pub downloads: Mutex<Ring<Download>>,
    child: Mutex<Option<Child>>,
    pub alive: AtomicBool,
    pub profile: Option<PathBuf>,
    pub profile_is_temp: bool,
    pub download_dir: Option<PathBuf>,
    pub pid: u32,
}

#[derive(Clone)]
pub struct Browser(pub Arc<Inner>);

pub struct Pending {
    rx: Receiver<Reply>,
    id: u64,
    inner: Arc<Inner>,
    method: String,
}

impl Pending {
    /// Wait for the reply; on timeout the entry is removed so nothing leaks.
    pub fn wait(self, d: Duration) -> Res<Value> {
        match self.rx.recv_timeout(d) {
            Ok(r) => reply_to_res(&self.method, r),
            Err(RecvTimeoutError::Timeout) => {
                self.inner.lock_pending().remove(&self.id);
                err(
                    "timeout",
                    format!("{} did not answer in {} ms", self.method, d.as_millis()),
                    "the page may be blocked by a dialog (try: dialog accept) or a long task; raise --timeout",
                )
            }
            Err(RecvTimeoutError::Disconnected) => {
                err("closed", "the browser closed", "run launch again")
            }
        }
    }

    /// Like `wait`, but gives up early (with `Ok(None)`) when `stop` becomes true, checked every
    /// 40 ms while blocked. A reply that arrives is returned at once.
    pub fn wait_or(self, d: Duration, mut stop: impl FnMut() -> bool) -> Res<Option<Value>> {
        let end = Instant::now() + d;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                self.inner.lock_pending().remove(&self.id);
                return err(
                    "timeout",
                    format!("{} did not answer in {} ms", self.method, d.as_millis()),
                    "raise --timeout",
                );
            }
            match self.rx.recv_timeout(left.min(crate::util::WAIT_40MS)) {
                Ok(r) => return reply_to_res(&self.method, r).map(Some),
                Err(RecvTimeoutError::Timeout) => {
                    if stop() {
                        // the reply may still come; leave the waiter, it is dropped with the channel
                        self.inner.lock_pending().remove(&self.id);
                        return Ok(None);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return err("closed", "the browser closed", "run launch again");
                }
            }
        }
    }
}

fn reply_to_res(method: &str, r: Reply) -> Res<Value> {
    r.map_err(|(code, m)| {
        let hint = if m.contains("No node with given id") || m.contains("Could not find node") {
            "the page changed; run snapshot again"
        } else if m.contains("Cannot find context") || m.contains("context was destroyed") {
            "the page navigated; retry"
        } else {
            ""
        };
        DriveError::new("cdp", format!("{method} failed ({code}): {m}"), hint)
    })
}

impl Inner {
    fn lock_pending(&self) -> MutexGuard<'_, HashMap<u64, Waiter>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn lock_tabs(&self) -> MutexGuard<'_, Tabs> {
        self.tabs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub struct LaunchConfig {
    pub binary: PathBuf,
    pub headless: bool,
    pub profile: Option<PathBuf>,
    pub extra_args: Vec<String>,
    pub download_dir: Option<PathBuf>,
    pub no_sandbox: bool,
    pub disable_gpu: bool,
    pub url: String,
    pub log: Option<PathBuf>,
}

impl Default for LaunchConfig {
    fn default() -> Self {
        Self {
            binary: PathBuf::from(shared::atlas::BROWSER),
            headless: true,
            profile: None,
            extra_args: vec![],
            download_dir: None,
            no_sandbox: false,
            disable_gpu: false,
            url: "about:blank".into(),
            log: None,
        }
    }
}

/// The arguments of the browser, in one place and under test: the pipe, never a port.
pub fn browser_args(c: &LaunchConfig, profile: &std::path::Path) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "--remote-debugging-pipe".into(),
        format!("--user-data-dir={}", profile.display()),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--password-store=basic".into(),
        "--disable-background-networking".into(),
        // A page left for another stays alive in the back/forward cache, in the same renderer when
        // the site is the same: measured (docs/bench/atlas-drive.md, round 2) it kept the browser
        // 20 % bigger and made every later click wait 4 times longer. A driven browser reloads
        // on `back` instead (Playwright passes the same flag).
        "--disable-back-forward-cache".into(),
    ];
    if c.headless {
        a.push("--headless=new".into());
        a.push("--hide-scrollbars".into());
    } else {
        // a window the person can see says so itself: the engine's "controlled by automated test software" bar
        a.push("--enable-automation".into());
    }
    if c.no_sandbox {
        a.push("--no-sandbox".into());
    }
    if c.disable_gpu {
        a.push("--disable-gpu".into());
    }
    a.extend(c.extra_args.iter().cloned());
    a.push(c.url.clone());
    a
}

/// Refuse any argument that opens a network debugging door (the product never passes one).
pub fn forbid_debug_port(args: &[String]) -> Res<()> {
    for a in args {
        if a.starts_with("--remote-debugging-port")
            || a.starts_with("--remote-debugging-address")
            || a.starts_with("--remote-allow-origins")
        {
            return err(
                "forbidden",
                format!("{a} opens a TCP debugging port"),
                "Atlas Drive uses the pipe only; remove the flag",
            );
        }
    }
    Ok(())
}

impl Browser {
    pub fn launch(c: LaunchConfig) -> Res<Browser> {
        forbid_debug_port(&c.extra_args)?;
        if !c.binary.exists() {
            return err(
                "no_browser",
                format!("{} does not exist", c.binary.display()),
                "pass --browser <path to the atlas binary>",
            );
        }
        let (profile, temp) = match &c.profile {
            Some(p) => (p.clone(), false),
            None => {
                let d = std::env::temp_dir().join(format!(
                    "atlas-drive-{}-{}",
                    std::process::id(),
                    crate::util::now_ms()
                ));
                (d, true)
            }
        };
        let args = browser_args(&c, &profile);
        let stderr: Stdio = match &c.log {
            Some(p) => std::fs::File::create(p)
                .map(Stdio::from)
                .unwrap_or_else(|_| Stdio::null()),
            None => Stdio::null(),
        };
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(r#"exec "$0" "$@" 3<&0 4>&1 0</dev/null 1>/dev/null"#)
            .arg(&c.binary)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .map_err(|e| {
                DriveError::new(
                    "launch",
                    format!("cannot start the browser: {e}"),
                    "check --browser",
                )
            })?;
        let w = child
            .stdin
            .take()
            .ok_or_else(|| DriveError::new("launch", "no pipe", ""))?;
        let r = child
            .stdout
            .take()
            .ok_or_else(|| DriveError::new("launch", "no pipe", ""))?;
        let pid = child.id();
        let b = Browser::from_streams(
            Box::new(r),
            Box::new(w),
            Some(child),
            Some(profile),
            temp,
            c.download_dir.clone(),
            pid,
        );
        b.bootstrap()?;
        Ok(b)
    }

    /// Wire a browser from two streams (the pipe, or a test double).
    pub fn from_streams(
        r: Box<dyn Read + Send>,
        w: Box<dyn Write + Send>,
        child: Option<Child>,
        profile: Option<PathBuf>,
        profile_is_temp: bool,
        download_dir: Option<PathBuf>,
        pid: u32,
    ) -> Browser {
        let inner = Arc::new(Inner {
            writer: Mutex::new(w),
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            tabs: Mutex::new(Tabs::default()),
            tabs_cv: Condvar::new(),
            subs: Mutex::new(vec![]),
            downloads: Mutex::new(Ring::new(100)),
            child: Mutex::new(child),
            alive: AtomicBool::new(true),
            profile,
            profile_is_temp,
            download_dir,
            pid,
        });
        let me = Browser(inner.clone());
        let reader = me.clone();
        let spawned = std::thread::Builder::new()
            .name("cdp-reader".into())
            .stack_size(1024 * 1024)
            .spawn(move || reader.read_loop(r));
        if spawned.is_err() {
            // no reader, no browser: every verb now answers "the browser closed" instead of hanging
            me.0.alive.store(false, Ordering::Relaxed);
        }
        me
    }

    fn bootstrap(&self) -> Res<()> {
        // discovery reports the initial tab; each page target is attached in on_event
        self.send(None, "Target.setDiscoverTargets", json!({"discover": true}))?
            .wait(CALL_TIMEOUT)?;
        if let Some(dir) = &self.0.download_dir {
            let _ = std::fs::create_dir_all(dir);
            let _ = self.send(
                None,
                "Browser.setDownloadBehavior",
                json!({"behavior":"allow","downloadPath":dir.display().to_string(),"eventsEnabled":true}),
            );
        }
        self.wait_first_page(crate::util::WAIT_20S)?;
        Ok(())
    }

    pub fn wait_first_page(&self, d: Duration) -> Res<Arc<Page>> {
        let end = Instant::now() + d;
        let mut t = self.0.lock_tabs();
        loop {
            if let Some(p) = t.list.get(t.current).cloned() {
                return Ok(p);
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() || !self.0.alive.load(Ordering::Relaxed) {
                return err(
                    "launch",
                    "the browser opened no tab",
                    "check the browser log with --log",
                );
            }
            t = self
                .0
                .tabs_cv
                .wait_timeout(t, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    // ---------------------------------------------------------------- sending

    fn write(&self, text: &str) -> Res<()> {
        let mut w = self.0.writer.lock().unwrap_or_else(PoisonError::into_inner);
        w.write_all(&to_pipe(text))
            .and_then(|_| w.flush())
            .map_err(|e| {
                DriveError::new(
                    "closed",
                    format!("the browser pipe is closed: {e}"),
                    "run launch again",
                )
            })
    }

    fn register(&self, w: Waiter) -> u64 {
        let id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        self.0.lock_pending().insert(id, w);
        id
    }

    fn envelope(id: u64, session: Option<&str>, method: &str, params: &Value) -> String {
        let mut m = serde_json::Map::new();
        m.insert("id".into(), id.into());
        if let Some(s) = session {
            m.insert("sessionId".into(), s.into());
        }
        m.insert("method".into(), method.into());
        if !params.is_null() {
            m.insert("params".into(), params.clone());
        }
        Value::Object(m).to_string()
    }

    /// Send a command; the reply is collected with [`Pending::wait`]. Several can be in flight.
    pub fn send(&self, session: Option<&str>, method: &str, params: Value) -> Res<Pending> {
        let (tx, rx) = sync_channel(1);
        let id = self.register(Waiter::Chan(tx));
        let text = Self::envelope(id, session, method, &params);
        if let Err(e) = self.write(&text) {
            self.0.lock_pending().remove(&id);
            return Err(e);
        }
        Ok(Pending {
            rx,
            id,
            inner: self.0.clone(),
            method: method.to_string(),
        })
    }

    /// Send and wait.
    pub fn call(&self, session: Option<&str>, method: &str, params: Value) -> Res<Value> {
        self.send(session, method, params)?.wait(CALL_TIMEOUT)
    }

    /// Send without waiting for or keeping the reply.
    pub fn fire(&self, session: Option<&str>, method: &str, params: Value) {
        let _ = self.send_cb(session, method, params, Box::new(|_| {}));
    }

    pub fn send_cb(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
        cb: Callback,
    ) -> Res<()> {
        let id = self.register(Waiter::Call(cb));
        let text = Self::envelope(id, session, method, &params);
        if let Err(e) = self.write(&text) {
            self.0.lock_pending().remove(&id);
            return Err(e);
        }
        Ok(())
    }

    /// Pass a client's raw command through: ids are rewritten, the reply goes back to `sink`
    /// with the client's own id.
    pub fn forward(&self, sink: Arc<dyn RawSink>, mut msg: Value) -> Res<()> {
        let orig = msg.get("id").cloned().unwrap_or(Value::Null);
        let id = self.register(Waiter::Raw(sink, orig));
        msg["id"] = id.into();
        if let Err(e) = self.write(&msg.to_string()) {
            self.0.lock_pending().remove(&id);
            return Err(e);
        }
        Ok(())
    }

    pub fn subscribe(&self, s: Arc<Sub>) {
        self.0
            .subs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(s);
    }

    pub fn unsubscribe(&self, s: &Arc<Sub>) {
        self.0
            .subs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|x| !Arc::ptr_eq(x, s));
    }

    // ---------------------------------------------------------------- tabs

    pub fn tabs(&self) -> Vec<Arc<Page>> {
        self.0.lock_tabs().list.clone()
    }

    pub fn current(&self) -> Option<Arc<Page>> {
        let t = self.0.lock_tabs();
        t.list.get(t.current).cloned()
    }

    pub fn current_index(&self) -> usize {
        self.0.lock_tabs().current
    }

    pub fn select(&self, idx: usize) -> Res<Arc<Page>> {
        let mut t = self.0.lock_tabs();
        match t.list.get(idx).cloned() {
            Some(p) => {
                t.current = idx;
                Ok(p)
            }
            None => err(
                "no_tab",
                format!("no tab {idx}"),
                format!("tabs are 0..{}", t.list.len().saturating_sub(1)),
            ),
        }
    }

    pub fn page_by_session(&self, s: &str) -> Option<Arc<Page>> {
        self.0
            .lock_tabs()
            .list
            .iter()
            .find(|p| p.session == s)
            .cloned()
    }

    /// Open a tab and return it once attached (becomes the current tab).
    pub fn new_tab(&self, url: &str) -> Res<Arc<Page>> {
        let before = self.0.lock_tabs().list.len();
        let r = self.call(None, "Target.createTarget", json!({"url": url}))?;
        let tid = r["targetId"].as_str().unwrap_or("").to_string();
        let end = Instant::now() + crate::util::WAIT_10S;
        let mut t = self.0.lock_tabs();
        loop {
            if let Some(i) = t.list.iter().position(|p| p.target_id == tid) {
                t.current = i;
                return Ok(t.list[i].clone());
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return err(
                    "no_tab",
                    format!("tab {tid} did not attach (had {before})"),
                    "retry",
                );
            }
            t = self
                .0
                .tabs_cv
                .wait_timeout(t, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    pub fn close_tab(&self, idx: usize) -> Res<()> {
        let p = {
            let t = self.0.lock_tabs();
            t.list.get(idx).cloned()
        };
        let Some(p) = p else {
            return err("no_tab", format!("no tab {idx}"), "run tabs");
        };
        self.call(None, "Target.closeTarget", json!({"targetId": p.target_id}))?;
        // the destroyed event follows the reply: wait for it so the next verb does not pick the dead tab
        let end = Instant::now() + crate::util::WAIT_3S;
        let mut t = self.0.lock_tabs();
        while t.list.iter().any(|x| x.target_id == p.target_id) {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            t = self
                .0
                .tabs_cv
                .wait_timeout(t, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- lifetime

    pub fn is_alive(&self) -> bool {
        self.0.alive.load(Ordering::Relaxed)
    }

    /// Close the browser and remove a temporary profile.
    pub fn close(&self) {
        if self.is_alive() {
            let _ = self
                .send(None, "Browser.close", Value::Null)
                .map(|p| p.wait(crate::util::WAIT_3S));
        }
        let child = self
            .0
            .child
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(mut c) = child {
            let end = Instant::now() + crate::util::WAIT_3S;
            loop {
                match c.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < end => std::thread::sleep(crate::util::WAIT_20MS),
                    _ => {
                        let _ = c.kill();
                        let _ = c.wait();
                        break;
                    }
                }
            }
        }
        if self.0.profile_is_temp
            && let Some(p) = &self.0.profile
        {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    // ---------------------------------------------------------------- reading

    fn read_loop(&self, r: Box<dyn Read + Send>) {
        let mut f = Framer::new(r, 0, MAX_BROWSER_MSG);
        while let Ok(Some(raw)) = f.next_msg() {
            let t_parse = Instant::now();
            let Ok(v) = serde_json::from_slice::<Value>(&raw) else {
                continue;
            };
            if raw.len() > 1_000_000 && crate::verbs::trace_timing() {
                eprintln!(
                    "atlas-drive: reply of {} KB parsed in {:.1} ms",
                    raw.len() / 1024,
                    t_parse.elapsed().as_secs_f64() * 1000.0
                );
            }
            if let Some(id) = v.get("id").and_then(Value::as_u64) {
                let w = self.0.lock_pending().remove(&id);
                let reply: Reply = match v.get("error") {
                    Some(e) => Err((
                        e["code"].as_i64().unwrap_or(0),
                        e["message"].as_str().unwrap_or("error").to_string(),
                    )),
                    None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                };
                match w {
                    Some(Waiter::Chan(tx)) => {
                        let _ = tx.try_send(reply);
                    }
                    Some(Waiter::Call(cb)) => cb(reply),
                    Some(Waiter::Raw(sink, orig)) => {
                        let mut out = v;
                        out["id"] = orig;
                        if let Some(o) = out.as_object_mut() {
                            // keep the session so a multiplexed client can tell its replies apart
                            let _ = o;
                        }
                        sink.send(out.to_string());
                    }
                    None => {}
                }
            } else if let Some(method) = v.get("method").and_then(Value::as_str) {
                let session = v.get("sessionId").and_then(Value::as_str);
                self.on_event(session, method, &v["params"], &raw);
            }
        }
        // the browser is gone: fail every waiter, wake every waiter of a page
        self.0.alive.store(false, Ordering::Relaxed);
        let pend: Vec<Waiter> = self.0.lock_pending().drain().map(|(_, w)| w).collect();
        for w in pend {
            match w {
                Waiter::Chan(_) => {}
                Waiter::Call(cb) => cb(Err((-1, "the browser closed".into()))),
                Waiter::Raw(sink, orig) => {
                    sink.send(json!({"id": orig, "error": {"code": -32000, "message": "the browser closed"}}).to_string());
                }
            }
        }
        for p in self.tabs() {
            p.close();
        }
        for s in self
            .0
            .subs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            s.finish();
        }
        self.0.tabs_cv.notify_all();
    }

    fn on_event(&self, session: Option<&str>, method: &str, params: &Value, raw: &[u8]) {
        match method {
            "Target.targetCreated" => {
                let t = &params["targetInfo"];
                if t["type"] == "page"
                    && !t["url"].as_str().unwrap_or("").starts_with("devtools://")
                {
                    self.attach(t);
                }
            }
            "Target.targetDestroyed" => {
                let tid = params["targetId"].as_str().unwrap_or("");
                let mut t = self.0.lock_tabs();
                if let Some(i) = t.list.iter().position(|p| p.target_id == tid) {
                    let p = t.list.remove(i);
                    p.close();
                    if t.current >= t.list.len() {
                        t.current = t.list.len().saturating_sub(1);
                    }
                }
                drop(t);
                self.0.tabs_cv.notify_all();
            }
            "Target.targetInfoChanged" => {
                let tid = params["targetInfo"]["targetId"].as_str().unwrap_or("");
                if let Some(p) = self
                    .0
                    .lock_tabs()
                    .list
                    .iter()
                    .find(|p| p.target_id == tid)
                    .cloned()
                {
                    p.on_event(method, params);
                }
            }
            "Browser.downloadWillBegin" => {
                let d = Download {
                    guid: params["guid"].as_str().unwrap_or("").into(),
                    url: params["url"].as_str().unwrap_or("").into(),
                    name: params["suggestedFilename"].as_str().unwrap_or("").into(),
                    state: "inProgress".into(),
                    received: 0.0,
                    total: 0.0,
                };
                self.0
                    .downloads
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(d);
                for p in self.tabs() {
                    p.lock().downloads_seen += 1;
                    p.cv.notify_all();
                }
            }
            "Browser.downloadProgress" => {
                let g = params["guid"].as_str().unwrap_or("");
                let mut ds = self
                    .0
                    .downloads
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if let Some(d) = ds.items.iter_mut().find(|d| d.guid == g) {
                    d.state = params["state"].as_str().unwrap_or("").into();
                    d.received = params["receivedBytes"].as_f64().unwrap_or(0.0);
                    d.total = params["totalBytes"].as_f64().unwrap_or(0.0);
                }
                drop(ds);
                for p in self.tabs() {
                    p.cv.notify_all();
                }
            }
            _ => {
                if let Some(s) = session
                    && let Some(p) = self.page_by_session(s)
                {
                    p.on_event(method, params);
                }
            }
        }
        // raw subscribers (cdp-events, raw clients)
        let subs: Vec<Arc<Sub>> = self
            .0
            .subs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if subs.is_empty() {
            return;
        }
        let mut text: Option<String> = None;
        for s in subs {
            if !s.wants(session, method) {
                continue;
            }
            let t = text.get_or_insert_with(|| String::from_utf8_lossy(raw).into_owned());
            let n = s.sent.fetch_add(1, Ordering::Relaxed) + 1;
            if !s.sink.send(t.clone()) || (s.limit > 0 && n >= s.limit) {
                s.finish();
            }
        }
    }

    /// Attach to a page target (flatten sessions) and switch on the domains the verbs rely on.
    fn attach(&self, t: &Value) {
        let tid = t["targetId"].as_str().unwrap_or("").to_string();
        let url = t["url"].as_str().unwrap_or("").to_string();
        let title = t["title"].as_str().unwrap_or("").to_string();
        if self.0.lock_tabs().list.iter().any(|p| p.target_id == tid) {
            return;
        }
        self.0.lock_tabs().attaching += 1;
        let me = self.clone();
        let tid2 = tid.clone();
        let _ = self.send_cb(
            None,
            "Target.attachToTarget",
            json!({"targetId": tid, "flatten": true}),
            Box::new(move |r| {
                let Ok(v) = r else {
                    me.0.lock_tabs().attaching -= 1;
                    return;
                };
                let sid = v["sessionId"].as_str().unwrap_or("").to_string();
                let page = Arc::new(Page::new(&tid2, &sid, &url, &title));
                // enables are pipelined; the browser runs them in order before any later verb
                let s = Some(sid.as_str());
                me.fire(s, "Page.enable", Value::Null);
                me.fire(s, "Page.setLifecycleEventsEnabled", json!({"enabled": true}));
                me.fire(s, "Runtime.enable", Value::Null);
                me.fire(
                    s,
                    "Network.enable",
                    json!({"maxTotalBufferSize": NETWORK_BODIES_MAX, "maxResourceBufferSize": 8 * 1024 * 1024, "maxPostDataSize": 65536}),
                );
                me.fire(s, "Log.enable", Value::Null);
                let mut tabs = me.0.lock_tabs();
                tabs.attaching -= 1;
                tabs.list.push(page);
                drop(tabs);
                me.0.tabs_cv.notify_all();
            }),
        );
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(mut c) = self
            .child
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_use_the_pipe_and_never_a_port() {
        let c = LaunchConfig {
            headless: true,
            no_sandbox: true,
            ..Default::default()
        };
        let a = browser_args(&c, std::path::Path::new("/p"));
        assert!(a.contains(&"--remote-debugging-pipe".to_string()));
        assert!(a.iter().all(|x| !x.contains("remote-debugging-port")));
        assert!(a.contains(&"--user-data-dir=/p".to_string()));
        assert_eq!(a.last().unwrap(), "about:blank");
        assert!(!a.contains(&"--enable-automation".to_string()));
        assert!(a.contains(&"--disable-back-forward-cache".to_string()));
        let headed = LaunchConfig {
            headless: false,
            ..Default::default()
        };
        let a = browser_args(&headed, std::path::Path::new("/p"));
        assert!(
            a.contains(&"--enable-automation".to_string())
                && !a.iter().any(|x| x.starts_with("--headless"))
        );
    }

    #[test]
    fn a_debugging_port_flag_is_refused() {
        let bad = vec!["--remote-debugging-port=9222".to_string()];
        assert_eq!(forbid_debug_port(&bad).unwrap_err().code, "forbidden");
        assert!(forbid_debug_port(&["--lang=en".to_string()]).is_ok());
    }
}
