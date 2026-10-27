//! Theme -> GTK CSS. Catalog, contract and active-theme file come from `wol_core::theme` / `shared::theme`;
//! this module only adds the kit stylesheet.
pub use shared::theme::Variant;
use wol_core::theme::{self as core_theme, ThemeCatalog, active};
pub type Theme = core_theme::ResolvedTheme;

thread_local! {
    static CATALOG: ThemeCatalog = core_theme::accent::variants()
        .into_iter()
        .fold(ThemeCatalog::load(&core_theme::search_paths(&core_theme::config_home())), ThemeCatalog::with);
}

/// Saved id (None = follow appearance) -> resolved theme; unknown ids fall back to the variant default.
/// `wol-auto-<accent>` follows `prefer_dark` (`wol_core::theme::accent`).
pub fn resolve(saved: Option<&str>, prefer_dark: bool) -> Theme {
    let saved = core_theme::accent::effective(saved, prefer_dark);
    let id = active::effective(saved.as_deref(), prefer_dark);
    CATALOG.with(|c| {
        c.resolve(&id)
            .or_else(|_| {
                c.resolve(
                    if prefer_dark {
                        Variant::Dark
                    } else {
                        Variant::Light
                    }
                    .default_theme(),
                )
            })
            .expect("built-in themes resolve")
    })
}

/// `$XDG_CONFIG_HOME/wolos/theme`.
pub fn active_path() -> std::path::PathBuf {
    active::default_file(&core_theme::config_home())
}

pub fn saved() -> Option<String> {
    active::get(&active_path())
}

/// Saves the active theme, journaled (`wol undo`, Settings > Restore).
pub fn save(id: &str) -> std::io::Result<()> {
    journal_theme(Some(&format!("{id}\n")))
}

/// Back to following the appearance (no theme file), journaled.
pub fn clear() -> std::io::Result<()> {
    journal_theme(None)
}

fn journal_theme(text: Option<&str>) -> std::io::Result<()> {
    let home = core_theme::config_home();
    wol_core::journal::Journal::for_config(&home)
        .save(
            "theme",
            "settings",
            &[(shared::theme::ACTIVE_FILE, text)],
            wol_core::journal::now(),
        )
        .map(drop)
}

/// Kit stylesheet: the only visual style (HIG). Colours via `var(--token)`.
pub const KIT_CSS: &str = include_str!("kit/kit.css");

/// Token block, then the kit CSS, then surface glue.
pub fn stylesheet(t: &Theme, extra: &str) -> String {
    let mut s = t.css();
    s.push_str(KIT_CSS);
    s.push_str(extra);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_builtins_and_falls_back() {
        assert_eq!(resolve(None, true).variant, Variant::Dark);
        assert_eq!(resolve(Some("garbage"), false).variant, Variant::Light);
        assert_eq!(resolve(Some("wol-dark"), false).id, "wol-dark");
        assert_eq!(resolve(Some("wol-light-blue"), true).id, "wol-light-blue");
        assert_eq!(resolve(Some("wol-auto-green"), true).id, "wol-dark-green");
        assert_eq!(resolve(Some("wol-auto-green"), false).id, "wol-light-green");
    }

    #[test]
    fn stylesheet_has_tokens_then_kit() {
        let css = stylesheet(&resolve(Some("wol-light"), false), "");
        for t in shared::theme::TOKENS {
            assert!(
                css.contains(&format!("{}: ", shared::theme::css_var(t))),
                "{t}"
            );
        }
        assert!(css.find("--accent:").unwrap() < css.find(".glass-panel").unwrap());
    }

    #[test]
    fn kit_css_uses_tokens_not_raw_accent() {
        assert!(KIT_CSS.contains("var(--accent)"));
        for raw in ["#0a84ff", "#7c5cff", "#7C5CFF"] {
            assert!(!KIT_CSS.contains(raw), "{raw}");
        }
    }
}
