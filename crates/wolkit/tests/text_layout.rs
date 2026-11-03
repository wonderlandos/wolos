// SPDX-License-Identifier: BSD-2-Clause
//! Shaping, itemisation, bidi, wrapping and caret behaviour on the bundled fonts.

mod text_support;

use text_support::fonts;
use wolkit::text::cache::ShapeCache;
use wolkit::text::layout::{Align, BaseDir, LayoutOptions, TextStyle, layout_text};

#[test]
fn latin_line_has_width_and_one_line() {
    let f = fonts();
    let mut c = ShapeCache::new(64);
    let st = TextStyle::new(f.latin, 16.0);
    let l = layout_text(&f.db, &mut c, "Hello world", &st, &LayoutOptions::default());
    assert_eq!(l.lines.len(), 1);
    assert!(l.width > 60.0 && l.width < 140.0, "width {}", l.width);
    assert!(l.height > 16.0);
}

#[test]
fn wraps_at_word_boundaries() {
    let f = fonts();
    let mut c = ShapeCache::new(64);
    let st = TextStyle::new(f.latin, 16.0);
    let opts = LayoutOptions {
        max_width: 80.0,
        ..LayoutOptions::default()
    };
    let text = "The quick brown fox jumps over the lazy dog";
    let l = layout_text(&f.db, &mut c, text, &st, &opts);
    assert!(l.lines.len() >= 3, "{} lines", l.lines.len());
    for line in &l.lines {
        assert!(
            line.width <= 80.5,
            "line {:?} width {}",
            &text[line.range.clone()],
            line.width
        );
    }
    // Lines tile the text without gaps or overlap.
    let mut at = 0;
    for line in &l.lines {
        assert_eq!(line.range.start, at);
        at = line.range.end;
    }
    assert_eq!(at, text.len());
    // A break never falls inside a word.
    for line in &l.lines[..l.lines.len() - 1] {
        assert!(
            text[line.range.clone()].ends_with(' '),
            "{:?}",
            &text[line.range.clone()]
        );
    }
}

#[test]
fn long_word_breaks_between_clusters() {
    let f = fonts();
    let mut c = ShapeCache::new(64);
    let st = TextStyle::new(f.latin, 16.0);
    let opts = LayoutOptions {
        max_width: 50.0,
        ..LayoutOptions::default()
    };
    let l = layout_text(
        &f.db,
        &mut c,
        "Supercalifragilisticexpialidocious",
        &st,
        &opts,
    );
    assert!(l.lines.len() >= 3);
    assert!(l.lines.iter().all(|l| l.width <= 50.5));
}

#[test]
fn newline_makes_paragraphs() {
    let f = fonts();
    let mut c = ShapeCache::new(64);
    let st = TextStyle::new(f.latin, 14.0);
    let l = layout_text(
        &f.db,
        &mut c,
        "one\ntwo\n\nfour",
        &st,
        &LayoutOptions::default(),
    );
    assert_eq!(l.lines.len(), 4);
    assert_eq!(l.lines[2].range.len(), 0);
}

#[test]
fn rtl_text_is_shaped_right_to_left_and_aligned_start_right() {
    let f = fonts();
    let mut c = ShapeCache::new(64);
    let st = TextStyle::new(f.latin, 18.0);
    let text = "\u{645}\u{631}\u{62D}\u{628}\u{627}";
    let opts = LayoutOptions {
        max_width: 200.0,
        ..LayoutOptions::default()
    };
    let l = layout_text(&f.db, &mut c, text, &st, &opts);
    assert_eq!(l.lines.len(), 1);
    let line = &l.lines[0];
    assert!(line.rtl);
    assert!(line.frags[0].run.dir == wolkit::text::shape::Dir::Rtl);
    assert!(
        line.frags[0].run.font == f.arabic,
        "arabic falls back to the arabic face"
    );
    assert!(
        (line.x + line.width - 200.0).abs() < 0.6,
        "right aligned: x {} w {}",
        line.x,
        line.width
    );
}

#[test]
fn mixed_bidi_orders_runs_visually() {
    let f = fonts();
    let mut c = ShapeCache::new(64);
    let st = TextStyle::new(f.latin, 16.0);
    let text = "abc \u{5E9}\u{5DC}\u{5D5}\u{5DD} def";
    let opts = LayoutOptions {
        base: BaseDir::Ltr,
        ..LayoutOptions::default()
    };
    let l = layout_text(&f.db, &mut c, text, &st, &opts);
    let line = &l.lines[0];
    let mut dirs: Vec<_> = line.frags.iter().map(|fr| fr.run.dir).collect();
    dirs.dedup();
    use wolkit::text::shape::Dir::*;
    assert_eq!(dirs, vec![Ltr, Rtl, Ltr]);
    // x positions strictly increase, fragments abut.
    for w in line.frags.windows(2) {
        assert!((w[0].x + w[0].run.advance - w[1].x).abs() < 1e-3);
    }
}

#[test]
fn cjk_uses_the_cjk_face_and_breaks_between_ideographs() {
    let f = fonts();
    let mut c = ShapeCache::new(64);
    let st = TextStyle::new(f.latin, 16.0);
    let text = "你好，世界。日本語のテキスト";
    let wide = layout_text(&f.db, &mut c, text, &st, &LayoutOptions::default());
    assert!(wide.lines[0].frags.iter().all(|fr| fr.run.font == f.cjk));
    let opts = LayoutOptions {
        max_width: 64.0,
        ..LayoutOptions::default()
    };
    let l = layout_text(&f.db, &mut c, text, &st, &opts);
    assert!(l.lines.len() >= 3, "{} lines", l.lines.len());
    assert!(l.lines.iter().all(|l| l.width <= 64.5));
}

#[test]
fn caret_and_hit_test_roundtrip_ltr() {
    let f = fonts();
    let mut c = ShapeCache::new(64);
    let st = TextStyle::new(f.latin, 16.0);
    let text = "Hello world";
    let l = layout_text(&f.db, &mut c, text, &st, &LayoutOptions::default());
    for (i, _) in text.char_indices() {
        let (line, x) = l.caret(text, i);
        assert_eq!(line, 0);
        let back = l.hit_test(text, x + 0.01, 1.0);
        assert_eq!(back, i, "caret {i} at x={x}");
    }
    let (_, end) = l.caret(text, text.len());
    assert!((end - l.width).abs() < 0.01);
    assert_eq!(l.hit_test(text, 10_000.0, 1.0), text.len());
    assert_eq!(l.hit_test(text, -5.0, 1.0), 0);
}

#[test]
fn center_alignment_splits_the_slack() {
    let f = fonts();
    let mut c = ShapeCache::new(64);
    let st = TextStyle::new(f.latin, 16.0);
    let opts = LayoutOptions {
        max_width: 300.0,
        align: Align::Center,
        ..LayoutOptions::default()
    };
    let l = layout_text(&f.db, &mut c, "Hi", &st, &opts);
    let line = &l.lines[0];
    assert!((line.x - ((300.0 - line.width) / 2.0).floor()).abs() < 0.01);
}

#[test]
fn selection_rects_cover_the_range() {
    let f = fonts();
    let mut c = ShapeCache::new(64);
    let st = TextStyle::new(f.latin, 16.0);
    let text = "Hello world";
    let l = layout_text(&f.db, &mut c, text, &st, &LayoutOptions::default());
    let r = l.selection_rects(2..7);
    assert_eq!(r.len(), 1);
    let (_, x0) = l.caret(text, 2);
    let (_, x1) = l.caret(text, 7);
    assert!((r[0].x - x0).abs() < 0.01 && (r[0].right() - x1).abs() < 0.01);
}

// ---- robustness ---------------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn layout_survives_random_text_and_keeps_its_invariants() {
    let f = fonts();
    let mut c = ShapeCache::new(256);
    let st = TextStyle::new(f.latin, 15.0);
    let pool = [
        "a",
        "bb",
        " ",
        "  ",
        "\n",
        "\r\n",
        "\u{2029}",
        "é",
        "e\u{301}\u{301}\u{301}",
        "日",
        "本語",
        "、",
        "。",
        "\u{5E9}\u{5DC}",
        "\u{645}\u{631}",
        "\u{200D}",
        "\u{200B}",
        "\u{202E}",
        "\u{202A}",
        "\u{2066}",
        "🧑‍🚀",
        "\u{1F600}",
        "\u{0}",
        "\u{7}",
        "\u{FFFD}",
        "\u{10FFFF}",
        "x-y",
        "wide_word_without_any_break_opportunity",
        "\u{FE0F}",
        "\t",
        "\u{AD}",
    ];
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for case in 0..300 {
        let n = 1 + (rng.next() % 30) as usize;
        let text: String = (0..n)
            .map(|_| pool[(rng.next() % pool.len() as u64) as usize])
            .collect();
        let max_width = match rng.next() % 4 {
            0 => f32::INFINITY,
            1 => 1.0,
            2 => 60.0,
            _ => 200.0,
        };
        let base = match rng.next() % 3 {
            0 => BaseDir::Auto,
            1 => BaseDir::Ltr,
            _ => BaseDir::Rtl,
        };
        let opts = LayoutOptions {
            max_width,
            base,
            align: Align::Center,
            ..LayoutOptions::default()
        };
        let l = layout_text(&f.db, &mut c, &text, &st, &opts);
        assert!(!l.lines.is_empty(), "case {case}: {text:?}");
        assert!(l.width.is_finite() && l.height.is_finite());
        let mut prev_end = 0;
        for line in &l.lines {
            assert!(
                line.range.start >= prev_end,
                "case {case}: lines overlap in {text:?}"
            );
            assert!(line.range.end <= text.len());
            assert!(
                text.is_char_boundary(line.range.start) && text.is_char_boundary(line.range.end)
            );
            prev_end = line.range.end;
            assert!(line.baseline.is_finite() && line.x.is_finite() && line.width >= 0.0);
            for fr in &line.frags {
                assert!(fr.x.is_finite() && fr.run.advance.is_finite());
            }
        }
        // Carets and hit tests stay inside the text on every boundary.
        for (i, _) in text
            .char_indices()
            .chain(std::iter::once((text.len(), ' ')))
        {
            let (li, x) = l.caret(&text, i);
            assert!(
                li < l.lines.len() && x.is_finite(),
                "case {case}: caret {i} in {text:?}"
            );
        }
        let hit = l.hit_test(&text, 7.5, 3.0);
        assert!(
            hit <= text.len() && text.is_char_boundary(hit),
            "case {case}: hit {hit} in {text:?}"
        );
        for r in l.selection_rects(0..text.len()) {
            assert!(r.x.is_finite() && r.w >= 0.0);
        }
    }
}
