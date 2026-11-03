// SPDX-License-Identifier: BSD-2-Clause
//! Compositor colours from the theme (ADR 0031 themes v2): the title strip (hyprbars bar, title,
//! inactive look) and window shadows. The shipped hyprland.lua holds no colour of its own: it
//! loads [`SYSTEM_HYPR_THEME`] (this render of the default theme) and the window-button file
//! [`LIGHTS_LUA`]; `wol theme set` writes the user copy at [`HYPR_THEME_FILE`] (loaded by the user
//! hyprland.lua's `hypr/wolos/*.lua`, after the system one) and sets the same keys live.
use super::ResolvedTheme;
use super::apps::{controls_colors, mix, token_color};
use shared::theme::Color;
pub use shared::theme::{HYPR_THEME_FILE, SYSTEM_HYPR_THEME};

/// Compositor `rgba(rrggbbaa)`.
pub fn rgba(c: Color, alpha: u8) -> String {
    format!("rgba({}{alpha:02x})", &c.to_hex()[1..])
}

/// (`hyprctl keyword` name, value), in file order.
pub fn keywords(t: &ResolvedTheme) -> Vec<(&'static str, String)> {
    let (bar, text) = (token_color(t, "surface"), token_color(t, "text"));
    let black = Color::rgb(0, 0, 0);
    let (_, accent, pressed) = controls_colors(t);
    // controls_colors renders `#rrggbb`, which always parses back.
    let parse = |hex: &str| Color::parse(hex).unwrap_or(black);
    let (accent, pressed) = (parse(&accent), parse(&pressed));
    let on_accent = token_color(t, "accentText");
    vec![
        ("plugin:hyprbars:bar_color", rgba(bar, 0xe6)),
        ("plugin:hyprbars:col.text", rgba(text, 0xff)),
        // Inactive windows look inactive: dimmed buttons, a flatter bar, a dimmer title
        // (bar_color_inactive / col.text_inactive: desktop/hyprland/hyprbars patch).
        (
            "plugin:hyprbars:inactive_button_color",
            rgba(mix(bar, text, 0.22), 0xff),
        ),
        ("plugin:hyprbars:bar_color_inactive", rgba(bar, 0xcc)),
        ("plugin:hyprbars:col.text_inactive", rgba(text, 0x8c)),
        // The button under the pointer: the accent, deeper while pressed, its glyph in the text on
        // the accent, the client-side controls' colours (apps::controls_colors; hyprbars patch 0002).
        ("plugin:hyprbars:button_hover_color", rgba(accent, 0xff)),
        ("plugin:hyprbars:button_pressed_color", rgba(pressed, 0xff)),
        (
            "plugin:hyprbars:col.button_hover_text",
            rgba(on_accent, 0xff),
        ),
        ("decoration:shadow:color", rgba(black, 0x59)),
        ("decoration:shadow:color_inactive", rgba(black, 0x33)),
    ]
}

/// The Lua configuration of the compositor 0.57 series (WolOS 0.13): the [`keywords`] as `hl.config`
/// calls ([`lua`]). The names of the Lua files are the shared ones.
pub const SYSTEM_HYPR_THEME_LUA: &str = SYSTEM_HYPR_THEME;
/// Under `$XDG_CONFIG_HOME`: the active theme's colours for the Lua configuration.
pub const HYPR_THEME_FILE_LUA: &str = HYPR_THEME_FILE;
/// The window buttons (`hl.plugin.hyprbars.add_button`, one fixed neutral and ink glyphs, not a
/// theme value), loaded once by hyprland.lua: each call adds a button.
pub const LIGHTS_LUA: &str = "/usr/share/wolos/hypr/wolos/lights.lua";

/// The Lua key of a keyword: the compositor maps `:` to `.` and `-` to `_`
/// (`plugin:hyprbars:col.text` is `plugin.hyprbars.col.text`).
pub fn lua_key(keyword: &str) -> String {
    keyword.replace(':', ".").replace('-', "_")
}

/// A Lua string literal (double quotes; `\`, `"` and control characters escaped).
pub fn lua_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c if u32::from(c) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\{:03}", u32::from(c)))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A keyword value as the Lua literal its option type takes: `true`/`false`, a number, or a string
/// (colours, gradients written as one colour, vectors). The compositor refuses a number in quotes.
pub fn lua_value(v: &str) -> String {
    let t = v.trim();
    let numeric = !t.is_empty()
        && t.parse::<f64>().is_ok_and(f64::is_finite)
        && t.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '-' | '.'));
    match t {
        "true" | "false" => t.to_string(),
        _ if numeric => t.to_string(),
        _ => lua_str(v),
    }
}

/// One `hl.config` call that sets `keyword` (one line).
pub fn lua_set(keyword: &str, value: &str) -> String {
    format!(
        "hl.config({{ [{}] = {} }})",
        lua_str(&lua_key(keyword)),
        lua_value(value)
    )
}

/// `hypr/wolos/theme.lua`: one `hl.config` line per keyword. The title strip's keys exist once its
/// plugin is loaded; the compositor loads plugins after its first pass of the configuration and
/// then reads it again, so the first pass skips them.
pub fn lua(t: &ResolvedTheme) -> String {
    let mut s = format!(
        "-- WolOS {} — generated by `wol theme`; edits are overwritten.\n\
         local strip = hl.plugin.hyprbars ~= nil\n",
        t.id
    );
    for (k, v) in keywords(t) {
        let line = lua_set(k, &v);
        if k.starts_with("plugin:") {
            s += &format!("if strip then {line} end\n");
        } else {
            s += &format!("{line}\n");
        }
    }
    s
}

/// Live switch on the Lua configuration: `hyprctl eval` of each line's `hl.config` call.
pub fn lua_argv(t: &ResolvedTheme) -> Vec<Vec<String>> {
    keywords(t)
        .into_iter()
        .map(|(k, v)| vec!["hyprctl".into(), "eval".into(), lua_set(k, &v)])
        .collect()
}

/// The binds of a WolOS Lua configuration (`hyprland.lua`, `tiling.lua`) as `.conf` `bind` lines,
/// for the readers of binds ([`crate::shortcuts::parse`]) while both forms ship. Reads the forms
/// those files use: `bind(KEYS, "cmd"[, OPTS])` (exec), `hl.bind(KEYS, hl.dsp.…(…)[, OPTS])`,
/// `local mod =`, `local NAME = { flags }`, `local function` helpers and
/// `hl.define_submap("name", function() … end)`. A form it does not know comes out as dispatcher
/// `?lua` with the call as its argument.
pub fn conf_binds(lua: &str) -> String {
    let mut vars: Vec<(String, String)> = Vec::new();
    let mut opts: Vec<(String, String)> = Vec::new();
    let mut out = String::new();
    let mut in_submap = false;
    let mut in_function = false;
    for raw in lua.lines() {
        let line = raw.split(" -- ").next().unwrap_or("").trim();
        if line.starts_with("--") || line.is_empty() {
            continue;
        }
        // A helper's own body (`local function bind(…) hl.bind(…) end`) binds nothing.
        if line.starts_with("local function ") {
            in_function = true;
            continue;
        }
        if in_function {
            in_function = line != "end";
            continue;
        }
        if let Some(rest) = line.strip_prefix("local ") {
            if let Some((name, val)) = rest.split_once(" = ") {
                let val = val.trim();
                if let Some(v) = unquote(val) {
                    vars.push((name.trim().to_string(), v));
                } else if val.starts_with('{') {
                    opts.push((name.trim().to_string(), bind_flags(val)));
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("hl.define_submap(") {
            let name = split_args(rest.trim_end_matches(" function()"))
                .first()
                .and_then(|a| unquote(a))
                .unwrap_or_default();
            out += &format!("submap = {name}\n");
            in_submap = true;
            continue;
        }
        if in_submap && line == "end)" {
            out += "submap = reset\n";
            in_submap = false;
            continue;
        }
        let (exec, body) = if let Some(b) = line.strip_prefix("bind(") {
            (true, b)
        } else if let Some(b) = line.strip_prefix("hl.bind(") {
            (false, b)
        } else {
            continue;
        };
        let Some(body) = body.strip_suffix(')') else {
            continue;
        };
        let args = split_args(body);
        let keys = args
            .first()
            .map(|k| keys_expr(k, &vars))
            .unwrap_or_default();
        let (dispatcher, arg) = match args.get(1) {
            Some(a) if exec => ("exec".to_string(), unquote(a).unwrap_or_else(|| a.clone())),
            Some(a) => dsp_conf(a),
            None => ("?lua".into(), body.to_string()),
        };
        let flags = match args.get(2).map(|o| o.trim()) {
            Some(o) if o.starts_with('{') => bind_flags(o),
            Some(o) => opts
                .iter()
                .find(|(n, _)| n == o)
                .map_or_else(|| format!("?{o}"), |(_, f)| f.clone()),
            None => String::new(),
        };
        let mut parts: Vec<&str> = keys.split(" + ").collect();
        let key = parts.pop().unwrap_or_default();
        let mut l = format!("bind{flags} = {}, {key}, {dispatcher}", parts.join(" "));
        if !arg.is_empty() {
            l += &format!(", {arg}");
        }
        out += &l;
        out.push('\n');
    }
    out
}

/// Top-level comma-separated arguments of a call body (quotes, brackets and braces nest).
fn split_args(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth, mut quote) = (Vec::new(), String::new(), 0i32, None::<char>);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                cur.push(c);
                if c == '\\' {
                    if let Some(n) = chars.next() {
                        cur.push(n);
                    }
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => {
                    quote = Some(c);
                    cur.push(c);
                }
                '(' | '{' | '[' => {
                    depth += 1;
                    cur.push(c);
                }
                ')' | '}' | ']' => {
                    depth -= 1;
                    cur.push(c);
                }
                ',' if depth == 0 => out.push(std::mem::take(&mut cur).trim().to_string()),
                c => cur.push(c),
            },
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// The text of a Lua string literal (`"…"`, `'…'`, `[[…]]`), or `None`.
fn unquote(s: &str) -> Option<String> {
    let s = s.trim();
    if let Some(b) = s.strip_prefix("[[").and_then(|b| b.strip_suffix("]]")) {
        return Some(b.to_string());
    }
    let q = s.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let b = s.strip_prefix(q)?.strip_suffix(q)?;
    let (mut out, mut it) = (String::new(), b.chars());
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// `mod .. " + Space"`, `"CTRL + " .. mod .. " + Q"`: the key string with `local` names filled in.
fn keys_expr(e: &str, vars: &[(String, String)]) -> String {
    e.split("..")
        .map(str::trim)
        .map(|p| {
            unquote(p).unwrap_or_else(|| {
                vars.iter()
                    .find(|(n, _)| n == p)
                    .map_or_else(|| format!("?{p}"), |(_, v)| v.clone())
            })
        })
        .collect()
}

/// `{ locked = true, repeating = true }` as `.conf` bind letters (`el`).
fn bind_flags(table: &str) -> String {
    let on = |f: &str| table.contains(&format!("{f} = true"));
    [
        ("repeating", 'e'),
        ("locked", 'l'),
        ("mouse", 'm'),
        ("non_consuming", 'n'),
        ("release", 'r'),
    ]
    .iter()
    .filter(|(f, _)| on(f))
    .map(|(_, c)| *c)
    .collect()
}

/// A `hl.dsp` dispatcher as the `.conf` (dispatcher, argument).
fn dsp_conf(d: &str) -> (String, String) {
    let field = |name: &str| -> Option<String> {
        let at = d.find(&format!("{name} = "))? + name.len() + 3;
        let rest = &d[at..];
        let end = rest.find([',', '}']).unwrap_or(rest.len());
        let v = rest[..end].trim();
        Some(unquote(v).unwrap_or_else(|| v.to_string()))
    };
    let dir = |v: Option<String>| {
        v.and_then(|v| v.chars().next())
            .map(String::from)
            .unwrap_or_default()
    };
    let (name, args) = d.split_once('(').unwrap_or((d, ""));
    let r = match name.trim() {
        "hl.dsp.exec_cmd" => Some((
            "exec",
            unquote(args.trim_end_matches(')')).unwrap_or_default(),
        )),
        "hl.dsp.submap" => Some((
            "submap",
            unquote(args.trim_end_matches(')')).unwrap_or_default(),
        )),
        "hl.dsp.window.drag" => Some(("movewindow", String::new())),
        "hl.dsp.window.fullscreen" => match field("mode").as_deref() {
            Some("maximized") => Some(("fullscreen", "1".into())),
            _ => Some(("fullscreen", "0".into())),
        },
        "hl.dsp.window.resize" if args.trim() == ")" => Some(("resizewindow", String::new())),
        "hl.dsp.window.resize" => field("x")
            .zip(field("y"))
            .filter(|_| field("relative").as_deref() == Some("true"))
            .map(|(x, y)| ("resizeactive", format!("{x} {y}"))),
        "hl.dsp.window.move" => field("direction").map(|v| ("movewindow", dir(Some(v)))),
        "hl.dsp.window.float" if field("action").as_deref() == Some("toggle") => {
            Some(("togglefloating", String::new()))
        }
        "hl.dsp.focus" => field("workspace")
            .map(|w| ("workspace", w))
            .or_else(|| field("direction").map(|v| ("movefocus", dir(Some(v))))),
        _ => None,
    };
    r.map_or_else(
        || ("?lua".into(), d.to_string()),
        |(n, a)| (n.to_string(), a),
    )
}

/// A compositor colour literal (`rgb(`, `rgba(`, `0x` + 6 or more hex digits) in the code part of
/// `line` (comments do not count).
pub fn has_colour(line: &str) -> bool {
    let code = line.split('#').next().unwrap_or("");
    let code = code.split("--").next().unwrap_or("");
    code.contains("rgb(")
        || code.contains("rgba(")
        || code.match_indices("0x").any(|(i, _)| {
            code[i + 2..]
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .count()
                >= 6
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeCatalog;
    use std::path::{Path, PathBuf};

    const SHARE: &str = "desktop/overlay/usr/share/wolos/";

    fn path(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel)
    }

    fn repo(rel: &str) -> String {
        std::fs::read_to_string(path(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    /// Themes own every colour: the shipped hyprland.lua has none left but the black every
    /// compositor starts on (ADR 0012: the loader splash, the greeter and the desktop are one
    /// black picture; `crates/controller/tests/image_seamless.rs`); the theme and the buttons load
    /// once, after the plugin and the strip's own settings.
    #[test]
    fn hyprland_lua_has_no_colour_literals() {
        let lua = repo(&format!("{SHARE}hypr/hyprland.lua"));
        let left: Vec<&str> = lua
            .lines()
            .filter(|l| has_colour(l))
            .filter(|l| l.trim() != "background_color = \"rgb(000000)\",")
            .collect();
        assert!(left.is_empty(), "colour literals in hyprland.lua: {left:?}");
        for want in [SYSTEM_HYPR_THEME_LUA, LIGHTS_LUA] {
            let call = format!("require({})", lua_str(want));
            assert_eq!(
                lua.lines().filter(|l| l.trim() == call).count(),
                1,
                "hyprland.lua loads {want} once"
            );
            let rel = want.strip_prefix("/usr/share/wolos/").unwrap();
            assert!(path(&format!("{SHARE}{rel}")).is_file(), "{want} ships");
        }
        let at = |needle: &str| lua.find(needle).unwrap_or_else(|| panic!("{needle}"));
        let theme = at(&format!("require({})", lua_str(SYSTEM_HYPR_THEME_LUA)));
        assert!(at("hl.plugin.load(\"/usr/lib/hyprland/plugins/libhyprbars.so\")") < theme);
        assert!(at("bar_height = ") < theme);
    }

    /// The shipped theme.lua is this module's render of the theme it names, under its SPDX line.
    #[test]
    fn shipped_theme_lua_is_the_render() {
        let rel = SYSTEM_HYPR_THEME.strip_prefix("/usr/share/wolos/").unwrap();
        let file = repo(&format!("{SHARE}{rel}"));
        let shipped = file
            .strip_prefix("-- SPDX-License-Identifier: GPL-2.0-or-later\n")
            .expect("theme.lua starts with its SPDX line");
        let id = shipped
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("-- WolOS "))
            .and_then(|l| l.split(' ').next())
            .expect("theme.lua names its theme");
        let t = ThemeCatalog::builtin().resolve(id).unwrap();
        assert_eq!(shipped, lua(&t), "theme.lua");
    }

    #[test]
    fn lua_keys_and_literals() {
        assert_eq!(
            lua_key("plugin:hyprbars:col.text"),
            "plugin.hyprbars.col.text"
        );
        assert_eq!(
            lua_key("input:touchpad:tap-to-click"),
            "input.touchpad.tap_to_click"
        );
        for (v, want) in [
            ("true", "true"),
            ("false", "false"),
            ("6", "6"),
            ("-40", "-40"),
            ("0.20", "0.20"),
            ("rgba(eff1f5e6)", "\"rgba(eff1f5e6)\""),
            ("0 8", "\"0 8\""),
            ("1e3", "\"1e3\""),
            ("inf", "\"inf\""),
            ("", "\"\""),
            ("a\"b\\c\nd", "\"a\\\"b\\\\c\\nd\""),
        ] {
            assert_eq!(lua_value(v), want, "{v:?}");
        }
        assert_eq!(
            lua_set("decoration:shadow:color", "rgba(00000059)"),
            "hl.config({ [\"decoration.shadow.color\"] = \"rgba(00000059)\" })"
        );
    }

    /// Every theme: one Lua line per keyword (strip keys behind the plugin check) and the live
    /// `hyprctl eval` sets exactly the file's values.
    #[test]
    fn every_theme_renders_lua() {
        let cat = ThemeCatalog::builtin();
        for s in cat.list() {
            let t = cat.resolve(&s.id).unwrap();
            let file = lua(&t);
            let lines: Vec<&str> = file.lines().filter(|l| l.contains("hl.config(")).collect();
            assert_eq!(lines.len(), keywords(&t).len(), "{}", s.id);
            for (l, (k, _)) in lines.iter().zip(keywords(&t)) {
                assert_eq!(
                    l.starts_with("if strip then "),
                    k.starts_with("plugin:"),
                    "{}: {l}",
                    s.id
                );
            }
            let live: Vec<String> = lua_argv(&t).into_iter().map(|a| a[2].clone()).collect();
            let calls: Vec<String> = lines
                .iter()
                .map(|l| {
                    l.trim_start_matches("if strip then ")
                        .trim_end_matches(" end")
                        .to_string()
                })
                .collect();
            assert_eq!(live, calls, "{}", s.id);
            assert!(lua_argv(&t).iter().all(|a| a[..2] == ["hyprctl", "eval"]));
        }
    }

    /// The Lua buttons file adds the same three buttons, guarded until the plugin is loaded.
    #[test]
    fn the_lua_lights_file_adds_the_three_buttons() {
        let rel = LIGHTS_LUA.strip_prefix("/usr/share/wolos/").unwrap();
        let f = repo(&format!("{SHARE}{rel}"));
        let adds: Vec<&str> = f.lines().filter(|l| l.contains(".add_button(")).collect();
        assert_eq!(adds.len(), 3);
        for (l, action) in adds.iter().zip(["close", "zoom", "minimize"]) {
            assert!(l.contains(&format!("action = \"{action}\"")), "{l}");
        }
        assert!(f.contains("if bars then"));
        assert!(!f.contains("require("));
    }

    /// While both forms ship, the Lua files bind exactly what the `.conf` files bind: every test
    /// that reads the conf's binds (shortcuts sheet, app keys, the Mac keys) holds for the Lua.
    #[test]
    fn lua_binds_equal_the_conf_binds() {
        for (conf, lua) in [
            ("hyprland.conf", "hyprland.lua"),
            ("tiling.conf", "tiling.lua"),
        ] {
            let c = repo(&format!("{SHARE}hypr/{conf}"));
            let lowered = conf_binds(&repo(&format!("{SHARE}hypr/{lua}")));
            assert!(
                !lowered.contains("?"),
                "{lua}: a bind form conf_binds does not read:\n{lowered}"
            );
            let norm = |b: &crate::shortcuts::Bind| {
                let mut f: Vec<char> = b.flags.chars().collect();
                f.sort_unstable();
                (f, b.combo.label(), b.dispatcher.clone(), b.args.clone())
            };
            let want: Vec<_> = crate::shortcuts::parse(&c).iter().map(norm).collect();
            let got: Vec<_> = crate::shortcuts::parse(&lowered).iter().map(norm).collect();
            assert!(want.len() >= 13, "{conf}: {}", want.len());
            assert_eq!(got, want, "{lua} binds differ from {conf}");
            // The submap's own binds (not shortcuts) too.
            let sub = |t: &str| -> Vec<String> {
                let mut on = false;
                let mut v = Vec::new();
                for l in t.lines().map(str::trim) {
                    if let Some(n) = l.strip_prefix("submap = ") {
                        on = n != "reset";
                        v.push(l.to_string());
                    } else if on && l.starts_with("bind") {
                        v.push(l.replace(", ", ",").replace(" ,", ","));
                    }
                }
                v
            };
            assert_eq!(sub(&lowered), sub(&c), "{lua}: submaps");
        }
    }

    #[test]
    fn conf_binds_reads_the_lua_forms() {
        let lua = r#"
local mod = "SUPER"
local LOCKED = { locked = true }
bind(mod .. " + Space", "wol-search") -- a comment
bind("XF86AudioMute", "wol-volume mute", LOCKED)
hl.bind("CTRL + " .. mod .. " + F", hl.dsp.window.fullscreen({ mode = "fullscreen" }))
hl.bind(mod .. " + mouse:272", hl.dsp.window.drag(), { mouse = true })
hl.bind(mod .. " + CTRL + left", hl.dsp.window.resize({ x = -40, y = 0, relative = true }), { repeating = true })
hl.bind("CTRL + ALT + 1", hl.dsp.focus({ workspace = 1 }))
hl.bind(mod .. " + ALT + up", hl.dsp.focus({ direction = "up" }))
hl.bind(mod .. " + Q", hl.dsp.window.close())
hl.define_submap("rec", function()
    hl.bind("escape", hl.dsp.submap("reset"), { non_consuming = true })
end)
"#;
        assert_eq!(
            conf_binds(lua),
            "bind = SUPER, Space, exec, wol-search\n\
             bindl = , XF86AudioMute, exec, wol-volume mute\n\
             bind = CTRL SUPER, F, fullscreen, 0\n\
             bindm = SUPER, mouse:272, movewindow\n\
             binde = SUPER CTRL, left, resizeactive, -40 0\n\
             bind = CTRL ALT, 1, workspace, 1\n\
             bind = SUPER ALT, up, movefocus, u\n\
             bind = SUPER, Q, ?lua, hl.dsp.window.close()\n\
             submap = rec\n\
             bindn = , escape, submap, reset\n\
             submap = reset\n"
        );
    }

    #[test]
    fn colour_literals_are_found() {
        for (l, want) in [
            ("bar_color = rgba(2c2c2ee6)", true),
            ("    color = rgb(000000)", true),
            ("col.active_border = 0xff8b6cff", true),
            ("# rgba(00000000) in a comment", false),
            ("-- rgba(00000000) in a Lua comment", false),
            ("    background_color = \"rgb(000000)\",", true),
            ("gaps_in = 6", false),
            ("bind = $mod, 0, exec, wol-app actual-size", false),
        ] {
            assert_eq!(has_colour(l), want, "{l}");
        }
    }

    /// Every keyword is a hyprbars/shadow one with a valid colour, for every theme, and the title
    /// reads on the strip.
    #[test]
    fn every_theme_colours_the_strip_and_shadows() {
        let cat = ThemeCatalog::builtin();
        for s in cat.list() {
            let t = cat.resolve(&s.id).unwrap();
            for (k, v) in keywords(&t) {
                assert!(
                    k.starts_with("plugin:hyprbars:") || k.starts_with("decoration:shadow:"),
                    "{k}"
                );
                let hex = v.strip_prefix("rgba(").and_then(|v| v.strip_suffix(')'));
                assert!(
                    hex.is_some_and(|h| h.len() == 8 && h.chars().all(|c| c.is_ascii_hexdigit())),
                    "{}: {k} = {v}",
                    s.id
                );
            }
            let bar = token_color(&t, "surface");
            assert!(
                Color::contrast(token_color(&t, "text"), bar) >= 4.5,
                "{}: title on the strip",
                s.id
            );
        }
    }

    #[test]
    fn a_light_theme_gets_a_light_strip() {
        let t = ThemeCatalog::builtin().resolve("catppuccin-latte").unwrap();
        let k = keywords(&t);
        assert_eq!(k[0], ("plugin:hyprbars:bar_color", "rgba(eff1f5e6)".into()));
        assert_eq!(k[1], ("plugin:hyprbars:col.text", "rgba(4c4f69ff)".into()));
    }
}
