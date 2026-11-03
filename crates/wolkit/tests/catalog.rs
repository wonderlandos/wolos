// SPDX-License-Identifier: BSD-2-Clause
//! The text catalog through the C ABI (ADR 0045): tables compiled by `shared::i18n` are loaded
//! with `wolkit_catalog_load` and read back by key name, language and plural count, with the
//! fallback to English, the pseudo-locale, and refusal of bad handles, ids and tables.
use shared::i18n::Lang;
use shared::i18n::compile::{self, Unit};
use shared::i18n::tsv::{LangRow, SourceRow};
use std::ffi::c_char;
use std::ptr::{null, null_mut};
use wolkit::abi;
use wolkit::ffi::catalog::*;

fn src(key: &str, source: &str) -> SourceRow {
    SourceRow {
        key: key.into(),
        source: source.into(),
        comment: String::new(),
        budget: None,
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
        src("sound.title", "Sound"),
        src("sound.louder", "Lets volume sliders pass 100%."),
        src("files.count#one", "{n} file"),
        src("files.count#other", "{n} files"),
    ])
    .expect("unit")
}

fn table(l: Lang, rows: &[LangRow]) -> Vec<u8> {
    compile::compile(&unit(), l, rows).expect("compile").bytes
}

struct Cat(*mut wolkit_catalog);

impl Cat {
    fn load(tables: &[Vec<u8>]) -> Cat {
        let ptrs: Vec<*const u8> = tables.iter().map(|t| t.as_ptr()).collect();
        let lens: Vec<usize> = tables.iter().map(Vec::len).collect();
        let mut out = null_mut();
        let s = unsafe {
            wolkit_catalog_load(
                ptrs.as_ptr(),
                lens.as_ptr(),
                ptrs.len(),
                &mut out,
                null_mut(),
            )
        };
        assert_eq!(s, 0);
        Cat(out)
    }

    fn key(&self, name: &str) -> u32 {
        unsafe { wolkit_catalog_find(self.0, name.as_ptr().cast(), name.len()) }
    }

    fn text(&self, key: u32, lang: i32) -> (i32, String) {
        let (mut p, mut n): (*const c_char, usize) = (null(), 9);
        let s = unsafe { wolkit_catalog_text(self.0, key, lang, &mut p, &mut n) };
        (s, read(p, n))
    }

    fn plural(&self, key: u32, count: u64, lang: i32) -> String {
        let (mut p, mut n): (*const c_char, usize) = (null(), 9);
        let s = unsafe { wolkit_catalog_plural(self.0, key, count, lang, &mut p, &mut n) };
        assert_eq!(s, 0);
        read(p, n)
    }
}

// Releases the catalog reference the test loaded.
impl Drop for Cat {
    fn drop(&mut self) {
        unsafe { wolkit_catalog_release(self.0) };
    }
}

#[allow(clippy::disallowed_methods)] // a test reads what the C boundary hands out (clippy.toml)
fn read(p: *const c_char, n: usize) -> String {
    assert!(!p.is_null());
    // SAFETY: the catalog wrote `n` readable bytes at `p` and outlives this copy.
    String::from_utf8(unsafe { std::slice::from_raw_parts(p.cast::<u8>(), n) }.to_vec()).unwrap()
}

fn sample() -> Cat {
    let de = [
        tr("sound.title", "Ton"),
        tr("files.count#one", "{n} Datei"),
        tr("files.count#other", "{n} Dateien"),
    ];
    Cat::load(&[
        table(Lang::De, &de),
        table(Lang::En, &[]),
        table(Lang::Xx, &[]),
    ])
}

#[test]
fn a_key_reads_in_its_language_and_falls_back_to_english() {
    let c = sample();
    assert_eq!(unsafe { wolkit_catalog_count(c.0) }, 3);
    let title = c.key("sound.title");
    let louder = c.key("sound.louder");
    assert_ne!(title, abi::NO_KEY);
    assert_eq!(c.text(title, abi::LANG_EN), (0, "Sound".into()));
    assert_eq!(c.text(title, abi::LANG_DE), (0, "Ton".into()));
    // German has no sentence for `louder`: English answers.
    assert_eq!(
        c.text(louder, abi::LANG_DE).1,
        "Lets volume sliders pass 100%."
    );
    // A language with no table at all reads English too.
    assert_eq!(c.text(title, abi::LANG_KO).1, "Sound");
}

#[test]
fn the_pseudo_locale_lengthens_and_keeps_placeholders() {
    let c = sample();
    let (_, xx) = c.text(c.key("sound.title"), abi::LANG_XX);
    assert_ne!(xx, "Sound");
    assert!(xx.chars().count() > "Sound".len());
    let one = c.plural(c.key("files.count"), 1, abi::LANG_XX);
    assert!(one.contains("{n}"), "{one}");
}

#[test]
fn plural_forms_follow_the_count() {
    let c = sample();
    let files = c.key("files.count");
    assert_eq!(c.plural(files, 1, abi::LANG_EN), "{n} file");
    assert_eq!(c.plural(files, 3, abi::LANG_EN), "{n} files");
    assert_eq!(c.plural(files, 3, abi::LANG_DE), "{n} Dateien");
}

#[test]
fn the_current_language_is_the_one_set() {
    let c = sample();
    let title = c.key("sound.title");
    assert_eq!(wolkit_lang_get(), abi::LANG_EN);
    assert_eq!(wolkit_lang_set(abi::LANG_DE), 0);
    assert_eq!(c.text(title, abi::LANG_CURRENT).1, "Ton");
    assert_eq!(wolkit_lang_set(abi::LANG_CURRENT), 1);
    assert_eq!(wolkit_lang_set(99), 1);
    assert_eq!(wolkit_lang_set(abi::LANG_EN), 0);
    assert_eq!(c.text(title, abi::LANG_CURRENT).1, "Sound");
}

#[test]
fn bad_input_gives_empty_text_and_a_status_never_a_crash() {
    let c = sample();
    assert_eq!(c.key("no.such.key"), abi::NO_KEY);
    let (s, t) = c.text(77, abi::LANG_EN);
    assert_eq!((s, t.as_str()), (1, ""));
    assert_eq!(c.text(c.key("sound.title"), 99).0, 1);
    let (mut p, mut n): (*const c_char, usize) = (null(), 9);
    assert_eq!(
        unsafe { wolkit_catalog_text(null(), 0, 0, &mut p, &mut n) },
        1
    );
    assert_eq!((read(p, n).as_str(), n), ("", 0));
    assert_eq!(unsafe { wolkit_catalog_count(null()) }, 0);
}

#[test]
fn a_catalog_needs_exactly_one_english_table_that_parses() {
    let mut out = null_mut();
    let de = table(Lang::De, &[tr("sound.title", "Ton")]);
    let (p, l) = ([de.as_ptr()], [de.len()]);
    assert_eq!(
        unsafe { wolkit_catalog_load(p.as_ptr(), l.as_ptr(), 1, &mut out, null_mut()) },
        1
    );
    assert!(out.is_null());
    let junk = [0u8; 8];
    let (p, l) = ([junk.as_ptr()], [junk.len()]);
    assert_eq!(
        unsafe { wolkit_catalog_load(p.as_ptr(), l.as_ptr(), 1, &mut out, null_mut()) },
        1
    );
    assert_eq!(
        unsafe { wolkit_catalog_load(null(), null(), 0, &mut out, null_mut()) },
        1
    );
}

#[test]
fn a_directory_of_tables_loads_by_unit_name() {
    let d = std::env::temp_dir().join(format!("wolkit-catalog-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("demo.en.wtx"), table(Lang::En, &[])).unwrap();
    std::fs::write(
        d.join("demo.de.wtx"),
        table(Lang::De, &[tr("sound.title", "Ton")]),
    )
    .unwrap();
    let dir = d.to_str().unwrap();
    let mut out = null_mut();
    let s = unsafe {
        wolkit_catalog_load_dir(
            dir.as_ptr().cast(),
            dir.len(),
            c"demo".as_ptr(),
            4,
            &mut out,
            null_mut(),
        )
    };
    assert_eq!(s, 0);
    let c = Cat(out);
    assert_eq!(c.text(c.key("sound.title"), abi::LANG_DE).1, "Ton");
    let mut none = null_mut();
    let s = unsafe {
        wolkit_catalog_load_dir(
            dir.as_ptr().cast(),
            dir.len(),
            c"other".as_ptr(),
            5,
            &mut none,
            null_mut(),
        )
    };
    assert_eq!(s, 3);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn languages_parse_from_tags_and_locales() {
    let p = |s: &str| unsafe { wolkit_lang_parse(s.as_ptr().cast(), s.len()) };
    assert_eq!(p("pt-BR"), abi::LANG_PT_BR);
    assert_eq!(p("pt_PT.UTF-8"), abi::LANG_PT_BR);
    assert_eq!(p("zh_CN.UTF-8"), abi::LANG_ZH_HANS);
    assert_eq!(p("zh-Hans"), abi::LANG_ZH_HANS);
    assert_eq!(p("zh_TW.UTF-8"), -1);
    assert_eq!(p("it_IT.UTF-8"), -1);
    assert_eq!(p("xx"), abi::LANG_XX);
    let tag = wolkit_lang_tag(abi::LANG_ZH_HANS);
    assert_eq!(
        unsafe { std::ffi::CStr::from_ptr(tag) }.to_str().unwrap(),
        "zh-Hans"
    );
    assert!(wolkit_lang_tag(40).is_null());
}

#[test]
fn the_crystal_spec_tables_are_what_the_compiler_makes_of_their_tsv_files() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../wolos-crystal/shards/wolkit/spec/fixtures/i18n");
    for (lang, bytes, _) in shared::i18n::files::compile_dir(&root.join("demo")).expect("compile") {
        let name = format!("demo.{}.wtx", lang.tag());
        let have = std::fs::read(root.join("tables").join(&name)).unwrap_or_default();
        assert!(
            have == bytes,
            "{name} is out of date: cargo run -p shared --bin wol-i18n -- compile \
             wolos-crystal/shards/wolkit/spec/fixtures/i18n/demo wolos-crystal/shards/wolkit/spec/fixtures/i18n/tables"
        );
    }
}
