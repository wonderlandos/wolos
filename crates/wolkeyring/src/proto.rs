// SPDX-License-Identifier: BSD-2-Clause
//! The fast path (ADR 0054 section 5): one JSON line per request on
//! `$XDG_RUNTIME_DIR/wolos/keyring.sock`, one JSON line back. Our own apps use it instead of the
//! Secret Service; the identity of the caller comes from the kernel (SO_PEERCRED, a pinned
//! `/proc/<pid>`), never from the line.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::item::Class;

/// Longest request or reply line.
pub const MAX_LINE: usize = 256 * 1024;
/// Most items a search returns.
pub const MAX_RESULTS: usize = 1024;
/// Read and write timeout of the socket, in milliseconds.
pub const TIMEOUT_MS: u64 = 5000;

/// A request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Request {
    /// Whether the keyring is unlocked, and its item count.
    Status,
    /// Unlock with the login password (wol-login, the lock screen).
    Unlock { password: String },
    /// Lock now.
    Lock,
    /// Items whose attributes include these.
    Search { attrs: BTreeMap<String, String> },
    /// One item's secret (base64url).
    Get { id: String },
    /// Store a secret: replaces the item with the same collection and attributes.
    Set {
        label: String,
        attrs: BTreeMap<String, String>,
        /// base64url.
        secret: String,
        #[serde(default)]
        class: Class,
    },
    /// Delete one item.
    Delete { id: String },
    /// The last `limit` audit lines (raw JSON lines).
    Audit { limit: usize },
}

/// One search hit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hit {
    pub id: String,
    pub label: String,
    pub attrs: BTreeMap<String, String>,
    pub modified: u64,
}

/// A reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub ok: bool,
    /// On failure: `locked`, `denied`, `not-found`, `wrong-factors`, `policy`, `limit`,
    /// `malformed`, `damaged`, `unsupported`, `io`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unlocked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hits: Option<Vec<Hit>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<String>>,
}

impl Reply {
    /// A success with nothing else.
    pub fn ok() -> Reply {
        Reply {
            ok: true,
            error: None,
            message: None,
            unlocked: None,
            items: None,
            hits: None,
            id: None,
            secret: None,
            lines: None,
        }
    }
    /// The failure for `e`.
    pub fn err(e: &crate::Error) -> Reply {
        use crate::Error as E;
        let code = match e {
            E::Locked => "locked",
            E::Denied => "denied",
            E::NotFound => "not-found",
            E::WrongFactors => "wrong-factors",
            E::Policy(_) => "policy",
            E::Limit(_) => "limit",
            E::Malformed(_) => "malformed",
            E::Damaged(_) => "damaged",
            E::Unsupported(_) => "unsupported",
            E::Io(_) => "io",
        };
        Reply {
            ok: false,
            error: Some(code.into()),
            message: Some(e.to_string()),
            ..Reply::ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_and_unknown_fields_are_refused() {
        let r: Request = serde_json::from_str(r#"{"op":"search","attrs":{"vpn":"x"}}"#).unwrap();
        assert!(matches!(r, Request::Search { .. }));
        assert!(serde_json::from_str::<Request>(r#"{"op":"get","id":"a","as":"root"}"#).is_err());
        assert!(serde_json::from_str::<Request>(r#"{"op":"steal"}"#).is_err());
    }
}
