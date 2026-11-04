// SPDX-License-Identifier: BSD-2-Clause
//! The trust verdict and the report JSON of interface I7: `{verdict, findings, trust, cert?, origin?}`. The
//! system's sheet and the hub print the same sentences because both take them from here.
use crate::cert::{Level, Subject};
use serde::{Deserialize, Serialize};

/// What the system makes of a signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
#[serde(deny_unknown_fields)]
pub enum Trust {
    /// Made by WolOS (Packages CA chain).
    Wolos,
    /// A developer or organisation a WolOS Hub administrator approved (a `trusted` certificate).
    Trusted,
    /// Who published it is proven (GitHub sign-in, the CLA, the namespace): an `origin`
    /// certificate, or a `trusted` one whose approval was withdrawn.
    Origin,
    /// Signed only with WolOS's previous key (a raw Ed25519 `SHA256.sig`, no certificate and no
    /// post-quantum half; audit H1): shown as "Signed with the previous key", never as WolOS.
    Legacy,
    /// Nobody vouches: no signature or an expired certificate.
    Community,
    /// The certificate was withdrawn.
    Withdrawn,
    /// The signature is present and wrong.
    Invalid,
}

/// The hub's automatic check, as ADR 0022: a clean result means only that no rule broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub enum Verdict {
    /// No rule broke.
    Pass,
    /// Listed, with every finding in plain words.
    Flag,
    /// Not listed: malware hit, wrong id, damaged file.
    Block,
    /// Too large, or a check timed out.
    NotChecked,
}

/// One sentence about a file.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    /// Stable rule id (`trust/<code>` for trust findings).
    pub rule: String,
    /// What a person reads.
    pub sentence: String,
    /// What was seen (a serial, a name, a count); may be empty.
    pub evidence: String,
}

/// Who signed, as shown.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CertInfo {
    /// Serial of the signing certificate.
    pub serial: String,
    /// GitHub id, login and name (or the label of a WolOS signer).
    pub subject: Subject,
    /// How the maker is shown.
    pub level: Level,
    /// Origin or trusted, for people and organisations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<crate::cert::Class>,
    /// When the hub verified the subject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<u64>,
}

/// Interface I7.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    /// The automatic check.
    pub verdict: Verdict,
    /// What it found, plus one trust sentence when the file is not plainly trusted.
    pub findings: Vec<Finding>,
    /// What the signature is worth.
    pub trust: Trust,
    /// The signing certificate, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert: Option<CertInfo>,
    /// Where the release comes from, when its signature carries a verified origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<crate::origin::Origin>,
}

impl Report {
    /// JSON text of the report (key order as declared, no spaces).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}
