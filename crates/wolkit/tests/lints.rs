// SPDX-License-Identifier: BSD-2-Clause
//! One test per lint id: a source that triggers it, the exact text the caret points at, and what
//! the message and suggestion say. Plus the catalogue itself: every id is tested, documented, and
//! none of the sentences says "failed" (RU-003).
use wolkit::style::lint::*;
use wolkit::style::wolcss::Builder;

fn run(src: &str) -> Vec<Diag> {
    let mut b = Builder::new();
    b.add("t.wolcss", src);
    b.finish().1
}

/// The text under the caret of the first diagnostic with `id`.
fn highlighted(d: &Diag) -> String {
    d.source_line
        .chars()
        .skip(d.col as usize - 1)
        .take(d.len as usize)
        .collect()
}

fn find<'a>(ds: &'a [Diag], id: &str) -> &'a Diag {
    ds.iter().find(|d| d.id == id).unwrap_or_else(|| {
        panic!(
            "no {id} in {:#?}",
            ds.iter().map(|d| (d.id, &d.message)).collect::<Vec<_>>()
        )
    })
}

macro_rules! lint {
    ($name:ident, $id:expr, $src:expr, $under:expr, $help:expr) => {
        #[test]
        fn $name() {
            let src: String = $src.to_string();
            let ds = run(&src);
            let d = find(&ds, $id);
            assert_eq!(
                highlighted(d),
                $under,
                "the caret points at the offending text: {}",
                d.render()
            );
            assert_eq!(d.file, "t.wolcss");
            assert!(d.line >= 1 && d.col >= 1);
            let help = d.help.clone().unwrap_or_default();
            assert!(
                help.contains($help),
                "help `{help}` should mention `{}`",
                $help
            );
            let r = d.render();
            assert!(
                r.starts_with(&format!("t.wolcss:{}:{}: ", d.line, d.col)),
                "{r}"
            );
            assert!(r.contains(&format!("[{}]", $id)));
            assert!(r.contains('^'));
            // The severity matches the catalogue.
            assert_eq!(d.severity, def($id).unwrap().severity);
        }
    };
}

lint!(
    wk_001_hex_literal,
    "WK-001",
    ".a { background: #fff; }",
    "#fff",
    "token"
);
lint!(
    wk_002_important,
    "WK-002",
    ".a { opacity: 1 !important; }",
    "!important",
    "order"
);
lint!(
    wk_003_font_too_small,
    "WK-003",
    ".a { font-size: 10px; }",
    "10px",
    "11px"
);
lint!(
    wk_004_duration_too_long,
    "WK-004",
    ".a { transition: opacity 600ms; }",
    "600ms",
    "slow"
);
lint!(
    wk_005_off_scale_spacing,
    "WK-005",
    ".a { padding: 13px; }",
    "13px",
    "nearest is `md`"
);
lint!(
    wk_006_off_scale_radius,
    "WK-006",
    ".a { border-radius: 7px; }",
    "7px",
    "nearest is `s`"
);
lint!(
    wk_007_unknown_token,
    "WK-007",
    ".a { color: acent; }",
    "acent",
    "did you mean `accent`"
);
lint!(
    wk_008_unknown_property,
    "WK-008",
    ".a { colour: text; }",
    "colour",
    "did you mean `color`"
);
lint!(
    wk_009_unsupported_grid,
    "WK-009",
    ".a { display: grid; }",
    "grid",
    "flex"
);
lint!(wk_010_bad_value, "WK-010", ".a { opacity: 2; }", "2", "");
lint!(
    wk_011_unknown_state,
    "WK-011",
    ".a:hovr { opacity: 1; }",
    "hovr",
    "did you mean `:hover`"
);
lint!(
    wk_012_selector_form,
    "WK-012",
    ".a .b { opacity: 1; }",
    ".a .b",
    ".parent:state > .child"
);
lint!(
    wk_013_duplicate_property,
    "WK-013",
    ".a { opacity: 1; opacity: 0.5; }",
    "opacity",
    "later"
);
lint!(
    wk_014_too_many_variants,
    "WK-014",
    ".a { opacity: 1; } .a:hover { opacity: 1; } .a:active { opacity: 1; } .a:focus { opacity: 1; } .a:disabled { opacity: 1; } .a:selected { opacity: 1; } .a:checked { opacity: 1; } .a:expanded { opacity: 1; } .a:empty { opacity: 1; }",
    ".a:empty",
    "two classes"
);
lint!(wk_015_empty_rule, "WK-015", ".a { }", ".a", "declaration");
lint!(
    wk_016_table_limit,
    "WK-016",
    (0..=4096)
        .map(|i| format!(".c{} {{ opacity: 1; }}\n", name(i)))
        .collect::<String>(),
    ".c-names-4096",
    "split"
);
lint!(
    wk_017_bad_weight,
    "WK-017",
    ".a { font-weight: 300; }",
    "300",
    "400, 500, 600 and 700"
);
lint!(
    wk_018_syntax,
    "WK-018",
    ".a { opacity 1; }",
    "opacity 1",
    "property: value"
);
lint!(
    wk_019_unknown_name,
    "WK-019",
    ".a { padding: huge; }",
    "huge",
    "names are"
);
lint!(
    wk_020_opt_out,
    "WK-020",
    "/* wolkit: off-scale */\n.a { padding: 13px; }",
    "/*",
    "counted"
);
lint!(
    wk_021_merged_variant,
    "WK-021",
    ".a { opacity: 1; } .a { color: dim; }",
    ".a",
    "one block"
);
lint!(
    wk_022_not_animatable,
    "WK-022",
    ".a { transition: width fast; }",
    "width",
    "colours"
);
lint!(
    wk_023_color_function,
    "WK-023",
    ".a { color: rgb(1, 2, 3); }",
    "rgb(",
    "alpha(token"
);
lint!(
    wk_024_conflicting_media,
    "WK-024",
    "@dark @light { .a { opacity: 1; } }",
    "@dark @light",
    "two blocks"
);

/// Class names for the limit test: lower-case words only.
fn name(i: usize) -> String {
    let mut s = String::from("-names-");
    s.push_str(&i.to_string());
    s
}

#[test]
fn the_limit_test_names_are_valid_classes() {
    // `.c-names-4096` is the 4097th class; make sure the generator really produced valid names.
    let ds = run(&format!(".c{} {{ opacity: 1; }}", name(5)));
    assert!(ds.is_empty(), "{ds:#?}");
}

#[test]
fn every_lint_has_a_test_and_a_place_in_the_docs() {
    let me = include_str!("lints.rs");
    let doc = wolkit::style::doc::render();
    let mut seen = std::collections::HashSet::new();
    for l in LINTS {
        assert!(seen.insert(l.id), "duplicate id {}", l.id);
        assert!(l.id.starts_with("WK-") && l.id.len() == 6, "{}", l.id);
        assert!(
            me.contains(&format!("\"{}\"", l.id)),
            "no test for {}",
            l.id
        );
        assert!(doc.contains(l.id), "{} is not in the generated docs", l.id);
        assert!(!l.summary.to_lowercase().contains("failed"));
    }
    // Ids are contiguous: WK-001 .. WK-nnn.
    for (i, l) in LINTS.iter().enumerate() {
        assert_eq!(l.id, format!("WK-{:03}", i + 1));
    }
}

#[test]
fn no_sentence_says_failed() {
    let bad = [
        ".a { background: #fff; opacity 1; font-size: 9px; padding: 13px; colour: x; display: grid; }",
        ".a:hovr .b { opacity: 2 !important; } @import 'x'; @wat { } .a { transition: width 900ms; color: rgb(1,2,3); }",
        ".a { font-weight: 300; padding: huge; } .b { ",
    ];
    for src in bad {
        for d in run(src) {
            let all =
                format!("{} {}", d.message, d.help.clone().unwrap_or_default()).to_lowercase();
            assert!(!all.contains("failed"), "{}", d.render());
            assert!(!d.message.is_empty());
        }
    }
}

#[test]
fn errors_stop_the_output_and_warnings_do_not() {
    assert!(wolkit::style::compile(".a { opacity: 1; }").is_ok());
    assert!(
        wolkit::style::compile(".a { opacity: 1; opacity: 0.5; }").is_ok(),
        "WK-013 is a warning"
    );
    let e = wolkit::style::compile(".a { color: #fff; }").unwrap_err();
    assert!(e.message.contains("WK-001"), "{e}");
    let c = wolkit::style::compile_sources(&[("a.wolcss", ".a { opacity: 1; opacity: 0.5; }")])
        .unwrap();
    assert_eq!(c.warnings.len(), 1);
}

#[test]
fn render_shows_the_line_a_caret_and_the_suggestion() {
    let ds = run(".ok { opacity: 1; }\n.a {\n  color: acent;\n}\n");
    let d = find(&ds, "WK-007");
    let text = d.render();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        "t.wolcss:3:10: error[WK-007]: `acent` is not a colour token."
    );
    assert_eq!(lines[2], "3 |   color: acent;");
    assert_eq!(lines[3], "  |          ^^^^^");
    assert!(lines[4].contains("help: did you mean `accent`?"));
}

#[test]
fn the_pragma_lifts_only_the_scale() {
    let ds = run(
        "/* wolkit: off-scale */\n.a { padding: 13px; border-radius: 7px; color: #fff; font-size: 9px; }",
    );
    let ids: Vec<&str> = ds.iter().map(|d| d.id).collect();
    assert!(
        !ids.contains(&"WK-005") && !ids.contains(&"WK-006"),
        "{ids:?}"
    );
    assert!(
        ids.contains(&"WK-001") && ids.contains(&"WK-003") && ids.contains(&"WK-020"),
        "{ids:?}"
    );
}

fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        if name == "target" || name == ".git" || name == "node_modules" {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "wolcss") {
            out.push(p);
        }
    }
}

/// Opt-outs are counted: no sheet in the tree uses the pragma today.
#[test]
fn off_scale_pragma_ratchet() {
    const BASELINE: usize = 0;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = vec![];
    walk(&root.join("crates/wolkit"), &mut files);
    walk(&root.join("wolos-crystal"), &mut files);
    let mut n = 0;
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap_or_default();
        let ds = run(&src);
        let errs: Vec<_> = ds.iter().filter(|d| d.is_error()).collect();
        assert!(errs.is_empty(), "{}: {}", f.display(), errs[0].render());
        n += ds.iter().filter(|d| d.id == OPT_OUT).count();
    }
    #[allow(clippy::absurd_extreme_comparisons)]
    let over = n > BASELINE;
    assert!(
        !over,
        "{n} sheets opt out of the scale; the ratchet is {BASELINE}. Lower it when you remove one, never raise it."
    );
}
