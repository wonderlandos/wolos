// SPDX-License-Identifier: BSD-2-Clause
//! The revocation list (ADR 0046): `{"format":1,"serial":N,"issued":T,"expires":T+7d,"entries":[
//! {"serial":"<16 hex>","at":T,"reason":"key-compromise|abuse|superseded|ceased|withdrawn-version"}]}`,
//! signed with
//! label `wolos/revocation/1` by the Index intermediate (leaf revocations) or by a root (any
//! revocation, including the other root). Rules: `key-compromise` and `abuse` invalidate every
//! signature by that certificate whenever made; `superseded` and `ceased` keep files the index saw
//! before `at`; `withdrawn-version` names a release statement, not a certificate. A revoked
//! certificate fails closed.
use crate::cert::is_serial;
use crate::{MAX_REVOKED_BYTES, MAX_REVOKED_ENTRIES, STALE_WINDOW};
use serde::{Deserialize, Serialize};

/// Why a certificate was withdrawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub enum Reason {
    /// The private key is, or may be, in other hands: every signature is void.
    KeyCompromise,
    /// The certificate was used to harm: every signature is void.
    Abuse,
    /// Replaced by a newer certificate: earlier files stay valid.
    Superseded,
    /// The owner stopped publishing: earlier files stay valid.
    Ceased,
    /// Not a certificate: one release statement (`crate::statement`) is withdrawn, so that one
    /// version stops being offered while the author's certificate stays good. Certificate checks
    /// ignore these entries.
    WithdrawnVersion,
}

impl Reason {
    /// True for `key-compromise` and `abuse`: no file signed by the certificate counts, whenever
    /// it was signed.
    pub fn is_hard(self) -> bool {
        matches!(self, Reason::KeyCompromise | Reason::Abuse)
    }

    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::KeyCompromise => "key-compromise",
            Reason::Abuse => "abuse",
            Reason::Superseded => "superseded",
            Reason::Ceased => "ceased",
            Reason::WithdrawnVersion => "withdrawn-version",
        }
    }
}

/// One withdrawn certificate.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Serial of the withdrawn certificate.
    pub serial: String,
    /// When it was withdrawn, Unix seconds.
    pub at: u64,
    /// Why.
    pub reason: Reason,
}

/// The list as written (the signed bytes of `revoked.json`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListText {
    /// Always 1.
    pub format: u32,
    /// Grows with every publication; a lower serial never replaces a higher one.
    pub serial: u64,
    /// When it was issued, Unix seconds.
    pub issued: u64,
    /// Fresh until here (issued + 7 days).
    pub expires: u64,
    /// The withdrawn certificates.
    pub entries: Vec<Entry>,
}

/// A list whose signature was verified by the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevList {
    /// The list.
    pub list: ListText,
    /// Serial of the certificate that signed it.
    pub signer: String,
    /// True when a root signed it (only a root list may revoke an intermediate or a root).
    pub signer_is_root: bool,
}

/// One entry the machine has seen in a verified list, with who signed that list: kept forever in
/// `seen-revocations.json` (audit M1), so a later list without it, a rolled-back state file or an
/// expired Index never un-revokes it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeenEntry {
    /// The withdrawn serial.
    pub serial: String,
    /// When, as the list said (never later than that list's `issued`).
    pub at: u64,
    /// Why.
    pub reason: Reason,
    /// Serial of the certificate that signed the list it came in.
    pub signer: String,
    /// True when that signer was a root.
    pub by_root: bool,
}

/// The memory of seen revocations (`/var/lib/wolos/trust/seen-revocations.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Seen {
    /// Always 1.
    pub format: u32,
    /// Every entry seen, in the order first seen; never shortened.
    pub entries: Vec<SeenEntry>,
}

/// Largest memory file read: 40,000 entries of about 100 bytes.
pub const MAX_SEEN_BYTES: usize = 4 << 20;
/// Most entries kept in memory.
pub const MAX_SEEN_ENTRIES: usize = 40_000;

impl Seen {
    /// Parses the memory file within [`MAX_SEEN_BYTES`] and [`MAX_SEEN_ENTRIES`].
    pub fn parse(bytes: &[u8]) -> Result<Seen, &'static str> {
        if bytes.len() > MAX_SEEN_BYTES {
            return Err("revocation memory too large");
        }
        let s: Seen = serde_json::from_slice(bytes).map_err(|_| "revocation memory damaged")?;
        if s.format != crate::FORMAT
            || s.entries.len() > MAX_SEEN_ENTRIES
            || !s
                .entries
                .iter()
                .all(|e| is_serial(&e.serial) && is_serial(&e.signer))
        {
            return Err("revocation memory out of range");
        }
        Ok(s)
    }

    /// Adds every entry of `l` not held yet (same serial, reason and signer); the old entries
    /// stay, in order. Returns how many were added.
    pub fn add(&mut self, l: &RevList) -> usize {
        self.format = crate::FORMAT;
        let mut n = 0;
        for e in &l.list.entries {
            let have = self
                .entries
                .iter()
                .any(|s| s.serial == e.serial && s.reason == e.reason && s.signer == l.signer);
            if !have && self.entries.len() < MAX_SEEN_ENTRIES {
                self.entries.push(SeenEntry {
                    serial: e.serial.clone(),
                    at: e.at.min(l.list.issued),
                    reason: e.reason,
                    signer: l.signer.clone(),
                    by_root: l.signer_is_root,
                });
                n += 1;
            }
        }
        n
    }

    /// The memory as lists for the check: one per signer, so the rule "only a root withdraws an
    /// intermediate or a root" still applies.
    pub fn as_lists(&self) -> Vec<RevList> {
        let mut out: Vec<RevList> = Vec::new();
        for e in &self.entries {
            let entry = Entry {
                serial: e.serial.clone(),
                at: e.at,
                reason: e.reason,
            };
            match out.iter_mut().find(|l| l.signer == e.signer) {
                Some(l) => l.list.entries.push(entry),
                None => out.push(RevList {
                    list: ListText {
                        format: crate::FORMAT,
                        serial: 0,
                        issued: e.at,
                        expires: e.at.saturating_add(1),
                        entries: vec![entry],
                    },
                    signer: e.signer.clone(),
                    signer_is_root: e.by_root,
                }),
            }
        }
        out
    }

    /// JSON text of the memory.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = serde_json::to_vec_pretty(self).unwrap_or_default();
        v.push(b'\n');
        v
    }
}

/// Parses `revoked.json` (at most 1 MiB, 10,000 entries); the signature is checked elsewhere.
pub fn parse(bytes: &[u8]) -> Result<ListText, &'static str> {
    if bytes.len() > MAX_REVOKED_BYTES {
        return Err("revocation list too large");
    }
    let l: ListText = serde_json::from_slice(bytes).map_err(|_| "revocation list damaged")?;
    if l.format != crate::FORMAT {
        return Err("unknown revocation list format");
    }
    if l.entries.len() > MAX_REVOKED_ENTRIES || l.expires <= l.issued {
        return Err("revocation list out of range");
    }
    if !l.entries.iter().all(|e| is_serial(&e.serial)) {
        return Err("revocation serial malformed");
    }
    Ok(l)
}

/// How current the best list is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// Before `expires`.
    Fresh,
    /// Up to 30 days past `expires`: verified files show as verified with one more line.
    Stale,
    /// Older: signatures still verify and keep their value, with the line "not checked for
    /// withdrawn certificates since" (audit M3; never silently unsigned).
    TooOld,
    /// No list at all.
    Missing,
}

/// Freshness of a list that expires at `expires`, seen at `now`.
pub fn freshness(expires: u64, now: u64) -> Freshness {
    if now < expires {
        Freshness::Fresh
    } else if now < expires.saturating_add(STALE_WINDOW) {
        Freshness::Stale
    } else {
        Freshness::TooOld
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freshness_has_three_windows() {
        assert_eq!(freshness(1000, 999), Freshness::Fresh);
        assert_eq!(freshness(1000, 1000), Freshness::Stale);
        assert_eq!(freshness(1000, 1000 + STALE_WINDOW - 1), Freshness::Stale);
        assert_eq!(freshness(1000, 1000 + STALE_WINDOW), Freshness::TooOld);
    }

    #[test]
    fn hard_reasons_are_exactly_two() {
        let hard: Vec<_> = [
            Reason::KeyCompromise,
            Reason::Abuse,
            Reason::Superseded,
            Reason::Ceased,
            Reason::WithdrawnVersion,
        ]
        .into_iter()
        .filter(|r| r.is_hard())
        .collect();
        assert_eq!(hard, [Reason::KeyCompromise, Reason::Abuse]);
    }

    #[test]
    fn bad_lists_are_refused() {
        for t in [
            r#"{"format":2,"serial":1,"issued":1,"expires":2,"entries":[]}"#,
            r#"{"format":1,"serial":1,"issued":5,"expires":5,"entries":[]}"#,
            r#"{"format":1,"serial":1,"issued":1,"expires":2,"entries":[{"serial":"x","at":1,"reason":"abuse"}]}"#,
            r#"{"format":1,"serial":1,"issued":1,"expires":2,"entries":[{"serial":"0123456789abcdef","at":1,"reason":"oops"}]}"#,
            r#"{"format":1,"serial":1.5,"issued":1,"expires":2,"entries":[]}"#,
            r#"{"format":1,"serial":1,"issued":1,"expires":2,"entries":[],"extra":1}"#,
        ] {
            assert!(parse(t.as_bytes()).is_err(), "{t}");
        }
        let ok = r#"{"format":1,"serial":1,"issued":1,"expires":2,"entries":[{"serial":"0123456789abcdef","at":1,"reason":"ceased"}]}"#;
        assert!(parse(ok.as_bytes()).is_ok());
    }
}
