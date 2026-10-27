//! Daily update check results. `update-check` (image/files/usr/local/libexec/wolos, cron, root)
//! refreshes the package catalogue and the vulnerability list, fetches FreeBSD fixes, and leaves
//! its findings in [`DIR`], readable by every account; the shell notifies from them and
//! `wol update status` prints them. Installing is separate and asked for: wol-admind's
//! `pkg-upgrade`, after a restore point ([`super::be::restore_point`]).
use serde::Serialize;
use std::path::Path;

pub const DIR: &str = "/var/db/wolos/updates";
/// The script cron runs (hourly; it checks at most once every [`EVERY_SECS`]).
pub const SCRIPT: &str = "/usr/local/libexec/wolos/update-check";
pub const EVERY_SECS: u64 = 20 * 3600;
/// freebsd-update configuration both the check and the install use: the stock one, limited to
/// `world` when the running kernel is not GENERIC (WolKernel is never replaced by GENERIC).
pub const BASE_CONF: &str = "/var/db/wolos/updates/freebsd-update.conf";

/// Exit 0 (Some output) when fetched FreeBSD fixes wait to be installed, 2 otherwise.
pub fn base_ready_argv() -> Vec<String> {
    ["freebsd-update", "-f", BASE_CONF, "updatesready"]
        .map(String::from)
        .to_vec()
}

/// Installed userland patch level (`15.1-RELEASE-p3`); any account may run it.
pub const USERLAND_ARGV: [&str; 2] = ["freebsd-version", "-u"];

pub fn base_install_argv() -> Vec<String> {
    ["freebsd-update", "-f", BASE_CONF, "install"]
        .map(String::from)
        .to_vec()
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Updates {
    /// When the last check finished (seconds since 1970).
    pub checked: u64,
    /// Packages with a newer version in the catalogue.
    pub packages: Vec<String>,
    /// Of those, the ones whose installed version has a known vulnerability (`pkg audit`).
    pub security: Vec<String>,
    /// FreeBSD patch level ready to install (`15.1-RELEASE-p3`): security advisories and errata.
    pub base: Option<String>,
}

impl Updates {
    /// From the check's files: `checked`, `pkg` (`pkg version -vRL=`), `audit` (`pkg audit -q`),
    /// `base` (`freebsd-update fetch` output, empty when nothing is ready). None: never checked.
    pub fn parse(checked: &str, pkg: &str, audit: &str, base: &str) -> Option<Self> {
        let checked = checked.trim().parse().ok().filter(|t| *t > 0)?;
        let packages = super::parse_pending(Some(pkg)).unwrap_or_default();
        let vulnerable = parse_audit(audit);
        let security = packages
            .iter()
            .filter(|p| vulnerable.contains(p))
            .cloned()
            .collect();
        Some(Self {
            checked,
            packages,
            security,
            base: parse_base(base),
        })
    }

    pub fn read(dir: &Path) -> Option<Self> {
        let f = |n: &str| std::fs::read_to_string(dir.join(n)).unwrap_or_default();
        Self::parse(&f("checked"), &f("pkg"), &f("audit"), &f("base"))
    }

    /// Updates to install: each package, plus FreeBSD's fixes as one.
    pub fn count(&self) -> usize {
        self.packages.len() + usize::from(self.base.is_some())
    }

    /// Drop FreeBSD fixes already installed since the check (`freebsd-version -u` output).
    pub fn installed(mut self, userland: Option<&str>) -> Self {
        if userland.map(str::trim) == self.base.as_deref() {
            self.base = None;
        }
        self
    }

    /// Notify once per check: this check is newer than the one already shown and found something.
    pub fn notify_after(&self, shown: u64) -> bool {
        self.checked > shown && self.count() > 0
    }
}

/// Package names from `pkg audit -q` (one `name-version` per line).
pub fn parse_audit(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|p| p.rsplit_once('-').map_or(p, |(n, _)| n).to_owned())
        .collect()
}

/// Target patch level from `freebsd-update fetch` output ("... as part of updating to
/// 15.1-RELEASE-p3:"); None when it found nothing to install.
pub fn parse_base(text: &str) -> Option<String> {
    let (_, rest) = text.split_once("updating to")?;
    let v = rest.split_whitespace().next()?.trim_end_matches([':', '.']);
    (!v.is_empty()).then(|| v.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PKG: &str = "curl-8.1 <   needs updating (remote has 8.2)\n\
                       firefox-130.0,2 <   needs updating (remote has 131.0,2)\n";

    #[test]
    fn findings_from_the_check_files() {
        assert_eq!(Updates::parse("", PKG, "", ""), None, "never checked");
        assert_eq!(Updates::parse("x", PKG, "", ""), None);
        let u = Updates::parse("1761600000\n", PKG, "curl-8.1\nlibxml2-2.11\n", "").unwrap();
        assert_eq!(u.checked, 1761600000);
        assert_eq!(u.packages, ["curl", "firefox"]);
        assert_eq!(u.security, ["curl"], "vulnerable and updatable");
        assert_eq!(u.base, None);
        assert_eq!(u.count(), 2);
        let none = Updates::parse("5", "", "", "").unwrap();
        assert_eq!(none.count(), 0);
    }

    #[test]
    fn freebsd_fixes_count_as_one_update() {
        let fetch = "Fetching 5 patches..... done.\nApplying patches... done.\n\
                     The following files will be updated as part of updating to\n\
                     15.1-RELEASE-p3:\n/bin/freebsd-version\n/lib/libc.so.7\n";
        assert_eq!(parse_base(fetch).as_deref(), Some("15.1-RELEASE-p3"));
        let one_line =
            "The following files will be updated as part of updating to 15.1-RELEASE-p2:\n";
        assert_eq!(parse_base(one_line).as_deref(), Some("15.1-RELEASE-p2"));
        assert_eq!(parse_base(""), None);
        let u = Updates::parse("9", "", "", fetch).unwrap();
        assert_eq!((u.count(), u.packages.len()), (1, 0));
    }

    #[test]
    fn installed_freebsd_fixes_are_not_offered_again() {
        let fetch = "updating to 15.1-RELEASE-p3:\n";
        let u = Updates::parse("9", "", "", fetch).unwrap();
        assert_eq!(u.clone().installed(Some("15.1-RELEASE-p3\n")).base, None);
        let older = u.clone().installed(Some("15.1-RELEASE-p2\n"));
        assert_eq!(older.base.as_deref(), Some("15.1-RELEASE-p3"));
        assert_eq!(u.installed(None).count(), 1, "unknown: keep");
        assert_eq!(USERLAND_ARGV, ["freebsd-version", "-u"]);
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
        assert_eq!(Updates::read(&d).map(|u| u.count()), Some(2));
        std::fs::remove_dir_all(&d).unwrap();
        assert!(base_install_argv().ends_with(&["install".to_string()]));
        assert_eq!(base_ready_argv()[2], BASE_CONF);
    }
}
