// SPDX-License-Identifier: BSD-2-Clause
//! Third-party app theming from one resolved WolOS theme: GTK4/libadwaita, GTK3 (adw-gtk3),
//! Qt (qt6ct Fusion palette, Kvantum), foot, Firefox (`firefox`), GtkSourceView (`sourceview`),
//! zsh colours, compositor colours (`hypr`); the console files come from `console`. Pure renderers + `install`
//! (every `generate::consumers` file under a toolkit dir, linked into `$XDG_CONFIG_HOME`/`$XDG_DATA_HOME`
//! without replacing the user's own files) + `gsettings` keys.

use super::ResolvedTheme;
use shared::theme::{Color, Variant};
use std::path::Path;

/// One generated file, relative to the toolkit dir.
#[derive(Clone, Debug, PartialEq)]
pub struct File {
    pub rel: &'static str,
    pub text: String,
}

/// Generated files linked into `$XDG_CONFIG_HOME` at the same relative path.
pub const LINKS: [&str; 9] = [
    "gtk-4.0/gtk.css",
    "gtk-4.0/settings.ini",
    "gtk-3.0/gtk.css",
    "gtk-3.0/settings.ini",
    "qt6ct/qt6ct.conf",
    "foot/foot.ini",
    "Kvantum/kvantum.kvconfig",
    "Kvantum/WolOS/WolOS.kvconfig",
    "Kvantum/WolOS/WolOS.svg",
];
/// GTK 3 colours as a theme (`@import` adw-gtk3 + WolOS colours), linked as
/// `$XDG_DATA_HOME/themes/WolOS-<theme id>`: GTK 3 reloads a theme when its name changes but never
/// rereads the user gtk.css, so a new name per WolOS theme restyles running apps live.
pub const GTK3_THEME_DIR: &str = "gtk3-theme";
pub const GTK3_THEME_CSS: &str = "gtk3-theme/gtk-3.0/gtk.css";
/// The same colours as the dark variant: GTK 3 loads it under prefer-dark and libhandy falls back
/// to Adwaita in dark mode when a theme has none.
pub const GTK3_THEME_DARK_CSS: &str = "gtk3-theme/gtk-3.0/gtk-dark.css";
/// adw-gtk3 install dir (`image/fetch-theme.sh`).
pub const ADW_GTK3_DIR: &str = "/usr/share/themes";

/// `WolOS-<id>-gtk3`: never ends in `-dark`, which libhandy strips and then cannot find.
pub fn gtk3_theme_name(t: &ResolvedTheme) -> String {
    format!("WolOS-{}-gtk3", t.id)
}

/// Generated files linked into `$XDG_DATA_HOME` at the same relative path.
pub const DATA_LINKS: [&str; 2] = super::sourceview::FILES;
/// Sourced by the system zshrc from `$XDG_CONFIG_HOME/wolos/toolkits/` (prompt colours).
pub const SHELL_COLORS: &str = "zsh/colors.zsh";
/// System copy rendered from the default theme (`wolos-iso/build.sh`); users get their own under
/// `$XDG_CONFIG_HOME/wolos/toolkits`.
pub const SYSTEM_DIR: &str = "/usr/share/wolos";
/// Shipped icon theme (`design/icons`, inherits Adwaita); the catalog's `iconTheme` names themes
/// the image does not install, so apps always get this one, like the shell.
pub const ICON_THEME: &str = "WolOS";
pub const FONT: &str = "Inter 10";
pub const MONO_FONT: &str = "JetBrains Mono:size=11";
/// Window controls on the right, close rightmost, the order the compositor draws its own in.
pub const BUTTON_LAYOUT: &str = ":minimize,maximize,close";
/// libadwaita accent presets (`org.gnome.desktop.interface accent-color`) and their colours.
pub const ADW_ACCENTS: [(&str, &str); 9] = [
    ("blue", "#3584e4"),
    ("teal", "#2190a4"),
    ("green", "#3a944a"),
    ("yellow", "#c88800"),
    ("orange", "#ed5b00"),
    ("red", "#e62d42"),
    ("pink", "#d56199"),
    ("purple", "#9141ac"),
    ("slate", "#6f8396"),
];

/// Preset nearest the theme accent by hue; greys map to slate. The settings portal carries it to
/// libadwaita and to Chromium's colour mixer (Helium); gtk.css still sets the exact colour.
pub fn adw_accent(t: &ResolvedTheme) -> &'static str {
    let (h, s) = hue_sat(token_color(t, "accent"));
    if s < 0.2 {
        return "slate";
    }
    let dist = |x: f64| {
        let d = (x - h).abs() % 360.0;
        d.min(360.0 - d)
    };
    ADW_ACCENTS[..8]
        .iter()
        .map(|(n, hex)| {
            (
                *n,
                dist(hue_sat(Color::parse(hex).unwrap_or(Color::WHITE)).0),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or("blue", |x| x.0)
}

/// HSL hue (degrees) and saturation (0..1).
pub(crate) fn hue_sat(c: Color) -> (f64, f64) {
    let [r, g, b] = [c.r, c.g, c.b].map(|v| f64::from(v) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let (d, l) = (max - min, (max + min) / 2.0);
    if d == 0.0 {
        return (0.0, 0.0);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, s)
}
pub const FOOT_ALPHA: &str = "0.9";
/// foot's initial window title (its own default is "foot", while the menu bar says Terminal; QA-LIVE-12).
/// Programs in the terminal still set their own title; the class stays `foot` for window rules.
pub const FOOT_TITLE: &str = "Terminal";
/// Window buttons as Kit AppWindow / hyprbars draw them (kit goldens, pixel edges): 12 px
/// circles, 22 px apart, close centred 23 px in from the window's right edge. Every toolkit matches.
pub use shared::theme::scaffold::{LIGHT_FIRST_CENTRE, LIGHT_PITCH, LIGHT_SIZE};
/// adw-gtk3 6.5 `headerbar { padding: 0 6px }`; Chromium's GTK frame reads the same padding.
const GTK3_HEADER_PADDING: i32 = 6;
/// GtkHeaderBar's default child spacing; Chromium hard-codes it (`kHeaderSpacing = 6`).
const GTK3_BUTTON_SPACING: i32 = 6;

/// User toolkit dir: `$XDG_CONFIG_HOME/wolos/toolkits`.
pub fn user_dir(config_home: &Path) -> std::path::PathBuf {
    config_home.join("wolos/toolkits")
}

struct Pal {
    dark: bool,
    bg: Color,
}

impl Pal {
    fn new(t: &ResolvedTheme) -> Self {
        let dark = t.variant == Variant::Dark;
        let bg = Color::parse(&t.tokens["surface"]).unwrap_or(if dark {
            Color::rgb(30, 30, 34)
        } else {
            Color::WHITE
        });
        Pal { dark, bg }
    }
    /// Token as an opaque colour (alpha flattened over the surface).
    fn c(&self, t: &ResolvedTheme, tok: &str) -> Color {
        t.tokens
            .get(tok)
            .and_then(|v| Color::parse(v))
            .map_or(self.bg, |c| c.over(self.bg))
    }
}

/// Token as an opaque colour (alpha flattened over the surface).
pub(crate) fn token_color(t: &ResolvedTheme, tok: &str) -> Color {
    Pal::new(t).c(t, tok)
}

/// `fg`, moved toward black or white (whichever the background contrasts with) just enough to
/// reach `min` against `bg`.
pub(crate) fn readable(fg: Color, bg: Color, min: f64) -> Color {
    let toward = if Color::contrast(Color::WHITE, bg) >= Color::contrast(Color::rgb(0, 0, 0), bg) {
        Color::WHITE
    } else {
        Color::rgb(0, 0, 0)
    };
    (0..=20)
        .map(|i| mix(fg, toward, f64::from(i) / 20.0))
        .find(|c| Color::contrast(*c, bg) >= min)
        .unwrap_or(toward)
}

pub(crate) fn mix(a: Color, b: Color, t: f64) -> Color {
    let m = |x: u8, y: u8| (f64::from(x) * (1.0 - t) + f64::from(y) * t).round() as u8;
    Color::rgb(m(a.r, b.r), m(a.g, b.g), m(a.b, b.b))
}

fn token<'a>(t: &'a ResolvedTheme, k: &str) -> &'a str {
    t.tokens.get(k).map_or("", String::as_str)
}

pub fn render(t: &ResolvedTheme, dir: &Path) -> Vec<File> {
    let gtk3 = format!(
        "@import url(\"file://{ADW_GTK3_DIR}/{}/gtk-3.0/gtk.css\");\n{}",
        gtk3_theme(t.variant == Variant::Dark),
        gtk3_css(t)
    );
    vec![
        File {
            rel: "Kvantum/kvantum.kvconfig",
            text: "[General]\ntheme=WolOS\n".into(),
        },
        File {
            rel: "Kvantum/WolOS/WolOS.kvconfig",
            text: super::kvantum::kvconfig(t),
        },
        File {
            rel: "Kvantum/WolOS/WolOS.svg",
            text: super::kvantum::svg(t),
        },
        File {
            rel: "gtk-4.0/gtk.css",
            text: gtk4_css(t),
        },
        File {
            rel: "gtk-4.0/settings.ini",
            text: settings_ini(t, false),
        },
        File {
            rel: "gtk-3.0/gtk.css",
            text: header(t) + "/* Colours come from the GTK 3 theme `WolOS-<id>` (gtk-theme-name); a\n   user gtk.css is read once per app, so it must not carry them. */\n",
        },
        File {
            rel: GTK3_THEME_CSS,
            text: gtk3.clone(),
        },
        File {
            rel: GTK3_THEME_DARK_CSS,
            text: gtk3,
        },
        File {
            rel: "gtk-3.0/settings.ini",
            text: settings_ini(t, true),
        },
        File {
            rel: "qt6ct/qt6ct.conf",
            text: qt6ct_conf(dir),
        },
        File {
            rel: "qt6ct/colors/WolOS.conf",
            text: qt_colors(t),
        },
        File {
            rel: "foot/foot.ini",
            text: foot_ini(t),
        },
        File {
            rel: super::firefox::CSS,
            text: super::firefox::chrome_css(t),
        },
        File {
            rel: SHELL_COLORS,
            text: shell_colors(t),
        },
        File {
            rel: super::hypr::HYPR_THEME_FILE,
            text: super::hypr::lua(t),
        },
    ]
    .into_iter()
    .chain(
        super::sourceview::FILES
            .into_iter()
            .zip(super::sourceview::render(t))
            .map(|(rel, text)| File { rel, text }),
    )
    .collect()
}

/// zsh prompt colours for the system zshrc (`WOLOS_PROMPT_*`, `%F{#rrggbb}`): readable on the
/// terminal background (the surface); autosuggestions dimmer but >= 3:1.
fn shell_colors(t: &ResolvedTheme) -> String {
    let mut s = format!(
        "# WolOS {} — generated by `wol theme`; edits are overwritten.\nWOLOS_THEME={}\n",
        t.id, t.id
    );
    // The zshrc reads five; `warn` came with the other shells (`console`).
    for (k, c) in super::console::roles(t)
        .into_iter()
        .filter(|r| r.0 != "warn")
    {
        s += &format!("WOLOS_PROMPT_{}='{}'\n", k.to_uppercase(), c.to_hex());
    }
    s
}

fn defines(pairs: &[(&str, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("@define-color {k} {v};\n"))
        .collect()
}

/// libadwaita colour names (GTK3 `@define-color`, libadwaita >= 1.6 `--name` with dashes).
pub(crate) fn adw_pairs(t: &ResolvedTheme) -> Vec<(&'static str, String)> {
    let p = Pal::new(t);
    let tk = |k| token(t, k).to_string();
    let hx = |k| p.c(t, k).to_hex();
    // Text-use colours (labels, links, icons) must read on cards; fills keep the exact token.
    let fg = |k| readable(p.c(t, k), p.c(t, "card"), 4.5).to_hex();
    vec![
        ("accent_bg_color", tk("accent")),
        ("accent_fg_color", tk("accentText")),
        ("accent_color", fg("accent")),
        ("destructive_bg_color", tk("danger")),
        ("destructive_color", fg("danger")),
        ("success_bg_color", tk("success")),
        ("success_color", fg("success")),
        ("warning_bg_color", tk("warning")),
        ("warning_color", fg("warning")),
        ("error_bg_color", tk("danger")),
        ("error_color", fg("danger")),
        ("window_bg_color", tk("surface")),
        ("window_fg_color", tk("text")),
        ("view_bg_color", tk("card")),
        ("view_fg_color", tk("text")),
        ("headerbar_bg_color", tk("surface")),
        ("headerbar_fg_color", tk("text")),
        ("headerbar_backdrop_color", tk("surface")),
        ("sidebar_bg_color", hx("sidebar")),
        ("sidebar_fg_color", tk("text")),
        ("sidebar_backdrop_color", hx("sidebar")),
        ("secondary_sidebar_bg_color", hx("sidebar")),
        ("card_bg_color", tk("card")),
        ("card_fg_color", tk("text")),
        ("dialog_bg_color", tk("card")),
        ("dialog_fg_color", tk("text")),
        ("popover_bg_color", tk("card")),
        ("popover_fg_color", tk("text")),
        ("borders", tk("line")),
    ]
}

fn header(t: &ResolvedTheme) -> String {
    format!(
        "/* WolOS {} — generated by `wol theme`; edits are overwritten. */\n",
        t.id
    )
}

/// libadwaita 1.6+: CSS custom properties on :root (libadwaita's colours, its knobs, the WolOS
/// tokens as `--wol-*`), then the Kit scaffold (`scaffold::gtk4`).
fn gtk4_css(t: &ResolvedTheme) -> String {
    let mut s = header(t);
    s += ":root {\n";
    for (k, v) in adw_pairs(t) {
        s += &format!("  --{}: {v};\n", k.replace('_', "-"));
    }
    s += &super::scaffold::adw_knobs(t);
    s += &super::scaffold::gtk4_vars(t);
    s += "}\n";
    s += &super::scaffold::gtk4(t);
    s += &format!(
        "windowcontrols {{ border-spacing: {}px; }}\n",
        LIGHT_PITCH - LIGHT_SIZE
    );
    s += &window_controls(t, "windowcontrols > button");
    s
}

/// WolOS window controls (`button.light` in kit.css): a quiet 12 px circle (the text colour at
/// half way over the surface), 22 px centre to centre (measured on kit goldens). The glyph
/// (close, minimise, zoom: the symbolic icons) shows while the pointer is over the group; only the
/// control under the pointer takes the accent, a shade deeper while pressed. They sit on the right,
/// close at the edge ([`BUTTON_LAYOUT`] puts every button in `windowcontrols.end`, inset by
/// `adwaita.css`), like the compositor's strip (`realtime::chrome::STRIP_ALIGN`).
fn window_controls(t: &ResolvedTheme, button: &str) -> String {
    let (idle, hover, pressed) = controls_colors(t);
    let text = token(t, "text");
    let on_accent = token(t, "accentText");
    // The header bar stretches buttons vertically, so the circle is drawn on the (centred) image.
    format!(
        "{button} {{ min-width: {LIGHT_SIZE}px; min-height: {LIGHT_SIZE}px; padding: 0; margin: 0; border: none; background: none; box-shadow: none; }}\n\
         {button} image {{ min-width: {LIGHT_SIZE}px; min-height: {LIGHT_SIZE}px; padding: 0; margin: 0; border-radius: 999px; -gtk-icon-size: 8px; color: transparent; background: {idle}; box-shadow: none; }}\n\
         windowcontrols:hover > button image {{ color: {text}; }}\n\
         {button}:hover image {{ background: {hover}; color: {on_accent}; }}\n\
         {button}:active image {{ background: {pressed}; color: {on_accent}; }}\n"
    )
}

/// (idle circle, hovered, pressed) of the window controls, from the theme's tokens.
pub(crate) fn controls_colors(t: &ResolvedTheme) -> (String, String, String) {
    let (surface, text, accent) = (
        token_color(t, "surface"),
        token_color(t, "text"),
        token_color(t, "accent"),
    );
    (
        mix(surface, text, CONTROL_IDLE_MIX).to_hex(),
        accent.to_hex(),
        mix(accent, text, CONTROL_PRESSED_MIX).to_hex(),
    )
}

/// How far the idle circle sits from the surface toward the text colour, and the pressed accent
/// toward the text colour.
const CONTROL_IDLE_MIX: f64 = 0.5;
const CONTROL_PRESSED_MIX: f64 = 0.22;

/// adw-gtk3 6.5's own titlebutton sizing selectors (`min-width: 24px; margin: 0 4px`). Ours
/// repeat them verbatim: equal specificity, later in the cascade (after the @import), so ours win.
const ADW_TITLEBUTTON: [&str; 3] = [
    "button.titlebutton:not(.appmenu)",
    "headerbar.default-decoration button.titlebutton",
    ".default-decoration.titlebar:not(headerbar) button.titlebutton",
];

/// Selector list for every adw-gtk3 titlebutton selector, each followed by `suffix`.
fn titlebutton(suffix: &str) -> String {
    ADW_TITLEBUTTON
        .map(|sel| format!("{sel}{suffix}"))
        .join(", ")
}

/// GTK 3 window controls: the circle is the 12 px button (GTK 3 centres titlebuttons). Chromium's
/// GTK mode renders only the button background under GTK 3, so the glyph is a hover background:
/// that way GTK 3 apps and Helium (default browser) show the same one. Chromium styles each button
/// on its own, so unlike GTK 4 only the hovered control shows its glyph (on the accent). The real
/// icon stays hidden. GTK 3 and Chromium place buttons at header padding + margins, 6 px apart;
/// close is the last of [`BUTTON_LAYOUT`], so its extra right margin is the right-edge inset.
fn gtk3_window_controls(t: &ResolvedTheme) -> String {
    let side = (LIGHT_PITCH - LIGHT_SIZE - GTK3_BUTTON_SPACING) / 2;
    let first = LIGHT_FIRST_CENTRE - LIGHT_SIZE / 2 - GTK3_HEADER_PADDING;
    let (idle, hover, pressed) = controls_colors(t);
    let on_accent = token(t, "accentText");
    let mut s = format!(
        "{} {{ min-width: {LIGHT_SIZE}px; min-height: {LIGHT_SIZE}px; margin: 0 {side}px; padding: 0; border: none; border-radius: 999px; background-image: none; background-color: {idle}; box-shadow: none; }}\n\
         {} {{ margin-right: {first}px; }}\n\
         {} {{ margin: -2px; padding: 0; color: transparent; background: none; box-shadow: none; -gtk-icon-transform: scale(0.6); }}\n\
         {} {{ background-color: {hover}; color: {on_accent}; }}\n\
         {} {{ background-color: {pressed}; color: {on_accent}; }}\n",
        titlebutton(""),
        titlebutton(".close"),
        titlebutton(" image"),
        titlebutton(":hover"),
        titlebutton(":active"),
    );
    for (class, glyph) in [
        ("close", "window-close-symbolic"),
        ("minimize", "window-minimize-symbolic"),
        ("maximize", "window-maximize-symbolic"),
    ] {
        s += &format!(
            "{} {{ background-image: -gtk-icontheme(\"{glyph}\"); background-repeat: no-repeat; background-position: center; background-size: 10px 10px; }}\n",
            titlebutton(&format!(".{class}:hover")),
        );
    }
    s
}

/// adw-gtk3 reads the same names plus the GTK3 legacy ones.
fn gtk3_css(t: &ResolvedTheme) -> String {
    let p = Pal::new(t);
    let tk = |k| token(t, k).to_string();
    let mut s = header(t) + &defines(&adw_pairs(t));
    s += "window, .background { font-family: \"Inter\", sans-serif; }\n";
    s += &defines(&[
        ("theme_selected_bg_color", tk("accent")),
        ("theme_selected_fg_color", tk("accentText")),
        ("theme_bg_color", tk("surface")),
        ("theme_fg_color", tk("text")),
        ("theme_base_color", tk("card")),
        ("theme_text_color", tk("text")),
        ("insensitive_fg_color", p.c(t, "dim").to_hex()),
        ("borders", p.c(t, "line").to_hex()),
    ]);
    s += &super::scaffold::gtk3_defines(t);
    s += &super::scaffold::gtk3(t);
    s += &gtk3_window_controls(t);
    s += &super::gtk_atlas::css(t);
    s
}

fn settings_ini(t: &ResolvedTheme, gtk3: bool) -> String {
    let dark = t.variant == Variant::Dark;
    let mut s = String::from("[Settings]\n");
    if gtk3 {
        s += &format!("gtk-theme-name={}\n", gtk3_theme_name(t));
    }
    s += &format!("gtk-icon-theme-name={ICON_THEME}\n");
    s += &format!(
        "gtk-application-prefer-dark-theme={}\ngtk-font-name={FONT}\ngtk-decoration-layout={BUTTON_LAYOUT}\n",
        dark as u8
    );
    s
}

fn gtk3_theme(dark: bool) -> &'static str {
    if dark { "adw-gtk3-dark" } else { "adw-gtk3" }
}

fn qt6ct_conf(dir: &Path) -> String {
    format!(
        "[Appearance]\ncolor_scheme_path={}/qt6ct/colors/WolOS.conf\ncustom_palette=true\nstyle=kvantum\nstandard_dialogs=xdgdesktopportal\n\n[Fonts]\nfixed=\"monospace,11,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\"\ngeneral=\"Inter,11,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\"\n",
        dir.display()
    )
}

/// qt6ct palette: 21 roles (QPalette::ColorRole order) per group, `#aarrggbb`.
fn qt_colors(t: &ResolvedTheme) -> String {
    let p = Pal::new(t);
    let c = |k| p.c(t, k);
    let (bg, text, card, accent) = (c("surface"), c("text"), c("card"), c("accent"));
    let black = Color::rgb(0, 0, 0);
    let button = c("control");
    let group = |text: Color, dim: Color| {
        let roles = [
            text,                                           // WindowText
            button,                                         // Button
            mix(button, Color::WHITE, 0.4),                 // Light
            mix(button, Color::WHITE, 0.2),                 // Midlight
            mix(button, black, 0.4),                        // Dark
            mix(button, black, 0.2),                        // Mid
            text,                                           // Text
            Color::WHITE,                                   // BrightText
            text,                                           // ButtonText
            card,                                           // Base
            bg,                                             // Window
            mix(bg, black, if p.dark { 0.7 } else { 0.5 }), // Shadow
            accent,                                         // Highlight
            c("accentText"),                                // HighlightedText
            accent,                                         // Link
            mix(accent, text, 0.3),                         // LinkVisited
            mix(card, bg, 0.5),                             // AlternateBase
            black,                                          // NoRole
            card,                                           // ToolTipBase
            text,                                           // ToolTipText
            dim,                                            // PlaceholderText
        ];
        roles
            .iter()
            .map(|c| format!("#ff{}", &c.to_hex()[1..]))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let dim = c("dim");
    format!(
        "[ColorScheme]\nactive_colors={}\ndisabled_colors={}\ninactive_colors={}\n",
        group(text, dim),
        group(dim, mix(dim, bg, 0.4)),
        group(text, dim),
    )
}

/// foot: the theme's terminal palette (`terminal::palette`); translucent background. foot 1.28
/// reads its config only at start and switches between `[colors-dark]` and `[colors-light]` at
/// runtime only on SIGUSR1 / SIGUSR2 ([`terminal_argv`]), so both sections hold their own
/// appearance: the dark and light variants of this theme (same accent, [`counterpart`]), and
/// `initial-color-theme` names the one in use. A theme without a counterpart (a pack, a custom
/// theme) is in both sections.
fn foot_ini(t: &ResolvedTheme) -> String {
    let other = counterpart(t);
    let other = other.as_ref().unwrap_or(t);
    let (dark, light) = if t.variant == Variant::Dark {
        (t, other)
    } else {
        (other, t)
    };
    format!(
        concat!(
            "# WolOS {} — generated by `wol theme`; edits are overwritten.\n",
            "[main]\ntitle={}\nfont={}\npad=12x12\ninitial-color-theme={}\n",
            "# The shell chosen in Settings > General (wol_core::terminal_shell).\nshell={}\n\n",
            "[colors-dark]\n{}\n[colors-light]\n{}"
        ),
        t.id,
        FOOT_TITLE,
        MONO_FONT,
        foot_theme(t.variant),
        crate::terminal_shell::LAUNCHER,
        foot_colors(dark),
        foot_colors(light),
    )
}

/// foot's name for an appearance (`initial-color-theme`).
fn foot_theme(v: Variant) -> &'static str {
    if v == Variant::Dark { "dark" } else { "light" }
}

/// One foot colour section's body.
fn foot_colors(t: &ResolvedTheme) -> String {
    let p = super::terminal::palette(t);
    let h = super::terminal::bare;
    let mut colors = format!(
        "alpha={FOOT_ALPHA}\nbackground={}\nforeground={}\nselection-background={}\nselection-foreground={}\n",
        h(p.background),
        h(p.foreground),
        h(p.selection_bg),
        h(p.selection_fg),
    );
    for (i, col) in p.regular().iter().enumerate() {
        colors += &format!("regular{i}={}\n", h(*col));
    }
    for (i, col) in p.bright().iter().enumerate() {
        colors += &format!("bright{i}={}\n", h(*col));
    }
    colors
}

/// The other appearance of a WolOS theme with the same accent (`wol-dark-blue` for
/// `wol-light-blue`), from the built-ins; None for a theme that has no such pair.
pub fn counterpart(t: &ResolvedTheme) -> Option<ResolvedTheme> {
    let (Some(v), accent) = super::accent::split(Some(&t.id)) else {
        return None;
    };
    let other = if v == Variant::Dark {
        Variant::Light
    } else {
        Variant::Dark
    };
    let id = super::accent::theme_id(Some(other), accent)?;
    super::accent::variants()
        .into_iter()
        .fold(super::ThemeCatalog::builtin(), super::ThemeCatalog::with)
        .resolve(&id)
        .ok()
}

/// Running foot terminals (and footclients, which pass it to their server) of account `uid`
/// switch to the section of `t`'s appearance: SIGUSR1 `[colors-dark]`, SIGUSR2 `[colors-light]`
/// (foot(1), SIGNALS). Only for a foot.ini WolOS generated ([`foot_is_ours`]): a person's own
/// may have no section for the other appearance.
pub fn terminal_argv(t: &ResolvedTheme, uid: u32) -> Vec<String> {
    let sig = if t.variant == Variant::Dark {
        "USR1"
    } else {
        "USR2"
    };
    [
        "pkill",
        "--signal",
        sig,
        "--uid",
        &uid.to_string(),
        "--exact",
        "foot|footclient",
    ]
    .map(String::from)
    .to_vec()
}

/// `$XDG_CONFIG_HOME/foot/foot.ini` is WolOS's link into the toolkit dir `dir`.
pub fn foot_is_ours(dir: &Path, config_home: &Path) -> bool {
    points_into(&config_home.join("foot/foot.ini"), dir)
}

/// (schema, key, value) for `gsettings set`; reaches running GTK apps through the settings portal.
pub fn gsettings(t: &ResolvedTheme) -> Vec<(&'static str, &'static str, String)> {
    let dark = t.variant == Variant::Dark;
    const IF: &str = "org.gnome.desktop.interface";
    let mut v = vec![
        (
            IF,
            "color-scheme",
            if dark { "prefer-dark" } else { "default" }.to_string(),
        ),
        (IF, "accent-color", adw_accent(t).to_string()),
        (IF, "gtk-theme", gtk3_theme_name(t)),
        (IF, "font-name", FONT.to_string()),
        (
            "org.gnome.desktop.wm.preferences",
            "button-layout",
            BUTTON_LAYOUT.to_string(),
        ),
    ];
    v.push((IF, "icon-theme", ICON_THEME.to_string()));
    v.push((
        "org.gnome.TextEditor",
        "style-scheme",
        super::sourceview::scheme_id(t.variant).to_string(),
    ));
    v
}

/// `gsettings set` argv per key.
pub fn gsettings_argv(t: &ResolvedTheme) -> Vec<Vec<String>> {
    gsettings(t)
        .into_iter()
        .map(|(schema, key, value)| {
            ["gsettings", "set", schema, key, &value]
                .map(String::from)
                .to_vec()
        })
        .collect()
}

/// The compositor's colours, live (`hypr::lua_argv`: the title strip and shadows; the same calls
/// as the `hypr/wolos/theme.lua` that `install` writes for the next start or reload).
pub fn strip_argv(t: &ResolvedTheme) -> Vec<Vec<String>> {
    super::hypr::lua_argv(t)
}

/// Session/shell entry: resolve `id`, install the user toolkit files, push gsettings (blocking;
/// call off the UI thread). gsettings failures are ignored (not every host has it).
pub fn apply(id: &str, config_home: &Path) -> Result<(), String> {
    let t = write_files(id, config_home)?;
    push(&t, config_home);
    Ok(())
}

/// The file half of [`apply`]: resolve `id` and write the toolkit files and links. Quick text
/// writes, so a caller that must tell the person about a failure (a full disk: QA-LIVE-19) runs
/// it before it says "changed", and [`push`] afterwards.
pub fn write_files(id: &str, config_home: &Path) -> Result<ResolvedTheme, String> {
    let data = data_home(|k| std::env::var(k).ok());
    let t = super::ThemeCatalog::installed(config_home, data.as_deref())
        .resolve(id)
        .map_err(|e| format!("{e:?}"))?;
    install(&t, &user_dir(config_home), config_home).map_err(|e| e.to_string())?;
    if let Some(data) = data {
        link_data(&t, &user_dir(config_home), &data).map_err(|e| e.to_string())?;
    }
    Ok(t)
}

/// The slow half of [`apply`]: gsettings, the strip and foot (each failure ignored).
pub fn push(t: &ResolvedTheme, config_home: &Path) {
    let foot = foot_is_ours(&user_dir(config_home), config_home)
        .then(|| terminal_argv(t, shared::sys::euid()));
    for a in gsettings_argv(t)
        .into_iter()
        .chain(strip_argv(t))
        .chain(foot)
    {
        let _ = std::process::Command::new(&a[0])
            .args(&a[1..])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// `$XDG_DATA_HOME`, else `$HOME/.local/share`; None without either.
pub fn data_home(var: impl Fn(&str) -> Option<String>) -> Option<std::path::PathBuf> {
    let set = |k| var(k).filter(|v| !v.is_empty());
    set("XDG_DATA_HOME")
        .map(Into::into)
        .or_else(|| set("HOME").map(|h| Path::new(&h).join(".local/share")))
}

/// Links each of `DATA_LINKS` and the GTK 3 theme from `data_home` to the toolkit dir (after
/// `install`); earlier `WolOS-*` names pointing at the same theme are removed.
pub fn link_data(t: &ResolvedTheme, dir: &Path, data_home: &Path) -> std::io::Result<()> {
    link_all(dir, data_home, &DATA_LINKS)?;
    let themes = data_home.join("themes");
    std::fs::create_dir_all(&themes)?;
    let target = dir.join(GTK3_THEME_DIR);
    let name = gtk3_theme_name(t);
    for e in std::fs::read_dir(&themes)?.flatten() {
        let n = e.file_name();
        let ours = std::fs::read_link(e.path()).is_ok_and(|l| l == target);
        if ours && n.to_string_lossy().starts_with("WolOS-") && *n != *name {
            // Another install at the same moment may have removed it first (audit C6-3).
            match std::fs::remove_file(e.path()) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                r => r?,
            }
        }
    }
    let link = themes.join(&name);
    if link.symlink_metadata().is_err() {
        match std::os::unix::fs::symlink(&target, &link) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            r => r?,
        }
    }
    Ok(())
}

/// Writes every consumer (`generate::consumers`) into `dir` (atomic per file), links each of
/// `LINKS` and the compositor theme file from `config_home` unless the user already has a real file
/// or a link of their own there, makes sure the user hyprland.lua loads `hypr/wolos/*.lua`, and renders the user's
/// templates (`templates`; a broken template is skipped, see `templates::install`).
pub fn install(t: &ResolvedTheme, dir: &Path, config_home: &Path) -> std::io::Result<()> {
    for f in super::generate::consumers(t, dir) {
        let p = dir.join(f.rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        shared::atomic::atomic_write(&p, super::kept_mode(&p), f.text.as_bytes())?;
    }
    link_all(dir, config_home, &LINKS)?;
    link_all(dir, config_home, &[super::hypr::HYPR_THEME_FILE])?;
    hypr_source(config_home)?;
    let _ = super::templates::install(t, config_home, dir);
    Ok(())
}

/// The user hyprland.lua loads the WolOS files (`prefs::ensure_require`), so the theme file is read
/// after the system default on the next start or reload. Under the journal lock and atomic, like
/// every writer of that file (`hyprprefs::edit_user_lua`, which also keeps a linked file a link).
fn hypr_source(config_home: &Path) -> std::io::Result<()> {
    crate::hyprprefs::edit_user_lua(config_home, crate::prefs::ensure_require).map(|_| ())
}

/// True when the link at `link` points into the toolkit dir `dir`, so WolOS made it and may
/// replace it. A relative target is read from the link's own directory.
fn points_into(link: &Path, dir: &Path) -> bool {
    made_by_wolos(link, dir, None)
}

/// [`points_into`], or (with `rel`) the link `wol-session` makes before the first theme: the
/// shipped default at the same relative path under [`SYSTEM_DIR`]. WolOS made that one too, so
/// the theme replaces it; kept, it pinned every toolkit (and foot) to the light default whatever
/// the theme said (MANUAL 0.12.0).
fn made_by_wolos(link: &Path, dir: &Path, rel: Option<&str>) -> bool {
    let Ok(t) = std::fs::read_link(link) else {
        return false;
    };
    if rel.is_some_and(|r| t == Path::new(SYSTEM_DIR).join(r)) {
        return true;
    }
    let t = match link.parent() {
        Some(p) if t.is_relative() => p.join(t),
        _ => t,
    };
    t.starts_with(dir)
}

/// Links each of `rels` from `home` to `dir`. A real file and a link that points anywhere else
/// (the person's own dotfiles) are kept; only a missing name, a link into `dir` or the shipped
/// default `wol-session` linked ([`made_by_wolos`]) is (re)made.
///
/// Safe against another install doing the same at once (audit C6-3): a link that already points
/// at its target is left alone, a stale link into `dir` is replaced in one step (a new link at a
/// temporary name renamed over it, never remove-then-create), and a missing name that someone
/// else links first (`EEXIST`) is kept, as if their install had run just after ours.
fn link_all(dir: &Path, home: &Path, rels: &[&str]) -> std::io::Result<()> {
    for rel in rels {
        let (link, want) = (home.join(rel), dir.join(rel));
        match link.symlink_metadata() {
            Ok(m) if !m.is_symlink() => continue,
            Ok(_) if !made_by_wolos(&link, dir, Some(rel)) => continue,
            Ok(_) if std::fs::read_link(&link).is_ok_and(|t| t == want) => continue,
            Ok(_) => {
                replace_link(&want, &link)?;
                continue;
            }
            Err(_) => {}
        }
        if let Some(d) = link.parent() {
            std::fs::create_dir_all(d)?;
        }
        match std::os::unix::fs::symlink(&want, &link) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            r => r?,
        }
    }
    Ok(())
}

/// Points the existing link `link` at `target` atomically: a new link at a private temporary
/// name ([`shared::atomic::temp_name`]) is renamed over it; the temporary is removed on error.
fn replace_link(target: &Path, link: &Path) -> std::io::Result<()> {
    let tmp = shared::atomic::temp_name(link)?;
    std::os::unix::fs::symlink(target, &tmp)?;
    std::fs::rename(&tmp, link).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeCatalog;
    use std::path::Path;

    fn theme(id: &str) -> ResolvedTheme {
        ThemeCatalog::builtin().resolve(id).unwrap()
    }

    fn file<'a>(files: &'a [File], rel: &str) -> &'a str {
        &files
            .iter()
            .find(|f| f.rel == rel)
            .unwrap_or_else(|| panic!("{rel}"))
            .text
    }

    #[test]
    fn window_controls_are_wolos_own() {
        let t = theme("wol-light");
        let f = render(&t, Path::new("/x"));
        let (idle, hover, pressed) = controls_colors(&t);
        assert_eq!(hover, t.tokens["accent"].to_ascii_lowercase());
        let gtk4 = file(&f, "gtk-4.0/gtk.css");
        assert!(gtk4.contains(&format!(
            "windowcontrols > button image {{ min-width: 12px; min-height: 12px; padding: 0; margin: 0; border-radius: 999px; -gtk-icon-size: 8px; color: transparent; background: {idle}; box-shadow: none; }}"
        )));
        assert!(
            gtk4.contains(&format!(
                "windowcontrols:hover > button image {{ color: {}; }}",
                t.tokens["text"]
            )),
            "hovering the group shows the glyphs on all three"
        );
        assert!(gtk4.contains(&format!(
            "windowcontrols > button:hover image {{ background: {hover}; color: {}; }}",
            t.tokens["accentText"]
        )));
        assert!(gtk4.contains(&format!(
            "windowcontrols > button:active image {{ background: {pressed};"
        )));
        assert!(
            gtk4.contains("color: transparent;"),
            "glyph hidden until hover"
        );
        // Not the traffic lights: one quiet colour for all three, never the status colours.
        for class in ["close", "minimize", "maximize"] {
            assert!(
                !gtk4.contains(&format!("windowcontrols > button.{class} image")),
                "{class}: no colour of its own"
            );
        }
        // GTK 3: the circle is the button itself. Chromium's GTK mode (Helium) renders only the
        // titlebutton background under GTK 3, never the image's (ui/gtk/nav_button_provider_gtk.cc).
        let gtk3 = file(&f, GTK3_THEME_CSS);
        assert!(gtk3.contains(&format!("background-color: {idle}; box-shadow: none; }}")));
        assert!(gtk3.contains(&format!(
            "{} {{ background-color: {hover}; color: {}; }}",
            titlebutton(":hover"),
            t.tokens["accentText"]
        )));
        assert!(gtk3.contains(&format!(
            "{} {{ background-color: {pressed}; color: {}; }}",
            titlebutton(":active"),
            t.tokens["accentText"]
        )));
        for tok in ["danger", "warning", "success"] {
            assert!(
                !gtk3.contains(&format!("background-color: {}; }}", t.tokens[tok])),
                "{tok}: the status colours are not window buttons"
            );
        }
        assert!(gtk3.contains(".titlebar:not(headerbar) button.titlebutton { min-width: 12px; min-height: 12px; margin: 0 2px; padding: 0; border: none; border-radius: 999px;"));
        assert!(
            gtk3.contains(".titlebar:not(headerbar) button.titlebutton image { margin: -2px;"),
            "16 px icon in a 12 px circle"
        );
        assert!(
            !gtk3.contains("titlebutton.close image"),
            "no circle on the image"
        );
        // Chromium paints the background only, so that is where the hover glyph has to be.
        for (class, glyph) in [
            ("close", "window-close-symbolic"),
            ("minimize", "window-minimize-symbolic"),
            ("maximize", "window-maximize-symbolic"),
        ] {
            assert!(
                gtk3.contains(&format!(
                    "{} {{ background-image: -gtk-icontheme(\"{glyph}\");",
                    titlebutton(&format!(".{class}:hover"))
                )),
                "{class} glyph on hover"
            );
        }
        assert!(!gtk3.contains(":hover image"), "one glyph, not two");
        assert!(file(&f, "gtk-4.0/gtk.css").contains("windowcontrols { border-spacing: 10px; }"));
        // Header bar (bleeds 1 px past the window) pads 17, controls inset 1: close covers the
        // window's x 17..28 counted from the right edge, y 20..31 (the compositor row, mirrored).
        let g4 = file(&f, "gtk-4.0/gtk.css");
        assert!(g4.contains("headerbar > windowhandle > box { padding: 10px 17px; }"));
        assert!(g4.contains("windowcontrols.end { margin-left: 8px; margin-right: 1px; }"));
    }

    /// GTK 3 rejects GTK 4-only CSS (Firefox logged `'-gtk-icon-size' is not a valid property name`).
    const GTK4_ONLY: [&str; 8] = [
        "-gtk-icon-size",
        "-gtk-icon-filter",
        "-gtk-icon-weight",
        ":root",
        "var(",
        "color-mix(",
        "border-spacing",
        " transform:",
    ];

    #[test]
    fn gtk3_css_has_no_gtk4_only_properties() {
        for t in ThemeCatalog::builtin().list() {
            let f = render(&theme(&t.id), Path::new("/x"));
            for rel in ["gtk-3.0/gtk.css", GTK3_THEME_CSS] {
                let css = file(&f, rel);
                for bad in GTK4_ONLY {
                    assert!(!css.contains(bad), "{}: {rel} uses {bad}", t.id);
                }
                assert!(
                    !css.lines().any(|l| l.trim_start().starts_with("--")),
                    "{}: custom properties are GTK 4.16+",
                    t.id
                );
            }
        }
    }

    /// Same window-button geometry in every toolkit as Kit AppWindow / hyprbars (kit goldens,
    /// pixel edges): 12 px circles, centres 23/45/67 px in from the window's right edge (close,
    /// zoom, minimise).
    #[test]
    fn window_buttons_match_the_kit_geometry() {
        assert_eq!((LIGHT_SIZE, LIGHT_PITCH, LIGHT_FIRST_CENTRE), (12, 22, 23));
        let f = render(&theme("wol-light"), Path::new("/x"));
        // GTK 3 and Chromium's GTK frame (Helium): header padding + margins + 6 px button spacing.
        // adw-gtk3 sizes titlebuttons with these exact selectors (gtk-3.0/gtk.css, 6.5); a weaker
        // selector loses the cascade and the buttons render 24 px, 34 px apart (measured).
        let g3 = file(&f, GTK3_THEME_CSS);
        let (side, first) = (2, 11);
        for adw in [
            "button.titlebutton:not(.appmenu)",
            "headerbar.default-decoration button.titlebutton",
            ".default-decoration.titlebar:not(headerbar) button.titlebutton",
        ] {
            let rule = g3
                .lines()
                .find(|l| l.split(" {").next().unwrap().split(", ").any(|s| s == adw))
                .unwrap_or_else(|| panic!("{adw} not overridden"));
            assert!(
                rule.contains(&format!(
                    "min-width: {LIGHT_SIZE}px; min-height: {LIGHT_SIZE}px; margin: 0 {side}px;"
                )),
                "{rule}"
            );
            assert!(g3.contains(&format!("{adw}.close")), "{adw}.close");
        }
        assert!(g3.contains(&format!(
            "{} {{ margin-right: {first}px; }}",
            titlebutton(".close")
        )));
        assert!(
            g3.find("@import") < g3.find("titlebutton"),
            "ours after adw-gtk3"
        );
        assert_eq!(LIGHT_SIZE + 2 * side + GTK3_BUTTON_SPACING, LIGHT_PITCH);
        assert_eq!(
            GTK3_HEADER_PADDING + first + LIGHT_SIZE / 2,
            LIGHT_FIRST_CENTRE
        );
        assert!(!g3.contains("margin: 0 5px"), "the old 28 px pitch");
        // GTK 4 windowcontrols: border-spacing between 12 px images; the end box (the layout puts
        // every button there) holds close at the right: the header box's padding, less the 1 px
        // libadwaita lays the bar past the window edge, plus the end inset.
        let g4 = file(&f, "gtk-4.0/gtk.css");
        assert!(g4.contains("windowcontrols { border-spacing: 10px; }"));
        assert_eq!(LIGHT_SIZE + 10, LIGHT_PITCH);
        // The first number after `key: ` in the rule starting with `rule` (px), or its `nth` one.
        let px = |rule: &str, key: &str, nth: usize| -> i32 {
            let body = g4
                .lines()
                .find(|l| l.starts_with(rule))
                .unwrap_or_else(|| panic!("{rule} missing"));
            let v = body.split(&format!("{key}: ")).nth(1).unwrap();
            let v = v.split(';').next().unwrap().split(' ').nth(nth).unwrap();
            v.trim_end_matches("px").parse().unwrap()
        };
        // `padding: <vertical>px <horizontal>px`.
        let pad = px("headerbar > windowhandle > box {", "padding", 1);
        let end = px("windowcontrols.end {", "margin-right", 0);
        assert_eq!(
            pad - crate::theme::scaffold::ADW_BAR_BLEED + end + LIGHT_SIZE / 2,
            LIGHT_FIRST_CENTRE,
            "GTK 4 close centre from the right edge"
        );
        assert!(
            BUTTON_LAYOUT.starts_with(':') && BUTTON_LAYOUT.ends_with(",close"),
            "every button in the end box, close last"
        );
        // Firefox: container inset + 5 px button padding + radius.
        let ff = crate::theme::firefox::chrome_css(&theme("wol-light"));
        assert!(ff.contains("margin-inline-end: 12px !important;"));
        assert!(ff.contains(".titlebar-button { padding-inline: 5px !important; }"));
        assert_eq!(12 + 5 + LIGHT_SIZE / 2, LIGHT_FIRST_CENTRE);
        assert_eq!(5 + LIGHT_SIZE + 5, LIGHT_PITCH);
    }

    #[test]
    fn every_toolkit_gets_the_tokens() {
        let dir = Path::new("/x/toolkits");
        for (id, dark) in [("wol-light", false), ("wol-dark", true)] {
            let t = theme(id);
            let accent = &t.tokens["accent"];
            let surface = &t.tokens["surface"];
            let f = render(&t, dir);
            let gtk4 = file(&f, "gtk-4.0/gtk.css");
            for want in [
                format!("--accent-bg-color: {accent};"),
                format!(
                    "--accent-color: {};",
                    readable(token_color(&t, "accent"), token_color(&t, "card"), 4.5).to_hex()
                ),
                format!("--window-bg-color: {surface};"),
            ] {
                assert!(gtk4.contains(&want), "{id} gtk4 {want}");
            }
            let gtk3 = file(&f, GTK3_THEME_CSS);
            assert!(gtk3.contains(&format!("@define-color theme_selected_bg_color {accent};")));
            assert!(gtk3.contains(&format!("@define-color accent_bg_color {accent};")));
            let ini3 = file(&f, "gtk-3.0/settings.ini");
            assert!(
                ini3.contains(&format!("gtk-theme-name=WolOS-{id}-gtk3\n")),
                "{id}"
            );
            for ini in [ini3, file(&f, "gtk-4.0/settings.ini")] {
                assert!(ini.contains("gtk-decoration-layout=:minimize,maximize,close\n"));
                assert!(ini.contains("gtk-font-name=Inter 10"));
                assert!(ini.contains("gtk-icon-theme-name=WolOS\n"));
                assert!(ini.contains(&format!("gtk-application-prefer-dark-theme={}", dark as u8)));
            }
            let qt = file(&f, "qt6ct/colors/WolOS.conf");
            let roles: Vec<&str> = qt
                .lines()
                .find(|l| l.starts_with("active_colors="))
                .unwrap()
                .trim_start_matches("active_colors=")
                .split(", ")
                .collect();
            assert_eq!(roles.len(), 21, "{id}");
            assert_eq!(roles[12], format!("#ff{}", &accent[1..]), "Highlight");
            assert_eq!(roles[10], format!("#ff{}", &surface[1..]), "Window");
            assert!(qt.contains("inactive_colors=") && qt.contains("disabled_colors="));
            let qtc = file(&f, "qt6ct/qt6ct.conf");
            assert!(qtc.contains("style=kvantum") && qtc.contains("custom_palette=true"));
            assert!(qtc.contains("color_scheme_path=/x/toolkits/qt6ct/colors/WolOS.conf"));
            let foot = file(&f, "foot/foot.ini");
            assert!(foot.contains(&format!("background={}", &surface[1..])));
            assert!(foot.contains("alpha=0.9"));
            assert_eq!(foot.matches("regular").count(), 16);
            assert_eq!(foot.matches("bright").count(), 16);
            // foot 1.28 rejects `[colors]` (renamed `[colors-dark]`); "monospace" matched Font Awesome.
            assert!(foot.contains("\n[colors-dark]\n") && !foot.contains("[colors]"));
            // The section of this theme's appearance holds its colours, and foot starts in it.
            let (dark_s, light_s) = foot.split_once("\n[colors-light]\n").unwrap();
            let dark_s = dark_s.split_once("\n[colors-dark]\n").unwrap().1;
            let mine = if dark { dark_s } else { light_s };
            assert!(
                mine.contains(&format!("background={}\n", &surface[1..])),
                "{foot}"
            );
            let start = format!(
                "initial-color-theme={}\n",
                if dark { "dark" } else { "light" }
            );
            assert!(foot.contains(&start), "{foot}");
            assert!(foot.contains("font=JetBrains Mono:size=11\n"));
            assert!(
                foot.contains("\n[main]\ntitle=Terminal\n"),
                "{id}: window title"
            );
        }
    }

    /// The foot.ini the image ships (what a fresh account links before its first `wol theme set`)
    /// is the default theme's render, so the window is titled "Terminal" like the menu bar and not
    /// foot's own "foot" (QA-LIVE-12). Edit `foot_ini`, then copy its output there.
    #[test]
    fn shipped_foot_ini_is_the_default_render() {
        let shipped = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../desktop/overlay/usr/share/wolos/foot/foot.ini");
        let f = render(&theme("wol-light"), Path::new("/x/toolkits"));
        let want = file(&f, "foot/foot.ini");
        let have = std::fs::read_to_string(&shipped).unwrap();
        assert_eq!(have, want, "{}", shipped.display());
        assert!(have.contains("\n[main]\ntitle=Terminal\n"));
    }

    /// Contract with the system zshrc (`/usr/share/wolos/zsh/zshrc`): it sources
    /// `toolkits/zsh/colors.zsh` and reads `WOLOS_PROMPT_*` as `#rrggbb`.
    #[test]
    fn zsh_prompt_colours() {
        assert_eq!(SHELL_COLORS, "zsh/colors.zsh");
        for s in ThemeCatalog::builtin().list() {
            let t = theme(&s.id);
            let files = render(&t, Path::new("/x"));
            let z = file(&files, SHELL_COLORS);
            let bg = token_color(&t, "surface");
            for (var, min) in [
                ("WOLOS_PROMPT_ACCENT", 4.5),
                ("WOLOS_PROMPT_MUTED", 4.5),
                ("WOLOS_PROMPT_OK", 4.5),
                ("WOLOS_PROMPT_ERROR", 4.5),
                ("WOLOS_PROMPT_SUGGEST", 3.0),
            ] {
                let v = z
                    .lines()
                    .find_map(|l| l.strip_prefix(&format!("{var}='")))
                    .and_then(|v| Color::parse(v.trim_end_matches('\'')))
                    .unwrap_or_else(|| panic!("{}: {var}", s.id));
                assert!(Color::contrast(v, bg) >= min, "{}: {var}", s.id);
            }
            assert!(z.contains(&format!("WOLOS_THEME={}\n", s.id)));
        }
    }

    #[test]
    fn data_home_and_links() {
        let v = |pairs: &'static [(&str, &str)]| {
            move |k: &str| pairs.iter().find(|p| p.0 == k).map(|p| p.1.to_string())
        };
        assert_eq!(
            data_home(v(&[("XDG_DATA_HOME", "/d"), ("HOME", "/h")])),
            Some("/d".into())
        );
        assert_eq!(
            data_home(v(&[("XDG_DATA_HOME", ""), ("HOME", "/h")])),
            Some("/h/.local/share".into())
        );
        assert_eq!(data_home(v(&[])), None);
        let root = std::env::temp_dir().join(format!("wol-data-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (tk, cfg, data) = (root.join("tk"), root.join("cfg"), root.join("data"));
        install(&theme("nord"), &tk, &cfg).unwrap();
        link_data(&theme("nord"), &tk, &data).unwrap();
        let x =
            std::fs::read_to_string(data.join("gtksourceview-5/styles/wolos-dark.xml")).unwrap();
        assert!(x.contains("<!-- WolOS nord"));
        assert!(gsettings(&theme("nord")).contains(&(
            "org.gnome.TextEditor",
            "style-scheme",
            "wolos-dark".to_string()
        )));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// libhandy apps (Image Viewer, Mail) strip a `-dark` suffix from gtk-theme-name and, in dark
    /// mode, need the theme's `gtk-dark.css`; either missing and they fall back to Adwaita.
    #[test]
    fn libhandy_apps_keep_the_theme() {
        for id in ["wol-dark", "wol-light", "nord"] {
            let t = theme(id);
            assert!(!gtk3_theme_name(&t).ends_with("-dark"), "{id}");
            let f = render(&t, Path::new("/x"));
            assert_eq!(
                file(&f, GTK3_THEME_DARK_CSS),
                file(&f, GTK3_THEME_CSS),
                "{id}"
            );
        }
    }

    /// GTK 3 reads the user gtk.css once; a theme is reloaded whenever its name changes. Colours
    /// live in a GTK 3 theme named after the WolOS theme, so every switch restyles running GTK 3
    /// apps (and Helium's GTK mode) live; the user gtk.css carries no colours to shadow it.
    #[test]
    fn gtk3_colours_are_a_theme_that_switches_live() {
        for (id, adw) in [("wol-light", "adw-gtk3"), ("nord", "adw-gtk3-dark")] {
            let t = theme(id);
            let f = render(&t, Path::new("/x"));
            let th = file(&f, GTK3_THEME_CSS);
            assert!(th.starts_with(&format!(
                "@import url(\"file:///usr/share/themes/{adw}/gtk-3.0/gtk.css\");\n"
            )));
            assert!(th.contains(&format!(
                "@define-color accent_bg_color {};",
                t.tokens["accent"]
            )));
            assert!(th.contains("button.titlebutton:hover { background-color:"));
            let user = file(&f, "gtk-3.0/gtk.css");
            assert!(
                !user.contains("@define-color") && !user.contains("titlebutton"),
                "{id}"
            );
            assert!(
                file(&f, "gtk-3.0/settings.ini")
                    .contains(&format!("gtk-theme-name=WolOS-{id}-gtk3\n"))
            );
            assert!(gsettings(&t).contains(&(
                "org.gnome.desktop.interface",
                "gtk-theme",
                format!("WolOS-{id}-gtk3")
            )));
        }
        let root = std::env::temp_dir().join(format!("wol-gtk3th-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (tk, cfg, data) = (root.join("tk"), root.join("cfg"), root.join("data"));
        for id in ["wol-light", "nord"] {
            install(&theme(id), &tk, &cfg).unwrap();
            link_data(&theme(id), &tk, &data).unwrap();
        }
        let css =
            std::fs::read_to_string(data.join("themes/WolOS-nord-gtk3/gtk-3.0/gtk.css")).unwrap();
        assert!(css.contains("WolOS nord"));
        assert!(
            !data.join("themes/WolOS-wol-light-gtk3").exists(),
            "previous name pruned"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The settings portal carries `accent-color` to libadwaita and to Chromium's colour mixer
    /// (Helium): send the preset nearest the theme's accent, not a fixed one.
    #[test]
    fn portal_accent_is_the_nearest_preset() {
        for (id, want) in [
            ("wol-light", "purple"),
            ("wol-light-blue", "teal"),
            ("wol-dark-green", "green"),
            ("wol-light-pink", "pink"),
            ("nord", "teal"),
            ("ristretto", "orange"),
            ("white", "slate"),
            ("vantablack", "slate"),
        ] {
            let mut cat = ThemeCatalog::builtin();
            for v in crate::theme::accent::variants() {
                cat = cat.with(v);
            }
            let t = cat.resolve(id).unwrap();
            let got = gsettings(&t)
                .into_iter()
                .find(|x| x.1 == "accent-color")
                .unwrap()
                .2;
            assert_eq!(got, want, "{id}");
        }
    }

    /// libadwaita draws text in `accent_color`, `destructive_color`, `success_color`,
    /// `warning_color`, `error_color` (fills use `*_bg_color`): every one reads >= 4.5:1 on cards.
    #[test]
    fn adwaita_text_colours_are_readable() {
        for s in ThemeCatalog::builtin().list() {
            let t = theme(&s.id);
            let card = token_color(&t, "card");
            for (k, v) in adw_pairs(&t) {
                let text_use = k.ends_with("_color")
                    && !k.contains("_bg_")
                    && !k.contains("_fg_")
                    && !k.starts_with("window")
                    && !k.starts_with("view")
                    && !k.starts_with("headerbar")
                    && !k.starts_with("sidebar")
                    && !k.starts_with("secondary")
                    && !k.starts_with("card")
                    && !k.starts_with("dialog")
                    && !k.starts_with("popover");
                if text_use {
                    let c = Color::parse(&v).unwrap();
                    assert!(Color::contrast(c, card) >= 4.5, "{}: {k} {v}", s.id);
                }
            }
            let fill = adw_pairs(&t)
                .into_iter()
                .find(|p| p.0 == "accent_bg_color")
                .unwrap()
                .1;
            assert_eq!(
                fill, t.tokens["accent"],
                "{}: fills keep the exact token",
                s.id
            );
        }
    }

    /// Client-side controls sit where the compositor draws its own: on the right, close rightmost,
    /// then zoom and minimise leftward (realtime `titlebar::Align::Right`).
    #[test]
    fn client_controls_follow_the_compositor_row() {
        assert_eq!(BUTTON_LAYOUT, ":minimize,maximize,close");
        let (start, end) = BUTTON_LAYOUT.split_once(':').unwrap();
        assert!(start.is_empty(), "nothing on the left");
        assert_eq!(end.rsplit(',').next(), Some("close"), "close rightmost");
        let hypr = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../desktop/overlay/usr/share/wolos/hypr/hyprland.lua"),
        )
        .unwrap();
        assert!(
            hypr.lines()
                .any(|l| l.trim().replace(' ', "") == "bar_buttons_alignment=\"right\","),
            "the compositor row is right-aligned too"
        );
    }

    #[test]
    fn gsettings_follow_the_variant() {
        let l = gsettings(&theme("wol-light"));
        let d = gsettings(&theme("wol-dark"));
        let get =
            |v: &[(&str, &str, String)], k: &str| v.iter().find(|x| x.1 == k).unwrap().2.clone();
        assert_eq!(get(&l, "color-scheme"), "default");
        assert_eq!(get(&d, "color-scheme"), "prefer-dark");
        assert_eq!(get(&d, "gtk-theme"), "WolOS-wol-dark-gtk3");
        assert_eq!(get(&l, "accent-color"), "purple");
        assert_eq!(get(&l, "button-layout"), ":minimize,maximize,close");
        assert_eq!(get(&l, "font-name"), "Inter 10");
        assert_eq!(
            get(&d, "icon-theme"),
            "WolOS",
            "the shipped theme, not the catalog's"
        );
    }

    /// C6-3: two installs at once leave every consumer file whole (one theme's text) and no
    /// temporary behind.
    #[test]
    fn parallel_installs_leave_whole_files_and_no_temporaries() {
        let root = std::env::temp_dir().join(format!("wol-apps-par-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (tk, cfg) = (root.join("toolkits"), root.join("config"));
        let hs: Vec<_> = ["wol-dark", "wol-light"]
            .into_iter()
            .map(|id| {
                let (tk, cfg) = (tk.clone(), cfg.clone());
                std::thread::spawn(move || {
                    for _ in 0..5 {
                        install(&theme(id), &tk, &cfg).unwrap();
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        let (dark, light) = (
            super::super::generate::consumers(&theme("wol-dark"), &tk),
            super::super::generate::consumers(&theme("wol-light"), &tk),
        );
        for (d, l) in dark.iter().zip(light.iter()) {
            let got = std::fs::read_to_string(tk.join(d.rel)).unwrap();
            assert!(got == d.text || got == l.text, "{} is torn", d.rel);
        }
        fn walk(p: &Path, bad: &mut Vec<String>) {
            for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.ends_with(".tmp") || n.ends_with(shared::atomic::TMP_SUFFIX) {
                    bad.push(n);
                }
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    walk(&e.path(), bad);
                }
            }
        }
        let mut bad = Vec::new();
        walk(&root, &mut bad);
        assert!(bad.is_empty(), "temporaries left: {bad:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Two installs that (re)make the same links at once both succeed: a link that already
    /// points at its target is left, a stale one is replaced by a rename, and a name another
    /// install linked a moment ago is not an error (the cause of the intermittent failure of
    /// `parallel_installs_leave_whole_files_and_no_temporaries`: remove ENOENT / symlink EEXIST).
    #[test]
    fn concurrent_link_all_never_fails_and_leaves_one_link() {
        let root = std::env::temp_dir().join(format!("wol-apps-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (tk, home) = (root.join("toolkits"), root.join("home"));
        std::fs::create_dir_all(&tk).unwrap();
        let rels = ["a/one.css", "two.ini"];
        let go = std::sync::Arc::new(std::sync::Barrier::new(8));
        let hs: Vec<_> = (0..8)
            .map(|i| {
                let (tk, home, go) = (tk.clone(), home.clone(), go.clone());
                std::thread::spawn(move || {
                    go.wait();
                    for _ in 0..200 {
                        if i == 0 {
                            // Points the link somewhere stale inside the toolkit dir.
                            let l = home.join("two.ini");
                            let t = shared::atomic::temp_name(&l).unwrap();
                            if std::os::unix::fs::symlink(tk.join("old"), &t).is_ok() {
                                let _ = std::fs::rename(&t, &l);
                            }
                        }
                        link_all(&tk, &home, &rels).unwrap();
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        link_all(&tk, &home, &rels).unwrap();
        for rel in rels {
            assert_eq!(std::fs::read_link(home.join(rel)).unwrap(), tk.join(rel));
        }
        let left: Vec<_> = std::fs::read_dir(&home)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(shared::atomic::TMP_SUFFIX))
            .collect();
        assert!(left.is_empty(), "temporaries left: {left:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    /// `wol appearance set dark` with a terminal open: foot switches sections at runtime only, so
    /// `[colors-dark]` must hold the dark theme and `[colors-light]` the light one with the same
    /// accent, whichever appearance wrote the file.
    #[test]
    fn foot_holds_both_appearances() {
        let dir = Path::new("/x/toolkits");
        let section = |foot: &str, name: &str| -> String {
            let rest = foot.split_once(&format!("\n[{name}]\n")).unwrap().1;
            rest.split("\n[").next().unwrap().to_owned()
        };
        let cat = crate::theme::accent::variants()
            .into_iter()
            .fold(ThemeCatalog::builtin(), ThemeCatalog::with);
        let bg = |t: &ResolvedTheme| format!("background={}\n", &t.tokens["surface"][1..]);
        let sel =
            |t: &ResolvedTheme| format!("selection-background={}\n", &t.tokens["accent"][1..]);
        for (id, other) in [
            ("wol-light", "wol-dark"),
            ("wol-dark", "wol-light"),
            ("wol-dark-blue", "wol-light-blue"),
        ] {
            let t = cat.resolve(id).unwrap();
            assert_eq!(counterpart(&t).map(|c| c.id).as_deref(), Some(other));
            let foot = file(&render(&t, dir), "foot/foot.ini").to_owned();
            let o = cat.resolve(other).unwrap();
            let (d, l) = if t.variant == Variant::Dark {
                (&t, &o)
            } else {
                (&o, &t)
            };
            for (name, want) in [("colors-dark", d), ("colors-light", l)] {
                let got = section(&foot, name);
                assert!(got.contains(&bg(want)), "{id} {name}: {foot}");
                assert!(got.contains(&sel(want)), "{id} {name}: the accent too");
            }
            assert_ne!(
                section(&foot, "colors-dark"),
                section(&foot, "colors-light")
            );
        }
        // A theme with no pair (not a WolOS accent theme) is in both sections.
        let mut t = theme("wol-dark");
        t.id = "my-night".into();
        assert!(counterpart(&t).is_none());
        let foot = file(&render(&t, dir), "foot/foot.ini").to_owned();
        assert_eq!(
            section(&foot, "colors-dark"),
            section(&foot, "colors-light")
        );
    }

    #[test]
    fn running_terminals_switch_by_signal() {
        assert_eq!(
            terminal_argv(&theme("wol-dark"), 1000).join(" "),
            "pkill --signal USR1 --uid 1000 --exact foot|footclient"
        );
        assert_eq!(terminal_argv(&theme("wol-light"), 7)[2], "USR2");
    }

    /// `wol-session` links the shipped defaults (`/usr/share/wolos/<rel>`) before the first theme.
    /// Those links are WolOS's: the theme replaces them (they pinned foot and every toolkit to the
    /// light default). A link elsewhere, even into /usr/share/wolos under another name, is kept.
    #[test]
    fn install_replaces_the_shipped_default_links() {
        let root = std::env::temp_dir().join(format!("wol-apps-shipped-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (tk, cfg) = (root.join("toolkits"), root.join("config"));
        std::fs::create_dir_all(cfg.join("foot")).unwrap();
        std::fs::create_dir_all(cfg.join("gtk-4.0")).unwrap();
        let shipped = |rel: &str| Path::new(SYSTEM_DIR).join(rel);
        std::os::unix::fs::symlink(shipped("foot/foot.ini"), cfg.join("foot/foot.ini")).unwrap();
        std::os::unix::fs::symlink(shipped("gtk-3.0/gtk.css"), cfg.join("gtk-4.0/gtk.css"))
            .unwrap();
        assert!(!foot_is_ours(&tk, &cfg));
        install(&theme("wol-dark"), &tk, &cfg).unwrap();
        assert_eq!(
            std::fs::read_link(cfg.join("foot/foot.ini")).unwrap(),
            tk.join("foot/foot.ini")
        );
        assert!(foot_is_ours(&tk, &cfg));
        let foot = std::fs::read_to_string(cfg.join("foot/foot.ini")).unwrap();
        assert!(foot.contains("initial-color-theme=dark\n"), "{foot}");
        assert_eq!(
            std::fs::read_link(cfg.join("gtk-4.0/gtk.css")).unwrap(),
            shipped("gtk-3.0/gtk.css"),
            "not the shipped default of this name: the person's"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn install_links_without_clobbering_user_files() {
        let root = std::env::temp_dir().join(format!("wol-apps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (tk, cfg) = (root.join("toolkits"), root.join("config"));
        std::fs::create_dir_all(cfg.join("foot")).unwrap();
        std::fs::write(cfg.join("foot/foot.ini"), "mine").unwrap();
        install(&theme("wol-dark"), &tk, &cfg).unwrap();
        let css = std::fs::read_to_string(cfg.join("gtk-4.0/gtk.css")).unwrap();
        assert!(css.contains("--accent-bg-color"));
        assert!(
            cfg.join("gtk-4.0/gtk.css")
                .symlink_metadata()
                .unwrap()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(cfg.join("foot/foot.ini")).unwrap(),
            "mine"
        );
        // Second install (theme switch) rewrites through the same links.
        install(&theme("wol-light"), &tk, &cfg).unwrap();
        let css2 = std::fs::read_to_string(cfg.join("gtk-4.0/gtk.css")).unwrap();
        assert_ne!(css, css2);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// C8-5: links the person made (a dotfiles manager) are kept, with their targets; a symlinked
    /// hyprland.lua stays a link and its target gets the `require` block; WolOS's own links are
    /// still refreshed.
    #[test]
    fn install_keeps_the_users_own_links() {
        let root = std::env::temp_dir().join(format!("wol-apps-dot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (tk, cfg, dot) = (root.join("toolkits"), root.join("config"), root.join("dot"));
        std::fs::create_dir_all(cfg.join("foot")).unwrap();
        std::fs::create_dir_all(cfg.join("hypr")).unwrap();
        std::fs::create_dir_all(&dot).unwrap();
        std::fs::write(dot.join("foot.ini"), "mine").unwrap();
        let mine = "hl.bind(\"SUPER + B\", hl.dsp.exec_cmd(\"firefox\"))\n";
        std::fs::write(dot.join("hyprland.lua"), mine).unwrap();
        std::os::unix::fs::symlink(dot.join("foot.ini"), cfg.join("foot/foot.ini")).unwrap();
        std::os::unix::fs::symlink("../../dot/hyprland.lua", cfg.join("hypr/hyprland.lua"))
            .unwrap();
        for id in ["wol-dark", "wol-light"] {
            install(&theme(id), &tk, &cfg).unwrap();
        }
        let foot = cfg.join("foot/foot.ini");
        assert_eq!(std::fs::read_link(&foot).unwrap(), dot.join("foot.ini"));
        assert_eq!(
            std::fs::read_to_string(dot.join("foot.ini")).unwrap(),
            "mine"
        );
        let hypr = cfg.join("hypr/hyprland.lua");
        assert!(
            hypr.symlink_metadata().unwrap().is_symlink(),
            "still a link"
        );
        let text = std::fs::read_to_string(dot.join("hyprland.lua")).unwrap();
        assert!(text.starts_with(mine), "{text}");
        let req = crate::prefs::USER_REQUIRE;
        assert_eq!(text.lines().filter(|l| *l == req).count(), 1, "{text}");
        // Ours is refreshed: a link into the toolkit dir is remade, a dangling one too.
        let css = cfg.join("gtk-4.0/gtk.css");
        assert_eq!(
            std::fs::read_link(&css).unwrap(),
            tk.join("gtk-4.0/gtk.css")
        );
        std::fs::remove_file(&css).unwrap();
        std::os::unix::fs::symlink(tk.join("gone"), &css).unwrap();
        install(&theme("wol-dark"), &tk, &cfg).unwrap();
        assert_eq!(
            std::fs::read_link(&css).unwrap(),
            tk.join("gtk-4.0/gtk.css")
        );
        // A dangling hyprland.lua link of the person's is left as it is.
        std::fs::remove_file(dot.join("hyprland.lua")).unwrap();
        install(&theme("wol-dark"), &tk, &cfg).unwrap();
        assert!(hypr.symlink_metadata().unwrap().is_symlink());
        assert!(!dot.join("hyprland.lua").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A switch writes the compositor colours where the user hyprland.lua's `hypr/wolos/*.lua`
    /// reads them, and every console file; the user file keeps its own lines and gets the
    /// `require` once.
    #[test]
    fn install_feeds_hyprland_and_the_consoles() {
        let root = std::env::temp_dir().join(format!("wol-apps-hypr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (tk, cfg) = (root.join("toolkits"), root.join("config"));
        std::fs::create_dir_all(cfg.join("hypr")).unwrap();
        let mine = "require(\"/usr/share/wolos/hypr/hyprland.lua\")\nhl.bind(\"SUPER + B\", hl.dsp.exec_cmd(\"firefox\"))\n";
        std::fs::write(cfg.join("hypr/hyprland.lua"), mine).unwrap();
        for _ in 0..2 {
            install(&theme("catppuccin-latte"), &tk, &cfg).unwrap();
        }
        let user = std::fs::read_to_string(cfg.join("hypr/hyprland.lua")).unwrap();
        assert!(user.starts_with(mine), "{user}");
        let req = crate::prefs::USER_REQUIRE;
        assert_eq!(user.lines().filter(|l| *l == req).count(), 1, "{user}");
        let hypr = cfg.join(super::super::hypr::HYPR_THEME_FILE);
        assert!(hypr.to_string_lossy().ends_with("hypr/wolos/theme.lua"));
        assert!(hypr.symlink_metadata().unwrap().is_symlink());
        assert!(
            std::fs::read_to_string(&hypr)
                .unwrap()
                .contains("hl.config({ [\"plugin.hyprbars.bar_color\"] = \"rgba(eff1f5e6)\" })")
        );

        for rel in super::super::console::FILES {
            assert!(tk.join(rel).is_file(), "{rel}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The circle has to be seen on the bar and the glyph on the circle (WCAG 1.4.11, 3:1), in the
    /// two themes the image starts with.
    #[test]
    fn window_controls_are_visible() {
        for id in ["wol-light", "wol-dark"] {
            let t = theme(id);
            let (idle, hover, _) = controls_colors(&t);
            let c = |h: &str| Color::parse(h).unwrap();
            let on = |k: &str| token_color(&t, k);
            assert!(
                Color::contrast(c(&idle), on("surface")) >= 3.0,
                "{id}: circle"
            );
            assert!(Color::contrast(on("text"), c(&idle)) >= 3.0, "{id}: glyph");
            assert!(
                Color::contrast(on("accentText"), c(&hover)) >= 3.0,
                "{id}: glyph on the accent"
            );
        }
    }

    /// The icon theme every builtin theme names is the one the image ships: the `Name` of
    /// `design/icons/index.theme`, which is also what the GTK settings carry.
    #[test]
    fn builtin_themes_name_the_shipped_icon_theme() {
        let index = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design/icons/index.theme"),
        )
        .unwrap();
        let name = index
            .lines()
            .find_map(|l| l.strip_prefix("Name="))
            .map(str::trim);
        assert_eq!(name, Some(ICON_THEME));
        for spec in ThemeCatalog::builtin().list() {
            let t = theme(&spec.id);
            assert!(
                t.icon_theme.as_deref().is_none_or(|n| n == ICON_THEME),
                "{}: {:?}",
                spec.id,
                t.icon_theme
            );
        }
        for id in ["wol-light", "wol-dark"] {
            assert_eq!(theme(id).icon_theme.as_deref(), Some(ICON_THEME), "{id}");
        }
        let files = render(&theme("wol-dark"), Path::new("/x"));
        let ini = file(&files, "gtk-3.0/settings.ini");
        assert!(ini.contains(&format!("gtk-icon-theme-name={ICON_THEME}")));
    }

    /// The image ships the default theme's files (`desktop/overlay`); `WOL_REGEN=1` rewrites them.
    #[test]
    fn system_copy_is_the_default_render() {
        const SPDX_LUA: &str = "-- SPDX-License-Identifier: GPL-2.0-or-later\n";
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../desktop/overlay/usr/share/wolos");
        // settings.ini names `WolOS-wol-light`: the system theme dir resolves it before any
        // `wol theme set`.
        let link = root
            .join("../themes")
            .join(gtk3_theme_name(&theme("wol-light")));
        assert_eq!(
            std::fs::read_link(&link).ok(),
            Some(Path::new(SYSTEM_DIR).join(GTK3_THEME_DIR)),
            "{}",
            link.display()
        );
        for f in render(&theme("wol-light"), Path::new(SYSTEM_DIR)) {
            let p = root.join(f.rel);
            // A shipped .lua file starts with its SPDX line; the render does not.
            let shipped = if f.rel.ends_with(".lua") {
                format!("{SPDX_LUA}{}", f.text)
            } else {
                f.text.clone()
            };
            if std::env::var_os("WOL_REGEN").is_some() {
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, &shipped).unwrap();
            }
            assert_eq!(
                std::fs::read_to_string(&p).ok().as_deref(),
                Some(shipped.as_str()),
                "{}",
                f.rel
            );
        }
    }

    /// QA-LIVE-19: the toolkit files are written before the caller says "changed": a config
    /// home that cannot take them is an Err, not a log line on a worker thread.
    #[test]
    fn write_files_fails_loudly_when_the_files_cannot_be_written() {
        let root = std::env::temp_dir().join(format!("wol-wf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // A file where the config folder should be: every create_dir_all under it fails.
        let blocked = root.join("cfg");
        std::fs::write(&blocked, "x").unwrap();
        let e = write_files("wol-dark", &blocked).unwrap_err();
        assert!(!e.is_empty(), "{e}");
        let ok = root.join("ok");
        let t = write_files("wol-dark", &ok).expect("a writable home");
        assert_eq!(t.id, "wol-dark");
        assert!(user_dir(&ok).join("gtk-3.0/settings.ini").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
