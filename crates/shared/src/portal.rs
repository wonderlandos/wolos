//! libwol -> wold service calls (ADR 0020): `call <json>` on the wold socket, one [`Answer`] line back.
//! wold decides with the caller's identity from the kernel (peer pid -> executable), never from the
//! message. Every field is bounded and checked here, on both sides.
use serde::{Deserialize, Serialize};

/// Largest call line wold reads.
pub const MAX_LINE: usize = 16 * 1024;
pub const MAX_TITLE: usize = 256;
pub const MAX_BODY: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "call", deny_unknown_fields)]
pub enum Call {
    /// Show a notification in the caller's session.
    #[serde(rename = "notification.post")]
    NotificationPost {
        title: String,
        #[serde(default)]
        body: String,
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
}

impl Answer {
    pub fn ok() -> Self {
        Self {
            ok: true,
            code: None,
            message: None,
        }
    }

    pub fn err(code: Code, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            code: Some(code),
            message: Some(message.into()),
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

fn clean(s: &str, max: usize, newlines: bool) -> bool {
    s.len() <= max
        && !s
            .chars()
            .any(|c| c.is_control() && !(newlines && c == '\n'))
}

impl Call {
    /// Capability (`wol_core::consent::CAPABILITIES`) the caller needs.
    pub fn capability(&self) -> &'static str {
        match self {
            Self::NotificationPost { .. } => "notifications",
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::NotificationPost { title, body } => {
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
    fn encode_roundtrips_and_names_capability() {
        let c = post("Build done", "3 warnings");
        assert_eq!(Call::decode(c.encode().as_bytes()), Ok(c.clone()));
        assert_eq!(c.capability(), "notifications");
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
