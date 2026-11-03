// SPDX-License-Identifier: BSD-2-Clause
//! The scene lowered to pixels: resolved styles and the token palette become display-list items,
//! text is shaped with real fonts both to measure and to draw, and the headless renderer shows it.
#[path = "support/text.rs"]
mod text_support;

use std::sync::Arc;
use wolkit::cpu::{Frame, render_frame};
use wolkit::env::Environment;
use wolkit::layout::LayoutCache;
use wolkit::paint::list::{DisplayList, Fill, Item};
use wolkit::paint::palette::Palette;
use wolkit::paint::scene::{Lower, build};
use wolkit::scene::patch::{APPEND, Patch, Scratch, Writer};
use wolkit::scene::prop::id;
use wolkit::scene::{Journal, Kind, Scene, SceneRead};
use wolkit::style::tokens::Token;
use wolkit::style::{Styles, Table, compile_sources, resolve};
use wolkit::text::measure::TextCache;
use wolkit::text::system::TextSystem;

const SHEET: &str = "
.card { background: card; padding: md; border-radius: m; box-shadow: sm; border: 1px line; }
.title { font-size: title; color: text; }
.hint { color: dim; }
.gone { display: none; background: danger; }
.clear { opacity: 0; background: danger; }
";

struct Win {
    scene: Scene,
    styles: Styles,
    layout: LayoutCache,
    table: Arc<Table>,
    env: Environment,
    text: TextSystem,
    cache: TextCache,
}

impl Win {
    fn new(f: impl FnOnce(&mut Writer, &dyn Fn(&str) -> u32)) -> Win {
        let c = compile_sources(&[("t.wolcss", SHEET)])
            .unwrap_or_else(|d| panic!("{}", wolkit::style::render_all(&d)));
        let table = Arc::new(Table::new(c.table));
        let mut w = Writer::new();
        let class = |n: &str| table.table.class_id(n).unwrap();
        f(&mut w, &class);
        let bytes = w.finish(1).to_vec();
        let mut scene = Scene::new();
        let p = Patch::parse(&bytes, &mut Scratch::default()).unwrap();
        scene.apply(&p, &mut Journal::default()).unwrap();
        Win {
            scene,
            styles: Styles::default(),
            layout: LayoutCache::new(),
            table,
            env: Environment::default(),
            text: TextSystem::from_db(text_support::fonts().db),
            cache: TextCache::new(),
        }
    }

    fn frame(&mut self, w: u32, h: u32) {
        resolve(
            &mut self.scene,
            Some(&self.table),
            &mut self.styles,
            &self.env,
            w,
        );
        wolkit::layout::scene::run_text(
            &mut self.scene,
            &self.styles,
            &mut self.layout,
            (&mut self.text, &mut self.cache),
            w as f32,
            h as f32,
        );
    }

    fn list(&mut self) -> DisplayList {
        let palette = Palette::for_env(&self.env);
        let mut list = DisplayList::new();
        build(
            &mut Lower {
                scene: &self.scene,
                styles: &self.styles,
                palette: &palette,
                text: &mut self.text,
                cache: &mut self.cache,
            },
            &mut list,
        );
        list
    }

    fn pixels(&mut self, w: u32, h: u32) -> Vec<u8> {
        let mut out = vec![0u8; (w * h * 4) as usize];
        let f = Frame {
            scene: &self.scene,
            styles: &self.styles,
            env: &self.env,
            width: w,
            height: h,
            scale: 1.0,
            canvases: None,
        };
        render_frame(&f, &mut self.text, &mut self.cache, &mut out);
        out
    }
}

/// A size in pixels, as the wire writes a `Dim`.
fn px(v: f32) -> [u32; 2] {
    [wolkit::scene::prop::DIM_PX, v.to_bits()]
}

fn card_with_title(w: &mut Writer, class: &dyn Fn(&str) -> u32) {
    w.create(1, Kind::Box).insert(0, 1, APPEND);
    w.set_style(1, &[class("card")]);
    w.create(2, Kind::Text).insert(1, 2, APPEND);
    w.set_style(2, &[class("title")]);
    w.set_text(2, "Hello");
}

#[test]
fn a_card_lowers_to_shadow_fill_border_and_shaped_text() {
    let mut win = Win::new(card_with_title);
    win.frame(320, 200);
    let pal = Palette::for_env(&win.env);
    let list = win.list();
    let items = list.chunk_items(&list.chunks[0]);
    let kinds: Vec<&str> = items
        .iter()
        .map(|i| match i {
            Item::Shadow { .. } => "shadow",
            Item::Rect { .. } => "rect",
            Item::Border { .. } => "border",
            Item::Glyphs { .. } => "glyphs",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["shadow", "rect", "border", "glyphs"], "{items:?}");
    assert!(
        matches!(items[1], Item::Rect { fill: Fill::Solid(c), .. } if c == pal.token(Token::Card))
    );
    let Item::Glyphs { color, glyphs, .. } = &items[3] else {
        unreachable!()
    };
    assert_eq!(*color, pal.token(Token::Text));
    assert_eq!(glyphs.len(), 5, "one glyph per letter of Hello");
}

#[test]
fn layout_measures_with_the_shaper_and_paint_reuses_the_paragraph() {
    let mut win = Win::new(card_with_title);
    win.frame(320, 200);
    let r = win.scene.rect(2);
    // The real measure: the title role is larger than the 0.5 em-per-letter stand-in guesses.
    let approx = wolkit::layout::tree::approx_text("Hello", 17.0, None);
    assert!(r.w > 0.0 && r.w != approx.w, "{r:?} vs {approx:?}");
    assert_eq!(r.w, r.w.ceil(), "measured widths are whole pixels");
    let misses = win.cache.misses;
    let _ = win.list();
    let _ = win.list();
    assert!(
        win.cache.misses <= misses + 1,
        "paint shapes at most once more"
    );
    assert!(win.cache.hits >= 1);
}

#[test]
fn hidden_and_fully_transparent_parts_draw_nothing() {
    let mut win = Win::new(|w, class| {
        w.create(1, Kind::Box).insert(0, 1, APPEND);
        w.set_style(1, &[class("gone")]);
        w.create(2, Kind::Box).insert(0, 2, APPEND);
        w.set_style(2, &[class("clear")]);
        w.set_prop(2, id::WIDTH, &px(0.0));
    });
    win.frame(100, 100);
    let list = win.list();
    assert!(list.items.is_empty(), "{:?}", list.items);
}

#[test]
fn an_overlay_is_its_own_chunk_above_the_window() {
    let mut win = Win::new(|w, class| {
        w.create(1, Kind::Box).insert(0, 1, APPEND);
        w.set_style(1, &[class("card")]);
        w.create(2, Kind::Overlay).insert(0, 2, APPEND);
        w.create(3, Kind::Box).insert(2, 3, APPEND);
        w.set_style(3, &[class("card")]);
    });
    win.frame(300, 300);
    let list = win.list();
    assert_eq!(list.chunks.len(), 2);
    assert_eq!((list.chunks[0].z, list.chunks[1].z), (0, 1));
    assert!(!list.chunk_items(&list.chunks[1]).is_empty());
}

#[test]
fn a_scroll_offset_moves_and_clips_the_content() {
    let mut win = Win::new(|w, class| {
        w.create(1, Kind::Scroll).insert(0, 1, APPEND);
        w.set_prop(1, id::HEIGHT, &px(50.0));
        w.set_prop(1, id::SCROLL_OFFSET, &[0f32.to_bits(), 30f32.to_bits()]);
        w.create(2, Kind::Box).insert(1, 2, APPEND);
        w.set_style(2, &[class("card")]);
        w.set_prop(2, id::HEIGHT, &px(200.0));
    });
    win.frame(200, 200);
    let laid = win.scene.rect(2);
    let list = win.list();
    let items = list.chunk_items(&list.chunks[0]);
    assert!(matches!(items[0], Item::PushClip { .. }), "{items:?}");
    let drawn = items
        .iter()
        .find_map(|i| match i {
            Item::Rect { rect, .. } => Some(*rect),
            _ => None,
        })
        .unwrap();
    assert_eq!(drawn.y, laid.y - 30.0);
    assert!(matches!(items.last(), Some(Item::PopClip)));
}

#[test]
fn the_background_is_the_themes_surface_and_cards_are_card() {
    for dark in [false, true] {
        let mut win = Win::new(|w, class| {
            w.create(1, Kind::Box).insert(0, 1, APPEND);
            w.set_style(1, &[class("card")]);
            w.set_prop(1, id::WIDTH, &px(40.0));
            w.set_prop(1, id::HEIGHT, &px(40.0));
        });
        win.env.dark = dark;
        win.frame(64, 64);
        let pal = Palette::for_env(&win.env);
        let px = win.pixels(64, 64);
        let at = |x: usize, y: usize| {
            let i = (y * 64 + x) * 4;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        let s = pal.background();
        assert_eq!(at(60, 60), [s.r, s.g, s.b, 255], "dark={dark}");
        let c = pal.token(Token::Card);
        assert_eq!(at(20, 20), [c.r, c.g, c.b, 255], "dark={dark}");
    }
}

#[test]
fn increase_contrast_draws_dim_text_as_text() {
    let mut win = Win::new(|w, class| {
        w.create(1, Kind::Text).insert(0, 1, APPEND);
        w.set_style(1, &[class("hint")]);
        w.set_text(1, "hint");
    });
    win.env.contrast = true;
    win.frame(200, 100);
    let pal = Palette::for_env(&win.env);
    let list = win.list();
    let color = list
        .items
        .iter()
        .find_map(|i| match i {
            Item::Glyphs { color, .. } => Some(*color),
            _ => None,
        })
        .unwrap();
    assert_eq!(color, pal.token(Token::Text));
    assert_ne!(color, pal.token(Token::Dim));
}
