// SPDX-License-Identifier: BSD-2-Clause
//! The translator's format: UTF-8, LF, one row per string, fields separated by a tab. A line
//! starting with `#` is a comment, a blank line is skipped. Inside a field `\t`, `\n` and `\\`
//! stand for a tab, a line break and a backslash; nothing else is escaped.
//!
//! - `en.tsv` (the source): `key  source  comment  budget`. `budget` is the most columns the
//!   text may take in any language (a wide East Asian character counts two), empty for none.
//! - `<lang>.tsv`: `key  source  text  hash`. `source` is the English the row was last synced
//!   with (shown to the translator, rewritten by `wol-i18n sync`), `text` the translation (empty:
//!   not translated yet), `hash` the source hash the translation was made for (empty: made for
//!   the current source; sync fills it in).
//!
//! A plural key has one row per form, `name#one`, `name#other` (CLDR categories, plural.rs).
use super::Lang;

/// A row of `en.tsv`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceRow {
    /// `unit.name` or `unit.name#category`.
    pub key: String,
    /// The English text.
    pub source: String,
    /// What the translator should know: where it shows, what a placeholder holds.
    pub comment: String,
    /// Most columns the text may take, if bounded.
    pub budget: Option<u32>,
}

/// A row of `<lang>.tsv`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LangRow {
    /// Same key as in `en.tsv`.
    pub key: String,
    /// The English at the last sync.
    pub source: String,
    /// The translation; empty: not translated.
    pub text: String,
    /// [`super::source_hash`] of the English it translates; empty: the current one.
    pub hash: String,
}

/// A line that could not be read: (line number from 1, why).
pub type LineError = (usize, String);

/// The header line of `en.tsv`.
pub const SOURCE_HEADER: &str = "# key\tsource\tcomment\tbudget";
/// The header line of `<lang>.tsv`.
pub const LANG_HEADER: &str = "# key\tsource\ttext\thash";

/// `s` with tabs, line breaks and backslashes escaped.
pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\t' => o.push_str("\\t"),
            '\n' => o.push_str("\\n"),
            c => o.push(c),
        }
    }
    o
}

/// The field `s` stands for. Err: a backslash before anything but `t`, `n` or `\`.
pub fn unescape(s: &str) -> Result<String, String> {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match it.next() {
            Some('t') => o.push('\t'),
            Some('n') => o.push('\n'),
            Some('\\') => o.push('\\'),
            Some(x) => {
                return Err(format!(
                    "“\\{x}” is not an escape: write \\\\ for a backslash"
                ));
            }
            None => return Err("the field ends in a lone backslash".into()),
        }
    }
    Ok(o)
}

/// The unescaped fields of every row: (line number, up to four fields), and every bad line.
fn rows(text: &str) -> (Vec<(usize, [String; 4])>, Vec<LineError>) {
    let (mut out, mut bad) = (Vec::new(), Vec::new());
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 || parts.len() > 4 {
            bad.push((n, format!("{} fields, expected 2 to 4", parts.len())));
            continue;
        }
        let mut f: [String; 4] = Default::default();
        for (slot, p) in f.iter_mut().zip(&parts) {
            match unescape(p) {
                Ok(v) => *slot = v,
                Err(e) => bad.push((n, e)),
            }
        }
        out.push((n, f));
    }
    (out, bad)
}

/// Reads `en.tsv`. Err: every line that is not a row, with why.
pub fn parse_source(text: &str) -> Result<Vec<SourceRow>, Vec<LineError>> {
    let (all, mut bad) = rows(text);
    let mut out = Vec::new();
    for (n, [key, source, comment, budget]) in all {
        let budget = if budget.is_empty() {
            None
        } else if let Ok(b) = budget.parse::<u32>() {
            Some(b)
        } else {
            bad.push((n, format!("budget “{budget}” is not a number")));
            None
        };
        out.push(SourceRow {
            key,
            source,
            comment,
            budget,
        });
    }
    if bad.is_empty() { Ok(out) } else { Err(bad) }
}

/// Reads `<lang>.tsv`. Err: every line that is not a row, with why.
pub fn parse_lang(text: &str) -> Result<Vec<LangRow>, Vec<LineError>> {
    let (all, bad) = rows(text);
    if !bad.is_empty() {
        return Err(bad);
    }
    Ok(all
        .into_iter()
        .map(|(_, [key, source, text, hash])| LangRow {
            key,
            source,
            text,
            hash,
        })
        .collect())
}

/// `en.tsv` for `rows`, with a header naming `unit` and where the rows came from.
pub fn write_source(unit: &str, origin: &str, rows: &[SourceRow]) -> String {
    let mut s = format!(
        "# WolOS text: {unit}, English source. {origin}\n# Translators: docs/l10n/README.md. Columns:\n{SOURCE_HEADER}\n"
    );
    for r in rows {
        let budget = r.budget.map(|b| b.to_string()).unwrap_or_default();
        s.push_str(&format!(
            "{}\t{}\t{}\t{budget}\n",
            escape(&r.key),
            escape(&r.source),
            escape(&r.comment)
        ));
    }
    s
}

/// `<lang>.tsv` for `rows`.
pub fn write_lang(unit: &str, lang: Lang, rows: &[LangRow]) -> String {
    let mut s = format!(
        "# WolOS text: {unit}, {} ({}). Write the translation in the third column; leave the hash alone.\n\
         # Translators: docs/l10n/README.md and docs/l10n/RULES-{}.md. Columns:\n{LANG_HEADER}\n",
        lang.endonym(),
        lang.tag(),
        lang.tag()
    );
    for r in rows {
        s.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            escape(&r.key),
            escape(&r.source),
            escape(&r.text),
            escape(&r.hash)
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_round_trip_through_a_field() {
        for s in [
            "plain",
            "tab\there",
            "line\nbreak",
            "back\\slash",
            "\\n literally",
        ] {
            assert_eq!(unescape(&escape(s)).as_deref(), Ok(s));
        }
        assert!(unescape("bad \\q").is_err());
        assert!(unescape("ends \\").is_err());
    }

    #[test]
    fn source_rows_parse_and_write_back_the_same() {
        let rows = vec![
            SourceRow {
                key: "a.title".into(),
                source: "Language & Region".into(),
                comment: "Page title.".into(),
                budget: Some(32),
            },
            SourceRow {
                key: "a.n#one".into(),
                source: "{n} file".into(),
                ..Default::default()
            },
        ];
        let text = write_source("a", "Test.", &rows);
        assert_eq!(parse_source(&text), Ok(rows));
    }

    #[test]
    fn bad_lines_are_reported_with_their_number() {
        let e = parse_source("# header\nonly-key\na\tb\tc\tlots\n").unwrap_err();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].0, 2);
        assert_eq!(e[1].0, 3);
        let lang = parse_lang("a.x\tSource\tTexto\t\n").expect("lang row parses");
        assert_eq!(lang[0].text, "Texto");
        assert!(lang[0].hash.is_empty());
    }
}
