// SPDX-License-Identifier: BSD-2-Clause
//! Atlas Host Protocol, version 2 (ADR 0043 D4, docs/modules/atlas.md): what the browser's host
//! component and `atlas-bridge` say to each other over `$XDG_RUNTIME_DIR/atlas/bridge.sock`.
//!
//! The bridge speaks first (ADR 0033, QA-A9-SEC-1): `hello <nonce>`, which the browser echoes
//! (`shared::jobs::answer_hello`); the bridge checks that every byte of the echo was written by the
//! process that connected (SCM_CREDENTIALS). Then one JSON object per line, UTF-8, at most
//! [`MAX_LINE`] bytes with the newline. The browser sends [`Request::Hello`] first; every later
//! request carries an `id` (1 or more) and gets exactly one [`Reply`] with that id, in the order
//! the answers are ready (a share sheet that waits for the person does not hold back a Trash).
//! Errors that belong to no request use id 0.
//!
//! The encoding is canonical: [`Request::encode`] and [`Reply::encode`] write the fields in the
//! order of the fixtures in `tests/fixtures/atlas/`, which the browser side tests byte for byte.
//! An error's text never names a path: the browser knows which file it asked about.
use serde::{Deserialize, Serialize};

/// The protocol version this side speaks.
pub const VERSION: u32 = 2;
/// Directory under `$XDG_RUNTIME_DIR` (0700) and the socket in it (0600).
pub const SOCKET_DIR: &str = "atlas";
/// The socket's file name in that directory.
pub const SOCKET_NAME: &str = "bridge.sock";
/// Longest line either side sends, newline included: a path of [`crate::portal::MAX_PATH`] bytes
/// with every byte escaped still fits.
pub const MAX_LINE: usize = 32 * 1024;
/// Requests one connection may have waiting at once; the next is answered [`ErrorCode::Busy`].
pub const MAX_IN_FLIGHT: usize = 4;
/// The one program that is the browser (checked through a pinned `/proc/<pid>`).
pub const BROWSER: &str = "/opt/atlas/atlas";

/// What the browser asks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// First line of a connection: the version the browser speaks and its own pid (which must be
    /// the pid the kernel reports for the socket).
    Hello { version: u32, pid: i32 },
    /// Show the share sheet for a file.
    Share { id: u32, path: String },
    /// Let the person choose the app that opens a file.
    OpenWith { id: u32, path: String },
    /// Move a file or folder to the Trash.
    Trash { id: u32, path: String },
}

/// The three verbs, for texts and logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    /// Show the share sheet.
    Share,
    /// Let the person choose the app.
    OpenWith,
    /// Move to the Trash.
    Trash,
}

/// What the bridge answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    /// Answer to [`Request::Hello`]: the version the bridge speaks on this connection.
    Hello { version: u32 },
    /// The verb ran; `outcome` says what the person did.
    Ok { id: u32, outcome: Outcome },
    /// The verb did not run (or the line was refused); `text` is shown to the person as is.
    Error {
        id: u32,
        code: ErrorCode,
        text: String,
    },
}

/// What happened, as far as the browser may know. Share: copied, saved, opened or cancelled.
/// Open With: opened or cancelled. Trash: trashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum Outcome {
    /// Copied to the clipboard.
    Copied,
    /// Saved to a place the person chose.
    Saved,
    /// Opened in an app.
    Opened,
    /// The person closed the sheet.
    Cancelled,
    /// Moved to the Trash.
    Trashed,
}

/// Why a request did not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum ErrorCode {
    /// Not a request of this version (bad JSON, unknown op, id 0, a path that is not a full path).
    InvalidRequest,
    /// The hello's version is not one the bridge speaks; the connection closes.
    Version,
    /// The peer is not the browser, or its hello came late or named another pid; the connection
    /// closes.
    NotBrowser,
    /// The person has not allowed Atlas this (System Settings, Privacy & Security).
    PermissionDenied,
    /// The verb cannot run on this system now: the desktop is not running, or this WolOS is older
    /// than the verb (Share and Trash need libwol 1.15).
    Unavailable,
    /// The file is not there any more.
    NotFound,
    /// Something else is waiting for the person; try again after it.
    Busy,
    /// The desktop refused or failed (a folder to share, the home folder to the Trash).
    Failed,
}

impl Outcome {
    /// The `done` a share or open-with answer carries ("cancelled" for anything else, as libwol).
    pub fn of_done(done: Option<&str>) -> Self {
        match done {
            Some("copied") => Self::Copied,
            Some("saved") => Self::Saved,
            Some("opened") => Self::Opened,
            Some(_) | None => Self::Cancelled,
        }
    }
}

impl Request {
    /// The id of a verb; None for the hello.
    pub fn id(&self) -> Option<u32> {
        match self {
            Self::Hello { .. } => None,
            Self::Share { id, .. } | Self::OpenWith { id, .. } | Self::Trash { id, .. } => {
                Some(*id)
            }
        }
    }

    /// The verb and its path; None for the hello.
    pub fn verb(&self) -> Option<(Verb, &str)> {
        match self {
            Self::Hello { .. } => None,
            Self::Share { path, .. } => Some((Verb::Share, path)),
            Self::OpenWith { path, .. } => Some((Verb::OpenWith, path)),
            Self::Trash { path, .. } => Some((Verb::Trash, path)),
        }
    }

    /// One line, newline included.
    pub fn encode(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_default();
        s.push('\n');
        s
    }

    /// Read one line. On refusal: the id to answer (the line's `id` if it had a usable one, else 0)
    /// and an [`ErrorCode::InvalidRequest`] reply.
    pub fn decode(line: &[u8]) -> Result<Self, Reply> {
        let refuse = |id: u32| Reply::error(id, ErrorCode::InvalidRequest, TEXT_INVALID);
        if line.len() > MAX_LINE {
            return Err(refuse(0));
        }
        let Ok(text) = std::str::from_utf8(line) else {
            return Err(refuse(0));
        };
        let text = text.strip_suffix('\n').unwrap_or(text);
        // Only an object: serde would also read a tagged enum from an array.
        if !text.starts_with('{') {
            return Err(refuse(0));
        }
        let r: Self = match serde_json::from_str(text) {
            Ok(r) => r,
            Err(_) => {
                // Answer on the request's own id when one can be read, so the browser can match it.
                let id = serde_json::from_str::<serde_json::Value>(text)
                    .ok()
                    .and_then(|v| v.get("id")?.as_u64())
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0);
                return Err(refuse(id));
            }
        };
        if let Some(id) = r.id() {
            if id == 0 {
                return Err(refuse(0));
            }
            if let Some((_, path)) = r.verb()
                && crate::portal::file_path(path).is_err()
            {
                return Err(refuse(id));
            }
        }
        if let Self::Hello { version, pid } = r
            && (version == 0 || pid <= 0)
        {
            return Err(refuse(0));
        }
        Ok(r)
    }
}

impl Reply {
    /// An error reply for request `id` (0 when the line belongs to no request).
    pub fn error(id: u32, code: ErrorCode, text: &str) -> Self {
        Self::Error {
            id,
            code,
            text: text.to_owned(),
        }
    }

    /// One line, newline included.
    pub fn encode(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_default();
        s.push('\n');
        s
    }

    /// What the browser reads: one line, refused if it is not exactly a reply of this version.
    pub fn decode(line: &[u8]) -> Option<Self> {
        if line.len() > MAX_LINE {
            return None;
        }
        let text = std::str::from_utf8(line).ok()?;
        let text = text.strip_suffix('\n').unwrap_or(text);
        if !text.starts_with('{') {
            return None;
        }
        let r: Self = serde_json::from_str(text).ok()?;
        match &r {
            Self::Hello { version } if *version == 0 => None,
            Self::Ok { id, .. } if *id == 0 => None,
            Self::Error { text, .. } if text.trim().is_empty() => None,
            Self::Hello { .. } | Self::Ok { .. } | Self::Error { .. } => Some(r),
        }
    }
}

// The texts the person reads (hig.md section 3: sentence case, no exclamation marks, the person
// as the subject where there is one). None names a path.

/// A line that is not a request of this version.
pub const TEXT_INVALID: &str = "Atlas sent a request WolOS does not understand.";
/// A hello whose version the bridge does not speak.
pub const TEXT_VERSION: &str =
    "This Atlas and this WolOS do not match. Update both to share files and use the Trash.";
/// A peer that is not the browser.
pub const TEXT_NOT_BROWSER: &str = "Only Atlas can use this service.";
/// wold or the desktop cannot be reached.
pub const TEXT_UNREACHABLE: &str = "WolOS could not reach the desktop. Try again in a moment.";
/// The path is not there.
pub const TEXT_NOT_FOUND: &str =
    "The file is not there any more. It may have been moved or deleted.";
/// Another sheet is on screen, or the connection has too many verbs waiting.
pub const TEXT_BUSY: &str =
    "Something else is waiting for you to choose. Finish it, then try again.";
/// The bridge already has as many connections open as it takes (16).
pub const TEXT_TOO_MANY: &str =
    "Atlas has too many connections open to WolOS. Close some and try again.";

/// The text for a verb's error. `code` is one of the verb errors; others get their own text.
pub fn text(verb: Verb, code: ErrorCode) -> &'static str {
    match (code, verb) {
        (ErrorCode::PermissionDenied, Verb::Trash) => {
            "You have not allowed Atlas to move files to the Trash. You can allow it in System Settings, Privacy & Security."
        }
        (ErrorCode::PermissionDenied, _) => {
            "You have not allowed Atlas to share files or open them in other apps. You can allow it in System Settings, Privacy & Security."
        }
        (ErrorCode::Unavailable, Verb::Share) => "Sharing a file from Atlas needs a newer WolOS.",
        (ErrorCode::Unavailable, Verb::Trash) => {
            "Moving to the Trash from Atlas needs a newer WolOS."
        }
        (ErrorCode::Unavailable, Verb::OpenWith) => TEXT_UNREACHABLE,
        (ErrorCode::Failed, Verb::Share) => "This item cannot be shared. Only a file can.",
        (ErrorCode::Failed, Verb::OpenWith) => "No app on this computer opens this kind of file.",
        (ErrorCode::Failed, Verb::Trash) => "This item cannot be moved to the Trash.",
        (ErrorCode::NotFound, _) => TEXT_NOT_FOUND,
        (ErrorCode::Busy, _) => TEXT_BUSY,
        (ErrorCode::InvalidRequest, _) => TEXT_INVALID,
        (ErrorCode::Version, _) => TEXT_VERSION,
        (ErrorCode::NotBrowser, _) => TEXT_NOT_BROWSER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_refuses_and_keeps_the_id() {
        let bad = |l: &str| Request::decode(l.as_bytes()).unwrap_err();
        let e = |id| Reply::error(id, ErrorCode::InvalidRequest, TEXT_INVALID);
        assert_eq!(bad(r#"{"op":"share","id":3,"path":"a/b"}"#), e(3));
        assert_eq!(bad(r#"{"op":"share","id":3,"path":"/a/../b"}"#), e(3));
        assert_eq!(bad(r#"{"op":"print","id":9,"path":"/a"}"#), e(9));
        assert_eq!(bad(r#"{"op":"share","id":0,"path":"/a"}"#), e(0));
        assert_eq!(bad(r#"{"op":"share","id":4294967296,"path":"/a"}"#), e(0));
        assert_eq!(bad(r#"{"op":"hello","version":0,"pid":1}"#), e(0));
        assert_eq!(bad("\u{ff}"), e(0));
        assert_eq!(bad(&"x".repeat(MAX_LINE + 1)), e(0));
        assert!(Request::decode(b"{\"op\":\"trash\",\"id\":1,\"path\":\"/a\"}\n").is_ok());
    }

    #[test]
    fn no_text_names_a_path_and_every_text_reads_as_a_sentence() {
        let codes = [
            ErrorCode::InvalidRequest,
            ErrorCode::Version,
            ErrorCode::NotBrowser,
            ErrorCode::PermissionDenied,
            ErrorCode::Unavailable,
            ErrorCode::NotFound,
            ErrorCode::Busy,
            ErrorCode::Failed,
        ];
        for v in [Verb::Share, Verb::OpenWith, Verb::Trash] {
            for c in codes {
                let t = text(v, c);
                assert!(!t.contains('/') && !t.contains('!'), "{t}");
                assert!(t.ends_with('.') && t.starts_with(char::is_uppercase), "{t}");
            }
        }
    }

    #[test]
    fn outcome_of_done_is_cancelled_unless_known() {
        assert_eq!(Outcome::of_done(Some("saved")), Outcome::Saved);
        assert_eq!(Outcome::of_done(Some("other")), Outcome::Cancelled);
        assert_eq!(Outcome::of_done(None), Outcome::Cancelled);
    }
}
