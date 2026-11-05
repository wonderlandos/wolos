// SPDX-License-Identifier: BSD-2-Clause
//! Reads the English source out of Rust copy modules (ADR 0045, migration step 1): every
//! `const NAME: &str = "…";` outside `#[cfg(test)]` becomes the row `<module>.<name>` of
//! en.tsv, its `///` lines the translator's comment. A doc line `budget: N` sets the row's
//! budget and is not part of the comment. Functions that build text (`format!`) are not
//! extracted: they move to `{name}` templates by hand (ADR 0045, step 3).
//!
//! Text written where it is used is read from the calls that translate it ([`calls`]):
//! `tr("…")`, `tr_fill("…", …)`, `trn("…one…", "…other…", n)` and the const marker `n_("…")`
//! (L10N-2). Their key is made from
//! the English (`setup.choose_a_password`); the program finds it again by its English at run time.
use super::tsv::SourceRow;

/// A Rust string literal starting at `s` (`"…"`, with escapes and `\` line continuations, or
/// `r"…"` / `r#"…"#`): (its value, bytes consumed). Err: not a literal or not closed.
pub fn rust_string(s: &str) -> Result<(String, usize), String> {
    if let Some(raw) = s.strip_prefix('r') {
        let hashes = raw.bytes().take_while(|b| *b == b'#').count();
        let open = 1 + hashes;
        if s.as_bytes().get(open) != Some(&b'"') {
            return Err("not a string literal".into());
        }
        let close = format!("\"{}", "#".repeat(hashes));
        let body = &s[open + 1..];
        let end = body.find(&close).ok_or("raw string not closed")?;
        return Ok((body[..end].to_string(), open + 1 + end + close.len()));
    }
    if !s.starts_with('"') {
        return Err("not a string literal".into());
    }
    let mut out = String::new();
    let mut it = s.char_indices().skip(1).peekable();
    while let Some((i, c)) = it.next() {
        match c {
            '"' => return Ok((out, i + 1)),
            '\\' => {
                let Some((_, e)) = it.next() else {
                    return Err("string not closed".into());
                };
                match e {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    '0' => out.push('\0'),
                    '\\' | '"' | '\'' => out.push(e),
                    '\n' => {
                        while it.peek().is_some_and(|(_, c)| c.is_whitespace()) {
                            it.next();
                        }
                    }
                    'u' => {
                        let mut hex = String::new();
                        for (_, h) in it.by_ref() {
                            if h == '}' {
                                break;
                            }
                            if h != '{' {
                                hex.push(h);
                            }
                        }
                        let ch = u32::from_str_radix(&hex, 16)
                            .ok()
                            .and_then(char::from_u32)
                            .ok_or("bad \\u escape")?;
                        out.push(ch);
                    }
                    other => return Err(format!("escape \\{other} is not supported here")),
                }
            }
            other => out.push(other),
        }
    }
    Err("string not closed".into())
}

/// The doc lines collected above a const: (comment, budget).
fn comment_of(doc: &[String]) -> (String, Option<u32>) {
    let mut budget = None;
    let mut words = Vec::new();
    for d in doc {
        if let Some(b) = d.strip_prefix("budget:") {
            budget = b.trim().parse().ok();
        } else if !d.is_empty() {
            words.push(d.as_str());
        }
    }
    (words.join(" "), budget)
}

/// The rows of `src` (a copy module) under `module` (`settings` -> `settings.sound_louder`).
/// Err: a string const whose value is not one literal (it cannot be translated as written).
pub fn consts(module: &str, src: &str) -> Result<Vec<SourceRow>, String> {
    let mut rows = Vec::new();
    let mut doc: Vec<String> = Vec::new();
    let mut at = 0;
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    let mut n = 0;
    while n < lines.len() {
        let line = lines[n];
        let start = at;
        at += line.len();
        n += 1;
        let t = line.trim();
        if t.starts_with("#[cfg(test)]") {
            break;
        }
        if let Some(d) = t.strip_prefix("///") {
            doc.push(d.trim().to_string());
            continue;
        }
        if t.starts_with("#[") {
            continue;
        }
        let decl = t
            .strip_prefix("pub ")
            .or_else(|| t.strip_prefix("pub(crate) "))
            .or_else(|| t.strip_prefix("pub(super) "))
            .unwrap_or(t);
        let Some((name, _)) = decl
            .strip_prefix("const ")
            .and_then(|d| d.split_once(": &str ="))
        else {
            doc.clear();
            continue;
        };
        let name = name.trim();
        // The literal starts after `=`, on this line or the next.
        let eq = start + line.find(": &str =").unwrap_or(0) + ": &str =".len();
        let tail = src[eq..].trim_start();
        let lit_at = src.len() - tail.len();
        let (value, used) = rust_string(tail).map_err(|e| format!("{module}::{name}: {e}"))?;
        if !src[lit_at + used..].trim_start().starts_with(';') {
            return Err(format!(
                "{module}::{name}: the value is not one string literal"
            ));
        }
        let end = lit_at + used;
        while at < end && n < lines.len() {
            at += lines[n].len();
            n += 1;
        }
        let (comment, budget) = comment_of(&doc);
        doc.clear();
        rows.push(SourceRow {
            key: format!("{module}.{}", name.to_ascii_lowercase()),
            source: value,
            comment,
            budget,
        });
    }
    Ok(rows)
}

/// The translating calls [`calls`] reads, and how many literals each takes (`trn`: one, other).
/// `n_("…")` marks a text in a `const` (where no call can run) that is translated where it is
/// shown, with `tr(x)`.
const CALLS: [(&str, usize); 4] = [("tr_fill(", 1), ("trn(", 2), ("tr(", 1), ("n_(", 1)];

/// The key name made from an English text: its first six words, lowercase ASCII joined by `_`
/// (`Choose a password.` -> `choose_a_password`); `text` for a text with no ASCII word.
pub fn slug(source: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut w = String::new();
    for c in source.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_alphanumeric() {
            w.push(c.to_ascii_lowercase());
        } else if c == '{' || c == '}' {
            continue;
        } else if !w.is_empty() {
            words.push(std::mem::take(&mut w));
        }
        if words.len() == 6 {
            break;
        }
    }
    let s = words.join("_");
    if s.is_empty() { "text".into() } else { s }
}

/// The rows of the translating calls in `src` (a surface or a copy module) under `module`: one
/// row per distinct English, keyed by [`slug`] (`_2`, `_3` for two texts with one slug); a `trn`
/// call makes the `#one` and `#other` rows of one key. A `// l10n: …` line right above a call is
/// its translator's comment. Lines after `#[cfg(test)]` are not read. A call on a name
/// (`tr(copy::X)`) is skipped: that text has its own row. Err: a `trn` with one text.
pub fn calls(module: &str, src: &str) -> Result<Vec<SourceRow>, String> {
    let src = src.split("#[cfg(test)]").next().unwrap_or(src);
    let mut rows: Vec<SourceRow> = Vec::new();
    let mut at = 0;
    while at < src.len() {
        let rest = &src[at..];
        let Some((off, call, n)) = CALLS
            .iter()
            .filter_map(|(c, n)| {
                let mut from = 0;
                while let Some(i) = rest[from..].find(c) {
                    let i = from + i;
                    let before = rest[..i].chars().next_back();
                    if !before.is_some_and(|b| b.is_alphanumeric() || b == '_') {
                        return Some((i, *c, *n));
                    }
                    from = i + c.len();
                }
                None
            })
            .min_by_key(|x| x.0)
        else {
            break;
        };
        let start = at + off;
        let line_start = src[..start].rfind('\n').map_or(0, |i| i + 1);
        let line = &src[line_start..start];
        // A call inside a comment or a doc line is an example, not text.
        if line.trim_start().starts_with("//") {
            at = start + call.len();
            continue;
        }
        let comment = src[..line_start]
            .lines()
            .next_back()
            .and_then(|l| l.trim().strip_prefix("// l10n:"))
            .map(|c| c.trim().to_string())
            .unwrap_or_default();
        let mut pos = start + call.len();
        let mut lits = Vec::new();
        let mut literal = true;
        for k in 0..n {
            let tail = src[pos..].trim_start();
            pos = src.len() - tail.len();
            let Ok((v, used)) = rust_string(tail) else {
                // `tr(copy::X)`, `tr(row.1)`: a text the unit has by its own row, found by its
                // English at run time. Not a row here.
                literal = false;
                break;
            };
            lits.push(v);
            pos += used;
            if k + 1 < n {
                let tail = src[pos..].trim_start();
                let Some(t) = tail.strip_prefix(',') else {
                    return Err(format!(
                        "{module}: {call}…) at byte {start}: expected two texts"
                    ));
                };
                pos = src.len() - t.len();
            }
        }
        if !literal {
            at = pos.max(start + call.len());
            continue;
        }
        at = pos;
        let other = lits.last().cloned().unwrap_or_default();
        if rows
            .iter()
            .any(|r| r.source == other && !r.key.ends_with("#one"))
        {
            continue;
        }
        let base = format!("{module}.{}", slug(&other));
        let mut key = base.clone();
        let mut i = 2;
        while rows
            .iter()
            .any(|r| r.key.split('#').next() == Some(key.as_str()))
        {
            key = format!("{base}_{i}");
            i += 1;
        }
        let row = |key: String, source: String| SourceRow {
            key,
            source,
            comment: comment.clone(),
            budget: None,
        };
        if n == 2 {
            rows.push(row(format!("{key}#one"), lits[0].clone()));
            rows.push(row(format!("{key}#other"), other));
        } else {
            rows.push(row(key, other));
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_literals_read_as_their_value() {
        assert_eq!(
            rust_string(r#""a \"b\" c" ;"#),
            Ok(("a \"b\" c".into(), 11))
        );
        assert_eq!(
            rust_string("\"one \\\n     two\"").map(|v| v.0),
            Ok("one two".into())
        );
        assert_eq!(
            rust_string(r#""\u{2019}\n""#).map(|v| v.0),
            Ok("’\n".into())
        );
        assert_eq!(
            rust_string(r##"r#"raw "q""#"##).map(|v| v.0),
            Ok("raw \"q\"".into())
        );
        assert!(rust_string("\"open").is_err());
        assert!(rust_string("x").is_err());
    }

    #[test]
    fn copy_module_consts_become_rows() {
        let src = r#"//! Module.
/// The page title.
/// budget: 28
pub const PAGE_TITLE: &str = "Language & Region";
/// Long one.
pub const LONG: &str =
    "First line \
     continues.";
const PRIVATE: &str = "Kept too.";
pub const LIST: [&str; 1] = ["not extracted"];
pub fn not_a_const() -> &'static str { "no" }
#[cfg(test)]
mod tests { const T: &str = "test only"; }
"#;
        let rows = consts("lang", src).expect("module reads");
        let got: Vec<(&str, &str, &str, Option<u32>)> = rows
            .iter()
            .map(|r| {
                (
                    r.key.as_str(),
                    r.source.as_str(),
                    r.comment.as_str(),
                    r.budget,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (
                    "lang.page_title",
                    "Language & Region",
                    "The page title.",
                    Some(28)
                ),
                ("lang.long", "First line continues.", "Long one.", None),
                ("lang.private", "Kept too.", "", None),
            ]
        );
    }

    #[test]
    fn translating_calls_become_rows_keyed_by_their_english() {
        let src = r#"fn page() {
    let a = tr("Choose a password.");
    // l10n: {ssid} is a network name.
    let b = tr_fill("Enter the password for “{ssid}”.", &[("ssid", s)]);
    let c = i18n::trn("About {n} minute remaining", "About {n} minutes remaining", m);
    let d = tr("Choose a password.");
    let e = tr("Choose a password!");
    const C: [&str; 1] = [n_("Swap file")];
    // tr("an example in a comment")
    let f = attr("not a call");
}
#[cfg(test)]
mod tests { fn t() { tr("test only"); } }
"#;
        let rows = calls("setup", src).expect("calls read");
        let got: Vec<(&str, &str, &str)> = rows
            .iter()
            .map(|r| (r.key.as_str(), r.source.as_str(), r.comment.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("setup.choose_a_password", "Choose a password.", ""),
                (
                    "setup.enter_the_password_for_ssid",
                    "Enter the password for “{ssid}”.",
                    "{ssid} is a network name."
                ),
                (
                    "setup.about_n_minutes_remaining#one",
                    "About {n} minute remaining",
                    ""
                ),
                (
                    "setup.about_n_minutes_remaining#other",
                    "About {n} minutes remaining",
                    ""
                ),
                ("setup.choose_a_password_2", "Choose a password!", ""),
                ("setup.swap_file", "Swap file", ""),
            ]
        );
        assert_eq!(calls("m", "let x = tr(name);"), Ok(Vec::new()));
        assert_eq!(slug("…"), "text");
    }

    #[test]
    fn a_const_built_from_parts_is_refused() {
        let e = consts("m", "pub const X: &str = concat!(\"a\", \"b\");\n").unwrap_err();
        assert!(e.contains("m::X"), "{e}");
        let e = consts(
            "m",
            "pub const Y: &str = \"a\" ;\npub const Z: &str = \"b\" + 1;\n",
        );
        assert!(e.is_err());
    }
}
