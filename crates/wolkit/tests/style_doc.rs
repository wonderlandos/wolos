// SPDX-License-Identifier: BSD-2-Clause
//! `docs/modules/wolkit-style.md` is generated from the property table, the tokens and the lint
//! catalogue. This test fails when the file is not what the code generates; regenerate with
//! `WOLKIT_BLESS=1 cargo test -p wolkit --test style_doc` or `wolkit-stylec --doc`.
fn path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/modules/wolkit-style.md")
}

#[test]
fn the_style_doc_is_what_the_code_generates() {
    let want = wolkit::style::doc::render();
    if std::env::var("WOLKIT_BLESS").is_ok() {
        std::fs::write(path(), &want).unwrap();
        return;
    }
    let got = std::fs::read_to_string(path()).unwrap_or_default();
    if got != want {
        let at = got
            .lines()
            .zip(want.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(got.lines().count().min(want.lines().count()));
        panic!(
            "docs/modules/wolkit-style.md is stale (first difference at line {}): regenerate with WOLKIT_BLESS=1 cargo test -p wolkit --test style_doc\n  file: {}\n  code: {}",
            at + 1,
            got.lines().nth(at).unwrap_or("<end>"),
            want.lines().nth(at).unwrap_or("<end>")
        );
    }
}

#[test]
fn the_doc_follows_the_house_style() {
    let d = wolkit::style::doc::render();
    assert!(d.ends_with('\n') || !d.contains("\r"));
    assert!(!d.contains("\t"), "RU-058 style: no tabs");
    assert!(!d.lines().any(|l| l.ends_with(' ')), "no trailing spaces");
    // Mentions every property, token, lint and condition.
    for p in wolkit::style::prop::PROPS {
        assert!(d.contains(&format!("`{}`", p.name)), "{} missing", p.name);
    }
    for t in wolkit::style::tokens::Token::NAMES {
        assert!(d.contains(&format!("`{t}`")));
    }
    for m in wolkit::style::select::MEDIA {
        assert!(d.contains(&format!("`@{m}`")));
    }
    // The wording rules: no upstream names, no em dashes.
    assert!(!d.contains('\u{2014}'), "no em dashes");
}
