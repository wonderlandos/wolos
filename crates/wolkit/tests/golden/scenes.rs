// SPDX-License-Identifier: BSD-2-Clause
//! The twelve golden scenes. Each builds a display list on the bundled fonts; the CPU rasteriser
//! renders it and the bytes are compared with `scenes/<name>.png`.

use wolkit::paint::geom::{Color, Rect};
use wolkit::paint::list::{DisplayList, Fill, Radii, Shadow};
use wolkit::paint::text::{push_layout, push_selection};
use wolkit::text::cache::ShapeCache;
use wolkit::text::edit::Editor;
use wolkit::text::glyphcache::GlyphCache;
use wolkit::text::layout::{Align, BaseDir, LayoutOptions, TextLayout, TextStyle, layout_text};

use crate::text_support::{Fonts, fonts};

pub const BG: Color = Color::rgb(243, 241, 248);
pub const INK: Color = Color::rgb(28, 28, 34);
pub const MUTED: Color = Color::rgb(100, 104, 120);
pub const ACCENT: Color = Color::rgb(124, 92, 255);
pub const LINE: Color = Color::rgb(214, 216, 226);
pub const WHITE: Color = Color::rgb(255, 255, 255);
pub const DANGER: Color = Color::rgb(214, 64, 64);

pub struct Ctx {
    pub f: Fonts,
    pub shapes: ShapeCache,
    pub glyphs: GlyphCache,
}

impl Ctx {
    pub fn new() -> Ctx {
        Ctx {
            f: fonts(),
            shapes: ShapeCache::new(512),
            glyphs: GlyphCache::default(),
        }
    }

    pub fn style(&self, size: f32) -> TextStyle {
        TextStyle::new(self.f.latin, size)
    }

    pub fn bold(&self, size: f32) -> TextStyle {
        TextStyle::new(self.f.latin_bold, size)
    }

    /// Lays text out and appends it with its top-left corner at `(x, y)`.
    pub fn text(
        &mut self,
        l: &mut DisplayList,
        s: &str,
        at: (f32, f32),
        st: &TextStyle,
        o: &LayoutOptions,
        c: Color,
    ) -> TextLayout {
        let lay = layout_text(&self.f.db, &mut self.shapes, s, st, o);
        push_layout(l, &lay, at.0, at.1, c);
        lay
    }

    pub fn line(
        &mut self,
        l: &mut DisplayList,
        s: &str,
        x: f32,
        y: f32,
        size: f32,
        c: Color,
    ) -> TextLayout {
        let st = self.style(size);
        self.text(l, s, (x, y), &st, &LayoutOptions::default(), c)
    }
}

pub struct Scene {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub build: fn(&mut Ctx, &mut DisplayList),
}

pub fn all() -> Vec<Scene> {
    vec![
        Scene {
            name: "01_boxes",
            width: 256,
            height: 160,
            scale: 1.0,
            build: boxes,
        },
        Scene {
            name: "02_gradients_clips",
            width: 256,
            height: 160,
            scale: 1.0,
            build: gradients_clips,
        },
        Scene {
            name: "03_shadows_cards",
            width: 256,
            height: 160,
            scale: 1.0,
            build: shadows_cards,
        },
        Scene {
            name: "04_text_latin",
            width: 320,
            height: 200,
            scale: 1.0,
            build: text_latin,
        },
        Scene {
            name: "05_text_bidi",
            width: 320,
            height: 200,
            scale: 1.0,
            build: text_bidi,
        },
        Scene {
            name: "06_text_cjk",
            width: 320,
            height: 160,
            scale: 1.0,
            build: text_cjk,
        },
        Scene {
            name: "07_text_wrap_align",
            width: 320,
            height: 200,
            scale: 1.0,
            build: text_wrap_align,
        },
        Scene {
            name: "08_text_input_ime",
            width: 320,
            height: 160,
            scale: 1.0,
            build: text_input_ime,
        },
        Scene {
            name: "09_ui_form",
            width: 320,
            height: 220,
            scale: 1.0,
            build: ui_form,
        },
        Scene {
            name: "10_list_scroll_chunks",
            width: 256,
            height: 200,
            scale: 1.0,
            build: list_scroll,
        },
        Scene {
            name: "11_hidpi_scale2",
            width: 320,
            height: 160,
            scale: 2.0,
            build: hidpi,
        },
        Scene {
            name: "12_layers_overlay",
            width: 256,
            height: 180,
            scale: 1.0,
            build: layers_overlap,
        },
    ]
}

fn boxes(_: &mut Ctx, l: &mut DisplayList) {
    for (i, r) in [0.0, 4.0, 8.0, 16.0].into_iter().enumerate() {
        l.rect(
            Rect::new(12.0 + i as f32 * 60.0, 12.0, 48.0, 48.0),
            Radii::uniform(r),
            Fill::Solid(ACCENT),
        );
    }
    // Fractional geometry: edges land inside pixels.
    l.rect(
        Rect::new(12.5, 72.25, 60.5, 30.75),
        Radii::uniform(6.0),
        Fill::Solid(Color::rgb(40, 160, 120)),
    );
    l.rect(
        Rect::new(80.0, 72.0, 50.0, 31.0),
        Radii([14.0, 2.0, 14.0, 2.0]),
        Fill::Solid(Color::rgb(230, 140, 40)),
    );
    l.border(
        Rect::new(140.0, 72.0, 50.0, 31.0),
        Radii::uniform(8.0),
        1.0,
        INK,
    );
    l.border(
        Rect::new(200.0, 72.0, 44.0, 31.0),
        Radii::uniform(15.5),
        3.0,
        DANGER,
    );
    // Translucent overlap.
    l.solid(
        Rect::new(20.0, 116.0, 60.0, 30.0),
        Color::rgba(255, 0, 0, 128),
    );
    l.solid(
        Rect::new(50.0, 124.0, 60.0, 30.0),
        Color::rgba(0, 0, 255, 128),
    );
    l.rect(
        Rect::new(130.0, 116.0, 110.0, 30.0),
        Radii::uniform(15.0),
        Fill::Solid(Color::rgba(124, 92, 255, 90)),
    );
    l.border(
        Rect::new(130.5, 116.5, 109.0, 29.0),
        Radii::uniform(14.5),
        1.0,
        Color::rgba(28, 28, 34, 180),
    );
}

fn gradients_clips(_: &mut Ctx, l: &mut DisplayList) {
    l.rect(
        Rect::new(10.0, 10.0, 110.0, 50.0),
        Radii::uniform(10.0),
        Fill::vertical(Color::rgb(255, 214, 120), Color::rgb(240, 90, 120)),
    );
    l.rect(
        Rect::new(130.0, 10.0, 116.0, 30.0),
        Radii::uniform(15.0),
        Fill::horizontal(Color::rgb(60, 120, 255), Color::rgb(160, 60, 255)),
    );
    l.rect(
        Rect::new(130.0, 46.0, 116.0, 30.0),
        Radii::uniform(4.0),
        Fill::Linear {
            from: WHITE,
            to: INK,
            p0: (0.0, 0.0),
            p1: (1.0, 1.0),
        },
    );
    // Rounded clip around oversized stripes.
    l.push_clip(Rect::new(10.0, 72.0, 110.0, 78.0), 14.0);
    for i in 0..12 {
        let c = if i % 2 == 0 {
            Color::rgb(40, 160, 120)
        } else {
            Color::rgb(250, 250, 250)
        };
        l.solid(Rect::new(-10.0 + i as f32 * 16.0, 60.0, 14.0, 110.0), c);
    }
    // A clip inside the clip, off pixel boundaries.
    l.push_clip(Rect::new(30.5, 90.25, 60.0, 40.0), 10.0);
    l.solid(
        Rect::new(0.0, 0.0, 256.0, 160.0),
        Color::rgba(20, 20, 60, 190),
    );
    l.pop_clip();
    l.pop_clip();
    l.push_clip(Rect::new(130.0, 90.0, 116.0, 60.0), 0.0);
    l.rect(
        Rect::new(120.0, 80.0, 90.0, 90.0),
        Radii::uniform(45.0),
        Fill::vertical(Color::rgb(255, 255, 255), Color::rgb(90, 90, 200)),
    );
    l.pop_clip();
}

fn shadows_cards(c: &mut Ctx, l: &mut DisplayList) {
    let card = Rect::new(20.0, 16.0, 130.0, 100.0);
    l.shadow(
        card,
        Radii::uniform(12.0),
        Shadow::new(24.0, 0.0, 0.0, 8.0, Color::rgba(20, 20, 60, 110)),
    );
    l.shadow(
        card,
        Radii::uniform(12.0),
        Shadow::new(4.0, 0.0, 0.0, 1.0, Color::rgba(20, 20, 40, 80)),
    );
    l.rect(card, Radii::uniform(12.0), Fill::Solid(WHITE));
    l.border(card, Radii::uniform(12.0), 1.0, LINE);
    let st = c.bold(15.0);
    c.text(
        l,
        "Card title",
        (36.0, 30.0),
        &st,
        &LayoutOptions::default(),
        INK,
    );
    c.line(l, "Soft shadows, nine", 36.0, 56.0, 12.0, MUTED);
    c.line(l, "slices on the GPU,", 36.0, 72.0, 12.0, MUTED);
    c.line(l, "a blur here.", 36.0, 88.0, 12.0, MUTED);
    let small = Rect::new(168.0, 36.0, 70.0, 40.0);
    l.shadow(
        small,
        Radii::uniform(8.0),
        Shadow::new(10.0, 2.0, 0.0, 3.0, Color::rgba(0, 0, 0, 90)),
    );
    l.rect(
        small,
        Radii::uniform(8.0),
        Fill::Solid(Color::rgb(250, 250, 255)),
    );
    let pill = Rect::new(168.0, 100.0, 80.0, 30.0);
    l.shadow(
        pill,
        Radii::uniform(15.0),
        Shadow::new(14.0, -2.0, 0.0, 6.0, Color::rgba(124, 92, 255, 120)),
    );
    l.rect(pill, Radii::uniform(15.0), Fill::Solid(ACCENT));
    let st = c.bold(13.0);
    let lay = layout_text(
        &c.f.db,
        &mut c.shapes,
        "Accent",
        &st,
        &LayoutOptions::default(),
    );
    push_layout(
        l,
        &lay,
        pill.x + (pill.w - lay.width) / 2.0,
        pill.y + (pill.h - lay.height) / 2.0,
        WHITE,
    );
    // Negative spread, offset to the left.
    let tile = Rect::new(20.0, 130.0, 100.0, 22.0);
    l.shadow(
        tile,
        Radii::uniform(5.0),
        Shadow::new(8.0, -3.0, -4.0, 4.0, Color::rgba(0, 0, 0, 100)),
    );
    l.rect(tile, Radii::uniform(5.0), Fill::Solid(WHITE));
}

fn text_latin(c: &mut Ctx, l: &mut DisplayList) {
    let mut y = 8.0;
    for (i, size) in [11.0f32, 13.0, 16.0, 20.0, 28.0].into_iter().enumerate() {
        let s = if size > 18.0 {
            "Hamburgefonstiv 01"
        } else {
            "Hamburgefonstiv 0123456789"
        };
        let lay = c.line(l, s, 10.0, y, size, if i % 2 == 0 { INK } else { MUTED });
        y += lay.height + 2.0;
    }
    let st = c.bold(14.0);
    let lay = c.text(
        l,
        "Bold weight fi ffl office",
        (10.0, y),
        &st,
        &LayoutOptions::default(),
        INK,
    );
    y += lay.height + 6.0;
    let st = c.style(13.0);
    let o = LayoutOptions {
        max_width: 300.0,
        ..LayoutOptions::default()
    };
    c.text(l, "The quick brown fox jumps over the lazy dog, and then it wraps onto the next line at a word boundary.", (10.0, y), &st, &o, INK);
    // Quarter-pixel positioned repeats of one letter.
    for i in 0..6 {
        let st = c.style(14.0);
        let lay = layout_text(&c.f.db, &mut c.shapes, "l", &st, &LayoutOptions::default());
        push_layout(
            l,
            &lay,
            250.0 + i as f32 * 10.25 + (i as f32) * 0.25,
            10.0,
            INK,
        );
    }
}

fn text_bidi(c: &mut Ctx, l: &mut DisplayList) {
    let st = c.style(15.0);
    let mixed = "Hello \u{5E9}\u{5DC}\u{5D5}\u{5DD} world \u{645}\u{631}\u{62D}\u{628}\u{627} 123";
    let ltr = LayoutOptions {
        max_width: 300.0,
        base: BaseDir::Ltr,
        ..LayoutOptions::default()
    };
    let rtl = LayoutOptions {
        max_width: 300.0,
        base: BaseDir::Rtl,
        ..LayoutOptions::default()
    };
    let mut y = 8.0;
    c.line(l, "LTR base", 10.0, y, 11.0, MUTED);
    y += 16.0;
    let lay = c.text(l, mixed, (10.0, y), &st, &ltr, INK);
    y += lay.height + 8.0;
    c.line(l, "RTL base", 10.0, y, 11.0, MUTED);
    y += 16.0;
    let lay = c.text(l, mixed, (10.0, y), &st, &rtl, INK);
    y += lay.height + 8.0;
    c.line(l, "Auto base, Arabic first", 10.0, y, 11.0, MUTED);
    y += 16.0;
    let auto = LayoutOptions {
        max_width: 300.0,
        ..LayoutOptions::default()
    };
    let lay = c.text(
        l,
        "\u{645}\u{631}\u{62D}\u{628}\u{627} Wonderland 42 \u{628}\u{627}",
        (10.0, y),
        &st,
        &auto,
        INK,
    );
    y += lay.height + 8.0;
    // A wrapped bidi paragraph.
    let narrow = LayoutOptions {
        max_width: 140.0,
        base: BaseDir::Rtl,
        ..LayoutOptions::default()
    };
    let st = c.style(13.0);
    let mixed = concat!(
        "\u{5E9}\u{5DC}\u{5D5}\u{5DD} \u{5E2}\u{5D5}\u{5DC}\u{5DD} and some English ",
        "\u{645}\u{631}\u{62D}\u{628}\u{627} \u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}",
    );
    c.text(l, mixed, (10.0, y), &st, &narrow, INK);
    l.border(
        Rect::new(9.5, y - 0.5, 141.0, 56.0),
        Radii::uniform(2.0),
        1.0,
        LINE,
    );
}

fn text_cjk(c: &mut Ctx, l: &mut DisplayList) {
    let st = c.style(15.0);
    let o = LayoutOptions {
        max_width: 200.0,
        ..LayoutOptions::default()
    };
    let lay = c.text(
        l,
        "你好，世界。日本語のテキスト こんにちは 漢字一二三四五六七八九十",
        (10.0, 10.0),
        &st,
        &o,
        INK,
    );
    l.border(
        Rect::new(9.5, 9.5, 201.0, lay.height + 1.0),
        Radii::uniform(2.0),
        1.0,
        LINE,
    );
    let st = c.style(13.0);
    c.text(
        l,
        "文本 mixed 日本語 text 12",
        (10.0, 80.0),
        &st,
        &LayoutOptions::default(),
        INK,
    );
    // U+1F600 has no glyph in any bundled face: the primary face's missing-glyph box is drawn.
    c.text(
        l,
        "tofu: \u{1F600} end",
        (10.0, 102.0),
        &st,
        &LayoutOptions::default(),
        DANGER,
    );
    let big = c.style(32.0);
    c.text(
        l,
        "日本語",
        (224.0, 10.0),
        &big,
        &LayoutOptions {
            max_width: 100.0,
            ..LayoutOptions::default()
        },
        ACCENT,
    );
}

fn text_wrap_align(c: &mut Ctx, l: &mut DisplayList) {
    let text = "Wonderland apps wrap long paragraphs at word boundaries and align them.";
    let st = c.style(12.0);
    for (i, a) in [Align::Start, Align::Center, Align::End]
        .into_iter()
        .enumerate()
    {
        let x = 8.0 + i as f32 * 104.0;
        let o = LayoutOptions {
            max_width: 96.0,
            align: a,
            ..LayoutOptions::default()
        };
        let lay = c.text(l, text, (x, 10.0), &st, &o, INK);
        l.border(
            Rect::new(x - 0.5, 9.5, 97.0, lay.height + 1.0),
            Radii::ZERO,
            1.0,
            LINE,
        );
    }
    // Selection and a caret in a wrapped paragraph.
    let o = LayoutOptions {
        max_width: 200.0,
        ..LayoutOptions::default()
    };
    let para = "Select some words across a line break to see the highlight.";
    let lay = layout_text(&c.f.db, &mut c.shapes, para, &st, &o);
    let (x0, y0) = (10.0, 110.0);
    push_selection(
        l,
        &lay.selection_rects(7..38),
        x0,
        y0,
        Color::rgba(124, 92, 255, 80),
    );
    push_layout(l, &lay, x0, y0, INK);
    let (li, cx) = lay.caret(para, 38);
    let line = &lay.lines[li];
    l.solid(Rect::new(x0 + cx, y0 + line.top, 1.0, line.height), ACCENT);
}

fn text_input_ime(c: &mut Ctx, l: &mut DisplayList) {
    let st = c.style(14.0);
    let o = LayoutOptions::default();
    let field = |l: &mut DisplayList, y: f32, focused: bool| {
        let r = Rect::new(10.0, y, 300.0, 30.0);
        l.rect(r, Radii::uniform(6.0), Fill::Solid(WHITE));
        l.border(
            r,
            Radii::uniform(6.0),
            if focused { 2.0 } else { 1.0 },
            if focused { ACCENT } else { LINE },
        );
    };
    // 1. Plain text with a caret.
    field(l, 10.0, true);
    let mut ed = Editor::new(true);
    ed.set_text("Wonderland 日本語");
    ed.set_selection(11, 11);
    let text = ed.display_text();
    let lay = layout_text(&c.f.db, &mut c.shapes, &text, &st, &o);
    push_layout(l, &lay, 18.0, 16.0, INK);
    let (_, cx) = lay.caret(&text, ed.display_caret());
    l.solid(Rect::new(18.0 + cx, 15.0, 1.0, 18.0), INK);
    // 2. Selection.
    field(l, 56.0, false);
    ed.set_selection(0, 6);
    let sel = ed.selection().range();
    push_selection(
        l,
        &lay.selection_rects(sel),
        18.0,
        62.0,
        Color::rgba(124, 92, 255, 90),
    );
    push_layout(l, &lay, 18.0, 62.0, INK);
    // 3. Composition: the preedit is underlined and the caret sits inside it.
    field(l, 102.0, true);
    ed.set_text("Hi ");
    ed.ime_enable();
    ed.ime_preedit_string("こんにちは", Some((9, 12)));
    ed.ime_done(1);
    let text = ed.display_text();
    let lay = layout_text(&c.f.db, &mut c.shapes, &text, &st, &o);
    push_layout(l, &lay, 18.0, 108.0, INK);
    if let Some(r) = ed.display_preedit_range() {
        for rc in lay.selection_rects(r) {
            l.solid(
                Rect::new(18.0 + rc.x, 108.0 + rc.bottom() - 3.0, rc.w, 1.0),
                INK,
            );
        }
    }
    let (_, cx) = lay.caret(&text, ed.display_caret());
    l.solid(Rect::new(18.0 + cx, 107.0, 1.0, 18.0), ACCENT);
}

fn ui_form(c: &mut Ctx, l: &mut DisplayList) {
    // Header.
    l.solid(Rect::new(0.0, 0.0, 320.0, 36.0), WHITE);
    l.solid(Rect::new(0.0, 36.0, 320.0, 1.0), LINE);
    let st = c.bold(15.0);
    c.text(
        l,
        "Settings",
        (14.0, 9.0),
        &st,
        &LayoutOptions::default(),
        INK,
    );
    // Buttons.
    let button = |c: &mut Ctx,
                  l: &mut DisplayList,
                  r: Rect,
                  label: &str,
                  fill: Option<Color>,
                  ink: Color| {
        match fill {
            Some(f) => l.rect(r, Radii::uniform(8.0), Fill::Solid(f)),
            None => {
                l.rect(r, Radii::uniform(8.0), Fill::Solid(WHITE));
                l.border(r, Radii::uniform(8.0), 1.0, LINE);
            }
        }
        let st = c.style(13.0);
        let lay = layout_text(
            &c.f.db,
            &mut c.shapes,
            label,
            &st,
            &LayoutOptions::default(),
        );
        push_layout(
            l,
            &lay,
            r.x + (r.w - lay.width) / 2.0,
            r.y + (r.h - lay.height) / 2.0,
            ink,
        );
    };
    button(
        c,
        l,
        Rect::new(14.0, 50.0, 90.0, 32.0),
        "Save",
        Some(ACCENT),
        WHITE,
    );
    button(
        c,
        l,
        Rect::new(112.0, 50.0, 90.0, 32.0),
        "Cancel",
        None,
        INK,
    );
    button(
        c,
        l,
        Rect::new(210.0, 50.0, 96.0, 32.0),
        "Delete",
        Some(DANGER.with_opacity(110)),
        WHITE,
    );
    // Toggles.
    for (i, on) in [true, false].into_iter().enumerate() {
        let y = 100.0 + i as f32 * 30.0;
        c.line(
            l,
            if on { "Notifications" } else { "Reduce motion" },
            14.0,
            y + 3.0,
            13.0,
            INK,
        );
        let track = Rect::new(250.0, y, 44.0, 24.0);
        l.rect(
            track,
            Radii::uniform(12.0),
            Fill::Solid(if on {
                ACCENT
            } else {
                Color::rgb(190, 192, 204)
            }),
        );
        let thumb = Rect::new(if on { 272.0 } else { 252.0 }, y + 2.0, 20.0, 20.0);
        l.shadow(
            thumb,
            Radii::uniform(10.0),
            Shadow::new(3.0, 0.0, 0.0, 1.0, Color::rgba(0, 0, 0, 80)),
        );
        l.rect(thumb, Radii::uniform(10.0), Fill::Solid(WHITE));
    }
    // Slider.
    c.line(l, "Volume 62%", 14.0, 163.0, 13.0, INK);
    l.rect(
        Rect::new(110.0, 168.0, 184.0, 4.0),
        Radii::uniform(2.0),
        Fill::Solid(Color::rgb(210, 212, 222)),
    );
    l.rect(
        Rect::new(110.0, 168.0, 114.0, 4.0),
        Radii::uniform(2.0),
        Fill::Solid(ACCENT),
    );
    let thumb = Rect::new(214.0, 160.0, 20.0, 20.0);
    l.shadow(
        thumb,
        Radii::uniform(10.0),
        Shadow::new(5.0, 0.0, 0.0, 2.0, Color::rgba(0, 0, 0, 90)),
    );
    l.rect(thumb, Radii::uniform(10.0), Fill::Solid(WHITE));
    l.border(thumb, Radii::uniform(10.0), 1.0, LINE);
    // Checkbox row.
    let boxr = Rect::new(14.0, 192.0, 18.0, 18.0);
    l.rect(boxr, Radii::uniform(4.0), Fill::Solid(ACCENT));
    l.solid(Rect::new(18.0, 200.0, 4.0, 2.0), WHITE);
    l.solid(Rect::new(21.0, 197.0, 2.0, 7.0), WHITE);
    c.line(l, "Keep me signed in", 40.0, 193.0, 13.0, INK);
}

fn list_scroll(c: &mut Ctx, l: &mut DisplayList) {
    // The scrolling content: chunk 2, below the header, scrolled by 37.5 px.
    l.begin_chunk(
        2,
        1,
        Rect::new(0.0, 0.0, 256.0, 160.0),
        (0.0, 40.0 - 37.5),
        0,
    );
    for i in 0..20 {
        let y = i as f32 * 28.0;
        let r = Rect::new(8.0, y, 226.0, 24.0);
        l.rect(
            r,
            Radii::uniform(6.0),
            Fill::Solid(if i % 2 == 0 {
                WHITE
            } else {
                Color::rgb(240, 241, 247)
            }),
        );
        c.line(
            l,
            &format!("Item {} \u{2014} 日本語 row", i + 1),
            18.0,
            y + 4.0,
            13.0,
            INK,
        );
    }
    // The header covers whatever scrolls beneath it.
    l.begin_chunk(1, 1, Rect::new(0.0, 0.0, 256.0, 40.0), (0.0, 0.0), 1);
    l.solid(Rect::new(0.0, 0.0, 256.0, 40.0), Color::rgb(255, 255, 255));
    l.solid(Rect::new(0.0, 39.0, 256.0, 1.0), LINE);
    let st = c.bold(15.0);
    c.text(
        l,
        "Scrolling list",
        (12.0, 11.0),
        &st,
        &LayoutOptions::default(),
        INK,
    );
    // A scrollbar thumb on top.
    l.begin_chunk(3, 1, Rect::new(0.0, 0.0, 256.0, 200.0), (0.0, 0.0), 2);
    l.rect(
        Rect::new(244.0, 52.0, 5.0, 60.0),
        Radii::uniform(2.5),
        Fill::Solid(Color::rgba(28, 28, 34, 110)),
    );
}

fn hidpi(c: &mut Ctx, l: &mut DisplayList) {
    // Logical 160 x 80 at scale 2.
    let panel = Rect::new(8.0, 8.0, 144.0, 64.0);
    l.shadow(
        panel,
        Radii::uniform(8.0),
        Shadow::new(8.0, 0.0, 0.0, 2.0, Color::rgba(0, 0, 0, 50)),
    );
    l.rect(panel, Radii::uniform(8.0), Fill::Solid(WHITE));
    l.border(panel, Radii::uniform(8.0), 1.0, LINE);
    let st = c.bold(13.0);
    c.text(
        l,
        "HiDPI panel",
        (18.0, 16.0),
        &st,
        &LayoutOptions::default(),
        INK,
    );
    c.line(l, "Text 日本語 at 2x scale", 18.0, 36.0, 11.0, MUTED);
    l.rect(
        Rect::new(18.0, 54.0, 60.0, 12.0),
        Radii::uniform(6.0),
        Fill::Solid(ACCENT),
    );
    l.border(
        Rect::new(84.5, 54.5, 59.0, 11.0),
        Radii::uniform(5.5),
        1.0,
        INK,
    );
}

fn layers_overlap(c: &mut Ctx, l: &mut DisplayList) {
    // A group whose children overlap: composed as one layer at 50%.
    l.push_opacity(128, true);
    l.rect(
        Rect::new(14.0, 14.0, 80.0, 60.0),
        Radii::uniform(10.0),
        Fill::Solid(ACCENT),
    );
    l.rect(
        Rect::new(54.0, 34.0, 80.0, 60.0),
        Radii::uniform(10.0),
        Fill::Solid(Color::rgb(40, 160, 120)),
    );
    c.line(l, "Layer", 30.0, 40.0, 14.0, WHITE);
    l.pop_opacity();
    // The same two rectangles pushed down to the leaves: the overlap shows through.
    l.push_opacity(128, false);
    l.rect(
        Rect::new(14.0, 100.0, 80.0, 60.0),
        Radii::uniform(10.0),
        Fill::Solid(ACCENT),
    );
    l.rect(
        Rect::new(54.0, 120.0, 80.0, 50.0),
        Radii::uniform(10.0),
        Fill::Solid(Color::rgb(40, 160, 120)),
    );
    l.pop_opacity();
    // An overlay menu above everything, with its own shadow.
    l.begin_chunk(9, 1, Rect::new(0.0, 0.0, 256.0, 180.0), (150.0, 30.0), 5);
    let menu = Rect::new(0.0, 0.0, 96.0, 104.0);
    l.shadow(
        menu,
        Radii::uniform(10.0),
        Shadow::new(20.0, 0.0, 0.0, 8.0, Color::rgba(0, 0, 0, 80)),
    );
    l.rect(menu, Radii::uniform(10.0), Fill::Solid(WHITE));
    l.border(menu, Radii::uniform(10.0), 1.0, LINE);
    l.push_clip(menu, 10.0);
    l.solid(
        Rect::new(0.0, 38.0, 96.0, 28.0),
        Color::rgba(124, 92, 255, 40),
    );
    l.pop_clip();
    for (i, s) in ["Open", "Rename", "Share", "Delete"]
        .into_iter()
        .enumerate()
    {
        c.line(
            l,
            s,
            14.0,
            10.0 + i as f32 * 28.0,
            13.0,
            if i == 3 { DANGER } else { INK },
        );
    }
}
