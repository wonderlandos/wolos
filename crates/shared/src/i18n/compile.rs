// SPDX-License-Identifier: BSD-2-Clause
//! The build-time compiler: `en.tsv` (+ `<lang>.tsv`) -> one `.wtx` table per language, the key
//! constants for Rust ([`keys_rs`]), and the pseudo-locale `xx` ([`pseudo`]). Every rule a
//! translation must keep is checked here, so a broken row fails the build that ships it:
//!
//! - same placeholders as the English (`{name}`; `{{` and `}}` are literal braces);
//! - within the key's budget, in columns ([`width`]: a wide East Asian character counts two);
//! - plural forms only of the categories the language uses ([`super::plural::categories`]);
//! - and in English: a budget leaves room for the pseudo-locale (40 % longer).
//!
//! Warnings do not fail: a row for a key English no longer has, a translation of an older
//! English (stale hash, still shown until it is redone), a missing plural form.
use super::tsv::{LangRow, SourceRow};
use super::wtx::{self, Entry, WtxError};
use super::{Lang, Plural, plural, source_hash};

/// One thing wrong with a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// An error fails the build; a warning is reported by `wol-i18n status`.
    pub error: bool,
    /// The row's key.
    pub key: String,
    /// What is wrong, for the translator.
    pub what: String,
}

fn err(key: &str, what: String) -> Problem {
    Problem {
        error: true,
        key: key.into(),
        what,
    }
}

fn warn(key: &str, what: String) -> Problem {
    Problem {
        error: false,
        key: key.into(),
        what,
    }
}

/// A key of the unit: its name and its rows in en.tsv (one, or one per plural form).
#[derive(Debug, Clone)]
struct Logical {
    name: String,
    /// (form, row in `Unit::rows`); a key that is not plural has one `(None, row)`.
    forms: Vec<(Option<Plural>, usize)>,
}

/// The English source of one unit, checked: the key space every language compiles against.
#[derive(Debug, Clone)]
pub struct Unit {
    rows: Vec<SourceRow>,
    keys: Vec<Logical>,
}

/// Whether `name` is a key name: dot-separated lowercase words (`settings.sound_louder`).
pub fn is_key_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|w| {
            !w.is_empty()
                && w.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}

/// (name, form) of a row key; Err: not a key.
fn split_key(key: &str) -> Result<(&str, Option<Plural>), String> {
    let (name, form) = match key.split_once('#') {
        Some((n, f)) => (
            n,
            Some(Plural::from_name(f).ok_or_else(|| format!("“{f}” is not a plural category"))?),
        ),
        None => (key, None),
    };
    if is_key_name(name) {
        Ok((name, form))
    } else {
        Err("a key is lowercase words joined by dots (settings.sound_louder)".into())
    }
}

/// The `{name}` placeholders of `s`, sorted.
pub fn placeholders(s: &str) -> Vec<&str> {
    let mut v = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find(['{', '}']) {
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            rest = &tail[2..];
            continue;
        }
        if let Some((name, after)) = tail.strip_prefix('{').and_then(|t| t.split_once('}'))
            && !name.is_empty()
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            v.push(name);
            rest = after;
            continue;
        }
        rest = &tail[1..];
    }
    v.sort_unstable();
    v
}

/// Whether `c` takes two columns (East Asian Wide and Fullwidth: Hangul, CJK, kana, fullwidth forms).
fn wide(c: char) -> bool {
    matches!(u32::from(c),
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFD)
}

/// Columns `s` takes: one per character, two per wide character; the longest line of several.
pub fn width(s: &str) -> usize {
    s.split('\n')
        .map(|l| l.chars().map(|c| if wide(c) { 2 } else { 1 }).sum())
        .max()
        .unwrap_or(0)
}

/// Accented look-alikes of ASCII letters (same width, taller), for the pseudo-locale.
const ACCENTS: [(char, char); 40] = [
    ('a', 'å'),
    ('c', 'ç'),
    ('d', 'ð'),
    ('e', 'é'),
    ('g', 'ĝ'),
    ('h', 'ĥ'),
    ('i', 'î'),
    ('j', 'ĵ'),
    ('k', 'ķ'),
    ('l', 'ĺ'),
    ('n', 'ñ'),
    ('o', 'ö'),
    ('r', 'ŕ'),
    ('s', 'š'),
    ('t', 'ţ'),
    ('u', 'ü'),
    ('w', 'ŵ'),
    ('y', 'ý'),
    ('z', 'ž'),
    ('b', 'ƀ'),
    ('A', 'Å'),
    ('C', 'Ç'),
    ('D', 'Ð'),
    ('E', 'É'),
    ('G', 'Ĝ'),
    ('H', 'Ĥ'),
    ('I', 'Î'),
    ('J', 'Ĵ'),
    ('K', 'Ķ'),
    ('L', 'Ĺ'),
    ('N', 'Ñ'),
    ('O', 'Ö'),
    ('R', 'Ŕ'),
    ('S', 'Š'),
    ('T', 'Ţ'),
    ('U', 'Ü'),
    ('W', 'Ŵ'),
    ('Y', 'Ý'),
    ('Z', 'Ž'),
    ('B', 'Ɓ'),
];

/// Columns the pseudo-locale gives a text of `w` columns: 40 % more, rounded up.
pub fn pseudo_width(w: usize) -> usize {
    (w * 14).div_ceil(10).max(w + 2)
}

/// The pseudo-locale of `s`: letters accented, placeholders kept, the longest line 40 % longer
/// and bracketed (`[Ŀåñĝüåĝé ~~]`), so a cut-off end or an untranslated string shows at once.
pub fn pseudo(s: &str) -> String {
    let mut lines = Vec::new();
    for line in s.split('\n') {
        let mut o = String::from("[");
        let mut in_ph = false;
        for c in line.chars() {
            if c == '{' {
                in_ph = true;
            }
            o.push(if in_ph {
                c
            } else {
                ACCENTS.iter().find(|a| a.0 == c).map_or(c, |a| a.1)
            });
            if c == '}' {
                in_ph = false;
            }
        }
        let w = width(line);
        let pad = pseudo_width(w).saturating_sub(w + 2);
        if pad > 0 {
            o.push(' ');
            o.push_str(&"~".repeat(pad.saturating_sub(1)));
        }
        o.push(']');
        lines.push(o);
    }
    lines.join("\n")
}

impl Unit {
    /// Checks en.tsv's rows: key names, no duplicates, plural keys with `one` and `other`,
    /// budgets that fit the English and leave room for the pseudo-locale. Err: every problem.
    pub fn new(rows: Vec<SourceRow>) -> Result<Unit, Vec<Problem>> {
        let mut bad = Vec::new();
        let mut keys: Vec<Logical> = Vec::new();
        for (i, r) in rows.iter().enumerate() {
            let (name, form) = match split_key(&r.key) {
                Ok(v) => v,
                Err(e) => {
                    bad.push(err(&r.key, e));
                    continue;
                }
            };
            if let Some(f) = form
                && !plural::categories(Lang::En).contains(&f)
            {
                bad.push(err(&r.key, format!("English has no “{}” form", f.name())));
            }
            if let Some(b) = r.budget {
                let need = pseudo_width(width(&r.source));
                if usize::try_from(b).unwrap_or(usize::MAX) < need {
                    bad.push(err(
                        &r.key,
                        format!("budget {b} leaves no room for 40 % longer text: at least {need}"),
                    ));
                }
            }
            match keys.iter_mut().find(|k| k.name == name) {
                Some(k)
                    if k.forms
                        .iter()
                        .any(|f| f.0 == form || f.0.is_none() || form.is_none()) =>
                {
                    bad.push(err(&r.key, "this key is listed twice".into()));
                }
                Some(k) => k.forms.push((form, i)),
                None => keys.push(Logical {
                    name: name.into(),
                    forms: vec![(form, i)],
                }),
            }
        }
        for k in &keys {
            let plural = k.forms.iter().any(|f| f.0.is_some());
            if plural && !k.forms.iter().any(|f| f.0 == Some(Plural::Other)) {
                bad.push(err(&k.name, "a plural key needs an “other” form".into()));
            }
        }
        if bad.is_empty() {
            Ok(Unit { rows, keys })
        } else {
            Err(bad)
        }
    }

    /// The English rows, as given.
    pub fn rows(&self) -> &[SourceRow] {
        &self.rows
    }

    /// The key names, in table order.
    pub fn names(&self) -> Vec<String> {
        self.keys.iter().map(|k| k.name.clone()).collect()
    }

    /// Rows a translation into `l` has, with the English each translates: one per key, or one
    /// per plural category of `l` (a category English lacks translates the English `other`).
    pub fn rows_for(&self, l: Lang) -> Vec<(String, &SourceRow)> {
        let mut v = Vec::new();
        for k in &self.keys {
            let row = |f: Option<Plural>| {
                k.forms
                    .iter()
                    .find(|x| x.0 == f)
                    .or_else(|| k.forms.iter().find(|x| x.0 == Some(Plural::Other)))
                    .and_then(|x| self.rows.get(x.1))
            };
            if k.forms.iter().all(|f| f.0.is_none()) {
                if let Some(r) = row(None) {
                    v.push((k.name.clone(), r));
                }
                continue;
            }
            for c in plural::categories(l) {
                if let Some(r) = row(Some(*c)) {
                    v.push((format!("{}#{}", k.name, c.name()), r));
                }
            }
        }
        v
    }
}

/// One language compiled.
#[derive(Debug, Clone)]
pub struct Compiled {
    /// The `.wtx` table.
    pub bytes: Vec<u8>,
    /// Errors (the build fails) and warnings.
    pub problems: Vec<Problem>,
    /// Rows the language should have (keys, plural forms counted one by one).
    pub rows: usize,
    /// Rows translated (English and `xx`: all).
    pub translated: usize,
    /// Rows whose translation was made for an older English.
    pub stale: usize,
}

impl Compiled {
    /// Whether nothing is an error.
    pub fn ok(&self) -> bool {
        !self.problems.iter().any(|p| p.error)
    }
}

/// The checks of one translated text against its English row.
fn check_text(key: &str, text: &str, src: &SourceRow, out: &mut Vec<Problem>) {
    if placeholders(text) != placeholders(&src.source) {
        out.push(err(
            key,
            format!(
                "placeholders differ from the English: {:?} here, {:?} there",
                placeholders(text),
                placeholders(&src.source)
            ),
        ));
    }
    if let Some(b) = src.budget
        && width(text) > usize::try_from(b).unwrap_or(usize::MAX)
    {
        out.push(err(
            key,
            format!("{} columns, the budget is {b}", width(text)),
        ));
    }
}

/// Compiles `l` for `unit`: English and `xx` from the source (`rows` ignored), any other
/// language from its rows. Err: the table could not be written (too large).
pub fn compile(unit: &Unit, l: Lang, rows: &[LangRow]) -> Result<Compiled, WtxError> {
    let mut problems = Vec::new();
    let wanted = unit.rows_for(l);
    let mut seen: Vec<&str> = Vec::new();
    let (mut translated, mut stale) = (0, 0);
    for r in rows {
        if seen.contains(&r.key.as_str()) {
            problems.push(err(&r.key, "this key is listed twice".into()));
        }
        seen.push(&r.key);
        if r.text.is_empty() {
            continue;
        }
        let Some((_, src)) = wanted.iter().find(|w| w.0 == r.key) else {
            let form_of_a_key = match split_key(&r.key) {
                Ok((name, Some(f))) => unit.keys.iter().any(|k| k.name == name).then_some(f),
                Ok((_, None)) | Err(_) => None,
            };
            problems.push(match form_of_a_key {
                Some(f) => err(&r.key, format!("{} has no “{}” form", l.tag(), f.name())),
                None => warn(&r.key, "English has no such key any more".into()),
            });
            continue;
        };
        translated += 1;
        if !r.hash.is_empty() && r.hash != source_hash(&src.source) {
            stale += 1;
            problems.push(warn(&r.key, "made for an older English: check it".into()));
        }
        check_text(&r.key, &r.text, src, &mut problems);
    }
    let text_of = |row_key: &str, src: &SourceRow| -> Option<String> {
        match l {
            Lang::En => Some(src.source.clone()),
            Lang::Xx => Some(pseudo(&src.source)),
            Lang::PtBr | Lang::Ja | Lang::Fr | Lang::Ru | Lang::It | Lang::De => rows
                .iter()
                .find(|r| r.key == row_key && !r.text.is_empty())
                .map(|r| r.text.clone()),
        }
    };
    let mut entries = Vec::with_capacity(unit.keys.len());
    for k in &unit.keys {
        let forms: Vec<&(String, &SourceRow)> = wanted
            .iter()
            .filter(|w| w.0.split('#').next() == Some(k.name.as_str()))
            .collect();
        if k.forms.iter().all(|f| f.0.is_none()) {
            entries.push(
                forms
                    .first()
                    .and_then(|(key, src)| text_of(key, src))
                    .map_or(Entry::Absent, Entry::Text),
            );
            continue;
        }
        let mut slots: [Option<String>; 6] = Default::default();
        for (key, src) in &forms {
            let cat = key
                .split_once('#')
                .and_then(|(_, c)| Plural::from_name(c))
                .unwrap_or(Plural::Other);
            slots[cat.index()] = text_of(key, src);
        }
        let any = slots.iter().any(Option::is_some);
        if any && slots[Plural::Other.index()].is_none() {
            problems.push(warn(&k.name, format!("{} has no “other” form", l.tag())));
        }
        entries.push(if any {
            Entry::Plural(slots)
        } else {
            Entry::Absent
        });
    }
    if matches!(l, Lang::En | Lang::Xx) {
        translated = wanted.len();
    }
    Ok(Compiled {
        bytes: wtx::encode(l, &unit.names(), &entries)?,
        problems,
        rows: wanted.len(),
        translated,
        stale,
    })
}

/// The Rust constant name of key `name` (`settings.sound_louder` -> `SETTINGS_SOUND_LOUDER`).
pub fn const_name(name: &str) -> String {
    name.replace('.', "_").to_ascii_uppercase()
}

/// Rust source with one `Key` constant per key of `unit` (included by the unit's `build.rs`
/// output), each documented with its English. Err: two keys with the same constant name.
pub fn keys_rs(unit: &Unit, origin: &str) -> Result<String, Vec<Problem>> {
    let mut s =
        format!("// Generated from {origin} by shared::i18n::compile::keys_rs: do not edit.\n");
    let mut names: Vec<String> = Vec::new();
    let mut bad = Vec::new();
    for (i, k) in unit.keys.iter().enumerate() {
        let c = const_name(&k.name);
        if names.contains(&c) {
            bad.push(err(&k.name, format!("two keys make the constant {c}")));
        }
        let english = k
            .forms
            .iter()
            .find(|f| f.0.is_none() || f.0 == Some(Plural::Other))
            .and_then(|f| unit.rows.get(f.1))
            .map(|r| r.source.replace('\n', " "))
            .unwrap_or_default();
        s.push_str(&format!(
            "/// `{}`: {english}\npub const {c}: shared::i18n::Key = shared::i18n::Key({i});\n",
            k.name
        ));
        names.push(c);
    }
    s.push_str(&format!(
        "/// Keys in the unit.\npub const COUNT: usize = {};\n",
        unit.keys.len()
    ));
    if bad.is_empty() { Ok(s) } else { Err(bad) }
}

/// Rust source with `SOURCES`, every key's English (a plural key's `other` form) and its index,
/// sorted by the English: how a program finds the key of a text it translates where it uses it
/// (`tr("Choose a password.")`, L10N-2). Two keys with one English: the first one.
pub fn sources_rs(unit: &Unit, origin: &str) -> String {
    let mut v: Vec<(&str, usize)> = Vec::new();
    for (i, k) in unit.keys.iter().enumerate() {
        if let Some(r) = k
            .forms
            .iter()
            .find(|f| f.0.is_none() || f.0 == Some(Plural::Other))
            .and_then(|f| unit.rows.get(f.1))
            && !v.iter().any(|x| x.0 == r.source)
        {
            v.push((&r.source, i));
        }
    }
    v.sort_unstable();
    let mut s = format!(
        "// Generated from {origin} by shared::i18n::compile::sources_rs: do not edit.\n\
         /// (English, key index), sorted by the English.\n\
         pub const SOURCES: &[(&str, u32)] = &[\n"
    );
    for (src, i) in v {
        s.push_str(&format!("    ({src:?}, {i}),\n"));
    }
    s.push_str("];\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Catalog, Key, Table};

    fn src(key: &str, source: &str, budget: Option<u32>) -> SourceRow {
        SourceRow {
            key: key.into(),
            source: source.into(),
            comment: String::new(),
            budget,
        }
    }

    fn tr(key: &str, text: &str) -> LangRow {
        LangRow {
            key: key.into(),
            text: text.into(),
            ..Default::default()
        }
    }

    fn unit() -> Unit {
        Unit::new(vec![
            src("lang.title", "Language & Region", Some(30)),
            src("lang.join", "Join “{name}”.", None),
            src("lang.files#one", "{n} file", None),
            src("lang.files#other", "{n} files", None),
        ])
        .expect("sample unit is valid")
    }

    #[test]
    fn a_translation_compiles_and_falls_back_to_english() {
        let u = unit();
        let en = compile(&u, Lang::En, &[]).expect("en compiles");
        let fr = compile(
            &u,
            Lang::Fr,
            &[
                tr("lang.title", "Langue et région"),
                tr("lang.files#one", "{n} fichier"),
                tr("lang.files#many", "{n} de fichiers"),
                tr("lang.files#other", "{n} fichiers"),
            ],
        )
        .expect("fr compiles");
        assert!(fr.ok(), "{:?}", fr.problems);
        assert_eq!((fr.rows, fr.translated), (5, 4));
        let mut c = Catalog::new(Table::from_vec(en.bytes).expect("en")).expect("catalog");
        c.add(Table::from_vec(fr.bytes).expect("fr"))
            .expect("fr added");
        assert_eq!(c.text(Key(0), Lang::Fr), "Langue et région");
        assert_eq!(
            c.text(Key(1), Lang::Fr),
            "Join “{name}”.",
            "untranslated: English"
        );
        assert_eq!(c.plural(Key(2), Some(0), Lang::Fr), "{n} fichier");
        assert_eq!(c.plural(Key(2), Some(2), Lang::Fr), "{n} fichiers");
        assert_eq!(
            c.plural(Key(2), Some(3_000_000), Lang::Fr),
            "{n} de fichiers"
        );
        assert_eq!(
            c.plural(Key(2), Some(1), Lang::Ja),
            "{n} file",
            "no ja table: English rules"
        );
        assert_eq!(c.plural(Key(2), Some(1), Lang::En), "{n} file");
        assert_eq!(c.find("lang.join"), Some(Key(1)));
        assert_eq!(c.text(Key(9), Lang::Fr), "", "unknown key reads empty");
    }

    #[test]
    fn broken_translations_fail_the_build() {
        let u = unit();
        let c = compile(
            &u,
            Lang::De,
            &[
                tr("lang.title", "Sprache und Region und noch viel mehr Text"),
                tr("lang.join", "„{netz}“ beitreten."),
                tr("lang.files#many", "{n} Dateien"),
                tr("lang.gone", "Weg"),
            ],
        )
        .expect("de compiles");
        assert!(!c.ok());
        let errors: Vec<&str> = c
            .problems
            .iter()
            .filter(|p| p.error)
            .map(|p| p.key.as_str())
            .collect();
        assert_eq!(errors, ["lang.title", "lang.join", "lang.files#many"]);
        assert!(c.problems.iter().any(|p| !p.error && p.key == "lang.gone"));
    }

    #[test]
    fn stale_translations_still_show_but_are_counted() {
        let u = unit();
        let mut row = tr("lang.title", "Idioma e região");
        row.hash = source_hash("Language");
        let c = compile(&u, Lang::PtBr, &[row]).expect("pt-BR compiles");
        assert!(c.ok());
        assert_eq!(c.stale, 1);
        let mut fresh = tr("lang.title", "Idioma e região");
        fresh.hash = source_hash("Language & Region");
        assert_eq!(
            compile(&u, Lang::PtBr, &[fresh]).expect("compiles").stale,
            0
        );
    }

    #[test]
    fn source_rules_are_checked_before_anything_compiles() {
        let bad = Unit::new(vec![
            src("Bad Key", "x", None),
            src("a.b", "One", None),
            src("a.b", "Two", None),
            src("a.n#one", "{n} thing", None),
            src("a.n#few", "{n} things", None),
            src("a.short", "Language & Region", Some(20)),
        ])
        .unwrap_err();
        let keys: Vec<&str> = bad.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["Bad Key", "a.b", "a.n#few", "a.short", "a.n"]);
    }

    #[test]
    fn pseudo_text_is_forty_percent_longer_and_keeps_placeholders() {
        let p = pseudo("Join “{name}” now.");
        assert!(p.contains("{name}"), "{p}");
        assert!(p.starts_with('[') && p.ends_with(']'));
        assert_eq!(width(&p), pseudo_width(width("Join “{name}” now.")));
        assert_eq!(placeholders(&p), ["name"]);
        assert_eq!(pseudo("Language & Region"), "[Ĺåñĝüåĝé & Ŕéĝîöñ ~~~~]");
        assert_eq!(width(&pseudo("OK")), 4);
        assert_eq!(width("日本語"), 6);
        assert_eq!(width("two\nlines here"), 10);
    }

    #[test]
    fn keys_become_documented_rust_constants() {
        let rs = keys_rs(&unit(), "locale/x/en.tsv").expect("constants");
        assert!(rs.contains("pub const LANG_TITLE: shared::i18n::Key = shared::i18n::Key(0);"));
        assert!(rs.contains("pub const LANG_FILES: shared::i18n::Key = shared::i18n::Key(2);"));
        assert!(rs.contains("/// `lang.files`: {n} files"));
        assert!(rs.contains("pub const COUNT: usize = 3;"));
    }
}
