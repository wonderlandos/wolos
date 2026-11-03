// SPDX-License-Identifier: BSD-2-Clause
//! WolOS-managed compositor file (`hypr/wolos/settings.lua`): typed desktop prefs from System Settings
//! rendered as one Lua call per keyword (`hl.config`, `hl.window_rule`, `hl.env`; the keyword names
//! `hyprctl keyword` takes), plus the minimal command batch that applies a change live. Never touches the
//! user's own lines in hyprland.lua; never reloads.
use crate::prefs::{
    DISPLAYS_FILE, HYPR_USER, KEYBOARD_FILE, LAYOUTS, REPEATS, ensure_require, keyboard_lua,
    monitor_lua, monitor_rules, parse_keyboard,
};
use crate::shortcuts::{self, Overrides, Shortcut};
use crate::system::Host;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Settings' source of truth, relative to `$XDG_CONFIG_HOME`.
pub const PREFS_FILE: &str = "wolos/desktop.json";
/// Rendered file, loaded by the user hyprland.lua through `hypr/wolos/*.lua` (`prefs::ensure_require`).
pub const INCLUDE: &str = "hypr/wolos/settings.lua";
/// Named rules in the shipped hyprland.lua: everything floats; listed apps keep floating while tiling.
pub const FLOAT_RULE: &str = "wol-float";
pub const FLOAT_APPS_RULE: &str = "wol-float-apps";
/// Cursor theme the shipped session exports (`env = XCURSOR_THEME`).
pub const CURSOR_THEME: &str = "Adwaita";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    #[default]
    Dwindle,
    Master,
}

impl Layout {
    pub fn name(self) -> &'static str {
        match self {
            Layout::Dwindle => "dwindle",
            Layout::Master => "master",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopPrefs {
    /// Windows tile instead of floating (floating is the default).
    pub tiling: bool,
    pub layout: Layout,
    /// Effects & Looks (gaps, corners, blur, borders, shadows, motion). Flattened: its keys sit at the
    /// top level of desktop.json (older files' `gaps` keeps working).
    #[serde(flatten)]
    pub looks: crate::looks::Looks,
    /// Window classes that keep floating while tiling is on.
    pub float_apps: Vec<String>,
    /// xkb layouts, first = default: "us", "us(intl)", "br".
    pub layouts: Vec<String>,
    /// Keys per second.
    pub repeat_rate: u32,
    /// Milliseconds before repeat starts.
    pub repeat_delay: u32,
    /// Pointer speed, -1.0..=1.0 (0 = unchanged).
    pub pointer_speed: f32,
    pub mouse_natural_scroll: bool,
    pub touchpad_natural_scroll: bool,
    pub tap_to_click: bool,
    pub two_finger_secondary: bool,
    pub disable_while_typing: bool,
    pub reduce_motion: bool,
    pub reduce_transparency: bool,
    /// Cursor size in px (24 = normal).
    pub cursor_size: u32,
    /// Session language (`env = LANG`), applied at the next login.
    pub language: Option<String>,
    /// Commands started at login (`exec-once`).
    pub login_items: Vec<String>,
}

impl Default for DesktopPrefs {
    /// Same values as the shipped hyprland.lua: writing defaults changes nothing.
    fn default() -> Self {
        Self {
            tiling: false,
            layout: Layout::Dwindle,
            looks: crate::looks::Looks::default(),
            float_apps: Vec::new(),
            layouts: vec!["us".into()],
            repeat_rate: 25,
            repeat_delay: 400,
            pointer_speed: 0.0,
            mouse_natural_scroll: true,
            touchpad_natural_scroll: true,
            tap_to_click: true,
            two_finger_secondary: true,
            disable_while_typing: true,
            reduce_motion: false,
            reduce_transparency: false,
            cursor_size: 24,
            language: None,
            login_items: Vec::new(),
        }
    }
}

/// Window class as a literal regex alternative (dots escaped).
fn class_re(c: &str) -> String {
    c.chars()
        .flat_map(|ch| match ch {
            '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$' | '\\' => {
                vec!['\\', ch]
            }
            _ => vec![ch],
        })
        .collect()
}

/// Class names Settings accepts for exceptions (no spaces, commas or control characters).
pub fn is_valid_class(c: &str) -> bool {
    !c.is_empty()
        && c.len() <= 128
        && c.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "._-".contains(ch))
}

/// One login item: a command line without newlines or `;` (hyprctl batch separator).
pub fn is_valid_command(c: &str) -> bool {
    let c = c.trim();
    !c.is_empty() && c.len() <= 256 && !c.chars().any(|ch| ch.is_control() || ch == ';')
}

fn b(v: bool) -> String {
    if v { "true" } else { "false" }.into()
}

/// (xkb layout list, variant list): "us(intl)" -> layout "us", variant "intl".
fn xkb(layouts: &[String]) -> (String, String) {
    let (mut l, mut v) = (Vec::new(), Vec::new());
    for x in layouts {
        match x.split_once('(') {
            Some((a, var)) => {
                l.push(a.to_string());
                v.push(var.trim_end_matches(')').to_string());
            }
            None => {
                l.push(x.clone());
                v.push(String::new());
            }
        }
    }
    (l.join(","), v.join(","))
}

impl DesktopPrefs {
    /// Missing or broken file = defaults; unknown keys ignored.
    pub fn parse(text: Option<&str>) -> Self {
        text.and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// `desktop.json`, else (first run) the setup assistant's `hypr/wolos/keyboard.lua` carried over.
    pub fn from_files(desktop_json: Option<&str>, keyboard: Option<&str>) -> Self {
        match desktop_json {
            Some(t) => Self::parse(Some(t)),
            None => Self::from_legacy_keyboard(keyboard),
        }
    }

    /// First run: carry over the setup assistant's `hypr/wolos/keyboard.lua` (layout, repeat).
    pub fn from_legacy_keyboard(keyboard_conf: Option<&str>) -> Self {
        let mut p = Self::default();
        if let Some(t) = keyboard_conf {
            let (l, r) = parse_keyboard(Some(t));
            p.layouts = vec![LAYOUTS[l].0.to_string()];
            let (_, rate, delay) = REPEATS[r];
            (p.repeat_rate, p.repeat_delay) = (rate, delay);
        }
        p
    }

    /// Every managed keyword in apply order (a rule's match before its enable).
    pub fn keywords(&self) -> Vec<(String, String)> {
        let (kb, variant) = xkb(&self.layouts);
        let apps = self.tiling && !self.float_apps.is_empty();
        let apps_re = if self.float_apps.is_empty() {
            "^$".to_string()
        } else {
            let alts: Vec<String> = self.float_apps.iter().map(|c| class_re(c)).collect();
            format!("^({})$", alts.join("|"))
        };
        let kv = |k: &str, v: String| (k.to_string(), v);
        vec![
            kv(&format!("windowrule[{FLOAT_RULE}]:enable"), b(!self.tiling)),
            kv(
                &format!("windowrule[{FLOAT_APPS_RULE}]:match:class"),
                apps_re,
            ),
            kv(&format!("windowrule[{FLOAT_APPS_RULE}]:enable"), b(apps)),
            kv("general:layout", self.layout.name().into()),
            kv("input:kb_layout", kb),
            kv("input:kb_variant", variant),
            kv(
                "input:kb_options",
                if self.layouts.len() > 1 {
                    "grp:ctrl_space_toggle".into()
                } else {
                    String::new()
                },
            ),
            kv("input:repeat_rate", self.repeat_rate.to_string()),
            kv("input:repeat_delay", self.repeat_delay.to_string()),
            kv(
                "input:sensitivity",
                format!("{:.2}", self.pointer_speed.clamp(-1.0, 1.0)),
            ),
            kv("input:natural_scroll", b(self.mouse_natural_scroll)),
            kv(
                "input:touchpad:natural_scroll",
                b(self.touchpad_natural_scroll),
            ),
            kv("input:touchpad:tap-to-click", b(self.tap_to_click)),
            kv(
                "input:touchpad:clickfinger_behavior",
                b(self.two_finger_secondary),
            ),
            kv(
                "input:touchpad:disable_while_typing",
                b(self.disable_while_typing),
            ),
        ]
        .into_iter()
        .chain(
            self.looks
                .keywords(self.reduce_motion, self.reduce_transparency),
        )
        .collect()
    }

    /// The Lua file: keywords, then session env and login items. A named rule's keywords
    /// (`windowrule[NAME]:enable`, `:match:class`) become one `hl.window_rule` call that updates the
    /// shipped rule of that name; every other keyword is `looks::keyword_lua`.
    pub fn include(&self) -> String {
        use crate::theme::hypr::lua_str;
        let mut s = String::from(
            "-- Written by WolOS System Settings. Change these there; your own lines belong in hyprland.lua.\n",
        );
        let mut rules: Vec<(String, Option<String>, Option<String>)> = Vec::new();
        let mut rest = Vec::new();
        for (k, v) in self.keywords() {
            let Some((name, field)) = k
                .strip_prefix("windowrule[")
                .and_then(|r| r.split_once("]:"))
            else {
                rest.push((k, v));
                continue;
            };
            let at = match rules.iter().position(|r| r.0 == name) {
                Some(at) => at,
                None => {
                    rules.push((name.to_string(), None, None));
                    rules.len() - 1
                }
            };
            match field {
                "enable" => rules[at].1 = Some(v),
                "match:class" => rules[at].2 = Some(v),
                _ => rest.push((k, v)),
            }
        }
        for (name, enabled, class) in rules {
            let mut call = format!("hl.window_rule({{ name = {}", lua_str(&name));
            if let Some(e) = enabled {
                call += &format!(", enabled = {e}");
            }
            if let Some(c) = class {
                call += &format!(", match = {{ class = {} }}", lua_str(&c));
            }
            s += &call;
            s += " })\n";
        }
        for (k, v) in rest {
            s += &crate::looks::keyword_lua(&k, &v);
            s.push('\n');
        }
        let env = |k: &str, v: &str| format!("hl.env({}, {})\n", lua_str(k), lua_str(v));
        let size = self.cursor_size.to_string();
        s += &env("XCURSOR_SIZE", &size);
        // The compositor's own pointer (hyprcursor) and the apps' (Xcursor) agree on theme and size.
        s += &env("HYPRCURSOR_THEME", CURSOR_THEME);
        s += &env("HYPRCURSOR_SIZE", &size);
        if let Some(l) = self
            .language
            .as_deref()
            .filter(|l| crate::region::is_valid_locale(l))
        {
            s += &env("LANG", l);
            s += &env("LC_ALL", l);
        }
        for c in self.login_items.iter().filter(|c| is_valid_command(c)) {
            s += &format!(
                "hl.on(\"hyprland.start\", function() hl.exec_cmd({}) end)\n",
                lua_str(c.trim())
            );
        }
        s
    }

    /// hyprctl batch commands that move the running session from `old` to `self`: only changed keywords.
    pub fn live(&self, old: &Self) -> Vec<String> {
        let before = old.keywords();
        let mut out: Vec<String> = self
            .keywords()
            .into_iter()
            .filter(|kv| !before.contains(kv))
            .map(|(k, v)| format!("keyword {k} {v}"))
            .collect();
        if self.cursor_size != old.cursor_size {
            out.push(format!("setcursor {CURSOR_THEME} {}", self.cursor_size));
        }
        out
    }
}

/// The session step that writes the X resource database (Xft.dpi, Xcursor.*) and the X11 scale
/// variables from the monitor scale (`hl.exec_cmd` in hyprland.lua; desktop/overlay/usr/bin).
pub const X11_ENV: &str = "wol-x11-env";

/// Commands that move every toolkit's pointer to `size` at once; `setcursor` changes only the
/// compositor's. GTK and libadwaita read gsettings, Xwayland apps the X resource database
/// ([`X11_ENV`] multiplies the size by the monitor scale: Xwayland renders 1:1).
pub fn cursor_sync(size: u32) -> Vec<Vec<String>> {
    let argv = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
    let px = size.to_string();
    vec![
        argv(&[
            "gsettings",
            "set",
            "org.gnome.desktop.interface",
            "cursor-size",
            px.as_str(),
        ]),
        argv(&[
            "gsettings",
            "set",
            "org.gnome.desktop.interface",
            "cursor-theme",
            CURSOR_THEME,
        ]),
        argv(&[X11_ENV, "--cursor-size", px.as_str()]),
    ]
}

/// `hyprctl -j clients` -> dispatches that tile (or float) the windows already open, so a tiling switch
/// takes effect at once. Special workspaces (minimised) and floating exceptions are left alone.
pub fn retile(clients_json: &str, tile: bool, float_apps: &[String]) -> Vec<String> {
    let Ok(serde_json::Value::Array(a)) = serde_json::from_str::<serde_json::Value>(clients_json)
    else {
        return Vec::new();
    };
    a.iter()
        .filter_map(|c| {
            let addr = c.get("address")?.as_str()?;
            let floating = c.get("floating")?.as_bool()?;
            let ws = c.get("workspace")?.get("id")?.as_i64()?;
            let class = c.get("class").and_then(|x| x.as_str()).unwrap_or("");
            let keep = float_apps.iter().any(|f| f == class);
            let change = ws > 0 && floating == tile && !(tile && keep);
            change.then(|| {
                format!(
                    "dispatch {} address:{addr}",
                    if tile { "settiled" } else { "setfloating" }
                )
            })
        })
        .collect()
}

// ---- Applying (System Settings) ----

/// The setup assistant's keyboard file, folded into the include on the first save.
const LEGACY_KEYBOARD: &str = KEYBOARD_FILE;

fn read_in(home: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(home.join(rel)).ok()
}

/// Saved desktop prefs under `home` (`$XDG_CONFIG_HOME`); first run carries over the older keyboard fragment.
pub fn load_desktop(home: &Path) -> DesktopPrefs {
    DesktopPrefs::from_files(
        read_in(home, PREFS_FILE).as_deref(),
        read_in(home, LEGACY_KEYBOARD).as_deref(),
    )
}

pub fn load_overrides(home: &Path) -> Overrides {
    Overrides::parse(read_in(home, shortcuts::PREFS_FILE).as_deref())
}

/// Shipped shortcuts: the installed Lua files, else the copies built in.
pub fn shortcut_defaults(host: &dyn Host) -> Vec<Shortcut> {
    shortcuts::load_defaults(|p| host.read_text(p))
}

/// Writes prefs, both Lua files and the user hyprland.lua's `require` of them: one journal entry
/// (`wol undo`, Settings > Restore).
fn persist(home: &Path, d: &DesktopPrefs, o: &Overrides, list: &[Shortcut]) -> std::io::Result<()> {
    let user = HYPR_USER;
    let source = ensure_require(read_in(home, user).as_deref());
    let texts = [
        (PREFS_FILE, Some(d.to_json())),
        (INCLUDE, Some(d.include())),
        (shortcuts::PREFS_FILE, Some(o.to_json())),
        (shortcuts::INCLUDE, Some(shortcuts::include(list, d.tiling))),
        (user, source),
    ];
    let files: Vec<(&str, Option<&str>)> = texts
        .iter()
        .filter_map(|(rel, t)| Some((*rel, Some(t.as_deref()?))))
        .collect();
    crate::journal::Journal::for_config(home).save(
        "desktop",
        "settings",
        &files,
        crate::journal::now(),
    )?;
    let _ = std::fs::remove_file(home.join(LEGACY_KEYBOARD));
    Ok(())
}

/// The WolOS files of the compositor 0.56 series (WolOS 0.12 and before) under `$XDG_CONFIG_HOME`,
/// the last one the theme render in the toolkit dir (`theme::apps::user_dir`). The 0.57 series reads
/// only Lua and never these; [`retire_plan`] carries them over once.
pub const STALE_CONF: [&str; 6] = [
    "hypr/wolos/settings.conf",
    "hypr/wolos/shortcuts.conf",
    "hypr/wolos/theme.conf",
    "hypr/wolos/keyboard.conf",
    "hypr/wolos/displays.conf",
    "wolos/toolkits/hypr/wolos/theme.conf",
];

/// What [`retire_plan`] found, relative to `$XDG_CONFIG_HOME`: Lua files to write, then stale
/// files to remove. Empty when nothing is stale.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Retire {
    pub write: Vec<(&'static str, String)>,
    pub remove: Vec<&'static str>,
}

/// First run after the upgrade to the Lua configuration: the person's keyboard and display choices
/// in [`STALE_CONF`] move to their Lua files, the Settings and shortcut files are rendered again
/// from `desktop.json` and `shortcuts.json` (the source of truth), and the stale files go. A Lua
/// file that already exists wins. `read` and `present` look under `$XDG_CONFIG_HOME` (`present`
/// counts a dangling link), `shipped` reads the installed defaults ([`shortcut_defaults`]). Pure:
/// the caller writes, removes and reloads the compositor (`wol theme set`, at every session start).
pub fn retire_plan(
    read: &dyn Fn(&str) -> Option<String>,
    present: &dyn Fn(&str) -> bool,
    shipped: &dyn Fn(&str) -> Option<String>,
) -> Retire {
    let remove: Vec<&'static str> = STALE_CONF.into_iter().filter(|r| present(r)).collect();
    let mut write: Vec<(&'static str, String)> = Vec::new();
    if remove.is_empty() {
        return Retire::default();
    }
    if let Some(t) = read(STALE_CONF[3]).filter(|_| !present(KEYBOARD_FILE)) {
        let (l, r) = parse_keyboard(Some(&t));
        write.extend(keyboard_lua(LAYOUTS[l].0, r).map(|lua| (KEYBOARD_FILE, lua)));
    }
    if let Some(t) = read(STALE_CONF[4]).filter(|_| !present(DISPLAYS_FILE)) {
        let rules = monitor_rules(&t);
        if !rules.is_empty() {
            let mut lua = String::from("-- Written by WolOS Settings.\n");
            for r in rules {
                lua += &monitor_lua(&r);
                lua.push('\n');
            }
            write.push((DISPLAYS_FILE, lua));
        }
    }
    let keyboard = write
        .iter()
        .find(|w| w.0 == KEYBOARD_FILE)
        .map(|w| w.1.clone())
        .or_else(|| read(LEGACY_KEYBOARD));
    let d = DesktopPrefs::from_files(read(PREFS_FILE).as_deref(), keyboard.as_deref());
    if read(PREFS_FILE).is_some() && !present(INCLUDE) {
        write.push((INCLUDE, d.include()));
    }
    if let Some(o) = read(shortcuts::PREFS_FILE).filter(|_| !present(shortcuts::INCLUDE)) {
        let list = shortcuts::effective(
            &shortcuts::load_defaults(shipped),
            &Overrides::parse(Some(&o)),
        );
        write.push((shortcuts::INCLUDE, shortcuts::include(&list, d.tiling)));
    }
    Retire { write, remove }
}

/// One apply at a time: each reads the saved state, diffs and writes it back.
static APPLYING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Submap with no binds of its own (shipped hyprland.lua): while Settings records a shortcut, WolOS
/// shortcuts pause so the keys reach it. Esc (non-consuming bind) always leaves it.
pub const RECORD_SUBMAP: &str = "wol-record";

/// Pauses (true) or resumes global shortcuts around a shortcut recording.
pub fn capture(host: &dyn Host, on: bool) -> bool {
    let to = if on { RECORD_SUBMAP } else { "reset" };
    host.run(&["hyprctl", "dispatch", "submap", to])
        .is_some_and(|o| o.trim() == "ok")
}

/// One `hyprctl --batch` for the running session (none when nothing changed). Best effort: without a
/// running compositor the files still apply at the next login.
fn send(host: &dyn Host, cmds: &[String]) {
    if !cmds.is_empty() {
        let _ = host.run(&["hyprctl", "--batch", &cmds.join(" ; ")]);
    }
}

/// Saves `new` and moves the session to it: changed keywords only, open windows re-tiled or floated on a
/// tiling switch, tiling shortcuts bound or unbound with it. Returns the commands sent.
pub fn apply_desktop(
    host: &dyn Host,
    home: &Path,
    new: &DesktopPrefs,
) -> std::io::Result<Vec<String>> {
    let _one = APPLYING.lock().unwrap_or_else(|e| e.into_inner());
    let old = load_desktop(home);
    let o = load_overrides(home);
    let list = shortcuts::effective(&shortcut_defaults(host), &o);
    persist(home, new, &o, &list)?;
    let mut cmds = new.live(&old);
    if new.tiling != old.tiling {
        let clients = host.run(&["hyprctl", "-j", "clients"]).unwrap_or_default();
        cmds.extend(retile(&clients, new.tiling, &new.float_apps));
        cmds.extend(shortcuts::live(
            &shortcuts::active(&list, old.tiling),
            &shortcuts::active(&list, new.tiling),
        ));
    }
    send(host, &cmds);
    if new.cursor_size != old.cursor_size {
        for c in cursor_sync(new.cursor_size) {
            let argv: Vec<&str> = c.iter().map(String::as_str).collect();
            let _ = host.run(&argv);
        }
    }
    Ok(cmds)
}

/// Saves shortcut overrides and rebinds the changed ones live. Returns the commands sent.
pub fn apply_shortcuts(
    host: &dyn Host,
    home: &Path,
    new: &Overrides,
) -> std::io::Result<Vec<String>> {
    let _one = APPLYING.lock().unwrap_or_else(|e| e.into_inner());
    let d = load_desktop(home);
    let defaults = shortcut_defaults(host);
    let old = shortcuts::effective(&defaults, &load_overrides(home));
    let list = shortcuts::effective(&defaults, new);
    persist(home, &d, new, &list)?;
    let cmds = shortcuts::live(
        &shortcuts::active(&old, d.tiling),
        &shortcuts::active(&list, d.tiling),
    );
    send(host, &cmds);
    Ok(cmds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

    fn kw(p: &DesktopPrefs, k: &str) -> String {
        p.keywords()
            .into_iter()
            .find(|(x, _)| x == k)
            .map(|(_, v)| v)
            .unwrap_or_else(|| panic!("{k}"))
    }

    /// Tiling on/off -> the shipped float rule, the exceptions rule, layout and gaps.
    #[test]
    fn tiling_config_table() {
        let off = DesktopPrefs::default();
        let on = DesktopPrefs {
            tiling: true,
            ..Default::default()
        };
        let on_master = DesktopPrefs {
            tiling: true,
            layout: Layout::Master,
            looks: crate::looks::Looks {
                gaps: 4,
                ..Default::default()
            },
            float_apps: vec!["org.gnome.Calculator".into(), "pavucontrol".into()],
            ..Default::default()
        };
        let off_with_apps = DesktopPrefs {
            float_apps: vec!["pavucontrol".into()],
            ..Default::default()
        };
        let cases: [(&DesktopPrefs, [&str; 6]); 4] = [
            (&off, ["true", "^$", "false", "dwindle", "6", "12"]),
            (&on, ["false", "^$", "false", "dwindle", "6", "12"]),
            (
                &on_master,
                [
                    "false",
                    r"^(org\.gnome\.Calculator|pavucontrol)$",
                    "true",
                    "master",
                    "2",
                    "4",
                ],
            ),
            (
                &off_with_apps,
                ["true", "^(pavucontrol)$", "false", "dwindle", "6", "12"],
            ),
        ];
        for (p, want) in cases {
            let got = [
                kw(p, "windowrule[wol-float]:enable"),
                kw(p, "windowrule[wol-float-apps]:match:class"),
                kw(p, "windowrule[wol-float-apps]:enable"),
                kw(p, "general:layout"),
                kw(p, "general:gaps_in"),
                kw(p, "general:gaps_out"),
            ];
            assert_eq!(got, want.map(String::from), "{p:?}");
            let inc = p.include();
            let rule = format!(
                "hl.window_rule({{ name = \"wol-float\", enabled = {} }})\n",
                want[0]
            );
            assert!(inc.contains(&rule), "{inc}");
            let apps = format!(
                "hl.window_rule({{ name = \"wol-float-apps\", enabled = {}, match = {{ class = {} }} }})\n",
                want[2],
                crate::theme::hypr::lua_str(want[1])
            );
            assert!(inc.contains(&apps), "{inc}");
        }
    }

    #[test]
    fn live_sets_only_changed_keywords() {
        let a = DesktopPrefs::default();
        assert!(a.live(&a).is_empty(), "no change, no command");
        let t = DesktopPrefs {
            tiling: true,
            ..a.clone()
        };
        assert_eq!(t.live(&a), ["keyword windowrule[wol-float]:enable false"]);
        let r = DesktopPrefs {
            repeat_rate: 40,
            cursor_size: 32,
            ..a.clone()
        };
        assert_eq!(
            r.live(&a),
            ["keyword input:repeat_rate 40", "setcursor Adwaita 32"]
        );
        let apps = DesktopPrefs {
            tiling: true,
            float_apps: vec!["mpv".into()],
            ..a.clone()
        };
        let cmds = apps.live(&t);
        assert_eq!(
            cmds,
            [
                "keyword windowrule[wol-float-apps]:match:class ^(mpv)$",
                "keyword windowrule[wol-float-apps]:enable true"
            ],
            "match before enable"
        );
    }

    /// Looks live in desktop.json at the top level (older files' `gaps` still read) and apply with
    /// the same batch: one changed look, one keyword.
    #[test]
    fn looks_are_flattened_and_applied_by_difference() {
        let old = DesktopPrefs::parse(Some(r#"{"gaps":20,"tiling":true}"#));
        assert_eq!((old.looks.gaps, old.tiling), (20, true));
        let json = DesktopPrefs::default().to_json();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            v.get("rounding").is_some() && v.get("looks").is_none(),
            "{json}"
        );
        let mut round = old.clone();
        round.looks.rounding = 4;
        assert_eq!(round.live(&old), ["keyword decoration:rounding 4"]);
        let mut slow = old.clone();
        slow.looks.speed = 0.5;
        let cmds = slow.live(&old);
        assert_eq!(cmds.len(), 7, "every animation line: {cmds:?}");
        assert!(cmds[0].starts_with("keyword animation windowsIn,1,4.4,"));
        let calm = DesktopPrefs {
            reduce_transparency: true,
            ..old.clone()
        };
        assert!(
            calm.live(&old)
                .contains(&"keyword decoration:blur:enabled false".to_string())
        );
        assert_eq!(DesktopPrefs::parse(Some(&slow.to_json())), slow);
    }

    #[test]
    fn keyboard_layouts_and_legacy_migration() {
        let p = DesktopPrefs {
            layouts: vec!["us".into(), "us(intl)".into(), "br".into()],
            ..Default::default()
        };
        assert_eq!(kw(&p, "input:kb_layout"), "us,us,br");
        assert_eq!(kw(&p, "input:kb_variant"), ",intl,");
        assert_eq!(kw(&p, "input:kb_options"), "grp:ctrl_space_toggle");
        assert_eq!(kw(&DesktopPrefs::default(), "input:kb_options"), "");
        let legacy = crate::prefs::keyboard_lua("br", 2).unwrap();
        let m = DesktopPrefs::from_legacy_keyboard(Some(&legacy));
        assert_eq!(
            (m.layouts, m.repeat_rate, m.repeat_delay),
            (vec!["br".to_string()], 40, 250)
        );
        assert_eq!(
            DesktopPrefs::from_legacy_keyboard(None),
            DesktopPrefs::default()
        );
    }

    #[test]
    fn include_env_login_items_and_json_round_trip() {
        let p = DesktopPrefs {
            reduce_motion: true,
            language: Some("pt_BR.UTF-8".into()),
            login_items: vec!["foot".into(), "evil; rm -rf ~".into(), "  ".into()],
            ..Default::default()
        };
        let inc = p.include();
        for line in [
            "hl.config({ [\"animations.enabled\"] = false })\n",
            "hl.config({ [\"input.touchpad.tap_to_click\"] = true })\n",
            "hl.config({ [\"input.kb_variant\"] = \"\" })\n",
            "hl.env(\"XCURSOR_SIZE\", \"24\")\n",
            "hl.env(\"HYPRCURSOR_THEME\", \"Adwaita\")\n",
            "hl.env(\"HYPRCURSOR_SIZE\", \"24\")\n",
            "hl.env(\"LANG\", \"pt_BR.UTF-8\")\n",
            "hl.on(\"hyprland.start\", function() hl.exec_cmd(\"foot\") end)\n",
        ] {
            assert!(inc.contains(line), "{line}");
        }
        assert!(!inc.contains("rm -rf"), "invalid login items are dropped");
        assert!(
            !DesktopPrefs {
                language: Some("x; y".into()),
                ..Default::default()
            }
            .include()
            .contains("LANG")
        );
        assert_eq!(DesktopPrefs::parse(Some(&p.to_json())), p);
        assert_eq!(DesktopPrefs::parse(Some("junk")), DesktopPrefs::default());
        assert!(DesktopPrefs::parse(Some(r#"{"tiling":true}"#)).tap_to_click);
    }

    #[test]
    fn validation() {
        for (c, ok) in [
            ("org.gnome.Calculator", true),
            ("mpv", true),
            ("a b", false),
            ("x,y", false),
            ("", false),
        ] {
            assert_eq!(is_valid_class(c), ok, "{c}");
        }
        assert!(is_valid_command("foot --server"));
        assert!(!is_valid_command("a\nb"));
        assert!(!is_valid_command("a ; b"));
    }

    /// `WOL_HYPR_GEN=<dir> cargo test -p core write_generated_lua -- --ignored`: every Lua file WolOS
    /// writes under `$XDG_CONFIG_HOME/hypr` (the user hyprland.lua, Settings, shortcuts with the
    /// tiling set and custom keys, the setup keyboard, displays, each theme) into `<dir>/hypr`, for
    /// `Hyprland --verify-config -c <dir>/hypr/hyprland.lua` with the shipped files installed (the
    /// compositor-config CI job's setup). `<dir>/hypr/themes/` holds one file per theme.
    #[test]
    #[ignore]
    fn write_generated_lua() {
        use crate::shortcuts as sc;
        let dir = std::path::PathBuf::from(std::env::var("WOL_HYPR_GEN").expect("WOL_HYPR_GEN"));
        let p = DesktopPrefs {
            tiling: true,
            layout: Layout::Master,
            float_apps: vec!["org.gnome.Calculator".into()],
            layouts: vec!["us".into(), "br".into()],
            reduce_motion: true,
            language: Some("pt_BR.UTF-8".into()),
            login_items: vec!["foot --server".into()],
            looks: crate::looks::Looks {
                gaps: 4,
                rounding: 6,
                ..Default::default()
            },
            ..Default::default()
        };
        let d = sc::defaults(sc::SHIPPED, sc::TILING);
        let mut o = sc::Overrides::default();
        o.assign(
            &d,
            "exec wol-search",
            sc::Combo::parse("SUPER ALT", "Space"),
            false,
        )
        .unwrap();
        o.assign(&d, "exec wol-settings", None, false).unwrap();
        o.add_custom(
            &d,
            sc::Custom {
                name: "Top".into(),
                combo: sc::Combo::parse("SUPER CTRL", "Y").unwrap(),
                command: "foot -e top".into(),
            },
        )
        .unwrap();
        let write = |rel: &str, text: &str| {
            let f = dir.join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        };
        write(
            crate::prefs::HYPR_USER,
            &crate::prefs::ensure_require(None).unwrap(),
        );
        write(INCLUDE, &p.include());
        write(sc::INCLUDE, &sc::include(&sc::effective(&d, &o), true));
        write(
            crate::prefs::KEYBOARD_FILE,
            &crate::prefs::keyboard_lua("us(intl)", 2).unwrap(),
        );
        let two = crate::prefs::displays_lua(None, "Virtual-1,1920x1080@60.00,auto,1");
        write(
            crate::prefs::DISPLAYS_FILE,
            &crate::prefs::displays_lua(
                Some(&two),
                "HDMI-A-1,1920x1080@60.00,auto,1,mirror,Virtual-1",
            ),
        );
        let cat = crate::theme::ThemeCatalog::builtin();
        let t = cat.resolve("wol-dark").unwrap();
        write(
            crate::theme::hypr::HYPR_THEME_FILE,
            &crate::theme::hypr::lua(&t),
        );
        for s in cat.list() {
            let t = cat.resolve(&s.id).unwrap();
            write(
                &format!("hypr/themes/{}.lua", s.id),
                &crate::theme::hypr::lua(&t),
            );
        }
    }

    #[test]
    fn retile_flips_open_windows() {
        let j = r#"[
            {"address":"0x1","class":"foot","floating":true,"workspace":{"id":1}},
            {"address":"0x2","class":"mpv","floating":true,"workspace":{"id":1}},
            {"address":"0x3","class":"foot","floating":false,"workspace":{"id":2}},
            {"address":"0x4","class":"foot","floating":true,"workspace":{"id":-98}}
        ]"#;
        assert_eq!(
            retile(j, true, &["mpv".into()]),
            ["dispatch settiled address:0x1"]
        );
        assert_eq!(retile(j, false, &[]), ["dispatch setfloating address:0x3"]);
        assert!(retile("nope", true, &[]).is_empty());
    }

    fn home(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wol-hyprprefs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn batch(host: &FakeHost) -> Vec<String> {
        host.ran()
            .iter()
            .filter_map(|l| l.strip_prefix("hyprctl --batch "))
            .flat_map(|b| b.split(" ; ").map(str::to_owned).collect::<Vec<_>>())
            .collect()
    }

    /// Live: Settings > Accessibility > Cursor size changed the compositor's pointer (`setcursor`)
    /// while GTK apps and Xwayland kept the old size until the next login.
    #[test]
    fn cursor_size_reaches_every_toolkit() {
        let cmds = cursor_sync(32);
        assert_eq!(
            cmds[0],
            [
                "gsettings",
                "set",
                "org.gnome.desktop.interface",
                "cursor-size",
                "32"
            ]
        );
        assert_eq!(cmds[1][4], "Adwaita");
        assert_eq!(cmds[2], ["wol-x11-env", "--cursor-size", "32"]);
        let h = home("cursor");
        let host = FakeHost::new(true);
        let old = load_desktop(&h);
        apply_desktop(
            &host,
            &h,
            &DesktopPrefs {
                cursor_size: 32,
                ..old.clone()
            },
        )
        .unwrap();
        let ran = host.ran();
        assert!(
            ran.iter()
                .any(|l| l == "gsettings set org.gnome.desktop.interface cursor-size 32"),
            "{ran:?}"
        );
        assert!(
            ran.iter().any(|l| l == "wol-x11-env --cursor-size 32"),
            "{ran:?}"
        );
        assert!(batch(&host).contains(&"setcursor Adwaita 32".to_string()));
        apply_desktop(&host, &h, &old).unwrap(); // back to 24
        let host = FakeHost::new(true);
        apply_desktop(&host, &h, &old).unwrap();
        assert!(
            !host.ran().iter().any(|l| l.starts_with("gsettings")),
            "same size: no toolkit commands"
        );
    }

    #[test]
    fn desktop_changes_are_journaled_and_undoable() {
        let h = home("journal");
        let old = load_desktop(&h);
        let new = DesktopPrefs {
            tiling: !old.tiling,
            ..old.clone()
        };
        apply_desktop(&FakeHost::new(true), &h, &new).unwrap();
        let j = crate::journal::Journal::for_config(&h);
        let e = j.entries();
        assert_eq!(
            (e.len(), e[0].what.as_str(), e[0].by.as_str()),
            (1, "desktop", "settings")
        );
        let undone = j.undo(None, "cli", crate::journal::now()).unwrap();
        assert!(crate::journal::reload_for(&undone).hyprland);
        assert_eq!(load_desktop(&h), old);
        assert!(!h.join(INCLUDE).exists(), "back to no include at all");
        let _ = std::fs::remove_dir_all(h);
    }

    #[test]
    fn apply_desktop_writes_include_and_sends_only_the_change() {
        let h = home("desktop");
        std::fs::create_dir_all(h.join("hypr/wolos")).unwrap();
        std::fs::write(
            h.join(crate::prefs::KEYBOARD_FILE),
            crate::prefs::keyboard_lua("br", 2).unwrap(),
        )
        .unwrap();
        let host = FakeHost::new(true);
        host.command(
            "hyprctl -j clients",
            r#"[{"address":"0x1","class":"foot","floating":true,"workspace":{"id":1}}]"#,
        );
        // First load carries over the setup assistant's keyboard file.
        let old = load_desktop(&h);
        assert_eq!(old.layouts, ["br"]);
        let new = DesktopPrefs {
            tiling: true,
            ..old.clone()
        };
        apply_desktop(&host, &h, &new).unwrap();
        assert_eq!(load_desktop(&h), new);
        let inc = std::fs::read_to_string(h.join(INCLUDE)).unwrap();
        assert!(inc.contains("hl.window_rule({ name = \"wol-float\", enabled = false })\n"));
        assert!(inc.contains("hl.config({ [\"input.kb_layout\"] = \"br\" })\n"));
        assert!(!h.join(crate::prefs::KEYBOARD_FILE).exists(), "migrated");
        let user = std::fs::read_to_string(h.join("hypr/hyprland.lua")).unwrap();
        assert_eq!(user, crate::prefs::user_lua("/usr/share/wolos"));
        let sent = batch(&host);
        assert!(sent.contains(&"keyword windowrule[wol-float]:enable false".to_string()));
        assert!(sent.contains(&"dispatch settiled address:0x1".to_string()));
        // Tiling binds come with tiling, and the shortcuts include follows.
        assert!(
            sent.iter()
                .any(|c| c.starts_with("keyword bind SUPER ALT,left,movefocus")),
            "{sent:?}"
        );
        let sc = std::fs::read_to_string(h.join(shortcuts::INCLUDE)).unwrap();
        assert!(
            sc.contains(
                "hl.bind(\"SUPER + ALT + left\", hl.dsp.focus({ direction = \"left\" }))\n"
            ),
            "{sc}"
        );
        assert!(
            !sent.iter().any(|c| c.contains("kb_layout")),
            "unchanged keys stay"
        );

        // Off again: windows float, tiling binds go.
        let host = FakeHost::new(true);
        host.command(
            "hyprctl -j clients",
            r#"[{"address":"0x1","class":"foot","floating":false,"workspace":{"id":1}}]"#,
        );
        apply_desktop(
            &host,
            &h,
            &DesktopPrefs {
                tiling: false,
                ..new
            },
        )
        .unwrap();
        let sent = batch(&host);
        assert!(sent.contains(&"dispatch setfloating address:0x1".to_string()));
        assert!(sent.contains(&"keyword unbind SUPER ALT,left".to_string()));
        // Nothing changed: nothing sent.
        let host = FakeHost::new(true);
        apply_desktop(&host, &h, &load_desktop(&h)).unwrap();
        assert!(host.ran().iter().all(|l| !l.starts_with("hyprctl --batch")));
        let _ = std::fs::remove_dir_all(h);
    }

    #[test]
    fn capture_pauses_and_resumes_shortcuts() {
        let host = FakeHost::new(true);
        host.command("hyprctl dispatch submap wol-record", "ok")
            .command("hyprctl dispatch submap reset", "ok");
        assert!(capture(&host, true));
        assert!(capture(&host, false));
        assert_eq!(
            host.ran(),
            [
                "hyprctl dispatch submap wol-record",
                "hyprctl dispatch submap reset"
            ]
        );
        assert!(!capture(&FakeHost::new(true), true), "no compositor");
        let shipped = crate::theme::hypr::conf_binds(shortcuts::SHIPPED);
        let block =
            format!("submap = {RECORD_SUBMAP}\nbindn = , escape, submap, reset\nsubmap = reset\n");
        assert!(
            shipped.contains(&block),
            "shipped hyprland.lua defines the record submap"
        );
    }

    #[test]
    fn apply_shortcuts_rebinds_live_and_persists() {
        let h = home("shortcuts");
        let host = FakeHost::new(true);
        let defaults = shortcut_defaults(&host);
        let shot = defaults
            .iter()
            .find(|s| s.name == "Show Search")
            .expect("searchbar shortcut")
            .clone();
        let mut o = load_overrides(&h);
        let k = shortcuts::Combo::parse("SUPER SHIFT", "K").unwrap();
        o.assign(&defaults, &shot.id, Some(k), false).unwrap();
        apply_shortcuts(&host, &h, &o).unwrap();
        assert_eq!(load_overrides(&h), o);
        let d = shot.default.clone().unwrap();
        let sent = batch(&host);
        assert_eq!(
            sent.first(),
            Some(&format!("keyword unbind {},{}", d.hypr_mods(), d.key))
        );
        assert!(
            sent.iter()
                .any(|c| c.starts_with("keyword bind SUPER SHIFT,K,")),
            "{sent:?}"
        );
        let sc = std::fs::read_to_string(h.join(shortcuts::INCLUDE)).unwrap();
        assert!(sc.contains("hl.bind(\"SUPER + SHIFT + K\", "), "{sc}");
        let _ = std::fs::remove_dir_all(h);
    }

    /// An upgrade from the 0.56 configuration: the person's keyboard and displays move to Lua, the
    /// Settings and shortcut files are rendered again from the JSON, every stale .conf file is
    /// listed for removal (a dangling link too), and a Lua file that already exists wins.
    #[test]
    fn stale_conf_files_are_carried_over_once() {
        use crate::prefs;
        let mut files: std::collections::BTreeMap<&str, String> = Default::default();
        let none = |_: &str| None;
        let plan = |f: &std::collections::BTreeMap<&str, String>| {
            retire_plan(
                &|r| f.get(r).filter(|t| *t != "->").cloned(),
                &|r| f.contains_key(r),
                &none,
            )
        };
        assert_eq!(
            plan(&files),
            Retire::default(),
            "nothing stale, nothing done"
        );
        files.insert(
            "hypr/wolos/keyboard.conf",
            "# Written by WolOS Settings.\ninput {\n    kb_layout = br\n    kb_variant = \n    repeat_rate = 40\n    repeat_delay = 250\n}\n".into(),
        );
        files.insert(
            "hypr/wolos/displays.conf",
            "# Written by WolOS Settings.\nmonitor = eDP-1,1920x1080@60.00,auto,1.5\nmonitor = HDMI-A-1,1920x1080@60.00,auto,1,mirror,eDP-1\n".into(),
        );
        files.insert(
            "hypr/wolos/settings.conf",
            "general:layout = master\n".into(),
        );
        files.insert(
            "hypr/wolos/shortcuts.conf",
            "bind = SUPER, K, exec, foot\n".into(),
        );
        // A dangling link (the toolkit render gone): present, unreadable.
        files.insert("hypr/wolos/theme.conf", "->".into());
        files.insert(STALE_CONF[5], "plugin:hyprbars:bar_color = x\n".into());
        let tiled = DesktopPrefs {
            tiling: true,
            ..Default::default()
        };
        files.insert(PREFS_FILE, tiled.to_json());
        let mut o = Overrides::default();
        let defaults = shortcuts::defaults(shortcuts::SHIPPED, shortcuts::TILING);
        o.add_custom(
            &defaults,
            shortcuts::Custom {
                name: "Top".into(),
                combo: shortcuts::Combo::parse("SUPER CTRL", "U").unwrap(),
                command: "foot -e top".into(),
            },
        )
        .unwrap();
        files.insert(shortcuts::PREFS_FILE, o.to_json());
        // The person already has a displays.lua: it wins over the stale displays.conf.
        files.insert(
            prefs::DISPLAYS_FILE,
            prefs::displays_lua(None, "eDP-1,2560x1600@60.00,auto,2"),
        );

        let p = plan(&files);
        assert_eq!(p.remove, STALE_CONF);
        let wrote = |rel: &str| p.write.iter().find(|w| w.0 == rel).map(|w| w.1.clone());
        let kb = wrote(prefs::KEYBOARD_FILE).unwrap();
        assert_eq!(prefs::parse_keyboard(Some(&kb)), (2, 2), "{kb}");
        assert_eq!(
            wrote(prefs::DISPLAYS_FILE),
            None,
            "the person's displays.lua stays"
        );
        assert_eq!(wrote(INCLUDE), Some(tiled.include()));
        let sc = wrote(shortcuts::INCLUDE).unwrap();
        assert!(sc.contains("hl.dsp.exec_cmd(\"foot -e top\")"), "{sc}");
        assert!(
            sc.contains("hl.bind(\"SUPER + ALT + left\""),
            "tiling set: {sc}"
        );
        assert_eq!(
            crate::theme::apps::user_dir(Path::new("/h")).join("hypr/wolos/theme.conf"),
            Path::new("/h").join(STALE_CONF[5]),
            "the toolkit render"
        );

        // Applied: the stale files are gone, so the next run does nothing.
        for (rel, text) in p.write {
            files.insert(rel, text);
        }
        for rel in p.remove {
            files.remove(rel);
        }
        assert_eq!(plan(&files), Retire::default(), "once");

        // Without a displays.lua of its own the rules move over, mirror included; without
        // desktop.json the keyboard file carried over is the layout Settings starts from.
        files.remove(prefs::DISPLAYS_FILE);
        files.remove(PREFS_FILE);
        files.remove(INCLUDE);
        files.remove(prefs::KEYBOARD_FILE);
        files.insert(
            "hypr/wolos/displays.conf",
            "monitor = HDMI-A-1,1920x1080@60.00,auto,1,mirror,eDP-1\n".into(),
        );
        files.insert(
            "hypr/wolos/keyboard.conf",
            "input {\n    kb_layout = de\n    repeat_rate = 15\n}\n".into(),
        );
        let p = plan(&files);
        let d = p
            .write
            .iter()
            .find(|w| w.0 == prefs::DISPLAYS_FILE)
            .unwrap();
        assert_eq!(
            prefs::monitor_rules(&d.1),
            ["HDMI-A-1,1920x1080@60.00,auto,1,mirror,eDP-1"]
        );
        assert!(
            p.write.iter().all(|w| w.0 != INCLUDE),
            "no desktop.json, no settings.lua"
        );
        let kb = &p
            .write
            .iter()
            .find(|w| w.0 == prefs::KEYBOARD_FILE)
            .unwrap()
            .1;
        assert_eq!(prefs::parse_keyboard(Some(kb)), (5, 0));
    }
}
