// SPDX-License-Identifier: BSD-2-Clause
//! wopkg -> wold system package calls (ADR 0021): `call <json>` on the wold socket, one
//! `portal::Answer` line back. Each call names the consent grant it needs; wold checks it against
//! the caller's executable from the kernel. Every field is bounded and checked on both sides: a
//! package name never starts with `-`, so it can never be read as a pacman option.
use serde::{Deserialize, Serialize};

pub const MAX_NAMES: usize = 64;
pub const MAX_NAME: usize = 128;
pub const MAX_MANIFEST: usize = 8 * 1024;
pub const CHANNELS: [&str; 3] = ["stable", "beta", "nightly"];
/// The channels a person can follow in this release: stable only (lead, 0.13: no testing
/// channel). Release feeds may still name the others; nothing on this system switches to them.
pub const OFFERED: [&str; 1] = ["stable"];

/// The channel this computer follows, from `/etc/wolos/channel` (None: no file): the file's word
/// when it is an [`OFFERED`] one, else stable (a file left from before reads as what is used).
pub fn channel_of(text: Option<&str>) -> &'static str {
    let w = text.map(str::trim).unwrap_or_default();
    OFFERED
        .iter()
        .find(|c| **c == w)
        .copied()
        .unwrap_or(OFFERED[0])
}
/// Grants these calls need (`wol_core::consent::CAPABILITIES`).
pub const GRANTS: [&str; 4] = [
    "packages-install",
    "packages-remove",
    "packages-update",
    "packages-unreviewed",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verb {
    Install,
    Remove,
    /// With names: those packages; without: every pending package of the running release.
    Update,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "call", deny_unknown_fields)]
pub enum PackageCall {
    /// System packages from the configured repos (signed; pacman checks them).
    #[serde(rename = "package.change")]
    Change { verb: Verb, names: Vec<String> },
    /// Update everything except the packages the person pinned.
    #[serde(rename = "package.update-holding")]
    UpdateHolding { hold: Vec<String> },
    /// A package built on this machine (community package, recipe): unreviewed, its own grant.
    #[serde(rename = "package.install-file")]
    InstallFile { path: String },
    /// Move the system to the release in `manifest` (signed `release.json`, ADR 0022).
    #[serde(rename = "system.update")]
    SystemUpdate { manifest: String, signature: String },
    #[serde(rename = "system.channel")]
    SystemChannel { channel: String },
    /// Boot the restore slot next (`wol update rollback`).
    #[serde(rename = "system.rollback")]
    SystemRollback {},
}

/// pacman package name: `[a-z0-9@._+-]`, not starting with `-` or `.`.
pub fn name_ok(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= MAX_NAME
        && !n.starts_with(['-', '.'])
        && n.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"@._+-".contains(&b))
}

/// Built package file: absolute, canonical, no control characters, `.pkg.tar.zst`.
pub fn file_ok(p: &str) -> bool {
    p.len() <= 1024
        && p.ends_with(".pkg.tar.zst")
        && p.strip_prefix('/')
            .is_some_and(|r| r.split('/').all(|s| !s.is_empty() && s != "." && s != ".."))
        && !p.chars().any(char::is_control)
}

impl PackageCall {
    pub fn grant(&self) -> &'static str {
        match self {
            Self::Change {
                verb: Verb::Install,
                ..
            } => "packages-install",
            Self::Change {
                verb: Verb::Remove, ..
            } => "packages-remove",
            Self::Change {
                verb: Verb::Update, ..
            } => "packages-update",
            Self::UpdateHolding { .. } => "packages-update",
            Self::InstallFile { .. } => "packages-unreviewed",
            Self::SystemUpdate { .. } | Self::SystemChannel { .. } | Self::SystemRollback {} => {
                "packages-update"
            }
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Change { verb, names } => {
                if names.is_empty() && *verb != Verb::Update {
                    return Err("Name at least one package.".into());
                }
                if names.len() > MAX_NAMES {
                    return Err(format!("At most {MAX_NAMES} packages at a time."));
                }
                if let Some(n) = names.iter().find(|n| !name_ok(n)) {
                    return Err(format!(
                        "“{}” is not a package name.",
                        n.chars().take(MAX_NAME).collect::<String>()
                    ));
                }
                Ok(())
            }
            Self::UpdateHolding { hold } => {
                if hold.len() > MAX_NAMES || !hold.iter().all(|n| name_ok(n)) {
                    return Err("That is not a list of package names.".into());
                }
                Ok(())
            }
            Self::InstallFile { path } if !file_ok(path) => {
                Err("That is not a built package file.".into())
            }
            Self::SystemUpdate {
                manifest,
                signature,
            } => {
                if manifest.len() > MAX_MANIFEST {
                    return Err("The release description is too long.".into());
                }
                if signature.len() != 128 || !signature.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("The release signature is not valid.".into());
                }
                Ok(())
            }
            // Only the channel this release offers (QA-STORE-68): no sheet for a switch that
            // would be refused after the password.
            Self::SystemChannel { channel } if !OFFERED.contains(&channel.as_str()) => {
                Err("WolOS offers the stable channel only.".into())
            }
            _ => Ok(()),
        }
    }

    pub fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Is this call line one of ours (cheap check before decoding)?
    pub fn is_package_call(json: &str) -> bool {
        let t = json.trim_start();
        ["\"package.", "\"system."]
            .iter()
            .any(|p| t.contains(&format!("\"call\":{p}")))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > crate::portal::MAX_LINE {
            return Err("The request is too long.".into());
        }
        let text =
            std::str::from_utf8(bytes).map_err(|_| "The request is not UTF-8.".to_owned())?;
        let c: Self = serde_json::from_str(text.trim())
            .map_err(|_| "The request is not a known call.".to_owned())?;
        c.validate()?;
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// QA-STORE-9/68: one channel is offered; whatever the file says, stable is what is read.
    #[test]
    fn the_channel_read_is_always_offered() {
        assert_eq!(OFFERED, ["stable"]);
        for t in [
            None,
            Some(""),
            Some("stable\n"),
            Some("beta\n"),
            Some("nightly"),
            Some("x"),
        ] {
            assert_eq!(channel_of(t), "stable", "{t:?}");
        }
    }

    #[test]
    fn decode_table() {
        let cases: [(&str, Result<&str, ()>); 13] = [
            (
                r#"{"call":"package.change","verb":"install","names":["firefox","lib32-mesa"]}"#,
                Ok("packages-install"),
            ),
            (
                r#"{"call":"package.change","verb":"remove","names":["vim"]}"#,
                Ok("packages-remove"),
            ),
            (
                r#"{"call":"package.change","verb":"update","names":[]}"#,
                Ok("packages-update"),
            ),
            (
                r#"{"call":"package.change","verb":"install","names":[]}"#,
                Err(()),
            ),
            (
                r#"{"call":"package.change","verb":"install","names":["--config=/tmp/x"]}"#,
                Err(()),
            ),
            (
                r#"{"call":"package.change","verb":"install","names":["Firefox"]}"#,
                Err(()),
            ),
            (
                r#"{"call":"package.change","verb":"install","names":["a"],"extra":1}"#,
                Err(()),
            ),
            (
                r#"{"call":"package.install-file","path":"/home/a/.cache/wopkg/aur/x-1-1-x86_64.pkg.tar.zst"}"#,
                Ok("packages-unreviewed"),
            ),
            (
                r#"{"call":"package.install-file","path":"/home/a/../etc/x.pkg.tar.zst"}"#,
                Err(()),
            ),
            (
                r#"{"call":"system.channel","channel":"stable"}"#,
                Ok("packages-update"),
            ),
            // 0.13 offers one channel (QA-STORE-68): a testing one is refused before any sheet.
            (r#"{"call":"system.channel","channel":"beta"}"#, Err(())),
            (r#"{"call":"system.channel","channel":"canary"}"#, Err(())),
            (r#"{"call":"system.rollback"}"#, Ok("packages-update")),
        ];
        for (json, want) in cases {
            let got = PackageCall::decode(json.as_bytes())
                .map(|c| c.grant())
                .map_err(|_| ());
            assert_eq!(got, want, "{json}");
            assert!(PackageCall::is_package_call(json));
        }
        let sig = "a".repeat(128);
        let c = PackageCall::SystemUpdate {
            manifest: "{}".into(),
            signature: sig,
        };
        assert_eq!(PackageCall::decode(c.encode().as_bytes()).unwrap(), c);
        assert!(!PackageCall::is_package_call(
            r#"{"call":"notification.post","title":"x"}"#
        ));
    }

    #[test]
    fn names_never_look_like_options() {
        for n in [
            "-Syu",
            "--overwrite",
            ".hidden",
            "",
            "a b",
            "a/b",
            &"x".repeat(129),
        ] {
            assert!(!name_ok(n), "{n}");
        }
        for n in [
            "linux-wolos",
            "gtk4",
            "python-pyqt5",
            "lib32-glibc",
            "g++",
            "foo@bar",
            "x86_energy_perf_policy",
        ] {
            assert!(name_ok(n), "{n}");
        }
    }
}
