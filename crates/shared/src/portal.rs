//! libwol -> wold service calls (ADR 0020): `call <json>` on the wold socket, one [`Answer`] line back.
//! wold decides with the caller's identity from the kernel (peer pid -> executable), never from the
//! message. Every field is bounded and checked here, on both sides.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Largest call line wold reads.
pub const MAX_LINE: usize = 60 * 1024;
/// Clipboard text through wold.
pub const MAX_CLIPBOARD: usize = 48 * 1024;
pub const MAX_TITLE: usize = 256;
pub const MAX_BODY: usize = 4096;
/// Buttons a notification may carry (the banner shows two; the rest are in the list).
pub const MAX_ACTIONS: usize = 3;
pub const MAX_ACTION_KEY: usize = 32;
pub const MAX_ACTION_LABEL: usize = 64;

/// A notification button: `key` comes back to the app when it is pressed. Key `default` is a
/// click on the banner itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteAction {
    pub key: String,
    pub label: String,
}

/// App preferences: a key, its value, keys per app.
pub const MAX_PREF_KEY: usize = 64;
pub const MAX_PREF_VALUE: usize = 4096;
pub const MAX_PREFS: usize = 256;

/// A panel's title; a suggested file name.
pub const MAX_NAME: usize = 255;

/// Keep-awake holds per account.
pub const MAX_HOLDS: usize = 16;

/// Every capability a [`Call`] can need (`wol_core::consent::CAPABILITIES`), test-checked.
pub const CAPABILITIES: &[&str] = &[
    "notifications",
    "clipboard-read",
    "clipboard-write",
    "app-preferences",
    "file-panels",
    "power-info",
    "keep-awake",
];

/// What wold checks before a call runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// A consent grant for the caller's executable (`wol_core::consent::CAPABILITIES`).
    Grant(&'static str),
    /// The caller is the session's notification shower (the shell).
    Shower,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "call", deny_unknown_fields)]
pub enum Call {
    /// Show a notification in the caller's session.
    #[serde(rename = "notification.post")]
    NotificationPost {
        title: String,
        #[serde(default)]
        body: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        actions: Vec<NoteAction>,
    },
    /// New text and buttons for a notification this process posted.
    #[serde(rename = "notification.replace")]
    NotificationReplace {
        id: u32,
        title: String,
        #[serde(default)]
        body: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        actions: Vec<NoteAction>,
    },
    /// Take back a notification this process posted.
    #[serde(rename = "notification.withdraw")]
    NotificationWithdraw { id: u32 },
    /// The text on the clipboard.
    #[serde(rename = "clipboard.read")]
    ClipboardRead {},
    /// Put text on the clipboard.
    #[serde(rename = "clipboard.write")]
    ClipboardWrite { text: String },
    /// A preference of the calling app (its own: wold takes the app from the kernel).
    #[serde(rename = "prefs.get")]
    PrefsGet { key: String },
    #[serde(rename = "prefs.set")]
    PrefsSet { key: String, value: String },
    #[serde(rename = "prefs.remove")]
    PrefsRemove { key: String },
    /// The keys the calling app has set.
    #[serde(rename = "prefs.keys")]
    PrefsKeys {},
    /// Let the person pick a file; only that file reaches the app, as an open descriptor.
    #[serde(rename = "panel.open")]
    PanelOpen { title: String },
    /// Let the person pick where to save; the app gets that file, opened for writing.
    #[serde(rename = "panel.save")]
    PanelSave { title: String, name: String },
    /// Battery and power state.
    #[serde(rename = "power.info")]
    PowerInfo {},
    /// Keep the machine awake until this connection closes.
    #[serde(rename = "power.keep-awake")]
    PowerKeepAwake { reason: String },
    /// The desktop's reply to an [`Ask`] (from the shell).
    #[serde(rename = "session.answer")]
    SessionAnswer {
        id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
    },
    /// The person pressed a notification button (from the shell): wold tells the app that posted it.
    #[serde(rename = "notification.answer")]
    NotificationAnswer { id: u32, key: String },
}

/// What wold asks the person's desktop to do once a call passed its check: wold is a system
/// daemon, so anything of the session runs in the shell. Reaches only the shell of the caller's
/// account (`Event::SessionAsk`); the shell replies with [`Call::SessionAnswer`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ask", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Ask {
    /// A file panel. Answer: `{"name": ...}` with the open file attached, or `{"cancelled": true}`.
    Panel {
        save: bool,
        title: String,
        #[serde(default)]
        name: String,
    },
    ClipboardRead,
    ClipboardWrite {
        text: String,
    },
}

/// Why a call did not happen. Same numbers as `wol_status` in `wol.h`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Code {
    InvalidArgument,
    PermissionDenied,
    Unavailable,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// What the call made (a notification's id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// What the call returns, when it returns more than an id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Answer {
    pub fn ok() -> Self {
        Self {
            ok: true,
            code: None,
            message: None,
            id: None,
            data: None,
        }
    }

    pub fn with(data: Value) -> Self {
        let mut a = Self::ok();
        a.data = Some(data);
        a
    }

    pub fn err(code: Code, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            code: Some(code),
            message: Some(message.into()),
            id: None,
            data: None,
        }
    }

    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_default();
        s.push('\n');
        s
    }

    /// A reply that is not exactly an answer reads as `Internal`.
    pub fn parse(line: &str) -> Self {
        serde_json::from_str::<Answer>(line.trim())
            .ok()
            .filter(|a| a.ok == a.code.is_none())
            .unwrap_or_else(|| {
                Self::err(
                    Code::Internal,
                    "The system service sent an unreadable reply.",
                )
            })
    }
}

/// `[a-z0-9_-]{1,32}`.
pub fn action_key_ok(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= MAX_ACTION_KEY
        && k.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn clean(s: &str, max: usize, newlines: bool) -> bool {
    s.len() <= max
        && !s
            .chars()
            .any(|c| c.is_control() && !(newlines && c == '\n'))
}

fn panel_title(t: &str) -> Result<(), String> {
    if !clean(t, MAX_TITLE, false) {
        return Err(format!(
            "A panel title is one line of at most {MAX_TITLE} bytes."
        ));
    }
    Ok(())
}

/// `[A-Za-z0-9._-]{1,64}`.
fn pref_key(k: &str) -> Result<(), String> {
    let ok = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-');
    if k.is_empty() || k.len() > MAX_PREF_KEY || !k.bytes().all(ok) {
        return Err(format!(
            "A preference key is 1 to {MAX_PREF_KEY} letters, digits, '.', '_' or '-'."
        ));
    }
    Ok(())
}

impl Call {
    pub fn need(&self) -> Need {
        match self {
            Self::NotificationPost { .. }
            | Self::NotificationReplace { .. }
            | Self::NotificationWithdraw { .. } => Need::Grant("notifications"),
            Self::PrefsGet { .. }
            | Self::PrefsSet { .. }
            | Self::PrefsRemove { .. }
            | Self::PrefsKeys {} => Need::Grant("app-preferences"),
            Self::PanelOpen { .. } | Self::PanelSave { .. } => Need::Grant("file-panels"),
            Self::PowerInfo {} => Need::Grant("power-info"),
            Self::PowerKeepAwake { .. } => Need::Grant("keep-awake"),
            Self::NotificationAnswer { .. } | Self::SessionAnswer { .. } => Need::Shower,
            Self::ClipboardRead {} => Need::Grant("clipboard-read"),
            Self::ClipboardWrite { .. } => Need::Grant("clipboard-write"),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::NotificationAnswer { key, .. } => {
                if !action_key_ok(key) {
                    return Err(
                        "An action key is up to 32 lowercase letters, digits, '-' or '_'.".into(),
                    );
                }
                Ok(())
            }
            Self::NotificationWithdraw { .. } | Self::ClipboardRead {} => Ok(()),
            Self::ClipboardWrite { text } => {
                if text.len() > MAX_CLIPBOARD || text.contains('\0') {
                    return Err(format!(
                        "Clipboard text is at most {MAX_CLIPBOARD} bytes, with no NUL."
                    ));
                }
                Ok(())
            }
            Self::PrefsKeys {} => Ok(()),
            Self::PrefsGet { key } | Self::PrefsRemove { key } => pref_key(key),
            Self::PrefsSet { key, value } => {
                pref_key(key)?;
                if value.len() > MAX_PREF_VALUE || value.contains('\0') {
                    return Err(format!(
                        "A preference is at most {MAX_PREF_VALUE} bytes, with no NUL."
                    ));
                }
                Ok(())
            }
            Self::PanelOpen { title } => panel_title(title),
            Self::PanelSave { title, name } => {
                panel_title(title)?;
                if name.is_empty() || name.contains('/') || !clean(name, MAX_NAME, false) {
                    return Err(format!(
                        "A file name is 1 to {MAX_NAME} bytes, with no '/'."
                    ));
                }
                Ok(())
            }
            Self::PowerInfo {} => Ok(()),
            Self::PowerKeepAwake { reason } => {
                if reason.trim().is_empty() || !clean(reason, MAX_TITLE, false) {
                    return Err(format!(
                        "A reason is one line of 1 to {MAX_TITLE} bytes: the person sees it."
                    ));
                }
                Ok(())
            }
            Self::SessionAnswer { error, .. } => {
                if error.as_ref().is_some_and(|m| !clean(m, MAX_BODY, true)) {
                    return Err("A desktop answer's message is short text.".into());
                }
                Ok(())
            }
            Self::NotificationPost {
                title,
                body,
                actions,
            }
            | Self::NotificationReplace {
                title,
                body,
                actions,
                ..
            } => {
                if actions.len() > MAX_ACTIONS {
                    return Err(format!("A notification has at most {MAX_ACTIONS} buttons."));
                }
                for (i, a) in actions.iter().enumerate() {
                    if !action_key_ok(&a.key) {
                        return Err(
                            "An action key is up to 32 lowercase letters, digits, '-' or '_'."
                                .into(),
                        );
                    }
                    if a.label.trim().is_empty() || !clean(&a.label, MAX_ACTION_LABEL, false) {
                        return Err(format!(
                            "A button label is one line of 1 to {MAX_ACTION_LABEL} bytes."
                        ));
                    }
                    if actions[..i].iter().any(|b| b.key == a.key) {
                        return Err(format!("The action key \"{}\" is used twice.", a.key));
                    }
                }
                if title.trim().is_empty() {
                    return Err("A notification needs a title.".into());
                }
                if !clean(title, MAX_TITLE, false) {
                    return Err(format!(
                        "A notification title is one line of at most {MAX_TITLE} bytes."
                    ));
                }
                if !clean(body, MAX_BODY, true) {
                    return Err(format!(
                        "A notification body is text of at most {MAX_BODY} bytes."
                    ));
                }
                Ok(())
            }
        }
    }

    /// JSON, no newline. The caller checks [`Call::validate`] first.
    pub fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Bounded, UTF-8, exactly one known call with no extra keys, valid fields.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_LINE {
            return Err("The request is too long.".into());
        }
        let text =
            std::str::from_utf8(bytes).map_err(|_| "The request is not UTF-8.".to_owned())?;
        let call: Call = serde_json::from_str(text.trim())
            .map_err(|_| "The request is not a known call.".to_owned())?;
        call.validate()?;
        Ok(call)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(t: &str, b: &str) -> Call {
        Call::NotificationPost {
            title: t.into(),
            body: b.into(),
            actions: Vec::new(),
        }
    }

    #[test]
    fn decode_table() {
        let long = "x".repeat(MAX_TITLE + 1);
        let cases: Vec<(String, Option<Call>)> = vec![
            (
                r#"{"call":"notification.post","title":"Hi","body":"a\nb"}"#.into(),
                Some(post("Hi", "a\nb")),
            ),
            (
                r#"{"call":"notification.post","title":"Hi"}"#.into(),
                Some(post("Hi", "")),
            ),
            (r#"{"call":"notification.post","title":"  "}"#.into(), None),
            (
                r#"{"call":"notification.post","title":"a\nb"}"#.into(),
                None,
            ),
            (
                r#"{"call":"notification.post","title":"a\u0007"}"#.into(),
                None,
            ),
            (
                format!(r#"{{"call":"notification.post","title":"{long}"}}"#),
                None,
            ),
            (
                r#"{"call":"notification.post","title":"a","app":"x"}"#.into(),
                None,
            ),
            (r#"{"call":"files.delete","title":"a"}"#.into(), None),
            ("{}".into(), None),
            ("not json".into(), None),
        ];
        for (line, want) in cases {
            assert_eq!(Call::decode(line.as_bytes()).ok(), want, "{line}");
        }
        assert!(Call::decode(&[0xff, 0xfe]).is_err());
        assert!(Call::decode(&vec![b' '; MAX_LINE + 1]).is_err());
    }

    #[test]
    fn actions_and_answers() {
        let act = |k: &str, l: &str| NoteAction {
            key: k.into(),
            label: l.into(),
        };
        let with = |actions: Vec<NoteAction>| Call::NotificationPost {
            title: "t".into(),
            body: String::new(),
            actions,
        };
        assert!(
            with(vec![act("reply", "Reply"), act("default", "Open")])
                .validate()
                .is_ok()
        );
        for bad in [
            vec![act("Bad Key", "x")],
            vec![act("a", "")],
            vec![act("a", "x"), act("a", "y")],
            vec![act("a", "1"), act("b", "2"), act("c", "3"), act("d", "4")],
        ] {
            assert!(with(bad.clone()).validate().is_err(), "{bad:?}");
        }
        let c = with(vec![act("ok", "OK")]);
        assert_eq!(Call::decode(c.encode().as_bytes()), Ok(c));
        assert_eq!(
            post("t", "").encode(),
            r#"{"call":"notification.post","title":"t","body":""}"#,
            "1.0 wire unchanged without actions"
        );
        let a = Call::NotificationAnswer {
            id: 3,
            key: "ok".into(),
        };
        assert_eq!(Call::decode(a.encode().as_bytes()), Ok(a.clone()));
        assert_eq!(a.need(), Need::Shower);
        assert_eq!(post("t", "").need(), Need::Grant("notifications"));
        let mut ok = Answer::ok();
        ok.id = Some(7);
        assert_eq!(ok.to_line(), "{\"ok\":true,\"id\":7}\n");
        assert_eq!(Answer::parse(&ok.to_line()), ok);
    }

    #[test]
    fn clipboard_calls() {
        let w = Call::ClipboardWrite {
            text: "two\nlines\ttab é".into(),
        };
        assert_eq!(Call::decode(w.encode().as_bytes()), Ok(w.clone()));
        assert_eq!(w.need(), Need::Grant("clipboard-write"));
        let r = Call::decode(br#"{"call":"clipboard.read"}"#).unwrap();
        assert_eq!(r, Call::ClipboardRead {});
        assert_eq!(r.need(), Need::Grant("clipboard-read"));
        assert!(
            Call::ClipboardWrite {
                text: "x".repeat(MAX_CLIPBOARD + 1)
            }
            .validate()
            .is_err()
        );
        assert!(
            Call::ClipboardWrite {
                text: "a\0b".into()
            }
            .validate()
            .is_err()
        );
        assert!(Call::decode(br#"{"call":"clipboard.read","uid":0}"#).is_err());
    }

    #[test]
    fn session_answers_come_only_from_the_desktop() {
        let a = Call::SessionAnswer {
            id: 1,
            error: None,
            data: Some(serde_json::json!({"text": "hi"})),
        };
        assert_eq!(Call::decode(a.encode().as_bytes()), Ok(a.clone()));
        assert_eq!(a.need(), Need::Shower);
        let long = Call::SessionAnswer {
            id: 1,
            error: Some("x".repeat(MAX_BODY + 1)),
            data: None,
        };
        assert!(long.validate().is_err());
        let done = Answer::with(serde_json::json!({"text": "hi"}));
        assert_eq!(done.to_line(), "{\"ok\":true,\"data\":{\"text\":\"hi\"}}\n");
        assert_eq!(Answer::parse(&done.to_line()), done);
        let line = serde_json::to_string(&Ask::ClipboardRead).unwrap();
        assert_eq!(line, r#"{"ask":"clipboard-read"}"#);
    }

    #[test]
    fn prefs_calls() {
        let s = Call::PrefsSet {
            key: "window.width".into(),
            value: "any text\n".into(),
        };
        assert_eq!(Call::decode(s.encode().as_bytes()), Ok(s.clone()));
        assert_eq!(s.need(), Need::Grant("app-preferences"));
        for key in ["", "a/b", "../x", "a b", "é", &"k".repeat(MAX_PREF_KEY + 1)] {
            let c = Call::PrefsGet { key: key.into() };
            assert!(c.validate().is_err(), "{key}");
        }
        let long = Call::PrefsSet {
            key: "k".into(),
            value: "x".repeat(MAX_PREF_VALUE + 1),
        };
        assert!(long.validate().is_err());
        assert!(Call::decode(br#"{"call":"prefs.get","key":"k","app":"/usr/bin/x"}"#).is_err());
    }

    #[test]
    fn panel_calls() {
        let o = Call::PanelOpen {
            title: "Open a report".into(),
        };
        assert_eq!(Call::decode(o.encode().as_bytes()), Ok(o.clone()));
        assert_eq!(o.need(), Need::Grant("file-panels"));
        let save = |t: &str, n: &str| Call::PanelSave {
            title: t.into(),
            name: n.into(),
        };
        assert!(save("Save", "report.txt").validate().is_ok());
        for bad in [
            save("Save", ""),
            save("Save", "../up"),
            save("Save", "a\nb"),
            save("a\nb", "x"),
            save(&"t".repeat(MAX_TITLE + 1), "x"),
            save("t", &"n".repeat(MAX_NAME + 1)),
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn power_calls() {
        let k = Call::PowerKeepAwake {
            reason: "Exporting".into(),
        };
        assert_eq!(Call::decode(k.encode().as_bytes()), Ok(k.clone()));
        assert_eq!(k.need(), Need::Grant("keep-awake"));
        assert_eq!(Call::PowerInfo {}.need(), Need::Grant("power-info"));
        for reason in ["", " ", "a\nb", &"x".repeat(MAX_TITLE + 1)] {
            let c = Call::PowerKeepAwake {
                reason: reason.into(),
            };
            assert!(c.validate().is_err(), "{reason}");
        }
    }

    #[test]
    fn replace_and_withdraw() {
        let r = Call::NotificationReplace {
            id: 3,
            title: "Copying 2 of 5".into(),
            body: String::new(),
            actions: Vec::new(),
        };
        assert_eq!(Call::decode(r.encode().as_bytes()), Ok(r.clone()));
        assert_eq!(r.need(), Need::Grant("notifications"));
        let w = Call::NotificationWithdraw { id: 3 };
        assert_eq!(Call::decode(w.encode().as_bytes()), Ok(w.clone()));
        assert_eq!(w.need(), Need::Grant("notifications"));
        for bad in [
            r#"{"call":"notification.replace","id":1,"title":" "}"#,
            r#"{"call":"notification.replace","id":-1,"title":"x"}"#,
            r#"{"call":"notification.replace","title":"x"}"#,
            r#"{"call":"notification.replace","id":1,"title":"a
b"}"#,
            r#"{"call":"notification.withdraw"}"#,
            r#"{"call":"notification.withdraw","id":1,"app":"x"}"#,
        ] {
            assert!(Call::decode(bad.as_bytes()).is_err(), "{bad}");
        }
    }

    #[test]
    fn encode_roundtrips_and_names_capability() {
        let c = post("Build done", "3 warnings");
        assert_eq!(Call::decode(c.encode().as_bytes()), Ok(c.clone()));
        assert_eq!(c.need(), Need::Grant("notifications"));
    }

    #[test]
    fn answer_wire() {
        assert_eq!(Answer::ok().to_line(), "{\"ok\":true}\n");
        let d = Answer::err(Code::PermissionDenied, "No.");
        assert_eq!(
            d.to_line(),
            "{\"ok\":false,\"code\":\"permission-denied\",\"message\":\"No.\"}\n"
        );
        assert_eq!(Answer::parse(&d.to_line()), d);
        for bad in [
            "",
            "{\"ok\":false}",
            "{\"ok\":true,\"code\":\"internal\"}",
            "[]",
        ] {
            assert_eq!(Answer::parse(bad).code, Some(Code::Internal), "{bad}");
        }
    }
}
