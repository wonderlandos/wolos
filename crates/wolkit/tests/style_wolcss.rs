// SPDX-License-Identifier: BSD-2-Clause
//! The `.wolcss` front end: sheet text to table, table to `WSB1` and back, and the resolved styles
//! (goldens in `tests/style_golden`, regenerate with `WOLKIT_BLESS=1`). Also the shapes the parser
//! accepts: shorthands, selector lists, kinds, media, the parent form, comments.
use wolkit::style::prop::*;
use wolkit::style::select::*;
use wolkit::style::value::*;
use wolkit::style::{compile_sources, render_all, wsb};

fn compile(src: &str) -> wolkit::style::Compiled {
    compile_sources(&[("t.wolcss", src)]).unwrap_or_else(|d| panic!("{}", render_all(&d)))
}

fn dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/style_golden")
}

/// Resolve one class under some conditions and list what differs from the default style.
fn resolved(
    t: &StyleTable,
    class: u32,
    state: u16,
    media: u8,
    parent: &[u32],
    parent_state: u16,
) -> String {
    let store = StyleStore::default();
    let inp = Input {
        kind: 0,
        classes: &[class],
        state,
        media,
        parent_classes: parent,
        parent_state,
        ..Input::default()
    };
    let p = cascade(t, &store, &inp);
    let mask = diff(&Props::default(), &p);
    let mut out = String::new();
    for id in PropId::ALL {
        if mask & id.bit() != 0 {
            out.push_str(&format!("    {}: {}\n", id.name(), fmt_prop(*id, &p)));
        }
    }
    if out.is_empty() {
        out.push_str("    (default)\n");
    }
    out
}

fn dump(t: &StyleTable) -> String {
    let mut s = String::new();
    let scenarios: [(&str, u16, u8); 5] = [
        ("plain", 0, MEDIA_LIGHT),
        ("hover", STATE_HOVER, MEDIA_LIGHT),
        ("dark", 0, MEDIA_DARK),
        (
            "focus-visible dark contrast",
            8,
            MEDIA_DARK | MEDIA_CONTRAST,
        ),
        ("compact reduce-motion dark", 0, MEDIA_DARK | 8 | 32),
    ];
    for (i, c) in t.classes.iter().enumerate() {
        s.push_str(&format!(".{} ({} variants)\n", c.name, c.variants.len()));
        for (name, st, md) in scenarios {
            s.push_str(&format!(
                "  [{name}]\n{}",
                resolved(t, i as u32, st, md, &[], 0)
            ));
        }
    }
    s.push_str("kinds\n");
    for (k, def) in t.kinds.iter().enumerate() {
        if !def.variants.is_empty() {
            s.push_str(&format!(
                "  {} ({} variants)\n",
                KINDS[k],
                def.variants.len()
            ));
            let store = StyleStore::default();
            let p = cascade(
                t,
                &store,
                &Input {
                    kind: k as u8,
                    ..Input::default()
                },
            );
            let mask = diff(&Props::default(), &p);
            for id in PropId::ALL {
                if mask & id.bit() != 0 {
                    s.push_str(&format!("    {}: {}\n", id.name(), fmt_prop(*id, &p)));
                }
            }
        }
    }
    s
}

fn check_golden(name: &str) {
    let src = std::fs::read_to_string(dir().join(format!("{name}.wolcss"))).unwrap();
    let c = compile(&src);
    // The table survives the binary format exactly, and resolves the same.
    let back = wsb::load(&c.bytes).unwrap();
    assert_eq!(back, c.table, "{name}: WSB1 round trip");
    assert_eq!(
        wsb::write(&back).unwrap(),
        c.bytes,
        "{name}: writing is deterministic"
    );
    let a = dump(&c.table);
    let b = dump(&back);
    assert_eq!(a, b, "{name}: resolved styles differ after the round trip");
    let path = dir().join(format!("{name}.golden"));
    if std::env::var("WOLKIT_BLESS").is_ok() {
        std::fs::write(&path, &a).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        a, want,
        "{name}: resolved styles changed; review and bless with WOLKIT_BLESS=1"
    );
}

#[test]
fn basic_sheet_golden() {
    check_golden("basic");
}

#[test]
fn layout_sheet_golden() {
    check_golden("layout");
}

#[test]
fn goldens_have_no_diagnostics() {
    for n in ["basic", "layout"] {
        let src = std::fs::read_to_string(dir().join(format!("{n}.wolcss"))).unwrap();
        let c = compile(&src);
        assert!(c.warnings.is_empty(), "{n}: {}", render_all(&c.warnings));
    }
}

fn one(src: &str) -> Decls {
    let c = compile(src);
    c.table.classes[0].variants[0].decls.clone()
}

#[test]
fn shorthands_expand_like_css() {
    let d = one(".a { padding: sm; }");
    assert_eq!(d.p.layout.padding, [8.0; 4]);
    let d = one(".a { padding: sm lg; }");
    assert_eq!(d.p.layout.padding, [8.0, 16.0, 8.0, 16.0]);
    let d = one(".a { padding: xs sm md; }");
    assert_eq!(d.p.layout.padding, [4.0, 8.0, 12.0, 8.0]);
    let d = one(".a { padding: xs sm md lg; }");
    assert_eq!(d.p.layout.padding, [4.0, 8.0, 12.0, 16.0]);
    assert_eq!(
        d.mask,
        PropId::PaddingTop.bit()
            | PropId::PaddingRight.bit()
            | PropId::PaddingBottom.bit()
            | PropId::PaddingLeft.bit()
    );
    let d = one(".a { margin: auto 0; }");
    assert_eq!(
        d.p.layout.margin,
        [Dim::Auto, Dim::Px(0.0), Dim::Auto, Dim::Px(0.0)]
    );
    let d = one(".a { gap: sm; }");
    assert_eq!((d.p.layout.row_gap, d.p.layout.col_gap), (8.0, 8.0));
    let d = one(".a { gap: sm lg; }");
    assert_eq!(
        (d.p.layout.row_gap, d.p.layout.col_gap),
        (8.0, 16.0),
        "row then column"
    );
    let d = one(".a { border-radius: s m; }");
    assert_eq!(d.p.paint.radius, [8.0, 12.0, 8.0, 12.0]);
    let d = one(".a { inset: 0px 4px; }");
    assert_eq!(
        d.p.layout.inset,
        [Dim::Px(0.0), Dim::Px(4.0), Dim::Px(0.0), Dim::Px(4.0)]
    );
    let d = one(".a { border: 2px danger; }");
    assert_eq!(
        (d.p.layout.border_width, d.p.paint.border_color),
        (2.0, ColorRef::Token(wolkit::style::tokens::Token::Danger))
    );
    let d = one(".a { border: none; }");
    assert_eq!(d.mask, PropId::BorderWidth.bit());
    let d = one(".a { outline: 3px; }");
    assert_eq!(
        (d.p.paint.outline_width, d.mask),
        (3.0, PropId::OutlineWidth.bit()),
        "colour omitted: only the width is set"
    );
}

#[test]
fn selectors_in_all_their_forms() {
    let c = compile(
        ".a, .b:hover, button { opacity: 1; } .p:focus > .c { opacity: 0; } @dark @compact { .a:active { opacity: 0.5; } }",
    );
    let t = &c.table;
    let a = t.class_id("a").unwrap();
    let b = t.class_id("b").unwrap();
    assert_eq!(t.classes[a as usize].variants.len(), 2);
    let act = t.classes[a as usize]
        .variants
        .iter()
        .find(|v| v.state == 2)
        .unwrap();
    assert_eq!(act.media, MEDIA_DARK | 32);
    assert_eq!(t.classes[b as usize].variants[0].state, STATE_HOVER);
    assert_eq!(
        t.kinds[kind_id("button").unwrap() as usize].variants.len(),
        1
    );
    let child = &t.classes[t.class_id("c").unwrap() as usize].variants[0];
    let p = child.parent.unwrap();
    assert_eq!(
        (t.classes[p.class as usize].name.as_str(), p.state),
        ("p", 4)
    );
    // The parent class exists even though no rule styles it.
    assert!(t.class_id("p").is_some());
}

#[test]
fn comments_whitespace_and_last_semicolon() {
    let d = one("/* top */ .a /* x */ {\n\topacity :  0.5 ; /* y */ color:dim\n}\n/* end */");
    assert_eq!(d.p.paint.opacity, 0.5);
    assert_eq!(d.mask, PropId::Opacity.bit() | PropId::Color.bit());
    let c = compile(".a{opacity:1}.b{opacity:0}");
    assert_eq!(c.table.classes.len(), 2);
}

#[test]
fn values_parse_to_what_they_say() {
    let d = one(".a { background: linear-gradient(to bottom, card, accent); }");
    assert!(matches!(
        d.p.paint.background,
        Paint::Linear {
            angle: 180,
            n: 2,
            ..
        }
    ));
    let d = one(".a { font-size: 14px; font-weight: bold; line-height: normal; max-lines: none; }");
    assert_eq!(
        (
            d.p.text.size,
            d.p.text.weight,
            d.p.text.line_height,
            d.p.text.max_lines
        ),
        (14.0, 700, 0.0, 0)
    );
    let d = one(".a { transition: all 200ms linear; }");
    let tr = d.p.motion.transition;
    assert_eq!((tr.ms, tr.ease), (200, Ease::Linear));
    assert_eq!(tr.props & PropId::Opacity.bit(), PropId::Opacity.bit());
    assert_eq!(
        tr.props & PropId::Width.bit(),
        0,
        "layout properties do not animate"
    );
    let d = one(".a { aspect-ratio: 2; }");
    assert_eq!(d.p.layout.aspect, 2.0);
    let d = one(".a { box-shadow: lg; }");
    assert_eq!(d.p.paint.shadow[0].dy, 6.0);
    assert_eq!(d.p.paint.shadow[1].dy, 1.0);
    let d = one(".a { opacity: 0; width: 0; }");
    assert_eq!(d.p.layout.width, Dim::Px(0.0));
}

#[test]
fn merging_later_blocks_win() {
    let c = compile_sources(&[
        ("a.wolcss", ".a { opacity: 1; color: dim; }"),
        ("b.wolcss", ".a { opacity: 0.5; }"),
    ])
    .unwrap();
    let v = &c.table.classes[0].variants[0];
    assert_eq!(v.decls.p.paint.opacity, 0.5);
    assert_eq!(
        v.decls.p.text.color,
        ColorRef::Token(wolkit::style::tokens::Token::Dim)
    );
    assert_eq!(c.warnings.len(), 1, "WK-021");
}

#[test]
fn class_ids_follow_first_appearance() {
    let c = compile(".z { opacity: 1; } .a { opacity: 1; } .m:hover > .z { opacity: 0; }");
    let names: Vec<&str> = c.table.classes.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["z", "a", "m"]);
    let cr = wsb::crystal_constants(&c.table, "BSD-2-Clause");
    assert!(
        cr.contains("Z = 0_u32") && cr.contains("A = 1_u32") && cr.contains("M = 2_u32"),
        "{cr}"
    );
    // Line 1 is the licence, then a sentence of purpose (RCR-001, RCR-002).
    let mut lines = cr.lines();
    assert_eq!(
        lines.next(),
        Some("# SPDX-License-Identifier: BSD-2-Clause")
    );
    let purpose = lines.next().unwrap();
    assert!(
        purpose.starts_with("# Generated by wolkit-stylec") && purpose.len() >= 42,
        "{purpose}"
    );
}

#[test]
fn crystal_names_are_valid_constants() {
    let c = compile(".nav-item { opacity: 1; } .x2 { opacity: 1; }");
    let cr = wsb::crystal_constants(&c.table, "MIT");
    assert!(cr.starts_with("# SPDX-License-Identifier: MIT\n"), "{cr}");
    assert!(
        cr.contains("NAV_ITEM = 0_u32") && cr.contains("X2 = 1_u32"),
        "{cr}"
    );
}

// ---- the parser has to survive anything

mod robust {
    use super::*;
    use wolkit::style::wolcss::Builder;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() >> 11) as usize % n.max(1)
        }
    }

    const FRAGMENTS: &[&str] = &[
        ".a",
        ".b-c",
        "button",
        ":hover",
        ":x",
        " ",
        "\n",
        "\t",
        "{",
        "}",
        ";",
        ":",
        ",",
        ">",
        "(",
        ")",
        "/*",
        "*/",
        "@dark",
        "@",
        "@import",
        "padding",
        "color",
        "opacity",
        "transition",
        "background",
        "linear-gradient(",
        "alpha(",
        "rgb(",
        "#",
        "#fff",
        "!important",
        "!",
        "1px",
        "13px",
        "-4px",
        "50%",
        "0.5",
        "1e9",
        "md",
        "accent",
        "none",
        "auto",
        "é",
        "日本",
        "😀",
        "\u{0}",
        "\r\n",
        "'",
        "\"",
        "/",
        "\\",
        "calc(",
        "var(--x)",
        "..",
        "--",
        "999999999999px",
        "-",
        "+",
        "*",
        "[",
        "]",
        "=",
        "~",
        "|",
        "^",
        "$",
        "%",
        "&",
    ];

    fn check(src: &str) {
        let mut b = Builder::new();
        b.add("fuzz.wolcss", src);
        let (table, diags) = b.finish();
        for d in &diags {
            assert!(d.line >= 1 && d.col >= 1 && d.len >= 1, "{d:?}");
            let r = d.render();
            assert!(r.contains(d.id));
            assert!(!r.to_lowercase().contains("failed"));
        }
        // Whatever compiled round-trips.
        if !diags.iter().any(|d| d.is_error()) {
            let bytes = wsb::write(&table).unwrap();
            assert_eq!(wsb::load(&bytes).unwrap(), table, "{src:?}");
        }
    }

    #[test]
    fn random_fragments_never_panic() {
        let mut r = Rng(0x1234_5678_9abc_def1);
        for _ in 0..20_000 {
            let n = r.below(40) + 1;
            let mut s = String::new();
            for _ in 0..n {
                s.push_str(FRAGMENTS[r.below(FRAGMENTS.len())]);
            }
            check(&s);
        }
    }

    #[test]
    fn mutated_real_sheets_never_panic() {
        let mut r = Rng(0xfeed_f00d_0000_0003);
        for name in ["basic", "layout"] {
            let src = std::fs::read_to_string(dir().join(format!("{name}.wolcss"))).unwrap();
            let chars: Vec<char> = src.chars().collect();
            for _ in 0..3_000 {
                let mut c = chars.clone();
                for _ in 0..=r.below(4) {
                    let at = r.below(c.len());
                    match r.below(3) {
                        0 => {
                            c.remove(at);
                        }
                        1 => c.insert(
                            at,
                            FRAGMENTS[r.below(FRAGMENTS.len())]
                                .chars()
                                .next()
                                .unwrap_or(' '),
                        ),
                        _ => c.truncate(at.max(1)),
                    }
                    if c.is_empty() {
                        c.push('x');
                    }
                }
                check(&c.into_iter().collect::<String>());
            }
        }
    }

    #[test]
    fn deep_and_long_input_is_bounded() {
        // A very long single value and a very long selector list are fine.
        let long = format!(".a {{ padding: {} }}", "sm ".repeat(5000));
        check(&long);
        let list: String =
            (0..3000).map(|i| format!(".c{i}-x, ")).collect::<String>() + ".z { opacity: 1; }";
        check(&list);
        check(&"{".repeat(10_000));
        check(&"}".repeat(10_000));
        check(&"/*".repeat(10_000));
        check(&"@dark ".repeat(10_000));
        check(&format!(
            ".a {{ transition: {} }}",
            "opacity, ".repeat(5000)
        ));
    }
}
