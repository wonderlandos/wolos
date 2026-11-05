// SPDX-License-Identifier: GPL-2.0-or-later
//! One tab: the state the daemon keeps from CDP events so that a verb can answer "what changed"
//! without asking the browser again. Every collection is a bounded ring (no unbounded Vec).
use crate::util::Ring;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

pub const CONSOLE_CAP: usize = 200;
pub const REQUEST_CAP: usize = 1000;
pub const FAILURE_CAP: usize = 200;
pub const WS_CAP: usize = 500;
pub const SCRIPT_CAP: usize = 4000;
const LOADER_CAP: usize = 64;

#[derive(Debug, Clone)]
pub struct ConsoleEntry {
    pub level: String,
    pub text: String,
    pub url: String,
    pub line: i64,
}

#[derive(Debug, Clone)]
pub struct ReqLog {
    pub id: String,
    pub method: String,
    pub url: String,
    pub kind: String,
    pub status: Option<i64>,
    pub mime: String,
    pub size: Option<f64>,
    pub failed: Option<String>,
    pub done: bool,
    pub started_ms: u64,
    pub headers: Value,
    pub post: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WsFrame {
    pub url: String,
    pub sent: bool,
    pub op: i64,
    pub data: String,
}

#[derive(Debug, Clone)]
pub struct Dialog {
    pub kind: String,
    pub message: String,
    pub default: String,
}

#[derive(Debug, Clone)]
pub struct ScriptInfo {
    pub url: String,
    pub source_map: String,
    pub len: i64,
    pub start_line: i64,
}

/// What the agent sees change between two points.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub url: String,
    pub title: String,
    pub console_errors: u64,
    pub net_failures: u64,
    pub dialogs: u64,
    pub nav_gen: u64,
    pub downloads: u64,
    pub loads: u64,
}

#[derive(Default)]
pub struct PState {
    pub url: String,
    pub title: String,
    pub main_frame: String,
    pub loader: String,
    pub loading: bool,
    pub nav_gen: u64,
    /// Main-frame `load` events seen, whatever their loader id.
    pub loads: u64,
    pub loaded: Vec<String>,
    pub dcl: Vec<String>,
    pub inflight: HashMap<String, ()>,
    pub last_net: Option<Instant>,
    pub console_errors: u64,
    pub console: Option<Ring<ConsoleEntry>>,
    pub net_failures: u64,
    pub failures: Option<Ring<String>>,
    pub requests: Option<Ring<ReqLog>>,
    pub ws: Option<Ring<WsFrame>>,
    pub dialog: Option<Dialog>,
    pub dialogs_seen: u64,
    pub closed: bool,
    pub paused: Option<Value>,
    pub scripts: HashMap<String, ScriptInfo>,
    pub debugger_on: bool,
    pub frame_last: Option<Instant>,
    pub frames: u64,
    pub screencast: bool,
    pub downloads_seen: u64,
}

pub struct Page {
    pub target_id: String,
    pub session: String,
    pub st: Mutex<PState>,
    pub cv: Condvar,
    /// Refs, snapshot memory: touched only by verbs, never by the event thread.
    pub agent: Mutex<crate::agent::AgentState>,
    pub cursor: Mutex<crate::cursor::Cursor>,
}

impl Page {
    pub fn new(target_id: &str, session: &str, url: &str, title: &str) -> Self {
        let st = PState {
            url: url.into(),
            title: title.into(),
            last_net: Instant::now().checked_sub(crate::util::WAIT_2S),
            console: Some(Ring::new(CONSOLE_CAP)),
            failures: Some(Ring::new(FAILURE_CAP)),
            requests: Some(Ring::new(REQUEST_CAP)),
            ws: Some(Ring::new(WS_CAP)),
            ..Default::default()
        };
        Self {
            target_id: target_id.into(),
            session: session.into(),
            st: Mutex::new(st),
            cv: Condvar::new(),
            agent: Mutex::new(Default::default()),
            cursor: Mutex::new(Default::default()),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, PState> {
        self.st.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn mark(&self) -> Mark {
        let s = self.lock();
        Mark {
            url: s.url.clone(),
            title: s.title.clone(),
            console_errors: s.console_errors,
            net_failures: s.net_failures,
            dialogs: s.dialogs_seen,
            nav_gen: s.nav_gen,
            downloads: s.downloads_seen,
            loads: s.loads,
        }
    }

    /// Wait until `cond` holds on the state, or the deadline. Returns whether it held.
    pub fn wait_until(&self, deadline: Instant, mut cond: impl FnMut(&PState) -> bool) -> bool {
        let mut s = self.lock();
        loop {
            if cond(&s) {
                return true;
            }
            if s.closed {
                return false;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            s = self
                .cv
                .wait_timeout(s, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// The network has had `quiet` of no request in flight (Playwright's networkidle is 500 ms).
    pub fn wait_network_idle(&self, quiet: Duration, deadline: Instant) -> bool {
        let mut s = self.lock();
        loop {
            let now = Instant::now();
            if s.closed {
                return false;
            }
            if s.inflight.is_empty() {
                let since = s.last_net.unwrap_or(now);
                if now.duration_since(since) >= quiet {
                    return true;
                }
                let until = (since + quiet).min(deadline);
                if until <= now {
                    return false;
                }
                s = self
                    .cv
                    .wait_timeout(s, until - now)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            } else {
                if now >= deadline {
                    return false;
                }
                s = self
                    .cv
                    .wait_timeout(s, deadline - now)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
        }
    }

    /// Apply one CDP event of this page's session.
    pub fn on_event(&self, method: &str, p: &Value) {
        let mut s = self.lock();
        let str_of =
            |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        match method {
            "Page.frameNavigated" => {
                let f = &p["frame"];
                if f.get("parentId").is_none() {
                    s.url = str_of(f, "url");
                    s.main_frame = str_of(f, "id");
                    s.loader = str_of(f, "loaderId");
                    s.nav_gen += 1;
                    s.loading = true;
                    s.paused = None;
                    // script ids belong to a document; the debugger reports the new ones as they parse
                    s.scripts.clear();
                }
            }
            "Page.navigatedWithinDocument" => {
                if str_of(p, "frameId") == s.main_frame || s.main_frame.is_empty() {
                    s.url = str_of(p, "url");
                }
            }
            "Page.frameStartedLoading" => {
                if str_of(p, "frameId") == s.main_frame || s.main_frame.is_empty() {
                    s.loading = true;
                }
            }
            "Page.frameStoppedLoading" => {
                if str_of(p, "frameId") == s.main_frame {
                    s.loading = false;
                }
            }
            "Page.lifecycleEvent" => {
                let name = str_of(p, "name");
                let loader = str_of(p, "loaderId");
                match name.as_str() {
                    "load" => {
                        if s.loaded.len() >= LOADER_CAP {
                            s.loaded.clear();
                        }
                        s.loaded.push(loader);
                        if str_of(p, "frameId") == s.main_frame || s.main_frame.is_empty() {
                            s.loading = false;
                            s.loads += 1;
                        }
                    }
                    "DOMContentLoaded" => {
                        if s.dcl.len() >= LOADER_CAP {
                            s.dcl.clear();
                        }
                        s.dcl.push(loader);
                    }
                    _ => {}
                }
            }
            "Target.targetInfoChanged" => {
                let t = &p["targetInfo"];
                if str_of(t, "targetId") == self.target_id {
                    s.title = str_of(t, "title");
                    let u = str_of(t, "url");
                    if !u.is_empty() {
                        s.url = u;
                    }
                }
            }
            "Network.requestWillBeSent" => {
                let id = str_of(p, "requestId");
                let kind = str_of(p, "type");
                if kind != "EventSource" && kind != "WebSocket" && kind != "Ping" {
                    s.inflight.insert(id.clone(), ());
                }
                s.last_net = Some(Instant::now());
                let r = &p["request"];
                let url = str_of(r, "url");
                let log = ReqLog {
                    id: id.clone(),
                    method: str_of(r, "method"),
                    url: crate::util::clip(&url, 600).0.to_string(),
                    kind,
                    status: None,
                    mime: String::new(),
                    size: None,
                    failed: None,
                    done: false,
                    started_ms: crate::util::now_ms(),
                    headers: r.get("headers").cloned().unwrap_or(Value::Null),
                    post: r
                        .get("postData")
                        .and_then(Value::as_str)
                        .map(|t| crate::util::clip(t, 4096).0.to_string()),
                };
                // a redirect re-uses the id: the older entry is closed by its hop
                if let Some(rq) = s.requests.as_mut() {
                    rq.push(log);
                }
            }
            "Network.responseReceived" => {
                let id = str_of(p, "requestId");
                let status = p["response"]["status"].as_i64();
                let mime = str_of(&p["response"], "mimeType");
                if let Some(rq) = s.requests.as_mut()
                    && let Some(r) = rq.items.iter_mut().rev().find(|r| r.id == id)
                {
                    r.status = status;
                    r.mime = mime;
                }
            }
            "Network.loadingFinished" => {
                let id = str_of(p, "requestId");
                s.inflight.remove(&id);
                s.last_net = Some(Instant::now());
                let size = p["encodedDataLength"].as_f64();
                if let Some(rq) = s.requests.as_mut()
                    && let Some(r) = rq.items.iter_mut().rev().find(|r| r.id == id)
                {
                    r.done = true;
                    r.size = size;
                }
            }
            "Network.loadingFailed" => {
                let id = str_of(p, "requestId");
                s.inflight.remove(&id);
                s.last_net = Some(Instant::now());
                let err = str_of(p, "errorText");
                let canceled = p["canceled"].as_bool().unwrap_or(false);
                let mut url = String::new();
                if let Some(rq) = s.requests.as_mut()
                    && let Some(r) = rq.items.iter_mut().rev().find(|r| r.id == id)
                {
                    r.done = true;
                    r.failed = Some(err.clone());
                    url = r.url.clone();
                }
                if !canceled {
                    s.net_failures += 1;
                    let line = format!("{err} {url}");
                    if let Some(f) = s.failures.as_mut() {
                        f.push(line);
                    }
                }
            }
            "Network.webSocketFrameSent" | "Network.webSocketFrameReceived" => {
                let sent = method.ends_with("Sent");
                let fr = &p["response"];
                let data = str_of(fr, "payloadData");
                let url = {
                    let id = str_of(p, "requestId");
                    s.requests
                        .as_ref()
                        .and_then(|rq| {
                            rq.items
                                .iter()
                                .rev()
                                .find(|r| r.id == id)
                                .map(|r| r.url.clone())
                        })
                        .unwrap_or_default()
                };
                let w = WsFrame {
                    url,
                    sent,
                    op: fr["opcode"].as_i64().unwrap_or(1),
                    data: crate::util::clip(&data, 2048).0.to_string(),
                };
                if let Some(ws) = s.ws.as_mut() {
                    ws.push(w);
                }
            }
            "Network.webSocketCreated" => {
                let id = str_of(p, "requestId");
                let url = str_of(p, "url");
                let log = ReqLog {
                    id,
                    method: "GET".into(),
                    url,
                    kind: "WebSocket".into(),
                    status: Some(101),
                    mime: String::new(),
                    size: None,
                    failed: None,
                    done: false,
                    started_ms: crate::util::now_ms(),
                    headers: Value::Null,
                    post: None,
                };
                if let Some(rq) = s.requests.as_mut() {
                    rq.push(log);
                }
            }
            "Runtime.consoleAPICalled" => {
                let level = str_of(p, "type");
                if level == "error" || level == "assert" {
                    s.console_errors += 1;
                }
                let text = p["args"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|v| {
                                v.get("value")
                                    .map(|x| match x {
                                        Value::String(t) => t.clone(),
                                        o => o.to_string(),
                                    })
                                    .or_else(|| {
                                        v.get("description")
                                            .and_then(Value::as_str)
                                            .map(String::from)
                                    })
                                    .unwrap_or_default()
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                let frame = &p["stackTrace"]["callFrames"][0];
                let e = ConsoleEntry {
                    level,
                    text: crate::util::clip(&text, 500).0.to_string(),
                    url: str_of(frame, "url"),
                    line: frame["lineNumber"].as_i64().unwrap_or(0) + 1,
                };
                if let Some(c) = s.console.as_mut() {
                    c.push(e);
                }
            }
            "Runtime.exceptionThrown" => {
                s.console_errors += 1;
                let d = &p["exceptionDetails"];
                let text = d["exception"]["description"]
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| str_of(d, "text"));
                let first = text.lines().next().unwrap_or("").to_string();
                let e = ConsoleEntry {
                    level: "exception".into(),
                    text: crate::util::clip(&first, 500).0.to_string(),
                    url: str_of(d, "url"),
                    line: d["lineNumber"].as_i64().unwrap_or(0) + 1,
                };
                if let Some(c) = s.console.as_mut() {
                    c.push(e);
                }
            }
            "Log.entryAdded" => {
                let en = &p["entry"];
                let level = str_of(en, "level");
                // network failures are counted from Network.loadingFailed; Log duplicates them
                if level == "error" && str_of(en, "source") != "network" {
                    s.console_errors += 1;
                }
                let e = ConsoleEntry {
                    level: format!("log-{level}"),
                    text: crate::util::clip(&str_of(en, "text"), 500).0.to_string(),
                    url: str_of(en, "url"),
                    line: en["lineNumber"].as_i64().unwrap_or(0) + 1,
                };
                if let Some(c) = s.console.as_mut() {
                    c.push(e);
                }
            }
            "Page.javascriptDialogOpening" => {
                s.dialogs_seen += 1;
                s.dialog = Some(Dialog {
                    kind: str_of(p, "type"),
                    message: crate::util::clip(&str_of(p, "message"), 300).0.to_string(),
                    default: str_of(p, "defaultPrompt"),
                });
            }
            "Page.javascriptDialogClosed" => s.dialog = None,
            "Page.screencastFrame" => {
                s.frames += 1;
                s.frame_last = Some(Instant::now());
            }
            "Debugger.scriptParsed" | "Debugger.scriptFailedToParse" => {
                if s.scripts.len() < SCRIPT_CAP {
                    let id = str_of(p, "scriptId");
                    s.scripts.insert(
                        id,
                        ScriptInfo {
                            url: str_of(p, "url"),
                            source_map: str_of(p, "sourceMapURL"),
                            len: p["length"].as_i64().unwrap_or(0),
                            start_line: p["startLine"].as_i64().unwrap_or(0),
                        },
                    );
                }
            }
            "Debugger.paused" => s.paused = Some(p.clone()),
            "Debugger.resumed" => s.paused = None,
            _ => {}
        }
        drop(s);
        self.cv.notify_all();
    }

    pub fn close(&self) {
        self.lock().closed = true;
        self.cv.notify_all();
    }
}

/// The page's current change summary against a mark: only what differs, as a compact object.
pub fn delta(page: &Page, was: &Mark) -> Value {
    let now = page.mark();
    let mut o = serde_json::Map::new();
    if now.url != was.url {
        o.insert("url".into(), now.url.clone().into());
    }
    // until the page names itself the browser's title is the address: not a title
    let named = !now.title.is_empty() && !now.url.ends_with(&now.title);
    if now.title != was.title && named {
        o.insert("title".into(), now.title.clone().into());
    }
    if now.nav_gen != was.nav_gen {
        o.insert("navigated".into(), true.into());
    }
    if now.console_errors > was.console_errors {
        o.insert(
            "console_errors".into(),
            (now.console_errors - was.console_errors).into(),
        );
    }
    if now.net_failures > was.net_failures {
        o.insert(
            "net_failures".into(),
            (now.net_failures - was.net_failures).into(),
        );
    }
    if now.downloads > was.downloads {
        o.insert("downloads".into(), (now.downloads - was.downloads).into());
    }
    let s = page.lock();
    if let Some(d) = &s.dialog {
        o.insert(
            "dialog".into(),
            format!("{} \"{}\"", d.kind, d.message).into(),
        );
    }
    if s.paused.is_some() {
        o.insert("paused".into(), true.into());
    }
    Value::Object(o)
}

/// Seen-ids helper for tests of dedupe.
pub fn unique<T: std::hash::Hash + Eq + Clone>(v: &[T]) -> Vec<T> {
    let mut seen = HashSet::new();
    v.iter()
        .filter(|x| seen.insert((*x).clone()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_update_the_state_and_the_delta() {
        let p = Page::new("T", "S", "about:blank", "");
        let was = p.mark();
        p.on_event(
            "Page.frameNavigated",
            &json!({"frame":{"id":"F","loaderId":"L1","url":"https://a.test/"}}),
        );
        p.on_event(
            "Runtime.consoleAPICalled",
            &json!({"type":"error","args":[{"value":"boom"}]}),
        );
        p.on_event("Network.requestWillBeSent", &json!({"requestId":"1","type":"XHR","request":{"url":"https://a.test/x","method":"GET"}}));
        assert!(!p.lock().inflight.is_empty());
        p.on_event(
            "Network.loadingFailed",
            &json!({"requestId":"1","errorText":"net::ERR_FAILED"}),
        );
        assert!(p.lock().inflight.is_empty());
        let d = delta(&p, &was);
        assert_eq!(d["url"], "https://a.test/");
        assert_eq!(d["console_errors"], 1);
        assert_eq!(d["net_failures"], 1);
        assert_eq!(d["navigated"], true);
    }

    #[test]
    fn a_dialog_shows_in_the_delta_until_closed() {
        let p = Page::new("T", "S", "", "");
        let was = p.mark();
        p.on_event(
            "Page.javascriptDialogOpening",
            &json!({"type":"confirm","message":"Delete?"}),
        );
        assert_eq!(delta(&p, &was)["dialog"], "confirm \"Delete?\"");
        p.on_event("Page.javascriptDialogClosed", &json!({}));
        assert!(delta(&p, &was).get("dialog").is_none());
    }

    #[test]
    fn network_idle_waits_for_quiet() {
        let p = Page::new("T", "S", "", "");
        let t0 = Instant::now();
        assert!(p.wait_network_idle(Duration::from_millis(30), t0 + Duration::from_secs(1)));
        p.on_event(
            "Network.requestWillBeSent",
            &json!({"requestId":"9","type":"Fetch","request":{"url":"u","method":"GET"}}),
        );
        assert!(!p.wait_network_idle(
            Duration::from_millis(30),
            Instant::now() + Duration::from_millis(60)
        ));
        p.on_event("Network.loadingFinished", &json!({"requestId":"9"}));
        assert!(p.wait_network_idle(
            Duration::from_millis(30),
            Instant::now() + Duration::from_secs(1)
        ));
    }
}
