//! Sealed system volume: after an install or an update the slot's subvolume is read-only and
//! every executable and library under /usr/bin and /usr/lib carries fs-verity. The sealed
//! files' digests go to a manifest inside the slot; the manifest's own digest (the root digest)
//! goes next to the slot's kernel on the ESP. At boot [`verify`] compares the three; a mismatch
//! makes [`fallback`] boot the other slot and tells the person.
//! Pure: argv lists, text formats and decisions. The `wol seal` verbs run them.
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Directories (inside the slot) whose executables and libraries are sealed.
pub const DIRS: [&str; 2] = ["usr/bin", "usr/lib"];
/// Manifest inside the slot (not itself sealed).
pub const MANIFEST: &str = "usr/lib/wolos/verity.manifest";
/// Root digest of the slot's manifest, on the ESP next to its kernel.
pub fn digest_file(slot: &str) -> String {
    format!("{}/{slot}/verity", crate::update::ESP)
}
/// Where the boot check leaves its message for the shell to show.
pub const NOTICE: &str = "/var/lib/wolos/verity-notice";

/// A file the seal covers: a regular file in [`DIRS`] that is executable or an ELF object.
pub fn wants_verity(rel: &str, mode: u32, head: &[u8]) -> bool {
    rel != MANIFEST
        && DIRS.iter().any(|d| rel.starts_with(&format!("{d}/")))
        && (mode & 0o111 != 0 || head.starts_with(b"\x7fELF"))
}

pub fn enable_argv(path: &str) -> Vec<String> {
    ["fsverity", "enable", path].map(String::from).to_vec()
}
pub fn measure_argv(path: &str) -> Vec<String> {
    ["fsverity", "measure", path].map(String::from).to_vec()
}
/// The slot's subvolume becomes read-only (`btrfs property set -ts <root> ro true`).
pub fn seal_argv(root: &str) -> Vec<String> {
    ["btrfs", "property", "set", "-ts", root, "ro", "true"]
        .map(String::from)
        .to_vec()
}

/// `fsverity measure` prints `sha256:<hex> <path>`; the hex digest.
pub fn parse_measure(out: &str) -> Option<String> {
    let d = out.split_whitespace().next()?.strip_prefix("sha256:")?;
    (d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit())).then(|| d.to_ascii_lowercase())
}

/// path -> fs-verity digest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest(pub BTreeMap<String, String>);

impl Manifest {
    /// One `<digest> <path>` line per file, sorted by path.
    pub fn to_text(&self) -> String {
        self.0.iter().map(|(p, d)| format!("{d} {p}\n")).collect()
    }
    pub fn parse(text: &str) -> Manifest {
        Manifest(
            text.lines()
                .filter_map(|l| l.split_once(' '))
                .map(|(d, p)| (p.to_owned(), d.to_owned()))
                .collect(),
        )
    }
    /// SHA-256 of the manifest text: the one value the ESP keeps.
    pub fn root_digest(&self) -> String {
        Sha256::digest(self.to_text().as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    /// The manifest is not the one sealed at install/update time.
    ManifestChanged,
    /// Files whose digest differs from the manifest, or that lost fs-verity or went missing.
    Files(Vec<String>),
}

/// `expected`: the ESP's root digest; `measured`: current digest per path (None = no verity/missing).
pub fn verify(
    manifest: &Manifest,
    expected: &str,
    measured: &dyn Fn(&str) -> Option<String>,
) -> Verdict {
    if manifest.root_digest() != expected.trim() {
        return Verdict::ManifestChanged;
    }
    let bad: Vec<String> = manifest
        .0
        .iter()
        .filter(|(p, d)| measured(p).as_deref() != Some(d.as_str()))
        .map(|(p, _)| p.clone())
        .collect();
    if bad.is_empty() {
        Verdict::Ok
    } else {
        Verdict::Files(bad)
    }
}

/// What the boot check does about a bad slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fallback {
    /// Make `slot` the default and restart.
    Boot { argv: Vec<Vec<String>> },
    /// No safe slot to go to: carry on and tell the person.
    Stay,
}

/// `other_ok`: the other slot exists and has not failed this check itself (no restart loop).
pub fn fallback(running: &str, other_ok: bool) -> Fallback {
    match crate::update::other_slot(running) {
        Some(o) if other_ok => Fallback::Boot {
            argv: vec![
                vec!["bootctl".into(), "set-default".into(), format!("{o}.conf")],
                vec!["systemctl".into(), "reboot".into()],
            ],
        },
        _ => Fallback::Stay,
    }
}

/// The message for the person (no jargon).
pub fn notice(running: &str, v: &Verdict, switching: bool) -> String {
    let what = match v {
        Verdict::Ok => return String::new(),
        Verdict::ManifestChanged => "its protected file list was changed".to_owned(),
        Verdict::Files(f) => format!("{} protected system file(s) were changed", f.len()),
    };
    let then = if switching {
        "WolOS is starting the previous version instead."
    } else {
        "There is no other version to start; please reinstall or restore."
    };
    format!("The system version {running} did not pass its safety check: {what}. {then}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> Manifest {
        Manifest::parse(&format!(
            "{} usr/bin/a\n{} usr/lib/b.so\n",
            "a".repeat(64),
            "b".repeat(64)
        ))
    }

    #[test]
    fn covers_executables_and_libs_under_usr_only() {
        assert!(wants_verity("usr/bin/ls", 0o755, b""));
        assert!(wants_verity("usr/lib/libc.so.6", 0o644, b"\x7fELF.."));
        assert!(!wants_verity("usr/lib/os-release", 0o644, b"NAME"));
        assert!(!wants_verity("etc/x", 0o755, b""));
        assert!(!wants_verity("usr/share/x", 0o755, b""));
        assert!(!wants_verity(MANIFEST, 0o755, b""));
    }

    #[test]
    fn seal_makes_the_subvolume_read_only() {
        assert_eq!(
            seal_argv("/run/wolos/top/@wolos-b").join(" "),
            "btrfs property set -ts /run/wolos/top/@wolos-b ro true"
        );
        assert_eq!(
            enable_argv("/x/usr/bin/a").join(" "),
            "fsverity enable /x/usr/bin/a"
        );
    }

    #[test]
    fn measure_output_parses_only_a_sha256_digest() {
        let d = "c".repeat(64);
        assert_eq!(parse_measure(&format!("sha256:{d} usr/bin/a\n")), Some(d));
        assert_eq!(parse_measure("sha512:abcd f"), None);
        assert_eq!(parse_measure("sha256:zz f"), None);
        assert_eq!(parse_measure(""), None);
    }

    #[test]
    fn manifest_round_trips_and_has_a_stable_root_digest() {
        let a = m();
        assert_eq!(Manifest::parse(&a.to_text()), a);
        assert_eq!(a.root_digest().len(), 64);
        let mut b = a.clone();
        b.0.insert("usr/bin/evil".into(), "d".repeat(64));
        assert_ne!(a.root_digest(), b.root_digest());
    }

    #[test]
    fn boot_check_verdicts() {
        let man = m();
        let root = man.root_digest();
        let good = |p: &str| man.0.get(p).cloned();
        assert_eq!(verify(&man, &format!("{root}\n"), &good), Verdict::Ok);
        assert_eq!(
            verify(&man, &"0".repeat(64), &good),
            Verdict::ManifestChanged
        );
        let swapped = |p: &str| (p != "usr/bin/a").then(|| man.0[p].clone());
        assert_eq!(
            verify(&man, &root, &swapped),
            Verdict::Files(vec!["usr/bin/a".into()])
        );
        let other = |_: &str| Some("e".repeat(64));
        assert!(matches!(verify(&man, &root, &other), Verdict::Files(f) if f.len() == 2));
    }

    #[test]
    fn a_bad_slot_boots_the_other_unless_that_is_bad_too() {
        let Fallback::Boot { argv } = fallback("wolos-b", true) else {
            panic!()
        };
        assert_eq!(argv[0].join(" "), "bootctl set-default wolos-a.conf");
        assert_eq!(argv[1].join(" "), "systemctl reboot");
        assert_eq!(fallback("wolos-b", false), Fallback::Stay);
        assert_eq!(fallback("x", true), Fallback::Stay);
    }

    #[test]
    fn the_person_is_told_in_plain_words() {
        let t = notice("wolos-b", &Verdict::Files(vec!["usr/bin/a".into()]), true);
        assert!(
            t.contains("previous version") && t.contains("1 protected"),
            "{t}"
        );
        assert!(notice("wolos-a", &Verdict::ManifestChanged, false).contains("reinstall"));
        assert_eq!(notice("wolos-a", &Verdict::Ok, false), "");
    }
}
