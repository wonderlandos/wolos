// SPDX-License-Identifier: BSD-2-Clause
//! The bridge between a system web app's page and wold (ADR 0048), host side.
//!
//! Concept. The page sees `window.wolos` (`webapps/kit/bridge.js`): `call(method, params)`,
//! `subscribe(topic, handler)`, `app` and `permissions`. Underneath it is one JSON message channel
//! between the page and its host process ([`Channel`], over the engine's host-message calls of
//! ADR 0048), and this module is the host's end: it reads the page's messages, refuses what the
//! app did not declare or was not granted, hands each allowed call to wold ([`Wold`]) and turns the
//! answers and events into messages for the page.
//!
//! Why here. The host is one process per app id with the app's manifest in hand, so it can refuse
//! early and cheaply; wold checks again (method, caller's app id, grant) and is the authority.
//!
//! Messages, page to host: `{"id":n,"call":"<method>","params":{..}}`, `{"sub":"<topic>"}`,
//! `{"unsub":"<topic>"}`. Host to page: `{"id":n,"ok":<result>}`, `{"id":n,"err":{"code",
//! "message","hint"}}`, `{"topic":"<topic>","data":<value>}`. The first record, `init` (`{app,
//! permissions}`), is handed to the page before its scripts run ([`Bridge::init`]).
pub mod catalog;
pub mod manifest;
pub mod mock;

pub use manifest::Manifest;

use serde_json::{Map, Value, json};
use shared::errors::ErrorCode;
use std::collections::BTreeSet;

/// Largest page message the host reads.
pub const MAX_MESSAGE: usize = 64 * 1024;

/// An error as the page gets it: `code` is a [`shared::errors::ErrorCode`] name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    pub code: &'static str,
    pub message: String,
    pub hint: String,
}

impl Fault {
    pub fn new(code: ErrorCode, message: impl Into<String>, hint: impl Into<String>) -> Fault {
        Fault {
            code: code.name(),
            message: message.into(),
            hint: hint.into(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({"code": self.code, "message": self.message, "hint": self.hint})
    }
}

/// wold, as the host reaches it: one call for this app. The real one sends the request over the
/// host's wold connection; [`mock::Fixtures`] answers from recorded results.
pub trait Wold {
    fn call(&self, app_id: &str, method: &str, params: &Map<String, Value>)
    -> Result<Value, Fault>;
}

/// The page's end of the channel: the host posts one JSON text per message
/// (`wwe_view_post_message`).
pub trait Channel {
    fn post(&self, json: &str);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

/// `wolos.app`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    pub id: String,
    pub version: String,
    /// BCP 47 (`en`, `pt-BR`).
    pub locale: String,
    pub theme: Theme,
    /// An accent id of `wol_core::theme::accent::ACCENTS` (`violet`).
    pub accent: String,
}

impl AppInfo {
    pub fn to_json(&self) -> Value {
        let theme = match self.theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
        };
        json!({"id": self.id, "version": self.version, "locale": self.locale, "theme": theme, "accent": self.accent})
    }
}

/// The accent id (`wol_core::theme::accent::ACCENTS`) of a theme's accent colour `0xRRGGBB`, the
/// toolkit's `accent_rgb`; the default accent for a colour none has (a theme of others).
pub fn accent_id(dark: bool, rgb: u32) -> &'static str {
    use wol_core::theme::accent::{ACCENTS, DEFAULT};
    let rgb_of = |h: &str| u32::from_str_radix(h.trim_start_matches('#'), 16).ok();
    ACCENTS
        .iter()
        .find(|a| rgb_of(if dark { a.3 } else { a.2 }) == Some(rgb))
        .or_else(|| {
            ACCENTS
                .iter()
                .find(|a| rgb_of(a.2) == Some(rgb) || rgb_of(a.3) == Some(rgb))
        })
        .map_or(DEFAULT, |a| a.0)
}

/// One allowed call, to run where the host likes (a worker thread: wold may take a while).
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub id: u64,
    pub method: String,
    pub params: Map<String, Value>,
}

impl Call {
    /// The reply message for the page.
    pub fn run(&self, app_id: &str, wold: &dyn Wold) -> String {
        reply(self.id, wold.call(app_id, &self.method, &self.params))
    }
}

/// What a page message asks of the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Post this to the page now.
    Reply(String),
    /// Run it ([`Call::run`]) and post the reply.
    Call(Call),
    /// Nothing to answer (a subscription change, or a message too broken to carry an id).
    Nothing,
}

/// The `id` of a text that starts `{"id":N,` (how `bridge.js` writes a call), without parsing it.
fn leading_id(text: &str) -> Option<u64> {
    let rest = text.strip_prefix(r#"{"id":"#)?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    rest[..digits.min(20)].parse().ok()
}

fn reply(id: u64, r: Result<Value, Fault>) -> String {
    match r {
        Ok(v) => json!({"id": id, "ok": v}),
        Err(f) => json!({"id": id, "err": f.to_json()}),
    }
    .to_string()
}

/// The host's state for one page.
pub struct Bridge {
    manifest: Manifest,
    app: AppInfo,
    granted: Vec<String>,
    subs: BTreeSet<String>,
}

impl Bridge {
    /// `granted`: what wold says this app holds; only names the manifest declares are kept.
    pub fn new(manifest: Manifest, app: AppInfo, granted: &[String]) -> Bridge {
        let mut b = Bridge {
            manifest,
            app,
            granted: Vec::new(),
            subs: BTreeSet::new(),
        };
        b.granted = b.keep(granted);
        b
    }

    fn keep(&self, granted: &[String]) -> Vec<String> {
        self.manifest
            .permissions
            .iter()
            .filter(|p| granted.contains(p))
            .cloned()
            .collect()
    }

    pub fn app(&self) -> &AppInfo {
        &self.app
    }

    pub fn granted(&self) -> &[String] {
        &self.granted
    }

    /// The `init` record the page has before its scripts run.
    pub fn init(&self) -> Value {
        json!({"app": self.app.to_json(), "permissions": self.granted})
    }

    /// Reads one page message.
    pub fn handle(&mut self, text: &str) -> Step {
        if text.len() > MAX_MESSAGE {
            // A call the page numbered is told, so its promise settles (QA-WEB-6): `bridge.js`
            // writes the id first. The channel already drops a text over the cap in UTF-16 units;
            // this one is over it in bytes (multi-byte characters).
            return match leading_id(text) {
                Some(id) => Step::Reply(reply(
                    id,
                    Err(Fault::new(
                        ErrorCode::Usage,
                        "The request was too large.",
                        "keep the whole request under 64 KiB",
                    )),
                )),
                None => Step::Nothing,
            };
        }
        let Ok(Value::Object(m)) = serde_json::from_str::<Value>(text) else {
            return Step::Nothing;
        };
        if let Some(t) = m.get("sub").and_then(Value::as_str) {
            if catalog::TOPICS.contains(&t) {
                self.subs.insert(t.to_owned());
                // The record as it is now (QA-WEB-2): the page subscribes as it loads, a reload
                // included, and its `init` is the one of the host's start.
                let now = match t {
                    "app" => Some(self.app.to_json()),
                    "permissions" => Some(json!(self.granted)),
                    _ => None,
                };
                if let Some(e) = now.and_then(|d| self.event(t, d)) {
                    return Step::Reply(e);
                }
            }
            return Step::Nothing;
        }
        if let Some(t) = m.get("unsub").and_then(Value::as_str) {
            self.subs.remove(t);
            return Step::Nothing;
        }
        let Some(id) = m.get("id").and_then(Value::as_u64) else {
            return Step::Nothing;
        };
        let method = m.get("call").and_then(Value::as_str).unwrap_or("");
        let params = match m.get("params") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(p)) => p.clone(),
            Some(_) => {
                return Step::Reply(reply(
                    id,
                    Err(Fault::new(
                        ErrorCode::Usage,
                        "The request was not understood.",
                        "params must be an object",
                    )),
                ));
            }
        };
        match self.allowed(method) {
            Ok(()) => Step::Call(Call {
                id,
                method: method.to_owned(),
                params,
            }),
            Err(f) => Step::Reply(reply(id, Err(f))),
        }
    }

    /// Whether this app may call `method` now.
    pub fn allowed(&self, method: &str) -> Result<(), Fault> {
        let Some(p) = catalog::permission_of(method) else {
            return Err(Fault::new(
                ErrorCode::NotFound,
                "This app asked for something WolOS does not offer.",
                format!("unknown method {method:?}"),
            ));
        };
        if !self.manifest.permissions.iter().any(|d| d == p.name) {
            return Err(Fault::new(
                ErrorCode::Denied,
                "This app did not ask for that permission.",
                format!("add {:?} to permissions in app.toml", p.name),
            ));
        }
        if !self.granted.iter().any(|g| g == p.name) {
            return Err(Fault::new(
                ErrorCode::Denied,
                "This app is not allowed to do that.",
                "Turn it on in Settings > Privacy",
            ));
        }
        Ok(())
    }

    /// The event message for `topic`, when the page subscribed to it.
    pub fn event(&self, topic: &str, data: Value) -> Option<String> {
        self.subs
            .contains(topic)
            .then(|| json!({"topic": topic, "data": data}).to_string())
    }

    /// The theme, accent or locale changed: the new record, and the `app` event when subscribed.
    pub fn set_app(&mut self, app: AppInfo) -> Option<String> {
        self.app = app;
        self.event("app", self.app.to_json())
    }

    /// wold's grants changed (Settings > Privacy).
    pub fn set_granted(&mut self, granted: &[String]) -> Option<String> {
        self.granted = self.keep(granted);
        self.event("permissions", json!(self.granted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;
    impl Wold for Echo {
        fn call(
            &self,
            app: &str,
            method: &str,
            params: &Map<String, Value>,
        ) -> Result<Value, Fault> {
            Ok(json!({"app": app, "method": method, "params": params}))
        }
    }

    fn bridge(granted: &[&str]) -> Bridge {
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
        Bridge::new(m, app, &g)
    }

    fn err_code(s: &Step) -> String {
        let Step::Reply(t) = s else { panic!("{s:?}") };
        let v: Value = serde_json::from_str(t).unwrap();
        v["err"]["code"].as_str().unwrap().to_owned()
    }

    #[test]
    fn only_declared_permissions_are_kept() {
        let b = bridge(&["audio.read", "system.read"]);
        assert_eq!(b.granted(), ["audio.read"]);
        assert_eq!(b.init()["permissions"], json!(["audio.read"]));
        assert_eq!(b.init()["app"]["theme"], "light");
    }

    #[test]
    fn calls_pass_only_when_declared_and_granted() {
        let mut b = bridge(&["audio.read"]);
        let s = b.handle(r#"{"id":1,"call":"audio.doctor"}"#);
        let Step::Call(c) = s else { panic!("{s:?}") };
        let out: Value = serde_json::from_str(&c.run("org.wolos.webapp.t", &Echo)).unwrap();
        assert_eq!(out["id"], 1);
        assert_eq!(out["ok"]["method"], "audio.doctor");
        assert_eq!(
            err_code(&b.handle(r#"{"id":2,"call":"games.doctor"}"#)),
            "denied"
        );
        assert_eq!(err_code(&b.handle(r#"{"id":3,"call":"about"}"#)), "denied");
        assert_eq!(
            err_code(&b.handle(r#"{"id":4,"call":"rm.rf"}"#)),
            "not-found"
        );
        assert_eq!(
            err_code(&b.handle(r#"{"id":5,"call":"audio.get","params":[1]}"#)),
            "usage"
        );
        assert_eq!(b.handle("not json"), Step::Nothing);
        assert_eq!(b.handle(r#"{"call":"audio.get"}"#), Step::Nothing);
        assert_eq!(b.handle(&"x".repeat(MAX_MESSAGE + 1)), Step::Nothing);
        // Over the cap with an id: the page is told (QA-WEB-6), so its promise settles.
        let big = format!(
            r#"{{"id":7,"call":"audio.get","params":{{"s":"{}"}}}}"#,
            "é".repeat(MAX_MESSAGE)
        );
        let out: Value = match b.handle(&big) {
            Step::Reply(r) => serde_json::from_str(&r).unwrap(),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            (out["id"].as_u64(), out["err"]["code"].as_str()),
            (Some(7), Some("usage"))
        );
        assert_eq!(leading_id(r#"{"id":12,"call":"x"}"#), Some(12));
        assert_eq!(leading_id(r#"{"call":"x","id":12}"#), None);
        assert_eq!(leading_id(r#"{"id":99999999999999999999999999,"#), None);
    }

    /// QA-WEB-2: a page subscribes as it loads (a reload too) and gets the record as it is now,
    /// not the `init` of the host's start: the dark mode set before it subscribed is the first
    /// event it sees. The accent of a theme colour is the id Settings names.
    #[test]
    fn a_subscribe_answers_with_the_current_record() {
        let mut b = bridge(&[]);
        let mut dark = b.app().clone();
        dark.theme = Theme::Dark;
        dark.accent = "green".into();
        assert_eq!(b.set_app(dark), None, "not subscribed yet");
        let Step::Reply(r) = b.handle(r#"{"sub":"app"}"#) else {
            panic!("no record on subscribe");
        };
        let e: Value = serde_json::from_str(&r).unwrap();
        assert_eq!(e["topic"], "app");
        assert_eq!(
            (e["data"]["theme"].as_str(), e["data"]["accent"].as_str()),
            (Some("dark"), Some("green"))
        );
        assert_eq!(accent_id(true, 0x6bc681), "green");
        assert_eq!(accent_id(false, 0x00863c), "green");
        assert_eq!(accent_id(false, 0x7452f5), "violet");
        assert_eq!(
            accent_id(true, 0x123456),
            "violet",
            "a colour no accent has: the default"
        );
    }

    #[test]
    fn events_reach_only_subscribed_pages() {
        let mut b = bridge(&[]);
        assert_eq!(b.set_granted(&["games.read".into()]), None);
        assert_eq!(
            b.handle(r#"{"sub":"permissions"}"#),
            Step::Reply(json!({"topic": "permissions", "data": ["games.read"]}).to_string())
        );
        assert_eq!(b.handle(r#"{"sub":"secrets"}"#), Step::Nothing);
        assert!(b.event("secrets", json!(1)).is_none());
        let e: Value =
            serde_json::from_str(&b.set_granted(&["audio.read".into()]).unwrap()).unwrap();
        assert_eq!(e, json!({"topic": "permissions", "data": ["audio.read"]}));
        b.handle(r#"{"unsub":"permissions"}"#);
        assert!(b.event("permissions", json!([])).is_none());
    }
}
