//! Daily update check results. `update-check` (systemd timer, root) refreshes the pinned pacman
//! databases, lists `[wolos]` package fixes and known vulnerabilities (`arch-audit`), fetches the
//! signed release manifest and leaves its findings in [`DIR`], readable by every account; the
//! shell notifies from them and `wol update status` prints them. Installing is separate and asked
//! for: `wol update apply` (a new root slot, `super::plan`).
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const DIR: &str = "/var/lib/wolos/updates";
/// The script the timer runs (hourly; it checks at most once every [`EVERY_SECS`]).
pub const SCRIPT: &str = "/usr/lib/wolos/update-check";
pub const EVERY_SECS: u64 = 20 * 3600;
/// The manifest of the release this slot runs.
pub const RELEASE_FILE: &str = "/usr/lib/wolos-release.json";
/// The system kernel, only from `[wolos]`; the default boot entry always boots it.
pub const SYSTEM_KERNEL: &str = "linux-wolos";

/// Installed release manifest; any account may run it.
pub const USERLAND_ARGV: [&str; 2] = ["cat", RELEASE_FILE];

/// One WolOS release (`release.json`, signed with the package keys: `release.json.sig`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// `2026.11`.
    pub version: String,
    /// Arch Linux Archive date every repo is pinned to (`2026/11/02`).
    pub snapshot: String,
    /// `[wolos]` repo revision.
    pub wolos_repo: String,
    /// linux-wolos package version.
    pub kernel: String,
    /// Installer image and its sha256.
    pub image: String,
    pub sha256: String,
}

impl Manifest {
    pub fn parse(text: &str) -> Option<Self> {
        let m: Self = serde_json::from_str(text).ok()?;
        (!m.version.is_empty()
            && super::is_snapshot_date(&m.snapshot)
            && m.sha256.len() == 64
            && m.sha256.bytes().all(|b| b.is_ascii_hexdigit()))
        .then_some(m)
    }

    /// Only a manifest signed by a trusted key counts.
    pub fn verify(text: &[u8], sig: &[u8], keys: &[[u8; 32]]) -> Result<Self, String> {
        match crate::package::verify_index(text, Some(sig), keys)? {
            crate::package::Trust::Signed(_) => {}
            crate::package::Trust::Unsigned => return Err("release manifest not signed".into()),
        }
        Self::parse(std::str::from_utf8(text).map_err(|e| e.to_string())?)
            .ok_or_else(|| "bad release manifest".into())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Updates {
    /// When the last check finished (seconds since 1970).
    pub checked: u64,
    /// `[wolos]` packages with a newer version in the pinned repos.
    pub packages: Vec<String>,
    /// Of those, the ones with a known vulnerability (`arch-audit -q`).
    pub security: Vec<String>,
    /// Newer WolOS release ready to install (`2026.11`).
    pub base: Option<String>,
    /// Its snapshot date (`wol update apply` pins the new slot to it).
    pub snapshot: Option<String>,
    /// Kernels the person installed with pacman besides linux-wolos (reported, never removed).
    pub kernels: Vec<String>,
}

impl Updates {
    /// From the check's files: `checked`, `pkg` (`pacman -Qu`), `audit` (`arch-audit -q`),
    /// `base` (the newer release manifest, empty when none). None: never checked.
    pub fn parse(checked: &str, pkg: &str, audit: &str, base: &str) -> Option<Self> {
        let checked = checked.trim().parse().ok().filter(|t| *t > 0)?;
        let packages = super::parse_pending(Some(pkg)).unwrap_or_default();
        let vulnerable = parse_audit(audit);
        let security = packages
            .iter()
            .filter(|p| vulnerable.contains(p))
            .cloned()
            .collect();
        let release = Manifest::parse(base);
        Some(Self {
            checked,
            packages,
            security,
            base: release.as_ref().map(|m| m.version.clone()),
            snapshot: release.map(|m| m.snapshot),
            kernels: Vec::new(),
        })
    }

    /// [`Self::read_with`] the trusted package keys.
    pub fn read(dir: &Path) -> Option<Self> {
        let keys = crate::package::read_keys(Path::new(shared::package::KEYS_DIR));
        Self::read_with(dir, &keys)
    }

    /// The check's files; the release manifest counts only with a trusted `base.sig`.
    pub fn read_with(dir: &Path, keys: &[[u8; 32]]) -> Option<Self> {
        Self::from_files(
            |n| std::fs::read_to_string(dir.join(n)).unwrap_or_default(),
            keys,
        )
    }

    /// From the check's files by name (`base.sig` = the signature in lowercase hex).
    pub fn from_files(f: impl Fn(&str) -> String, keys: &[[u8; 32]]) -> Option<Self> {
        let base = f("base");
        let sig = unhex(f("base.sig").trim()).unwrap_or_default();
        let base = Manifest::verify(base.as_bytes(), &sig, keys)
            .map(|_| base)
            .unwrap_or_default();
        let mut u = Self::parse(&f("checked"), &f("pkg"), &f("audit"), &base)?;
        u.kernels = user_kernels(&f("kernels"));
        Some(u)
    }

    /// Updates to install: each package, plus a new release as one.
    pub fn count(&self) -> usize {
        self.packages.len() + usize::from(self.base.is_some())
    }

    /// Drop a release already installed since the check (the running [`RELEASE_FILE`]).
    pub fn installed(mut self, running: Option<&str>) -> Self {
        let v = running.and_then(Manifest::parse).map(|m| m.version);
        if v.is_some() && v == self.base {
            self.base = None;
            self.snapshot = None;
        }
        self
    }

    /// Notify once per check: this check is newer than the one already shown and found something.
    pub fn notify_after(&self, shown: u64) -> bool {
        self.checked > shown && self.count() > 0
    }
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Package names from `arch-audit -q` (one name per line).
pub fn parse_audit(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

/// Kernel packages in `pacman -Qq` output other than linux-wolos (`linux`, `linux-zen`, ...):
/// allowed, reported, never removed. Headers, firmware, docs and tools are not kernels.
pub fn user_kernels(qq: &str) -> Vec<String> {
    qq.split_whitespace()
        .filter(|p| {
            (*p == "linux" || p.starts_with("linux-"))
                && *p != SYSTEM_KERNEL
                && !p.starts_with("linux-firmware")
                && !p.starts_with("linux-api-headers")
                && !["-headers", "-docs", "-tools"]
                    .iter()
                    .any(|s| p.ends_with(s))
        })
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PKG: &str = "wol-shell 0.12.1-1 -> 0.12.2-1\nwolos-keyring 1-1 -> 2-1\n";
    const RELEASE: &str = r#"{"version":"2026.11","snapshot":"2026/11/02","wolos_repo":"r42","kernel":"6.17.3.wolos1-1","image":"https://example.invalid/wolos-2026.11.iso","sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}"#;

    #[test]
    fn findings_from_the_check_files() {
        assert_eq!(Updates::parse("", PKG, "", ""), None, "never checked");
        assert_eq!(Updates::parse("x", PKG, "", ""), None);
        let u = Updates::parse("1761600000\n", PKG, "wol-shell\nlibxml2\n", "").unwrap();
        assert_eq!(u.checked, 1761600000);
        assert_eq!(u.packages, ["wol-shell", "wolos-keyring"]);
        assert_eq!(u.security, ["wol-shell"], "vulnerable and updatable");
        assert_eq!(u.base, None);
        assert_eq!(u.count(), 2);
        let none = Updates::parse("5", "", "", "").unwrap();
        assert_eq!(none.count(), 0);
    }

    #[test]
    fn a_new_release_counts_as_one_update() {
        let u = Updates::parse("9", "", "", RELEASE).unwrap();
        assert_eq!(u.base.as_deref(), Some("2026.11"));
        assert_eq!(u.snapshot.as_deref(), Some("2026/11/02"));
        assert_eq!(u.count(), 1);
        let m = Manifest::parse(RELEASE).unwrap();
        assert_eq!(
            m.kernel, "6.17.3.wolos1-1",
            "records the linux-wolos version"
        );
        for bad in [
            "",
            "{}",
            &RELEASE.replace("2026/11/02", "latest"),
            &RELEASE.replace("ba78", "zz78"),
        ] {
            assert_eq!(Manifest::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn manifest_needs_a_trusted_signature() {
        let seed = [7u8; 32];
        let key = crate::package::public_of(&seed);
        let sig = crate::package::sign(&seed, RELEASE.as_bytes());
        assert!(Manifest::verify(RELEASE.as_bytes(), &sig, &[key]).is_ok());
        assert!(Manifest::verify(RELEASE.as_bytes(), &sig, &[[9; 32]]).is_err());
        let other = RELEASE.replace("r42", "r43");
        assert!(Manifest::verify(other.as_bytes(), &sig, &[key]).is_err());
    }

    #[test]
    fn installed_release_is_not_offered_again() {
        let u = Updates::parse("9", "", "", RELEASE).unwrap();
        assert_eq!(u.clone().installed(Some(RELEASE)).base, None);
        let older = RELEASE.replace("2026.11", "2026.10");
        assert_eq!(
            u.clone().installed(Some(&older)).base.as_deref(),
            Some("2026.11")
        );
        assert_eq!(u.installed(None).count(), 1, "unknown: keep");
        assert_eq!(USERLAND_ARGV, ["cat", "/usr/lib/wolos-release.json"]);
    }

    #[test]
    fn user_kernels_are_reported() {
        let qq = "linux-wolos\nlinux-wolos-headers\nlinux-firmware\nlinux-firmware-intel\n\
                  linux-zen\nlinux\nlinux-api-headers\nmesa\n";
        assert_eq!(user_kernels(qq), ["linux-zen", "linux"]);
        assert!(user_kernels("linux-wolos\n").is_empty());
    }

    #[test]
    fn notifies_once_per_check_with_findings() {
        let u = Updates::parse("100", PKG, "", "").unwrap();
        assert!(u.notify_after(0));
        assert!(u.notify_after(99));
        assert!(!u.notify_after(100), "already shown");
        assert!(
            !Updates::parse("200", "", "", "").unwrap().notify_after(0),
            "nothing found"
        );
    }

    #[test]
    fn reads_the_folder() {
        let d = std::env::temp_dir().join(format!("wol-updates-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(Updates::read(&d), None);
        std::fs::write(d.join("checked"), "42\n").unwrap();
        std::fs::write(d.join("pkg"), PKG).unwrap();
        std::fs::write(d.join("kernels"), "linux-wolos\nlinux-lts\n").unwrap();
        let u = Updates::read(&d).unwrap();
        assert_eq!(u.count(), 2);
        assert_eq!(u.kernels, ["linux-lts"]);
        // The manifest counts only with a trusted signature.
        let seed = [7u8; 32];
        let key = crate::package::public_of(&seed);
        std::fs::write(d.join("base"), RELEASE).unwrap();
        assert_eq!(Updates::read_with(&d, &[key]).unwrap().base, None, "no sig");
        let sig = crate::package::hex(crate::package::sign(&seed, RELEASE.as_bytes()));
        std::fs::write(d.join("base.sig"), format!("{sig}\n")).unwrap();
        assert_eq!(
            Updates::read_with(&d, &[key]).unwrap().base.as_deref(),
            Some("2026.11")
        );
        assert_eq!(Updates::read_with(&d, &[[9; 32]]).unwrap().base, None);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
