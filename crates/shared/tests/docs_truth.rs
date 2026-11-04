// SPDX-License-Identifier: BSD-2-Clause
//! Current docs describe the Linux system WolOS is (ADR 0016), not the base it left. Why: a
//! reader who finds ZFS boot environments in the README or a packet filter in the security record
//! acts on a system that does not exist any more (audit5, plan NS1).
//!
//! Read: README.md, BUILD.md, the unreleased section of CHANGELOG.md (released entries keep their
//! wording) and every docs/**/*.md. Not read:
//! - a decision record whose `Status:` line says `superseded` or `historical`: it is the record of
//!   its time, and its successor carries the current facts;
//! - a section marked historical: its heading says "historical", or a line of its own body (before
//!   its first sub-heading) starts with `> Historical`. The mark covers every sub-section, and a
//!   mark before the first `##` covers the file;
//! - a line in [`ALLOWED`], each with its reason.
//!
//! A hit is one of [`STALE`] as a whole word (case-sensitive). Fix the text, mark the section
//! historical, or add an [`ALLOWED`] line saying why it is still true.
use std::path::{Path, PathBuf};

/// Words that only belong to the old base, with what to write instead.
const STALE: [(&str, &str); 8] = [
    ("FreeBSD", "the system is Linux (ADR 0016)"),
    ("ZFS", "btrfs A/B root slots (ADR 0016)"),
    ("rc.d", "systemd units"),
    ("devd", "udev rules"),
    ("pf", "the nftables firewall"),
    ("kldload", "modprobe"),
    ("pkg(8)", "pacman, wopkg"),
    ("bectl", "wol update and the A/B slots"),
];

/// Lines that name the old base on purpose: (repo path, text on the line, why).
const ALLOWED: [(&str, &str, &str); 9] = [
    (
        "docs/adr/0016-arch-linux-base.md",
        "Supersedes the FreeBSD base of 0001",
        "the record that replaced the base names what it replaced",
    ),
    (
        "docs/adr/0016-arch-linux-base.md",
        "rc.d scripts become units",
        "the migration rule: what each old mechanism became",
    ),
    (
        "docs/adr/0016-arch-linux-base.md",
        "FreeBSD-only code (ZFS boot environments, pf,",
        "the list of code the move removed",
    ),
    (
        "docs/modules/boot.md",
        "macOS/FreeBSD` from GPT types",
        "install-check reports another system on the disk (core install::disk::FAMILIES)",
    ),
    (
        "docs/modules/packages.md",
        "\"abi\":\"FreeBSD:15:amd64\"",
        "the label published index files carry; the client reads it and ignores it",
    ),
    (
        "docs/modules/packages.md",
        "`abi` is a label kept from the first index format",
        "explains that label",
    ),
    (
        "docs/README.md",
        "one signed file around a pkg(8) package",
        "index title of ADR 0011, superseded by 0021",
    ),
    (
        "docs/README.md",
        "(historical: FreeBSD base)",
        "index row of a document marked historical",
    ),
    (
        "docs/README.md",
        "(historical record from the FreeBSD base)",
        "index row of a document marked historical",
    ),
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "md") {
            out.push(p);
        }
    }
}

/// Repo-relative paths of the current docs.
fn docs() -> Vec<String> {
    let mut files = vec![
        root().join("README.md"),
        root().join("BUILD.md"),
        root().join("CHANGELOG.md"),
    ];
    walk(&root().join("docs"), &mut files);
    let mut out: Vec<String> = files
        .iter()
        .map(|p| {
            p.strip_prefix(root())
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    out.sort();
    out
}

/// A decision record kept as the record of its time.
fn dated_record(path: &str, text: &str) -> bool {
    path.starts_with("docs/adr/")
        && text
            .lines()
            .find_map(|l| l.strip_prefix("Status:"))
            .is_some_and(|s| {
                let s = s.to_ascii_lowercase();
                s.contains("superseded") || s.contains("historical")
            })
}

fn heading_level(line: &str) -> Option<usize> {
    let n = line.bytes().take_while(|&b| b == b'#').count();
    (n > 0 && line[n..].starts_with(' ')).then_some(n)
}

/// For each line: true when it is inside a section marked historical. Fenced code is not a heading.
fn historical_lines(lines: &[&str]) -> Vec<bool> {
    // (start line, level); level 0 is the text before the first heading.
    let mut heads: Vec<(usize, usize)> = vec![(0, 0)];
    let mut fence = false;
    for (i, l) in lines.iter().enumerate() {
        if l.trim_start().starts_with("```") {
            fence = !fence;
        } else if !fence && let Some(n) = heading_level(l) {
            heads.push((i, n));
        }
    }
    let mut out = vec![false; lines.len()];
    for (k, &(start, level)) in heads.iter().enumerate() {
        let body_end = heads.get(k + 1).map_or(lines.len(), |h| h.0);
        let span_end = heads[k + 1..]
            .iter()
            .find(|h| h.1 <= level)
            .map_or(lines.len(), |h| h.0);
        let titled = level > 0 && lines[start].to_ascii_lowercase().contains("historical");
        let marked = lines[start..body_end]
            .iter()
            .any(|l| l.trim_start().starts_with("> Historical"));
        if titled || marked {
            for o in &mut out[start..span_end] {
                *o = true;
            }
        }
    }
    out
}

/// Whether `word` occurs in `line` with no letter, digit or `_` right before or after it.
fn has_word(line: &str, word: &str) -> bool {
    let joins = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    line.match_indices(word).any(|(i, _)| {
        !joins(line[..i].chars().next_back()) && !joins(line[i + word.len()..].chars().next())
    })
}

/// The unreleased CHANGELOG section: the lines up to the second `## ` heading.
fn changelog_current(lines: &[&str]) -> usize {
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("## "))
        .nth(1)
        .map_or(lines.len(), |(i, _)| i)
}

/// (path, 1-based line, word, line text) of every hit, allowlisted or not.
fn hits() -> Vec<(String, usize, &'static str, String)> {
    let mut out = Vec::new();
    for path in docs() {
        let text = std::fs::read_to_string(root().join(&path)).unwrap();
        if dated_record(&path, &text) {
            continue;
        }
        let lines: Vec<&str> = text.lines().collect();
        let mut historical = historical_lines(&lines);
        if path == "CHANGELOG.md" {
            let end = changelog_current(&lines);
            for h in &mut historical[end..] {
                *h = true;
            }
        }
        for (i, l) in lines.iter().enumerate() {
            if historical[i] {
                continue;
            }
            for (w, _) in STALE {
                if has_word(l, w) {
                    out.push((path.clone(), i + 1, w, (*l).to_owned()));
                }
            }
        }
    }
    out
}

fn allowed(path: &str, line: &str) -> bool {
    ALLOWED
        .iter()
        .any(|(p, needle, _)| *p == path && line.contains(needle))
}

#[test]
fn current_docs_name_no_part_of_the_old_base() {
    let bad: Vec<String> = hits()
        .into_iter()
        .filter(|(p, _, _, l)| !allowed(p, l))
        .map(|(p, n, w, _)| {
            let instead = STALE.iter().find(|s| s.0 == w).unwrap().1;
            format!("{p}:{n}: '{w}' (write: {instead}; or mark the section historical)")
        })
        .collect();
    assert!(
        bad.is_empty(),
        "stale platform words in current docs:\n{}",
        bad.join("\n")
    );
}

#[test]
fn every_allowed_line_still_exists() {
    let hits = hits();
    let stale: Vec<String> = ALLOWED
        .iter()
        .filter(|(p, needle, _)| {
            !hits
                .iter()
                .any(|(hp, _, _, l)| hp == p && l.contains(needle))
        })
        .map(|(p, needle, why)| format!("{p}: '{needle}' ({why})"))
        .collect();
    assert!(
        stale.is_empty(),
        "ALLOWED lines that no longer match a hit (drop them):\n{}",
        stale.join("\n")
    );
}

#[test]
fn the_rules_catch_what_they_should() {
    let files = docs();
    for f in ["README.md", "BUILD.md", "CHANGELOG.md", "docs/README.md"] {
        assert!(files.iter().any(|p| p == f), "{f} is not read");
    }
    assert!(files.iter().filter(|p| p.starts_with("docs/adr/")).count() >= 40);
    assert!(has_word(
        "Updates land in a new ZFS boot environment",
        "ZFS"
    ));
    assert!(has_word("On from the first boot: pf (inbound", "pf"));
    assert!(has_word("`rc.d/wolos_quiet`", "rc.d"));
    assert!(!has_word("a pdf or pfx file, src.d", "pf"));
    assert!(!has_word("src.d", "rc.d"));
    assert!(!has_word("devdocs", "devd"));
    let doc = ["# T", "> Historical. Old.", "FreeBSD", "## A", "FreeBSD"];
    assert_eq!(historical_lines(&doc), vec![true; 5]);
    let doc = [
        "# T",
        "now",
        "## Old (historical)",
        "FreeBSD",
        "### Sub",
        "FreeBSD",
        "## New",
        "FreeBSD",
    ];
    assert_eq!(
        historical_lines(&doc),
        vec![false, false, true, true, true, true, false, false]
    );
    let doc = ["# T", "## M", "> Historical: tools.", "x", "## N", "y"];
    assert_eq!(
        historical_lines(&doc),
        vec![false, true, true, true, false, false]
    );
    assert!(dated_record(
        "docs/adr/0009-x.md",
        "# 0009\nStatus: superseded by 0016\n"
    ));
    assert!(!dated_record(
        "docs/adr/0016-x.md",
        "# 0016\nStatus: accepted. Supersedes 0001\n"
    ));
}

/// QA-ADR-7/32: the code of conduct is the doctrine, not a joke that says there is none.
#[test]
fn coc_points_at_doctrine() {
    let doctrine =
        std::fs::read_to_string(root().join("docs/DOCTRINE.md")).expect("docs/DOCTRINE.md");
    assert!(
        doctrine.contains("This is the code of conduct of WolOS"),
        "docs/DOCTRINE.md no longer calls itself the code of conduct"
    );
    let coc =
        std::fs::read_to_string(root().join("CODE_OF_CONDUCT.md")).expect("CODE_OF_CONDUCT.md");
    assert!(
        coc.contains("(docs/DOCTRINE.md)"),
        "CODE_OF_CONDUCT.md does not link the doctrine"
    );
    assert!(
        !coc.to_lowercase()
            .contains("we do not have a code of conduct"),
        "CODE_OF_CONDUCT.md denies having a code of conduct"
    );
}

/// QA-ADR-28/6 (lead decision 2026-11-04): 0.13 asks no `Signed-off-by` on each commit; the hub's
/// Contributor Licence Agreement covers contributions. The guide, STANDARDS §6 and CI say the same:
/// no document asks for the trailer on every commit and no workflow refuses a commit without it.
/// (Kernel patches keep the Linux rules: `Signed-off-by` by a person, `Assisted-by` for a tool.)
#[test]
fn contributions_are_covered_by_the_cla_not_a_sign_off() {
    let read = |p: &str| std::fs::read_to_string(root().join(p)).unwrap();
    let guide = read("CONTRIBUTING.md");
    assert!(
        guide.contains("Contributor Licence Agreement"),
        "CONTRIBUTING.md names the CLA"
    );
    assert!(!guide.contains("There is no CLA"), "CONTRIBUTING.md");
    assert!(
        !guide.contains("Every commit carries a `Signed-off-by`"),
        "CONTRIBUTING.md"
    );
    let std = read("docs/STANDARDS.md");
    assert!(
        !std.contains("`Signed-off-by` on every commit"),
        "STANDARDS §6"
    );
    assert!(
        std.contains("Contributor Licence Agreement"),
        "STANDARDS §6 names the CLA"
    );
    assert!(
        !root().join(".github/workflows/dco.yml").exists(),
        "no sign-off gate in CI"
    );
}

/// QA-ADR-11/12/13/35/36: the records say what the code does. A decision line that names a fact the
/// code can lose is read here; the code half of each is a test of its own (`Flow::new` encrypts:
/// `crates/shell/src/install_flow/tests.rs`; the Secure Boot offer: the same file; the ratchets:
/// `rules_rust.rs`).
#[test]
fn records_of_installer_boot_and_ratchet_decisions_say_what_the_code_does() {
    let read = |p: &str| std::fs::read_to_string(root().join(p)).unwrap();
    let a23 = read("docs/adr/0023-installer-plans.md");
    assert!(
        a23.contains("encrypted unless the person turns it off"),
        "0023"
    );
    let plan = read("crates/shell/src/install_flow.rs");
    assert!(
        plan.contains("protect: if tpm { Protect::Tpm } else { Protect::Password }"),
        "the installer no longer starts encrypted: 0023 says it does"
    );
    let a25 = read("docs/adr/0025-signed-boot.md");
    assert!(
        a25.contains("Secure Boot is the person's choice in the installer"),
        "0025"
    );
    assert!(
        plan.contains("secure: false,"),
        "Secure Boot starts off: 0025 says it does"
    );
    let a37 = read("docs/adr/0037-shower-identity.md");
    assert!(
        a37.contains("Not landed"),
        "0037 must say the dumpable call is not landed"
    );
    let shell_calls_prctl = [
        "crates/shell/src/main.rs",
        "crates/shell/src/bin/wol-shell.rs",
    ]
    .iter()
    .any(|p| std::fs::read_to_string(root().join(p)).is_ok_and(|t| t.contains("PR_SET_DUMPABLE")));
    assert!(
        !shell_calls_prctl,
        "the shell now sets PR_SET_DUMPABLE: amend 0037 where it says Not landed"
    );
    let rules = read("docs/RULES.md");
    let row = |id: &str| {
        rules
            .lines()
            .find(|l| l.starts_with(&format!("| {id} ")))
            .unwrap_or("")
            .to_owned()
    };
    assert!(
        row("RR-068").contains("`BASELINE`") && row("RR-068").contains("never rises"),
        "RR-068 is a ratchet"
    );
    assert!(row("RR-010").contains("`BASELINE`"), "RR-010 is a ratchet");
}
