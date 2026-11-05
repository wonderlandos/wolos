// SPDX-License-Identifier: BSD-2-Clause
//! Sealed system volume: after an install or an update the slot's subvolume is read-only and
//! every executable and library under /usr/bin and /usr/lib carries fs-verity. The sealed
//! files' digests go to a manifest inside the slot; the manifest's own digest (the root digest)
//! goes next to the slot's kernel on the ESP. At boot [`verify`] compares the three; a mismatch
//! makes [`fallback`] boot the other slot and tells the person.
//! Pure: argv lists, text formats and decisions. The `wol seal` verbs run them.
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Directories (inside the slot) whose files are sealed.
pub const DIRS: [&str; 2] = ["usr/bin", "usr/lib"];
/// First line of a complete manifest (QA-F11, BOOT2-5): it lists every regular file and symlink
/// under [`VERITY_DIRS`] and [`HASHED_DIRS`], so a file that is added, swapped for a link or
/// re-pointed fails the check. A manifest without it (sealed before) is checked as before: its
/// listed files only.
pub const COMPLETE: &str = "wolos-seal 2";
/// Directories of a complete manifest whose regular files carry fs-verity: programs, libraries
/// and the data next to them (locale-archive too: locale-gen runs at install, before the seal,
/// and glibc maps it into setuid programs). Absent ones (usr/sbin is a link on Arch) are skipped;
/// a directory that is a symlink is recorded as one.
pub const VERITY_DIRS: [&str; 5] = [
    "usr/bin",
    "usr/lib",
    "usr/local/bin",
    "usr/libexec",
    "usr/sbin",
];
/// Directories of a complete manifest whose files are recorded with a SHA-256 of their content
/// (read whole at each check; small): what decides privileges or runs as code but is not under
/// [`VERITY_DIRS`]. The rest of /usr/share (icons, fonts, translations) is not covered.
pub const HASHED_DIRS: [&str; 4] = [
    "usr/share/polkit-1",
    "usr/share/dbus-1",
    "usr/share/perl5",
    "usr/share/wolos",
];
/// The digest field of a symlink: `link:` and the hex of its target.
pub const LINK: &str = "link:";
/// The digest field of a [`HASHED_DIRS`] file: `sha256:` and the hex of its content.
pub const HASHED: &str = "sha256:";
/// Manifest inside the slot (not itself sealed).
pub const MANIFEST: &str = "usr/lib/wolos/verity.manifest";
/// Root digest of the slot's manifest, on the ESP next to its kernel.
pub fn digest_file(slot: &str) -> String {
    format!("{}/{slot}/verity", crate::update::ESP)
}
/// Where the boot check leaves its message for the shell to show.
pub const NOTICE: &str = "/var/lib/wolos/verity-notice";

/// A file the seal covers: a regular file in [`DIRS`] that is executable or an ELF object.
/// (The rule of manifests before [`COMPLETE`]; see [`covers`] for the complete one.)
pub fn wants_verity(rel: &str, mode: u32, head: &[u8]) -> bool {
    rel != MANIFEST
        && DIRS.iter().any(|d| rel.starts_with(&format!("{d}/")))
        && (mode & 0o111 != 0 || head.starts_with(b"\x7fELF"))
}

/// How a complete manifest covers a path (QA-F11): fs-verity under [`VERITY_DIRS`], a content
/// hash under [`HASHED_DIRS`]; the manifest itself and everything else are outside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cover {
    Verity,
    Hashed,
    Outside,
}

fn under(rel: &str, d: &str) -> bool {
    rel == d || rel.strip_prefix(d).is_some_and(|r| r.starts_with('/'))
}

/// How the seal covers the file at `rel` (relative to the slot root).
pub fn covers(rel: &str) -> Cover {
    if rel == MANIFEST {
        Cover::Outside
    } else if VERITY_DIRS.iter().any(|d| under(rel, d)) {
        Cover::Verity
    } else if HASHED_DIRS.iter().any(|d| under(rel, d)) {
        Cover::Hashed
    } else {
        Cover::Outside
    }
}

/// Every directory a complete manifest walks.
pub fn sealed_dirs() -> impl Iterator<Item = &'static str> {
    VERITY_DIRS.into_iter().chain(HASHED_DIRS)
}

/// The digest field of a [`HASHED_DIRS`] file.
pub fn hash_field(content: &[u8]) -> String {
    let mut s = HASHED.to_owned();
    for b in Sha256::digest(content) {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// The digest field a symlink gets: [`LINK`] and the hex of its target's bytes (no spaces, so
/// the `<digest> <path>` line stays one split).
pub fn link_field(target: &[u8]) -> String {
    let mut s = LINK.to_owned();
    for b in target {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn enable_argv(path: &str) -> Vec<String> {
    ["fsverity", "enable", path].map(String::from).to_vec()
}
pub fn measure_argv(path: &str) -> Vec<String> {
    ["fsverity", "measure", path].map(String::from).to_vec()
}
/// One process for many files: the check runs on every boot, and a process per file costs seconds.
pub fn measure_many_argv(paths: &[String]) -> Vec<String> {
    let mut a = vec!["fsverity".to_owned(), "measure".to_owned()];
    a.extend_from_slice(paths);
    a
}
/// `(path, digest)` of each `sha256:<hex> <path>` line.
pub fn parse_measures(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| {
            let (d, p) = l.split_once(' ')?;
            Some((p.to_owned(), parse_measure(d)?))
        })
        .collect()
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

/// path -> fs-verity digest ([`link_field`] for a symlink, [`hash_field`] under [`HASHED_DIRS`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest(pub BTreeMap<String, String>);

impl Manifest {
    /// One `<digest> <path>` line per file, sorted by path.
    pub fn to_text(&self) -> String {
        self.0.iter().map(|(p, d)| format!("{d} {p}\n")).collect()
    }
    /// [`Self::to_text`] of a complete manifest: [`COMPLETE`] first (the root digest covers it,
    /// so it cannot be taken off without the check seeing it).
    pub fn to_complete_text(&self) -> String {
        format!("{COMPLETE}\n{}", self.to_text())
    }
    /// The text and whether it is complete ([`COMPLETE`] on the first line).
    pub fn parse_text(text: &str) -> (Manifest, bool) {
        let complete = text.lines().next() == Some(COMPLETE);
        let body = if complete {
            text.split_once('\n').map_or("", |(_, b)| b)
        } else {
            text
        };
        (Manifest::parse(body), complete)
    }
    pub fn parse(text: &str) -> Manifest {
        Manifest(
            text.lines()
                .filter(|l| *l != COMPLETE)
                .filter_map(|l| l.split_once(' '))
                .map(|(d, p)| (p.to_owned(), d.to_owned()))
                .collect(),
        )
    }
    /// Paths sealed with fs-verity (what `fsverity measure` is asked about).
    pub fn verity_paths(&self) -> Vec<&String> {
        self.0
            .iter()
            .filter(|(_, d)| !d.starts_with(LINK) && !d.starts_with(HASHED))
            .map(|(p, _)| p)
            .collect()
    }
    /// SHA-256 of a complete manifest's text.
    pub fn complete_root_digest(&self) -> String {
        Sha256::digest(self.to_complete_text().as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
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

/// What the boot check sees of the running slot for a complete manifest.
pub struct Seen<'a> {
    /// fs-verity digest of a path (None = no verity or missing).
    pub measured: &'a dyn Fn(&str) -> Option<String>,
    /// [`link_field`] of a path that is a symlink now (None = not a symlink).
    pub link: &'a dyn Fn(&str) -> Option<String>,
    /// [`hash_field`] of a path's content now (None = missing or not a regular file).
    pub hashed: &'a dyn Fn(&str) -> Option<String>,
    /// Every regular file and symlink under [`sealed_dirs`] now, relative to the slot root.
    pub present: &'a [String],
}

/// [`verify`] for a manifest read with [`Manifest::parse_text`]: a complete one also fails on
/// a path that is present but not listed (added), a listed path that is gone, and a symlink
/// whose target changed or that took a sealed file's place (QA-F11, BOOT2-5).
pub fn verify_text(text: &str, expected: &str, seen: &Seen) -> Verdict {
    let (manifest, complete) = Manifest::parse_text(text);
    if !complete {
        return verify(&manifest, expected, seen.measured);
    }
    if manifest.complete_root_digest() != expected.trim() {
        return Verdict::ManifestChanged;
    }
    let present: std::collections::BTreeSet<&str> =
        seen.present.iter().map(String::as_str).collect();
    let mut bad: Vec<String> = manifest
        .0
        .iter()
        .filter(|(p, d)| {
            if d.starts_with(LINK) {
                (seen.link)(p).as_deref() != Some(d.as_str())
            } else if d.starts_with(HASHED) {
                (seen.link)(p).is_some() || (seen.hashed)(p).as_deref() != Some(d.as_str())
            } else {
                (seen.link)(p).is_some() || (seen.measured)(p).as_deref() != Some(d.as_str())
            }
        })
        .map(|(p, _)| p.clone())
        .collect();
    bad.extend(
        present
            .iter()
            .filter(|p| covers(p) != Cover::Outside && !manifest.0.contains_key(**p))
            .map(|p| (*p).to_owned()),
    );
    bad.sort();
    bad.dedup();
    if bad.is_empty() {
        Verdict::Ok
    } else {
        Verdict::Files(bad)
    }
}

/// The menu entry file of `slot` in a listing of `loader/entries` (file names): `<slot>.conf`
/// (blessed) or a counted `<slot>+N.conf` / `<slot>+N-M.conf` with tries left (N > 0). An entry
/// whose tries are used up already failed to boot three times (QA-SYS-12).
pub fn entry_of<'a>(slot: &str, names: &[&'a str]) -> Option<&'a str> {
    names.iter().copied().find(|n| {
        let Some(rest) = n.strip_prefix(slot).and_then(|r| r.strip_suffix(".conf")) else {
            return false;
        };
        if rest.is_empty() {
            return true;
        }
        let Some(count) = rest.strip_prefix('+') else {
            return false;
        };
        let (left, done) = count.split_once('-').unwrap_or((count, "0"));
        let num = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
        num(left) && num(done) && left.parse::<u32>().is_ok_and(|l| l > 0)
    })
}

/// The image an entry starts (`uki` or `linux` line), as written (`/EFI/wolos/wolos-a.efi`).
pub fn entry_image(text: &str) -> Option<&str> {
    text.lines().find_map(|l| {
        let (k, v) = l.trim().split_once(char::is_whitespace)?;
        matches!(k, "uki" | "linux")
            .then(|| v.trim())
            .filter(|v| v.starts_with('/'))
    })
}

/// What the boot check does about a bad slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fallback {
    /// Make `slot` the default and restart.
    Boot { argv: Vec<Vec<String>> },
    /// No safe slot to go to: carry on and tell the person.
    Stay,
}

/// `other_entry`: the file name of the other slot's boot entry in `loader/entries` (found by
/// [`entry_of`] in the listing: `wolos-a.conf` or a counted `wolos-a+3.conf`), None when the other
/// slot has none, or has failed this check itself (no restart loop).
/// sd-boot's entry id is the file name with its counter and `LoaderEntryDefault` must equal it
/// (`efi_fnmatch`); `bootctl set-default` accepts any text and writes it. A default rebuilt from
/// the slot name (`wolos-a.conf`) matched no entry while the slot was counted, so the machine
/// restarted into the same bad slot (QA-A9-CORE-5). The name comes from the listing, never from
/// the slot name.
pub fn fallback(running: &str, other_entry: Option<&str>) -> Fallback {
    match (crate::update::other_slot(running), other_entry) {
        (Some(o), Some(entry)) if entry_of(o, &[entry]).is_some() => Fallback::Boot {
            argv: vec![
                crate::update::be::activate_argv(entry),
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
    fn one_process_measures_many_files() {
        let d = "a".repeat(64);
        assert_eq!(
            measure_many_argv(&["/a".into(), "/b".into()]),
            ["fsverity", "measure", "/a", "/b"]
        );
        let got = parse_measures(&format!("sha256:{d} /usr/bin/a b\nsha512:ff /c\n"));
        assert_eq!(got, [("/usr/bin/a b".to_owned(), d)]);
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
        let Fallback::Boot { argv } = fallback("wolos-b", Some("wolos-a.conf")) else {
            panic!()
        };
        assert_eq!(argv[0].join(" "), "bootctl set-default wolos-a.conf");
        assert_eq!(argv[1].join(" "), "systemctl reboot");
        assert_eq!(fallback("wolos-b", None), Fallback::Stay);
        assert_eq!(fallback("x", Some("wolos-a.conf")), Fallback::Stay);
        // An entry that is not the other slot's (or not an entry at all) is never handed to bootctl.
        for bad in [
            "wolos-b.conf",
            "wolos-a+0.conf",
            "../wolos-a.conf",
            "wolos-a",
        ] {
            assert_eq!(fallback("wolos-b", Some(bad)), Fallback::Stay, "{bad}");
        }
    }

    /// QA-A9-CORE-5: sd-boot's entry id carries the counter, so the default names the counted
    /// file: `wolos-a.conf` matched no entry while boot-rearm had counted the slot.
    #[test]
    fn the_fallback_names_the_counted_entry() {
        for entry in ["wolos-a+3.conf", "wolos-a+2-1.conf"] {
            let Fallback::Boot { argv } = fallback("wolos-b", Some(entry)) else {
                panic!("{entry}")
            };
            assert_eq!(argv[0].join(" "), format!("bootctl set-default {entry}"));
        }
    }

    /// QA-SYS-12: a half-counted entry is still the other slot's entry; one with no tries left
    /// is not, nor is another slot's or a lookalike.
    #[test]
    fn accepts_counted_entries() {
        let pick = |names: &[&'static str]| entry_of("wolos-a", names);
        assert_eq!(pick(&["wolos-a.conf"]), Some("wolos-a.conf"));
        assert_eq!(
            pick(&["wolos-b+3.conf", "wolos-a+3.conf"]),
            Some("wolos-a+3.conf")
        );
        assert_eq!(pick(&["wolos-a+1-2.conf"]), Some("wolos-a+1-2.conf"));
        assert_eq!(pick(&["wolos-a+2.conf"]), Some("wolos-a+2.conf"));
        assert_eq!(pick(&["wolos-a+0-3.conf"]), None);
        assert_eq!(pick(&["wolos-a-linux-lts.conf", "wolos-ab.conf"]), None);
        assert_eq!(
            pick(&["wolos-a+x.conf", "wolos-a+.conf", "wolos-a+1-.conf"]),
            None
        );
        assert_eq!(
            entry_image("title WolOS\nsort-key wolos\nuki /EFI/wolos/wolos-a.efi\n"),
            Some("/EFI/wolos/wolos-a.efi")
        );
        assert_eq!(
            entry_image("linux /wolos-a/vmlinuz\n"),
            Some("/wolos-a/vmlinuz")
        );
        assert_eq!(entry_image("title WolOS\n"), None);
    }

    /// QA-F11 / BOOT2-5: a complete manifest names symlinks and every file, so an added file, a
    /// re-pointed link and a sealed file swapped for a link all fail; an old manifest keeps the
    /// old rule (its listed files only).
    #[test]
    fn complete_manifest_catches_added_and_relinked_files() {
        let mut man = Manifest::default();
        man.0.insert("usr/bin/coreutils".into(), "a".repeat(64));
        man.0
            .insert("usr/bin/true".into(), link_field(b"coreutils"));
        man.0.insert("usr/lib/os.py".into(), "b".repeat(64));
        man.0.insert(
            "usr/share/polkit-1/rules.d/x.rules".into(),
            hash_field(b"polkit.addRule()"),
        );
        let text = man.to_complete_text();
        assert!(text.starts_with("wolos-seal 2\n"), "{text}");
        let (back, complete) = Manifest::parse_text(&text);
        assert!(complete);
        assert_eq!(back, man);
        assert_eq!(back.verity_paths(), ["usr/bin/coreutils", "usr/lib/os.py"]);
        let root = man.complete_root_digest();
        assert_ne!(root, man.root_digest(), "the header is part of the digest");
        let paths = |extra: &[&str]| -> Vec<String> {
            let mut v: Vec<String> = man.0.keys().cloned().collect();
            v.extend(extra.iter().map(|s| s.to_string()));
            v.push(MANIFEST.into());
            v
        };
        let measured = |p: &str| man.0.get(p).filter(|d| d.len() == 64).cloned();
        let link = |p: &str| (p == "usr/bin/true").then(|| link_field(b"coreutils"));
        let rules = "usr/share/polkit-1/rules.d/x.rules";
        let hashed = |p: &str| (p == rules).then(|| hash_field(b"polkit.addRule()"));
        let ok = paths(&[]);
        let seen = Seen {
            measured: &measured,
            link: &link,
            hashed: &hashed,
            present: &ok,
        };
        assert_eq!(verify_text(&text, &root, &seen), Verdict::Ok);
        // An added file.
        let added = paths(&["usr/lib/qa-tamper"]);
        let seen = Seen {
            measured: &measured,
            link: &link,
            hashed: &hashed,
            present: &added,
        };
        assert_eq!(
            verify_text(&text, &root, &seen),
            Verdict::Files(vec!["usr/lib/qa-tamper".into()])
        );
        // The link now points at a script.
        let relink = |p: &str| (p == "usr/bin/true").then(|| link_field(b"/tmp/evil"));
        let seen = Seen {
            measured: &measured,
            link: &relink,
            hashed: &hashed,
            present: &ok,
        };
        assert_eq!(
            verify_text(&text, &root, &seen),
            Verdict::Files(vec!["usr/bin/true".into()])
        );
        // The link replaced by a plain file (no verity).
        let nolink = |_: &str| None;
        let seen = Seen {
            measured: &measured,
            link: &nolink,
            hashed: &hashed,
            present: &ok,
        };
        assert_eq!(
            verify_text(&text, &root, &seen),
            Verdict::Files(vec!["usr/bin/true".into()])
        );
        // The header taken off: the digest no longer matches.
        let stripped = man.to_text();
        let seen = Seen {
            measured: &measured,
            link: &link,
            hashed: &hashed,
            present: &added,
        };
        assert_eq!(
            verify_text(&stripped, &root, &seen),
            Verdict::ManifestChanged
        );
        // An old manifest: only its listed files count, added ones are not looked at.
        let old = Manifest::parse(&format!("{} usr/bin/coreutils\n", "a".repeat(64)));
        let seen = Seen {
            measured: &measured,
            link: &link,
            hashed: &hashed,
            present: &added,
        };
        assert_eq!(
            verify_text(&old.to_text(), &old.root_digest(), &seen),
            Verdict::Ok
        );
        // A polkit rule rewritten (D1): its content hash no longer matches.
        let rewritten = |p: &str| (p == rules).then(|| hash_field(b"return polkit.Result.YES"));
        let seen = Seen {
            measured: &measured,
            link: &link,
            hashed: &rewritten,
            present: &ok,
        };
        assert_eq!(
            verify_text(&text, &root, &seen),
            Verdict::Files(vec![rules.into()])
        );
        // D2: locale data is sealed like the rest; D1: more program and policy directories.
        assert_eq!(covers("usr/lib/locale/C.utf8/LC_CTYPE"), Cover::Verity);
        for p in [
            "usr/sbin/x",
            "usr/sbin",
            "usr/libexec/x",
            "usr/local/bin/pacman",
        ] {
            assert_eq!(covers(p), Cover::Verity, "{p}");
        }
        for p in [
            "usr/share/polkit-1/actions/a.policy",
            "usr/share/dbus-1/system.d/a.conf",
            "usr/share/perl5/x.pm",
            "usr/share/wolos/hints.tsv",
        ] {
            assert_eq!(covers(p), Cover::Hashed, "{p}");
        }
        assert_eq!(covers("usr/sbinx/y"), Cover::Outside);
        assert_eq!(covers("usr/lib/os.py"), Cover::Verity);
        assert_eq!(covers(MANIFEST), Cover::Outside);
        assert_eq!(covers("usr/share/x"), Cover::Outside);
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
