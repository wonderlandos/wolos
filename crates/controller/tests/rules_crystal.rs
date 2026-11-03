// SPDX-License-Identifier: GPL-2.0-or-later
//! Crystal rules (docs/RULES-crystal.md, ids RCR-nnn): every `.cr` file in the repo (wopkg/, hue/,
//! crates/wol-abi/bindings/crystal/ and any tree added later; the walk is the whole checkout),
//! every `shard.yml`/`shard.lock`, and the build files that could fetch shards are read and checked
//! against a catalogue of safety and consistency rules. Nothing is compiled or run except
//! `crystal tool format --check` and `ameba` (each test says so and is skipped with a message when
//! the tool is missing; WOLOS_REQUIRE_CRYSTAL_LINT=1 turns a missing tool into a failure, CI sets
//! it). A rule the code does not meet yet carries a BASELINE below: the list only shrinks, a new
//! violation fails the test, a fixed one is reported on stderr so the entry is removed.
//! `rcr_090_fixtures_trip_exactly_their_rule` runs every scanner over
//! tests/fixtures/crystal/{good,bad-*}: a bad fixture trips only the rule named on its `# trips:`
//! line, a good one trips nothing. `rcr_091_doc_and_tests_agree` ties the tests to the document.
#[path = "support/siblings.rs"]
mod siblings;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------- baselines (growth-only)

/// RCR-002: non-spec files without a purpose comment (40+ characters in the first 30 lines).
const B_PURPOSE: &[&str] = &["wopkg/src/wopkg/app.cr"];
/// RCR-010: files outside the FFI allowlist that use pointers or other unsafe constructs.
const B_UNSAFE: &[&str] = &[];
/// RCR-020: files that start a process through a shell or with a command line in one string.
const B_PROCESS_SHELL: &[&str] = &["wopkg/spec/completions_spec.cr"];
/// RCR-022: files that use HTTP::Client class methods (no way to set a timeout) or a client
/// without connect and read timeouts.
const B_HTTP_TIMEOUT: &[&str] = &["wopkg/src/wopkg/app.cr", "wopkg/src/wopkg/aur.cr"];
/// RCR-024: untyped JSON/YAML parses per non-spec file (the number may only fall; new code uses
/// `Type.from_json` / `from_yaml`).
const B_UNTYPED_PARSE: &[(&str, usize)] = &[
    ("hue/src/hue/imports.cr", 1),
    ("wopkg/src/wopkg/app.cr", 5),
    ("wopkg/src/wopkg/aur.cr", 2),
    ("wopkg/src/wopkg/aur_guard.cr", 1),
    ("wopkg/src/wopkg/aur_hook.cr", 1),
    ("wopkg/src/wopkg/core.cr", 2),
    ("wopkg/src/wopkg/scan.cr", 1),
];
/// RCR-026: catch-all rescues (bare `rescue`, `rescue e`, `rescue nil` modifier) per non-spec
/// file (the number may only fall; new code names the exception types it handles).
const B_CATCH_ALL: &[(&str, usize)] = &[
    ("wopkg/src/wopkg/app.cr", 8),
    ("wopkg/src/wopkg/aur.cr", 2),
    ("wopkg/src/wopkg/aur_guard.cr", 1),
    ("wopkg/src/wopkg/aur_hook.cr", 3),
    ("wopkg/src/wopkg/sources.cr", 1),
];
/// RCR-030: files with a fixed /tmp path in a string (not a spec).
const B_TMP: &[&str] = &["wopkg/src/wopkg/sandbox.cr"];
/// RCR-032: files using MD5 or SHA-1.
const B_WEAK_HASH: &[&str] = &[];
/// RCR-033: files using a non-secure random generator.
const B_RANDOM: &[&str] = &[];
/// RCR-035: files naming /home/ in a string (not a spec).
const B_HOME: &[&str] = &[];
/// RCR-007: `Rule file` pairs in .ameba.yml `Excluded:` (the legacy files that still break the
/// rule). Only shrinks; a pair here that no longer breaks the rule is reported for removal.
const B_AMEBA_EXCLUDED: &[&str] = &[];

// ---------------------------------------------------------------- policy lists

/// RCR-010, RCR-011: the only files that may declare C bindings or touch raw pointers. A new entry
/// needs an ADR. A directory entry ends in `/`.
const FFI_ALLOW: &[&str] = &[
    "crates/wol-abi/bindings/crystal/",
    "hue/src/hue/color.cr",
    "wopkg/src/wopkg/core.cr",
    "wopkg/src/wopkg/lib_wopkg.cr",
];
/// RCR-042: the shards the repo may depend on (direct and transitive), by name. Empty today:
/// the Crystal code uses the standard library only. A shard is added with an ADR, here, and in
/// docs/THIRD-PARTY.md.
const ALLOWED_SHARDS: &[&str] = &[];
/// RCR-042: ceiling of the number of shard.yml dependency entries plus vendored shards.
const SHARD_CENSUS: usize = ALLOWED_SHARDS.len();
/// RCR-043: top-level names of the Crystal standard library (`require "name/..."`); anything else
/// that is not a relative path is a shard.
const STDLIB: &[&str] = &[
    "annotations",
    "array",
    "atomic",
    "base64",
    "benchmark",
    "big",
    "bit_array",
    "bool",
    "box",
    "channel",
    "char",
    "class",
    "colorize",
    "comparable",
    "compiler",
    "complex",
    "compress",
    "concurrent",
    "crypto",
    "crystal",
    "csv",
    "deque",
    "digest",
    "dir",
    "ecr",
    "enum",
    "enumerable",
    "env",
    "errno",
    "exception",
    "fiber",
    "file",
    "file_utils",
    "float",
    "gc",
    "hash",
    "html",
    "http",
    "humanize",
    "indexable",
    "ini",
    "int",
    "io",
    "iterable",
    "iterator",
    "json",
    "kernel",
    "levenshtein",
    "log",
    "math",
    "mime",
    "mutex",
    "named_tuple",
    "nil",
    "number",
    "oauth",
    "oauth2",
    "object",
    "openssl",
    "option_parser",
    "path",
    "pointer",
    "pretty_print",
    "prelude",
    "process",
    "proc",
    "random",
    "range",
    "reference",
    "regex",
    "semantic_version",
    "set",
    "signal",
    "slice",
    "socket",
    "spec",
    "static_array",
    "string",
    "string_pool",
    "string_scanner",
    "struct",
    "symbol",
    "system",
    "termios",
    "time",
    "tuple",
    "unicode",
    "union",
    "uri",
    "uuid",
    "value",
    "weak_ref",
    "xml",
    "yaml",
];
/// Directories whose `.cr` files are scanner fixtures (wrong on purpose, never linted).
const FIXTURES: &str = "crates/controller/tests/fixtures/crystal/";
/// The ameba release whose rule list .ameba.yml carries (tools/crystal/lint.sh passes the same).
const AMEBA_VERSION: &str = "1.7.0";
/// The Crystal release `shard.yml` asks for; the formatter output differs between releases, so a
/// toolchain older than this is not a reference for RCR-004.
const CRYSTAL_FLOOR: (u32, u32) = (1, 12);

// ---------------------------------------------------------------- repo access

fn repo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let n = e.file_name().to_string_lossy().into_owned();
        if n == "target" || n == ".git" || n == "node_modules" || n == ".work" {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Repo-relative '/'-separated paths of every file under `base` (the checkout, or the overlaid
/// three-repo tree), sorted.
fn all_files(base: &Path) -> Vec<String> {
    let mut v = Vec::new();
    walk(base, &mut v);
    let mut s: Vec<String> = v
        .iter()
        .filter_map(|p| p.strip_prefix(base).ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    s.sort();
    s
}

fn is_fixture(rel: &str) -> bool {
    rel.starts_with(FIXTURES)
}

fn is_spec(rel: &str) -> bool {
    rel.contains("/spec/") || rel.starts_with("spec/") || rel.ends_with("_spec.cr")
}

fn ffi_allowed(rel: &str) -> bool {
    FFI_ALLOW
        .iter()
        .any(|a| if a.ends_with('/') { rel.starts_with(a) } else { rel == *a })
}

/// The Crystal files of this checkout (fixtures left out): (relative path, text).
fn crystal_files() -> &'static Vec<(String, String)> {
    static CELL: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        let base = repo_dir();
        all_files(&base)
            .into_iter()
            .filter(|f| f.ends_with(".cr") && !is_fixture(f))
            .map(|f| {
                let t = std::fs::read_to_string(base.join(&f)).unwrap_or_default();
                (f, t)
            })
            .collect()
    })
}

fn read_repo(rel: &str) -> Option<String> {
    std::fs::read_to_string(repo_dir().join(rel)).ok()
}

// ---------------------------------------------------------------- ratchets

fn ratchet(rule: &str, found: BTreeSet<String>, baseline: &[&str]) {
    let base: BTreeSet<&str> = baseline.iter().copied().collect();
    let new: Vec<&String> = found
        .iter()
        .filter(|f| !base.contains(f.as_str()))
        .collect();
    assert!(
        new.is_empty(),
        "{rule}: new violations (fix them; baselines only shrink):\n{}",
        new.iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    for b in base {
        if !found.contains(b) {
            eprintln!("{rule}: baseline entry {b} is compliant now; remove it");
        }
    }
}

fn ratchet_count(rule: &str, found: BTreeMap<String, usize>, baseline: &[(&str, usize)]) {
    let mut bad = Vec::new();
    for (f, n) in &found {
        let allowed = baseline.iter().find(|(b, _)| b == f).map_or(0, |(_, n)| *n);
        if *n > allowed {
            bad.push(format!("(\"{f}\", {n}),  // allowed {allowed}"));
        } else if *n < allowed {
            eprintln!("{rule}: {f} is down to {n}; lower the baseline");
        }
    }
    for (f, _) in baseline.iter().filter(|(f, _)| !found.contains_key(*f)) {
        eprintln!("{rule}: {f} is down to 0; drop it from the baseline");
    }
    assert!(bad.is_empty(), "{rule}: counts grew:\n{}", bad.join("\n"));
}

// ---------------------------------------------------------------- the Crystal lexer
//
// A small scanner, not a parser: it blanks comments, string bodies, regex bodies, percent
// literals and heredoc bodies (keeping every byte offset and newline), so a pattern in a message
// or a PKGBUILD snippet inside a spec is not mistaken for code. `bare` has all of that blanked,
// `code` has only comments and heredocs blanked, and `strs` holds the string bodies by line.

struct Lexed {
    bare: String,
    code: String,
    strs: Vec<(usize, String)>,
    starts: Vec<usize>,
}

impl Lexed {
    fn line_of(&self, pos: usize) -> usize {
        match self.starts.binary_search(&pos) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
    }
    fn bare_lines(&self) -> Vec<(usize, &str)> {
        self.bare.split('\n').enumerate().map(|(i, l)| (i + 1, l)).collect()
    }
}

fn blank(v: &mut [u8], a: usize, b: usize) {
    let end = b.min(v.len());
    for x in v.iter_mut().take(end).skip(a) {
        if *x != b'\n' {
            *x = b' ';
        }
    }
}

/// Index just after the closing quote of the double-quoted string that opens at `i`.
fn scan_dq(c: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    while j < c.len() {
        match c[j] {
            b'\\' => j += 2,
            b'"' => return j + 1,
            b'#' if c.get(j + 1) == Some(&b'{') => j = skip_interp(c, j + 2),
            _ => j += 1,
        }
    }
    c.len()
}

/// Index just after the `}` that closes an interpolation whose body starts at `j`.
fn skip_interp(c: &[u8], mut j: usize) -> usize {
    let mut depth = 1;
    while j < c.len() {
        match c[j] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return j + 1;
                }
            }
            b'"' => {
                j = scan_dq(c, j);
                continue;
            }
            _ => {}
        }
        j += 1;
    }
    c.len()
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn lex(text: &str) -> Lexed {
    let c = text.as_bytes();
    let n = c.len();
    let mut bare = c.to_vec();
    let mut code = c.to_vec();
    let mut strs = Vec::new();
    let mut starts = vec![0usize];
    for (i, b) in c.iter().enumerate() {
        if *b == b'\n' {
            starts.push(i + 1);
        }
    }
    let line_at = |p: usize, starts: &Vec<usize>| match starts.binary_search(&p) {
        Ok(i) => i + 1,
        Err(i) => i,
    };
    let mut pending: Vec<Vec<u8>> = Vec::new();
    let mut i = 0;
    while i < n {
        let ch = c[i];
        match ch {
            b'\n' => {
                i += 1;
                // heredoc bodies start on the line after their opener
                for term in std::mem::take(&mut pending) {
                    while i < n {
                        let e = c[i..].iter().position(|&x| x == b'\n').map_or(n, |p| i + p);
                        let is_end = std::str::from_utf8(&c[i..e])
                            .map(|l| l.trim().as_bytes() == term.as_slice())
                            .unwrap_or(false);
                        blank(&mut bare, i, e);
                        blank(&mut code, i, e);
                        i = (e + 1).min(n);
                        if is_end {
                            break;
                        }
                    }
                }
            }
            b'#' => {
                let e = c[i..].iter().position(|&x| x == b'\n').map_or(n, |p| i + p);
                blank(&mut bare, i, e);
                blank(&mut code, i, e);
                i = e;
            }
            b'"' => {
                let e = scan_dq(c, i);
                let body_end = e.saturating_sub(1).max(i + 1);
                strs.push((
                    line_at(i, &starts),
                    String::from_utf8_lossy(&c[i + 1..body_end]).into_owned(),
                ));
                blank(&mut bare, i + 1, body_end);
                i = e;
            }
            b'\'' => {
                // char literal: 'a', '\n', '\u{1F600}'
                let lim = (i + 12).min(n);
                let mut j = i + 1;
                if j < n && c[j] == b'\\' {
                    j += 2;
                } else {
                    j += 1;
                }
                while j < lim && c[j] != b'\'' && c[j] != b'\n' {
                    j += 1;
                }
                if j < n && c[j] == b'\'' && j > i + 1 {
                    blank(&mut bare, i + 1, j);
                    blank(&mut code, i + 1, j);
                    i = j + 1;
                } else {
                    i += 1;
                }
            }
            b'`' => {
                let e = c[i + 1..].iter().position(|&x| x == b'`').map_or(n, |p| i + 1 + p + 1);
                blank(&mut bare, i, e);
                bare[i] = 1; // marker: a command literal
                i = e;
            }
            b'%' => {
                let prev = if i > 0 { c[i - 1] } else { b'\n' };
                let nx = c.get(i + 1).copied().unwrap_or(0);
                let (kind, open) = if b"wWiIqQrx".contains(&nx) && b"([{<|".contains(&c.get(i + 2).copied().unwrap_or(0)) {
                    (nx, i + 2)
                } else if b"([{<".contains(&nx) {
                    (b'Q', i + 1)
                } else {
                    (0, 0)
                };
                if kind == 0 || is_ident(prev) || prev == b')' || prev == b']' || prev == b'}' {
                    i += 1;
                    continue;
                }
                let op = c[open];
                let cl = match op {
                    b'(' => b')',
                    b'[' => b']',
                    b'{' => b'}',
                    b'<' => b'>',
                    o => o,
                };
                let mut depth = 1;
                let mut j = open + 1;
                while j < n {
                    if c[j] == b'\\' {
                        j += 2;
                        continue;
                    }
                    if c[j] == cl && cl != op {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    } else if c[j] == cl {
                        break;
                    } else if c[j] == op {
                        depth += 1;
                    }
                    j += 1;
                }
                let j = j.min(n);
                strs.push((
                    line_at(i, &starts),
                    String::from_utf8_lossy(&c[open + 1..j]).into_owned(),
                ));
                blank(&mut bare, open + 1, j);
                if kind == b'x' {
                    bare[i] = 1;
                }
                i = (j + 1).min(n);
            }
            b'/' => {
                // a regex literal when a value cannot end just before it
                let mut k = i;
                while k > 0 && (c[k - 1] == b' ' || c[k - 1] == b'\t') {
                    k -= 1;
                }
                let prev = if k > 0 { c[k - 1] } else { b'\n' };
                let regex = (if prev == b'\n' || b"(,=~!|&{[:;?+-*%^<>".contains(&prev) {
                    true
                } else if is_ident(prev) {
                    let mut s = k;
                    while s > 0 && is_ident(c[s - 1]) {
                        s -= 1;
                    }
                    let w = &c[s..k];
                    [
                        &b"if"[..],
                        b"unless",
                        b"when",
                        b"and",
                        b"or",
                        b"not",
                        b"return",
                        b"puts",
                        b"in",
                        b"then",
                        b"while",
                        b"until",
                        b"elsif",
                        b"case",
                        b"else",
                    ]
                    .contains(&w)
                } else {
                    false
                }) && c.get(i + 1) != Some(&b'/');
                if !regex {
                    i += 1;
                    continue;
                }
                let mut j = i + 1;
                while j < n && c[j] != b'\n' {
                    if c[j] == b'\\' {
                        j += 2;
                        continue;
                    }
                    if c[j] == b'#' && c.get(j + 1) == Some(&b'{') {
                        j = skip_interp(c, j + 2);
                        continue;
                    }
                    if c[j] == b'/' {
                        break;
                    }
                    j += 1;
                }
                let j = j.min(n);
                if j < n && c[j] == b'/' {
                    blank(&mut bare, i + 1, j);
                    i = j + 1;
                } else {
                    i += 1;
                }
            }
            b'<' => {
                if c.get(i + 1) == Some(&b'<') && c.get(i + 2) == Some(&b'-') {
                    let mut j = i + 3;
                    let quote = matches!(c.get(j), Some(b'\'') | Some(b'"'));
                    if quote {
                        j += 1;
                    }
                    let s = j;
                    while j < n && is_ident(c[j]) {
                        j += 1;
                    }
                    if j > s && (c[s].is_ascii_uppercase() || c[s] == b'_') {
                        pending.push(c[s..j].to_vec());
                    }
                    i = j.max(i + 1);
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    Lexed {
        bare: String::from_utf8_lossy(&bare).into_owned(),
        code: String::from_utf8_lossy(&code).into_owned(),
        strs,
        starts,
    }
}

// ---------------------------------------------------------------- scanning helpers

/// One finding: (line, why).
type Finding = (usize, String);

struct Src {
    rel: String,
    text: String,
    lx: Lexed,
}

impl Src {
    fn new(rel: &str, text: &str) -> Src {
        Src {
            rel: rel.to_string(),
            text: text.to_string(),
            lx: lex(text),
        }
    }
}

/// `tok` in `line` with identifier boundaries on the sides that are identifier characters.
fn has_tok(line: &str, tok: &str) -> bool {
    let b = line.as_bytes();
    let t = tok.as_bytes();
    let mut from = 0;
    while let Some(p) = line[from..].find(tok) {
        let s = from + p;
        let e = s + t.len();
        let before_ok = !is_ident(t[0]) || s == 0 || !is_ident(b[s - 1]);
        let after_ok = !is_ident(t[t.len() - 1]) || e >= b.len() || !is_ident(b[e]);
        if before_ok && after_ok {
            return true;
        }
        from = s + 1;
    }
    false
}

/// Byte ranges of the top-level, comma-separated pieces of the argument list of the call whose
/// name ends at `after` in `bare`: inside `( )` when there is one, else to the end of the line.
fn call_pieces(bare: &str, after: usize) -> Vec<(usize, usize)> {
    let b = bare.as_bytes();
    let mut a = after;
    while a < b.len() && (b[a] == b' ' || b[a] == b'\t') {
        a += 1;
    }
    let (start, end) = if a < b.len() && b[a] == b'(' {
        let mut depth = 0i32;
        let mut j = a;
        while j < b.len() {
            match b[j] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        (a + 1, j.min(b.len()))
    } else {
        let e = bare[a..].find('\n').map_or(b.len(), |p| a + p);
        (a, e)
    };
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut s = start;
    for j in start..end {
        match b[j] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                out.push((s, j));
                s = j + 1;
            }
            _ => {}
        }
    }
    if s < end || out.is_empty() {
        out.push((s, end));
    }
    out
}

/// `name: value` piece (a named argument), not `Foo::Bar` or a ternary.
fn named_arg(piece: &str) -> Option<&str> {
    let t = piece.trim_start();
    let end = t.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))?;
    if end == 0 {
        return None;
    }
    let rest = &t[end..];
    (rest.starts_with(':') && !rest.starts_with("::")).then_some(&t[..end])
}

fn find_all(hay: &str, needle: &str) -> Vec<usize> {
    let mut v = Vec::new();
    let mut from = 0;
    while let Some(p) = hay[from..].find(needle) {
        v.push(from + p);
        from += p + needle.len();
    }
    v
}

// ---------------------------------------------------------------- the per-file rules

/// RCR-001: line 1 is the SPDX line.
fn scan_spdx(s: &Src) -> Vec<Finding> {
    let l1 = s.text.lines().next().unwrap_or("");
    match l1.strip_prefix("# SPDX-License-Identifier: ") {
        Some(id) if !id.trim().is_empty() => vec![],
        _ => vec![(1, "line 1 is not `# SPDX-License-Identifier: <id>`".into())],
    }
}

/// RCR-002: a purpose comment: 40+ characters of comment (SPDX and notices left out) in the first
/// 30 lines. Specs name their unit in `describe` and are not asked.
fn scan_purpose(s: &Src) -> Vec<Finding> {
    let mut n = 0;
    for l in s.text.lines().take(30) {
        let t = l.trim_start();
        if let Some(c) = t.strip_prefix('#') {
            let c = c.trim();
            if !c.starts_with("SPDX-") && !c.starts_with("Portions:") {
                n += c.chars().count();
            }
        }
    }
    if n >= 40 {
        vec![]
    } else {
        vec![(2, format!("only {n} characters of purpose comment in the first 30 lines (need 40)"))]
    }
}

/// RCR-003: no carriage returns, a final newline, no trailing blank line, no tabs in code.
fn scan_text_hygiene(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    if let Some(p) = s.text.find('\r') {
        v.push((s.lx.line_of(p), "carriage return".into()));
    }
    if !s.text.is_empty() && !s.text.ends_with('\n') {
        v.push((s.text.lines().count(), "no newline at the end of the file".into()));
    }
    if s.text.ends_with("\n\n") {
        v.push((s.text.lines().count(), "blank line at the end of the file".into()));
    }
    v
}

const UNSAFE_TOKENS: &[&str] = &[
    "Pointer",
    "pointerof",
    "uninitialized",
    "to_unsafe",
    "unsafe_as",
    "unsafe_fetch",
    "unsafe_put",
    "unsafe_chr",
    "realloc",
    "Box",
    "LibC",
    "address",
];

/// RCR-010: raw pointers, unchecked access and casts to pointer types.
fn scan_unsafe(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        for t in UNSAFE_TOKENS {
            if has_tok(l, t) && (*t != "address" || l.contains(".address")) {
                v.push((n, format!("`{t}`")));
            }
        }
        // `.as(Void*)`, `.as(UInt8*)`: a cast to a pointer type
        for p in find_all(l, ".as(") {
            if let Some(e) = l[p..].find(')') {
                if l[p + 4..p + e].trim_end().ends_with('*') {
                    v.push((n, "cast to a pointer type".into()));
                }
            }
        }
    }
    v
}

/// RCR-011: `lib`, `fun` and `@[Link]` declarations.
fn scan_lib(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        let t = l.trim_start();
        if t.starts_with("lib ") || t.starts_with("fun ") || t.starts_with("@[Link") {
            v.push((n, "C binding declaration".into()));
        }
    }
    v
}

const SHELLS: &[&str] = &[
    "sh",
    "bash",
    "dash",
    "zsh",
    "ksh",
    "fish",
    "/bin/sh",
    "/bin/bash",
    "/usr/bin/sh",
    "/usr/bin/bash",
];

/// RCR-020: a command goes out as program + argument array. Not allowed: `shell: true`, a shell
/// with `-c`, a program string that carries arguments or shell syntax, backticks, `%x()`,
/// `system()`.
fn scan_process(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    let bare = &s.lx.bare;
    if let Some(p) = bare.find('\u{1}') {
        v.push((s.lx.line_of(p), "command literal (backticks or %x)".into()));
    }
    for p in find_all(bare, "system(") {
        let prev = bare[..p].chars().last().unwrap_or(' ');
        if !is_ident(prev as u8) && prev != '.' && prev != ':' {
            v.push((s.lx.line_of(p), "`system(` runs a shell command".into()));
        }
    }
    for name in ["run", "new", "exec", "capture", "capture_result", "fork"] {
        let pat = format!("Process.{name}");
        for p in find_all(bare, &pat) {
            let end = p + pat.len();
            if bare.as_bytes().get(end).is_some_and(|b| is_ident(*b)) {
                continue;
            }
            let pieces = call_pieces(bare, end);
            let orig: Vec<&str> = pieces.iter().map(|(a, b)| &s.text[*a..*b]).collect();
            let bp: Vec<&str> = pieces.iter().map(|(a, b)| &bare[*a..*b]).collect();
            let line = s.lx.line_of(p);
            let positional: Vec<usize> = (0..bp.len()).filter(|i| named_arg(bp[*i]).is_none()).collect();
            for i in 0..bp.len() {
                if named_arg(bp[i]) == Some("shell") && orig[i].contains("true") {
                    v.push((line, "`shell: true`".into()));
                }
            }
            if let Some(&f) = positional.first() {
                let prog = orig[f].trim();
                if let Some(lit) = prog.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
                    let plain = strip_interp(lit);
                    if plain.contains(char::is_whitespace) || plain.contains([';', '|', '&', '<', '>', '$', '`']) {
                        v.push((line, format!("program string `{lit}` carries arguments or shell syntax")));
                    }
                    if SHELLS.contains(&lit) && orig[f + 1..].iter().any(|o| has_dash_c(o)) {
                        v.push((line, format!("`{lit} -c`: a shell command string")));
                    }
                }
            }
        }
    }
    v
}

fn strip_interp(s: &str) -> String {
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'#' && b.get(i + 1) == Some(&b'{') {
            i = skip_interp(b, i + 2);
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

fn has_dash_c(piece: &str) -> bool {
    ["\"-c\"", "\"-lc\"", "\"-ec\"", "\"-ic\"", "%w(-c", "%w[-c"]
        .iter()
        .any(|p| piece.contains(p))
}

/// RCR-021: code that is built or loaded at run time.
fn scan_dynamic_code(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        let mut rest = l;
        while let Some(p) = rest.find(['{', '%']) {
            let tail = &rest[p..];
            if tail.starts_with("{{") || tail.starts_with("{%") {
                let inner = &tail[2..];
                let inner = inner.split("}}").next().unwrap_or(inner);
                if has_tok(inner, "run") || has_tok(inner, "system") || inner.contains('\u{1}') {
                    v.push((n, "macro runs a program at compile time".into()));
                }
            }
            rest = &rest[p + 1..];
        }
        if has_tok(l, "dlopen") || has_tok(l, "dlsym") || has_tok(l, "Dl_info") {
            v.push((n, "dynamic library loading".into()));
        }
    }
    for (n, st) in &s.lx.strs {
        if st == "crystal" || st.starts_with("crystal ") {
            v.push((*n, "starts the compiler at run time".into()));
        }
        if st.starts_with("compiler/") && s.text.lines().nth(*n - 1).is_some_and(|l| l.trim_start().starts_with("require")) {
            v.push((*n, "requires the compiler".into()));
        }
    }
    v
}

const HTTP_CLASS: &[&str] = &["get", "post", "put", "patch", "delete", "head", "exec", "options", "get_response"];

/// RCR-022: network clients set connect and read timeouts. The `HTTP::Client.get` family cannot.
fn scan_http_timeouts(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    let bare = &s.lx.bare;
    for m in HTTP_CLASS {
        let pat = format!("HTTP::Client.{m}");
        for p in find_all(bare, &pat) {
            if bare.as_bytes().get(p + pat.len()).is_some_and(|b| is_ident(*b)) {
                continue;
            }
            v.push((s.lx.line_of(p), format!("`HTTP::Client.{m}` has no timeouts; build a client and set them")));
        }
    }
    let has_connect = bare.contains("connect_timeout");
    let has_read = bare.contains("read_timeout");
    for p in find_all(bare, "HTTP::Client.new") {
        if !(has_connect && has_read) {
            v.push((s.lx.line_of(p), "`HTTP::Client.new` without connect_timeout and read_timeout in the file".into()));
        }
    }
    for p in find_all(bare, "TCPSocket.new") {
        let pieces = call_pieces(bare, p + "TCPSocket.new".len());
        if !pieces.iter().any(|(a, b)| bare[*a..*b].contains("connect_timeout")) {
            v.push((s.lx.line_of(p), "`TCPSocket.new` without connect_timeout".into()));
        }
    }
    v
}

/// RCR-023: TLS certificate and host name checks stay on.
fn scan_tls(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        if l.contains("VerifyMode::NONE") || l.contains("verify_mode =") || l.contains("Context::Client.insecure") || has_tok(l, "insecure") {
            v.push((n, "TLS verification switched off or weakened".into()));
        }
    }
    v
}

/// RCR-024: untyped decoding of JSON and YAML. Specs are not asked (they read their own output).
fn scan_untyped_parse(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        for pat in ["JSON.parse", "YAML.parse", "YAML::Nodes.parse", "JSON::PullParser.new", "YAML::PullParser.new"] {
            for p in find_all(l, pat) {
                if !l.as_bytes().get(p + pat.len()).is_some_and(|b| is_ident(*b)) || pat.ends_with("parse") {
                    v.push((n, format!("`{pat}`")));
                }
            }
        }
    }
    v
}

/// RCR-025: nothing rescues `Exception` itself.
fn scan_rescue_exception(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        if let Some(p) = l.find("rescue") {
            if has_tok(&l[p..], "Exception") {
                v.push((n, "rescues `Exception`".into()));
            }
        }
    }
    v
}

/// RCR-026: catch-all rescues: bare `rescue`, `rescue e` with no type, and the modifier form
/// `expr rescue value`. Specs are not asked.
fn scan_catch_all(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        let t = l.trim();
        if t == "rescue" {
            v.push((n, "bare `rescue`".into()));
        } else if let Some(rest) = t.strip_prefix("rescue ") {
            let r = rest.trim();
            let simple = !r.is_empty() && r.bytes().all(is_ident) && r.as_bytes()[0].is_ascii_lowercase();
            if simple {
                v.push((n, "`rescue e` with no exception type".into()));
            }
        } else if has_tok(t, "rescue") && !t.starts_with("rescue") && !t.starts_with('#') {
            v.push((n, "modifier `rescue` swallows every exception".into()));
        }
    }
    v
}

/// RCR-030: fixed /tmp paths.
fn scan_tmp(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, st) in &s.lx.strs {
        let fixed = ["/tmp/", "/var/tmp/", "/dev/shm/"].iter().any(|p| st.contains(p))
            || st.split_whitespace().any(|w| w == "/tmp" || w == "/var/tmp")
            || st == "/tmp";
        let tempdir_name = st.contains("Dir.tempdir}/") && !st.ends_with("Dir.tempdir}/");
        if fixed || tempdir_name {
            v.push((*n, format!("fixed temporary path in `{}`", st.chars().take(50).collect::<String>())));
        }
    }
    for (n, l) in s.lx.code.lines().enumerate() {
        if l.contains("Dir.tempdir,") && l.contains("File.join") {
            v.push((n + 1, "File.join(Dir.tempdir, <fixed name>)".into()));
        }
    }
    v
}

/// RCR-031: nothing is created or made world-writable.
fn scan_world_writable(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        if l.contains("OtherWrite") || l.contains("Permissions::All") {
            v.push((n, "world-writable permission flag".into()));
        }
        let ctx = ["chmod", "perm", "mkdir", "File.new", "File.open", "File.write", "umask", "mkfifo", "tempfile", "File.copy"]
            .iter()
            .any(|w| l.contains(w));
        if ctx {
            for p in find_all(l, "0o") {
                let digits: String = l[p + 2..].chars().take_while(|c| c.is_digit(8)).collect();
                if digits.len() >= 3 && matches!(digits.chars().last(), Some('2' | '3' | '6' | '7')) {
                    v.push((n, format!("mode 0o{digits} lets other users write")));
                }
            }
        }
    }
    for (n, st) in &s.lx.strs {
        let t = st.trim();
        let mode_word = matches!(t, "777" | "0777" | "666" | "0666" | "1777" | "a+w" | "o+w" | "go+w" | "ugo+w" | "a+rwx" | "o+rwx" | "a=rwx");
        if mode_word || t.contains(" 777") || t.contains("chmod 666") || t.contains("a+w") || t.contains("o+w") {
            v.push((*n, format!("`{t}` makes something world-writable")));
        }
    }
    v
}

/// RCR-032: no MD5 or SHA-1 where a digest protects anything.
fn scan_weak_hash(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        if l.contains("Digest::MD5") || l.contains("Digest::SHA1") || l.contains("Digest::MD4") {
            v.push((n, "MD5 or SHA-1 digest".into()));
        }
    }
    for (n, st) in &s.lx.strs {
        let up = st.to_ascii_uppercase();
        if matches!(up.as_str(), "MD5" | "MD4" | "SHA1") || st == "digest/md5" || st == "digest/sha1" {
            v.push((*n, format!("`{st}`")));
        }
    }
    v
}

/// RCR-033: secrets come from `Random::Secure`.
fn scan_random(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        if l.contains("Random::Secure") {
            continue;
        }
        if l.contains("Random.rand") || l.contains("Random.new") || l.contains("Random::DEFAULT") || l.contains("Random::PCG32") || l.contains("Random::ISAAC") || has_tok(l, "rand") && l.contains("rand(") && !l.contains(".rand(") {
            v.push((n, "non-secure random generator".into()));
        }
    }
    v
}

/// RCR-034: no literal credentials.
fn scan_credentials(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.code.lines().enumerate() {
        let low = l.to_ascii_lowercase();
        for key in ["password", "passwd", "passphrase", "secret", "token", "api_key", "apikey", "private_key"] {
            if let Some(p) = low.find(key) {
                let after = low[p + key.len()..].trim_start_matches(|c: char| is_ident(c as u8)).trim_start();
                if (after.starts_with("= \"") || after.starts_with("=\"") || after.starts_with(": \"")) && !after.contains("#{") {
                    let lit = after.trim_start_matches(['=', ':', ' ']).trim_start_matches('"');
                    let body = lit.split('"').next().unwrap_or("");
                    if body.len() >= 4 {
                        v.push((n + 1, format!("literal value assigned to `{key}`")));
                    }
                }
            }
        }
    }
    for (n, l) in s.text.lines().enumerate() {
        if l.contains("-----BEGIN") && l.contains("PRIVATE KEY") || l.contains("ghp_") && l.len() > 40 || l.contains("AKIA") && l.matches(char::is_uppercase).count() > 12 {
            v.push((n + 1, "a key or token pasted in the source".into()));
        }
    }
    v
}

/// RCR-035: no /home/ or /Users/ literals.
fn scan_home(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, st) in &s.lx.strs {
        if st.contains("/home/") || st.contains("/Users/") {
            v.push((*n, "home directory literal".into()));
        }
    }
    v
}

/// RCR-043: a `require` names the standard library or a file of this repo.
fn scan_requires(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.code.lines().enumerate() {
        let t = l.trim_start();
        let Some(r) = t.strip_prefix("require ") else {
            continue;
        };
        let name = r.trim().trim_matches('"');
        if name.starts_with("./") || name.starts_with("../") {
            continue;
        }
        let top = name.split('/').next().unwrap_or("");
        if !STDLIB.contains(&top) {
            v.push((n + 1, format!("`require \"{name}\"` is not the standard library: a shard")));
        }
    }
    v
}

// ---------------------------------------------------------------- shard.yml, shard.lock, build files

#[derive(Debug, Default, Clone)]
struct Dep {
    name: String,
    line: usize,
    keys: BTreeMap<String, String>,
}

/// The `dependencies:` and `development_dependencies:` entries of a shard.yml, and its top keys.
fn parse_shard_yml(text: &str) -> (BTreeMap<String, String>, Vec<Dep>) {
    let mut top = BTreeMap::new();
    let mut deps: Vec<Dep> = Vec::new();
    let mut in_deps = false;
    for (i, raw) in text.lines().enumerate() {
        let l = raw.split(" #").next().unwrap_or(raw);
        if l.trim().is_empty() || l.trim_start().starts_with('#') {
            continue;
        }
        let indent = l.len() - l.trim_start().len();
        let t = l.trim();
        if indent == 0 {
            in_deps = t == "dependencies:" || t == "development_dependencies:";
            if let Some((k, v)) = t.split_once(':') {
                top.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
            }
        } else if in_deps {
            if indent <= 2 && t.ends_with(':') {
                deps.push(Dep { name: t.trim_end_matches(':').to_string(), line: i + 1, keys: BTreeMap::new() });
            } else if let (Some(d), Some((k, v))) = (deps.last_mut(), t.split_once(':')) {
                d.keys.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
            }
        }
    }
    (top, deps)
}

fn is_sha40(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// RCR-040: shard.yml names `crystal:` and a licence, and every dependency is pinned by a full
/// commit. A tag or a version range can move.
fn scan_shard_yml(text: &str) -> Vec<Finding> {
    let (top, deps) = parse_shard_yml(text);
    let mut v = Vec::new();
    for k in ["name", "version", "crystal", "license"] {
        if !top.contains_key(k) {
            v.push((1, format!("shard.yml has no `{k}:`")));
        }
    }
    for d in &deps {
        match d.keys.get("commit") {
            Some(c) if is_sha40(c) => {}
            Some(c) => v.push((d.line, format!("{}: commit `{c}` is not a full 40-character hash", d.name))),
            None => v.push((d.line, format!("{}: not pinned by `commit:` (a tag, branch or version range can move)", d.name))),
        }
        for k in ["branch", "tag", "path"] {
            if d.keys.contains_key(k) {
                v.push((d.line, format!("{}: `{k}:` is not a pin", d.name)));
            }
        }
        if let Some(ver) = d.keys.get("version") {
            v.push((d.line, format!("{}: `version: {ver}` beside the commit (a range or tag moves)", d.name)));
        }
    }
    v
}

/// The `name -> version` entries of a shard.lock.
fn parse_shard_lock(text: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    let mut cur: Option<String> = None;
    let mut in_shards = false;
    for raw in text.lines() {
        let indent = raw.len() - raw.trim_start().len();
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        if indent == 0 {
            in_shards = t == "shards:";
        } else if in_shards && indent == 2 && t.ends_with(':') {
            cur = Some(t.trim_end_matches(':').to_string());
            m.insert(cur.clone().unwrap(), String::new());
        } else if in_shards && indent >= 4 {
            if let (Some(c), Some(v)) = (&cur, t.strip_prefix("version:")) {
                m.insert(c.clone(), v.trim().trim_matches('"').to_string());
            }
        }
    }
    m
}

/// RCR-041: a shard.yml with dependencies has a shard.lock, the lock names every dependency, and
/// the locked version of each carries the commit that shard.yml pins.
fn scan_shard_lock(yml: &str, lock: Option<&str>) -> Vec<Finding> {
    let (_, deps) = parse_shard_yml(yml);
    if deps.is_empty() {
        return vec![];
    }
    let Some(lock) = lock else {
        return vec![(1, "shard.yml has dependencies but there is no shard.lock".into())];
    };
    let locked = parse_shard_lock(lock);
    let mut v = Vec::new();
    for d in &deps {
        match locked.get(&d.name) {
            None => v.push((d.line, format!("{}: not in shard.lock", d.name))),
            Some(ver) => {
                if let Some(c) = d.keys.get("commit") {
                    if !ver.contains(&format!("git.commit.{c}")) {
                        v.push((d.line, format!("{}: shard.lock says `{ver}`, shard.yml pins {c}", d.name)));
                    }
                }
            }
        }
    }
    v
}

/// RCR-044: a build file that runs `shards` fetches nothing from the network: `install` and
/// `update` are not run; `shards build/check/run` only with `--frozen --local`.
fn scan_shards_network(text: &str) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in text.lines().enumerate() {
        let t = l.trim_start();
        if t.starts_with('#') {
            continue;
        }
        let code = l.split(" #").next().unwrap_or(l);
        let words: Vec<&str> = code.split_whitespace().collect();
        for (i, w) in words.iter().enumerate() {
            let cmd = w.trim_start_matches(['(', '"', '\'', '`', '$']);
            if cmd != "shards" {
                continue;
            }
            let Some(sub) = words.get(i + 1) else {
                continue;
            };
            if ["install", "update", "build", "check", "run", "prune"].contains(sub) {
                let rest = &words[i + 1..];
                if !(rest.contains(&"--frozen") && rest.contains(&"--local")) {
                    v.push((n + 1, format!("`shards {sub}` without --frozen --local fetches from the network")));
                }
            }
        }
    }
    v
}

// ---------------------------------------------------------------- the registry used by the tests and fixtures

type Scanner = fn(&Src) -> Vec<Finding>;

/// Every per-file `.cr` scanner by rule id (what a fixture is run through).
const FILE_SCANNERS: &[(&str, Scanner)] = &[
    ("RCR-001", scan_spdx),
    ("RCR-002", scan_purpose),
    ("RCR-003", scan_text_hygiene),
    ("RCR-010", scan_unsafe),
    ("RCR-011", scan_lib),
    ("RCR-020", scan_process),
    ("RCR-021", scan_dynamic_code),
    ("RCR-022", scan_http_timeouts),
    ("RCR-023", scan_tls),
    ("RCR-024", scan_untyped_parse),
    ("RCR-025", scan_rescue_exception),
    ("RCR-026", scan_catch_all),
    ("RCR-030", scan_tmp),
    ("RCR-031", scan_world_writable),
    ("RCR-032", scan_weak_hash),
    ("RCR-033", scan_random),
    ("RCR-034", scan_credentials),
    ("RCR-035", scan_home),
    ("RCR-043", scan_requires),
];

fn scanner(id: &str) -> Scanner {
    FILE_SCANNERS.iter().find(|(i, _)| *i == id).unwrap().1
}

/// Files (not specs when `skip_specs`, not the FFI allowlist when `skip_ffi`) with their findings.
fn findings(id: &str, skip_specs: bool, skip_ffi: bool) -> BTreeMap<String, Vec<Finding>> {
    let f = scanner(id);
    let mut out = BTreeMap::new();
    for (rel, text) in crystal_files() {
        if (skip_specs && is_spec(rel)) || (skip_ffi && ffi_allowed(rel)) {
            continue;
        }
        let r = f(&Src::new(rel, text));
        if !r.is_empty() {
            out.insert(rel.clone(), r);
        }
    }
    out
}

fn show(m: &BTreeMap<String, Vec<Finding>>) -> BTreeSet<String> {
    m.keys().cloned().collect()
}

fn explain(rule: &str, m: &BTreeMap<String, Vec<Finding>>, baseline: &[&str]) -> BTreeSet<String> {
    for (f, v) in m {
        if !baseline.contains(&f.as_str()) {
            for (n, why) in v {
                eprintln!("{rule}: {f}:{n}: {why}");
            }
        }
    }
    show(m)
}

fn counts(m: &BTreeMap<String, Vec<Finding>>) -> BTreeMap<String, usize> {
    m.iter().map(|(k, v)| (k.clone(), v.len())).collect()
}

fn names(m: &BTreeMap<String, Vec<Finding>>) -> Vec<String> {
    m.iter().map(|(k, v)| format!("{k}: {} finding(s), first line {}: {}", v.len(), v[0].0, v[0].1)).collect()
}

fn no_findings(rule: &str, m: BTreeMap<String, Vec<Finding>>) {
    assert!(m.is_empty(), "{rule}:\n{}", names(&m).join("\n"));
}

// ---------------------------------------------------------------- tools (format, ameba)

fn require_tools() -> bool {
    std::env::var("WOLOS_REQUIRE_CRYSTAL_LINT").is_ok_and(|v| v == "1")
}

fn skip_or_fail(rule: &str, why: &str) {
    assert!(!require_tools(), "{rule}: {why} (WOLOS_REQUIRE_CRYSTAL_LINT=1)");
    eprintln!("{rule}: skipped, {why}");
}

fn crystal_version() -> Option<(u32, u32)> {
    let o = Command::new("crystal").arg("--version").output().ok()?;
    let t = String::from_utf8_lossy(&o.stdout);
    let v = t.split_whitespace().nth(1)?;
    let mut p = v.split('.');
    Some((p.next()?.parse().ok()?, p.next()?.parse().ok()?))
}

fn ameba_runs() -> bool {
    Command::new("ameba")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Tracked-style list: the checkout's `.cr` files minus the fixtures.
fn lint_files() -> Vec<String> {
    crystal_files().iter().map(|(f, _)| f.clone()).collect()
}

/// The rule names `ameba --rules` prints (colour codes stripped).
fn ameba_rule_names() -> Vec<String> {
    let o = Command::new("ameba").arg("--rules").arg("--no-color").current_dir(repo_dir()).output().expect("ameba --rules");
    let t = String::from_utf8_lossy(&o.stdout);
    let mut v = Vec::new();
    for l in t.lines() {
        for w in l.split_whitespace() {
            if w.contains('/') && w.chars().next().is_some_and(char::is_uppercase) && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '/') {
                v.push(w.to_string());
            }
        }
    }
    v.sort();
    v.dedup();
    v
}

/// Rules in .ameba.yml: name -> (enabled, excluded files).
fn ameba_yml() -> BTreeMap<String, (Option<bool>, Vec<String>)> {
    let text = read_repo(".ameba.yml").unwrap_or_default();
    let mut m: BTreeMap<String, (Option<bool>, Vec<String>)> = BTreeMap::new();
    let mut cur: Option<String> = None;
    let mut in_ex = false;
    for l in text.lines() {
        if l.trim().is_empty() || l.trim_start().starts_with('#') {
            continue;
        }
        let indent = l.len() - l.trim_start().len();
        let t = l.trim();
        if indent == 0 {
            in_ex = false;
            cur = t.strip_suffix(':').filter(|n| n.contains('/')).map(str::to_string);
            if let Some(c) = &cur {
                m.insert(c.clone(), (None, vec![]));
            }
        } else if let Some(c) = cur.clone() {
            if let Some(v) = t.strip_prefix("Enabled:") {
                m.get_mut(&c).unwrap().0 = match v.trim() {
                    "true" => Some(true),
                    "false" => Some(false),
                    _ => None,
                };
                in_ex = false;
            } else if t == "Excluded:" {
                in_ex = true;
            } else if in_ex {
                if let Some(f) = t.strip_prefix("- ") {
                    m.get_mut(&c).unwrap().1.push(f.trim().trim_matches('"').to_string());
                }
            }
        }
    }
    m
}

// ================================================================ the rules

#[test]
fn rcr_001_spdx_on_line_one() {
    no_findings("RCR-001", findings("RCR-001", false, false));
}

#[test]
fn rcr_002_header_says_what_the_file_is_for() {
    let m = findings("RCR-002", true, false);
    ratchet("RCR-002", explain("RCR-002", &m, B_PURPOSE), B_PURPOSE);
}

#[test]
fn rcr_003_text_hygiene() {
    no_findings("RCR-003", findings("RCR-003", false, false));
}

#[test]
fn rcr_004_crystal_format_is_clean() {
    let Some((maj, min)) = crystal_version() else {
        return skip_or_fail("RCR-004", "crystal is not installed");
    };
    if (maj, min) < CRYSTAL_FLOOR {
        return skip_or_fail(
            "RCR-004",
            &format!("crystal {maj}.{min} is older than {}.{} (the formatter output differs between releases)", CRYSTAL_FLOOR.0, CRYSTAL_FLOOR.1),
        );
    }
    let o = Command::new("crystal")
        .args(["tool", "format", "--check"])
        .args(lint_files())
        .current_dir(repo_dir())
        .output()
        .expect("crystal tool format");
    assert!(
        o.status.success(),
        "RCR-004: crystal {maj}.{min} tool format --check fails (run `make -C wopkg fmt` with the CI toolchain):\n{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn rcr_005_ameba_is_clean() {
    if !ameba_runs() {
        return skip_or_fail("RCR-005", "ameba is not installed (or does not start)");
    }
    let o = Command::new("ameba")
        .args(["--up-to-version", AMEBA_VERSION, "--no-color"])
        .args(lint_files())
        .current_dir(repo_dir())
        .output()
        .expect("ameba");
    assert!(
        o.status.success(),
        "RCR-005: ameba finds problems (config: .ameba.yml; tools/crystal/lint.sh):\n{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn rcr_006_ameba_config_lists_every_rule_explicitly() {
    let yml = ameba_yml();
    assert!(!yml.is_empty(), "RCR-006: .ameba.yml is missing or lists no rule");
    let undecided: Vec<&String> = yml.iter().filter(|(_, v)| v.0.is_none()).map(|(k, _)| k).collect();
    assert!(undecided.is_empty(), "RCR-006: rules without `Enabled: true|false`: {undecided:?}");
    let text = read_repo(".ameba.yml").unwrap();
    assert!(text.lines().next().is_some_and(|l| l.starts_with("# SPDX-License-Identifier:")), "RCR-006: .ameba.yml line 1 is the SPDX line");
    assert!(text.contains(FIXTURES.trim_end_matches('/')), "RCR-006: .ameba.yml must keep the scanner fixtures out of Globs");
    // the ten rules that guard memory, injection and error handling stay on
    for must in [
        "Lint/Syntax",
        "Lint/UnreachableCode",
        "Lint/ShadowedException",
        "Lint/UselessAssign",
        "Lint/UnusedArgument",
        "Lint/DebugCalls",
        "Lint/DebuggerStatement",
        "Lint/HashDuplicatedKey",
        "Lint/NotNil",
        "Lint/SharedVarInFiber",
    ] {
        assert_eq!(yml.get(must).and_then(|v| v.0), Some(true), "RCR-006: {must} must stay enabled");
    }
    if !ameba_runs() {
        return skip_or_fail("RCR-006", "ameba is not installed: the rule list was not compared with `ameba --rules`");
    }
    let have = ameba_rule_names();
    let missing: Vec<&String> = have.iter().filter(|r| !yml.contains_key(*r)).collect();
    let stale: Vec<&String> = yml.keys().filter(|r| !have.contains(r)).collect();
    assert!(missing.is_empty(), "RCR-006: this ameba has rules .ameba.yml does not decide (add each with Enabled:): {missing:?}");
    assert!(stale.is_empty(), "RCR-006: .ameba.yml names rules this ameba does not have: {stale:?}");
}

#[test]
fn rcr_007_ameba_exclusions_only_shrink() {
    let yml = ameba_yml();
    let mut found = BTreeSet::new();
    for (rule, (_, ex)) in &yml {
        for f in ex {
            found.insert(format!("{rule} {f}"));
        }
    }
    ratchet("RCR-007", found, B_AMEBA_EXCLUDED);
    // an excluded file must exist
    let files: BTreeSet<&str> = crystal_files().iter().map(|(f, _)| f.as_str()).collect();
    for (rule, (_, ex)) in &yml {
        for f in ex {
            assert!(files.contains(f.as_str()), "RCR-007: {rule} excludes {f}, which is not a Crystal file of this repo");
        }
    }
}

#[test]
fn rcr_010_unsafe_constructs_stay_in_the_ffi_modules() {
    let m = findings("RCR-010", false, true);
    ratchet("RCR-010", explain("RCR-010", &m, B_UNSAFE), B_UNSAFE);
}

#[test]
fn rcr_011_c_bindings_stay_in_the_ffi_modules() {
    no_findings("RCR-011", findings("RCR-011", false, true));
    for a in FFI_ALLOW {
        let p = repo_dir().join(a);
        assert!(p.exists(), "RCR-011: FFI_ALLOW lists {a}, which does not exist (drop it)");
    }
}

#[test]
fn rcr_020_commands_go_out_as_argv() {
    let m = findings("RCR-020", false, false);
    ratchet("RCR-020", explain("RCR-020", &m, B_PROCESS_SHELL), B_PROCESS_SHELL);
}

#[test]
fn rcr_021_no_code_built_or_loaded_at_run_time() {
    no_findings("RCR-021", findings("RCR-021", false, false));
}

#[test]
fn rcr_022_network_clients_set_timeouts() {
    let m = findings("RCR-022", true, false);
    ratchet("RCR-022", explain("RCR-022", &m, B_HTTP_TIMEOUT), B_HTTP_TIMEOUT);
}

#[test]
fn rcr_023_tls_verification_stays_on() {
    no_findings("RCR-023", findings("RCR-023", false, false));
}

#[test]
fn rcr_024_json_and_yaml_decode_into_types() {
    let m = findings("RCR-024", true, false);
    ratchet_count("RCR-024", counts(&m), B_UNTYPED_PARSE);
}

#[test]
fn rcr_025_nothing_rescues_exception() {
    no_findings("RCR-025", findings("RCR-025", false, false));
}

#[test]
fn rcr_026_rescues_name_their_exceptions() {
    let m = findings("RCR-026", true, false);
    ratchet_count("RCR-026", counts(&m), B_CATCH_ALL);
}

#[test]
fn rcr_030_no_fixed_tmp_paths() {
    let m = findings("RCR-030", true, false);
    ratchet("RCR-030", explain("RCR-030", &m, B_TMP), B_TMP);
}

#[test]
fn rcr_031_nothing_is_made_world_writable() {
    no_findings("RCR-031", findings("RCR-031", false, false));
}

#[test]
fn rcr_032_no_md5_or_sha1() {
    let m = findings("RCR-032", false, false);
    ratchet("RCR-032", explain("RCR-032", &m, B_WEAK_HASH), B_WEAK_HASH);
}

#[test]
fn rcr_033_secrets_use_the_secure_random() {
    let m = findings("RCR-033", false, false);
    ratchet("RCR-033", explain("RCR-033", &m, B_RANDOM), B_RANDOM);
}

#[test]
fn rcr_034_no_literal_credentials() {
    no_findings("RCR-034", findings("RCR-034", false, false));
}

#[test]
fn rcr_035_no_home_directory_literals() {
    let m = findings("RCR-035", true, false);
    ratchet("RCR-035", explain("RCR-035", &m, B_HOME), B_HOME);
}

/// Every shard.yml of the checkout (the repo's own, and any vendored under a `lib/` directory).
fn shard_ymls() -> Vec<(String, String)> {
    let base = repo_dir();
    all_files(&base)
        .into_iter()
        .filter(|f| (f == "shard.yml" || f.ends_with("/shard.yml")) && !is_fixture(f))
        .map(|f| {
            let t = std::fs::read_to_string(base.join(&f)).unwrap_or_default();
            (f, t)
        })
        .collect()
}

#[test]
fn rcr_040_shard_dependencies_are_pinned_by_commit() {
    let all = shard_ymls();
    assert!(all.len() >= 2, "RCR-040: expected the wopkg and hue shard.yml, found {}", all.len());
    let mut bad = Vec::new();
    for (f, t) in &all {
        for (n, why) in scan_shard_yml(t) {
            bad.push(format!("{f}:{n}: {why}"));
        }
    }
    assert!(bad.is_empty(), "RCR-040:\n{}", bad.join("\n"));
}

#[test]
fn rcr_041_shard_lock_is_committed_and_matches() {
    let base = repo_dir();
    let mut bad = Vec::new();
    for (f, t) in shard_ymls() {
        let lock_path = f.replace("shard.yml", "shard.lock");
        let lock = std::fs::read_to_string(base.join(&lock_path)).ok();
        for (n, why) in scan_shard_lock(&t, lock.as_deref()) {
            bad.push(format!("{f}:{n}: {why}"));
        }
    }
    assert!(bad.is_empty(), "RCR-041:\n{}", bad.join("\n"));
}

#[test]
fn rcr_042_shard_census_only_falls() {
    let mut found: BTreeSet<String> = BTreeSet::new();
    let base = repo_dir();
    for (f, t) in shard_ymls() {
        for d in parse_shard_yml(&t).1 {
            found.insert(d.name);
        }
        // a vendored shard has its own shard.yml below a `lib/` directory
        if f.contains("/lib/") || f.starts_with("lib/") {
            let dir = f.trim_end_matches("shard.yml").trim_end_matches('/');
            found.insert(dir.rsplit('/').next().unwrap_or(dir).to_string());
        }
        let lock = std::fs::read_to_string(base.join(f.replace("shard.yml", "shard.lock"))).unwrap_or_default();
        found.extend(parse_shard_lock(&lock).into_keys());
    }
    // a require of a non-stdlib name is a shard too
    let unknown: Vec<&String> = found.iter().filter(|n| !ALLOWED_SHARDS.contains(&n.as_str())).collect();
    assert!(unknown.is_empty(), "RCR-042: shards not in ALLOWED_SHARDS (needs an ADR, THIRD-PARTY.md and the list): {unknown:?}");
    assert!(found.len() <= SHARD_CENSUS, "RCR-042: {} shards, census allows {SHARD_CENSUS}", found.len());
    if found.len() < SHARD_CENSUS {
        eprintln!("RCR-042: {} shards found, census says {SHARD_CENSUS}; lower it", found.len());
    }
}

#[test]
fn rcr_043_requires_are_stdlib_or_in_repo() {
    let m = findings("RCR-043", false, false);
    // a required shard has to be in the census (RCR-042); here only the undeclared ones fail
    no_findings("RCR-043", m);
}

#[test]
fn rcr_044_build_files_do_not_fetch_shards() {
    let base = siblings::root();
    let mut bad = Vec::new();
    for f in all_files(&base) {
        let name = f.rsplit('/').next().unwrap_or(&f);
        let build_file = name == "PKGBUILD" || name == "Makefile" || name.ends_with(".sh") || name.ends_with(".yml") || name.ends_with(".yaml") || name.ends_with(".install");
        if !build_file || is_fixture(&f) || f.contains("/golden") {
            continue;
        }
        let Ok(t) = std::fs::read_to_string(base.join(&f)) else {
            continue;
        };
        for (n, why) in scan_shards_network(&t) {
            bad.push(format!("{f}:{n}: {why}"));
        }
    }
    assert!(bad.is_empty(), "RCR-044:\n{}", bad.join("\n"));
}

// ---------------------------------------------------------------- meta

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/crystal")
}

/// The rule id on a fixture's `# trips: RCR-nnn` line (first three lines), if any.
fn trips(text: &str) -> Option<String> {
    text.lines().take(3).find_map(|l| {
        l.trim_start_matches(['#', ' ', '-', ';'])
            .strip_prefix("trips: ")
            .map(|r| r.trim().to_string())
    })
}

#[test]
fn rcr_090_fixtures_trip_exactly_their_rule() {
    let dir = fixture_dir();
    let mut seen_rules = BTreeSet::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("tests/fixtures/crystal")
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    assert!(entries.iter().any(|p| p.file_name().is_some_and(|n| n == "good.cr")), "RCR-090: fixtures/crystal/good.cr is missing");
    for p in entries {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        if name.ends_with(".lock") {
            continue; // read with its .yml
        }
        let text = std::fs::read_to_string(&p).unwrap();
        let want: BTreeSet<String> = if name.starts_with("good") { BTreeSet::new() } else { trips(&text).into_iter().collect() };
        assert!(name.starts_with("good") || !want.is_empty(), "RCR-090: {name} needs a `# trips: RCR-nnn` line");
        let rel = format!("{FIXTURES}{name}");
        let mut got: BTreeSet<String> = BTreeSet::new();
        if name.ends_with(".cr") {
            let s = Src::new(&rel, &text);
            for (id, f) in FILE_SCANNERS {
                // bad-format.cr and bad-ameba.cr are checked by the tools below, not by a scanner
                if !f(&s).is_empty() {
                    got.insert((*id).to_string());
                }
            }
            if name.starts_with("bad-format") || name.starts_with("bad-ameba") {
                assert!(got.is_empty(), "RCR-090: {name} must look clean to the scanners, trips {got:?}");
                continue;
            }
        } else if name.ends_with(".yml") {
            let stem = name.trim_end_matches(".yml");
            if name.contains("lock-missing") {
                if !scan_shard_lock(&text, None).is_empty() {
                    got.insert("RCR-041".into());
                }
            } else if name.contains("lock") {
                let lock = std::fs::read_to_string(dir.join(format!("{stem}.lock"))).ok();
                if !scan_shard_lock(&text, lock.as_deref()).is_empty() {
                    got.insert("RCR-041".into());
                }
            }
            if !scan_shard_yml(&text).is_empty() {
                got.insert("RCR-040".into());
            }
        } else if name.ends_with(".build") {
            if !scan_shards_network(&text).is_empty() {
                got.insert("RCR-044".into());
            }
        } else {
            panic!("RCR-090: do not know how to check fixture {name}");
        }
        assert_eq!(got, want, "RCR-090: fixture {name} trips {got:?}, expected {want:?}");
        seen_rules.extend(want);
    }
    // every scanner has a bad fixture
    let mut need: BTreeSet<String> = FILE_SCANNERS.iter().map(|(i, _)| (*i).to_string()).collect();
    need.extend(["RCR-040".into(), "RCR-041".into(), "RCR-044".into()]);
    let missing: Vec<&String> = need.iter().filter(|r| !seen_rules.contains(*r)).collect();
    assert!(missing.is_empty(), "RCR-090: no bad fixture for {missing:?}");

    // the tool rules prove their fixtures when the tools exist
    if let Some((maj, min)) = crystal_version().filter(|v| *v >= CRYSTAL_FLOOR) {
        let ok = |f: &str| Command::new("crystal").args(["tool", "format", "--check"]).arg(dir.join(f)).output().is_ok_and(|o| o.status.success());
        assert!(ok("good.cr"), "RCR-090: good.cr is not crystal-format clean on {maj}.{min}");
        assert!(!ok("bad-format.cr"), "RCR-090: bad-format.cr passes the formatter");
    } else {
        eprintln!("RCR-090: formatter fixtures skipped, no crystal >= {}.{}", CRYSTAL_FLOOR.0, CRYSTAL_FLOOR.1);
    }
    if ameba_runs() {
        let run = |f: &str| {
            Command::new("ameba")
                .args(["--up-to-version", AMEBA_VERSION, "--no-color"])
                .arg(dir.join(f))
                .current_dir(repo_dir())
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        assert!(run("good.cr"), "RCR-090: good.cr is not ameba clean");
        assert!(!run("bad-ameba.cr"), "RCR-090: bad-ameba.cr passes ameba (is the config live?)");
    } else {
        eprintln!("RCR-090: ameba fixtures skipped, ameba is not installed");
    }
}

#[test]
fn rcr_091_doc_and_tests_agree() {
    let doc = read_repo("docs/RULES-crystal.md").unwrap_or_default();
    assert!(!doc.is_empty(), "docs/RULES-crystal.md");
    let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/rules_crystal.rs")).unwrap();
    let mut tests: BTreeMap<String, String> = BTreeMap::new();
    for l in src.lines() {
        if let Some(r) = l.strip_prefix("fn rcr_") {
            let name = r.split('(').next().unwrap();
            tests.insert(format!("RCR-{}", &name[..3]), format!("rcr_{name}"));
        }
    }
    let mut documented = BTreeMap::new();
    let mut manual = BTreeSet::new();
    for l in doc.lines().filter(|l| l.starts_with("| RCR-")) {
        let c: Vec<&str> = l.split('|').map(str::trim).collect();
        // | id | area | rule | test | status |
        if c.len() >= 6 {
            if c[5] == "manual" {
                manual.insert(c[1].to_string());
            } else {
                documented.insert(c[1].to_string(), c[4].trim_matches('`').to_string());
            }
        }
    }
    for (id, fnname) in &documented {
        assert_eq!(tests.get(id), Some(fnname), "{id}: the document names {fnname}, the tests have {:?}", tests.get(id));
    }
    for id in tests.keys() {
        assert!(documented.contains_key(id), "{id} has a test but no row in docs/RULES-crystal.md");
    }
    for id in &manual {
        assert!(!tests.contains_key(id), "{id} is marked manual but has a test");
    }
    // each baseline constant is named in the document
    for b in [
        "B_PURPOSE", "B_UNSAFE", "B_PROCESS_SHELL", "B_HTTP_TIMEOUT", "B_UNTYPED_PARSE", "B_CATCH_ALL", "B_TMP",
        "B_WEAK_HASH", "B_RANDOM", "B_HOME", "B_AMEBA_EXCLUDED", "FFI_ALLOW", "ALLOWED_SHARDS", "SHARD_CENSUS",
    ] {
        assert!(doc.contains(b), "docs/RULES-crystal.md does not mention {b}");
    }
}

#[test]
fn rcr_092_baselines_are_sorted_and_unique() {
    for (n, b) in [
        ("B_PURPOSE", B_PURPOSE),
        ("B_UNSAFE", B_UNSAFE),
        ("B_PROCESS_SHELL", B_PROCESS_SHELL),
        ("B_HTTP_TIMEOUT", B_HTTP_TIMEOUT),
        ("B_TMP", B_TMP),
        ("B_WEAK_HASH", B_WEAK_HASH),
        ("B_RANDOM", B_RANDOM),
        ("B_HOME", B_HOME),
        ("B_AMEBA_EXCLUDED", B_AMEBA_EXCLUDED),
        ("FFI_ALLOW", FFI_ALLOW),
        ("ALLOWED_SHARDS", ALLOWED_SHARDS),
        ("STDLIB", STDLIB),
    ] {
        let mut s = b.to_vec();
        s.sort();
        s.dedup();
        assert_eq!(s, b, "{n} must be sorted and free of duplicates");
    }
    for (n, b) in [("B_UNTYPED_PARSE", B_UNTYPED_PARSE), ("B_CATCH_ALL", B_CATCH_ALL)] {
        let keys: Vec<&str> = b.iter().map(|(k, _)| *k).collect();
        let mut s = keys.clone();
        s.sort();
        s.dedup();
        assert_eq!(s, keys, "{n} must be sorted and free of duplicates");
    }
}

#[test]
fn rcr_093_every_crystal_file_is_found() {
    let n = crystal_files().len();
    assert!(n >= 39, "RCR-093: only {n} Crystal files found; the walk lost a tree");
    let dirs: BTreeSet<&str> = crystal_files().iter().filter_map(|(f, _)| f.split('/').next()).collect();
    for must in ["wopkg", "hue", "crates"] {
        assert!(dirs.contains(must), "RCR-093: no Crystal file under {must}/");
    }
    assert!(fixture_dir().join("good.cr").exists(), "RCR-093: tests/fixtures/crystal/good.cr");
}
