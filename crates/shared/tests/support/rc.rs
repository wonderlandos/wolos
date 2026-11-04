// SPDX-License-Identifier: BSD-2-Clause
//! The reader shared by the root-cause rules (`crates/shared/tests/rules_rc_*.rs`, docs/RULES.md
//! ids RH-nnn, ADR 0051): repo walk, the one lexer (`support/lexer.rs`), `#[cfg(test)]` blanking,
//! bracket matching, the growth-only ratchet and the check that docs/RULES.md and a test file name
//! the same ids. Include it with `#[path = "support/rc.rs"] mod rc;`.
#![allow(dead_code)] // each rules_rc_*.rs file uses only the helpers its rules need

#[path = "lexer.rs"]
mod lexer;
pub use lexer::is_ident;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The repository root (two levels above `crates/shared`).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// A repo-relative text file; panics when it is missing (a rule must not pass on nothing).
pub fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// Repo-relative paths of every file under `dir` whose extension is one of `exts` (sorted).
/// Build output, dependencies and fixtures are skipped.
pub fn walk(dir: &str, exts: &[&str]) -> Vec<String> {
    fn go(p: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(p) else {
            return;
        };
        for e in rd.flatten() {
            let q = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if q.is_dir() {
                let skip = name == "target"
                    || name.starts_with("target-")
                    || matches!(name.as_str(), "node_modules" | ".git" | "fixtures");
                if !skip {
                    go(&q, exts, out);
                }
            } else if q.extension().is_some_and(|x| exts.iter().any(|w| x == *w)) {
                out.push(q);
            }
        }
    }
    let base = root();
    let mut v = Vec::new();
    go(&base.join(dir), exts, &mut v);
    let mut out: Vec<String> = v
        .iter()
        .filter_map(|p| {
            p.strip_prefix(&base)
                .ok()
                .map(|r| r.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    out.sort();
    out
}

/// True for a Rust file that ships: under some `src/`, not a test, bench, example or build script.
pub fn shipped(rel: &str) -> bool {
    let file = rel.rsplit('/').next().unwrap_or("");
    rel.contains("/src/")
        && !rel.contains("/tests/")
        && !rel.contains("/benches/")
        && !rel.contains("/examples/")
        && file != "tests.rs"
        && file != "build.rs"
}

/// One Rust source: its text, its code with comments, strings and `#[cfg(test)]` items blanked
/// (offsets and newlines kept) and its shipped string literals (offset, text with quotes).
pub struct Src {
    pub rel: String,
    pub text: String,
    pub code: Vec<u8>,
    pub lits: Vec<(usize, String)>,
    starts: Vec<usize>,
}

impl Src {
    /// Reads and lexes the repo-relative `rel`.
    pub fn load(rel: &str) -> Src {
        Src::parse(rel, read(rel))
    }

    /// Lexes `text` as if it were the file `rel` (the self-tests feed snippets this way).
    pub fn parse(rel: &str, text: String) -> Src {
        let (mut code, spans) = lexer::lex(&text);
        let cut = cfg_test_ranges(&code);
        for &(a, z) in &cut {
            for c in &mut code[a..z] {
                if *c != b'\n' {
                    *c = b' ';
                }
            }
        }
        let lits = spans
            .iter()
            .filter(|(a, _)| !cut.iter().any(|(x, y)| a >= x && a < y))
            .map(|&(a, z)| (a, text[a..z].to_owned()))
            .collect();
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Src {
            rel: rel.to_owned(),
            text,
            code,
            lits,
            starts,
        }
    }

    /// The 1-based line of byte offset `off`.
    pub fn line_of(&self, off: usize) -> usize {
        self.starts.partition_point(|&x| x <= off)
    }

    /// The text of 1-based line `n` (empty past the end).
    pub fn line(&self, n: usize) -> &str {
        let Some(&a) = self.starts.get(n.saturating_sub(1)) else {
            return "";
        };
        let z = self.starts.get(n).copied().unwrap_or(self.text.len());
        self.text[a..z].trim_end_matches(['\n', '\r'])
    }

    /// `rel:line` of byte offset `off`, for a failure message.
    pub fn at(&self, off: usize) -> String {
        format!("{}:{}", self.rel, self.line_of(off))
    }

    /// The blanked code between `a` and `z` as text.
    pub fn code_str(&self, a: usize, z: usize) -> String {
        String::from_utf8_lossy(&self.code[a..z.min(self.code.len())]).into_owned()
    }
}

/// Every shipped Rust source under the repo-relative directories `dirs`.
pub fn sources(dirs: &[&str]) -> Vec<Src> {
    dirs.iter()
        .flat_map(|d| walk(d, &["rs"]))
        .filter(|r| shipped(r))
        .map(|r| Src::load(&r))
        .collect()
}

/// Byte ranges of the items behind each `#[cfg(test)]` (attribute included).
fn cfg_test_ranges(code: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for a in find_all(code, "#[cfg(test)]") {
        let mut i = a + "#[cfg(test)]".len();
        let mut end = code.len();
        while i < code.len() {
            match code[i] {
                b'{' => {
                    end = close_of(code, i) + 1;
                    break;
                }
                b';' => {
                    end = i + 1;
                    break;
                }
                _ => i += 1,
            }
        }
        out.push((a, end.min(code.len())));
    }
    out
}

/// Offsets where `needle` starts in `h`.
pub fn find_all<'a>(h: &'a [u8], needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    let n = needle.as_bytes();
    (0..(h.len() + 1).saturating_sub(n.len())).filter(move |&i| h[i..].starts_with(n))
}

/// Offsets where `needle` starts in `h` with no identifier byte before it.
pub fn find_words<'a>(h: &'a [u8], needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    find_all(h, needle).filter(move |&i| i == 0 || !is_ident(h[i - 1]))
}

/// The offset of the bracket closing the one at `open` (`(`, `[` or `{`); the end of `code`
/// when it never closes. Strings and comments must already be blanked.
pub fn close_of(code: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    for (i, c) in code.iter().enumerate().skip(open) {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    code.len()
}

/// The code after offset `i` with leading whitespace skipped.
pub fn after(code: &[u8], i: usize) -> &[u8] {
    let rest = code.get(i..).unwrap_or(&[]);
    let skip = rest.iter().take_while(|c| c.is_ascii_whitespace()).count();
    &rest[skip..]
}

/// A frozen debt row: (rule id, key, count, owner). The key is a crate, a file or a fact; the
/// owner is the brief of the 0.13 fix plan that brings the count down (ADR 0051).
pub type Row = (&'static str, &'static str, usize, &'static str);

/// Hits per key, each hit a `path:line` with a word on what it is.
pub type Hits = BTreeMap<String, Vec<String>>;

/// Adds one hit under `key`.
pub fn hit(h: &mut Hits, key: &str, what: String) {
    h.entry(key.to_owned()).or_default().push(what);
}

/// The growth-only ratchet: every key's count may fall below its row, never rise above it; a
/// key with no row starts at zero. Prints `BASELINE-ROW` lines (run with `--nocapture`).
pub fn assert_ratchet(id: &str, rule: &str, baseline: &[Row], hits: &Hits) {
    let mut over = Vec::new();
    for (k, v) in hits {
        let row = baseline.iter().find(|r| r.0 == id && r.1 == k);
        let base = row.map_or(0, |r| r.2);
        let owner = row.map_or("?", |r| r.3);
        println!(
            "BASELINE-ROW    (\"{id}\", \"{k}\", {}, \"{owner}\"),",
            v.len()
        );
        if v.len() > base {
            over.push(format!(
                "{id} {k}: {} hits, baseline {base}:\n  {}",
                v.len(),
                v.iter().take(40).cloned().collect::<Vec<_>>().join("\n  ")
            ));
        } else if v.len() < base {
            println!(
                "{id} {k} is down to {} (baseline {base}): lower the row",
                v.len()
            );
        }
    }
    for r in baseline
        .iter()
        .filter(|r| r.0 == id && !hits.contains_key(r.1))
    {
        println!(
            "{} {} is down to 0 (baseline {}): drop the row",
            r.0, r.1, r.2
        );
    }
    assert!(
        over.is_empty(),
        "{id} {rule}\nA root-cause rule of docs/RULES.md grew (fix the new hit; never raise a row):\n{}",
        over.join("\n")
    );
}

/// A brief id of the 0.13 fix plan (`STEP-0`, `A2`, `D2a`, ...), or `STD-7`, the sweep that
/// annotates the debt the origin rules (STANDARDS "Origins of the 0.13 bugs") found.
pub fn is_brief(b: &str) -> bool {
    let mut c = b.chars();
    b == "STEP-0"
        || b == "STD-7"
        || (c.next().is_some_and(|w| ('A'..='D').contains(&w))
            && b.len() >= 2
            && b[1..]
                .trim_end_matches(['a', 'b'])
                .chars()
                .all(|d| d.is_ascii_digit())
            && b[1..].starts_with(|d: char| d.is_ascii_digit()))
}

/// Rows of docs/RULES.md that start `| RH-nnn`: (id, cells). The catalogue row `| RH- |` is not one.
pub fn rh_rows() -> Vec<(String, Vec<String>)> {
    read("docs/RULES.md")
        .lines()
        .filter(|l| {
            l.strip_prefix("| RH-")
                .is_some_and(|r| r.len() > 3 && r.as_bytes()[..3].iter().all(u8::is_ascii_digit))
        })
        .map(|l| {
            let cells: Vec<String> = l.split('|').map(|c| c.trim().to_owned()).collect();
            (cells[1].clone(), cells)
        })
        .collect()
}

/// docs/RULES.md and the test file `me` (its name, its text) name the same RH ids: every id in
/// `ids` has exactly one row whose "How checked" cell names `me`, every row naming `me` is in
/// `ids`, every baseline row is for one of `ids` and names an owner, and `me` quotes each id
/// at least twice (its table and its test).
pub fn assert_doc_agrees(me: &str, text: &str, ids: &[&str], baseline: &[Row]) {
    let rows = rh_rows();
    for id in ids {
        let mine: Vec<_> = rows.iter().filter(|(r, _)| r == id).collect();
        assert_eq!(
            mine.len(),
            1,
            "{id}: docs/RULES.md must list it exactly once"
        );
        let how = &mine[0].1[mine[0].1.len() - 2];
        assert!(
            how.contains(me),
            "{id}: docs/RULES.md says it is checked by `{how}`, not {me}"
        );
        assert!(
            text.matches(&format!("\"{id}\"")).count() >= 2,
            "{id}: {me} has no test that checks it"
        );
    }
    for (r, cells) in &rows {
        if cells[cells.len() - 2].contains(me) {
            assert!(
                ids.contains(&r.as_str()),
                "{r}: docs/RULES.md says {me} checks it, and {me} does not"
            );
        }
    }
    for b in baseline {
        assert!(
            ids.contains(&b.0),
            "BASELINE row {} is not a rule of {me}",
            b.0
        );
        assert!(
            is_brief(b.3),
            "BASELINE row {} {} names no owner brief ({:?})",
            b.0,
            b.1,
            b.3
        );
    }
}

/// Top directories of this repository a document may point into.
pub const TOPS: &[&str] = &[
    "crates/",
    "files/",
    "desktop/",
    "tools/",
    "docs/",
    "webapps/",
    "design/",
    "kernel/",
    "fuzz/",
    "hue/",
    "plugins/",
    "wopkg/",
    "wolos-crystal/",
    "wolos-shards/",
    "assets/",
    "third_party/",
];

/// Every `code span` of a markdown text: (1-based line, text).
pub fn spans(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut fence = false;
    for (i, l) in text.lines().enumerate() {
        if l.trim_start().starts_with("```") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        let parts: Vec<&str> = l.split('`').collect();
        for (k, p) in parts.iter().enumerate() {
            if k % 2 == 1 && k + 1 < parts.len() {
                out.push((i + 1, (*p).to_owned()));
            }
        }
    }
    out
}

/// The repo path a span names, if it names one we can check (no pattern, no placeholder).
pub fn repo_path(span: &str) -> Option<String> {
    let s = span.trim();
    if !TOPS.iter().any(|t| s.starts_with(t)) {
        return None;
    }
    if s.contains(|c: char| " *{}<>$…|[]?".contains(c)) || s.contains("...") || s.contains("NNNN")
    {
        return None;
    }
    let s = s.split("::").next().unwrap_or(s);
    let s = s.split('#').next().unwrap_or(s);
    let s = match s.rsplit_once(':') {
        Some((a, b))
            if b.chars()
                .all(|c| c.is_ascii_digit() || c == '-' || c == ',') =>
        {
            a
        }
        _ => s,
    };
    Some(
        s.trim_end_matches('/')
            .trim_end_matches(['.', ',', ';', ')'])
            .to_owned(),
    )
}

/// True when the repo path `p` exists, or names a decision record by its number
/// (`docs/adr/0029` for `docs/adr/0029-*.md`).
pub fn resolves(base: &Path, p: &str) -> bool {
    if base.join(p).exists() {
        return true;
    }
    let Some((dir, last)) = p.rsplit_once('/') else {
        return false;
    };
    !last.contains('.')
        && std::fs::read_dir(base.join(dir)).is_ok_and(|rd| {
            rd.flatten().any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{last}-"))
            })
        })
}
