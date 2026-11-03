// SPDX-License-Identifier: GPL-2.0-or-later
//! Upstream names stay out of product text and docs (docs/THIRD-PARTY.md:4-5, docs/DOCTRINE.md
//! §6). The projects credited in docs/THIRD-PARTY.md and their copyright holders are named only
//! there (and in NOTICE, COPYRIGHT, LICENSES/); the compositor and the community package
//! collection are "the compositor"/"the desktop" and "community packages" in prose.
//!
//! Literal identifiers stay: they are code, so docs write them in code spans or fences
//! (`hyprctl`, `hyprland.conf`, `Hyprland --verify-config`, the source id `aur`), and lowercase
//! file, package and group names are never matched. Released CHANGELOG entries keep their wording;
//! only the unreleased section is read.
//!
//! The same rule covers the text the `wol` and `wopkg` tools print (help, problems, notes): the
//! string literals of crates/controller, crates/shared, crates/wopkg and wopkg/src (Crystal).
//! The shell's on-screen text is covered by `shell::copy::BANNED` (copy_lint, RU-006).
#[path = "support/repo.rs"]
mod repo;

use repo::{read, repo};
use std::path::{Path, PathBuf};

/// Prose names that need neutral wording, matched case-sensitively as whole words.
const PROSE: [(&str, &str); 2] = [
    ("Hyprland", "say the compositor or the desktop"),
    ("AUR", "say the community package collection"),
];

/// Places where a PROSE name is on purpose: (repo path, text on the line, why).
const ALLOWED: [(&str, &str, &str); 1] = [(
    "docs/DOCTRINE.md",
    "One example: WolOS ships a fork of Hyprland.",
    "§6 names the upstream whose voices the doctrine does not share",
)];

/// Files that name credited projects by design.
const LICENCE_FILES: [&str; 3] = ["docs/THIRD-PARTY.md", "NOTICE", "COPYRIGHT"];

/// Credited projects (the `## id` sections of THIRD-PARTY.md, plus the family name before a
/// `-`) and their copyright holders, lowercased.
fn credited() -> Vec<String> {
    let text = read("docs/THIRD-PARTY.md");
    let mut out = Vec::new();
    for l in text.lines() {
        if let Some(id) = l.strip_prefix("## ") {
            let id = id.trim().to_lowercase();
            if let Some((family, _)) = id.split_once('-') {
                out.push(family.to_owned());
            }
            out.push(id);
        } else if let Some(rest) = l.trim().strip_prefix("Copyright (c) ") {
            // "Copyright (c) 2026 Holder Name"
            let holder = rest.split_once(' ').map_or(rest, |(_, h)| h).trim();
            if !holder.is_empty() {
                out.push(holder.to_lowercase());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Whether `pat` occurs in `t` as a whole word (no letter, digit or `_` either side).
fn word_hits(t: &str, pat: &str) -> bool {
    t.match_indices(pat).any(|(i, _)| {
        let edge = |c: Option<char>| !c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        edge(t[..i].chars().next_back()) && edge(t[i + pat.len()..].chars().next())
    })
}

fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let n = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !matches!(n.as_str(), "target" | ".git" | "node_modules" | ".claude") {
                walk(&p, ext, out);
            }
        } else if p.extension().is_some_and(|x| x == ext) {
            out.push(p);
        }
    }
}

fn rel(p: &Path) -> String {
    let root = repo("").canonicalize().unwrap();
    let p = p.canonicalize().unwrap();
    p.strip_prefix(&root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

/// The docs the rule reads: docs/**/*.md, every README*.md, and the top-level guides.
fn docs() -> Vec<String> {
    let mut v = Vec::new();
    walk(&repo("docs"), "md", &mut v);
    let mut all = Vec::new();
    walk(&repo(""), "md", &mut all);
    v.extend(all.into_iter().filter(|p| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("README"))
    }));
    let mut out: Vec<String> = v.iter().map(|p| rel(p)).collect();
    for top in [
        "BUILD.md",
        "CONTRIBUTING.md",
        "CODE_OF_CONDUCT.md",
        "CHANGELOG.md",
    ] {
        if repo(top).exists() {
            out.push(top.to_owned());
        }
    }
    out.retain(|p| !LICENCE_FILES.contains(&p.as_str()));
    out.sort();
    out.dedup();
    out
}

/// The lines of a doc the rule reads: the CHANGELOG up to its first released version only.
fn doc_lines(path: &str) -> Vec<(usize, String)> {
    let text = read(path);
    let mut out = Vec::new();
    for (i, l) in text.lines().enumerate() {
        if path == "CHANGELOG.md" && l.starts_with("## ") && !l.contains("unreleased") {
            break;
        }
        out.push((i + 1, l.to_owned()));
    }
    out
}

/// A markdown line without its code spans and link targets (`[text](target)` keeps `text`).
fn prose(l: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    let mut chars = l.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '`' {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if c == ']' && chars.peek() == Some(&'(') {
            out.push(c);
            for d in chars.by_ref() {
                if d == ')' {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[test]
fn credited_projects_are_named_only_in_the_licence_files() {
    let names = credited();
    assert!(
        names.len() >= 4,
        "THIRD-PARTY.md lost its sections: {names:?}"
    );
    let mut bad = Vec::new();
    for path in docs() {
        for (n, l) in doc_lines(&path) {
            let lower = l.to_lowercase();
            for name in &names {
                if word_hits(&lower, name) {
                    bad.push(format!("{path}:{n}: “{name}”: {}", l.trim()));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "credited projects and their authors are named only in docs/THIRD-PARTY.md, NOTICE and COPYRIGHT:\n{}",
        bad.join("\n")
    );
}

#[test]
fn docs_prose_says_the_compositor_and_community_packages() {
    let files = docs();
    assert!(
        files.iter().any(|f| f == "docs/DOCTRINE.md") && files.len() > 30,
        "the docs scan lost its files: {files:?}"
    );
    let mut bad = Vec::new();
    let mut allowed_seen = vec![false; ALLOWED.len()];
    for path in files {
        let mut fence = false;
        for (n, l) in doc_lines(&path) {
            if l.trim_start().starts_with("```") || l.trim_start().starts_with("~~~") {
                fence = !fence;
                continue;
            }
            if fence {
                continue;
            }
            let p = prose(&l);
            for (name, why) in PROSE {
                if !word_hits(&p, name) {
                    continue;
                }
                if let Some(i) = ALLOWED.iter().position(|a| a.0 == path && l.contains(a.1)) {
                    allowed_seen[i] = true;
                    continue;
                }
                bad.push(format!("{path}:{n}: “{name}”: {why}: {}", l.trim()));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "upstream names in docs prose (write identifiers in `code`):\n{}",
        bad.join("\n")
    );
    for (i, seen) in allowed_seen.iter().enumerate() {
        assert!(
            seen,
            "stale allowlist entry (fix or drop it): {:?}",
            ALLOWED[i]
        );
    }
}

/// Double-quoted string literals of a Rust or Crystal source, by line, comments and Rust test
/// modules skipped. Good enough for a word search: a Crystal interpolation like
/// `#{n == 1 ? "" : "s"}` splits a literal in parts, each still searched.
fn literals(text: &str, rust: bool) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut cur: Option<(usize, String)> = None;
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim_start();
        if cur.is_none() {
            if rust
                && t.starts_with("#[cfg(test)]")
                && lines[i + 1..]
                    .iter()
                    .find(|n| !n.trim().is_empty())
                    .is_some_and(|n| n.trim_start().starts_with("mod "))
            {
                break;
            }
            if t.starts_with("//") || (!rust && t.starts_with('#')) {
                continue;
            }
        }
        let mut chars = l.chars().peekable();
        while let Some(c) = chars.next() {
            match &mut cur {
                Some((_, s)) => match c {
                    '\\' => {
                        chars.next();
                    }
                    '"' => out.push(cur.take().unwrap()),
                    _ => s.push(c),
                },
                None => match c {
                    '"' => cur = Some((i + 1, String::new())),
                    // '"' as a char literal
                    '\'' if rust && chars.peek() == Some(&'"') => {
                        chars.next();
                    }
                    '\'' if rust && chars.peek() == Some(&'\\') => {
                        chars.next();
                        chars.next();
                    }
                    '/' if chars.peek() == Some(&'/') => break,
                    '#' if !rust && chars.peek() != Some(&'{') => break,
                    _ => {}
                },
            }
        }
        if let Some((_, s)) = &mut cur {
            s.push('\n');
        }
    }
    out
}

#[test]
fn tool_text_says_the_compositor_and_community_packages() {
    let names = credited();
    let mut srcs = Vec::new();
    for dir in [
        "crates/controller/src",
        "crates/shared/src",
        "crates/wopkg/src",
    ] {
        walk(&repo(dir), "rs", &mut srcs);
    }
    walk(&repo("wopkg/src"), "cr", &mut srcs);
    assert!(srcs.len() > 50, "the source scan lost its files");
    let mut bad = Vec::new();
    let mut seen_help = false;
    for p in &srcs {
        let path = rel(p);
        let rust = path.ends_with(".rs");
        if rust && (path.ends_with("/tests.rs") || path.contains("/tests/")) {
            continue;
        }
        let text = std::fs::read_to_string(p).unwrap();
        for (n, s) in literals(&text, rust) {
            if path == "crates/controller/src/cli.rs" && s.contains("compositor render config") {
                seen_help = true;
            }
            for (name, why) in PROSE {
                if word_hits(&s, name) {
                    bad.push(format!("{path}:{n}: “{name}”: {why}: \"{s}\""));
                }
            }
            let lower = s.to_lowercase();
            for name in &names {
                if word_hits(&lower, name) {
                    bad.push(format!("{path}:{n}: “{name}”: credited project: \"{s}\""));
                }
            }
        }
    }
    assert!(
        seen_help,
        "the literal scanner lost the wol help text (cli.rs)"
    );
    assert!(
        bad.is_empty(),
        "upstream names in what wol and wopkg print:\n{}",
        bad.join("\n")
    );
}

#[test]
fn the_scanners_find_what_they_must() {
    assert!(word_hits("the Hyprland fragment", "Hyprland"));
    assert!(word_hits("Hyprland's socket", "Hyprland"));
    assert!(!word_hits("hyprland.conf", "Hyprland"));
    assert!(word_hits("is not in the AUR.", "AUR"));
    assert!(!word_hits("AURORA", "AUR"));
    assert!(!word_hits("aura", "aur"));
    assert_eq!(prose("see `Hyprland --verify-config` here"), "see  here");
    assert_eq!(prose("[the fork](modules/hyprland-fork.md)"), "[the fork]");
    let l = literals(
        "let a = \"one Hyprland\"; // \"not this\"\nlet c = '\"';\n// \"nor this\"\nlet b = \"two\";\n#[cfg(test)]\nmod tests {\n    \"never\"\n}\n",
        true,
    );
    let texts: Vec<&str> = l.iter().map(|x| x.1.as_str()).collect();
    assert_eq!(texts, ["one Hyprland", "two"]);
    let cr = literals(
        "  note \"#{n} AUR thing#{n == 1 ? \"\" : \"s\"} here\" # \"comment\"\n",
        false,
    );
    assert!(cr.iter().any(|x| x.1.contains("AUR thing")), "{cr:?}");
    assert!(!cr.iter().any(|x| x.1.contains("comment")), "{cr:?}");
}
