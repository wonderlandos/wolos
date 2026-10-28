//! Third-party app theming from one resolved WolOS theme: GTK4/libadwaita, GTK3 (adw-gtk3),
//! Qt (qt6ct Fusion palette, Kvantum), foot, Firefox (`firefox`), GtkSourceView (`sourceview`),
//! zsh colours. Pure renderers + `install` (files under a toolkit dir, linked into
//! `$XDG_CONFIG_HOME`/`$XDG_DATA_HOME` without replacing the user's own files) + `gsettings` keys.

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
/// adw-gtk3 install dir (`image/fetch-theme.sh`).
pub const ADW_GTK3_DIR: &str = "/usr/share/themes";

pub fn gtk3_theme_name(t: &ResolvedTheme) -> String {
    format!("WolOS-{}", t.id)
}

/// Generated files linked into `$XDG_DATA_HOME` at the same relative path.
pub const DATA_LINKS: [&str; 2] = super::sourceview::FILES;
/// Sourced by the system zshrc from `$XDG_CONFIG_HOME/wolos/toolkits/` (prompt colours).
pub const SHELL_COLORS: &str = "zsh/colors.zsh";
/// System copy rendered from the default theme (`image/build.sh`); users get their own under
/// `$XDG_CONFIG_HOME/wolos/toolkits`.
pub const SYSTEM_DIR: &str = "/usr/share/wolos";
/// Shipped icon theme (`design/icons`, inherits Adwaita); the catalog's `iconTheme` names themes
/// the image does not install, so apps always get this one, like the shell.
pub const ICON_THEME: &str = "WolOS";
pub const FONT: &str = "Inter 11";
pub const MONO_FONT: &str = "JetBrains Mono:size=11";
/// macOS order: window controls on the left.
pub const BUTTON_LAYOUT: &str = "close,minimize,maximize:";
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
fn hue_sat(c: Color) -> (f64, f64) {
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
/// Window buttons as Kit AppWindow / hyprbars draw them (kit goldens, pixel edges): 12 px
/// circles, 22 px apart, first centre 23 px from the window's left edge. Every toolkit matches.
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
            text: format!(
                "@import url(\"file://{ADW_GTK3_DIR}/{}/gtk-3.0/gtk.css\");\n{}",
                gtk3_theme(t.variant == Variant::Dark),
                gtk3_css(t)
            ),
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
    let bg = token_color(t, "surface");
    let r = |tok: &str| readable(token_color(t, tok), bg, 4.5).to_hex();
    let suggest = readable(mix(token_color(t, "dim"), bg, 0.35), bg, 3.0).to_hex();
    let mut s = format!(
        "# WolOS {} — generated by `wol theme`; edits are overwritten.\nWOLOS_THEME={}\n",
        t.id, t.id
    );
    for (var, v) in [
        ("WOLOS_PROMPT_ACCENT", r("accent")),
        ("WOLOS_PROMPT_MUTED", r("dim")),
        ("WOLOS_PROMPT_OK", r("success")),
        ("WOLOS_PROMPT_ERROR", r("danger")),
        ("WOLOS_PROMPT_SUGGEST", suggest),
    ] {
        s += &format!("{var}='{v}'\n");
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
    s += &traffic_lights(
        t,
        "windowcontrols > button",
        ["close", "minimize", "maximize"],
    );
    s
}

/// Kit AppWindow traffic lights (`button.light` in kit.css): 12 px token-coloured circles with a
/// hairline rim, 22 px centre to centre (measured on kit goldens); the glyph shows on hover only.
fn traffic_lights(t: &ResolvedTheme, button: &str, classes: [&str; 3]) -> String {
    // The header bar stretches buttons vertically, so the circle is drawn on the (centred) image.
    let mut s = format!(
        "{button} {{ min-width: {LIGHT_SIZE}px; min-height: {LIGHT_SIZE}px; padding: 0; margin: 0; border: none; background: none; box-shadow: none; }}\n\
         {button} image {{ min-width: {LIGHT_SIZE}px; min-height: {LIGHT_SIZE}px; padding: 0; margin: 0; border-radius: 999px; -gtk-icon-size: 8px; color: transparent; box-shadow: inset 0 0 0 1px rgba(0,0,0,0.12); }}\n\
         {button}:hover image {{ color: rgba(0,0,0,0.55); }}\n"
    );
    for (class, tok) in classes.iter().zip(["danger", "warning", "success"]) {
        s += &format!(
            "{button}.{class} image {{ background: {}; }}\n",
            token(t, tok)
        );
    }
    s
}

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

/// GTK 3 traffic lights: the circle is the 12 px button (GTK 3 centres titlebuttons). Chromium's
/// GTK mode renders only the button background under GTK 3, and GTK 3 has no `-gtk-icon-size`:
/// the 16 px icon is pulled to 12 px by a negative margin and scaled, transparent until hover.
/// GTK 3 and Chromium place buttons at header padding + margins, 6 px apart.
fn gtk3_traffic_lights(t: &ResolvedTheme) -> String {
    let side = (LIGHT_PITCH - LIGHT_SIZE - GTK3_BUTTON_SPACING) / 2;
    let first = LIGHT_FIRST_CENTRE - LIGHT_SIZE / 2 - GTK3_HEADER_PADDING;
    let mut s = format!(
        "{} {{ min-width: {LIGHT_SIZE}px; min-height: {LIGHT_SIZE}px; margin: 0 {side}px; padding: 0; border: none; border-radius: 999px; background-image: none; box-shadow: inset 0 0 0 1px rgba(0,0,0,0.12); }}\n\
         {} {{ margin-left: {first}px; }}\n\
         {} {{ margin: -2px; padding: 0; color: transparent; background: none; box-shadow: none; -gtk-icon-transform: scale(0.6); }}\n\
         {} {{ color: rgba(0,0,0,0.55); }}\n",
        titlebutton(""),
        titlebutton(".close"),
        titlebutton(" image"),
        titlebutton(":hover image"),
    );
    for (class, tok) in [
        ("close", "danger"),
        ("minimize", "warning"),
        ("maximize", "success"),
    ] {
        s += &format!(
            "{} {{ background-color: {}; }}\n",
            titlebutton(&format!(".{class}")),
            token(t, tok)
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
    s += &gtk3_traffic_lights(t);
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

/// foot: token colours + a fixed ANSI set tuned per variant; translucent background.
fn foot_ini(t: &ResolvedTheme) -> String {
    let p = Pal::new(t);
    let h = |c: Color| c.to_hex()[1..].to_string();
    let c = |k| p.c(t, k);
    let (red, green, yellow, accent) = (c("danger"), c("success"), c("warning"), c("accent"));
    let (black, white) = if p.dark {
        (Color::rgb(58, 58, 64), Color::rgb(229, 229, 234))
    } else {
        (Color::rgb(29, 29, 31), Color::rgb(199, 199, 204))
    };
    let blue = Color::rgb(10, 132, 255);
    let cyan = if p.dark {
        Color::rgb(100, 210, 255)
    } else {
        Color::rgb(0, 122, 168)
    };
    let regular = [black, red, green, yellow, blue, accent, cyan, white];
    let lift = if p.dark {
        Color::WHITE
    } else {
        Color::rgb(0, 0, 0)
    };
    let mut s = format!(
        "# WolOS {} — generated by `wol theme`; edits are overwritten.\n[main]\nfont={MONO_FONT}\npad=12x12\n\n[colors-dark]\nalpha={FOOT_ALPHA}\nbackground={}\nforeground={}\nselection-background={}\nselection-foreground={}\n",
        t.id,
        h(p.bg),
        h(c("text")),
        h(accent),
        h(c("accentText")),
    );
    for (i, col) in regular.iter().enumerate() {
        s += &format!("regular{i}={}\n", h(*col));
    }
    for (i, col) in regular.iter().enumerate() {
        s += &format!("bright{i}={}\n", h(mix(*col, lift, 0.25)));
    }
    s
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

/// Session/shell entry: resolve `id`, install the user toolkit files, push gsettings (blocking;
/// call off the UI thread). gsettings failures are ignored (not every host has it).
pub fn apply(id: &str, config_home: &Path) -> Result<(), String> {
    let t = super::ThemeCatalog::load(&super::search_paths(config_home))
        .resolve(id)
        .map_err(|e| format!("{e:?}"))?;
    install(&t, &user_dir(config_home), config_home).map_err(|e| e.to_string())?;
    if let Some(data) = data_home(|k| std::env::var(k).ok()) {
        link_data(&t, &user_dir(config_home), &data).map_err(|e| e.to_string())?;
    }
    for a in gsettings_argv(&t) {
        let _ = std::process::Command::new(&a[0])
            .args(&a[1..])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    Ok(())
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
            std::fs::remove_file(e.path())?;
        }
    }
    let link = themes.join(&name);
    if link.symlink_metadata().is_err() {
        std::os::unix::fs::symlink(&target, &link)?;
    }
    Ok(())
}

/// Writes `render` into `dir` (atomic per file) and links each of `LINKS` from `config_home`
/// unless the user already has a real file there.
pub fn install(t: &ResolvedTheme, dir: &Path, config_home: &Path) -> std::io::Result<()> {
    for f in render(t, dir) {
        let p = dir.join(f.rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = p.with_extension("tmp");
        std::fs::write(&tmp, f.text)?;
        std::fs::rename(&tmp, &p)?;
    }
    link_all(dir, config_home, &LINKS)
}

fn link_all(dir: &Path, home: &Path, rels: &[&str]) -> std::io::Result<()> {
    for rel in rels {
        let link = home.join(rel);
        match link.symlink_metadata() {
            Ok(m) if !m.is_symlink() => continue,
            Ok(_) => std::fs::remove_file(&link)?,
            Err(_) => {}
        }
        if let Some(d) = link.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::os::unix::fs::symlink(dir.join(rel), &link)?;
    }
    Ok(())
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
    fn window_controls_are_kit_traffic_lights() {
        let t = theme("wol-light");
        let f = render(&t, Path::new("/x"));
        let gtk4 = file(&f, "gtk-4.0/gtk.css");
        assert!(gtk4.contains(&format!(
            "windowcontrols > button.close image {{ background: {}; }}",
            t.tokens["danger"]
        )));
        assert!(gtk4.contains("min-width: 12px; min-height: 12px;"));
        assert!(
            gtk4.contains("color: transparent;"),
            "glyph hidden until hover"
        );
        // GTK 3: the circle is the button itself. Chromium's GTK mode (Helium) renders only the
        // titlebutton background under GTK 3, never the image's (ui/gtk/nav_button_provider_gtk.cc).
        let gtk3 = file(&f, GTK3_THEME_CSS);
        for (class, tok) in [
            ("close", "danger"),
            ("minimize", "warning"),
            ("maximize", "success"),
        ] {
            assert!(
                gtk3.contains(&format!(
                    "{} {{ background-color: {}; }}",
                    titlebutton(&format!(".{class}")),
                    t.tokens[tok]
                )),
                "{class}"
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
        assert!(file(&f, "gtk-4.0/gtk.css").contains("windowcontrols { border-spacing: 10px; }"));
        // Header bar (bleeds 1 px past the window) pads 17, controls inset 1: close covers x 17..28,
        // y 20..31 of the window, the Kit golden's pixels (measured on nautilus and calculator).
        let g4 = file(&f, "gtk-4.0/gtk.css");
        assert!(g4.contains("headerbar > windowhandle > box { padding: 10px 17px; }"));
        assert!(g4.contains("windowcontrols.start { margin-left: 1px;"));
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
    /// pixel edges): 12 px circles, centres 23/45/67 px from the window's left edge.
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
            "{} {{ margin-left: {first}px; }}",
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
        // GTK 4 windowcontrols: border-spacing between 12 px images; start inset.
        let g4 = file(&f, "gtk-4.0/gtk.css");
        assert!(g4.contains("windowcontrols { border-spacing: 10px; }"));
        assert_eq!(LIGHT_SIZE + 10, LIGHT_PITCH);
        // Firefox: container inset + 5 px button padding + radius.
        let ff = crate::theme::firefox::chrome_css(&theme("wol-light"));
        assert!(ff.contains("margin-inline-start: 12px !important;"));
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
                ini3.contains(&format!("gtk-theme-name=WolOS-{id}\n")),
                "{id}"
            );
            for ini in [ini3, file(&f, "gtk-4.0/settings.ini")] {
                assert!(ini.contains("gtk-decoration-layout=close,minimize,maximize:"));
                assert!(ini.contains("gtk-font-name=Inter 11"));
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
            assert_eq!(foot.matches("regular").count(), 8);
            assert_eq!(foot.matches("bright").count(), 8);
            // foot 1.28 rejects `[colors]` (renamed `[colors-dark]`); "monospace" matched Font Awesome.
            assert!(foot.contains("\n[colors-dark]\n") && !foot.contains("[colors]"));
            assert!(foot.contains("font=JetBrains Mono:size=11\n"));
        }
    }

    /// Contract with the system zshrc (`/etc/zshrc`): it sources
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
            assert!(th.contains("button.titlebutton.close { background-color:"));
            let user = file(&f, "gtk-3.0/gtk.css");
            assert!(
                !user.contains("@define-color") && !user.contains("titlebutton"),
                "{id}"
            );
            assert!(
                file(&f, "gtk-3.0/settings.ini").contains(&format!("gtk-theme-name=WolOS-{id}\n"))
            );
            assert!(gsettings(&t).contains(&(
                "org.gnome.desktop.interface",
                "gtk-theme",
                format!("WolOS-{id}")
            )));
        }
        let root = std::env::temp_dir().join(format!("wol-gtk3th-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (tk, cfg, data) = (root.join("tk"), root.join("cfg"), root.join("data"));
        for id in ["wol-light", "nord"] {
            install(&theme(id), &tk, &cfg).unwrap();
            link_data(&theme(id), &tk, &data).unwrap();
        }
        let css = std::fs::read_to_string(data.join("themes/WolOS-nord/gtk-3.0/gtk.css")).unwrap();
        assert!(css.contains("WolOS nord"));
        assert!(
            !data.join("themes/WolOS-wol-light").exists(),
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
            ("wol-light-blue", "blue"),
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

    #[test]
    fn gsettings_follow_the_variant() {
        let l = gsettings(&theme("wol-light"));
        let d = gsettings(&theme("wol-dark"));
        let get =
            |v: &[(&str, &str, String)], k: &str| v.iter().find(|x| x.1 == k).unwrap().2.clone();
        assert_eq!(get(&l, "color-scheme"), "default");
        assert_eq!(get(&d, "color-scheme"), "prefer-dark");
        assert_eq!(get(&d, "gtk-theme"), "WolOS-wol-dark");
        assert_eq!(get(&l, "accent-color"), "purple");
        assert_eq!(get(&l, "button-layout"), "close,minimize,maximize:");
        assert_eq!(get(&l, "font-name"), "Inter 11");
        assert_eq!(
            get(&d, "icon-theme"),
            "WolOS",
            "the shipped theme, not the catalog's"
        );
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

    /// The image ships the default theme's files (`desktop/overlay`); `WOL_REGEN=1` rewrites them.
    #[test]
    fn system_copy_is_the_default_render() {
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
            if std::env::var_os("WOL_REGEN").is_some() {
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, &f.text).unwrap();
            }
            assert_eq!(
                std::fs::read_to_string(&p).ok().as_deref(),
                Some(f.text.as_str()),
                "{}",
                f.rel
            );
        }
    }
}
