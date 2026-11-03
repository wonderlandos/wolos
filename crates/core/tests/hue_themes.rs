// SPDX-License-Identifier: BSD-2-Clause
//! Themes written by wol-hue (`hue/`, docs/modules/hue.md) are ordinary WolOS themes: every
//! fixture in `tests/hue_themes/` (theme.json text the tool generated from its test scenes,
//! `make -C hue` then `wol-hue from-wallpaper ... --no-apply`, wallpaper key dropped) parses with
//! `ThemeSpec::parse`, carries 13 tokens and 16 terminal colours, resolves and passes the contrast
//! contract `shared::theme::validate`.

use std::path::Path;
use wol_core::theme::{ThemeCatalog, ThemeSpec};

#[test]
fn hue_themes_parse_resolve_and_pass_the_contrast_contract() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/hue_themes");
    let mut n = 0;
    let mut variants = std::collections::BTreeSet::new();
    for e in std::fs::read_dir(&dir).expect("tests/hue_themes") {
        let path = e.expect("entry").path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read fixture");
        let spec =
            ThemeSpec::parse(&text, &dir).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(
            spec.tokens.len(),
            shared::theme::TOKENS.len(),
            "{}",
            spec.id
        );
        assert_eq!(
            spec.terminal.as_ref().map(Vec::len),
            Some(shared::theme::TERMINAL_COLORS),
            "{}",
            spec.id
        );
        let errors = shared::theme::validate(|t| spec.tokens.get(t).cloned());
        assert!(errors.is_empty(), "{}: {errors:?}", spec.id);
        let id = spec.id.clone();
        variants.insert(spec.variant.name());
        let resolved = ThemeCatalog::builtin()
            .with(spec)
            .resolve(&id)
            .expect("resolves");
        assert!(resolved.check().is_empty(), "{id}: {:?}", resolved.check());
        n += 1;
    }
    assert_eq!(n, 8, "fixtures");
    assert_eq!(variants.len(), 2, "both variants covered");
}
