// SPDX-License-Identifier: BSD-2-Clause
//! `wol_fake`: a scripted fake wold and fake Kit host for binding test suites (Cryswol's
//! `crystal spec`, docs/modules/cryswol.md "Tests"). It speaks the real wires: the wold socket
//! greets with `shared::jobs::greet` and decodes every call with `shared::portal::Call::decode`,
//! and the Kit socket checks every app line with `shared::view::ToHost::parse`. A test therefore
//! fails on a malformed message here as it would against the real system.
//!
//! The behaviour is data, not code, so a new verb group adds a scenario file and no Rust:
//!
//! ```json
//! {
//!   "answers": {"<call tag>": [<Answer JSON, optional "fd": "<file contents>">, ...]},
//!   "events":  [{"after": "<call tag>" | "subscribe", "emit": [<Event JSON>, ...]}],
//!   "kit":     [{"after_show": 2, "send": [<FromHost JSON>, ...]}]
//! }
//! ```
//!
//! - `answers`: per call tag (`"notification.post"`), used in order; the last one repeats. A call
//!   with no script gets `{"ok":true}`. An answer with `"fd"` is sent with a fresh read-write file
//!   holding that text attached (panels, devices).
//! - `events`: sent on every subscription after that call is answered (`"subscribe"`: as soon as a
//!   subscription connects). Events with no subscriber yet wait for one.
//! - `kit`: after the app's `after_show`-th line on one Kit connection (the hello counts), send
//!   these. In them `"window": "$window"` is the last shown window, `"widget": {...}` the id of the
//!   first node (depth first) of that window's content whose fields include all the given ones,
//!   and `"source": {"menu": "<title>"}` or `"source": "toolbar"` a menu or toolbar id.
//!
//! Usage: `wol_fake <scenario.json> <dir> <log>`. Binds `<dir>/wold.sock` and `<dir>/kit.sock`,
//! prints `ready` once both listen, writes one JSON line per message received to `<log>`
//! (`{"wold": <call>}` or `{"kit": <message>}`), and exits when its stdin closes, so it never
//! outlives the test that started it.
use serde_json::{Map, Value};
use shared::events::Event;
use shared::portal::{Answer, Call};
use shared::view::{FromHost, ToHost};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

/// One loaded scenario plus what is live: subscriptions and events waiting for one.
struct Fake {
    answers: Mutex<HashMap<String, Vec<Value>>>,
    events: Vec<(String, Vec<String>)>,
    kit: Vec<(usize, Vec<Value>)>,
    subscribers: Mutex<Vec<UnixStream>>,
    pending: Mutex<Vec<String>>,
    log: Mutex<std::fs::File>,
    dir: PathBuf,
    files: Mutex<u32>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Fake {
    fn load(scenario: &Path, dir: &Path, log: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(scenario)
            .map_err(|e| format!("{}: {e}", scenario.display()))?;
        let s: Value = serde_json::from_str(&text).map_err(|e| format!("scenario: {e}"))?;
        let mut answers = HashMap::new();
        if let Some(map) = s.get("answers").and_then(Value::as_object) {
            for (tag, list) in map {
                let list = list.as_array().cloned().unwrap_or_default();
                for a in &list {
                    answer_line(a).map_err(|e| format!("answers.{tag}: {e}"))?;
                }
                answers.insert(tag.clone(), list);
            }
        }
        let mut events = Vec::new();
        for e in s
            .get("events")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let after = e.get("after").and_then(Value::as_str).unwrap_or_default();
            let mut lines = Vec::new();
            for one in e
                .get("emit")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let ev = Event::parse(&one.to_string())
                    .ok_or_else(|| format!("events after {after}: not an event: {one}"))?;
                lines.push(ev.to_line());
            }
            events.push((after.to_owned(), lines));
        }
        let mut kit = Vec::new();
        for k in s.get("kit").and_then(Value::as_array).into_iter().flatten() {
            let n = k.get("after_show").and_then(Value::as_u64).unwrap_or(1);
            let send = k
                .get("send")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            kit.push((usize::try_from(n).unwrap_or(usize::MAX), send));
        }
        let log = std::fs::File::create(log).map_err(|e| format!("{}: {e}", log.display()))?;
        Ok(Self {
            answers: Mutex::new(answers),
            events,
            kit,
            subscribers: Mutex::default(),
            pending: Mutex::default(),
            log: Mutex::new(log),
            dir: dir.to_owned(),
            files: Mutex::new(0),
        })
    }

    fn record(&self, side: &str, v: Value) {
        let mut line = Value::Object(Map::from_iter([(side.to_owned(), v)])).to_string();
        line.push('\n');
        let mut f = lock(&self.log);
        let _ = f.write_all(line.as_bytes());
        let _ = f.flush();
    }

    /// The next scripted answer for `tag`; the last one stays.
    fn next_answer(&self, tag: &str) -> Value {
        let mut all = lock(&self.answers);
        match all.get_mut(tag) {
            Some(list) if list.len() > 1 => list.remove(0),
            Some(list) if list.len() == 1 => list[0].clone(),
            _ => serde_json::json!({"ok": true}),
        }
    }

    /// Send `lines` to every subscription, or keep them until one connects.
    fn emit(&self, lines: &[String]) {
        let mut subs = lock(&self.subscribers);
        if subs.is_empty() {
            lock(&self.pending).extend(lines.iter().cloned());
            return;
        }
        subs.retain(|mut s| lines.iter().all(|l| s.write_all(l.as_bytes()).is_ok()));
    }

    fn after(&self, tag: &str) {
        for (after, lines) in &self.events {
            if after == tag {
                self.emit(lines);
            }
        }
    }

    fn attached_file(&self, contents: &str) -> std::io::Result<std::fs::File> {
        let n = {
            let mut n = lock(&self.files);
            *n += 1;
            *n
        };
        let path = self.dir.join(format!("fd-{n}.txt"));
        std::fs::write(&path, contents)?;
        std::fs::File::options().read(true).write(true).open(path)
    }

    fn wold(self: &Arc<Self>, conn: UnixStream) {
        if shared::jobs::greet(&conn).is_err() {
            return;
        }
        let mut line = String::new();
        if BufReader::new(&conn).read_line(&mut line).is_err() {
            return;
        }
        if line.trim() == "subscribe" {
            let waiting: Vec<String> = std::mem::take(&mut *lock(&self.pending));
            let mut w = &conn;
            for l in &waiting {
                let _ = w.write_all(l.as_bytes());
            }
            lock(&self.subscribers).push(conn);
            self.after("subscribe");
            return;
        }
        let Some(json) = line.trim().strip_prefix("call ") else {
            self.record("wold_invalid", Value::String(line));
            return;
        };
        let call = match Call::decode(json.as_bytes()) {
            Ok(c) => c,
            Err(e) => {
                self.record("wold_invalid", Value::String(format!("{e}: {json}")));
                return;
            }
        };
        let v: Value = serde_json::from_str(&call.encode()).unwrap_or(Value::Null);
        let tag = v["call"].as_str().unwrap_or_default().to_owned();
        self.record("wold", v);
        let a = self.next_answer(&tag);
        let Ok(text) = answer_line(&a) else { return };
        let sent = match a.get("fd").and_then(Value::as_str) {
            Some(contents) => self
                .attached_file(contents)
                .and_then(|f| shared::sys::send_with_fd(&conn, text.as_bytes(), f.as_fd())),
            None => (&conn).write_all(text.as_bytes()),
        };
        if sent.is_ok() {
            self.after(&tag);
        }
    }

    fn kit(self: &Arc<Self>, conn: UnixStream) {
        let mut r = BufReader::new(&conn);
        let mut count = 0;
        let mut last_show: Option<Value> = None;
        loop {
            let mut line = String::new();
            match r.read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            let Some(m) = ToHost::parse(&line) else {
                self.record("kit_invalid", Value::String(line));
                continue;
            };
            let v = serde_json::to_value(&m).unwrap_or(Value::Null);
            if matches!(m, ToHost::Show { .. }) {
                last_show = Some(v.clone());
            }
            self.record("kit", v);
            count += 1;
            for (n, send) in &self.kit {
                if *n != count {
                    continue;
                }
                for s in send {
                    match resolve(s, last_show.as_ref()) {
                        Ok(e) => {
                            let _ = (&conn).write_all(e.to_line().as_bytes());
                        }
                        Err(e) => self.record("kit_unresolved", Value::String(e)),
                    }
                }
            }
        }
    }
}

/// The wire line for a scripted answer (without its `fd` key), checked to be a real `Answer`.
fn answer_line(a: &Value) -> Result<String, String> {
    let mut a = a.clone();
    if let Some(o) = a.as_object_mut() {
        o.remove("fd");
    }
    let parsed: Answer = serde_json::from_value(a).map_err(|e| format!("not an answer: {e}"))?;
    if parsed.ok != parsed.code.is_none() {
        return Err("an answer is ok without a code, or not ok with one".into());
    }
    Ok(parsed.to_line())
}

/// Fill `"$window"`, widget selectors and menu/toolbar sources from the last shown window.
fn resolve(send: &Value, show: Option<&Value>) -> Result<FromHost, String> {
    let mut e = send.clone();
    let o = e.as_object_mut().ok_or("a kit event is an object")?;
    if o.get("window").and_then(Value::as_str) == Some("$window") {
        let w = show
            .and_then(|s| s.get("window"))
            .ok_or("no window shown yet")?;
        o.insert("window".into(), w.clone());
    }
    if let Some(sel) = o.get("widget").filter(|w| w.is_object()).cloned() {
        let content = show
            .and_then(|s| s.get("content"))
            .ok_or("no window shown yet")?;
        let id = find(content, &sel).ok_or_else(|| format!("no widget matches {sel}"))?;
        o.insert("widget".into(), id);
    }
    match o.get("source").cloned() {
        Some(Value::String(s)) if s == "toolbar" => {
            let id = show
                .and_then(|s| s.pointer("/toolbar/id"))
                .ok_or("the shown window has no toolbar")?;
            o.insert("source".into(), id.clone());
        }
        Some(Value::Object(sel)) => {
            let title = sel.get("menu").and_then(Value::as_str).unwrap_or_default();
            let id = show
                .and_then(|s| s.get("menus"))
                .and_then(Value::as_array)
                .and_then(|ms| ms.iter().find(|m| m["title"] == title))
                .map(|m| m["id"].clone())
                .ok_or_else(|| format!("no menu titled {title:?}"))?;
            o.insert("source".into(), id);
        }
        _ => {}
    }
    serde_json::from_value(e).map_err(|err| format!("not a kit event: {err}"))
}

/// The `id` of the first node (depth first) whose fields include every field of `sel`.
fn find(node: &Value, sel: &Value) -> Option<Value> {
    match node {
        Value::Object(o) => {
            let fits = sel
                .as_object()
                .is_some_and(|s| s.iter().all(|(k, v)| o.get(k) == Some(v)));
            if fits && let Some(id) = o.get("id") {
                return Some(id.clone());
            }
            o.values().find_map(|v| find(v, sel))
        }
        Value::Array(a) => a.iter().find_map(|v| find(v, sel)),
        _ => None,
    }
}

fn listen(path: &Path) -> UnixListener {
    let _ = std::fs::remove_file(path);
    UnixListener::bind(path).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())))
}

fn fail(msg: &str) -> ! {
    eprintln!("wol_fake: {msg}");
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [scenario, dir, log] = args.as_slice() else {
        fail("usage: wol_fake <scenario.json> <dir> <log>")
    };
    let dir = PathBuf::from(dir);
    let fake = Arc::new(
        Fake::load(Path::new(scenario), &dir, Path::new(log)).unwrap_or_else(|e| fail(&e)),
    );
    let wold = listen(&dir.join("wold.sock"));
    let kit = listen(&dir.join("kit.sock"));
    let f = fake.clone();
    std::thread::spawn(move || {
        for conn in wold.incoming().flatten() {
            let f = f.clone();
            std::thread::spawn(move || f.wold(conn));
        }
    });
    let f = fake;
    std::thread::spawn(move || {
        for conn in kit.incoming().flatten() {
            let f = f.clone();
            std::thread::spawn(move || f.kit(conn));
        }
    });
    println!("ready");
    let _ = std::io::stdout().flush();
    // Live until the test closes our stdin (or dies).
    let _ = std::io::stdin().read_to_end(&mut Vec::new());
}
