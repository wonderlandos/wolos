// SPDX-License-Identifier: BSD-2-Clause
//! The cascade: the memoised, interning resolver against the plain reference on random inputs,
//! then the rules of resolution one by one.
#[path = "support/layout.rs"]
mod layout_support;
use layout_support::Rng;
use wolkit::style::prop::*;
use wolkit::style::select::*;
use wolkit::style::tokens::Token;
use wolkit::style::value::*;
use wolkit::style::{compile_sources, render_all};

const SHEET: &str = "
button { padding: sm; cursor: pointer; }
text { color: dim; }
.a { padding: lg; background: card; color: accent; }
.b { padding: xs; opacity: 0.5; font-size: title; }
.c { width: 50px; }
.a:hover { background: control; }
.b:hover { opacity: 1; }
.c:active { width: 60px; }
.a:disabled { opacity: 0.3; }
@dark { .a { background: sidebar; } .c { width: 40px; } }
@contrast { .b:hover { opacity: 0.9; } }
.p:hover > .a { color: danger; }
.p:focus > .c { height: 10px; }
.p { font-size: caption; cursor: grab; }
";

fn table() -> StyleTable {
    compile_sources(&[("t.wolcss", SHEET)])
        .unwrap_or_else(|d| panic!("{}", render_all(&d)))
        .table
}

#[test]
fn the_memoised_resolver_equals_the_reference() {
    let t = table();
    let mut store = StyleStore::default();
    let mut rs = Resolver::default();
    let mut r = Rng(0xc0ffee_1234567);
    let n_classes = t.classes.len() as u32;
    let mut parents: Vec<StyleId> = vec![];
    let mut checks = 0;
    for _ in 0..30_000 {
        let nc = r.below(4);
        let classes: Vec<u32> = (0..nc).map(|_| r.below(n_classes)).collect();
        let pc: Vec<u32> = (0..r.below(3)).map(|_| r.below(n_classes)).collect();
        let parent = if !parents.is_empty() && r.chance(70) {
            Some(r.pick(&parents))
        } else {
            None
        };
        let inp = Input {
            kind: r.below(14) as u8,
            classes: &classes,
            state: r.pick(&[0, 1, 2, 4, 8, 16, 32]) as u16,
            media: (r.below(64)) as u8
                & !(if r.chance(50) {
                    MEDIA_LIGHT
                } else {
                    MEDIA_DARK
                }),
            parent,
            parent_classes: &pc,
            parent_state: r.pick(&[0, 1, 4]) as u16,
            inline: None,
            text_scale: r.pick(&[1.0, 1.0, 1.25]),
        };
        let got = rs.resolve(&t, &mut store, &inp);
        let want = cascade(&t, &store, &inp);
        assert_eq!(*store.get(got), want, "{inp:?}");
        // Resolving again is a memo hit with the same id.
        assert_eq!(rs.resolve(&t, &mut store, &inp), got);
        if parents.len() < 40 {
            parents.push(got);
        }
        checks += 1;
    }
    assert_eq!(checks, 30_000);
    assert!(
        rs.hits > 20_000,
        "the memo works: {} hits, {} misses",
        rs.hits,
        rs.misses
    );
    assert!(store.len() < 5000, "styles are interned: {}", store.len());
}

fn resolve_one(t: &StyleTable, inp: Input) -> Props {
    cascade(t, &StyleStore::default(), &inp)
}

#[test]
fn classes_in_the_order_given_later_wins() {
    let t = table();
    let (a, b) = (t.class_id("a").unwrap(), t.class_id("b").unwrap());
    let ab = resolve_one(
        &t,
        Input {
            classes: &[a, b],
            ..Input::default()
        },
    );
    let ba = resolve_one(
        &t,
        Input {
            classes: &[b, a],
            ..Input::default()
        },
    );
    assert_eq!(ab.layout.padding, [4.0; 4], "b after a");
    assert_eq!(ba.layout.padding, [16.0; 4], "a after b");
    assert_eq!(
        ab.paint.background, ba.paint.background,
        "properties only one sets stay"
    );
}

#[test]
fn kind_rules_come_first_and_state_variants_after_every_base() {
    let t = table();
    let a = t.class_id("a").unwrap();
    let b = kind_id("button").unwrap();
    let p = resolve_one(
        &t,
        Input {
            kind: b,
            classes: &[a],
            ..Input::default()
        },
    );
    assert_eq!(p.layout.padding, [16.0; 4], "the class beats the kind");
    assert_eq!(
        p.paint.cursor,
        Cursor::Pointer,
        "kind rules the class does not set remain"
    );
    // A state variant of an early class beats a base of a later one.
    let (a, bcls) = (t.class_id("a").unwrap(), t.class_id("b").unwrap());
    let p = resolve_one(
        &t,
        Input {
            classes: &[a, bcls],
            state: STATE_HOVER,
            ..Input::default()
        },
    );
    assert_eq!(p.paint.opacity, 1.0, "b:hover");
    assert_eq!(
        p.paint.background,
        Paint::Color(ColorRef::Token(Token::Control)),
        "a:hover"
    );
    let p = resolve_one(
        &t,
        Input {
            classes: &[bcls, a],
            state: 16,
            ..Input::default()
        },
    );
    assert_eq!(p.paint.opacity, 0.3, "a:disabled after b's base opacity");
}

#[test]
fn media_and_states_are_requirements_not_equalities() {
    let t = table();
    let (a, c) = (t.class_id("a").unwrap(), t.class_id("c").unwrap());
    let dark = resolve_one(
        &t,
        Input {
            classes: &[a],
            media: MEDIA_DARK | MEDIA_CONTRAST,
            ..Input::default()
        },
    );
    assert_eq!(
        dark.paint.background,
        Paint::Color(ColorRef::Token(Token::Sidebar))
    );
    let light = resolve_one(
        &t,
        Input {
            classes: &[a],
            media: MEDIA_LIGHT,
            ..Input::default()
        },
    );
    assert_eq!(
        light.paint.background,
        Paint::Color(ColorRef::Token(Token::Card))
    );
    // :active together with hover still matches `.c:active`.
    let p = resolve_one(
        &t,
        Input {
            classes: &[c],
            state: STATE_HOVER | 2,
            media: MEDIA_DARK,
            ..Input::default()
        },
    );
    assert_eq!(
        p.layout.width,
        Dim::Px(40.0),
        "equal weight keeps the order written: the dark block is the later one"
    );
}

#[test]
fn the_parent_form_needs_the_parent_class_and_state() {
    let t = table();
    let (a, p_) = (t.class_id("a").unwrap(), t.class_id("p").unwrap());
    let base = resolve_one(
        &t,
        Input {
            classes: &[a],
            ..Input::default()
        },
    );
    assert_eq!(base.text.color, ColorRef::Token(Token::Accent));
    let hov = resolve_one(
        &t,
        Input {
            classes: &[a],
            parent_classes: &[p_],
            parent_state: STATE_HOVER,
            ..Input::default()
        },
    );
    assert_eq!(hov.text.color, ColorRef::Token(Token::Danger));
    let wrong_state = resolve_one(
        &t,
        Input {
            classes: &[a],
            parent_classes: &[p_],
            parent_state: 4,
            ..Input::default()
        },
    );
    assert_eq!(wrong_state.text.color, ColorRef::Token(Token::Accent));
    let wrong_class = resolve_one(
        &t,
        Input {
            classes: &[a],
            parent_classes: &[a],
            parent_state: STATE_HOVER,
            ..Input::default()
        },
    );
    assert_eq!(wrong_class.text.color, ColorRef::Token(Token::Accent));
}

#[test]
fn inherited_properties_come_from_the_parent_and_only_those() {
    let t = table();
    let (p_, c) = (t.class_id("p").unwrap(), t.class_id("c").unwrap());
    let mut store = StyleStore::default();
    let par = cascade(
        &t,
        &store,
        &Input {
            classes: &[p_],
            ..Input::default()
        },
    );
    let pid = store.intern(&par);
    let kid = cascade(
        &t,
        &store,
        &Input {
            classes: &[c],
            parent: Some(pid),
            ..Input::default()
        },
    );
    assert_eq!(kid.text.size, 11.0, "font-size inherits");
    assert_eq!(kid.paint.cursor, Cursor::Grab, "cursor inherits");
    assert_eq!(kid.layout.width, Dim::Px(50.0));
    // Text kind rule `color: dim` is the node's own, not the parent's.
    let t_kind = kind_id("text").unwrap();
    let txt = cascade(
        &t,
        &store,
        &Input {
            kind: t_kind,
            parent: Some(pid),
            ..Input::default()
        },
    );
    assert_eq!(txt.text.color, ColorRef::Token(Token::Dim));
    // Non-inherited properties start from the default.
    let a = t.class_id("a").unwrap();
    let ap = cascade(
        &t,
        &store,
        &Input {
            classes: &[a],
            ..Input::default()
        },
    );
    let apid = store.intern(&ap);
    let child = cascade(
        &t,
        &store,
        &Input {
            parent: Some(apid),
            ..Input::default()
        },
    );
    assert_eq!(child.layout.padding, [0.0; 4]);
    assert_eq!(child.paint.background, Paint::None);
    assert_eq!(
        child.text.color,
        ColorRef::Token(Token::Accent),
        "color inherits"
    );
}

#[test]
fn inline_beats_everything_and_scales_apply_once() {
    let t = table();
    let b = t.class_id("b").unwrap();
    let mut inline = Decls::default();
    inline.p.layout.padding = [3.0; 4];
    inline.mask = PropId::PaddingTop.bit()
        | PropId::PaddingRight.bit()
        | PropId::PaddingBottom.bit()
        | PropId::PaddingLeft.bit();
    inline.p.text.size = 20.0;
    inline.mask |= PropId::FontSize.bit();
    let p = cascade(
        &t,
        &StyleStore::default(),
        &Input {
            classes: &[b],
            inline: Some(&inline),
            text_scale: 1.5,
            ..Input::default()
        },
    );
    assert_eq!(p.layout.padding, [3.0; 4]);
    assert_eq!(p.text.size, 30.0, "declared 20 scaled by 1.5, once");
    // A child that inherits an already scaled size is not scaled again.
    let mut store = StyleStore::default();
    let pid = store.intern(&p);
    let kid = cascade(
        &t,
        &store,
        &Input {
            parent: Some(pid),
            text_scale: 1.5,
            ..Input::default()
        },
    );
    assert_eq!(kid.text.size, 30.0);
}

#[test]
fn interning_shares_equal_styles() {
    let t = table();
    let a = t.class_id("a").unwrap();
    let mut store = StyleStore::default();
    let mut rs = Resolver::default();
    let x = rs.resolve(
        &t,
        &mut store,
        &Input {
            classes: &[a],
            ..Input::default()
        },
    );
    // A different route to the same computed style gives the same id.
    let y = rs.resolve(
        &t,
        &mut store,
        &Input {
            classes: &[a, a],
            ..Input::default()
        },
    );
    assert_eq!(x, y);
    assert_eq!(store.get(StyleId(0)), &Props::default());
    assert_ne!(x, StyleId(0));
}

#[test]
fn diff_names_what_changed_and_layout_follows() {
    let base = Props::default();
    let mut c = base.clone();
    c.paint.opacity = 0.5;
    c.text.color = ColorRef::Token(Token::Danger);
    let d = diff(&base, &c);
    assert_eq!(d, PropId::Opacity.bit() | PropId::Color.bit());
    assert!(!needs_layout(d), "paint and color never need layout");
    c.text.size = 20.0;
    assert!(needs_layout(diff(&base, &c)));
    let mut e = base.clone();
    e.layout.border_width = 1.0;
    assert!(needs_layout(diff(&base, &e)), "a border takes room");
    assert_eq!(diff(&base, &base), 0);
}
