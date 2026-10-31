//! libwol -> wold service calls (ADR 0020): `call <json>` on the wold socket, one [`Answer`] line back.
//! wold decides with the caller's identity from the kernel (peer pid -> executable), never from the
//! message. Every field is bounded and checked here, on both sides.
use serde::{Deserialize, Serialize};

/// Largest call line wold reads.
pub const MAX_LINE: usize = 16 * 1024;
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
    /// The person pressed a notification button (from the shell): wold tells the app that posted it.
    #[serde(rename = "notification.answer")]
    NotificationAnswer { id: u32, key: String },
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
}

impl Answer {
    pub fn ok() -> Self {
        Self {
            ok: true,
            code: None,
            message: None,
            id: None,
        }
    }

    pub fn err(code: Code, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            code: Some(code),
            message: Some(message.into()),
            id: None,
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

impl Call {
    pub fn need(&self) -> Need {
        match self {
            Self::NotificationPost { .. }
            | Self::NotificationReplace { .. }
            | Self::NotificationWithdraw { .. } => Need::Grant("notifications"),
            Self::NotificationAnswer { .. } => Need::Shower,
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
            Self::NotificationWithdraw { .. } => Ok(()),
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
