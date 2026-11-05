// SPDX-License-Identifier: BSD-2-Clause
//! Items and their access lists (ADR 0054 sections 2 and 4). An item is what the Secret Service
//! calls an item: a label, string attributes and one secret, plus WolOS's class, the
//! this-device-only flag and the list of applications that may read the secret without a prompt.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Most attributes per item.
pub const MAX_ATTRS: usize = 32;
/// Longest attribute name or value, label, collection name.
pub const MAX_TEXT: usize = 1024;
/// Largest secret.
pub const MAX_SECRET: usize = 64 * 1024;
/// Most applications on one access list.
pub const MAX_ACL: usize = 32;

/// When an item's secret may be read (Apple's kSecAttrAccessible classes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Class {
    /// Only while the keyring is unlocked (the screen unlocked): the default.
    #[default]
    WhenUnlocked,
    /// From the first unlock of the session until sign-out, also while the screen is locked
    /// (a VPN key the network needs in the background).
    AfterFirstUnlock,
}

/// Who an application is, as the kernel and the system say (never as the app says).
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppId {
    /// Canonical executable path.
    pub exe: String,
    /// SHA-256 of the executable, hex, when it could be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The systemd unit or scope's app id (`app-org.wolos.Files@…` gives `org.wolos.Files`,
    /// Flatpak's scope gives its app id), when the process runs in one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// The id of the certificate that signed the package the executable came from (ADR 0046).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer: Option<String>,
}

impl AppId {
    /// Whether a running app `caller` is this grant: the same signer and app id when the grant has
    /// a signer (an update of a signed package keeps its grant), else the same executable path and
    /// the same digest when the grant recorded one.
    pub fn grants(&self, caller: &AppId) -> bool {
        if let Some(s) = &self.signer {
            return caller.signer.as_ref() == Some(s)
                && caller.app == self.app
                && caller.exe == self.exe;
        }
        if caller.exe != self.exe {
            return false;
        }
        match (&self.sha256, &caller.sha256) {
            (Some(a), Some(b)) => a == b,
            (Some(_), None) => false,
            (None, _) => true,
        }
    }

    /// The name Settings shows: the app id, else the program's file name.
    pub fn display(&self) -> &str {
        self.app
            .as_deref()
            .unwrap_or_else(|| self.exe.rsplit('/').next().unwrap_or(&self.exe))
    }
}

/// One item's metadata (everything but the secret), as stored in an encrypted record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    /// 16 random bytes, hex.
    pub id: String,
    pub collection: String,
    pub label: String,
    pub attrs: BTreeMap<String, String>,
    #[serde(default = "plain")]
    pub content_type: String,
    pub created: u64,
    pub modified: u64,
    #[serde(default)]
    pub class: Class,
    #[serde(default)]
    pub device_only: bool,
    #[serde(default)]
    pub acl: Vec<AppId>,
}

fn plain() -> String {
    "text/plain".into()
}

impl Meta {
    /// Checks every bound of the item.
    pub fn check(&self) -> crate::Result<()> {
        let short = |s: &str| s.len() <= MAX_TEXT;
        if self.id.len() != 32 || !self.id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(crate::Error::Malformed("item id"));
        }
        if !short(&self.collection) || !short(&self.label) || !short(&self.content_type) {
            return Err(crate::Error::Limit("text length"));
        }
        if self.attrs.len() > MAX_ATTRS || self.attrs.iter().any(|(k, v)| !short(k) || !short(v)) {
            return Err(crate::Error::Limit("attributes"));
        }
        if self.acl.len() > MAX_ACL {
            return Err(crate::Error::Limit("access list"));
        }
        Ok(())
    }

    /// Whether every attribute of `query` is on the item with the same value (the Secret
    /// Service's SearchItems).
    pub fn matches(&self, query: &BTreeMap<String, String>) -> bool {
        query.iter().all(|(k, v)| self.attrs.get(k) == Some(v))
    }

    /// Whether `caller` may read the secret without a prompt.
    pub fn allows(&self, caller: &AppId) -> bool {
        self.acl.iter().any(|g| g.grants(caller))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(exe: &str, sha: Option<&str>) -> AppId {
        AppId {
            exe: exe.into(),
            sha256: sha.map(Into::into),
            app: None,
            signer: None,
        }
    }

    #[test]
    fn grants_by_path_and_digest() {
        let g = app("/usr/bin/helium", Some("aa"));
        assert!(g.grants(&app("/usr/bin/helium", Some("aa"))));
        assert!(
            !g.grants(&app("/usr/bin/helium", Some("bb"))),
            "a changed binary asks again"
        );
        assert!(!g.grants(&app("/tmp/helium", Some("aa"))));
        assert!(!g.grants(&app("/usr/bin/helium", None)));
    }

    #[test]
    fn a_signed_grant_survives_an_update_by_the_same_signer() {
        let mut g = app("/usr/bin/wol-mail", Some("aa"));
        g.signer = Some("s1".into());
        g.app = Some("org.wolos.Mail".into());
        let mut c = app("/usr/bin/wol-mail", Some("bb"));
        c.signer = Some("s1".into());
        c.app = Some("org.wolos.Mail".into());
        assert!(g.grants(&c));
        c.signer = Some("s2".into());
        assert!(!g.grants(&c));
    }
}
