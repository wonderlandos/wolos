// SPDX-License-Identifier: BSD-2-Clause
//! The golden font set of `crates/wolkit/tests/support/text.rs`, read from wolkit's own folder
//! (the scenes in `crates/wolkit/tests/golden/scenes.rs` are shared, not copied).

#![allow(dead_code)] // the font set is read in part by each test file

use std::path::PathBuf;

use harfrust::Script;
use wolkit::text::fontdb::{FontDb, FontId};

pub struct Fonts {
    pub db: FontDb,
    pub latin: FontId,
    pub latin_bold: FontId,
    pub arabic: FontId,
    pub hebrew: FontId,
    pub cjk: FontId,
}

pub fn font_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../wolkit/tests/golden/fonts")
}

pub fn load(name: &str) -> Vec<u8> {
    std::fs::read(font_dir().join(name)).unwrap_or_else(|e| panic!("test font {name}: {e}"))
}

pub fn fonts() -> Fonts {
    let mut db = FontDb::new();
    let latin = db.add_data(load("WkLatin-Regular.ttf")).expect("latin");
    let latin_bold = db.add_data(load("WkLatin-Bold.ttf")).expect("latin bold");
    let arabic = db.add_data(load("WkArabic-Regular.ttf")).expect("arabic");
    let hebrew = db.add_data(load("WkHebrew-Regular.ttf")).expect("hebrew");
    let cjk = db.add_data(load("WkCJK-Regular.ttf")).expect("cjk");
    db.set_script_fallback(Script::ARABIC, &[arabic]);
    db.set_script_fallback(Script::HEBREW, &[hebrew]);
    db.set_script_fallback(Script::HAN, &[cjk]);
    db.set_script_fallback(Script::HIRAGANA, &[cjk]);
    db.set_script_fallback(Script::KATAKANA, &[cjk]);
    db.set_chain(&[latin, arabic, hebrew, cjk]);
    Fonts {
        db,
        latin,
        latin_bold,
        arabic,
        hebrew,
        cjk,
    }
}
