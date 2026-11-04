// SPDX-License-Identifier: BSD-2-Clause
//! The Rust rules of docs/RULES.md (ids RR-nnn), one `#[test]` each, enforced by scanning every
//! `.rs` file under `crates/` (and every `Cargo.toml`) with a small lexer: comments, strings and
//! char literals are blanked first (so a word in a comment or a string never counts), and every
//! `#[cfg(test)]` item is blanked for the rules that are about shipped code. Each rule counts its
//! hits per crate and compares them with a growth-only `BASELINE` row (rule, crate, count): a row
//! missing means zero. A count above its row fails; below passes and prints the lowered row. Run
//! `cargo test -p shared --test rules_rust -- --nocapture --test-threads=1 | grep BASELINE-ROW`
//! to print the current table. The invariant: the debt only comes down, and a new crate, file or
//! line starts clean. The doc and this file name the same ids (`the_doc_and_the_tests_agree`).
#[path = "support/lexer.rs"]
mod lexer;
use lexer::{is_ident, lex};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Frozen counts (rule, crate, count), taken 2026-11-02 with this scanner. Only shrinks.
const BASELINE: &[(&str, &str, usize)] = &[
    ("RR-001", "shell", 4),
    ("RR-003", "shell", 1),
    ("RR-003", "xguard-clam", 1),
    ("RR-004", "api", 21),
    ("RR-004", "audio", 4),
    ("RR-004", "controller", 3),
    ("RR-004", "shell", 55),
    // +1 (audit C7-2): guard.rs is the wol-xguard daemon body; it says on stderr (the journal)
    // when the download-folder thread cannot start, like its six other daemon diagnostics.
    ("RR-004", "xguard", 7),
    ("RR-004", "xguard-clam", 5),
    ("RR-008", "controller", 4),
    ("RR-008", "realtime", 4),
    ("RR-008", "shell", 9),
    ("RR-008", "wol-abi", 7),
    ("RR-008", "wopkg", 5),
    ("RR-009", "controller", 2),
    ("RR-009", "wol-abi", 2),
    ("RR-009", "wopkg", 1),
    ("RR-010", "controller", 1),
    ("RR-010", "shell", 1),
    ("RR-010", "wol-abi", 5),
    ("RR-011", "api", 1),
    ("RR-011", "core", 1),
    ("RR-011", "shell", 4),
    ("RR-012", "controller", 27),
    ("RR-012", "shell", 5),
    ("RR-012", "wol-abi", 78),
    ("RR-012", "wopkg", 18),
    ("RR-012", "xguard", 3),
    ("RR-012", "xguard-clam", 1),
    ("RR-013", "api", 6),
    ("RR-013", "controller", 1),
    ("RR-013", "proxy", 1),
    ("RR-013", "shared", 1),
    ("RR-013", "shell", 5),
    ("RR-013", "wol-abi", 2),
    ("RR-013", "wopkg", 3),
    ("RR-013", "xguard", 2),
    ("RR-013", "xguard-clam", 1),
    ("RR-014", "shell", 2),
    ("RR-014", "wopkg", 5),
    ("RR-015", "api", 2),
    ("RR-015", "controller", 1),
    ("RR-015", "core", 7),
    ("RR-015", "shell", 3),
    ("RR-015", "xguard", 14),
    ("RR-017", "api", 10),
    ("RR-017", "audio", 1),
    ("RR-017", "aurlaws", 74),
    ("RR-017", "controller", 3),
    ("RR-017", "core", 65),
    ("RR-017", "proxy", 1),
    ("RR-017", "realtime", 10),
    // +1 (INT-1, brand-ui): theme::accent_ink rounds an f64 channel in 0..=255 to u8, as Color::over does.
    ("RR-017", "shared", 40),
    ("RR-017", "shell", 133),
    ("RR-017", "wol-abi", 21),
    ("RR-017", "wol-lua", 12),
    ("RR-017", "wopkg", 39),
    ("RR-017", "xguard", 65),
    ("RR-017", "xguard-clam", 1),
    ("RR-019", "api", 1),
    ("RR-019", "core", 5),
    ("RR-019", "realtime", 3),
    ("RR-019", "xguard", 1),
    ("RR-020", "aurlaws", 7),
    ("RR-021", "controller", 4),
    ("RR-021", "core", 4),
    ("RR-021", "realtime", 1),
    ("RR-021", "shell", 3),
    ("RR-021", "wopkg", 5),
    ("RR-021", "xguard", 3),
    ("RR-022", "shell", 5),
    ("RR-023", "core", 3),
    ("RR-024", "shared", 36),
    ("RR-026", "api", 1),
    ("RR-026", "aurlaws", 1),
    ("RR-026", "controller", 2),
    ("RR-026", "core", 1),
    ("RR-026", "xguard", 1),
    ("RR-027", "audio", 1),
    ("RR-027", "controller", 1),
    ("RR-027", "core", 10),
    ("RR-027", "realtime", 1),
    ("RR-027", "shell", 55),
    ("RR-027", "wol-abi", 5),
    // wolkit 1 (ADR 0039): `ffi/engine.rs` reads $WOL_KIT_SOCKET and $XDG_RUNTIME_DIR once, at
    // `wolkit_engine_create`, and hands the engine the path; the library proper takes it as a value.
    ("RR-027", "wolkit", 1),
    ("RR-027", "wol-lua", 1),
    ("RR-027", "wopkg", 1),
    ("RR-027", "xguard-clam", 2),
    ("RR-028", "controller", 1),
    ("RR-029", "shell", 4),
    ("RR-030", "shell", 3),
    ("RR-031", "shell", 3),
    ("RR-033", "aurlaws", 1),
    ("RR-034", "core", 1),
    ("RR-034", "shared", 3),
    ("RR-034", "wol-lua", 2),
    ("RR-034", "wopkg", 1),
    ("RR-036", "aurlaws", 4),
    ("RR-036", "wopkg", 18),
    ("RR-036", "xguard", 34),
    ("RR-037", "api", 1),
    ("RR-037", "core", 4),
    ("RR-037", "proxy", 1),
    ("RR-037", "realtime", 1),
    ("RR-037", "shell", 30),
    ("RR-037", "wol-abi", 1),
    ("RR-037", "wopkg", 2),
    ("RR-037", "xguard", 2),
    ("RR-037", "xguard-clam", 1),
    ("RR-038", "shell", 2),
    ("RR-038", "xguard", 1),
    ("RR-041", "api", 4),
    ("RR-041", "audio", 1),
    ("RR-041", "controller", 5),
    ("RR-041", "core", 6),
    ("RR-041", "wopkg", 2),
    ("RR-041", "xguard", 4),
    ("RR-042", "api", 3),
    ("RR-042", "controller", 1),
    ("RR-042", "xguard", 1),
    ("RR-042", "xguard-clam", 1),
    ("RR-045", "aurlaws", 1),
    ("RR-045", "controller", 3),
    ("RR-045", "core", 7),
    ("RR-045", "proxy", 1),
    ("RR-045", "shell", 44),
    ("RR-047", "api", 5),
    ("RR-047", "aurlaws", 1),
    ("RR-047", "controller", 12),
    ("RR-047", "core", 50),
    ("RR-047", "proxy", 6),
    ("RR-047", "realtime", 5),
    ("RR-047", "shared", 28),
    ("RR-047", "shell", 43),
    ("RR-047", "wol-abi", 18),
    ("RR-047", "wol-lua", 1),
    ("RR-047", "wopkg", 7),
    ("RR-047", "xguard", 3),
    ("RR-048", "api", 1),
    ("RR-048", "aurlaws", 1),
    ("RR-048", "controller", 80),
    ("RR-048", "proxy", 1),
    ("RR-048", "realtime", 3),
    ("RR-048", "shared", 6),
    ("RR-048", "shell", 5),
    ("RR-048", "wol-abi", 10),
    ("RR-049", "api", 7),
    ("RR-049", "controller", 1),
    ("RR-049", "core", 1),
    ("RR-049", "shell", 11),
    ("RR-049", "wol-abi", 6),
    ("RR-049", "xguard", 12),
    ("RR-052", "core", 2),
    ("RR-052", "shell", 9),
    ("RR-054", "core", 189),
    ("RR-054", "shared", 97),
    ("RR-055", "core", 205),
    ("RR-055", "shared", 131),
    ("RR-056", "aurlaws", 1),
    ("RR-057", "api", 1),
    ("RR-057", "audio", 1),
    ("RR-057", "aurlaws", 1),
    ("RR-057", "controller", 1),
    ("RR-057", "core", 3),
    ("RR-057", "shared", 4),
    ("RR-057", "shell", 14),
    ("RR-057", "wol-lua", 1),
    ("RR-058", "api", 1),
    ("RR-058", "aurlaws", 20),
    ("RR-058", "controller", 14),
    ("RR-058", "core", 41),
    ("RR-058", "realtime", 1),
    ("RR-058", "shared", 6),
    ("RR-058", "shell", 10),
    ("RR-058", "wol-abi", 1),
    ("RR-058", "wol-lua", 15),
    ("RR-058", "wopkg", 8),
    ("RR-058", "xguard", 1),
    ("RR-060", "core", 13),
    ("RR-064", "aurlaws", 11),
    ("RR-064", "core", 102),
    ("RR-064", "realtime", 4),
    ("RR-064", "shared", 57),
    ("RR-064", "shell", 5),
    ("RR-064", "wopkg", 25),
    ("RR-064", "xguard", 7),
    ("RR-065", "core", 21),
    ("RR-066", "core", 128),
    ("RR-067", "core", 4),
    ("RR-068", "api", 15),
    // +4 (MANUAL 0.12.0, RR-114 justification): controller/src/secret.rs turns terminal echo off
    // for the administrator password; std has no termios and controller depends on no listed crate.
    ("RR-068", "controller", 55),
    ("RR-068", "core", 6),
    ("RR-068", "proxy", 1),
    ("RR-068", "shared", 16),
    ("RR-068", "shell", 6),
    ("RR-068", "wol-lua", 3),
    ("RR-071", "xguard-clam", 1),
    ("RR-073", "xguard-clam", 1),
    ("RR-074", "audio", 1),
    ("RR-074", "aurlaws", 2),
    ("RR-078", "realtime", 1),
    ("RR-078", "shell", 3),
    ("RR-078", "wol-abi", 1),
    ("RR-078", "wol-lua", 1),
    ("RR-078", "wopkg", 1),
    ("RR-078", "xguard", 1),
    ("RR-078", "xguard-clam", 1),
    ("RR-079", "aurlaws", 3),
    ("RR-079", "shell", 6),
    ("RR-080", "audio", 1),
    ("RR-080", "aurlaws", 1),
    ("RR-080", "controller", 2),
    ("RR-080", "xguard", 1),
    ("RR-081", "api", 1),
    ("RR-081", "aurlaws", 1),
    ("RR-081", "controller", 1),
    ("RR-081", "core", 1),
    ("RR-081", "realtime", 2),
    ("RR-081", "shared", 1),
    ("RR-081", "shell", 9),
    ("RR-081", "wopkg", 1),
    ("RR-082", "xguard", 1),
];

// ---------------------------------------------------------------------------------------------
// The lexer (`lex`, shared with unsafe_ratchet.rs in tests/support/lexer.rs) and item ranges.

/// The index after the `}` that closes the `{` at `open` (or the end of `code`).
fn match_brace(code: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    for (k, &c) in code.iter().enumerate().skip(open) {
        if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return k + 1;
            }
        }
    }
    code.len()
}

/// The byte ranges of the items under `#[cfg(test)]` (the attribute through the closing brace or
/// semicolon).
fn test_ranges(code: &[u8]) -> Vec<(usize, usize)> {
    let needle = b"#[cfg(test)]";
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= code.len() {
        if &code[i..i + needle.len()] != needle {
            i += 1;
            continue;
        }
        let mut j = i + needle.len();
        loop {
            while j < code.len() && code[j].is_ascii_whitespace() {
                j += 1;
            }
            if code.get(j..).is_some_and(|r| r.starts_with(b"#[")) {
                while j < code.len() && code[j] != b']' {
                    j += 1;
                }
                j += 1;
            } else {
                break;
            }
        }
        while j < code.len() && code[j] != b'{' && code[j] != b';' {
            j += 1;
        }
        let end = if code.get(j) == Some(&b'{') {
            match_brace(code, j)
        } else {
            (j + 1).min(code.len())
        };
        out.push((i, end));
        i = end.max(i + 1);
    }
    out
}

fn blank_ranges(code: &mut [u8], ranges: &[(usize, usize)]) {
    let len = code.len();
    for &(a, z) in ranges {
        for c in &mut code[a..z.min(len)] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The sources.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Lib,
    Bin,
    Test,
}

struct Src {
    krate: String,
    rel: String,
    kind: Kind,
    text: String,
    /// Code with comments and strings blanked; nothing else skipped.
    full: Vec<u8>,
    /// As `full` with every `#[cfg(test)]` item blanked (what ships).
    code: Vec<u8>,
    /// String literals of the shipped code: (offset, text with quotes).
    lits: Vec<(usize, String)>,
    starts: Vec<usize>,
    crate_has_unsafe: bool,
}

impl Src {
    fn new(krate: &str, rel: &str, text: String) -> Src {
        let kind = kind_of(rel, &text);
        let (full, spans) = lex(&text);
        let ranges = if kind == Kind::Test {
            vec![(0, full.len())]
        } else {
            test_ranges(&full)
        };
        let mut code = full.clone();
        blank_ranges(&mut code, &ranges);
        let lits = spans
            .iter()
            .filter(|(a, _)| !ranges.iter().any(|(r0, r1)| a >= r0 && a < r1))
            .map(|&(a, z)| {
                (
                    a,
                    String::from_utf8_lossy(&text.as_bytes()[a..z]).into_owned(),
                )
            })
            .collect();
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Src {
            krate: krate.to_string(),
            rel: rel.to_string(),
            kind,
            text,
            full,
            code,
            lits,
            starts,
            crate_has_unsafe: false,
        }
    }

    fn line_no(&self, off: usize) -> usize {
        self.starts.partition_point(|&x| x <= off) - 1
    }

    fn line(&self, n: usize) -> &str {
        let Some(&a) = self.starts.get(n) else {
            return "";
        };
        let z = self.starts.get(n + 1).copied().unwrap_or(self.text.len());
        self.text[a..z].trim_end_matches(['\n', '\r'])
    }

    fn line_count(&self) -> usize {
        self.starts.len() - usize::from(self.text.ends_with('\n'))
    }

    fn is_root(&self) -> bool {
        self.rel.ends_with("/src/lib.rs")
            || self.rel.ends_with("/src/main.rs")
            || (self.rel.contains("/src/bin/") && self.rel.matches('/').count() == 3)
    }
}

fn kind_of(rel: &str, text: &str) -> Kind {
    let file = rel.rsplit('/').next().unwrap_or("");
    if rel.contains("/tests/")
        || rel.contains("/benches/")
        || rel.contains("/examples/")
        || file == "build.rs"
        || file == "tests.rs"
        || text.contains("#![cfg(test)]")
    {
        Kind::Test
    } else if rel.contains("/src/bin/") || rel.ends_with("/src/main.rs") {
        Kind::Bin
    } else {
        Kind::Lib
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, ext: &str) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !matches!(
                name.as_str(),
                "target" | "fixtures" | "node_modules" | ".git"
            ) {
                walk(&p, out, ext);
            }
        } else if p.extension().is_some_and(|x| x == ext) {
            out.push(p);
        }
    }
}

fn sources() -> &'static Vec<Src> {
    static S: OnceLock<Vec<Src>> = OnceLock::new();
    S.get_or_init(|| {
        let crates = root().join("crates");
        let mut files = Vec::new();
        walk(&crates, &mut files, "rs");
        files.sort();
        let mut v: Vec<Src> = files
            .iter()
            .filter_map(|f| {
                let rel = f
                    .strip_prefix(&crates)
                    .ok()?
                    .to_string_lossy()
                    .replace('\\', "/");
                let krate = rel.split('/').next()?.to_string();
                let bytes = std::fs::read(f).ok()?;
                Some(Src::new(
                    &krate,
                    &rel,
                    String::from_utf8_lossy(&bytes).into_owned(),
                ))
            })
            .collect();
        let with_unsafe: Vec<String> = v
            .iter()
            .filter(|s| find_words(&s.full, "unsafe").next().is_some())
            .map(|s| s.krate.clone())
            .collect();
        for s in &mut v {
            s.crate_has_unsafe = with_unsafe.contains(&s.krate);
        }
        v
    })
}

/// (crate, Cargo.toml text) for every crate directory.
fn manifests() -> &'static Vec<(String, String)> {
    static M: OnceLock<Vec<(String, String)>> = OnceLock::new();
    M.get_or_init(|| {
        let mut v = Vec::new();
        let rd = std::fs::read_dir(root().join("crates")).expect("crates/ exists");
        for e in rd.flatten() {
            if let Ok(t) = std::fs::read_to_string(e.path().join("Cargo.toml")) {
                v.push((e.file_name().to_string_lossy().into_owned(), t));
            }
        }
        v.sort();
        v
    })
}

// ---------------------------------------------------------------------------------------------
// Counting helpers.

/// Offsets where `needle` starts in `h` with no identifier byte before it (and none after it
/// when it ends in one).
fn find_words<'a>(h: &'a [u8], needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    let n = needle.as_bytes();
    (0..h.len().saturating_sub(n.len() - 1)).filter(move |&i| {
        h[i..].starts_with(n)
            && (i == 0 || !is_ident(h[i - 1]))
            && (!is_ident(n[n.len() - 1]) || h.get(i + n.len()).is_none_or(|&c| !is_ident(c)))
    })
}

/// Offsets of a plain substring.
fn find_all<'a>(h: &'a [u8], needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    let n = needle.as_bytes();
    (0..h.len().saturating_sub(n.len() - 1)).filter(move |&i| h[i..].starts_with(n))
}

fn n_words(h: &[u8], needles: &[&str]) -> usize {
    needles.iter().map(|n| find_words(h, n).count()).sum()
}

fn n_all(h: &[u8], needles: &[&str]) -> usize {
    needles.iter().map(|n| find_all(h, n).count()).sum()
}

/// True when the call at `p` (its leading `.`) is made on a `.take(...)` adapter, so the read
/// stops at that cap (`f.take(CAP + 1).read_to_end(&mut v)`).
fn on_take(h: &[u8], p: usize) -> bool {
    let mut j = p;
    while j > 0 && h[j - 1].is_ascii_whitespace() {
        j -= 1;
    }
    if j == 0 || h[j - 1] != b')' {
        return false;
    }
    let mut depth = 0usize;
    while j > 0 {
        j -= 1;
        match h[j] {
            b')' => depth += 1,
            b'(' => {
                depth -= 1;
                if depth == 0 {
                    return h[..j].ends_with(b".take");
                }
            }
            _ => {}
        }
    }
    false
}

/// True when the literal at `p` is the right side of a bitwise `&` (a permission mask,
/// `mode & 0o777`), not a mode being set.
fn is_mask(h: &[u8], p: usize) -> bool {
    let mut j = p;
    while j > 0 && h[j - 1] == b' ' {
        j -= 1;
    }
    j > 0 && h[j - 1] == b'&' && !h[..j - 1].ends_with(b"&")
}

fn lit_has(s: &Src, needles: &[&str]) -> usize {
    s.lits
        .iter()
        .filter(|(_, t)| needles.iter().any(|n| t.contains(n)))
        .count()
}

/// Casts with `as` to a type that can lose a value.
fn narrowing_casts(code: &[u8]) -> usize {
    const TYPES: [&str; 9] = [
        "u8", "u16", "u32", "i8", "i16", "i32", "i64", "usize", "isize",
    ];
    let mut n = 0;
    for i in 1..code.len().saturating_sub(3) {
        if code[i] == b'a'
            && code[i + 1] == b's'
            && code[i + 2].is_ascii_whitespace()
            && (code[i - 1].is_ascii_whitespace() || code[i - 1] == b')')
        {
            let mut j = i + 2;
            while j < code.len() && code[j].is_ascii_whitespace() {
                j += 1;
            }
            let k = code[j..].iter().take_while(|c| is_ident(**c)).count();
            if TYPES.iter().any(|t| t.as_bytes() == &code[j..j + k]) {
                n += 1;
            }
        }
    }
    n
}

/// (name offset, name, body start, body end) of every `fn` with a body.
fn fn_bodies(code: &[u8]) -> Vec<(usize, String, usize, usize)> {
    let mut out = Vec::new();
    for p in find_words(code, "fn ") {
        let mut j = p + 3;
        while j < code.len() && code[j] == b' ' {
            j += 1;
        }
        let k = code[j..].iter().take_while(|c| is_ident(**c)).count();
        if k == 0 {
            continue;
        }
        let name = String::from_utf8_lossy(&code[j..j + k]).into_owned();
        let mut depth = 0i32;
        let mut m = j + k;
        while m < code.len() {
            match code[m] {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                b'{' if depth == 0 => {
                    out.push((j, name, m, match_brace(code, m)));
                    break;
                }
                b';' if depth == 0 => break,
                _ => {}
            }
            m += 1;
        }
    }
    out
}

/// (test fn name, body range, attribute offset) for every `#[test]` in `full`.
fn tests_in(s: &Src) -> Vec<(String, (usize, usize), usize)> {
    let fns = fn_bodies(&s.full);
    find_all(&s.full, "#[test]")
        .filter_map(|p| {
            let (_, name, a, z) = fns.iter().find(|f| f.0 > p)?;
            Some((name.clone(), (*a, *z), p))
        })
        .collect()
}

fn newlines(code: &[u8], a: usize, z: usize) -> usize {
    code[a..z].iter().filter(|c| **c == b'\n').count()
}

/// True when line `n` of `s` or one of the `up` lines above it contains `needle`.
fn near(s: &Src, n: usize, up: usize, needle: &str) -> bool {
    (n.saturating_sub(up)..=n).any(|l| s.line(l).contains(needle))
}

// ---------------------------------------------------------------------------------------------
// The ratchet.

fn tally(scope: impl Fn(&Src) -> bool, count: impl Fn(&Src) -> usize) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for s in sources().iter().filter(|s| scope(s)) {
        let n = count(s);
        if n > 0 {
            // RULES_FILES=1: the hits per file, to find which file a grown row points at.
            if std::env::var_os("RULES_FILES").is_some() {
                println!("FILE-HIT {} {n}", s.rel);
            }
            *m.entry(s.krate.clone()).or_insert(0) += n;
        }
    }
    m
}

fn ratchet(id: &str, now: BTreeMap<String, usize>) {
    let mut over = Vec::new();
    for (c, n) in &now {
        println!("BASELINE-ROW    (\"{id}\", \"{c}\", {n}),");
        let base = BASELINE
            .iter()
            .find(|(r, k, _)| *r == id && k == c)
            .map_or(0, |b| b.2);
        if *n > base {
            over.push(format!(
                "{id} {c}: {n} hits, baseline {base} (+{})",
                n - base
            ));
        } else if *n < base {
            println!("rules_rust: {id} {c} is down to {n} (baseline {base}): lower the row");
        }
    }
    for (r, k, base) in BASELINE.iter().filter(|(r, ..)| *r == id) {
        if !now.contains_key(*k) {
            println!("rules_rust: {r} {k} is down to 0 (baseline {base}): drop the row");
        }
    }
    assert!(
        over.is_empty(),
        "rules_rust: a rule of docs/RULES.md grew (fix the new hit; do not raise the row):\n{}",
        over.join("\n")
    );
}

macro_rules! rule {
    ($name:ident, $id:expr, $scope:expr, $count:expr) => {
        #[test]
        fn $name() {
            ratchet($id, tally($scope, $count));
        }
    };
}

fn lib(s: &Src) -> bool {
    s.kind == Kind::Lib
}
fn prod(s: &Src) -> bool {
    s.kind != Kind::Test
}
fn any(_: &Src) -> bool {
    true
}

// ---------------------------------------------------------------------------------------------
// Process, output and panics.

rule!(rr_001_no_process_exit_in_libraries, "RR-001", lib, |s| {
    n_all(&s.code, &["process::exit("])
});
rule!(
    rr_002_no_process_abort_in_shipped_code,
    "RR-002",
    prod,
    |s| n_all(&s.code, &["process::abort("])
);
rule!(rr_003_no_println_in_libraries, "RR-003", lib, |s| n_words(
    &s.code,
    &["println!", "print!"]
));
rule!(rr_004_no_eprintln_in_libraries, "RR-004", lib, |s| n_words(
    &s.code,
    &["eprintln!", "eprint!"]
));
rule!(rr_005_no_dbg, "RR-005", any, |s| n_words(
    &s.full,
    &["dbg!"]
));
rule!(rr_006_no_todo_macro_in_shipped_code, "RR-006", prod, |s| {
    n_words(&s.code, &["todo!", "unimplemented!"])
});
rule!(rr_007_no_static_mut, "RR-007", prod, |s| {
    find_words(&s.code, "static mut ").count()
});
rule!(rr_014_no_unwrap_on_runtime_data, "RR-014", prod, |s| {
    find_all(&s.code, ".unwrap()")
        .filter(|&p| {
            let before = &s.code[..p];
            !(before.ends_with(b".lock()")
                || before.ends_with(b".read()")
                || before.ends_with(b".write()"))
        })
        .count()
});
rule!(rr_015_no_expect_on_runtime_data, "RR-015", prod, |s| {
    find_all(&s.code, ".expect(").count()
});
rule!(rr_016_no_panic_in_shipped_code, "RR-016", prod, |s| {
    n_words(&s.code, &["panic!"])
});
rule!(rr_017_no_narrowing_as_casts, "RR-017", prod, |s| {
    narrowing_casts(&s.code)
});
rule!(
    rr_018_expect_messages_name_the_invariant,
    "RR-018",
    prod,
    |s| {
        find_all(&s.code, ".expect(")
            .filter(|&p| {
                let mut j = p + 8;
                while j < s.code.len() && s.code[j].is_ascii_whitespace() {
                    j += 1;
                }
                s.lits.iter().any(|(a, t)| *a == j && t.len() < 12)
            })
            .count()
    }
);

// ---------------------------------------------------------------------------------------------
// Attributes, unsafe, comments.

fn allow_hits(s: &Src) -> Vec<usize> {
    find_all(&s.full, "allow(")
        .filter(|&p| s.full[..p].ends_with(b"#[") || s.full[..p].ends_with(b"#!["))
        .collect()
}

rule!(rr_008_allow_names_its_reason, "RR-008", any, |s| {
    allow_hits(s)
        .into_iter()
        .filter(|&p| {
            let n = s.line_no(p);
            let col = p - s.starts[n];
            !s.line(n).get(col..).unwrap_or("").contains("//")
        })
        .count()
});
rule!(rr_009_no_crate_wide_allow, "RR-009", prod, |s| {
    allow_hits(s)
        .into_iter()
        .filter(|&p| s.code[..p].ends_with(b"#!["))
        .count()
});
rule!(rr_010_no_allow_dead_code, "RR-010", any, |s| {
    if s.rel.contains("/tests/support/") {
        return 0;
    }
    allow_hits(s)
        .into_iter()
        .filter(|&p| {
            s.full[p..].starts_with(b"allow(dead_code") || s.full[p..].starts_with(b"allow(unused")
        })
        .count()
});
rule!(rr_011_ignore_names_its_reason, "RR-011", any, |s| {
    find_all(&s.full, "#[ignore]").count()
});
rule!(rr_012_unsafe_has_a_safety_comment, "RR-012", prod, |s| {
    find_words(&s.code, "unsafe")
        .filter(|&p| {
            let after = s.code[p + 6..].iter().find(|c| !c.is_ascii_whitespace());
            if after == Some(&b'(') {
                return false;
            }
            let n = s.line_no(p);
            !(near(s, n, 5, "SAFETY") || near(s, n, 8, "# Safety"))
        })
        .count()
});
rule!(rr_013_impl_drop_has_a_comment, "RR-013", any, |s| {
    find_all(&s.full, "Drop for ")
        .filter(|&p| {
            let n = s.line_no(p);
            s.line(n).contains("impl")
                && !(s.line(n.wrapping_sub(1)).trim_start().starts_with("//")
                    || s.line(n).contains("//")
                    || s.line(n + 1).trim_start().starts_with("//"))
        })
        .count()
});
rule!(rr_025_todo_names_an_adr_or_issue, "RR-025", any, |s| {
    (0..s.line_count())
        .filter(|&n| {
            let line = s.line(n);
            let Some(at) = line.find("//") else {
                return false;
            };
            let c = &line.as_bytes()[at..];
            let hit = ["TODO", "FIXME", "XXX", "HACK"]
                .iter()
                .any(|w| find_words(c, w).next().is_some());
            let named = line.contains("ADR")
                || line.contains("issue")
                || line.contains("audit")
                || line.contains("STANDARDS")
                || line
                    .as_bytes()
                    .windows(2)
                    .any(|w| (w[0] == b'#' || w[0] == b'R') && w[1].is_ascii_digit());
            hit && !named
        })
        .count()
});
rule!(rr_081_cfg_test_module_is_named_tests, "RR-081", prod, |s| {
    find_all(&s.full, "#[cfg(test)]")
        .filter(|&p| {
            let rest = String::from_utf8_lossy(&s.full[p + 12..(p + 80).min(s.full.len())]);
            let rest = rest.trim_start();
            rest.starts_with("mod ") && !rest.starts_with("mod tests")
        })
        .count()
});

// ---------------------------------------------------------------------------------------------
// Process launch, sockets, environment, threads.

rule!(
    rr_019_daemons_start_processes_through_privileged_command,
    "RR-019",
    |s| prod(s)
        && matches!(
            s.krate.as_str(),
            "api" | "realtime" | "xguard" | "xguard-clam" | "core"
        )
        && !s.rel.ends_with("core/src/privileged.rs")
        && !s.rel.contains("core/src/system/"),
    |s| n_all(&s.code, &["Command::new("])
);
// controller/src/script.rs is left out: `wol script`'s KINDS table names interpreters that run a
// script file as an argv; no shell string is built there (ADR 0041).
rule!(
    rr_020_no_shell_in_command,
    "RR-020",
    |s| prod(s) && !s.rel.ends_with("controller/src/script.rs"),
    |s| {
        lit_has(
            s,
            &[
                "\"sh\"",
                "\"bash\"",
                "\"/bin/sh\"",
                "\"/bin/bash\"",
                "\"/usr/bin/sh\"",
            ],
        )
    }
);
rule!(
    rr_021_socket_reads_go_through_the_bounded_reader,
    "RR-021",
    |s| prod(s) && !s.rel.ends_with("shared/src/bounded.rs"),
    |s| [".read_line(", ".read_to_end(", ".read_to_string("]
        .iter()
        .flat_map(|n| find_all(&s.code, n))
        .filter(|&p| !on_take(&s.code, p))
        .count()
);
rule!(
    rr_022_no_sleep_in_the_shell,
    "RR-022",
    |s| prod(s) && s.krate == "shell",
    |s| n_all(&s.code, &["thread::sleep("])
);
rule!(
    rr_023_no_sleep_in_core_and_shared,
    "RR-023",
    // core/src/system/ is the OS adaptor (as RR-019, RR-066 and RR-067 have it): it waits for
    // children; the pure core around it never sleeps.
    |s| lib(s)
        && matches!(s.krate.as_str(), "core" | "shared")
        && !s.rel.contains("core/src/system/"),
    |s| n_all(&s.code, &["thread::sleep("])
);
rule!(
    rr_024_wire_matches_name_every_variant,
    "RR-024",
    |s| lib(s) && s.krate == "shared",
    |s| find_words(&s.code, "_ =>").count()
);
rule!(rr_026_no_hardcoded_run_user, "RR-026", prod, |s| {
    lit_has(s, &["/run/user", "/var/run/user"])
});
rule!(
    rr_027_libraries_read_the_environment_through_env,
    "RR-027",
    lib,
    |s| n_all(&s.code, &["env::var(", "env::var_os(", "env::vars("])
);
rule!(rr_028_no_set_var, "RR-028", prod, |s| {
    n_all(&s.code, &["env::set_var(", "env::remove_var("])
});
rule!(rr_029_libraries_do_not_read_argv, "RR-029", lib, |s| {
    n_all(&s.code, &["env::args("])
});
rule!(rr_037_threads_have_names, "RR-037", prod, |s| {
    n_all(&s.code, &["thread::spawn("])
});
rule!(
    rr_078_socket_files_set_a_read_timeout,
    "RR-078",
    |s| prod(s) && !s.rel.ends_with("shared/src/bounded.rs"),
    |s| {
        let serves = n_all(
            &s.code,
            &["UnixStream::connect(", ".accept()", ".incoming()"],
        ) > 0;
        let bounded = s.text.contains("set_read_timeout")
            || s.text.contains("bounded::")
            || s.text.contains("LineSplitter")
            || s.text.contains("set_nonblocking");
        usize::from(serves && !bounded)
    }
);

// ---------------------------------------------------------------------------------------------
// Literals.

rule!(
    rr_030_no_failed_in_the_shell_outside_copy,
    "RR-030",
    |s| prod(s) && s.krate == "shell" && !s.rel.ends_with("shell/src/copy.rs"),
    |s| {
        s.lits
            .iter()
            .filter(|(_, t)| t.to_lowercase().contains("failed"))
            .count()
    }
);
rule!(rr_031_no_errno_text_in_literals, "RR-031", prod, |s| {
    lit_has(
        s,
        &[
            "os error",
            "errno",
            "Permission denied",
            "No such file",
            "Operation not permitted",
        ],
    )
});
rule!(rr_032_no_tmp_literal, "RR-032", prod, |s| lit_has(
    s,
    &["\"/tmp/"]
));
rule!(rr_033_no_wildcard_bind, "RR-033", prod, |s| {
    s.lits.iter().filter(|(_, t)| binds_everywhere(t)).count()
});

/// A wildcard listen address in a literal: `0.0.0.0` not inside a longer address
/// (`10.0.0.0/8` is a network), or `[::]:`.
fn binds_everywhere(t: &str) -> bool {
    t.contains("[::]:")
        || t.match_indices("0.0.0.0").any(|(i, _)| {
            let before = t[..i].chars().next_back();
            let after = t[i + 7..].chars().next();
            !before.is_some_and(|c| c.is_ascii_digit() || c == '.')
                && !after.is_some_and(|c| c.is_ascii_digit() || c == '.' || c == '/')
        })
}
rule!(rr_034_no_plain_http, "RR-034", prod, |s| {
    s.lits
        .iter()
        .filter(|(_, t)| {
            t.contains("http://")
                && ![
                    "localhost",
                    "127.0.0.1",
                    "w3.org",
                    "freedesktop",
                    "purl.org",
                    "xmlns",
                ]
                .iter()
                .any(|ok| t.contains(ok))
        })
        .count()
});
rule!(rr_035_tls_verification_stays_on, "RR-035", prod, |s| {
    n_all(&s.code, &["danger_accept_invalid"]) + lit_has(s, &["--insecure"])
});
rule!(rr_036_no_weak_hashes, "RR-036", prod, |s| {
    let low = String::from_utf8_lossy(&s.code).to_lowercase();
    low.matches("md5").count()
        + low.matches("sha1").count()
        + lit_has(s, &["md5", "sha1sum", "sha-1"])
});
rule!(rr_079_no_sudo_literal, "RR-079", prod, |s| {
    lit_has(s, &["\"sudo\"", "\"pkexec\"", "\"doas\""])
});
rule!(rr_080_no_home_literal, "RR-080", prod, |s| {
    s.lits
        .iter()
        .filter(|(_, t)| t.starts_with("\"/home/"))
        .count()
});

// ---------------------------------------------------------------------------------------------
// Unsafe, memory and imports.

rule!(rr_038_no_leaks_or_forgets, "RR-038", prod, |s| {
    n_all(&s.code, &["mem::forget(", "Box::leak("])
});
rule!(
    rr_039_transmute_only_in_the_abi_crate,
    "RR-039",
    |s| prod(s) && s.krate != "wol-abi",
    |s| n_words(&s.code, &["transmute", "transmute_copy"])
);
rule!(rr_040_no_unchecked_operations, "RR-040", prod, |s| {
    n_all(
        &s.code,
        &[
            "get_unchecked",
            "from_utf8_unchecked",
            "unwrap_unchecked",
            "unreachable_unchecked",
        ],
    )
});
rule!(
    rr_041_no_recursive_delete_in_shipped_code,
    "RR-041",
    prod,
    |s| n_all(&s.code, &["remove_dir_all("])
);
rule!(rr_042_no_world_writable_mode, "RR-042", prod, |s| {
    ["0o777", "0o666", "0o776", "0o766"]
        .iter()
        .flat_map(|n| find_all(&s.code, n))
        .filter(|&p| !is_mask(&s.code, p))
        .count()
});
rule!(rr_043_std_lazy_types_only, "RR-043", prod, |s| {
    n_all(&s.code, &["lazy_static!", "once_cell::"])
});
rule!(rr_044_no_extern_crate_or_macro_use, "RR-044", any, |s| {
    n_words(&s.full, &["extern crate "]) + n_all(&s.full, &["#[macro_use]"])
});
rule!(rr_045_no_glob_imports, "RR-045", prod, |s| {
    find_all(&s.code, "::*;")
        .filter(|&p| {
            let l = s.line(s.line_no(p));
            !l.contains("prelude") && l.trim_start().starts_with("use ")
        })
        .count()
});
rule!(
    rr_046_no_boxed_dyn_error_in_core_and_shared,
    "RR-046",
    |s| lib(s) && matches!(s.krate.as_str(), "core" | "shared"),
    |s| n_all(
        &s.code,
        &[
            "Box<dyn Error",
            "Box<dyn std::error::Error",
            "Box<dyn core::error"
        ]
    )
);
rule!(rr_082_no_unit_error_type, "RR-082", prod, |s| {
    find_all(&s.code, ", ()>")
        .filter(|&p| s.line(s.line_no(p)).contains("Result<"))
        .count()
});
rule!(
    rr_065_libc_stays_in_os_crates,
    "RR-065",
    |s| prod(s) && matches!(s.krate.as_str(), "core" | "shell" | "audio" | "aurlaws"),
    |s| n_all(&s.code, &["libc::"])
);
rule!(
    rr_066_core_reaches_the_os_through_host,
    "RR-066",
    |s| lib(s)
        && s.krate == "core"
        && !s.rel.contains("core/src/system/")
        && !s.rel.ends_with("core/src/privileged.rs"),
    // As words: `hyprprefs::load()` is a module path, not `std::fs::`.
    |s| n_words(
        &s.code,
        &[
            "fs::",
            "File::open(",
            "TcpStream",
            "UnixStream",
            "UnixListener"
        ]
    )
);
rule!(
    rr_067_core_takes_the_clock_from_host,
    "RR-067",
    |s| lib(s) && s.krate == "core" && !s.rel.contains("core/src/system/"),
    |s| n_all(&s.code, &["SystemTime::now(", "Instant::now("])
);
rule!(
    rr_068_unsafe_stays_in_the_listed_crates,
    "RR-068",
    |s| {
        prod(s)
            && !matches!(
                s.krate.as_str(),
                "wol-abi"
                    | "wolkit"
                    | "wopkg"
                    | "xguard"
                    | "xguard-clam"
                    | "realtime"
                    // The web-app runtime's engine boundary (ADR 0042): its C shim and the
                    // run-time loaded side, counted by its unsafe_ratchet row.
                    | "webengine"
            )
            && !s.rel.ends_with("api/src/os.rs")
    },
    |s| find_words(&s.code, "unsafe").count()
);
rule!(
    rr_074_crates_without_unsafe_forbid_it,
    "RR-074",
    prod,
    |s| {
        usize::from(s.is_root() && !s.crate_has_unsafe && !s.text.contains("forbid(unsafe_code)"))
    }
);

// ---------------------------------------------------------------------------------------------
// Tests.

rule!(rr_047_test_names_read_as_sentences, "RR-047", any, |s| {
    tests_in(s)
        .iter()
        .filter(|(n, ..)| n.split('_').filter(|w| !w.is_empty()).count() < 3)
        .count()
});
rule!(rr_048_a_test_asserts_something, "RR-048", any, |s| {
    const SAYS: [&str; 11] = [
        "assert",
        "panic!",
        "unwrap",
        "expect",
        "?",
        "unreachable",
        "check",
        "verify",
        "ensure",
        "must",
        "fail",
    ];
    tests_in(s)
        .iter()
        .filter(|(_, (a, z), p)| {
            let body = &s.full[*a..*z];
            let head = &s.full[*p..*a];
            !(SAYS.iter().any(|w| n_all(body, &[w]) > 0) || n_all(head, &["should_panic"]) > 0)
        })
        .count()
});
rule!(rr_049_tests_do_not_sleep, "RR-049", any, |s| {
    n_all(&s.full, &["thread::sleep("]) - n_all(&s.code, &["thread::sleep("])
});
rule!(
    rr_077_every_crate_has_a_test,
    "RR-077",
    |s| s.rel.ends_with("/src/lib.rs") || s.rel.ends_with("/src/main.rs"),
    |s| {
        let has = sources().iter().any(|o| {
            o.krate == s.krate && (o.kind == Kind::Test || n_all(&o.full, &["#[test]"]) > 0)
        });
        usize::from(!has)
    }
);

// ---------------------------------------------------------------------------------------------
// File shape and docs.

const SPDX: &str = "// SPDX-License-Identifier: ";
const IDS: [&str; 5] = [
    "GPL-2.0-or-later",
    "GPL-2.0-only",
    "BSD-2-Clause",
    "MIT",
    "Apache-2.0",
];

rule!(rr_050_file_starts_with_spdx, "RR-050", any, |s| {
    usize::from(!s.line(0).starts_with(SPDX))
});
rule!(rr_051_spdx_id_is_a_project_license, "RR-051", any, |s| {
    let l = s.line(0);
    usize::from(l.starts_with(SPDX) && !IDS.contains(&l[SPDX.len()..].trim()))
});
rule!(rr_052_file_has_a_module_doc, "RR-052", any, |s| {
    usize::from(!s.line(1).starts_with("//!"))
});
rule!(rr_053_module_doc_says_something, "RR-053", any, |s| {
    usize::from(
        s.line(1).starts_with("//!") && s.line(1).trim_start_matches("//!").trim().len() < 20,
    )
});

fn documented(s: &Src, n: usize) -> bool {
    let mut l = n;
    while l > 0 {
        l -= 1;
        let t = s.line(l).trim_start();
        if t.starts_with("#[") || t.starts_with("#![") {
            continue;
        }
        return t.starts_with("///");
    }
    false
}

fn undocumented(s: &Src, needles: &[&str]) -> usize {
    needles
        .iter()
        .flat_map(|nd| find_all(&s.code, nd))
        .filter(|&p| {
            let n = s.line_no(p);
            s.code[s.starts[n]..p]
                .iter()
                .all(|c| c.is_ascii_whitespace())
                && !documented(s, n)
        })
        .filter(|&p| {
            !(s.code[p..].starts_with(b"pub const ") && s.code[p + 10..].starts_with(b"fn "))
        })
        .count()
}

rule!(
    rr_054_pub_fn_has_a_doc_line,
    "RR-054",
    |s| lib(s) && matches!(s.krate.as_str(), "core" | "shared"),
    |s| undocumented(
        s,
        &[
            "pub fn ",
            "pub const fn ",
            "pub async fn ",
            "pub unsafe fn "
        ]
    )
);
rule!(
    rr_055_pub_type_and_const_have_a_doc_line,
    "RR-055",
    |s| lib(s) && matches!(s.krate.as_str(), "core" | "shared"),
    |s| undocumented(
        s,
        &[
            "pub struct ",
            "pub enum ",
            "pub trait ",
            "pub const ",
            "pub static ",
            "pub type "
        ]
    )
);
rule!(rr_056_files_stay_under_2000_lines, "RR-056", any, |s| {
    usize::from(s.line_count() > 2000)
});
rule!(rr_057_functions_stay_under_150_lines, "RR-057", prod, |s| {
    fn_bodies(&s.code)
        .iter()
        .filter(|(_, _, a, z)| newlines(&s.code, *a, *z) > 150)
        .count()
});
rule!(rr_058_lines_stay_under_160_columns, "RR-058", any, |s| {
    (0..s.line_count())
        .filter(|&n| s.line(n).chars().count() > 160)
        .count()
});
rule!(rr_059_no_crlf, "RR-059", any, |s| usize::from(
    s.text.contains("\r\n")
));
rule!(rr_060_no_tabs, "RR-060", any, |s| {
    (0..s.line_count())
        .filter(|&n| s.line(n).contains('\t'))
        .count()
});
rule!(rr_061_file_ends_with_a_newline, "RR-061", any, |s| {
    usize::from(!s.text.is_empty() && !s.text.ends_with('\n'))
});
rule!(rr_062_no_trailing_whitespace, "RR-062", any, |s| {
    (0..s.line_count())
        .filter(|&n| {
            let l = s.line(n);
            l.ends_with(' ') || l.ends_with('\t')
        })
        .count()
});
rule!(rr_063_source_names_are_snake_case, "RR-063", any, |s| {
    if s.rel.contains("/src/bin/") {
        return 0;
    }
    let stem = s
        .rel
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim_end_matches(".rs");
    usize::from(
        !stem
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_'),
    )
});
rule!(
    rr_064_wire_structs_deny_unknown_fields,
    "RR-064",
    lib,
    |s| {
        find_all(&s.code, "derive(")
            .filter(|&p| {
                let end = s.code[p..]
                    .iter()
                    .position(|c| *c == b')')
                    .map_or(p, |e| p + e);
                if n_words(&s.code[p..end], &["Deserialize"]) == 0 {
                    return false;
                }
                let mut first = s.line_no(p);
                while first > 0 && s.line(first - 1).trim_start().starts_with("#[") {
                    first -= 1;
                }
                let item = ["struct ", "enum "]
                    .iter()
                    .filter_map(|k| find_all(&s.code[end..], k).next())
                    .min()
                    .map_or(end, |i| end + i);
                !String::from_utf8_lossy(&s.code[s.starts[first]..item])
                    .contains("deny_unknown_fields")
            })
            .count()
    }
);

// ---------------------------------------------------------------------------------------------
// Cargo.toml.

fn manifest_tally(count: impl Fn(&str) -> usize) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for (c, t) in manifests() {
        let n = count(t);
        if n > 0 {
            m.insert(c.clone(), n);
        }
    }
    m
}

fn has_key(t: &str, key: &str) -> bool {
    t.lines().any(|l| {
        let l = l.trim();
        l.starts_with(&format!("{key} "))
            || l.starts_with(&format!("{key}="))
            || l.starts_with(&format!("{key}."))
    })
}

/// The upstream names RR-118 looks for, as written (identifiers such as `libclamav`, `--clamav`,
/// `clamav/` and `wol-xguard-clam` are lower case and never match).
const UPSTREAM_NAMES: &[&str] = &["ClamAV"];

/// What RR-118 lets stand, and why. Exactly these, nothing by pattern:
/// - the token `ClamAV-VDB`: the first bytes of the publisher's signed file, a data format;
/// - `docs/adr/*.md` below the title line: ADRs are history (titles are not);
/// - `CHANGELOG.md` and `NOTICE`: history, and the license attribution the licenses ask for
///   (both outside the scanned set);
/// - string literals in Rust (user-facing text there is copy_scan's); Crystal sources under
///   `wopkg/` and `wolos-crystal/` are read whole, strings included.
const UPSTREAM_ALLOW: &[&str] = &["ClamAV-VDB"];

/// The lines of `text` RR-118 reads in a file at `rel`: comments of Rust sources (`//...`),
/// `#` comments and `description` of manifests and scripts, and every docs line except an ADR's
/// body.
fn prose_lines(rel: &str, text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (i, l) in text.lines().enumerate() {
        let t = l.trim_start();
        let keep = if rel.ends_with(".cr") {
            // Crystal: comments and string literals alike (its messages are what people read).
            Some(l.to_owned())
        } else if rel.ends_with(".rs") {
            l.find("//").map(|p| l[p..].to_owned())
        } else if rel.ends_with(".md") {
            (!(rel.starts_with("docs/adr/") && i > 0)).then(|| l.to_owned())
        } else if t.starts_with('#') || t.starts_with("description") {
            Some(l.to_owned())
        } else {
            None
        };
        if let Some(k) = keep {
            out.push((i + 1, k));
        }
    }
    out
}

#[test]
fn rr_118_no_upstream_names_in_our_prose() {
    let base = root();
    let mut files = Vec::new();
    for (dir, ext) in [
        ("crates", "rs"),
        ("crates", "toml"),
        ("docs", "md"),
        ("tools", "sh"),
        ("wopkg", "cr"),
        ("wolos-crystal", "cr"),
    ] {
        walk(&base.join(dir), &mut files, ext);
    }
    // Shipped scripts, units and configs: everything under files/ that is text.
    fn every(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                every(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    every(&base.join("files"), &mut files);
    let mut hits = Vec::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else {
            continue; // not text
        };
        let rel = f
            .strip_prefix(&base)
            .unwrap_or(f)
            .to_string_lossy()
            .replace('\\', "/");
        for (n, line) in prose_lines(&rel, &text) {
            let mut l = line.clone();
            for a in UPSTREAM_ALLOW {
                l = l.replace(a, "");
            }
            if UPSTREAM_NAMES.iter().any(|w| l.contains(w)) {
                hits.push(format!("{rel}:{n}: {}", line.trim()));
            }
        }
    }
    let id = "RR-118";
    assert!(
        hits.is_empty(),
        "{id}: an upstream name in our prose (UPSTREAM_ALLOW lists what may stand):\n{}",
        hits.join("\n")
    );
}

/// RR-118 reads what it should and leaves what it should.
#[test]
fn rr_118_reads_comments_and_docs_but_not_history() {
    let w = UPSTREAM_NAMES[0];
    let rs = format!("let x = \"{w} in a string\"; // {w} here\n");
    assert_eq!(
        prose_lines("crates/a/src/x.rs", &rs),
        [(1, format!("// {w} here"))]
    );
    let adr = "# 0028 - Title\nBody that is history\n";
    assert_eq!(
        prose_lines("docs/adr/0028-x.md", adr),
        [(1, "# 0028 - Title".to_owned())]
    );
    let sh = format!("#!/bin/sh\necho {w}\n# a comment\n");
    assert_eq!(prose_lines("tools/x.sh", &sh).len(), 2, "comments only");
    let cr = format!("puts \"{w} found it\"\n");
    assert_eq!(
        prose_lines("wopkg/src/x.cr", &cr).len(),
        1,
        "Crystal strings count"
    );
}

#[test]
fn rr_069_edition_is_2024() {
    ratchet(
        "RR-069",
        manifest_tally(|t| {
            usize::from(
                !(t.contains("edition.workspace = true") || t.contains("edition = \"2024\"")),
            )
        }),
    );
}

#[test]
fn rr_070_crate_declares_a_license() {
    ratchet(
        "RR-070",
        manifest_tally(|t| usize::from(!(has_key(t, "license") || has_key(t, "license-file")))),
    );
}

#[test]
fn rr_071_crate_takes_the_workspace_version() {
    ratchet(
        "RR-071",
        manifest_tally(|t| usize::from(!t.contains("version.workspace = true"))),
    );
}

#[test]
fn rr_072_no_wildcard_dependency_version() {
    ratchet(
        "RR-072",
        manifest_tally(|t| {
            t.lines()
                .filter(|l| l.contains("= \"*\"") || l.contains("version = \"*\""))
                .count()
        }),
    );
}

#[test]
fn rr_073_workspace_dependencies_are_not_redeclared() {
    let rootm = std::fs::read_to_string(root().join("Cargo.toml")).expect("root Cargo.toml");
    let names: Vec<String> = rootm
        .split("[workspace.dependencies]")
        .nth(1)
        .unwrap_or("")
        .lines()
        .skip(1)
        .take_while(|l| !l.starts_with('['))
        .filter_map(|l| l.split('=').next())
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty() && !n.starts_with('#'))
        .collect();
    assert!(names.len() >= 5, "no workspace dependencies parsed");
    ratchet(
        "RR-073",
        manifest_tally(|t| {
            t.lines()
                .filter(|l| {
                    names
                        .iter()
                        .any(|n| l.starts_with(&format!("{n} =")) && !l.contains("workspace"))
                })
                .count()
        }),
    );
}

#[test]
fn rr_075_the_lock_file_is_committed() {
    let mut m = BTreeMap::new();
    if !root().join("Cargo.lock").is_file() {
        m.insert("(root)".to_string(), 1);
    }
    ratchet("RR-075", m);
}

#[test]
fn rr_076_workspace_members_match_the_crate_directories() {
    let rootm = std::fs::read_to_string(root().join("Cargo.toml")).expect("root Cargo.toml");
    let mut m = BTreeMap::new();
    for (c, _) in manifests() {
        if !rootm.contains(&format!("\"crates/{c}\"")) {
            m.insert(c.clone(), 1);
        }
    }
    ratchet("RR-076", m);
}

// ---------------------------------------------------------------------------------------------
// The scanner and the doc.

#[test]
fn the_lexer_blanks_comments_strings_and_cfg_test() {
    let src = "fn a() { x.unwrap(); } // y.unwrap()\nlet s = \"z.unwrap()\"; let c = '\"';\n\
               #[cfg(test)]\nmod tests { fn t() { q.unwrap(); } }\nfn b() { w.unwrap(); }";
    let (code, lits) = lex(src);
    assert_eq!(code.len(), src.len());
    assert_eq!(find_all(&code, ".unwrap()").count(), 3);
    assert_eq!(lits.len(), 1);
    let mut shipped = code.clone();
    blank_ranges(&mut shipped, &test_ranges(&code));
    assert_eq!(find_all(&shipped, ".unwrap()").count(), 2);
}

#[test]
fn the_counters_count_what_they_say() {
    assert_eq!(
        narrowing_casts(b"let a = b as u32; let c = (d) as usize; let e = f as u64; use g as h;"),
        2
    );
    assert_eq!(
        find_words(b"eprintln!(1); println!(2);", "println!").count(),
        1
    );
    let reads =
        b"f.take(CAP + 1).read_to_end(&mut v); s.read_line(&mut l); (g).read_to_end(&mut w);";
    let bounded: Vec<bool> = find_all(reads, ".read_")
        .map(|p| on_take(reads, p))
        .collect();
    assert_eq!(bounded, [true, false, false]);
    let modes = b"m.mode() & 0o777; from_mode(0o666); a && 0o777";
    let masks: Vec<bool> = find_all(modes, "0o").map(|p| is_mask(modes, p)).collect();
    assert_eq!(masks, [true, false, false]);
    let code = b"fn a(x: (u8, u8)) -> u8 { { 1 } }\nfn b();\nfn c() {}";
    let f = fn_bodies(code);
    assert_eq!(
        f.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(),
        ["a", "c"]
    );
    let text = "// SPDX-License-Identifier: MIT\n//! Doc.\n/// d\n#[derive(Debug)]\npub struct A;\npub fn f() {}\n";
    let s = Src::new("x", "x/src/lib.rs", text.to_string());
    assert_eq!(undocumented(&s, &["pub struct "]), 0);
    assert_eq!(undocumented(&s, &["pub fn "]), 1);
}

#[test]
fn every_baseline_row_names_a_known_crate_and_rule() {
    let doc = std::fs::read_to_string(root().join("docs/RULES.md")).unwrap_or_default();
    for (r, c, _) in BASELINE {
        assert!(
            *c == "(root)" || manifests().iter().any(|(k, _)| k == c),
            "BASELINE row {r} names {c}, which is not a crate"
        );
        assert!(
            doc.contains(r),
            "BASELINE row names {r}, which docs/RULES.md does not list"
        );
    }
}

#[test]
fn the_doc_and_the_tests_agree() {
    let doc = std::fs::read_to_string(root().join("docs/RULES.md")).expect("docs/RULES.md");
    let me =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/rules_rust.rs"))
            .expect("this file");
    let mut doc_ids = Vec::new();
    let rows = doc
        .lines()
        .filter(|l| l.starts_with("| RR-0") || l.starts_with("| RR-1"));
    for l in rows {
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        let id = cells[1];
        assert!(
            !doc_ids.contains(&id),
            "{id} appears twice in docs/RULES.md"
        );
        doc_ids.push(id);
        let tested = me.contains(&format!("\"{id}\""));
        let says_test = cells.len() >= 3 && cells[cells.len() - 2].contains("rules_rust");
        assert!(
            tested == says_test,
            "{id}: docs/RULES.md says {} but this file {}",
            if says_test {
                "rules_rust.rs checks it"
            } else {
                "it is not checked here"
            },
            if tested { "has a test" } else { "has none" }
        );
    }
    assert!(
        doc_ids.len() >= 60,
        "docs/RULES.md lists {} Rust rules, at least 60 are required",
        doc_ids.len()
    );
}

/// RR-066 counts the fs module, not a module whose name ends in "fs" (`hyprprefs::`).
#[test]
fn rr_066_counts_the_fs_module_not_names_ending_in_fs() {
    assert_eq!(n_words(b"let d = crate::hyprprefs::load(h);", &["fs::"]), 0);
    assert_eq!(n_words(b"std::fs::read(p); fs::write(p, b)", &["fs::"]), 2);
}

/// RR-033 finds a listen address, not a network that ends in .0.0.0.
#[test]
fn rr_033_finds_wildcard_binds_not_networks() {
    assert!(binds_everywhere("0.0.0.0:8080"));
    assert!(binds_everywhere("bind 0.0.0.0"));
    assert!(binds_everywhere("[::]:631"));
    assert!(!binds_everywhere("ip saddr { 10.0.0.0/8, 172.16.0.0/12 }"));
    assert!(!binds_everywhere("100.0.0.0"));
}
