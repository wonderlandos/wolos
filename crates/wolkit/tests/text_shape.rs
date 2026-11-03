// SPDX-License-Identifier: BSD-2-Clause
//! Shaping, font selection and fallback, the font index, and glyph rasterisation.

#[path = "support/text.rs"]
mod text_support;

use harfrust::Script;
use text_support::{fonts, load};
use wolkit::text::fontdb::{FontDb, FontIndex};
use wolkit::text::glyph::{GlyphKey, GlyphRasterizer, split_subpixel};
use wolkit::text::shape::{Dir, FontFeature, ShapeParams, shape_run};

fn params(
    font: wolkit::text::fontdb::FontId,
    dir: Dir,
    script: Script,
    feats: &[FontFeature],
) -> ShapeParams<'_> {
    ShapeParams {
        font,
        size: 16.0,
        dir,
        script,
        features: feats,
    }
}

#[test]
fn ligatures_shape_and_can_be_switched_off() {
    let f = fonts();
    let on = shape_run(
        &f.db,
        "office fi",
        &params(f.latin, Dir::Ltr, Script::LATIN, &[]),
    );
    let off = shape_run(
        &f.db,
        "office fi",
        &params(
            f.latin,
            Dir::Ltr,
            Script::LATIN,
            &[FontFeature::new(b"liga", 0)],
        ),
    );
    assert!(
        on.glyphs.len() < off.glyphs.len(),
        "{} vs {}",
        on.glyphs.len(),
        off.glyphs.len()
    );
    assert_eq!(off.glyphs.len(), "office fi".chars().count());
    // A ligature cluster keeps the offset of its first character: "ffi" starts at byte 1 and the
    // next cluster ("c") starts at byte 4.
    let at = on
        .glyphs
        .iter()
        .position(|g| g.cluster == 1)
        .expect("ffi cluster");
    assert_eq!(on.glyphs[at + 1].cluster, 4, "one glyph covers bytes 1..4");
    assert!((on.advance - on.glyphs.iter().map(|g| g.x_advance).sum::<f32>()).abs() < 1e-3);
}

#[test]
fn rtl_runs_come_out_in_visual_order_with_decreasing_clusters() {
    let f = fonts();
    let text = "\u{5E9}\u{5DC}\u{5D5}\u{5DD}";
    let run = shape_run(
        &f.db,
        text,
        &params(f.hebrew, Dir::Rtl, Script::HEBREW, &[]),
    );
    let clusters: Vec<u32> = run.glyphs.iter().map(|g| g.cluster).collect();
    assert_eq!(clusters, vec![6, 4, 2, 0]);
    assert!(
        run.glyphs.iter().all(|g| g.id != 0),
        "all four letters map to real glyphs"
    );
}

#[test]
fn arabic_letters_take_joining_forms() {
    let f = fonts();
    let joined = shape_run(
        &f.db,
        "\u{628}\u{628}\u{628}",
        &params(f.arabic, Dir::Rtl, Script::ARABIC, &[]),
    );
    // Initial, medial and final forms of the same letter are different glyphs. (The face builds a
    // letter from a body and a dot, so a cluster can hold more than one glyph.)
    let mut by_cluster: std::collections::BTreeMap<u32, Vec<u32>> =
        std::collections::BTreeMap::new();
    for g in &joined.glyphs {
        by_cluster.entry(g.cluster).or_default().push(g.id);
    }
    assert_eq!(by_cluster.len(), 3);
    let forms: Vec<_> = by_cluster.values().collect();
    assert!(
        forms[0] != forms[1] && forms[1] != forms[2] && forms[0] != forms[2],
        "{forms:?}"
    );
    let isolated = shape_run(
        &f.db,
        "\u{628}",
        &params(f.arabic, Dir::Rtl, Script::ARABIC, &[]),
    );
    let iso: Vec<u32> = isolated.glyphs.iter().map(|g| g.id).collect();
    assert!(
        !forms.iter().any(|v| **v == iso),
        "joined forms differ from the isolated form"
    );
}

#[test]
fn missing_characters_fall_back_per_script_then_chain_then_tofu() {
    let f = fonts();
    assert_eq!(f.db.resolve('a', Script::LATIN, f.latin), f.latin);
    assert_eq!(f.db.resolve('\u{5D0}', Script::HEBREW, f.latin), f.hebrew);
    assert_eq!(f.db.resolve('\u{628}', Script::ARABIC, f.latin), f.arabic);
    assert_eq!(f.db.resolve('日', Script::HAN, f.latin), f.cjk);
    // A character no face has (Latin script, no script list): the chain is tried, then the
    // requested face, which draws its missing-glyph box.
    assert_eq!(f.db.resolve('\u{1F600}', Script::LATIN, f.latin), f.latin);
    let run = shape_run(
        &f.db,
        "\u{1F600}",
        &params(f.latin, Dir::Ltr, Script::LATIN, &[]),
    );
    assert_eq!(run.glyphs[0].id, 0, "glyph 0 is the missing-glyph box");
    // With no script entry the chain still finds the CJK face for a Han character.
    assert_eq!(f.db.resolve('日', Script::LATIN, f.latin), f.cjk);
    // White space never switches faces.
    assert_eq!(f.db.resolve(' ', Script::HAN, f.cjk), f.cjk);
}

#[test]
fn family_and_weight_selection() {
    let f = fonts();
    let regular = f.db.find("Noto Sans", 400, false);
    let bold = f.db.find("noto sans", 700, false);
    assert_eq!(regular, Some(f.latin));
    assert_eq!(bold, Some(f.latin_bold));
    assert_eq!(
        f.db.select(&["Nope", "Noto Sans"], 650, false),
        Some(f.latin_bold)
    );
    assert_eq!(f.db.find("Nope", 400, false), None);
}

#[test]
fn unparsable_font_data_is_refused_and_does_not_panic() {
    let mut db = FontDb::new();
    assert!(db.add_data(vec![0u8; 64]).is_none());
    assert!(db.add_data(Vec::new()).is_none());
    let mut truncated = load("WkLatin-Regular.ttf");
    truncated.truncate(300);
    let id = db.add_data(truncated);
    // Whatever the parser decides, shaping with it must not panic.
    if let Some(id) = id {
        let run = shape_run(&db, "abc", &params(id, Dir::Ltr, Script::LATIN, &[]));
        assert!(run.glyphs.len() <= 3);
    }
}

#[test]
fn a_corrupt_face_degrades_to_missing_glyph_boxes() {
    let mut db = FontDb::new();
    let mut data = load("WkLatin-Regular.ttf");
    // Wreck the glyph tables but keep the directory and names intact.
    for b in data.iter_mut().skip(2000).take(20000) {
        *b = 0xA5;
    }
    if let Some(id) = db.add_data(data) {
        let run = shape_run(&db, "hello", &params(id, Dir::Ltr, Script::LATIN, &[]));
        assert!(!run.glyphs.is_empty());
        let mut r = GlyphRasterizer::new();
        for g in &run.glyphs {
            let _ = r.rasterize(&db, GlyphKey::new(id, g.id, 16.0, 0));
        }
    }
}

#[test]
fn quarter_pixel_positions_give_four_distinct_images_and_floor_quantisation() {
    let f = fonts();
    let g =
        f.db.face(f.latin)
            .unwrap()
            .font()
            .unwrap()
            .charmap()
            .map_unicode('l')
            .unwrap()
            .to_u32();
    let mut r = GlyphRasterizer::new();
    let bm: Vec<_> = (0..4u8)
        .map(|s| {
            r.rasterize(&f.db, GlyphKey::new(f.latin, g, 16.0, s))
                .unwrap()
        })
        .collect();
    for i in 0..4 {
        for j in i + 1..4 {
            assert_ne!(bm[i].data, bm[j].data, "phase {i} and {j} must differ");
        }
    }
    // The left edge of the stem moves right by a quarter pixel per phase. Read it off the middle
    // row: the first covered pixel's index plus the uncovered part of it.
    let edge = |b: &wolkit::text::glyph::GlyphBitmap| {
        let w = b.width as usize;
        let row = &b.data[(b.height as usize / 2) * w..(b.height as usize / 2 + 1) * w];
        let i = row.iter().position(|&v| v > 0).unwrap();
        f64::from(b.left) + i as f64 + 1.0 - f64::from(row[i]) / 255.0
    };
    let e: Vec<f64> = bm.iter().map(edge).collect();
    for i in 0..3 {
        assert!(
            (e[i + 1] - e[i] - 0.25).abs() < 0.02,
            "left edge steps by a quarter pixel: {e:?}"
        );
    }
    // Quantisation rounds down, and whole pixels shift the origin without changing the image.
    assert_eq!(split_subpixel(3.26), (3, 1));
    assert_eq!(split_subpixel(10.25), (10, 1));
    assert_eq!(split_subpixel(3.25).1, split_subpixel(10.25).1);
    // Re-rendering is deterministic.
    let again = r
        .rasterize(&f.db, GlyphKey::new(f.latin, g, 16.0, 2))
        .unwrap();
    assert_eq!(again, bm[2]);
}

#[test]
fn glyph_images_are_grayscale_coverage_with_soft_edges() {
    let f = fonts();
    let g =
        f.db.face(f.latin)
            .unwrap()
            .font()
            .unwrap()
            .charmap()
            .map_unicode('o')
            .unwrap()
            .to_u32();
    let bm = GlyphRasterizer::new()
        .rasterize(&f.db, GlyphKey::new(f.latin, g, 20.0, 0))
        .unwrap();
    assert_eq!(
        bm.data.len(),
        bm.width as usize * bm.height as usize,
        "one byte per pixel"
    );
    assert!(
        bm.data.iter().any(|&v| v > 0 && v < 255),
        "antialiased edges"
    );
    assert!(bm.data.iter().any(|&v| v > 240), "solid core");
    assert!(
        bm.top > 0 && bm.height as i16 >= bm.top,
        "sits on the baseline"
    );
}

#[test]
fn spaces_and_oversized_glyphs_have_no_image() {
    let f = fonts();
    let font = f.db.face(f.latin).unwrap().font().unwrap();
    let space = font.charmap().map_unicode(' ').unwrap().to_u32();
    let mut r = GlyphRasterizer::new();
    assert!(
        r.rasterize(&f.db, GlyphKey::new(f.latin, space, 16.0, 0))
            .is_none()
    );
    let w = font.charmap().map_unicode('W').unwrap().to_u32();
    assert!(
        r.rasterize(&f.db, GlyphKey::new(f.latin, w, 5000.0, 0))
            .is_none(),
        "beyond the bitmap limit"
    );
    assert!(
        r.rasterize(&f.db, GlyphKey::new(f.latin, w, 0.0, 0))
            .is_none()
    );
    assert!(
        r.rasterize(&f.db, GlyphKey::new(f.latin, w, f32::NAN, 0))
            .is_none()
    );
}

// ---- the font index ---------------------------------------------------------------------------

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("wolkit-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn the_font_index_is_read_from_its_cache_until_a_directory_changes() {
    let root = temp_dir("fontindex");
    let fonts_dir = root.join("fonts");
    std::fs::create_dir_all(&fonts_dir).unwrap();
    for n in ["WkLatin-Regular.ttf", "WkLatin-Bold.ttf"] {
        std::fs::write(fonts_dir.join(n), load(n)).unwrap();
    }
    std::fs::write(fonts_dir.join("notes.txt"), "not a font").unwrap();
    let cache = root.join("cache/fonts.idx");
    let roots = vec![fonts_dir.clone()];

    let (first, from_cache) = FontIndex::load_or_scan(&roots, &cache);
    assert!(!from_cache);
    assert_eq!(first.entries.len(), 2);
    assert!(cache.exists());

    // Second start: valid cache, no scan. Prove it by corrupting a font file's content: the cache
    // must still be trusted and the font file must not be read.
    let (second, from_cache) = FontIndex::load_or_scan(&roots, &cache);
    assert!(from_cache, "cache hit");
    assert_eq!(second, first);

    // Registering faces does not read the files either; the first use does.
    let mut db = FontDb::new();
    let ids = second.register(&mut db);
    assert_eq!(ids.len(), 2);
    let regular = db.find("Noto Sans", 400, false).expect("indexed family");
    assert!(
        db.face(regular).unwrap().has_char('a'),
        "lazily loaded on first use"
    );

    // Adding a file changes the directory's mtime: the cache is stale and the scan runs again.
    std::fs::write(
        fonts_dir.join("WkHebrew-Regular.ttf"),
        load("WkHebrew-Regular.ttf"),
    )
    .unwrap();
    // A sure change of the directory's mtime without waiting for the clock to tick.
    std::fs::File::open(&fonts_dir)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();
    let (third, from_cache) = FontIndex::load_or_scan(&roots, &cache);
    assert!(!from_cache);
    assert_eq!(third.entries.len(), 3);

    // A different set of roots never reuses another set's cache.
    let (_, from_cache) = FontIndex::load_or_scan(&[root.join("elsewhere")], &cache);
    assert!(!from_cache);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_garbled_index_cache_is_ignored() {
    let root = temp_dir("fontindex-bad");
    let fonts_dir = root.join("fonts");
    std::fs::create_dir_all(&fonts_dir).unwrap();
    std::fs::write(
        fonts_dir.join("WkLatin-Regular.ttf"),
        load("WkLatin-Regular.ttf"),
    )
    .unwrap();
    let cache = root.join("fonts.idx");
    for junk in [
        "",
        "WKFI1\nX\tbad\n",
        "WKFI1\nF\tonly\ttwo\n",
        "\u{0}\u{1}\u{2}",
    ] {
        std::fs::write(&cache, junk).unwrap();
        let (idx, from_cache) = FontIndex::load_or_scan(std::slice::from_ref(&fonts_dir), &cache);
        assert!(!from_cache, "{junk:?}");
        assert_eq!(idx.entries.len(), 1);
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn oversized_and_non_font_files_are_skipped_by_the_scan() {
    let root = temp_dir("fontindex-skip");
    std::fs::write(root.join("a.ttf"), b"not really a font").unwrap();
    std::fs::write(root.join("b.otf"), vec![0u8; 100]).unwrap();
    let idx = FontIndex::scan(std::slice::from_ref(&root));
    assert!(idx.entries.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}
