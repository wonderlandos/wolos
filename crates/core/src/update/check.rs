// SPDX-License-Identifier: BSD-2-Clause
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
/// The manifest of the release this slot runs. An image without it (v0.12.0) is known by
/// [`OS_RELEASE`]'s `VERSION_ID` instead ([`Updates::installed`] reads either).
pub const RELEASE_FILE: &str = "/usr/lib/wolos-release.json";
/// os-release of the running system: its `VERSION_ID` is the release version.
pub const OS_RELEASE: &str = "/etc/os-release";
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
    /// Former `[wolos-cachy]` snapshot revision. Always empty since ADR 0030 (the `[cachyos-*-v3]`
    /// sections are not republished); kept so older manifests still parse.
    #[serde(default)]
    pub cachy: String,
    /// linux-wolos package version.
    pub kernel: String,
    /// Installer image and its sha256.
    pub image: String,
    pub sha256: String,
    /// Signed, increasing publication number (audit 8 #2), like the wopkg feed's; 0 in manifests
    /// made before it existed (those count only when strictly newer than the running version).
    #[serde(default)]
    pub serial: u64,
    /// Signed end of validity, Unix seconds; 0 = none.
    #[serde(default)]
    pub expires: u64,
}

/// Freshness of a signed list or manifest (audit C6-2, audit 8 #2), the one rule the release
/// feed (`wopkg`) and the release manifest share: Err when `serial` is lower than the `last` one
/// accepted (an old signed copy replayed) or when `expires` is set and already past `now`. Equal
/// serials pass (the same list downloaded again).
pub fn check_fresh(
    what: &str,
    serial: u64,
    expires: u64,
    last: u64,
    now: u64,
) -> Result<(), String> {
    if serial < last {
        return Err(format!("The {what} is older than the one already seen."));
    }
    if expires != 0 && expires <= now {
        return Err(format!("The {what} has expired."));
    }
    Ok(())
}

/// `2026.11` / `2026.11.2` / `2026.11-rc1` as numbers and a pre-release tag (a tag sorts below
/// the release); None when it is anything else.
fn release_number(v: &str) -> Option<(Vec<u64>, Option<&str>)> {
    let (num, pre) = match v.split_once('-') {
        Some((n, p)) if !p.is_empty() => (n, Some(p)),
        Some(_) => return None,
        None => (v, None),
    };
    let parts = num
        .split('.')
        .map(|p| {
            (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
                .then(|| p.parse().ok())
                .flatten()
        })
        .collect::<Option<Vec<u64>>>()?;
    Some((parts, pre))
}

/// `a` is a strictly newer release than `b`. Fails closed: a version that is not understood is
/// never newer (nothing is offered on a guess).
pub fn is_newer(a: &str, b: &str) -> bool {
    let (Some((mut x, p)), Some((mut y, q))) = (release_number(a), release_number(b)) else {
        return false;
    };
    let n = x.len().max(y.len());
    x.resize(n, 0);
    y.resize(n, 0);
    x.cmp(&y).then_with(|| match (p, q) {
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (p, q) => p.cmp(&q),
    }) == std::cmp::Ordering::Greater
}

/// The running slot's [`RELEASE_FILE`]: the release it runs, written by the `wolos-base` package of
/// that release (so a system update replaces it). No image URL or hash: an image cannot carry its
/// own. A published [`Manifest`] reads as one too (its other fields are ignored).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Running {
    /// `0.12.1` (`VERSION_ID` of os-release).
    pub version: String,
    /// Arch Linux Archive date the release is pinned to (`2026/11/02`).
    pub snapshot: String,
    /// The rest of a published [`Manifest`] (and `size`, which release-manifest.sh adds): read,
    /// not used. Any other key is a mistake and the file does not count.
    #[serde(default)]
    pub wolos_repo: String,
    #[serde(default)]
    pub cachy: String,
    #[serde(default)]
    pub kernel: String,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
    /// The manifest's signed serial (audit 8 #2): a release with a lower one is a replay. 0 in a
    /// release file without it.
    #[serde(default)]
    pub serial: u64,
    /// The manifest's end of validity: read, not used (the running release is not judged by it).
    #[serde(default)]
    pub expires: u64,
}

impl Running {
    /// None unless it has a version and a snapshot date and no unknown key.
    pub fn parse(text: &str) -> Option<Self> {
        let r: Self = serde_json::from_str(text).ok()?;
        (!r.version.is_empty() && super::is_snapshot_date(&r.snapshot)).then_some(r)
    }
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
    /// Its signed serial (0: none), kept to compare with the running release's (`installed`).
    pub base_serial: u64,
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
            base_serial: release.as_ref().map_or(0, |m| m.serial),
            snapshot: release.map(|m| m.snapshot),
            kernels: Vec::new(),
        })
    }

    /// [`Self::read_with`] the trusted package keys, at the time `now` (seconds since 1970; core
    /// takes the clock from its caller, RR-067).
    pub fn read(dir: &Path, now: u64) -> Option<Self> {
        let keys = crate::package::read_keys(Path::new(shared::package::KEYS_DIR));
        Self::read_with(dir, &keys, now)
    }

    /// The check's files; the release manifest counts only with a trusted `base.sig`.
    pub fn read_with(dir: &Path, keys: &[[u8; 32]], now: u64) -> Option<Self> {
        Self::from_files_at(
            |n| std::fs::read_to_string(dir.join(n)).unwrap_or_default(),
            keys,
            now,
        )
    }

    /// From the check's files by name (`base.sig` = the signature in lowercase hex) at the time
    /// `now`: a signed manifest past its `expires` is dropped.
    pub fn from_files_at(f: impl Fn(&str) -> String, keys: &[[u8; 32]], now: u64) -> Option<Self> {
        let base = f("base");
        let sig = unhex(f("base.sig").trim()).unwrap_or_default();
        let base = Manifest::verify(base.as_bytes(), &sig, keys)
            .ok()
            .filter(|m| check_fresh("release manifest", m.serial, m.expires, 0, now).is_ok())
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

    /// Drop a release that is not newer than the running one: the same, an older one (a signed
    /// copy replayed, audit 8 #2) or one with a lower serial. A release is offered only when
    /// strictly newer by version, with or without a serial. `running` is the text of the running
    /// [`RELEASE_FILE`], or of [`OS_RELEASE`] where the image has no release file (e2e D3: once the
    /// check worked, every 0.12.0 system was offered "WolOS 0.12.0"); os-release has no serial.
    pub fn installed(mut self, running: Option<&str>) -> Self {
        let Some((version, serial)) = running.and_then(|t| {
            Running::parse(t)
                .map(|r| (r.version, r.serial))
                .or_else(|| crate::base::version(t).map(|v| (v, 0)))
        }) else {
            return self;
        };
        let Some(b) = self.base.as_deref() else {
            return self;
        };
        if release_number(&version).is_none() {
            return self; // the running version cannot be compared: as before, keep
        }
        let newer = is_newer(b, &version)
            && check_fresh("release manifest", self.base_serial, 0, serial, 0).is_ok();
        if !newer {
            self.base = None;
            self.snapshot = None;
            self.base_serial = 0;
        }
        self
    }

    /// Notify once per check: this check is newer than the one already shown and found something.
    pub fn notify_after(&self, shown: u64) -> bool {
        self.checked > shown && self.count() > 0
    }
}

/// Hex text to bytes; None on an odd length, a non-hex digit or a non-ASCII character (never
/// panics on runtime text: `get` instead of slicing).
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    // Hex digits only: `from_str_radix` alone would take a sign ("+f").
    if !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
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
        assert_eq!(m.cachy, "", "manifests have no cachy field (ADR 0030)");
        let c = RELEASE.replace(r#""kernel""#, r#""cachy":"20261102-ab12","kernel""#);
        assert_eq!(Manifest::parse(&c).unwrap().cachy, "20261102-ab12");
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
        assert_eq!(u.clone().installed(None).count(), 1, "unknown: keep");
        assert_eq!(USERLAND_ARGV, ["cat", "/usr/lib/wolos-release.json"]);
        // The release file wolos-base writes (no image hash), and the published manifest, both read.
        let file = r#"{"version":"2026.11","snapshot":"2026/11/02"}"#;
        assert_eq!(u.clone().installed(Some(file)).base, None);
        assert_eq!(
            Running::parse(RELEASE).map(|r| r.version).as_deref(),
            Some("2026.11")
        );
        assert_eq!(
            Running::parse(r#"{"version":"2026.11","snapshot":"x"}"#),
            None
        );
        assert_eq!(
            Running::parse(r#"{"version":"","snapshot":"2026/11/02"}"#),
            None
        );
        let published = r#"{"version":"0.12.0","snapshot":"2026/10/31","wolos_repo":"r","kernel":"k","image":"https://x/i.iso","sha256":"00","size":5}"#;
        assert_eq!(
            Running::parse(published).map(|r| r.version).as_deref(),
            Some("0.12.0")
        );
        assert_eq!(
            Running::parse(r#"{"version":"1","snapshot":"2026/10/31","verison":"2"}"#),
            None,
            "a typo is not read"
        );
        // No manifest in the image: os-release names the running version.
        let os = "NAME=WolOS\nID=wolos\nVERSION_ID=2026.11\n";
        assert_eq!(u.clone().installed(Some(os)).base, None);
        let os_older = os.replace("2026.11", "2026.10");
        assert_eq!(
            u.installed(Some(&os_older)).base.as_deref(),
            Some("2026.11")
        );
    }

    /// e2e D3 with audit 8 #2: a system known only by os-release is offered a strictly newer
    /// release, never the one it runs (or an older one).
    #[test]
    fn os_release_running_version_is_compared_strictly() {
        let os = "NAME=WolOS\nID=wolos\nVERSION_ID=0.12.0\n";
        let offered =
            |v: &str| Updates::parse("9", "", "", &RELEASE.replace("2026.11", v)).unwrap();
        assert_eq!(offered("0.12.0").installed(Some(os)).base, None, "the same");
        assert_eq!(offered("0.11.3").installed(Some(os)).base, None, "older");
        assert_eq!(
            offered("0.12.1").installed(Some(os)).base.as_deref(),
            Some("0.12.1")
        );
        // A running release file with a serial outranks os-release; a lower offered serial is a replay.
        let running = r#"{"version":"0.12.0","snapshot":"2026/10/31","serial":5}"#;
        assert_eq!(
            offered("0.12.1").installed(Some(running)).base,
            None,
            "serial 0 < 5"
        );
    }

    /// Audit 8 #2: an older signed manifest, replayed, is not offered to a newer system.
    #[test]
    fn an_older_manifest_is_not_offered_to_a_newer_system() {
        let u = Updates::parse("9", "", "", RELEASE).unwrap();
        let newer = RELEASE.replace("2026.11", "2027.01");
        assert_eq!(u.clone().installed(Some(&newer)).base, None, "older");
        assert_eq!(u.clone().installed(Some(&newer)).snapshot, None);
        assert_eq!(u.clone().installed(Some(&newer)).count(), 0);
        let patch = RELEASE.replace("2026.11", "2026.11.1");
        assert_eq!(
            u.clone().installed(Some(&patch)).base,
            None,
            "2026.11 < 2026.11.1"
        );
        let rc = RELEASE.replace("2026.11", "2026.11-rc1");
        assert_eq!(
            u.clone().installed(Some(&rc)).base.as_deref(),
            Some("2026.11")
        );
        assert_eq!(u.clone().installed(Some(RELEASE)).base, None, "the same");
        let junk = RELEASE.replace("2026.11", "latest");
        assert_eq!(
            u.clone().installed(Some(&junk)).base.as_deref(),
            Some("2026.11"),
            "unknown: keep"
        );
        // A release whose own version is not understood is never offered.
        let odd = Updates {
            base: Some("x.y".into()),
            ..u.clone()
        };
        assert_eq!(odd.installed(Some(RELEASE)).base, None);
        assert!(is_newer("2026.12", "2026.11") && is_newer("2026.11.1", "2026.11"));
        assert!(!is_newer("2026.11", "2026.11") && !is_newer("2026.11", "2026.11.0"));
        assert!(is_newer("2026.11", "2026.11-rc1") && !is_newer("2026.11-rc1", "2026.11"));
        assert!(!is_newer("", "1") && !is_newer("1", "") && !is_newer("1..2", "1"));
    }

    /// Audit 8 #2: the serial and the end of validity are signed and checked like the feed's.
    #[test]
    fn a_manifest_needs_a_fresh_serial_and_a_future_end() {
        let with = |serial: u64, expires: u64| {
            RELEASE.replace(
                r#""kernel""#,
                &format!(r#""serial":{serial},"expires":{expires},"kernel""#),
            )
        };
        let m = Manifest::parse(&with(7, 2000)).unwrap();
        assert_eq!((m.serial, m.expires), (7, 2000));
        assert_eq!(
            Manifest::parse(RELEASE).unwrap().serial,
            0,
            "older manifests parse"
        );
        // Read from files: signed, then judged at `now`.
        let seed = [7u8; 32];
        let key = crate::package::public_of(&seed);
        let read = |text: &str, now: u64| {
            let sig = crate::package::sign(&seed, text.as_bytes());
            let hex: String = sig.iter().map(|b| format!("{b:02x}")).collect();
            let text = text.to_string();
            Updates::from_files_at(
                |n| match n {
                    "checked" => "9".into(),
                    "base" => text.clone(),
                    "base.sig" => hex.clone(),
                    _ => String::new(),
                },
                &[key],
                now,
            )
            .unwrap()
        };
        assert_eq!(read(&with(7, 2000), 1999).base.as_deref(), Some("2026.11"));
        assert_eq!(read(&with(7, 2000), 2000).base, None, "expired");
        assert_eq!(read(&with(7, 2000), 5000).count(), 0);
        assert_eq!(
            read(&with(7, 0), 5000).base.as_deref(),
            Some("2026.11"),
            "no end"
        );
        // A serial lower than the running one is a replay, even with a newer-looking version.
        let u = read(&with(7, 0), 1);
        let running = |serial: u64| with(serial, 0).replace("2026.11", "2026.10");
        assert_eq!(
            u.clone().installed(Some(&running(7))).base.as_deref(),
            Some("2026.11")
        );
        assert_eq!(
            u.clone().installed(Some(&running(8))).base,
            None,
            "lower serial"
        );
        // Without a serial: only strictly newer by version (above).
        assert_eq!(
            read(RELEASE, 1)
                .installed(Some(&running(0)))
                .base
                .as_deref(),
            Some("2026.11")
        );
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
        assert_eq!(Updates::read(&d, 1), None);
        std::fs::write(d.join("checked"), "42\n").unwrap();
        std::fs::write(d.join("pkg"), PKG).unwrap();
        std::fs::write(d.join("kernels"), "linux-wolos\nlinux-lts\n").unwrap();
        let u = Updates::read(&d, 1).unwrap();
        assert_eq!(u.count(), 2);
        assert_eq!(u.kernels, ["linux-lts"]);
        // The manifest counts only with a trusted signature.
        let seed = [7u8; 32];
        let key = crate::package::public_of(&seed);
        std::fs::write(d.join("base"), RELEASE).unwrap();
        assert_eq!(
            Updates::read_with(&d, &[key], 1).unwrap().base,
            None,
            "no sig"
        );
        let sig = crate::package::hex(crate::package::sign(&seed, RELEASE.as_bytes()));
        std::fs::write(d.join("base.sig"), format!("{sig}\n")).unwrap();
        assert_eq!(
            Updates::read_with(&d, &[key], 1).unwrap().base.as_deref(),
            Some("2026.11")
        );
        assert_eq!(Updates::read_with(&d, &[[9; 32]], 1).unwrap().base, None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// CORE-7 / R2C-21: hex digits only, never a slice that panics on a multi-byte character.
    #[test]
    fn unhex_takes_only_hex_digits() {
        assert_eq!(unhex("00ff"), Some(vec![0, 255]));
        assert_eq!(unhex(""), Some(vec![]));
        for bad in ["+1", "-1", "0", "zz", "\u{e9}a", "a\u{e9}", " 1"] {
            assert_eq!(unhex(bad), None, "{bad:?}");
        }
    }
}
