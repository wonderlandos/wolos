// SPDX-License-Identifier: BSD-2-Clause
//! The locale system (ADR 0045): text a person reads, in their language, from compiled tables.
//! Translators edit TSV files (`locale/<unit>/en.tsv` and `<lang>.tsv`, see docs/l10n/README.md);
//! the build compiles them into `.wtx` tables ([`wtx`]); at run time a [`Catalog`] answers a
//! [`Key`] for a [`Lang`] in constant time, without parsing text or allocating, walking the
//! fallback chain ([`Lang::chain`]: the language, then English).
//!
//! Invariant: an English table holds every key of its unit, so a lookup always finds text; a key
//! a broken table lacks reads as the empty string, never a panic.
pub mod compile;
pub mod extract;
pub mod files;
pub mod plural;
pub mod status;
pub mod tsv;
pub mod wtx;

use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};

pub use plural::Plural;
pub use wtx::{Table, WtxError};

/// One string of a unit: its index in the unit's English table (generated as a constant from
/// en.tsv by [`compile::keys_rs`], or found by name with [`Catalog::find`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key(pub u32);

/// A language WolOS is translated into, plus the pseudo-locale `xx` (every string 40 % longer
/// and accented, for layout tests; never offered in a menu). Variant order is table order
/// ([`Lang::ALL`]); what a person can pick is [`Lang::OFFERED`], what each catalogue is worth is
/// [`Lang::stage`]. A `.wtx` table names its language by tag, so the order is free to change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lang {
    /// English, the source language.
    En,
    /// Portuguese as written in Brazil.
    PtBr,
    /// Spanish, neutral international (RULEBOOK-es).
    Es,
    /// French.
    Fr,
    /// German.
    De,
    /// Japanese.
    Ja,
    /// Korean.
    Ko,
    /// Chinese in Simplified characters (`zh-Hans`; `zh_CN`, `zh_SG`).
    ZhHans,
    /// Indonesian.
    Id,
    /// Tagalog (Filipino; `tl_PH`, `fil_PH`).
    Tl,
    /// Russian: a draft catalogue, not offered (lead, 2026-11-06).
    Ru,
    /// Italian: a draft catalogue, not offered (lead, 2026-11-06).
    It,
    /// The pseudo-locale.
    Xx,
}

/// What a language's catalogue is worth today: whether it is offered and how strictly it is checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    /// English, the source: every key, by construction.
    Source,
    /// Offered, and every key is translated (`every_official_language_is_complete`).
    Complete,
    /// Offered, being translated: every row is in its file, the empty ones read English.
    Filling,
    /// Kept as a draft, not offered in any picker; its file still compiles.
    Draft,
    /// The pseudo-locale `xx`, for layout tests only.
    Pseudo,
}

/// Number of [`Lang`] values (tables a catalog can hold).
const N_LANGS: usize = 13;

/// (language, BCP 47 tag, the name of the language in itself, stage).
const LANGS: [(Lang, &str, &str, Stage); N_LANGS] = [
    (Lang::En, "en", "English", Stage::Source),
    (Lang::PtBr, "pt-BR", "Português (Brasil)", Stage::Complete),
    (Lang::Es, "es", "Español", Stage::Filling),
    (Lang::Fr, "fr", "Français", Stage::Complete),
    (Lang::De, "de", "Deutsch", Stage::Complete),
    (Lang::Ja, "ja", "日本語", Stage::Complete),
    (Lang::Ko, "ko", "한국어", Stage::Filling),
    (Lang::ZhHans, "zh-Hans", "简体中文", Stage::Filling),
    (Lang::Id, "id", "Bahasa Indonesia", Stage::Filling),
    (Lang::Tl, "tl", "Tagalog", Stage::Filling),
    (Lang::Ru, "ru", "Русский", Stage::Draft),
    (Lang::It, "it", "Italiano", Stage::Draft),
    (Lang::Xx, "xx", "Pseudo", Stage::Pseudo),
];

/// Language subtag (ISO 639, lowercase) -> language, for locales and tags without a script or
/// region that decides (`zh` is decided by [`chinese`]).
const BASE: [(&str, Lang); 12] = [
    ("en", Lang::En),
    ("pt", Lang::PtBr),
    ("es", Lang::Es),
    ("fr", Lang::Fr),
    ("de", Lang::De),
    ("ja", Lang::Ja),
    ("ko", Lang::Ko),
    ("id", Lang::Id),
    ("tl", Lang::Tl),
    ("fil", Lang::Tl),
    ("ru", Lang::Ru),
    ("it", Lang::It),
];

/// Chinese by script or region subtag: Simplified for `Hans`, mainland China, Singapore, Malaysia
/// and a bare `zh` (CLDR likely subtags: zh -> zh-Hans-CN); None for Traditional (`Hant`, Taiwan,
/// Hong Kong, Macao), which WolOS has no table for: reading English beats reading the other script.
fn chinese(subtags: &[&str]) -> Option<Lang> {
    let has = |want: &[&str]| {
        subtags
            .iter()
            .any(|t| want.iter().any(|w| t.eq_ignore_ascii_case(w)))
    };
    if has(&["Hans"]) {
        Some(Lang::ZhHans)
    } else if has(&["Hant", "TW", "HK", "MO"]) {
        None
    } else {
        Some(Lang::ZhHans)
    }
}

impl Lang {
    /// Every language, in table order (`Lang::ALL[l.index()] == l`).
    pub const ALL: [Lang; N_LANGS] = [
        Lang::En,
        Lang::PtBr,
        Lang::Es,
        Lang::Fr,
        Lang::De,
        Lang::Ja,
        Lang::Ko,
        Lang::ZhHans,
        Lang::Id,
        Lang::Tl,
        Lang::Ru,
        Lang::It,
        Lang::Xx,
    ];

    /// Every language with a catalogue file (`<tag>.tsv`): all but the pseudo-locale, English
    /// first. What `wol-i18n status`, `budgets` and `sync all` walk.
    pub const CATALOGUED: [Lang; 12] = [
        Lang::En,
        Lang::PtBr,
        Lang::Es,
        Lang::Fr,
        Lang::De,
        Lang::Ja,
        Lang::Ko,
        Lang::ZhHans,
        Lang::Id,
        Lang::Tl,
        Lang::Ru,
        Lang::It,
    ];

    /// The languages people can choose, in the order the installer, Setup and Settings list them:
    /// English, then the lead's nine (2026-11-06). Exactly the languages whose [`Lang::stage`] is
    /// offered; `wol_core::region::LANGUAGES` lists the same, in the same order.
    pub const OFFERED: [Lang; 10] = [
        Lang::En,
        Lang::PtBr,
        Lang::Es,
        Lang::Fr,
        Lang::De,
        Lang::Ja,
        Lang::Ko,
        Lang::ZhHans,
        Lang::Id,
        Lang::Tl,
    ];

    /// Position in [`Lang::ALL`].
    pub fn index(self) -> usize {
        Lang::ALL.iter().position(|l| *l == self).unwrap_or(0)
    }

    /// BCP 47 tag, also the TSV and table file name (`pt-BR.tsv`, `shell.zh-Hans.wtx`).
    pub fn tag(self) -> &'static str {
        LANGS.iter().find(|r| r.0 == self).map_or("en", |r| r.1)
    }

    /// The language's name in itself ("Deutsch"), for menus.
    pub fn endonym(self) -> &'static str {
        LANGS
            .iter()
            .find(|r| r.0 == self)
            .map_or("English", |r| r.2)
    }

    /// What this language's catalogue is worth today ([`Stage`]).
    pub fn stage(self) -> Stage {
        LANGS
            .iter()
            .find(|r| r.0 == self)
            .map_or(Stage::Draft, |r| r.3)
    }

    /// Whether a picker (installer, Setup, Settings) lists this language: English, complete and
    /// filling languages; never a draft (ru, it) or the pseudo-locale.
    pub fn offered(self) -> bool {
        matches!(
            self.stage(),
            Stage::Source | Stage::Complete | Stage::Filling
        )
    }

    /// The language a tag names exactly (`pt-BR`, `zh-Hans`), as file names and tables write it;
    /// None for any other text ([`Lang::from_locale`] reads the other spellings).
    pub fn from_tag(tag: &str) -> Option<Lang> {
        LANGS.iter().find(|r| r.1 == tag).map(|r| r.0)
    }

    /// The language of a POSIX locale (`pt_BR.UTF-8`, `de_DE`, `C`). Portuguese of any region
    /// reads Brazilian Portuguese, `C`/`POSIX` read English, `zh_CN`/`zh_SG` Simplified Chinese,
    /// `fil_PH` Tagalog. None: a language without a translation (`nl_NL`, `zh_TW`): the caller
    /// shows English.
    pub fn from_posix(locale: &str) -> Option<Lang> {
        let base = locale.split(['.', '@']).next().unwrap_or("");
        if base == "C" || base == "POSIX" {
            return Some(Lang::En);
        }
        Lang::from_locale(base)
    }

    /// The language of a BCP 47 tag or POSIX name in any common spelling, case-insensitive:
    /// `zh-Hans`, `zh-hans-CN`, `zh_CN`, `zh-SG`, `es-419`, `pt-PT`, `fil`. The exact tags of
    /// [`Lang::from_tag`] (`xx` included) read as themselves. None: no table for it.
    pub fn from_locale(name: &str) -> Option<Lang> {
        if let Some(l) = Lang::from_tag(name) {
            return Some(l);
        }
        let name = name.split(['.', '@']).next().unwrap_or("");
        let mut parts = name.split(['-', '_']);
        let code = parts.next()?.to_ascii_lowercase();
        let rest: Vec<&str> = parts.collect();
        if code == "zh" {
            return chinese(&rest);
        }
        BASE.iter().find(|b| b.0 == code).map(|b| b.1)
    }

    /// Where text comes from, in order: this language, then English (`pt-BR -> en`,
    /// `zh-Hans -> en`). English and the pseudo-locale have English last too.
    pub fn chain(self) -> &'static [Lang] {
        const CHAINS: [[Lang; 2]; N_LANGS] = [
            [Lang::En, Lang::En],
            [Lang::PtBr, Lang::En],
            [Lang::Es, Lang::En],
            [Lang::Fr, Lang::En],
            [Lang::De, Lang::En],
            [Lang::Ja, Lang::En],
            [Lang::Ko, Lang::En],
            [Lang::ZhHans, Lang::En],
            [Lang::Id, Lang::En],
            [Lang::Tl, Lang::En],
            [Lang::Ru, Lang::En],
            [Lang::It, Lang::En],
            [Lang::Xx, Lang::En],
        ];
        let c = &CHAINS[self.index()];
        if self == Lang::En { &c[..1] } else { &c[..] }
    }
}

/// The process's language, [`Lang::En`] until [`set_lang`].
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// The language this process shows text in.
pub fn lang() -> Lang {
    let i = usize::from(CURRENT.load(Ordering::Relaxed));
    Lang::ALL.get(i).copied().unwrap_or(Lang::En)
}

/// Shows text in `l` from now on (the account's language, or the one just chosen in Settings or
/// Setup). Surfaces that keep text on screen relabel themselves (shell `i18n::set_lang`).
pub fn set_lang(l: Lang) {
    let i = u8::try_from(l.index()).unwrap_or(0);
    CURRENT.store(i, Ordering::Relaxed);
}

/// A language's table as the catalog holds it: the table and, for a pack compiled against
/// another list of keys (an older or newer app), the catalog key -> table entry map.
struct Loaded {
    table: Table,
    remap: Option<Box<[u32]>>,
}

impl Loaded {
    fn entry(&self, key: Key) -> Option<u32> {
        match &self.remap {
            Some(m) => m
                .get(usize::try_from(key.0).ok()?)
                .copied()
                .filter(|e| *e != wtx::ABSENT),
            None => Some(key.0),
        }
    }
}

/// Every table of one unit (the shell, an app): English, which defines the keys, and any
/// translations. Lookups never allocate.
pub struct Catalog {
    tables: [Option<Loaded>; N_LANGS],
}

impl Catalog {
    /// A catalog whose keys are those of `en`. Err: `en` is not an English table.
    pub fn new(en: Table) -> Result<Catalog, WtxError> {
        if en.lang() != Lang::En {
            return Err(WtxError::NotEnglish);
        }
        let mut tables: [Option<Loaded>; N_LANGS] = Default::default();
        tables[0] = Some(Loaded {
            table: en,
            remap: None,
        });
        Ok(Catalog { tables })
    }

    /// A catalog with no keys: what a program shows when its own tables are broken (every lookup
    /// reads as the empty string, nothing panics).
    pub fn empty() -> Catalog {
        Catalog {
            tables: [const { None }; N_LANGS],
        }
    }

    /// Adds (or replaces) the table of a translation. A table compiled against another list of
    /// keys is matched by key name once, here. Err: an English table (use [`Catalog::new`]).
    pub fn add(&mut self, t: Table) -> Result<(), WtxError> {
        let l = t.lang();
        if l == Lang::En {
            return Err(WtxError::NotEnglish);
        }
        let en = self.english();
        let remap = (t.catalog() != en.catalog()).then(|| {
            (0..en.len())
                .map(|i| {
                    en.key_name(i)
                        .and_then(|name| t.find(name))
                        .unwrap_or(wtx::ABSENT)
                })
                .collect()
        });
        self.tables[l.index()] = Some(Loaded { table: t, remap });
        Ok(())
    }

    /// Tables built into the binary: `(lang tag, bytes)`, English among them. Err: a table that
    /// does not parse, or no English one.
    pub fn embedded(tables: &[(&str, &'static [u8])]) -> Result<Catalog, WtxError> {
        let mut en = None;
        let mut rest = Vec::new();
        for (_, bytes) in tables {
            let t = Table::from_static(bytes)?;
            if t.lang() == Lang::En {
                en = Some(t);
            } else {
                rest.push(t);
            }
        }
        let mut c = Catalog::new(en.ok_or(WtxError::NotEnglish)?)?;
        for t in rest {
            c.add(t)?;
        }
        Ok(c)
    }

    /// The tables a unit ships as files (`<dir>/<unit>.<lang>.wtx`, a language pack can add one
    /// without a rebuild); a translation that is missing or does not parse is skipped. Err: the
    /// English table is missing or broken.
    pub fn load_dir(dir: &Path, unit: &str) -> Result<Catalog, WtxError> {
        let read = |l: Lang| std::fs::read(dir.join(format!("{unit}.{}.wtx", l.tag())));
        let en = read(Lang::En).map_err(|_| WtxError::NotEnglish)?;
        let mut c = Catalog::new(Table::from_vec(en)?)?;
        for l in Lang::ALL.into_iter().skip(1) {
            if let Some(t) = read(l).ok().and_then(|b| Table::from_vec(b).ok())
                && t.lang() == l
            {
                c.add(t)?;
            }
        }
        Ok(c)
    }

    fn english(&self) -> &Table {
        // Catalog::new put English at 0 and add() never replaces it.
        match &self.tables[0] {
            Some(l) => &l.table,
            None => &EMPTY_EN,
        }
    }

    /// Whether `l` has a table here (English always does).
    pub fn has(&self, l: Lang) -> bool {
        self.tables[l.index()].is_some()
    }

    /// Number of keys.
    pub fn len(&self) -> usize {
        usize::try_from(self.english().len()).unwrap_or(usize::MAX)
    }

    /// Whether the unit has no keys.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The key named `name` (`settings.sound_louder`), for callers that only have the name (Lua
    /// plugins, Cryswol apps); None: no such key.
    pub fn find(&self, name: &str) -> Option<Key> {
        self.english().find(name).map(Key)
    }

    /// The name of `key` (tools, diagnostics); None: not a key of this unit.
    pub fn key_name(&self, key: Key) -> Option<&str> {
        self.english().key_name(key.0)
    }

    /// The text of `key` in `l`, or the next language of its chain that has it; "" for a key
    /// this unit does not have. A plural key reads its `other` form.
    pub fn text(&self, key: Key, l: Lang) -> &str {
        self.plural(key, None, l)
    }

    /// The form of plural key `key` for the count `n` in `l` (CLDR cardinal rules), falling
    /// back along the chain; a language without the form uses its own `other` before falling
    /// back. `n` None: the `other` form. A key that is not plural reads its only text.
    pub fn plural(&self, key: Key, n: Option<u64>, l: Lang) -> &str {
        for &step in l.chain() {
            let Some(loaded) = &self.tables[step.index()] else {
                continue;
            };
            let Some(e) = loaded.entry(key) else {
                continue;
            };
            let cat = n.map_or(Plural::Other, |n| plural::category(step, n));
            if let Some(s) = loaded.table.get(e, cat) {
                return s;
            }
        }
        ""
    }
}

/// What [`Catalog::english`] answers before a table exists (never, by construction).
static EMPTY_EN: std::sync::LazyLock<Table> = std::sync::LazyLock::new(Table::empty_english);

/// `template` with each `{name}` replaced by its value in `args` (`{{` and `}}` are braces), into
/// `out`. A name not in `args` stays as written, so a missing value shows instead of vanishing.
pub fn fill_into(template: &str, args: &[(&str, &str)], out: &mut String) {
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        if let Some(name) = tail.strip_prefix('{').and_then(|t| t.split_once('}'))
            && let Some((_, v)) = args.iter().find(|a| a.0 == name.0)
        {
            out.push_str(v);
            rest = name.1;
            continue;
        }
        out.push_str(&tail[..1]);
        rest = &tail[1..];
    }
    out.push_str(rest);
}

/// [`fill_into`] into a new string.
pub fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut s = String::with_capacity(template.len() + 16);
    fill_into(template, args, &mut s);
    s
}

/// FNV-1a, 32 bits: key slots in a table, the catalog identity and the source hash that marks a
/// translation stale (not a security hash).
pub fn fnv1a(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in bytes {
        h ^= u32::from(*b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// The hash a translation row records for the English it translated (8 hex digits).
pub fn source_hash(source: &str) -> String {
    format!("{:08x}", fnv1a(source.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_locales_map_to_their_language() {
        assert_eq!(Lang::from_posix("pt_BR.UTF-8"), Some(Lang::PtBr));
        assert_eq!(Lang::from_posix("pt_PT.UTF-8"), Some(Lang::PtBr));
        assert_eq!(Lang::from_posix("en_GB.UTF-8"), Some(Lang::En));
        assert_eq!(Lang::from_posix("ja_JP.UTF-8"), Some(Lang::Ja));
        assert_eq!(Lang::from_posix("ru_RU.UTF-8"), Some(Lang::Ru));
        assert_eq!(Lang::from_posix("it_IT.UTF-8"), Some(Lang::It));
        assert_eq!(Lang::from_posix("de_AT.UTF-8"), Some(Lang::De));
        assert_eq!(Lang::from_posix("es_ES.UTF-8"), Some(Lang::Es));
        assert_eq!(Lang::from_posix("es_MX.UTF-8"), Some(Lang::Es));
        assert_eq!(Lang::from_posix("ko_KR.UTF-8"), Some(Lang::Ko));
        assert_eq!(Lang::from_posix("id_ID.UTF-8"), Some(Lang::Id));
        assert_eq!(Lang::from_posix("tl_PH.UTF-8"), Some(Lang::Tl));
        assert_eq!(Lang::from_posix("fil_PH.UTF-8"), Some(Lang::Tl));
        assert_eq!(Lang::from_posix("zh_CN.UTF-8"), Some(Lang::ZhHans));
        assert_eq!(Lang::from_posix("zh_SG.UTF-8"), Some(Lang::ZhHans));
        assert_eq!(Lang::from_posix("zh_TW.UTF-8"), None);
        assert_eq!(Lang::from_posix("zh_HK.UTF-8"), None);
        assert_eq!(Lang::from_posix("nl_NL.UTF-8"), None);
        assert_eq!(Lang::from_posix("C.UTF-8"), Some(Lang::En));
        assert_eq!(Lang::from_posix("POSIX"), Some(Lang::En));
    }

    #[test]
    fn chinese_tags_pick_simplified_or_nothing() {
        for t in [
            "zh-Hans",
            "zh-hans",
            "zh-Hans-CN",
            "zh_Hans",
            "zh-CN",
            "zh-SG",
            "zh",
            "ZH-cn",
        ] {
            assert_eq!(Lang::from_locale(t), Some(Lang::ZhHans), "{t}");
        }
        for t in ["zh-Hant", "zh-Hant-CN", "zh-TW", "zh_HK", "zh-MO"] {
            assert_eq!(Lang::from_locale(t), None, "{t}");
        }
        assert_eq!(Lang::from_tag("zh-Hans"), Some(Lang::ZhHans));
        assert_eq!(
            Lang::from_tag("zh-CN"),
            None,
            "file names use the exact tag"
        );
        assert_eq!(Lang::ZhHans.tag(), "zh-Hans");
        assert_eq!(Lang::ZhHans.chain(), &[Lang::ZhHans, Lang::En]);
        assert_eq!(Lang::from_locale("es-419"), Some(Lang::Es));
        assert_eq!(Lang::from_locale("pt-PT"), Some(Lang::PtBr));
        assert_eq!(Lang::from_locale("xx"), Some(Lang::Xx));
        assert_eq!(Lang::from_locale(""), None);
    }

    #[test]
    fn tags_round_trip_for_every_language() {
        for (i, l) in Lang::ALL.into_iter().enumerate() {
            assert_eq!(l.index(), i);
            assert_eq!(Lang::from_tag(l.tag()), Some(l));
            assert_eq!(Lang::from_locale(l.tag()), Some(l));
            assert!(l.tag().len() <= 12, "a .wtx header holds 12 bytes of tag");
            assert_eq!(l.chain().first(), Some(&l));
            assert_eq!(l.chain().last(), Some(&Lang::En));
        }
        assert_eq!(Lang::PtBr.tag(), "pt-BR");
        assert_eq!(Lang::Ru.chain(), &[Lang::Ru, Lang::En]);
        assert_eq!(Lang::En.chain(), &[Lang::En]);
        let tags: std::collections::BTreeSet<_> = Lang::ALL.iter().map(|l| l.tag()).collect();
        assert_eq!(tags.len(), N_LANGS);
    }

    /// The lead's set (2026-11-06): English and pt-BR, es, fr, de, ja, ko, zh-Hans, id, tl are
    /// offered, in that order; ru and it are drafts no picker shows; `xx` is never offered.
    #[test]
    fn the_offered_languages_are_english_and_the_leads_nine() {
        let tags: Vec<&str> = Lang::OFFERED.iter().map(|l| l.tag()).collect();
        assert_eq!(
            tags,
            [
                "en", "pt-BR", "es", "fr", "de", "ja", "ko", "zh-Hans", "id", "tl"
            ]
        );
        let offered: Vec<Lang> = Lang::ALL.into_iter().filter(|l| l.offered()).collect();
        assert_eq!(offered, Lang::OFFERED);
        assert_eq!(Lang::Ru.stage(), Stage::Draft);
        assert_eq!(Lang::It.stage(), Stage::Draft);
        assert!(!Lang::Xx.offered());
        let catalogued: Vec<Lang> = Lang::ALL.into_iter().filter(|l| *l != Lang::Xx).collect();
        assert_eq!(catalogued, Lang::CATALOGUED);
        for l in [Lang::Es, Lang::Ko, Lang::ZhHans, Lang::Id, Lang::Tl] {
            assert_eq!(l.stage(), Stage::Filling, "{l:?}");
        }
        assert_eq!(Lang::ZhHans.endonym(), "简体中文");
        assert_eq!(Lang::Ko.endonym(), "한국어");
    }

    #[test]
    fn fill_replaces_named_values_only() {
        let args = [("name", "Rabbit Hole"), ("n", "3")];
        assert_eq!(fill("Join “{name}”.", &args), "Join “Rabbit Hole”.");
        assert_eq!(fill("{n} of {n}", &args), "3 of 3");
        assert_eq!(fill("{{literal}} {missing}", &args), "{literal} {missing}");
        assert_eq!(fill("odd } brace {", &args), "odd } brace {");
    }

    #[test]
    fn current_language_is_set_and_read_back() {
        let before = lang();
        set_lang(Lang::Ja);
        assert_eq!(lang(), Lang::Ja);
        set_lang(before);
        assert_eq!(lang(), before);
    }
}
