// SPDX-License-Identifier: BSD-2-Clause
//! The engine-facing text system: the fallback chain built from installed families, measuring,
//! and the installed-font path on whatever machine runs the test.

#[path = "support/text.rs"]
mod text_support;

use text_support::font_dir;
use wolkit::text::layout::{LayoutOptions, TextStyle};
use wolkit::text::system::{Family, TextSystem, default_font_roots};

fn temp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("wolkit-sys-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn chain_is_built_from_installed_family_names() {
    let tmp = temp("chain");
    let mut sys = TextSystem::with_system_fonts(&[font_dir()], &tmp.join("idx"));
    let ui = sys
        .font(Family::Ui, 400, false)
        .expect("a UI face (Noto Sans is in the list)");
    let st = TextStyle::new(ui, 14.0);
    // Latin, Hebrew, Arabic and CJK all resolve to a real glyph through the chain.
    for text in [
        "Hello",
        "\u{5E9}\u{5DC}\u{5D5}\u{5DD}",
        "\u{645}\u{631}\u{62D}\u{628}\u{627}",
        "日本語",
    ] {
        let lay = sys.layout(text, &st, &LayoutOptions::default());
        assert!(lay.width > 0.0, "{text}");
        let ids: Vec<u32> = lay
            .lines
            .iter()
            .flat_map(|l| l.frags.iter())
            .flat_map(|f| f.run.glyphs.iter().map(|g| g.id))
            .collect();
        assert!(
            ids.iter().all(|&g| g != 0),
            "{text}: every character found a face, got {ids:?}"
        );
    }
    // A bold request finds the bold face.
    let bold = sys.font(Family::Ui, 700, false).unwrap();
    assert_ne!(bold, ui);
    // No monospace face is installed in the fixture set.
    assert!(sys.font(Family::Mono, 400, false).is_none());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn measure_grows_with_text_and_shrinks_the_line_with_a_width_limit() {
    let tmp = temp("measure");
    let mut sys = TextSystem::with_system_fonts(&[font_dir()], &tmp.join("idx"));
    let st = TextStyle::new(sys.font(Family::Ui, 400, false).unwrap(), 13.0);
    let (w1, h1) = sys.measure("Wonderland", &st, None);
    let (w2, _) = sys.measure("Wonderland Wonderland", &st, None);
    assert!(w2 > w1 * 1.8, "{w1} {w2}");
    let (w3, h3) = sys.measure("Wonderland Wonderland Wonderland", &st, Some(w1 + 4.0));
    assert!(w3 <= w1 + 4.5);
    assert!(
        h3 > h1 * 2.5,
        "three words wrap onto three lines: {h1} {h3}"
    );
    let (we, he) = sys.measure("", &st, None);
    assert_eq!(we, 0.0);
    assert!(he > 0.0, "an empty line still has a height");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn installed_fonts_work_whatever_this_machine_has() {
    // On a machine with no fonts this still must not panic; with fonts, Latin text measures.
    let tmp = temp("installed");
    let mut sys = TextSystem::with_system_fonts(
        &default_font_roots(|k| std::env::var_os(k)),
        &tmp.join("idx"),
    );
    let again = TextSystem::with_system_fonts(
        &default_font_roots(|k| std::env::var_os(k)),
        &tmp.join("idx"),
    );
    assert_eq!(
        sys.db.len(),
        again.db.len(),
        "the cached index gives the same set"
    );
    if let Some(ui) = sys.font(Family::Ui, 400, false) {
        let st = TextStyle::new(ui, 13.0);
        let (w, h) = sys.measure("Installed font check", &st, None);
        assert!(w > 50.0 && h > 8.0, "{w} {h}");
        let mut src = sys.glyph_source();
        let mut list = wolkit::paint::list::DisplayList::new();
        list.solid(
            wolkit::paint::geom::Rect::new(0.0, 0.0, 4.0, 4.0),
            wolkit::paint::geom::Color::rgb(1, 2, 3),
        );
        list.finish();
        let img = wolkit::cpu::raster::render(&list, &mut src, 8, 8, &Default::default());
        assert_eq!(img.pixel(1, 1), [1, 2, 3, 255]);
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn trim_clears_caches_and_text_still_works() {
    let tmp = temp("trim");
    let mut sys = TextSystem::with_system_fonts(&[font_dir()], &tmp.join("idx"));
    let st = TextStyle::new(sys.font(Family::Ui, 400, false).unwrap(), 13.0);
    sys.measure("warm up", &st, None);
    assert!(!sys.shapes.is_empty());
    sys.trim();
    assert!(sys.shapes.is_empty());
    let (w, _) = sys.measure("warm up", &st, None);
    assert!(w > 0.0);
    let _ = std::fs::remove_dir_all(&tmp);
}
