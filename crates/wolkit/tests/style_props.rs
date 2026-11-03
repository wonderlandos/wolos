// SPDX-License-Identifier: BSD-2-Clause
//! The property table is part of the `WSB1` format: ids are append-only. A snapshot of
//! (id, name, group, effect, animatable) fails on any removed, renamed or reordered property and
//! on any change of what a property affects; additions at the end are blessed with
//! `WOLKIT_BLESS=1 cargo test -p wolkit --test style_props`.
use wolkit::style::prop::*;
use wolkit::style::select::{KINDS, MEDIA, STATES};
use wolkit::style::value::*;

fn snapshot() -> String {
    let mut s = String::new();
    for p in PROPS {
        s.push_str(&format!(
            "{} {} {} {} {}{}\n",
            p.id as u8,
            p.name,
            p.group.name().to_lowercase(),
            if p.effect == Effect::Layout {
                "layout"
            } else {
                "paint"
            },
            if p.anim { "anim" } else { "-" },
            if p.inherited { " inherited" } else { "" }
        ));
    }
    s.push_str("# kinds\n");
    for (i, k) in KINDS.iter().enumerate() {
        s.push_str(&format!("{i} {k}\n"));
    }
    s.push_str("# states\n");
    for (i, k) in STATES.iter().enumerate() {
        s.push_str(&format!("{} {k}\n", 1u32 << i));
    }
    s.push_str("# media\n");
    for (i, k) in MEDIA.iter().enumerate() {
        s.push_str(&format!("{} {k}\n", 1u32 << i));
    }
    macro_rules! en {
        ($($t:ty),+) => {$(
            s.push_str(&format!("# {}\n", stringify!($t)));
            for (i, k) in <$t>::KEYWORDS.iter().enumerate() {
                s.push_str(&format!("{i} {k}\n"));
            }
        )+};
    }
    en!(
        Display,
        Direction,
        Wrap,
        AlignItems,
        AlignSelf,
        AlignContent,
        Justify,
        Position,
        Overflow,
        Cursor,
        PointerEvents,
        Visibility,
        Backdrop
    );
    en!(
        FontFamily,
        FontStyle,
        TextAlign,
        TextDecoration,
        TextOverflow,
        WhiteSpace,
        UserSelect,
        Ease,
        Spring
    );
    s
}

#[test]
fn ids_are_append_only() {
    let got = snapshot();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/style_props.snap");
    if std::env::var("WOLKIT_BLESS").is_ok() {
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    if want == got {
        return;
    }
    // Additions at the end of a list are allowed; anything else is a break of the format.
    let (w, g): (Vec<&str>, Vec<&str>) = (want.lines().collect(), got.lines().collect());
    let mut section_w = vec![];
    let mut broken = vec![];
    for (i, line) in w.iter().enumerate() {
        match g.get(i) {
            Some(x) if x == line => section_w.push(*line),
            _ => {
                broken.push(*line);
                break;
            }
        }
    }
    assert!(
        broken.is_empty() || (broken[0].starts_with('#') && g.len() > w.len()),
        "tests/style_props.snap does not match the property table: `{}` changed or moved. Ids and keyword order are part of WSB1; append, never edit (bless with WOLKIT_BLESS=1 only for an addition).",
        broken.first().unwrap_or(&"")
    );
    panic!(
        "the table grew; review the additions and bless with WOLKIT_BLESS=1 cargo test -p wolkit --test style_props"
    );
}

#[test]
fn table_is_consistent() {
    assert_eq!(PROPS.len(), PropId::COUNT);
    const _: () = assert!(PropId::COUNT <= 128, "the mask is 128 bits");
    let mut names = std::collections::HashSet::new();
    for (i, p) in PROPS.iter().enumerate() {
        assert_eq!(p.id as usize, i, "{} is out of order", p.name);
        assert!(names.insert(p.name), "duplicate name {}", p.name);
        assert_eq!(PropId::parse_name(p.name), Some(p.id));
        assert_eq!(PropId::from_u8(i as u8), Some(p.id));
        assert!(
            !p.doc.is_empty() && !p.grammar.is_empty(),
            "{} lacks docs",
            p.name
        );
        assert!(p.name.chars().all(|c| c.is_ascii_lowercase() || c == '-'));
    }
    assert_eq!(PropId::from_u8(PropId::COUNT as u8), None);
    for s in SHORTHANDS {
        assert!(
            PropId::parse_name(s.name).is_none(),
            "a shorthand shadows a property: {}",
            s.name
        );
        assert!(!s.sets.is_empty());
    }
    // A paint-only property never needs layout; the font group and box model do.
    assert_eq!(PropId::Opacity.info().effect, Effect::Paint);
    assert_eq!(PropId::FontSize.info().effect, Effect::Layout);
    assert_eq!(PropId::Width.info().effect, Effect::Layout);
    assert_eq!(PropId::BorderWidth.info().effect, Effect::Layout);
    assert_eq!(layout_mask() & PropId::Color.bit(), 0);
    // Inherited properties are the text ones plus cursor and visibility.
    let inh: Vec<&str> = PROPS
        .iter()
        .filter(|p| p.inherited)
        .map(|p| p.name)
        .collect();
    assert!(
        inh.contains(&"color")
            && inh.contains(&"font-size")
            && inh.contains(&"cursor")
            && inh.contains(&"visibility")
    );
    assert!(!inh.contains(&"padding-top") && !inh.contains(&"background"));
}

#[test]
fn every_property_survives_the_codec_with_a_non_default_value() {
    // Set each property to something other than the default through the parser, encode, decode.
    let samples: &[(&str, &str)] = &[
        ("display", "none"),
        ("flex-direction", "column-reverse"),
        ("flex-wrap", "wrap"),
        ("grow", "2"),
        ("shrink", "0.5"),
        ("basis", "40%"),
        ("row-gap", "md"),
        ("column-gap", "lg"),
        ("align-items", "center"),
        ("align-self", "end"),
        ("align-content", "space-around"),
        ("justify-content", "space-evenly"),
        ("padding-top", "xs"),
        ("padding-right", "sm"),
        ("padding-bottom", "md"),
        ("padding-left", "xl"),
        ("margin-top", "auto"),
        ("margin-right", "lg"),
        ("margin-bottom", "xs"),
        ("margin-left", "auto"),
        ("width", "120px"),
        ("height", "50%"),
        ("min-width", "10px"),
        ("min-height", "20px"),
        ("max-width", "300px"),
        ("max-height", "90%"),
        ("aspect-ratio", "16 / 9"),
        ("position", "absolute"),
        ("top", "4px"),
        ("right", "-2px"),
        ("bottom", "10%"),
        ("left", "1px"),
        ("overflow", "scroll"),
        ("z-index", "-3"),
        ("border-width", "2px"),
        (
            "background",
            "linear-gradient(90deg, accent, card, alpha(danger, 30%))",
        ),
        ("border-color", "danger"),
        ("border-top-left-radius", "s"),
        ("border-top-right-radius", "m"),
        ("border-bottom-right-radius", "l"),
        ("border-bottom-left-radius", "full"),
        ("box-shadow", "md, sm"),
        ("opacity", "35%"),
        ("outline-width", "2px"),
        ("outline-color", "success"),
        (
            "transform",
            "translate(4px, -2px) scale(1.5, 2) rotate(10deg)",
        ),
        ("cursor", "grab"),
        ("pointer-events", "none"),
        ("visibility", "hidden"),
        ("backdrop", "glass"),
        ("color", "dim"),
        ("font-family", "mono"),
        ("font-size", "title"),
        ("font-weight", "600"),
        ("font-style", "italic"),
        ("line-height", "1.5"),
        ("letter-spacing", "1px"),
        ("text-align", "justify"),
        ("text-decoration", "strikethrough"),
        ("text-overflow", "ellipsis"),
        ("white-space", "pre-wrap"),
        ("max-lines", "3"),
        ("font-feature", "tnum"),
        ("user-select", "none"),
        ("transition", "opacity, background slow in-out"),
        ("spring", "bouncy"),
    ];
    let mut covered = std::collections::HashSet::new();
    for (name, value) in samples {
        let src = format!(".a {{ {name}: {value}; }}");
        let c = wolkit::style::compile_sources(&[("t.wolcss", &src)])
            .unwrap_or_else(|d| panic!("{src}: {}", wolkit::style::render_all(&d)));
        let id = PropId::parse_name(name).unwrap();
        let v = &c.table.classes[0].variants[0];
        assert_eq!(v.decls.mask, id.bit(), "{src}");
        let mut def = Props::default();
        let mut got_text = String::new();
        let mut want_text = String::new();
        got_text.push_str(&fmt_prop(id, &v.decls.p));
        want_text.push_str(&fmt_prop(id, &def));
        assert_ne!(got_text, want_text, "{src} left the default value");
        // WSB1 round trip
        let back = wolkit::style::wsb::load(&c.bytes).unwrap();
        assert_eq!(back, c.table, "{src}");
        let _ = &mut def;
        covered.insert(id);
    }
    for p in PropId::ALL {
        assert!(covered.contains(p), "no codec sample for `{}`", p.name());
    }
}
