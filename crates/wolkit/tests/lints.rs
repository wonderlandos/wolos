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

/// Run `src` and check lint `id` shows where it should: the caret sits under `under` and the suggestion mentions `help`.
fn check_lint(id: &str, src: &str, under: &str, help: &str) {
    let ds = run(src);
    let d = find(&ds, id);
    assert_eq!(
        highlighted(d),
        under,
        "the caret points at the offending text: {}",
        d.render()
    );
    assert_eq!(d.file, "t.wolcss");
    assert!(d.line >= 1 && d.col >= 1);
    let hint = d.help.clone().unwrap_or_default();
    assert!(hint.contains(help), "help `{hint}` should mention `{help}`");
    let r = d.render();
    assert!(
        r.starts_with(&format!("t.wolcss:{}:{}: ", d.line, d.col)),
        "{r}"
    );
    assert!(r.contains(&format!("[{id}]")));
    assert!(r.contains('^'));
    // The severity matches the catalogue.
    assert_eq!(d.severity, def(id).unwrap().severity);
}

#[test]
fn wk_001_hex_literal() {
    check_lint("WK-001", ".a { background: #fff; }", "#fff", "token");
}
#[test]
fn wk_002_important() {
    check_lint(
        "WK-002",
        ".a { opacity: 1 !important; }",
        "!important",
        "order",
    );
}
#[test]
fn wk_003_font_too_small() {
    check_lint("WK-003", ".a { font-size: 10px; }", "10px", "11px");
}
#[test]
fn wk_004_duration_too_long() {
    check_lint(
        "WK-004",
        ".a { transition: opacity 600ms; }",
        "600ms",
        "slow",
    );
}
#[test]
fn wk_005_off_scale_spacing() {
    check_lint("WK-005", ".a { padding: 13px; }", "13px", "nearest is `md`");
}
#[test]
fn wk_006_off_scale_radius() {
    check_lint(
        "WK-006",
        ".a { border-radius: 7px; }",
        "7px",
        "nearest is `s`",
    );
}
#[test]
fn wk_007_unknown_token() {
    check_lint(
        "WK-007",
        ".a { color: acent; }",
        "acent",
        "did you mean `accent`",
    );
}
#[test]
fn wk_008_unknown_property() {
    check_lint(
        "WK-008",
        ".a { colour: text; }",
        "colour",
        "did you mean `color`",
    );
}
#[test]
fn wk_009_unsupported_grid() {
    check_lint("WK-009", ".a { display: grid; }", "grid", "flex");
}
#[test]
fn wk_010_bad_value() {
    check_lint("WK-010", ".a { opacity: 2; }", "2", "");
}
#[test]
fn wk_011_unknown_state() {
    check_lint(
        "WK-011",
        ".a:hovr { opacity: 1; }",
        "hovr",
        "did you mean `:hover`",
    );
}
#[test]
fn wk_012_selector_form() {
    check_lint(
        "WK-012",
        ".a .b { opacity: 1; }",
        ".a .b",
        ".parent:state > .child",
    );
}
#[test]
fn wk_013_duplicate_property() {
    check_lint(
        "WK-013",
        ".a { opacity: 1; opacity: 0.5; }",
        "opacity",
        "later",
    );
}
#[test]
fn wk_014_too_many_variants() {
    check_lint(
        "WK-014",
        ".a { opacity: 1; } .a:hover { opacity: 1; } .a:active { opacity: 1; } .a:focus { opacity: 1; } .a:disabled { opacity: 1; } \
     .a:selected { opacity: 1; } .a:checked { opacity: 1; } .a:expanded { opacity: 1; } .a:empty { opacity: 1; }",
        ".a:empty",
        "two classes",
    );
}
#[test]
fn wk_015_empty_rule() {
    check_lint("WK-015", ".a { }", ".a", "declaration");
}
#[test]
fn wk_016_table_limit() {
    check_lint(
        "WK-016",
        &(0..=4096)
            .map(|i| format!(".c{} {{ opacity: 1; }}\n", name(i)))
            .collect::<String>()
            .to_string(),
        ".c-names-4096",
        "split",
    );
}
#[test]
fn wk_017_bad_weight() {
    check_lint(
        "WK-017",
        ".a { font-weight: 300; }",
        "300",
        "400, 500, 600 and 700",
    );
}
#[test]
fn wk_018_syntax() {
    check_lint(
        "WK-018",
        ".a { opacity 1; }",
        "opacity 1",
        "property: value",
    );
}
#[test]
fn wk_019_unknown_name() {
    check_lint("WK-019", ".a { padding: huge; }", "huge", "names are");
}
#[test]
fn wk_020_opt_out() {
    check_lint(
        "WK-020",
        "/* wolkit: off-scale */\n.a { padding: 13px; }",
        "/*",
        "counted",
    );
}
#[test]
fn wk_021_merged_variant() {
    check_lint(
        "WK-021",
        ".a { opacity: 1; } .a { color: dim; }",
        ".a",
        "one block",
    );
}
#[test]
fn wk_022_not_animatable() {
    check_lint(
        "WK-022",
        ".a { transition: width fast; }",
        "width",
        "colours",
    );
}
#[test]
fn wk_023_color_function() {
    check_lint(
        "WK-023",
        ".a { color: rgb(1, 2, 3); }",
        "rgb(",
        "alpha(token",
    );
}
#[test]
fn wk_024_conflicting_media() {
    check_lint(
        "WK-024",
        "@dark @light { .a { opacity: 1; } }",
        "@dark @light",
        "two blocks",
    );
}

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
    let over = n.saturating_sub(BASELINE);
    assert_eq!(
        over, 0,
        "{n} sheets opt out of the scale; the ratchet is {BASELINE}. Lower it when you remove one, never raise it."
    );
}
