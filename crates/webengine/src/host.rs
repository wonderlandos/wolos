// SPDX-License-Identifier: BSD-2-Clause
//! The host process's logic for a system web app (`/usr/bin/wol-webhost`, ADR 0048), without the
//! window and the engine: the pump between the page's messages and wold ([`Host`]), wold's end
//! ([`WoldClient`]: one `webapp-call` request per call, on the socket every wold client uses), and
//! the small decisions around them (which permissions the host starts with, the locale).
//!
//! Concept. A page message comes in from the engine on the host's thread ([`Host::on_page_message`]);
//! the [`Bridge`] refuses what the app did not declare or hold; an allowed call runs on a worker
//! thread (wold may take a while, an administrator sheet waits for a person) and its reply waits in
//! a queue the window loop drains ([`Host::take_ready`]) and posts into the page. At most
//! [`MAX_IN_FLIGHT`] calls run at once, and no text over [`MAX_MESSAGE`] goes either way.
//!
//! Why. The engine is single-threaded and calls back into the host; wold is slow and out of
//! process. Keeping the two apart with one queue means a slow wold never stalls painting, and the
//! whole path is a function of messages in and messages out, which the tests drive without an
//! engine, a window or a wold.
use crate::bridge::{AppInfo, Bridge, Call, Fault, MAX_MESSAGE, Step, Wold};
use serde_json::{Map, Value, json};
use shared::errors::ErrorCode;
use shared::portal::{Answer, Code};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

/// Calls one app may have waiting on wold at once.
pub const MAX_IN_FLIGHT: usize = 8;
/// Most of wold's reply the host reads.
pub const MAX_REPLY: u64 = 1 << 20;
/// How long the host waits for wold's reply: wold's own deadline plus the sheet's wait.
pub const WAIT: std::time::Duration = std::time::Duration::from_secs(300);

/// Page messages in, replies out.
pub struct Host {
    bridge: Bridge,
    wold: Arc<dyn Wold + Send + Sync>,
    app_id: String,
    out: Sender<String>,
    ready: Receiver<String>,
    in_flight: Arc<AtomicUsize>,
    /// Run (on the worker's thread) after a reply is queued: the window loop sleeps until woken.
    notify: Notify,
}

/// A wake-up for whoever drains [`Host::take_ready`].
pub type Notify = Arc<dyn Fn() + Send + Sync>;

fn busy(id: u64) -> String {
    reply_text(
        id,
        Err(Fault::new(
            ErrorCode::Unavailable,
            "WolOS is busy with other requests from this app. Try again in a moment.",
            "too many calls in flight",
        )),
    )
}

fn reply_text(id: u64, r: Result<Value, Fault>) -> String {
    let text = match r {
        Ok(v) => json!({"id": id, "ok": v}),
        Err(f) => json!({"id": id, "err": f.to_json()}),
    }
    .to_string();
    if text.len() <= MAX_MESSAGE {
        return text;
    }
    json!({"id": id, "err": Fault::new(
        ErrorCode::Failed,
        "The answer was too large for this app.",
        "reply over 64 KiB",
    ).to_json()})
    .to_string()
}

/// Releases one in-flight slot when the call's thread ends, however it ends.
struct Slot(Arc<AtomicUsize>);
// Gives the in-flight slot back (the count a new call is checked against).
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Host {
    pub fn new(bridge: Bridge, wold: Arc<dyn Wold + Send + Sync>) -> Host {
        let (out, ready) = channel();
        Host {
            app_id: bridge.app().id.clone(),
            bridge,
            wold,
            out,
            ready,
            in_flight: Arc::default(),
            notify: Arc::new(|| {}),
        }
    }

    /// Runs `f` on the worker's thread each time a reply it queued is ready. The window loop waits
    /// for events with no timeout, so this is what ends that wait for a reply from wold.
    pub fn set_notify(&mut self, f: Notify) {
        self.notify = f;
    }

    pub fn bridge(&self) -> &Bridge {
        &self.bridge
    }

    /// One text the page posted. Answers that need no wold are queued at once; a call starts on a
    /// worker thread.
    pub fn on_page_message(&mut self, text: &str) {
        match self.bridge.handle(text) {
            Step::Nothing => {}
            Step::Reply(r) => {
                let _ = self.out.send(r);
            }
            Step::Call(call) => self.start(call),
        }
    }

    fn start(&self, call: Call) {
        if self.in_flight.fetch_add(1, Ordering::AcqRel) >= MAX_IN_FLIGHT {
            self.in_flight.fetch_sub(1, Ordering::AcqRel);
            let _ = self.out.send(busy(call.id));
            return;
        }
        let slot = Slot(self.in_flight.clone());
        let (wold, out, app) = (self.wold.clone(), self.out.clone(), self.app_id.clone());
        let notify = self.notify.clone();
        let spawned = std::thread::Builder::new()
            .name("webhost-call".into())
            .spawn({
                let call = call.clone();
                move || {
                    let r = wold.call(&app, &call.method, &call.params);
                    // Before the reply is queued: whoever sees the reply may call again at once.
                    drop(slot);
                    let _ = out.send(reply_text(call.id, r));
                    notify();
                }
            });
        if spawned.is_err() {
            // The closure (and its slot) is gone with the failed spawn.
            let _ = self.out.send(busy(call.id));
        }
    }

    /// Replies and events ready for the page, in order.
    pub fn take_ready(&self) -> Vec<String> {
        self.ready.try_iter().collect()
    }

    /// Waits up to `limit` for the next ready text (tests, and a host with nothing else to do).
    pub fn wait_ready(&self, limit: std::time::Duration) -> Option<String> {
        self.ready.recv_timeout(limit).ok()
    }

    /// Queues an event text for the page, when there is one.
    pub fn push(&self, text: Option<String>) {
        if let Some(t) = text {
            let _ = self.out.send(t);
        }
    }

    pub fn set_app(&mut self, app: AppInfo) {
        let e = self.bridge.set_app(app);
        self.push(e);
    }

    pub fn set_granted(&mut self, granted: &[String]) {
        let e = self.bridge.set_granted(granted);
        self.push(e);
    }
}

/// wold, over its control socket: one `webapp-call <method> <params>` per call. The app is not in
/// the request; wold takes it from this process's scope.
pub struct WoldClient {
    pub socket: PathBuf,
}

impl WoldClient {
    pub fn new() -> WoldClient {
        WoldClient {
            socket: shared::jobs::SOCKET.into(),
        }
    }
}

impl Default for WoldClient {
    fn default() -> Self {
        Self::new()
    }
}

/// A page-side error code for wold's refusal.
fn fault_of(a: &Answer) -> Fault {
    let message = a
        .message
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| "WolOS could not do that.".into());
    let (code, hint) = match a.code {
        Some(Code::PermissionDenied) => (ErrorCode::Denied, "Turn it on in Settings > Privacy"),
        Some(Code::InvalidArgument) => (ErrorCode::Invalid, "wold refused the request"),
        Some(Code::Unavailable) => (ErrorCode::Unavailable, ""),
        _ => (ErrorCode::Failed, ""),
    };
    Fault::new(code, message, hint)
}

/// The verb's own reply inside wold's answer: `{"ok":v}` or `{"err":{code,message,hint}}`.
fn unwrap_reply(data: &Value) -> Result<Value, Fault> {
    if let Some(v) = data.get("ok") {
        return Ok(v.clone());
    }
    let e = data.get("err").cloned().unwrap_or(Value::Null);
    let text = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let code = ErrorCode::ALL
        .into_iter()
        .find(|c| *c != ErrorCode::Ok && c.name() == text("code"))
        .unwrap_or(ErrorCode::Failed);
    let message = if text("message").is_empty() {
        "It did not work.".to_owned()
    } else {
        text("message")
    };
    Err(Fault::new(code, message, text("hint")))
}

/// Wold's reply line as the result of the call.
pub fn read_answer(line: &str) -> Result<Value, Fault> {
    let a = Answer::parse(line);
    if !a.ok {
        return Err(fault_of(&a));
    }
    match &a.data {
        Some(d) => unwrap_reply(d),
        None => Err(Fault::new(
            ErrorCode::Failed,
            "WolOS gave no answer.",
            "wold's reply had no data",
        )),
    }
}

impl Wold for WoldClient {
    fn call(
        &self,
        _app_id: &str,
        method: &str,
        params: &Map<String, Value>,
    ) -> Result<Value, Fault> {
        let unavailable = |why: String| {
            Fault::new(
                ErrorCode::Unavailable,
                "WolOS is not answering, so this app cannot do that now.",
                why,
            )
        };
        let mut s = shared::jobs::connect(&self.socket).map_err(|e| unavailable(e.to_string()))?;
        s.set_read_timeout(Some(WAIT))
            .map_err(|e| unavailable(e.to_string()))?;
        let line = format!(
            "webapp-call {method} {}\n",
            serde_json::to_string(params).unwrap_or_else(|_| "{}".into())
        );
        s.write_all(line.as_bytes())
            .map_err(|e| unavailable(e.to_string()))?;
        let _ = s.shutdown(std::net::Shutdown::Write);
        let mut out = String::new();
        s.take(MAX_REPLY)
            .read_to_string(&mut out)
            .map_err(|e| unavailable(e.to_string()))?;
        read_answer(out.trim())
    }
}

/// The locale for `wolos.app.locale`: `LC_ALL`, `LC_MESSAGES` or `LANG` (`pt_BR.UTF-8` -> `pt-BR`),
/// `en` for the C locale and when none is set.
pub fn locale(vars: &dyn Fn(&str) -> Option<String>) -> String {
    let raw = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|k| vars(k).filter(|v| !v.is_empty()))
        .unwrap_or_default();
    let base = raw.split(['.', '@']).next().unwrap_or("");
    let ok = !base.is_empty()
        && base != "C"
        && base != "POSIX"
        && base.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if ok {
        base.replace('_', "-")
    } else {
        "en".into()
    }
}

/// The permissions the host starts with. A system app (its folder is the image's) holds what it
/// declares; any other app holds the ones the consent file grants to its folder (a permission with
/// no consent capability is never held by it). wold checks every call again.
pub fn initial_grants(
    declared: &[String],
    system: bool,
    folder: &str,
    consent: &[(String, String)],
) -> Vec<String> {
    declared
        .iter()
        .filter(|name| {
            system
                || crate::bridge::catalog::permission(name)
                    .and_then(|p| p.consent)
                    .is_some_and(|cap| consent.iter().any(|(a, c)| a == folder && c == cap))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{Manifest, Theme};
    use std::time::Duration;

    /// Answers once the test lets it (one release per call), so nothing here depends on timing.
    struct Gated(std::sync::Mutex<std::sync::mpsc::Receiver<()>>);
    impl Wold for Gated {
        fn call(&self, app: &str, m: &str, p: &Map<String, Value>) -> Result<Value, Fault> {
            let wait = self.0.lock().unwrap().recv_timeout(Duration::from_secs(10));
            assert!(wait.is_ok(), "the test never released this call");
            Ok(json!({"app": app, "method": m, "n": p.len()}))
        }
    }

    fn gated() -> (Gated, std::sync::mpsc::Sender<()>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Gated(std::sync::Mutex::new(rx)), tx)
    }

    fn host(wold: impl Wold + Send + Sync + 'static, granted: &[&str]) -> Host {
        let m = Manifest::parse(
            "id='org.wolos.webapp.t'\nname='T'\nversion='1.0.0'\nicon='i.svg'\npermissions=['audio.read','games.read']\n",
        )
        .unwrap();
        let app = AppInfo {
            id: m.id.clone(),
            version: m.version.clone(),
            locale: "en".into(),
            theme: Theme::Light,
            accent: "violet".into(),
        };
        let g: Vec<String> = granted.iter().map(|s| s.to_string()).collect();
        Host::new(Bridge::new(m, app, &g), Arc::new(wold))
    }

    #[test]
    fn a_workers_reply_wakes_the_window_loop() {
        let (wold, release) = gated();
        let mut h = host(wold, &["audio.read"]);
        let (tx, woken) = std::sync::mpsc::channel();
        h.set_notify(Arc::new(move || {
            let _ = tx.send(std::thread::current().id());
        }));
        h.on_page_message(r#"{"id":7,"call":"audio.doctor","params":{}}"#);
        assert!(woken.try_recv().is_err(), "nothing is ready yet");
        release.send(()).unwrap();
        let by = woken
            .recv_timeout(Duration::from_secs(5))
            .expect("the reply was queued but nothing woke the loop, which waits with no timeout");
        assert_ne!(by, std::thread::current().id(), "from the worker's thread");
        assert_eq!(
            h.take_ready().len(),
            1,
            "and the reply is there when it wakes"
        );
    }

    #[test]
    fn a_call_goes_to_wold_off_the_hosts_thread_and_its_reply_is_queued() {
        let (wold, release) = gated();
        let mut h = host(wold, &["audio.read"]);
        h.on_page_message(r#"{"id":7,"call":"audio.doctor","params":{"a":1}}"#);
        assert!(
            h.take_ready().is_empty(),
            "the call has not finished: the host was not blocked"
        );
        release.send(()).unwrap();
        let r = h.wait_ready(Duration::from_secs(5)).unwrap();
        let v: Value = serde_json::from_str(&r).unwrap();
        assert_eq!(
            v,
            json!({"id":7,"ok":{"app":"org.wolos.webapp.t","method":"audio.doctor","n":1}})
        );
    }

    #[test]
    fn a_refused_call_never_reaches_wold_and_answers_at_once() {
        struct Never;
        impl Wold for Never {
            fn call(&self, _: &str, _: &str, _: &Map<String, Value>) -> Result<Value, Fault> {
                panic!("wold was asked");
            }
        }
        let mut h = host(Never, &["audio.read"]);
        h.on_page_message(r#"{"id":1,"call":"games.doctor"}"#);
        h.on_page_message(r#"{"id":2,"call":"rm.rf"}"#);
        h.on_page_message(r#"{"id":3,"call":"about"}"#);
        h.on_page_message("{nope");
        let got: Vec<Value> = h
            .take_ready()
            .iter()
            .map(|t| serde_json::from_str(t).unwrap())
            .collect();
        let codes: Vec<&str> = got
            .iter()
            .map(|v| v["err"]["code"].as_str().unwrap())
            .collect();
        assert_eq!(codes, ["denied", "not-found", "denied"]);
    }

    #[test]
    fn calls_in_flight_are_capped_and_a_slot_comes_back() {
        let (wold, release) = gated();
        let mut h = host(wold, &["audio.read"]);
        for i in 0..MAX_IN_FLIGHT + 3 {
            h.on_page_message(&format!(r#"{{"id":{i},"call":"audio.get"}}"#));
        }
        let early = h.take_ready();
        assert_eq!(
            early.len(),
            3,
            "the three over the cap are told at once: {early:?}"
        );
        assert!(early[0].contains("unavailable"));
        for _ in 0..MAX_IN_FLIGHT {
            release.send(()).unwrap();
            assert!(
                h.wait_ready(Duration::from_secs(5))
                    .unwrap()
                    .contains("\"ok\"")
            );
        }
        release.send(()).unwrap();
        h.on_page_message(r#"{"id":99,"call":"audio.get"}"#);
        assert!(
            h.wait_ready(Duration::from_secs(5))
                .unwrap()
                .contains("\"ok\""),
            "slots were given back"
        );
    }

    #[test]
    fn a_reply_over_the_cap_becomes_an_error_and_events_reach_subscribers() {
        struct Big;
        impl Wold for Big {
            fn call(&self, _: &str, _: &str, _: &Map<String, Value>) -> Result<Value, Fault> {
                Ok(Value::String("x".repeat(MAX_MESSAGE)))
            }
        }
        let mut h = host(Big, &["audio.read"]);
        h.on_page_message(r#"{"id":1,"call":"audio.get"}"#);
        let v: Value =
            serde_json::from_str(&h.wait_ready(Duration::from_secs(5)).unwrap()).unwrap();
        assert_eq!(v["err"]["code"], "failed");
        h.set_granted(&["audio.read".into()]);
        assert!(h.take_ready().is_empty(), "not subscribed");
        h.on_page_message(r#"{"sub":"permissions"}"#);
        h.set_granted(&["games.read".into()]);
        assert_eq!(
            h.take_ready(),
            [r#"{"data":["games.read"],"topic":"permissions"}"#]
        );
    }

    #[test]
    fn wolds_answers_become_page_errors_with_stable_codes() {
        let ok = r#"{"ok":true,"data":{"ok":{"volume":40}}}"#;
        assert_eq!(read_answer(ok).unwrap(), json!({"volume":40}));
        let verb = r#"{"ok":true,"data":{"err":{"code":"unavailable","message":"No sound server.","hint":"start it"}}}"#;
        let f = read_answer(verb).unwrap_err();
        assert_eq!(
            (f.code, f.message.as_str(), f.hint.as_str()),
            ("unavailable", "No sound server.", "start it")
        );
        let denied = r#"{"ok":false,"code":"permission-denied","message":"This app is not allowed to do that."}"#;
        let f = read_answer(denied).unwrap_err();
        assert_eq!(f.code, "denied");
        assert!(
            f.hint.starts_with("Turn it on"),
            "a capital hint is for the person"
        );
        assert_eq!(read_answer("garbage").unwrap_err().code, "failed");
        assert_eq!(read_answer(r#"{"ok":true}"#).unwrap_err().code, "failed");
        let weird = r#"{"ok":true,"data":{"err":{"code":"made-up"}}}"#;
        assert_eq!(read_answer(weird).unwrap_err().code, "failed");
    }

    #[test]
    fn wold_is_reached_with_the_handshake_and_one_request_line() {
        use std::os::unix::net::UnixListener;
        let dir = std::env::temp_dir().join(format!("webhost-sock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wold.sock");
        let l = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            s.write_all(b"hello abc123\n").unwrap();
            let mut got = String::new();
            let mut r = std::io::BufReader::new(s.try_clone().unwrap());
            std::io::BufRead::read_line(&mut r, &mut got).unwrap(); // the echo
            let mut req = String::new();
            std::io::BufRead::read_line(&mut r, &mut req).unwrap();
            s.write_all(b"{\"ok\":true,\"data\":{\"ok\":{\"hi\":1}}}\n")
                .unwrap();
            (got, req)
        });
        let c = WoldClient { socket: path };
        let mut params = Map::new();
        params.insert("topic".into(), json!("disks"));
        let v = c.call("org.wolos.webapp.t", "sys", &params).unwrap();
        assert_eq!(v, json!({"hi":1}));
        let (echo, req) = server.join().unwrap();
        assert_eq!(echo, "hello abc123\n");
        assert_eq!(
            req, "webapp-call sys {\"topic\":\"disks\"}\n",
            "the app id is not in the request"
        );
        let gone = WoldClient {
            socket: dir.join("none.sock"),
        };
        assert_eq!(
            gone.call("x", "sys", &Map::new()).unwrap_err().code,
            "unavailable"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locale_and_initial_grants() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| pairs.iter().find(|p| p.0 == k).map(|p| p.1.to_owned())
        };
        assert_eq!(locale(&env(&[("LANG", "pt_BR.UTF-8")])), "pt-BR");
        assert_eq!(
            locale(&env(&[("LC_ALL", "de_DE@euro"), ("LANG", "en_US")])),
            "de-DE"
        );
        assert_eq!(locale(&env(&[("LANG", "C")])), "en");
        assert_eq!(locale(&env(&[])), "en");
        let declared: Vec<String> = ["audio.read", "system.read", "audio.control"]
            .map(String::from)
            .into();
        assert_eq!(
            initial_grants(&declared, true, "/usr/share/wolos/webapps/d", &[]),
            declared
        );
        let consent = vec![
            ("/opt/wolos/webapps/n".to_owned(), "audio-info".to_owned()),
            (
                "/opt/wolos/webapps/other".to_owned(),
                "audio-control".to_owned(),
            ),
        ];
        assert_eq!(
            initial_grants(&declared, false, "/opt/wolos/webapps/n", &consent),
            ["audio.read"]
        );
        assert!(initial_grants(&declared, false, "/opt/wolos/webapps/zzz", &consent).is_empty());
    }
}
