// SPDX-License-Identifier: BSD-2-Clause
//! User desktop preferences written by Settings: shell prefs (menu bar, Dock), compositor keyboard/display
//! files (Lua, `hypr/wolos/*.lua`), hypridle config (the idle daemon's own `.conf`). Pure render/parse; callers write files and reload.
use std::collections::HashMap;

/// `$XDG_CONFIG_HOME/wolos/shell.json`; the shell rereads it on `reload` (wol-shell.sock).
pub const SHELL_FILE: &str = "wolos/shell.json";
/// `$XDG_CONFIG_HOME/hypr/wolos/<name>.lua`, loaded by the user hyprland.lua ([`USER_LUA`]).
pub const HYPR_DIR: &str = "hypr/wolos";
/// The person's own compositor config, `$XDG_CONFIG_HOME/hypr/hyprland.lua` (written once, then theirs).
pub const HYPR_USER: &str = "hypr/hyprland.lua";
/// The keyboard file the setup assistant writes (first session, ADR 0004).
pub const KEYBOARD_FILE: &str = "hypr/wolos/keyboard.lua";
/// Settings > Displays: one `hl.monitor` per display.
pub const DISPLAYS_FILE: &str = "hypr/wolos/displays.lua";
/// `$XDG_CONFIG_HOME/hypr/hypridle.conf` (hypridle's default user path).
pub const IDLE_FILE: &str = "hypr/hypridle.conf";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ShellPrefs {
    pub battery_percent: bool,
    pub show_date: bool,
    pub dock_autohide: bool,
    pub dock_magnify: bool,
    /// Milliseconds the pointer rests at the edge before an autohidden Dock shows.
    pub dock_autohide_delay_ms: u32,
    /// Dock icon size in pixels.
    pub dock_tile_size: u32,
    /// Search: milliseconds a slower provider (the file index) waits after a keystroke before
    /// its rows may be merged in (`priority_index_delay_ms`).
    pub priority_index_delay_ms: u32,
    /// Search: slower providers answering later than this after a keystroke are left out.
    pub searchbar_slow_timeout_ms: u32,
    /// Settings > Control Center: which optional modules show.
    pub control_center: ControlCenterPrefs,
    /// Status items of apps in the bar (the shell runs the session's tray watcher). Off: no
    /// watcher, so an app that would hide in the tray keeps its window.
    pub tray: bool,
}

impl Default for ShellPrefs {
    fn default() -> Self {
        Self {
            battery_percent: false,
            show_date: true,
            dock_autohide: false,
            dock_magnify: true,
            dock_autohide_delay_ms: 0,
            dock_tile_size: TILE_SIZE.2,
            priority_index_delay_ms: 0,
            searchbar_slow_timeout_ms: 400,
            control_center: ControlCenterPrefs::default(),
            tray: true,
        }
    }
}

/// Dock icon size: (min, max, default) pixels.
pub const TILE_SIZE: (u32, u32, u32) = (32, 96, 56);
/// Longest autohide delay Settings offers.
pub const AUTOHIDE_DELAY_MAX_MS: u32 = 5000;

impl ShellPrefs {
    pub fn tile_size(&self) -> u32 {
        self.dock_tile_size.clamp(TILE_SIZE.0, TILE_SIZE.1)
    }
    pub fn autohide_delay_ms(&self) -> u32 {
        self.dock_autohide_delay_ms.min(AUTOHIDE_DELAY_MAX_MS)
    }
}

/// Optional Control Center modules. Wi-Fi, Bluetooth, Focus, Dark Mode, Display and Sound always
/// show; a module whose hardware is missing (battery, keyboard backlight) hides regardless.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ControlCenterPrefs {
    pub now_playing: bool,
    pub battery: bool,
    pub keyboard_brightness: bool,
    pub accessibility: bool,
    pub night_shift: bool,
    pub screenshot: bool,
    pub record: bool,
}

impl Default for ControlCenterPrefs {
    fn default() -> Self {
        Self {
            now_playing: true,
            battery: true,
            keyboard_brightness: false,
            accessibility: false,
            night_shift: true,
            screenshot: true,
            record: true,
        }
    }
}

impl ControlCenterPrefs {
    /// (key, label) in Control Center order: the Settings pane renders one toggle per entry.
    pub const MODULES: [(&'static str, &'static str); 7] = [
        ("now_playing", "Now Playing"),
        ("battery", "Battery"),
        ("keyboard_brightness", "Keyboard Brightness"),
        ("accessibility", "Accessibility Shortcuts"),
        ("night_shift", "Night Shift"),
        ("screenshot", "Screenshot"),
        ("record", "Screen Recording"),
    ];

    fn field(&mut self, key: &str) -> Option<&mut bool> {
        Some(match key {
            "now_playing" => &mut self.now_playing,
            "battery" => &mut self.battery,
            "keyboard_brightness" => &mut self.keyboard_brightness,
            "accessibility" => &mut self.accessibility,
            "night_shift" => &mut self.night_shift,
            "screenshot" => &mut self.screenshot,
            "record" => &mut self.record,
            _ => return None,
        })
    }

    pub fn get(&self, key: &str) -> Option<bool> {
        let mut c = *self;
        c.field(key).map(|v| *v)
    }

    /// False for an unknown key.
    pub fn set(&mut self, key: &str, on: bool) -> bool {
        self.field(key).map(|v| *v = on).is_some()
    }
}

impl ShellPrefs {
    /// Missing or broken file = defaults; unknown keys ignored.
    pub fn parse(text: Option<&str>) -> Self {
        text.and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or_default()
    }

    /// [`ShellPrefs::parse`] for the live reload: a file that does not load is an error naming
    /// the first key (in file order) whose value does not fit, `control_center.<module>` inside
    /// that object; text that is not a JSON object names its line. Unknown keys still load.
    pub fn try_parse(text: &str) -> Result<Self, ConfigError> {
        let pairs: Ordered =
            serde_json::from_str(text).map_err(|e| ConfigError::syntax(SHELL_NAME, &e))?;
        for (k, v) in &pairs.0 {
            let Err(e) = serde_json::from_value::<ShellPrefs>(one(k, v)) else {
                continue;
            };
            let inner = match v {
                serde_json::Value::Object(m) if k == "control_center" => {
                    m.iter().find_map(|(ik, iv)| {
                        serde_json::from_value::<ControlCenterPrefs>(one(ik, iv))
                            .err()
                            .map(|e| (format!("{k}.{ik}"), e))
                    })
                }
                _ => None,
            };
            let (key, e) = inner.unwrap_or((k.clone(), e));
            return Err(ConfigError::key(SHELL_NAME, key, &e));
        }
        // Each key fits on its own; what is left is the object as a whole (a key twice).
        serde_json::from_str(text).map_err(|e| ConfigError::syntax(SHELL_NAME, &e))
    }
    pub fn to_json(self) -> String {
        serde_json::to_string_pretty(&self).unwrap_or_default() + "\n"
    }
}

/// `shell.json` as a whole (ADR 0031): Settings' [`ShellPrefs`] plus the `layout` key (where the
/// bar and the Dock sit and what their slots hold). [`ShellPrefs`] stays `Copy` for Settings, so
/// the layout is a field of the file, next to it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellFile {
    pub prefs: ShellPrefs,
    /// Missing, broken or not valid (`Layout::validate`) = today's desktop.
    pub layout: shared::layout::Layout,
}

/// The `layout` key of `shell.json`.
pub const LAYOUT_KEY: &str = "layout";

impl ShellFile {
    pub fn parse(text: Option<&str>) -> Self {
        Self {
            prefs: ShellPrefs::parse(text),
            layout: layout(text),
        }
    }

    /// [`ShellFile::parse`] for the live reload: the prefs as [`ShellPrefs::try_parse`], and a
    /// `layout` that does not validate is an error naming `layout` (never today's desktop).
    pub fn try_parse(text: &str) -> Result<Self, ConfigError> {
        let prefs = ShellPrefs::try_parse(text)?;
        let v: serde_json::Value =
            serde_json::from_str(text).map_err(|e| ConfigError::syntax(SHELL_NAME, &e))?;
        let layout = match v.get(LAYOUT_KEY) {
            None => shared::layout::Layout::default(),
            Some(l) => shared::layout::Layout::parse(&l.to_string()).map_err(|e| ConfigError {
                file: SHELL_NAME,
                key: Some(LAYOUT_KEY.into()),
                line: None,
                detail: e,
            })?,
        };
        Ok(Self { prefs, layout })
    }

    pub fn to_json(&self) -> String {
        let mut v = serde_json::to_value(self.prefs).unwrap_or_default();
        if let (Some(o), Ok(l)) = (v.as_object_mut(), serde_json::to_value(&self.layout)) {
            o.insert(LAYOUT_KEY.into(), l);
        }
        serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
    }
}

/// File names in notices and logs (the files live under `$XDG_CONFIG_HOME/wolos/`).
pub const SHELL_NAME: &str = "shell.json";
/// The desktop widgets file, beside [`SHELL_NAME`].
pub const WIDGETS_NAME: &str = "widgets.json";

/// Why a config file did not load (live reload): the file, the first key whose value does not fit
/// (None when the text is not JSON, or not an object), its line when known, and the reader's own
/// words (logs only, never on screen).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    pub file: &'static str,
    pub key: Option<String>,
    pub line: Option<usize>,
    pub detail: String,
}

impl ConfigError {
    /// The text of `file` is not JSON: no key, the parser's line and words.
    pub fn syntax(file: &'static str, e: &serde_json::Error) -> Self {
        Self {
            file,
            key: None,
            line: Some(e.line()),
            detail: e.to_string(),
        }
    }

    fn key(file: &'static str, key: String, e: &serde_json::Error) -> Self {
        Self {
            file,
            key: Some(key),
            line: None,
            detail: e.to_string(),
        }
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.file)?;
        if let Some(l) = self.line {
            write!(f, " line {l}")?;
        }
        if let Some(k) = &self.key {
            write!(f, " key {k}")?;
        }
        write!(f, ": {}", self.detail)
    }
}

/// A JSON object's entries in file order (so "the first bad key" is the first one in the file).
struct Ordered(Vec<(String, serde_json::Value)>);

impl<'de> serde::Deserialize<'de> for Ordered {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Ordered;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut m: A,
            ) -> Result<Ordered, A::Error> {
                let mut v = Vec::new();
                while let Some(kv) = m.next_entry()? {
                    v.push(kv);
                }
                Ok(Ordered(v))
            }
        }
        d.deserialize_map(V)
    }
}

/// `{key: value}`.
fn one(k: &str, v: &serde_json::Value) -> serde_json::Value {
    serde_json::Value::Object([(k.to_string(), v.clone())].into_iter().collect())
}

/// `widgets.json` text that the live reload must not apply (the board keeps what it shows): not
/// a JSON object (its line), a `widgets` key that is not a list, or `grants` that do not load.
/// Single widget entries that do not load are no error: the board keeps them aside as before.
pub fn widgets_error(text: &str) -> Option<ConfigError> {
    let pairs: Ordered = match serde_json::from_str(text) {
        Ok(p) => p,
        Err(e) => return Some(ConfigError::syntax(WIDGETS_NAME, &e)),
    };
    pairs.0.iter().find_map(|(k, v)| {
        let e = match k.as_str() {
            "widgets" => serde_json::from_value::<Vec<serde_json::Value>>(v.clone()).err(),
            "grants" => serde_json::from_value::<Vec<shared::widgets::Grant>>(v.clone()).err(),
            _ => None,
        }?;
        Some(ConfigError::key(WIDGETS_NAME, k.clone(), &e))
    })
}

/// The layout saved in `shell.json` text; today's desktop when there is none or it does not
/// validate (the bar and the Dock on one edge, an item listed twice).
pub fn layout(text: Option<&str>) -> shared::layout::Layout {
    text.and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
        .and_then(|v| v.get(LAYOUT_KEY).map(ToString::to_string))
        .and_then(|l| shared::layout::Layout::parse(&l).ok())
        .unwrap_or_default()
}

/// What Settings writes after changing `p`: the new prefs with the layout `old` already had, so
/// saving a Menu Bar or Dock setting never moves the bar or the Dock.
pub fn keep_layout(old: Option<&str>, p: ShellPrefs) -> String {
    ShellFile {
        prefs: p,
        layout: layout(old),
    }
    .to_json()
}

/// `old` with a new layout, the prefs kept. A layout that does not validate is refused.
pub fn with_layout(old: Option<&str>, l: &shared::layout::Layout) -> Result<String, String> {
    l.validate()?;
    Ok(ShellFile {
        prefs: ShellPrefs::parse(old),
        layout: l.clone(),
    }
    .to_json())
}

/// (xkb layout, label).
pub const LAYOUTS: [(&str, &str); 8] = [
    ("us", "U.S."),
    ("us(intl)", "U.S. International"),
    ("br", "Brazilian (ABNT2)"),
    ("pt", "Portuguese"),
    ("gb", "British"),
    ("de", "German"),
    ("fr", "French"),
    ("es", "Spanish"),
];

/// (label, repeat_rate per s, repeat_delay ms).
pub const REPEATS: [(&str, u32, u32); 3] =
    [("Slow", 15, 600), ("Normal", 25, 400), ("Fast", 40, 250)];

/// `keyboard.lua`: the layout and repeat as one `hl.config` call. None for an unknown layout.
pub fn keyboard_lua(layout: &str, repeat: usize) -> Option<String> {
    use crate::theme::hypr::lua_str;
    let (_, rate, delay) = REPEATS.get(repeat)?;
    let (kb, variant) = match layout.split_once('(') {
        Some((l, v)) => (l, v.trim_end_matches(')')),
        None => (layout, ""),
    };
    LAYOUTS.iter().any(|l| l.0 == layout).then(|| {
        format!(
            "-- Written by WolOS Settings.\nhl.config({{\n    input = {{\n        kb_layout = {},\n        kb_variant = {},\n        repeat_rate = {rate},\n        repeat_delay = {delay},\n    }},\n}})\n",
            lua_str(kb),
            lua_str(variant)
        )
    })
}

/// Setup assistant `~/.config/wol/setup.json` (ADR 0004) -> `keyboard.lua`, normal repeat. None if unknown.
pub fn setup_keyboard(setup_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(setup_json).ok()?;
    keyboard_lua(v.get("keymap")?.as_str()?, 1)
}

/// Setup assistant choices, relative to `$XDG_CONFIG_HOME`; renamed to `SETUP_APPLIED` once applied.
pub const SETUP_FILE: &str = "wol/setup.json";
pub const SETUP_APPLIED: &str = "wol/setup.applied.json";

/// `key = value` lines, the value without a trailing `,` or quotes: `keyboard.lua` fields, and the
/// `input { }` block of the older `keyboard.conf` (read once, [`crate::hyprprefs::retire_plan`]).
fn kv(text: &str) -> HashMap<&str, &str> {
    crate::system::pairs(text, '=')
        .into_iter()
        .map(|(k, v)| {
            let v = v.trim_end_matches(',').trim();
            (
                k,
                v.strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .unwrap_or(v),
            )
        })
        .collect()
}

/// (layout index, repeat index) from `keyboard.lua`; defaults (U.S., Normal).
pub fn parse_keyboard(text: Option<&str>) -> (usize, usize) {
    let Some(t) = text else { return (0, 1) };
    let p = kv(t);
    let kb = p.get("kb_layout").copied().unwrap_or("us");
    let id = match p.get("kb_variant").copied().unwrap_or("") {
        "" => kb.to_string(),
        v => format!("{kb}({v})"),
    };
    let l = LAYOUTS.iter().position(|x| x.0 == id).unwrap_or(0);
    let rate: u32 = p
        .get("repeat_rate")
        .and_then(|r| r.parse().ok())
        .unwrap_or(25);
    let r = REPEATS.iter().position(|x| x.1 == rate).unwrap_or(1);
    (l, r)
}

/// A monitor rule (`name,mode,position,scale[,key,value…]`, `controls::monitor_rule`) as the
/// `hl.monitor` call that sets it. Each field is a string but `transform` and `bitdepth` (numbers).
pub fn monitor_lua(rule: &str) -> String {
    use crate::theme::hypr::lua_str;
    let parts: Vec<&str> = rule.split(',').map(str::trim).collect();
    let mut fields: Vec<String> = ["output", "mode", "position", "scale"]
        .iter()
        .zip(&parts)
        .map(|(k, v)| format!("{k} = {}", lua_str(v)))
        .collect();
    for kv in parts.get(4..).unwrap_or_default().chunks(2) {
        if let [k, v] = kv {
            let number = matches!(*k, "transform" | "bitdepth") && v.parse::<u8>().is_ok();
            fields.push(format!(
                "{k} = {}",
                if number { v.to_string() } else { lua_str(v) }
            ));
        }
    }
    format!("hl.monitor({{ {} }})", fields.join(", "))
}

/// The monitor rules of `displays.lua` (`hl.monitor` lines, [`monitor_lua`]) or of the older
/// `displays.conf` (`monitor = ` lines), in file order.
pub fn monitor_rules(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for l in text.lines().map(str::trim) {
        if let Some(r) = l.strip_prefix("monitor = ") {
            out.push(r.trim().to_string());
            continue;
        }
        let Some(body) = l
            .strip_prefix("hl.monitor({ ")
            .and_then(|b| b.strip_suffix(" })"))
        else {
            continue;
        };
        let mut vals = Vec::new();
        for f in body.split(", ") {
            let Some((k, v)) = f.split_once(" = ") else {
                continue;
            };
            let v = v
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(v);
            match k {
                "output" | "mode" | "position" | "scale" => vals.push(v.to_string()),
                _ => vals.extend([k.to_string(), v.to_string()]),
            }
        }
        out.push(vals.join(","));
    }
    out
}

/// `displays.lua`: one `hl.monitor` per monitor (a later rule for the same name replaces it).
pub fn displays_lua(current: Option<&str>, rule: &str) -> String {
    let name = rule.split(',').next().unwrap_or("");
    let mut out = String::from("-- Written by WolOS Settings.\n");
    for r in monitor_rules(current.unwrap_or("")) {
        if r.split(',').next() != Some(name) {
            out.push_str(&monitor_lua(&r));
            out.push('\n');
        }
    }
    out.push_str(&monitor_lua(rule));
    out.push('\n');
    out
}

/// The line of the user hyprland.lua that loads the WolOS files ([`HYPR_DIR`], relative to it).
pub const USER_REQUIRE: &str = "local ok, err = pcall(require, \"./wolos/*.lua\")";

/// The user hyprland.lua `wol-session` writes: the system defaults, then the WolOS files (none yet
/// is not an error, a broken one is).
pub fn user_lua(share: &str) -> String {
    format!(
        "-- WolOS defaults. Add your overrides below.\nrequire(\"{share}/hypr/hyprland.lua\")\n{USER_REQUIRE}\n\
         if not ok and not tostring(err):find(\"found no match\", 1, true) then\n    error(err, 0)\nend\n"
    )
}

/// The user hyprland.lua with the [`USER_REQUIRE`] block; None when it is already there.
pub fn ensure_require(user_lua_text: Option<&str>) -> Option<String> {
    let Some(cur) = user_lua_text else {
        return Some(user_lua("/usr/share/wolos"));
    };
    if cur.lines().any(|l| l.trim() == USER_REQUIRE) {
        return None;
    }
    let mut s = cur.trim_end_matches('\n').to_string();
    s.push_str(&format!(
        "\n-- WolOS Settings and theme (hypr/wolos).\n{USER_REQUIRE}\n\
         if not ok and not tostring(err):find(\"found no match\", 1, true) then\n    error(err, 0)\nend\n"
    ));
    Some(s)
}

/// The user compositor file of WolOS 0.12 and before (`source = ` lines). The compositor reads only
/// [`HYPR_USER`] now; the person's own lines in it are carried over by [`carry_conf`].
pub const USER_CONF: &str = "hypr/hyprland.conf";

/// First line of the block [`carry_conf`] adds to the user hyprland.lua.
pub const CARRIED: &str =
    "-- WolOS 0.13: your own lines from hyprland.conf, kept here as comments.";

/// The person's own lines of an older user hyprland.conf ([`USER_CONF`]), in file order: every
/// line but blank ones and the ones WolOS wrote there (its `# WolOS` comments, the `source =` of
/// the system defaults and of `hypr/wolos/*.conf`).
pub fn own_conf_lines(conf: &str) -> Vec<&str> {
    conf.lines()
        .map(str::trim_end)
        .filter(|l| {
            let t = l.trim();
            let wolos_source = t
                .strip_prefix("source")
                .map(|r| r.trim_start())
                .and_then(|r| r.strip_prefix('='))
                .map(str::trim)
                .is_some_and(|p| {
                    p.ends_with("/wolos/hypr/hyprland.conf") || p.ends_with("/hypr/wolos/*.conf")
                });
            !t.is_empty() && !t.starts_with("# WolOS") && !wolos_source
        })
        .collect()
}

/// The user hyprland.lua with the person's own hyprland.conf lines ([`own_conf_lines`]) added at
/// the end as one commented block under a notice ([`CARRIED`]): the compositor reads only Lua
/// now, so each line has to be written again in Lua, and none is dropped on the way. Lines the
/// file already holds as such a comment, and lines in `carried` (an earlier carry), are not added
/// again; None when nothing is left to add. A missing hyprland.lua starts from [`user_lua`].
pub fn carry_conf(conf: &str, user_lua_text: Option<&str>, carried: &[&str]) -> Option<String> {
    let cur = user_lua_text.map_or_else(|| user_lua("/usr/share/wolos"), str::to_owned);
    let have: std::collections::HashSet<&str> = cur.lines().map(str::trim_end).collect();
    let own: Vec<&str> = own_conf_lines(conf)
        .into_iter()
        .filter(|l| !carried.contains(l) && !have.contains(format!("-- {l}").as_str()))
        .collect();
    if own.is_empty() {
        return None;
    }
    let mut s = cur.trim_end_matches('\n').to_string();
    s.push_str(&format!(
        "\n\n{CARRIED}\n-- The compositor now reads only this file: write each line again in Lua (hl.bind,\n\
         -- hl.config, hl.window_rule, hl.exec_cmd), then delete its comment. hyprland.conf is unchanged.\n"
    ));
    for l in own {
        s.push_str("-- ");
        s.push_str(l);
        s.push('\n');
    }
    Some(s)
}

/// Lock-after choices in seconds (0 = never).
pub const IDLE_CHOICES: [(&str, u32); 5] = [
    ("1 minute", 60),
    ("2 minutes", 120),
    ("5 minutes", 300),
    ("10 minutes", 600),
    ("Never", 0),
];
/// Display off this long after the lock.
pub const DISPLAY_OFF_AFTER_LOCK: u32 = 60;

/// hypridle config: lock after `lock_secs`, display off a minute later; `require_password` = lock on sleep.
/// Then the battery-only steps (display off, sleep: `power::policy::hypridle_listeners`).
///
/// Lock on sleep holds the system's sleep (a logind delay inhibitor, `inhibit_sleep = 3`) until
/// the session is locked, that is until wol-lock has shown its first frame on every display, so
/// waking up never shows the desktop before the lock screen.
pub fn hypridle_conf(lock_secs: u32, lock_on_sleep: bool) -> String {
    use crate::power::policy::LOCK_CMD;
    let mut s = format!("# Written by WolOS Settings.\ngeneral {{\n    lock_cmd = {LOCK_CMD}\n");
    if lock_on_sleep {
        s.push_str(&format!(
            "    before_sleep_cmd = {LOCK_CMD}\n    inhibit_sleep = 3\n"
        ));
    }
    s.push_str("    after_sleep_cmd = hyprctl dispatch dpms on\n}\n");
    if lock_secs > 0 {
        s.push_str(&format!(
            "listener {{\n    timeout = {lock_secs}\n    on-timeout = {LOCK_CMD}\n}}\nlistener {{\n    timeout = {}\n    on-timeout = hyprctl dispatch dpms off\n    on-resume = hyprctl dispatch dpms on\n}}\n",
            lock_secs + DISPLAY_OFF_AFTER_LOCK
        ));
    }
    s.push_str(&crate::power::policy::hypridle_listeners());
    s
}

/// (lock_secs, lock_on_sleep) from a hypridle config; system defaults (300, true) when absent.
pub fn parse_hypridle(text: Option<&str>) -> (u32, bool) {
    let Some(t) = text else { return (300, true) };
    let lock = t
        .split("listener")
        .skip(1)
        .find(|b| b.contains("wol-lock"))
        .and_then(|b| kv(b).get("timeout").and_then(|v| v.parse().ok()))
        .unwrap_or(0);
    (lock, t.contains("before_sleep_cmd"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_prefs_defaults_and_round_trip() {
        assert_eq!(ShellPrefs::parse(None), ShellPrefs::default());
        assert_eq!(ShellPrefs::parse(Some("junk")), ShellPrefs::default());
        let p = ShellPrefs {
            battery_percent: true,
            dock_magnify: false,
            ..Default::default()
        };
        assert_eq!(ShellPrefs::parse(Some(&p.to_json())), p);
        assert!(ShellPrefs::parse(Some(r#"{"show_date":false}"#)).dock_magnify);
    }

    #[test]
    fn searchbar_keys_default() {
        let d = ShellPrefs::default();
        assert_eq!(d.priority_index_delay_ms, 0);
        assert_eq!(d.searchbar_slow_timeout_ms, 400);
        let p = ShellPrefs::parse(Some(r#"{"priority_index_delay_ms":50}"#));
        assert_eq!(p.priority_index_delay_ms, 50);
    }

    #[test]
    fn dock_keys_default_and_clamp() {
        let d = ShellPrefs::default();
        assert_eq!(d.dock_autohide_delay_ms, 0);
        assert_eq!(d.dock_tile_size, 56);
        let old = ShellPrefs::parse(Some(r#"{"dock_autohide":true}"#));
        assert_eq!(old.dock_tile_size, 56);
        let wild = ShellPrefs::parse(Some(
            r#"{"dock_tile_size":4000,"dock_autohide_delay_ms":999999,"dock_minimize_speed_pct":0}"#,
        ));
        assert_eq!(wild.tile_size(), TILE_SIZE.1);
        assert_eq!(wild.autohide_delay_ms(), AUTOHIDE_DELAY_MAX_MS);
        // A file from before audit2 A2-5 (the unused recents / minimise-speed keys) still loads.
        let older = ShellPrefs::parse(Some(r#"{"dock_show_recents":false,"show_date":false}"#));
        assert!(!older.show_date);
    }

    #[test]
    fn control_center_modules_persist_and_default() {
        let old = ShellPrefs::parse(Some(r#"{"show_date":false}"#));
        assert_eq!(old.control_center, ControlCenterPrefs::default());
        let d = ControlCenterPrefs::default();
        assert!(d.now_playing && d.battery && d.screenshot && d.record && d.night_shift);
        assert!(!d.keyboard_brightness && !d.accessibility, "opt-in");
        let mut p = ShellPrefs::default();
        for (key, label) in ControlCenterPrefs::MODULES {
            assert!(!label.is_empty());
            let on = p.control_center.get(key).unwrap();
            assert!(p.control_center.set(key, !on), "{key}");
            assert_eq!(p.control_center.get(key), Some(!on), "{key}");
        }
        assert_eq!(ShellPrefs::parse(Some(&p.to_json())), p);
        assert_eq!(p.control_center.get("airdrop"), None);
        assert!(!p.control_center.set("airdrop", true));
        let partial = ShellPrefs::parse(Some(r#"{"control_center":{"record":false}}"#));
        assert!(!partial.control_center.record && partial.control_center.screenshot);
    }

    #[test]
    fn keyboard_round_trip() {
        for (l, r) in [(0, 1), (1, 0), (2, 2), (7, 1)] {
            let c = keyboard_lua(LAYOUTS[l].0, r).unwrap();
            assert_eq!(parse_keyboard(Some(&c)), (l, r), "{c}");
        }
        let intl = keyboard_lua("us(intl)", 1).unwrap();
        assert!(intl.contains("        kb_variant = \"intl\",\n"), "{intl}");
        assert!(intl.contains("        repeat_rate = 25,\n"), "{intl}");
        assert_eq!(keyboard_lua("xx; exec", 1), None);
        assert_eq!(keyboard_lua("us", 9), None);
        assert_eq!(parse_keyboard(None), (0, 1));
        // The older keyboard.conf block reads the same (the upgrade carries it over once).
        let conf = "input {\n    kb_layout = br\n    kb_variant = \n    repeat_rate = 40\n    repeat_delay = 250\n}\n";
        assert_eq!(parse_keyboard(Some(conf)), (2, 2));
    }

    #[test]
    fn displays_and_the_user_file() {
        let a = displays_lua(None, "eDP-1,1920x1080@60.00,auto,1");
        let b = displays_lua(Some(&a), "HDMI-A-1,2560x1440@60.00,1920x0,1");
        let c = displays_lua(Some(&b), "eDP-1,1920x1080@60.00,auto,1.5");
        assert_eq!(c.matches("hl.monitor(").count(), 2);
        assert!(
            c.ends_with(
                "hl.monitor({ output = \"eDP-1\", mode = \"1920x1080@60.00\", position = \"auto\", scale = \"1.5\" })\n"
            ),
            "{c}"
        );
        assert_eq!(
            monitor_rules(&c),
            [
                "HDMI-A-1,2560x1440@60.00,1920x0,1",
                "eDP-1,1920x1080@60.00,auto,1.5"
            ]
        );
        let mirror = "HDMI-A-1,1920x1080@60.00,auto,1,mirror,eDP-1";
        assert_eq!(
            monitor_lua(mirror),
            "hl.monitor({ output = \"HDMI-A-1\", mode = \"1920x1080@60.00\", position = \"auto\", scale = \"1\", mirror = \"eDP-1\" })"
        );
        assert_eq!(monitor_rules(&monitor_lua(mirror)), [mirror]);
        assert_eq!(
            monitor_lua("DP-1,preferred,auto,1,transform,1"),
            "hl.monitor({ output = \"DP-1\", mode = \"preferred\", position = \"auto\", scale = \"1\", transform = 1 })"
        );
        // The older displays.conf lines read the same.
        assert_eq!(
            monitor_rules("# Written by WolOS Settings.\nmonitor = eDP-1,1920x1080@60.00,auto,1\n"),
            ["eDP-1,1920x1080@60.00,auto,1"]
        );
        let s = ensure_require(None).unwrap();
        assert_eq!(s, user_lua("/usr/share/wolos"));
        assert_eq!(ensure_require(Some(&s)), None);
        let mine = "require(\"/usr/share/wolos/hypr/hyprland.lua\")\nhl.bind(\"SUPER + B\", hl.dsp.exec_cmd(\"firefox\"))\n";
        let added = ensure_require(Some(mine)).unwrap();
        assert!(added.starts_with(mine), "the person's lines stay first");
        assert_eq!(added.matches(USER_REQUIRE).count(), 1);
        assert_eq!(ensure_require(Some(&added)), None);
    }

    /// wol-session writes the same user hyprland.lua as [`user_lua`] (with its `$share`).
    #[test]
    fn wol_session_writes_the_user_lua() {
        let s = include_str!("../../../desktop/overlay/usr/bin/wol-session");
        let body = s
            .split("cat > \"$lua\" <<LUA\n")
            .nth(1)
            .and_then(|b| b.split("\nLUA\n").next())
            .expect("wol-session writes hyprland.lua");
        assert_eq!(format!("{body}\n"), user_lua("$share"));
        // The compositor reads Lua only: the script writes no compositor .conf, it removes the old
        // VM fragment.
        assert!(!s.contains("<<CONF") && !s.contains("source = "), "{s}");
        assert!(s.contains("rm -f \"${vmlua%.lua}.conf\""), "{s}");
    }

    /// The VM fragment `wol-session` writes, as the text the compositor loads.
    #[test]
    fn wol_session_writes_the_vm_fragment() {
        let s = include_str!("../../../desktop/overlay/usr/bin/wol-session");
        let lines: Vec<&str> = s
            .lines()
            .filter_map(|l| l.trim().strip_prefix("printf '%s\\n' "))
            .filter(|l| l.contains("wol-session:"))
            .collect();
        assert_eq!(
            lines,
            [
                "'-- wol-session: this is a VM' 'hl.config({ cursor = { no_hardware_cursors = 1 } })' > \"$vmlua.tmp\"",
                "'-- wol-session: not a VM' > \"$vmlua.tmp\"",
            ]
        );
    }

    #[test]
    fn hypridle_round_trip() {
        for (secs, sleep) in [(60, true), (600, false), (0, true)] {
            assert_eq!(
                parse_hypridle(Some(&hypridle_conf(secs, sleep))),
                (secs, sleep)
            );
        }
        let c = hypridle_conf(300, true);
        assert!(c.contains("timeout = 360\n    on-timeout = hyprctl dispatch dpms off"));
        assert_eq!(parse_hypridle(None), (300, true));
        // Only this account's lock counts, and lock on sleep holds the sleep until it shows.
        assert!(c.contains("    before_sleep_cmd = pgrep -u \"$(id -u)\" -x wol-lock || wol-lock\n    inhibit_sleep = 3\n"), "{c}");
        assert!(!c.contains("pgrep -x"), "{c}");
        assert!(!hypridle_conf(300, false).contains("inhibit_sleep"));
        // The shipped default is Settings' 5 minutes with lock on sleep.
        let shipped = include_str!("../../../desktop/overlay/usr/share/wolos/hypr/hypridle.conf");
        assert_eq!(
            format!("# Written by WolOS Settings.\n{shipped}"),
            hypridle_conf(300, true)
        );
        // Battery display off and sleep (power::policy) whatever the lock setting, Never too.
        for secs in [0, 300] {
            let c = hypridle_conf(secs, false);
            assert!(
                c.ends_with(&crate::power::policy::hypridle_listeners()),
                "{c}"
            );
        }
    }

    /// ADR 0031: the layout lives in shell.json next to Settings' prefs; saving prefs keeps it,
    /// and a broken or invalid layout falls back to today's desktop without losing the prefs.
    #[test]
    fn the_layout_is_kept_when_settings_saves_prefs() {
        use shared::layout::{BarItem, Edge, Layout};
        assert_eq!(layout(None), Layout::default());
        let mut l = Layout::default();
        l.right.insert(
            0,
            BarItem::Plugin {
                plugin: "org.wolos.stats".into(),
                id: "bar".into(),
            },
        );
        let p = ShellPrefs {
            battery_percent: true,
            ..ShellPrefs::default()
        };
        let text = with_layout(Some(&p.to_json()), &l).unwrap();
        let f = ShellFile::parse(Some(&text));
        assert_eq!((f.prefs, &f.layout), (p, &l));
        assert_eq!(
            ShellPrefs::parse(Some(&text)),
            p,
            "old readers ignore the key"
        );
        let q = ShellPrefs {
            show_date: false,
            ..p
        };
        let saved = keep_layout(Some(&text), q);
        assert_eq!(
            ShellFile::parse(Some(&saved)),
            ShellFile {
                prefs: q,
                layout: l
            }
        );
        let bad = Layout {
            dock: Some(Edge::Top),
            ..Layout::default()
        };
        assert!(with_layout(None, &bad).is_err());
        let broken = r#"{"battery_percent": true, "layout": {"bar": "top", "dock": "top"}}"#;
        let f = ShellFile::parse(Some(broken));
        assert!(f.prefs.battery_percent);
        assert_eq!(f.layout, Layout::default());
        assert_eq!(layout(Some(r#"{"layout": 3}"#)), Layout::default());
    }

    /// SP-2: the live reload names the first key that does not fit and loads what Settings writes.
    #[test]
    fn try_parse_names_the_first_bad_key() {
        let p = ShellPrefs {
            dock_magnify: false,
            ..Default::default()
        };
        assert_eq!(ShellPrefs::try_parse(&p.to_json()), Ok(p));
        assert_eq!(ShellPrefs::try_parse("{}"), Ok(ShellPrefs::default()));
        assert!(ShellPrefs::try_parse(r#"{"gone_key":[1],"show_date":false}"#).is_ok());
        let key = |t: &str| ShellPrefs::try_parse(t).unwrap_err().key;
        assert_eq!(
            key(r#"{"show_date":false,"dock_tile_size":"big","battery_percent":3}"#).as_deref(),
            Some("dock_tile_size"),
            "file order, not name order"
        );
        assert_eq!(
            key(r#"{"control_center":{"battery":true,"record":"yes"}}"#).as_deref(),
            Some("control_center.record")
        );
        assert_eq!(
            key(r#"{"control_center":4}"#).as_deref(),
            Some("control_center")
        );
        let e = ShellPrefs::try_parse("{\n  \"show_date\": nope\n}").unwrap_err();
        assert_eq!((e.key, e.line, e.file), (None, Some(2), SHELL_NAME));
        assert_eq!(ShellPrefs::try_parse("[]").unwrap_err().key, None);
        assert!(ShellPrefs::try_parse(r#"{"show_date":true,"show_date":false}"#).is_err());
        let bad_layout = r#"{"battery_percent":true,"layout":{"bar":"top","dock":"top"}}"#;
        assert_eq!(
            ShellFile::try_parse(bad_layout).unwrap_err().key.as_deref(),
            Some(LAYOUT_KEY)
        );
        let good = keep_layout(None, p);
        assert_eq!(
            ShellFile::try_parse(&good),
            Ok(ShellFile::parse(Some(&good)))
        );
    }

    #[test]
    fn widgets_error_names_the_key() {
        assert_eq!(widgets_error(r#"{"version":1,"widgets":[]}"#), None);
        assert_eq!(
            widgets_error(r#"{"widgets":[{"not":"a widget"}]}"#),
            None,
            "kept aside"
        );
        let e = widgets_error(r#"{"widgets":{}}"#).unwrap();
        assert_eq!((e.file, e.key.as_deref()), (WIDGETS_NAME, Some("widgets")));
        assert_eq!(
            widgets_error(r#"{"widgets":[],"grants":7}"#)
                .unwrap()
                .key
                .as_deref(),
            Some("grants")
        );
        let e = widgets_error("{\n\"widgets\": [,]}").unwrap();
        assert_eq!((e.key, e.line), (None, Some(2)));
    }

    #[test]
    fn setup_keyboard_once() {
        let k = setup_keyboard(r#"{"keymap":"br","locale":"pt_BR.UTF-8","appearance":"auto"}"#)
            .unwrap();
        assert!(k.contains("kb_layout = \"br\",\n"), "{k}");
        assert!(
            setup_keyboard(r#"{"keymap":"us(intl)"}"#)
                .unwrap()
                .contains("kb_variant = \"intl\"")
        );
        assert_eq!(setup_keyboard(r#"{"keymap":"xx"}"#), None);
        assert_eq!(setup_keyboard("junk"), None);
    }
}
