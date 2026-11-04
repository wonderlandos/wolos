// SPDX-License-Identifier: BSD-2-Clause
//! The trust verdict and the report JSON of interface I7: `{verdict, findings, trust, cert?}`. The
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
    /// A certified author or organisation.
    Verified,
    /// Nobody vouches: no signature, an expired certificate or a list too old to rely on.
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
}

impl Report {
    /// JSON text of the report (key order as declared, no spaces).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}
