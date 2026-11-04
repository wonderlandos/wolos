// SPDX-License-Identifier: BSD-2-Clause
//! Style and layout over the real scene: classes, inline properties, state, media, the table
//! swap, and the dirty bits that tell later stages what to redo.
use std::sync::Arc;
use wolkit::env::Environment;
use wolkit::layout::LayoutCache;
use wolkit::scene::patch::{APPEND, Patch, Scratch, Writer};
use wolkit::scene::prop::{self, id};
use wolkit::scene::{Journal, Kind, Scene, SceneRead, SceneWrite, dirty, state};
use wolkit::style::tokens::Token;
use wolkit::style::value::*;
use wolkit::style::{Styles, Table, compile_sources, resolve};

struct Pipe {
    scene: Scene,
    journal: Journal,
    styles: Styles,
    cache: LayoutCache,
    table: Option<Arc<Table>>,
    env: Environment,
    seq: u32,
}

impl Pipe {
    fn new() -> Pipe {
        Pipe {
            scene: Scene::new(),
            journal: Journal::default(),
            styles: Styles::default(),
            cache: LayoutCache::new(),
            table: None,
            env: Environment::default(),
            seq: 0,
        }
    }
    fn with_sheet(src: &str) -> Pipe {
        let mut p = Pipe::new();
        p.set_sheet(src);
        p
    }
    fn set_sheet(&mut self, src: &str) {
        let c = compile_sources(&[("t.wolcss", src)])
            .unwrap_or_else(|d| panic!("{}", wolkit::style::render_all(&d)));
        self.table = Some(Arc::new(Table::new(c.table)));
    }
    fn class(&self, name: &str) -> u32 {
        self.table.as_ref().unwrap().table.class_id(name).unwrap()
    }
    fn patch(&mut self, f: impl FnOnce(&mut Writer)) {
        let mut w = Writer::new();
        f(&mut w);
        self.seq += 1;
        let bytes = w.finish(self.seq).to_vec();
        let p = Patch::parse(&bytes, &mut Scratch::default()).unwrap();
        self.scene.apply(&p, &mut self.journal).unwrap();
    }
    fn frame(&mut self, w: u32, h: u32) {
        resolve(
            &mut self.scene,
            self.table.as_deref(),
            &mut self.styles,
            &self.env,
            w,
        );
        wolkit::layout::scene::run(
            &mut self.scene,
            &self.styles,
            &mut self.cache,
            None,
            w as f32,
            h as f32,
        );
    }
    fn computed(&self, node: u32) -> &Props {
        self.styles.get(self.scene.computed(node))
    }
}

const SHEET: &str = "
.card { background: card; padding: md; border-radius: m; }
.card:hover { background: control; }
.big { width: 200px; height: 80px; }
.row:hover > .actions { opacity: 1; }
.actions { opacity: 0; }
@dark { .card { background: sidebar; } }
.label { color: dim; font-size: secondary; }
";

#[test]
fn a_class_gives_a_node_its_style() {
    let mut p = Pipe::with_sheet(SHEET);
    let card = p.class("card");
    p.patch(|w| {
        w.create(1, Kind::Box)
            .insert(0, 1, APPEND)
            .set_style(1, &[card]);
    });
    p.frame(400, 300);
    let c = p.computed(1);
    assert_eq!(c.layout.padding, [12.0; 4]);
    assert_eq!(
        c.paint.background,
        Paint::Color(ColorRef::Token(Token::Card))
    );
    assert_eq!(c.paint.radius, [12.0; 4]);
    // Nothing is left dirty for style, and the layout bit is cleared by the layout stage.
    assert_eq!(p.scene.dirty(1) & (dirty::STYLE | dirty::LAYOUT), 0);
}

#[test]
fn inline_properties_beat_classes_and_inherit_down() {
    let mut p = Pipe::with_sheet(SHEET);
    let card = p.class("card");
    p.patch(|w| {
        w.create(1, Kind::Box)
            .insert(0, 1, APPEND)
            .set_style(1, &[card]);
        w.set_f32(1, id::PADDING, 4.0);
        w.set_prop(1, id::COLOR, &[Token::Danger as u32]);
        w.create(2, Kind::Text)
            .insert(1, 2, APPEND)
            .set_text(2, "hi");
    });
    p.frame(400, 300);
    assert_eq!(
        p.computed(1).layout.padding,
        [4.0; 4],
        "inline wins over the class"
    );
    assert_eq!(
        p.computed(1).paint.background,
        Paint::Color(ColorRef::Token(Token::Card)),
        "the class still applies elsewhere"
    );
    assert_eq!(
        p.computed(2).text.color,
        ColorRef::Token(Token::Danger),
        "color is inherited"
    );
    assert_eq!(p.computed(2).layout.padding, [0.0; 4], "padding is not");
}

#[test]
fn later_inline_properties_win_and_sides_compose() {
    let mut p = Pipe::new();
    p.patch(|w| {
        w.create(1, Kind::Box).insert(0, 1, APPEND);
        w.set_f32(1, id::PADDING, 8.0);
        w.set_f32(1, id::PADDING_LEFT, 20.0);
        w.set_prop(
            1,
            id::MARGIN,
            &[wolkit::scene::prop::DIM_PX, 6f32.to_bits()],
        );
        w.set_prop(
            1,
            id::WIDTH,
            &[wolkit::scene::prop::DIM_PERCENT, 50f32.to_bits()],
        );
        w.set_prop(1, id::BORDER, &[2f32.to_bits(), Token::Accent as u32]);
        w.set_prop(
            1,
            id::SHADOW_2,
            &[0, 2f32.to_bits(), 6f32.to_bits(), Token::Line as u32],
        );
        w.set_prop(1, id::FONT_SIZE, &[0]);
        w.set_prop(1, id::TRANSITION, &[id::BACKGROUND as u32, 1, 1]);
    });
    p.frame(400, 300);
    let c = p.computed(1);
    assert_eq!(c.layout.padding, [8.0, 8.0, 8.0, 20.0]);
    assert_eq!(c.layout.margin, [Dim::Px(6.0); 4]);
    assert_eq!(c.layout.width, Dim::Pct(50.0));
    assert_eq!(
        (c.layout.border_width, c.paint.border_color),
        (2.0, ColorRef::Token(Token::Accent))
    );
    assert_eq!(
        c.paint.shadow[1],
        Shadow {
            dx: 0.0,
            dy: 2.0,
            blur: 6.0,
            color: ColorRef::Token(Token::Line)
        }
    );
    assert_eq!(c.paint.shadow[0], Shadow::NONE);
    assert_eq!(c.text.size, 34.0, "wire role 0 is `display`");
    assert_eq!(
        (c.motion.transition.ms, c.motion.transition.ease),
        (250, Ease::Linear)
    );
    assert_ne!(
        c.motion.transition.props & wolkit::style::prop::PropId::Background.bit(),
        0
    );
}

#[test]
fn hover_restyles_one_node_and_dirties_paint_only() {
    let mut p = Pipe::with_sheet(SHEET);
    let card = p.class("card");
    p.patch(|w| {
        for n in 1..=3 {
            w.create(n, Kind::Box)
                .insert(0, n, APPEND)
                .set_style(n, &[card]);
        }
    });
    p.frame(400, 300);
    let before = p.styles.stats();
    let a = p.scene.computed(1);
    p.scene.set_engine_state(1, state::HOVER, true);
    assert_ne!(p.scene.dirty(1) & dirty::STYLE, 0);
    p.frame(400, 300);
    assert_ne!(p.scene.computed(1), a);
    assert_eq!(
        p.computed(1).paint.background,
        Paint::Color(ColorRef::Token(Token::Control))
    );
    assert_eq!(
        p.computed(2).paint.background,
        Paint::Color(ColorRef::Token(Token::Card)),
        "siblings are untouched"
    );
    let after = p.styles.stats();
    assert_eq!(
        (after.0 + after.1) - (before.0 + before.1),
        1,
        "exactly one resolution"
    );
    assert_ne!(p.scene.dirty(1) & dirty::PAINT, 0, "paint is dirty");
    // And the layout stage had nothing to do for it: a background change moves nothing.
    p.scene.clear_dirty(1, dirty::PAINT);
    p.scene.set_engine_state(1, state::HOVER, false);
    let rect = p.scene.rect(1);
    p.frame(400, 300);
    assert_eq!(p.scene.rect(1), rect);
    // Back to the first style: the memo returns the same interned id.
    assert_eq!(p.scene.computed(1), a);
}

#[test]
fn the_parent_form_reveals_a_child_on_parent_hover() {
    let mut p = Pipe::with_sheet(SHEET);
    let (row, actions) = (p.class("row"), p.class("actions"));
    p.patch(|w| {
        w.create(1, Kind::Box)
            .insert(0, 1, APPEND)
            .set_style(1, &[row]);
        w.create(2, Kind::Box)
            .insert(1, 2, APPEND)
            .set_style(2, &[actions]);
    });
    p.frame(400, 300);
    assert_eq!(p.computed(2).paint.opacity, 0.0);
    p.scene.set_engine_state(1, state::HOVER, true);
    p.frame(400, 300);
    assert_eq!(
        p.computed(2).paint.opacity,
        1.0,
        "the child follows its parent's state"
    );
    p.scene.set_engine_state(1, state::HOVER, false);
    p.frame(400, 300);
    assert_eq!(p.computed(2).paint.opacity, 0.0);
}

#[test]
fn media_follow_the_environment() {
    let mut p = Pipe::with_sheet(SHEET);
    let card = p.class("card");
    p.patch(|w| {
        w.create(1, Kind::Box)
            .insert(0, 1, APPEND)
            .set_style(1, &[card]);
    });
    p.frame(400, 300);
    assert_eq!(
        p.computed(1).paint.background,
        Paint::Color(ColorRef::Token(Token::Card))
    );
    p.env.dark = true;
    p.frame(400, 300);
    assert_eq!(
        p.computed(1).paint.background,
        Paint::Color(ColorRef::Token(Token::Sidebar))
    );
    p.env.dark = false;
    p.frame(400, 300);
    assert_eq!(
        p.computed(1).paint.background,
        Paint::Color(ColorRef::Token(Token::Card))
    );
}

#[test]
fn a_new_table_restyles_everything() {
    let mut p = Pipe::with_sheet(".a { padding: sm; }");
    let a = p.class("a");
    p.patch(|w| {
        w.create(1, Kind::Box)
            .insert(0, 1, APPEND)
            .set_style(1, &[a]);
    });
    p.frame(400, 300);
    assert_eq!(p.computed(1).layout.padding, [8.0; 4]);
    p.set_sheet(".a { padding: lg; }");
    p.frame(400, 300);
    assert_eq!(p.computed(1).layout.padding, [16.0; 4]);
    // The layout followed.
    p.patch(|w| {
        w.create(2, Kind::Text)
            .insert(1, 2, APPEND)
            .set_text(2, "x");
    });
    p.frame(400, 300);
    assert_eq!(p.scene.rect(2).x, 16.0);
}

#[test]
fn text_scale_scales_declared_sizes() {
    let mut p = Pipe::with_sheet(SHEET);
    let label = p.class("label");
    p.patch(|w| {
        w.create(1, Kind::Text)
            .insert(0, 1, APPEND)
            .set_style(1, &[label])
            .set_text(1, "hello");
        w.create(2, Kind::Text)
            .insert(0, 2, APPEND)
            .set_text(2, "plain");
    });
    p.frame(400, 300);
    assert_eq!(p.computed(1).text.size, 11.0);
    assert_eq!(p.computed(2).text.size, 13.0);
    p.env.text_scale = 1.5;
    p.frame(400, 300);
    assert_eq!(p.computed(1).text.size, 16.5);
    assert_eq!(p.computed(2).text.size, 19.5);
}

#[test]
fn compact_follows_the_window_width() {
    let mut p = Pipe::with_sheet("@compact { .a { padding: xs; } } .a { padding: lg; }");
    let a = p.class("a");
    p.patch(|w| {
        w.create(1, Kind::Box)
            .insert(0, 1, APPEND)
            .set_style(1, &[a]);
    });
    p.frame(800, 600);
    assert_eq!(p.computed(1).layout.padding, [16.0; 4]);
    p.frame(500, 600);
    assert_eq!(p.computed(1).layout.padding, [4.0; 4]);
}

#[test]
fn rectangles_are_in_window_space() {
    let mut p = Pipe::with_sheet(SHEET);
    let (card, big) = (p.class("card"), p.class("big"));
    p.patch(|w| {
        w.create(1, Kind::Box)
            .insert(0, 1, APPEND)
            .set_style(1, &[card, big]);
        w.create(2, Kind::Text)
            .insert(1, 2, APPEND)
            .set_text(2, "ab");
        w.set_prop(0, id::FLEX_DIRECTION, &[1]);
    });
    p.frame(400, 300);
    let r = p.scene.rect(1);
    assert_eq!((r.x, r.y, r.w, r.h), (0.0, 0.0, 200.0, 80.0));
    let t = p.scene.rect(2);
    assert_eq!((t.x, t.y), (12.0, 12.0), "inside the padding");
    assert_eq!(p.scene.rect(0).w, 400.0);
}

#[test]
fn only_what_changed_is_laid_out_again() {
    let mut p = Pipe::new();
    p.patch(|w| {
        w.set_prop(0, id::FLEX_DIRECTION, &[1]);
        for n in 1..=40u32 {
            w.create(n, Kind::Box).insert(0, n, APPEND);
            w.set_prop(n, id::HEIGHT, &[prop::DIM_PX, 20f32.to_bits()]);
            w.create(100 + n, Kind::Text)
                .insert(n, 100 + n, APPEND)
                .set_text(100 + n, "row text");
        }
    });
    p.frame(400, 900);
    let before = p.cache.stats;
    p.patch(|w| {
        w.set_text(105, "a much longer row text than before");
    });
    p.frame(400, 900);
    let d = p.cache.stats.arranges - before.arranges;
    assert!(d <= 3, "one row re-arranged, not forty: {d}");
    assert!(
        p.scene.rect(105).w > 150.0,
        "the text got wider: {:?}",
        p.scene.rect(105)
    );
    // A clean frame does nothing.
    let before = p.cache.stats;
    p.frame(400, 900);
    assert_eq!(p.cache.stats.arranges, before.arranges);
    assert_eq!(p.cache.stats.measures, before.measures);
    // Resizing the window lays out again.
    p.frame(300, 900);
    assert!(p.cache.stats.arranges > before.arranges);
    assert_eq!(p.scene.rect(0).w, 300.0);
}

#[test]
fn builtin_kind_rules_make_spacers_grow_and_scrollers_scroll() {
    let mut p = Pipe::new();
    p.patch(|w| {
        w.create(1, Kind::Box).insert(0, 1, APPEND);
        w.create(2, Kind::Spacer).insert(0, 2, APPEND);
        w.create(3, Kind::Box).insert(0, 3, APPEND);
        w.create(4, Kind::Scroll).insert(0, 4, APPEND);
    });
    p.frame(300, 40);
    assert_eq!(p.computed(2).layout.grow, 1.0);
    assert_eq!(p.computed(1).layout.grow, 0.0);
    assert_eq!(p.computed(4).layout.overflow, Overflow::Auto);
    assert_eq!(p.computed(4).layout.direction, Direction::Column);
    assert_eq!(
        p.scene.rect(3).x + p.scene.rect(3).w,
        300.0 - p.scene.rect(4).w,
        "the spacer took the space between"
    );
}

#[test]
fn display_none_has_an_empty_rectangle() {
    let mut p = Pipe::new();
    p.patch(|w| {
        w.create(1, Kind::Box).insert(0, 1, APPEND);
        w.set_prop(1, id::WIDTH, &[prop::DIM_PX, 50f32.to_bits()]);
        w.create(2, Kind::Box).insert(0, 2, APPEND);
        w.set_prop(2, id::WIDTH, &[prop::DIM_PX, 50f32.to_bits()]);
    });
    p.frame(200, 100);
    assert_eq!(p.scene.rect(2).x, 50.0);
    p.patch(|w| {
        w.set_prop(1, id::DISPLAY, &[1]);
    });
    p.frame(200, 100);
    assert_eq!(p.scene.rect(1).w, 0.0);
    assert_eq!(
        p.scene.rect(2).x,
        0.0,
        "the second box moved up to the start"
    );
}

#[test]
fn overlays_sit_next_to_their_anchor() {
    let mut p = Pipe::new();
    p.patch(|w| {
        w.create(1, Kind::Box).insert(0, 1, APPEND);
        w.set_prop(1, id::WIDTH, &[prop::DIM_PX, 80f32.to_bits()]);
        w.set_prop(1, id::HEIGHT, &[prop::DIM_PX, 30f32.to_bits()]);
        w.create(2, Kind::Overlay).insert(0, 2, APPEND);
        w.set_prop(2, id::ANCHOR, &[1]);
        w.set_prop(2, id::ANCHOR_SIDE, &[0]);
        w.set_prop(2, id::WIDTH, &[prop::DIM_PX, 120f32.to_bits()]);
        w.set_prop(2, id::HEIGHT, &[prop::DIM_PX, 50f32.to_bits()]);
    });
    p.frame(400, 300);
    let (a, o) = (p.scene.rect(1), p.scene.rect(2));
    assert_eq!(
        (o.x, o.y),
        (a.x, a.y + a.h + 4.0),
        "below the anchor, start aligned, 4 px apart"
    );
    assert_eq!((o.w, o.h), (120.0, 50.0));
    // The anchor moving moves the overlay.
    p.patch(|w| {
        w.set_prop(1, id::MARGIN_LEFT, &[prop::DIM_PX, 100f32.to_bits()]);
    });
    p.frame(400, 300);
    assert_eq!(p.scene.rect(2).x, p.scene.rect(1).x);
    assert!(p.scene.rect(2).x >= 100.0);
}

#[test]
fn a_vlist_places_its_rows_by_key() {
    let mut p = Pipe::new();
    p.patch(|w| {
        w.set_prop(0, id::FLEX_DIRECTION, &[1]);
        w.create(1, Kind::Vlist).insert(0, 1, APPEND);
        w.set_prop(1, id::HEIGHT, &[prop::DIM_PX, 200f32.to_bits()]);
        w.list_range(1, 500, 24.0, 2);
        for (n, row) in [(2u32, 40u64), (3, 41), (4, 42)] {
            w.create(n, Kind::Box).insert(1, n, APPEND).set_key(n, row);
            w.set_prop(n, id::HEIGHT, &[prop::DIM_PX, 24f32.to_bits()]);
        }
    });
    p.frame(300, 400);
    assert_eq!(p.scene.rect(2).y, 40.0 * 24.0);
    assert_eq!(p.scene.rect(4).y, 42.0 * 24.0);
    assert_eq!(p.cache.content_size(1).h, 500.0 * 24.0);
}

#[test]
fn prop_table_and_style_table_agree_on_the_wire() {
    // Every wire style property maps to something (or is deliberately node-specific).
    for d in prop::TABLE {
        if d.id >= 150 || d.id == id::TAB_INDEX {
            continue;
        }
        let mut decls = wolkit::style::prop::Decls::default();
        let words: Vec<u32> = match d.ty {
            prop::Ty::Enum => vec![0],
            prop::Ty::Dim => vec![prop::DIM_PX, 4f32.to_bits()],
            prop::Ty::Token => vec![1],
            prop::Ty::F32 | prop::Ty::I32 => vec![2f32.to_bits()],
            prop::Ty::Pair => vec![1f32.to_bits(), 2f32.to_bits()],
            prop::Ty::Border => vec![1f32.to_bits(), 1],
            prop::Ty::Gradient => vec![0, 2, 1, 2, 0],
            prop::Ty::Shadow => vec![0, 1f32.to_bits(), 2f32.to_bits(), 3],
            prop::Ty::Transition => vec![0, 1, 1],
            _ => vec![0],
        };
        wolkit::style::inline::decl_from_wire(d.id, &PropValue::new(&words), &mut decls);
        assert!(
            !decls.is_empty(),
            "wire property `{}` ({}) sets nothing in the style system",
            d.name,
            d.id
        );
    }
}

use wolkit::scene::PropValue;

#[test]
fn an_inline_in_out_transition_passes_the_validator() {
    let mut p = Pipe::new();
    p.patch(|w| {
        w.create(1, Kind::Box).insert(0, 1, APPEND);
        // Easing 4 is `in-out`, the last of the style's curves.
        w.set_prop(1, id::TRANSITION, &[u32::from(id::OPACITY), 0, 4]);
    });
    p.frame(200, 100);
    let t = p.computed(1).motion.transition;
    assert_eq!((t.ms, t.ease), (150, Ease::InOut));
    // One past the last curve is refused whole.
    let mut w = Writer::new();
    w.set_prop(1, id::TRANSITION, &[u32::from(id::OPACITY), 0, 5]);
    let bytes = w.finish(p.seq + 1).to_vec();
    assert!(Patch::parse(&bytes, &mut Scratch::default()).is_err());
}
