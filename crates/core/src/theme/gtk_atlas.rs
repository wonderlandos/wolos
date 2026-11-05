// SPDX-License-Identifier: BSD-2-Clause
//! The GTK 3 colours the Atlas engine samples, in the WolOS tokens.
//!
//! The engine's GTK mode does not paint from a palette: at every theme change it builds a style
//! context for a CSS selector ("menu menuitem:hover", "textview.view", ...), renders the node into a
//! 24 x 24 bitmap and reads the average (`ui/gtk/gtk_color_mixers.cc`, `gtk_util.cc`:
//! `GetBgColor`, `GetFgColor`, `GetBorderColor`, `GetSeparatorColor`). Each row of [`ROWS`] is one
//! such read: the engine colour it feeds, the selector, what is read and the WolOS token that must
//! come out. [`css`] writes one rule per row, so the engine gets the exact token whatever the base
//! theme (adw-gtk3) derives.
//!
//! Every node the engine builds for a sample carries one fixed class, [`ENGINE_CLASS`] (an
//! identifier of the engine, kept as it is; `AppendCssNodeToStyleContext` adds it so themes can
//! style the browser's samples on their own). The rules name that class on each node, so
//! they reach the browser's samples and no other GTK 3 app: Files, Settings and the rest keep the
//! look [`super::apps`] gives them.
//!
//! The table is also the contract with `tools/theme/atlas-gtk3-sample.py`, which reads the same
//! selectors with real GTK 3 and compares them with [`expected`] (the journey `atlas-theme.yaml`
//! and the dev machine run it). `dump` writes the table and the generated theme for it.

use super::ResolvedTheme;
use shared::theme::{Color, Variant};

/// What the engine reads from the node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// `GetBgColor`: the background rendered over its parents.
    Bg,
    /// `GetFgColor`: the CSS `color`.
    Fg,
    /// `GetBorderColor`: the frame rendered on its own.
    Border,
    /// `GetSeparatorColor`: background and frame of a separator.
    Separator,
    /// `gtk_style_context_lookup_color`: a named colour (`@define-color`).
    Named,
}

/// Where the expected colour comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    /// The token itself, flattened over the surface.
    Token(&'static str),
    /// The token (translucent) painted over another token: hairlines on a card or a control.
    Over(&'static str, &'static str),
    /// The accent at the selection alpha ([`SELECTION_LIGHT`], [`SELECTION_DARK`]) over a token.
    Selection(&'static str),
    /// The accent as ink: `--accent-ink`, the accent shaded until it reads on the card.
    AccentInk,
}

/// One colour the engine samples.
#[derive(Clone, Copy, Debug)]
pub struct Row {
    /// The engine colour this feeds (`ui::kColor...`), for the reader of a failure.
    pub engine: &'static str,
    /// The selector as the engine passes it (GTK 3 spelling), or the colour name for [`Read::Named`].
    pub selector: &'static str,
    pub read: Read,
    pub want: Want,
}

const fn row(engine: &'static str, selector: &'static str, read: Read, want: Want) -> Row {
    Row {
        engine,
        selector,
        read,
        want,
    }
}

/// Accent alpha of a selected row or a text selection (design palette `selection`: 22 % in light,
/// 30 % in dark). The accent follows the preset; the alpha is the design's.
pub const SELECTION_LIGHT: f64 = 0.22;
/// [`SELECTION_LIGHT`] for the dark theme.
pub const SELECTION_DARK: f64 = 0.30;

/// The class the engine puts on every node of a sample. An engine identifier: it cannot be renamed
/// here, and the rules only work with it.
pub const ENGINE_CLASS: &str = "chromium";

const FOCUSED_CELL: &str = "treeview.view treeview.view.cell:selected:focus";
const FOCUSED_SELECTION: &str =
    "textview.view:focus:focus-within text:focus:focus-within selection:focus:focus-within";
const HEADER: &str = "headerbar.header-bar.titlebar";

/// The rows, in the order `AddGtkNativeColorMixer` reads them.
pub const ROWS: &[Row] = &[
    // Core colours.
    row(
        "kColorPrimaryBackground",
        "",
        Read::Bg,
        Want::Token("surface"),
    ),
    row(
        "kColorPrimaryForeground",
        "label",
        Read::Fg,
        Want::Token("text"),
    ),
    row(
        "kColorSecondaryForeground",
        "label:disabled",
        Read::Fg,
        Want::Token("dim"),
    ),
    row(
        "kColorAccent",
        FOCUSED_CELL,
        Read::Bg,
        Want::Token("accent"),
    ),
    row(
        "kColorButtonForegroundProminent",
        "treeview.view treeview.view.cell:selected:focus label",
        Read::Fg,
        Want::Token("accentText"),
    ),
    row(
        "kColorMidground",
        "separator.horizontal",
        Read::Separator,
        Want::Over("line", "surface"),
    ),
    // The frame (the vertical tab strip paints with it: the sidebar) and the toolbar.
    row(
        "kColorFrameActive",
        HEADER,
        Read::Bg,
        Want::Token("sidebar"),
    ),
    row(
        "kColorFrameInactive",
        "headerbar.header-bar.titlebar:backdrop",
        Read::Bg,
        Want::Token("sidebar"),
    ),
    row(
        "kColorFrameCaptionForegroundActive",
        "headerbar.header-bar.titlebar label.title",
        Read::Fg,
        Want::Token("text"),
    ),
    // Read by the Atlas colour mixer (patch 0070): the toolbar is glass over the surface.
    row(
        "AtlasToolbarBackground",
        "toolbar",
        Read::Bg,
        Want::Token("glass"),
    ),
    // Menus and popovers.
    row(
        "kColorMenuBackground",
        "menu",
        Read::Bg,
        Want::Token("card"),
    ),
    row(
        "kColorMenuBorder",
        "menu",
        Read::Border,
        Want::Over("line", "card"),
    ),
    row(
        "kColorMenuItemForeground",
        "menu menuitem label",
        Read::Fg,
        Want::Token("text"),
    ),
    row(
        "kColorMenuItemForegroundDisabled",
        "menu menuitem:disabled label",
        Read::Fg,
        Want::Token("dim"),
    ),
    row(
        "kColorMenuItemForegroundSecondary",
        "menu menuitem accelerator",
        Read::Fg,
        Want::Token("dim"),
    ),
    row(
        "kColorMenuSelectionBackground",
        "menu menuitem:hover",
        Read::Bg,
        Want::Selection("card"),
    ),
    row(
        "kColorMenuSeparator",
        "menu separator.horizontal",
        Read::Separator,
        Want::Over("line", "card"),
    ),
    row(
        "kColorTooltipBackground",
        "tooltip.background",
        Read::Bg,
        Want::Token("card"),
    ),
    // Buttons and fields (the omnibox is a field on `control`).
    row(
        "kColorButtonBackground",
        "button",
        Read::Bg,
        Want::Token("control"),
    ),
    row(
        "kColorButtonBorder",
        "button",
        Read::Border,
        Want::Over("line", "control"),
    ),
    row(
        "kColorButtonForeground",
        "button.text-button label",
        Read::Fg,
        Want::Token("text"),
    ),
    row(
        "kColorTextfieldBackground",
        "textview.view",
        Read::Bg,
        Want::Token("control"),
    ),
    row(
        "kColorTextfieldForeground",
        "textview.view text",
        Read::Fg,
        Want::Token("text"),
    ),
    row(
        "kColorTextfieldForegroundPlaceholder",
        "placeholder_text_color",
        Read::Named,
        Want::Token("dim"),
    ),
    row(
        "kColorFocusableBorderUnfocused",
        "entry",
        Read::Border,
        Want::Over("line", "control"),
    ),
    row(
        "kColorItemHighlight",
        "entry:focus",
        Read::Border,
        Want::Token("accent"),
    ),
    row(
        "kColorTextSelectionBackground",
        FOCUSED_SELECTION,
        Read::Bg,
        Want::Selection("control"),
    ),
    row(
        "kColorTextSelectionForeground",
        FOCUSED_SELECTION,
        Read::Fg,
        Want::Token("text"),
    ),
    row("kColorThrobber", "spinner", Read::Fg, Want::Token("accent")),
    row(
        "kColorIcon",
        "button.flat.scale image",
        Read::Fg,
        Want::Token("text"),
    ),
    row(
        "kColorToggleButtonTrackOn",
        "button.text-button.toggle:checked",
        Read::Bg,
        Want::Token("accent"),
    ),
    row(
        "kColorToggleButtonTrackOff",
        "button.text-button.toggle",
        Read::Bg,
        Want::Token("control"),
    ),
    row(
        "kColorLinkForegroundDefault",
        "label.link:link",
        Read::Fg,
        Want::AccentInk,
    ),
];

/// The colour a row must produce in theme `t`, opaque; an error names the theme and the token
/// when a token the row needs is missing or does not parse (no silent white).
pub fn try_want(t: &ResolvedTheme, w: Want) -> Result<Color, String> {
    let raw = |n: &str| -> Result<Color, String> {
        t.tokens
            .get(n)
            .and_then(|v| Color::parse(v))
            .ok_or_else(|| format!("theme {}: token `{n}` is missing or not a colour", t.id))
    };
    let surface = raw("surface")?.over(Color::WHITE);
    let tok = |n: &str| raw(n).map(|c| c.over(surface));
    Ok(match w {
        Want::Token(n) => tok(n)?,
        Want::Over(n, base) => raw(n)?.over(tok(base)?),
        Want::Selection(base) => {
            let alpha = if t.variant == Variant::Dark {
                SELECTION_DARK
            } else {
                SELECTION_LIGHT
            };
            Color {
                a: alpha,
                ..tok("accent")?
            }
            .over(tok(base)?)
        }
        Want::AccentInk => {
            let get = |n: &str| t.tokens.get(n).map_or("", String::as_str);
            shared::theme::accent_ink(get("accent"), get("card"), get("surface"))
                .and_then(|h| Color::parse(&h))
                .map_or_else(|| tok("accent"), Ok)?
        }
    })
}

/// [`try_want`] for a theme that passed the contract (every catalogue theme does): a missing token
/// is a bug, so it stops with its name.
pub fn want(t: &ResolvedTheme, w: Want) -> Color {
    try_want(t, w).unwrap_or_else(|e| panic!("{e}"))
}

/// `(row, expected colour)` for theme `t`.
pub fn expected(t: &ResolvedTheme) -> Vec<(&'static Row, Color)> {
    ROWS.iter().map(|r| (r, want(t, r.want))).collect()
}

/// The selector with [`ENGINE_CLASS`] on every node: "menu menuitem:hover" becomes
/// "menu.<class> menuitem.<class>:hover". The engine roots every selector in `window.background`.
/// GTK 3 has no `:focus-within` (the engine drops the state there too, and the CSS parser would drop
/// the whole rule), so it is left out.
fn scoped(selector: &str) -> String {
    let mut nodes = vec![format!("window.background.{ENGINE_CLASS}")];
    for node in selector.split_whitespace() {
        let node = node.replace(":focus-within", "");
        nodes.push(match node.find(':') {
            Some(i) => format!("{}.{ENGINE_CLASS}{}", &node[..i], &node[i..]),
            None => format!("{node}.{ENGINE_CLASS}"),
        });
    }
    nodes.join(" ")
}

/// The declarations that make a node read `c`.
fn paint(read: Read, c: &str) -> String {
    match read {
        Read::Bg => format!("background-color: {c}; background-image: none;"),
        Read::Fg => format!("color: {c};"),
        // Width and style are ours too, so the colour holds on any base theme (or none).
        Read::Border => format!("border-style: solid; border-width: 1px; border-color: {c};"),
        // A separator is a 1 px box: its own fill, no frame colour of another.
        Read::Separator => {
            format!("background-color: {c}; background-image: none; border-color: {c};")
        }
        Read::Named => String::new(),
    }
}

/// The rules, appended to the GTK 3 theme after adw-gtk3: one per row, each reaching the browser's
/// samples only, plus the named colours the engine looks up.
pub fn css(t: &ResolvedTheme) -> String {
    let mut s = String::from("/* Atlas: the colours the engine samples (gtk_atlas.rs). */\n");
    // Rows that read the same node (a background and a frame) share one rule.
    let mut rules: Vec<(String, String)> = Vec::new();
    for (r, c) in expected(t) {
        let hex = c.to_hex();
        if r.read == Read::Named {
            s += &format!("@define-color {} {hex};\n", r.selector);
            continue;
        }
        let (sel, decl) = (scoped(r.selector), paint(r.read, &hex));
        match rules.iter_mut().find(|(k, _)| *k == sel) {
            Some((_, d)) => *d = format!("{d} {decl}"),
            None => rules.push((sel, decl)),
        }
    }
    for (sel, decl) in rules {
        s += &format!("{sel} {{ {decl} }}\n");
    }
    s
}

/// Rows and expectations as JSON for `tools/theme/atlas-gtk3-sample.py`.
pub fn rows_json(t: &ResolvedTheme) -> String {
    let rows: Vec<String> = expected(t)
        .into_iter()
        .map(|(r, c)| {
            let read = match r.read {
                Read::Bg => "bg",
                Read::Fg => "fg",
                Read::Border => "border",
                Read::Separator => "separator",
                Read::Named => "named",
            };
            format!(
                "{{\"engine\":\"{}\",\"selector\":\"{}\",\"read\":\"{read}\",\"expect\":\"{}\"}}",
                r.engine,
                r.selector,
                c.to_hex()
            )
        })
        .collect();
    format!(
        "{{\"id\":\"{}\",\"dark\":{},\"rows\":[{}]}}\n",
        t.id,
        t.variant == Variant::Dark,
        rows.join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::super::ThemeCatalog;
    use super::*;
    use std::path::Path;

    fn theme(id: &str) -> ResolvedTheme {
        let mut cat = ThemeCatalog::builtin();
        for v in crate::theme::accent::variants() {
            cat = cat.with(v);
        }
        cat.resolve(id).unwrap()
    }

    /// The declarations of the rule for `scoped(selector)` in `css`, or None.
    fn rule<'a>(css: &'a str, selector: &str) -> Option<&'a str> {
        let head = format!("{} {{ ", scoped(selector));
        let at = css.find(&head)? + head.len();
        let end = css[at..].find(" }\n")? + at;
        Some(&css[at..end])
    }

    /// Every token the engine samples reaches it exactly: in both built-in themes, and in every
    /// accent preset, each row has a rule (or a named colour) carrying the colour the table wants,
    /// and that colour is the token's value.
    #[test]
    fn every_sampled_row_carries_its_token() {
        let ids = ["wol-light", "wol-dark", "wol-light-blue", "wol-dark-pink"];
        for id in ids {
            let t = theme(id);
            let css = css(&t);
            for (r, c) in expected(&t) {
                let hex = c.to_hex();
                if r.read == Read::Named {
                    assert!(
                        css.contains(&format!("@define-color {} {hex};", r.selector)),
                        "{id}: {} not defined as {hex}",
                        r.selector
                    );
                    continue;
                }
                let decl = rule(&css, r.selector)
                    .unwrap_or_else(|| panic!("{id}: no rule for {} ({})", r.selector, r.engine));
                let want = paint(r.read, &hex);
                assert!(
                    decl.contains(&want),
                    "{id}: {} ({}) is `{decl}`, want `{want}`",
                    r.selector,
                    r.engine
                );
            }
        }
    }

    /// The tokens a row names are the tokens' own values, not a derivation of another: the surface,
    /// the card, the control, the text, the dim text, the accent and its text come out as the
    /// built-in theme files say.
    #[test]
    fn the_exact_tokens_reach_the_engine() {
        for id in ["wol-light", "wol-dark"] {
            let t = theme(id);
            let exact = |tok: &str| Color::parse(&t.tokens[tok]).unwrap().to_hex();
            let got = |engine: &str| {
                let (_, c) = expected(&t)
                    .into_iter()
                    .find(|(r, _)| r.engine == engine)
                    .unwrap();
                c.to_hex()
            };
            assert_eq!(got("kColorPrimaryBackground"), exact("surface"), "{id}");
            assert_eq!(got("kColorMenuBackground"), exact("card"), "{id}");
            assert_eq!(got("kColorTooltipBackground"), exact("card"), "{id}");
            assert_eq!(got("kColorTextfieldBackground"), exact("control"), "{id}");
            assert_eq!(got("kColorButtonBackground"), exact("control"), "{id}");
            assert_eq!(got("kColorPrimaryForeground"), exact("text"), "{id}");
            assert_eq!(got("kColorSecondaryForeground"), exact("dim"), "{id}");
            assert_eq!(got("kColorAccent"), exact("accent"), "{id}");
            assert_eq!(got("kColorItemHighlight"), exact("accent"), "{id}");
            assert_eq!(
                got("kColorButtonForegroundProminent"),
                exact("accentText"),
                "{id}"
            );
        }
    }

    /// Every token a row needs resolves in every built-in and accent theme: none falls back to
    /// white or to the surface.
    #[test]
    fn every_row_token_resolves_in_every_theme() {
        let mut cat = ThemeCatalog::builtin();
        for v in crate::theme::accent::variants() {
            cat = cat.with(v);
        }
        let specs = cat.list();
        assert!(specs.len() >= 37, "{} themes", specs.len());
        for spec in specs {
            let t = cat.resolve(&spec.id).unwrap();
            for r in ROWS {
                try_want(&t, r.want).unwrap_or_else(|e| panic!("{e} (row {})", r.engine));
            }
        }
        let mut t = theme("wol-light");
        t.tokens.remove("control");
        let err = try_want(&t, Want::Token("control")).unwrap_err();
        assert!(
            err.contains("`control`") && err.contains("wol-light"),
            "{err}"
        );
    }

    /// The rules reach the browser only: every selector carries the engine's class on every node.
    #[test]
    fn rules_are_scoped_to_the_browser() {
        let css = css(&theme("wol-light"));
        for line in css.lines().filter(|l| l.contains(" { ")) {
            let sel = line.split(" { ").next().unwrap();
            for node in sel.split_whitespace() {
                assert!(
                    node.contains(&format!(".{ENGINE_CLASS}")),
                    "`{node}` in `{line}` is not scoped"
                );
            }
        }
        assert_eq!(scoped(""), "window.background.chromium");
        assert_eq!(
            scoped("menu menuitem:hover"),
            "window.background.chromium menu.chromium menuitem.chromium:hover"
        );
        assert_eq!(
            scoped("textview.view:focus:focus-within text:focus:focus-within"),
            "window.background.chromium textview.view.chromium:focus text.chromium:focus"
        );
    }

    /// An accent preset moves the accent rows and nothing else: the pill (selection), the ring and
    /// the accent read differently, the surfaces stay.
    #[test]
    fn an_accent_preset_moves_the_accent_rows_only() {
        let (a, b) = (theme("wol-light"), theme("wol-light-red"));
        assert_ne!(a.tokens["accent"], b.tokens["accent"]);
        let (ea, eb) = (expected(&a), expected(&b));
        for ((r, ca), (_, cb)) in ea.iter().zip(&eb) {
            let accent_row = matches!(r.want, Want::Selection(_) | Want::AccentInk)
                || matches!(r.want, Want::Token("accent"));
            if accent_row {
                assert_ne!(
                    ca.to_hex(),
                    cb.to_hex(),
                    "{} must follow the accent",
                    r.engine
                );
            } else {
                assert_eq!(ca.to_hex(), cb.to_hex(), "{} must not move", r.engine);
            }
        }
    }

    /// The generated GTK 3 theme carries the rules, after adw-gtk3's own, in the light and in the
    /// dark file, and the user's gtk.css stays free of them.
    #[test]
    fn the_theme_carries_the_rules() {
        for id in ["wol-light", "wol-dark"] {
            let t = theme(id);
            let files = super::super::apps::render(&t, Path::new("/x"));
            for rel in [
                super::super::apps::GTK3_THEME_CSS,
                super::super::apps::GTK3_THEME_DARK_CSS,
            ] {
                let f = files.iter().find(|f| f.rel == rel).unwrap();
                let at = f
                    .text
                    .find("/* Atlas: the colours the engine samples")
                    .unwrap();
                assert!(
                    at > f.text.find("@import").unwrap(),
                    "{id} {rel}: rules after the import"
                );
                assert!(f.text.contains(&css(&t)), "{id} {rel}");
            }
            let user = files.iter().find(|f| f.rel == "gtk-3.0/gtk.css").unwrap();
            assert!(!user.text.contains(ENGINE_CLASS), "{id}");
        }
    }

    /// `WOL_DUMP_GTK3=<dir> cargo test -p core dump_for_the_sampler` writes, per theme,
    /// `<dir>/themes/WolOS-<id>-gtk3/gtk-3.0/{gtk.css,gtk-dark.css}` and `<dir>/<id>.json` for
    /// `tools/theme/atlas-gtk3-sample.py`. Without the variable it does nothing.
    #[test]
    fn dump_for_the_sampler() {
        let Some(dir) = std::env::var_os("WOL_DUMP_GTK3") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let mut cat = ThemeCatalog::builtin();
        for v in crate::theme::accent::variants() {
            cat = cat.with(v);
        }
        for spec in cat.list() {
            let id = spec.id.as_str();
            let t = theme(id);
            let theme_dir = dir
                .join("themes")
                .join(super::super::apps::gtk3_theme_name(&t))
                .join("gtk-3.0");
            std::fs::create_dir_all(&theme_dir).unwrap();
            let files = super::super::apps::render(&t, Path::new("/x"));
            for (rel, name) in [
                (super::super::apps::GTK3_THEME_CSS, "gtk.css"),
                (super::super::apps::GTK3_THEME_DARK_CSS, "gtk-dark.css"),
            ] {
                let f = files.iter().find(|f| f.rel == rel).unwrap();
                std::fs::write(theme_dir.join(name), &f.text).unwrap();
            }
            std::fs::write(dir.join(format!("{id}.json")), rows_json(&t)).unwrap();
        }
    }
}
