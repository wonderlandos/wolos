// SPDX-License-Identifier: BSD-2-Clause
//! Translation status and upkeep: how much of a unit a language covers ([`coverage`]), and the
//! translator's file kept in step with the English ([`sync`]): every row English has, the current
//! English beside each translation, stale ones still marked by their old hash.
use super::compile::{self, Problem, Unit};
use super::tsv::LangRow;
use super::{Lang, source_hash};

/// One language of one unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// The language.
    pub lang: Lang,
    /// Rows it should have.
    pub rows: usize,
    /// Rows translated.
    pub translated: usize,
    /// Translated rows made for an older English.
    pub stale: usize,
    /// Errors (the build refuses them).
    pub errors: usize,
    /// Warnings.
    pub warnings: usize,
}

impl Coverage {
    /// Translated share in percent, rounded down (100 for a unit with no rows).
    pub fn percent(&self) -> usize {
        (self.translated * 100)
            .checked_div(self.rows)
            .unwrap_or(100)
    }
}

/// The coverage of `l` from its rows, and every problem found. Err: the table could not be
/// written (a unit too large).
pub fn coverage(
    unit: &Unit,
    l: Lang,
    rows: &[LangRow],
) -> Result<(Coverage, Vec<Problem>), super::WtxError> {
    let c = compile::compile(unit, l, rows)?;
    let errors = c.problems.iter().filter(|p| p.error).count();
    Ok((
        Coverage {
            lang: l,
            rows: c.rows,
            translated: c.translated,
            stale: c.stale,
            errors,
            warnings: c.problems.len() - errors,
        },
        c.problems,
    ))
}

/// The rows of `<l>.tsv` after a sync: one per row English wants in `l`, in en.tsv order, with
/// the current English; a translation keeps its text, and its hash (an empty hash becomes the
/// current English's: the translator made it for this text). Rows for keys English dropped are
/// left out; the second value counts them.
pub fn sync(unit: &Unit, l: Lang, rows: &[LangRow]) -> (Vec<LangRow>, usize) {
    let wanted = unit.rows_for(l);
    let out: Vec<LangRow> = wanted
        .iter()
        .map(|(key, src)| {
            let old = rows.iter().find(|r| &r.key == key);
            let text = old.map(|r| r.text.clone()).unwrap_or_default();
            let hash = match old {
                Some(r) if !r.text.is_empty() && r.hash.is_empty() => source_hash(&src.source),
                Some(r) if !r.text.is_empty() => r.hash.clone(),
                Some(_) | None => String::new(),
            };
            LangRow {
                key: key.clone(),
                source: src.source.clone(),
                text,
                hash,
            }
        })
        .collect();
    let dropped = rows
        .iter()
        .filter(|r| !r.text.is_empty() && !wanted.iter().any(|w| w.0 == r.key))
        .count();
    (out, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::tsv::SourceRow;

    fn unit() -> Unit {
        let row = |k: &str, s: &str| SourceRow {
            key: k.into(),
            source: s.into(),
            ..Default::default()
        };
        Unit::new(vec![
            row("a.title", "Language & Region"),
            row("a.note", "Apps use the new language."),
            row("a.n#one", "{n} file"),
            row("a.n#other", "{n} files"),
        ])
        .expect("unit")
    }

    #[test]
    fn sync_keeps_translations_and_marks_what_they_were_made_for() {
        let u = unit();
        let old = vec![
            LangRow {
                key: "a.title".into(),
                text: "Idioma e região".into(),
                ..Default::default()
            },
            LangRow {
                key: "a.removed".into(),
                text: "Removido".into(),
                ..Default::default()
            },
        ];
        let (rows, dropped) = sync(&u, Lang::PtBr, &old);
        let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(
            keys,
            ["a.title", "a.note", "a.n#one", "a.n#many", "a.n#other"]
        );
        assert_eq!(rows[0].hash, source_hash("Language & Region"));
        assert_eq!(
            rows[3].source, "{n} files",
            "many translates the English other"
        );
        assert!(rows[1].hash.is_empty() && rows[1].text.is_empty());
        assert_eq!(dropped, 1);
        let (ja, _) = sync(&u, Lang::Ja, &[]);
        assert_eq!(ja.len(), 3, "Japanese has one plural form");
    }

    #[test]
    fn coverage_counts_translated_and_stale_rows() {
        let u = unit();
        let rows = vec![LangRow {
            key: "a.title".into(),
            text: "Sprache und Region".into(),
            hash: source_hash("Language"),
            ..Default::default()
        }];
        let (c, problems) = coverage(&u, Lang::De, &rows).expect("coverage");
        assert_eq!((c.rows, c.translated, c.stale, c.errors), (4, 1, 1, 0));
        assert_eq!(c.percent(), 25);
        assert_eq!(problems.len(), 1);
        let (en, _) = coverage(&u, Lang::En, &[]).expect("en");
        assert_eq!(en.percent(), 100);
    }
}
