// SPDX-License-Identifier: BSD-2-Clause
//! Keyboard shortcuts for System Settings > Keyboard > Shortcuts. The shipped compositor configuration
//! (`hyprland.lua`, `tiling.lua`, read as bind lines by `theme::hypr::conf_binds`) is the single source of
//! truth for defaults; every bind gets a human name and group here. User changes and custom shortcuts
//! live in `wolos/shortcuts.json`, render to the managed Lua file `hypr/wolos/shortcuts.lua`
//! (`hl.unbind` + `hl.bind`) and apply live with `hyprctl keyword unbind/bind` (no reload).
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PREFS_FILE: &str = "wolos/shortcuts.json";
pub const INCLUDE: &str = "hypr/wolos/shortcuts.lua";
/// Installed defaults (Lua); the repo copies below are the fallback (dev, tests).
pub const SHIPPED_PATH: &str = "/usr/share/wolos/hypr/hyprland.lua";
pub const TILING_PATH: &str = "/usr/share/wolos/hypr/tiling.lua";
pub const SHIPPED: &str =
    include_str!("../../../desktop/overlay/usr/share/wolos/hypr/hyprland.lua");
pub const TILING: &str = include_str!("../../../desktop/overlay/usr/share/wolos/hypr/tiling.lua");

pub const CTRL: u8 = 1;
pub const ALT: u8 = 2;
pub const SHIFT: u8 = 4;
pub const SUPER: u8 = 8;
/// (bit, compositor name, symbol) in display order ⌃⌥⇧⌘.
const MODS: [(u8, &str, &str); 4] = [
    (CTRL, "CTRL", "⌃"),
    (ALT, "ALT", "⌥"),
    (SHIFT, "SHIFT", "⇧"),
    (SUPER, "SUPER", "⌘"),
];

/// Modifiers + one key (xkb keysym name as the compositor and GDK spell it).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Combo {
    pub mods: u8,
    pub key: String,
}

impl Combo {
    /// The compositor's MODS field ("SUPER SHIFT", "CTRL $mod" after substitution, "SUPER_SHIFT") + key.
    pub fn parse(mods: &str, key: &str) -> Option<Self> {
        let key = key.trim();
        if key.is_empty() || key.contains([',', ' ']) {
            return None;
        }
        let mut bits = 0;
        for m in mods.split([' ', '_', '+']).filter(|m| !m.is_empty()) {
            bits |= match m.to_ascii_uppercase().as_str() {
                "SUPER" | "WIN" | "LOGO" | "MOD4" => SUPER,
                "SHIFT" => SHIFT,
                "CTRL" | "CONTROL" => CTRL,
                "ALT" | "MOD1" => ALT,
                _ => return None,
            };
        }
        Some(Self {
            mods: bits,
            key: key.to_string(),
        })
    }

    /// Same keys, ignoring letter case of the key name.
    pub fn same(&self, o: &Combo) -> bool {
        self.mods == o.mods && self.key.eq_ignore_ascii_case(&o.key)
    }

    /// The key string of `hl.bind` and `hl.unbind`: "SUPER + SHIFT + 4", "Print".
    pub fn lua_keys(&self) -> String {
        let mods = self.hypr_mods();
        if mods.is_empty() {
            self.key.clone()
        } else {
            format!("{} + {}", mods.replace(' ', " + "), self.key)
        }
    }

    /// "SUPER SHIFT" (compositor order), "" for none.
    pub fn hypr_mods(&self) -> String {
        [SUPER, SHIFT, CTRL, ALT]
            .iter()
            .filter(|b| self.mods & **b != 0)
            .filter_map(|b| MODS.iter().find(|m| m.0 == *b).map(|m| m.1))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Symbol label: "⌃⌥⇧⌘Space", "⇧⌘4", "←".
    pub fn label(&self) -> String {
        let mut s: String = MODS
            .iter()
            .filter(|m| self.mods & m.0 != 0)
            .map(|m| m.2)
            .collect();
        s.push_str(&key_label(&self.key));
        s
    }

    /// A recorded key press: GDK modifier bits as [`CTRL`]/[`ALT`]/[`SHIFT`]/[`SUPER`] and the keyval name.
    /// None for a bare modifier, or a plain key without any modifier (except function/special keys).
    pub fn recorded(mods: u8, keyval_name: &str) -> Option<Self> {
        let k = keyval_name.trim();
        let modifier = [
            "Shift",
            "Control",
            "Alt",
            "Super",
            "Meta",
            "Hyper",
            "ISO_Level3",
            "Caps_Lock",
        ]
        .iter()
        .any(|m| k.starts_with(m));
        let lone_ok = k.starts_with("XF86")
            || ["Print", "Pause", "Scroll_Lock"].contains(&k)
            || (k.len() >= 2 && k.starts_with('F') && k[1..].bytes().all(|b| b.is_ascii_digit()));
        if k.is_empty() || modifier || (mods & (CTRL | ALT | SUPER) == 0 && !lone_ok) {
            return None;
        }
        Some(Self {
            mods: mods & (CTRL | ALT | SHIFT | SUPER),
            key: k.to_string(),
        })
    }
}

/// Display name of a key (xkb keysym name).
fn key_label(k: &str) -> String {
    let named = match k.to_ascii_lowercase().as_str() {
        "space" => "Space",
        "return" => "↩",
        "escape" => "⎋",
        "tab" => "⇥",
        "left" => "←",
        "right" => "→",
        "up" => "↑",
        "down" => "↓",
        "comma" => ",",
        "period" => ".",
        "equal" => "=",
        "minus" => "-",
        "slash" => "/",
        "backslash" => "\\",
        "semicolon" => ";",
        "apostrophe" => "'",
        "grave" => "`",
        "bracketleft" => "[",
        "bracketright" => "]",
        "backspace" => "⌫",
        "delete" => "⌦",
        "home" => "Home",
        "end" => "End",
        "prior" | "page_up" => "Page Up",
        "next" | "page_down" => "Page Down",
        "print" => "Print",
        "super_l" | "super_r" => "",
        "mouse:272" => "Drag",
        "mouse:273" => "Right-drag",
        "xf86audioraisevolume" => "Volume Up",
        "xf86audiolowervolume" => "Volume Down",
        "xf86audiomute" => "Mute",
        "xf86audiomicmute" => "Mic Mute",
        "xf86monbrightnessup" => "Brightness Up",
        "xf86monbrightnessdown" => "Brightness Down",
        "xf86audioplay" => "Play",
        "xf86audionext" => "Next Track",
        "xf86audioprev" => "Previous Track",
        _ => "",
    };
    if !named.is_empty() || k.eq_ignore_ascii_case("super_l") || k.eq_ignore_ascii_case("super_r") {
        named.to_string()
    } else if k.chars().count() == 1 {
        k.to_uppercase()
    } else {
        k.to_string()
    }
}

/// One `bind*` line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bind {
    /// Letters after `bind` ("", "el", "m", "rt").
    pub flags: String,
    pub combo: Combo,
    pub dispatcher: String,
    pub args: String,
}

impl Bind {
    /// `keyword` value / include right-hand side: "SUPER, Space, exec, wol-search".
    pub fn value(&self, combo: &Combo) -> String {
        let mut s = format!("{}, {}, {}", combo.hypr_mods(), combo.key, self.dispatcher);
        if !self.args.is_empty() {
            s.push_str(", ");
            s.push_str(&self.args);
        }
        s
    }

    /// The `hl.bind` call: the dispatcher as its `hl.dsp` function (the inverse of
    /// `theme::hypr::conf_binds`), the flag letters as the options table. A dispatcher without a
    /// Lua form runs through `hyprctl dispatch`.
    pub fn lua(&self, combo: &Combo) -> String {
        use crate::theme::hypr::lua_str;
        fn dir(a: &str) -> &str {
            match a {
                "l" => "left",
                "r" => "right",
                "u" => "up",
                "d" => "down",
                other => other,
            }
        }
        let num = |v: &str| {
            if v.parse::<i64>().is_ok() {
                v.to_string()
            } else {
                lua_str(v)
            }
        };
        let a = self.args.trim();
        let dsp = match (self.dispatcher.as_str(), a) {
            ("exec", cmd) => format!("hl.dsp.exec_cmd({})", lua_str(cmd)),
            ("?lua", call) => call.to_string(),
            ("submap", name) => format!("hl.dsp.submap({})", lua_str(name)),
            ("movefocus", d) => format!("hl.dsp.focus({{ direction = {} }})", lua_str(dir(d))),
            ("workspace", w) => format!("hl.dsp.focus({{ workspace = {} }})", num(w)),
            ("movewindow", "") => "hl.dsp.window.drag()".into(),
            ("movewindow", d) => {
                format!("hl.dsp.window.move({{ direction = {} }})", lua_str(dir(d)))
            }
            ("resizewindow", "") => "hl.dsp.window.resize()".into(),
            ("resizeactive", xy) => match xy.split_once(' ') {
                Some((x, y)) => format!(
                    "hl.dsp.window.resize({{ x = {}, y = {}, relative = true }})",
                    num(x.trim()),
                    num(y.trim())
                ),
                None => self.dispatch_lua(),
            },
            ("togglefloating", "") => "hl.dsp.window.float({ action = \"toggle\" })".into(),
            ("fullscreen", m) => format!(
                "hl.dsp.window.fullscreen({{ mode = {} }})",
                lua_str(if m == "1" { "maximized" } else { "fullscreen" })
            ),
            _ => self.dispatch_lua(),
        };
        let opts: Vec<String> = [
            ('e', "repeating"),
            ('l', "locked"),
            ('m', "mouse"),
            ('n', "non_consuming"),
            ('r', "release"),
            ('t', "transparent"),
        ]
        .iter()
        .filter(|(c, _)| self.flags.contains(*c))
        .map(|(_, o)| format!("{o} = true"))
        .collect();
        let mut s = format!("hl.bind({}, {dsp}", lua_str(&combo.lua_keys()));
        if !opts.is_empty() {
            s += &format!(", {{ {} }}", opts.join(", "));
        }
        s + ")"
    }

    /// `hyprctl dispatch <dispatcher> <args>` as an `hl.dsp.exec_cmd` (a dispatcher with no Lua form).
    fn dispatch_lua(&self) -> String {
        let cmd = format!("hyprctl dispatch {} {}", self.dispatcher, self.args);
        format!(
            "hl.dsp.exec_cmd({})",
            crate::theme::hypr::lua_str(cmd.trim())
        )
    }

    /// `hyprctl keyword` form: "bindel SUPER,Return,exec,foot".
    fn keyword(&self, combo: &Combo) -> String {
        let mut s = format!(
            "keyword bind{} {},{},{}",
            self.flags,
            combo.hypr_mods(),
            combo.key,
            self.dispatcher
        );
        if !self.args.is_empty() {
            s.push(',');
            s.push_str(&self.args);
        }
        s
    }
}

/// Global binds of bind lines (`bind = MODS, key, dispatcher, args`; none from inside a `submap`),
/// `$variables` substituted: the form `theme::hypr::conf_binds` lowers the Lua configuration to.
pub fn parse(conf: &str) -> Vec<Bind> {
    let mut vars: Vec<(String, String)> = Vec::new();
    let mut out = Vec::new();
    let mut global = true;
    for line in conf.lines().map(str::trim) {
        let Some((lhs, rhs)) = line.split_once('=') else {
            continue;
        };
        let lhs = lhs.trim();
        if lhs == "submap" {
            global = rhs.split('#').next().unwrap_or("").trim() == "reset";
            continue;
        }
        if !global {
            continue;
        }
        if let Some(name) = lhs.strip_prefix('$') {
            vars.push((format!("${name}"), rhs.trim().to_string()));
            vars.sort_by_key(|v| std::cmp::Reverse(v.0.len()));
            continue;
        }
        let Some(flags) = lhs.strip_prefix("bind") else {
            continue;
        };
        if !flags.bytes().all(|b| b.is_ascii_lowercase()) {
            continue;
        }
        let mut rhs = rhs.trim().to_string();
        for (k, v) in &vars {
            rhs = rhs.replace(k.as_str(), v);
        }
        let mut f = rhs.splitn(4, ',').map(str::trim);
        let (Some(mods), Some(key), Some(dispatcher)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let Some(combo) = Combo::parse(mods, key) else {
            continue;
        };
        out.push(Bind {
            flags: flags.into(),
            combo,
            dispatcher: dispatcher.into(),
            args: f.next().unwrap_or("").into(),
        });
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Group {
    Search,
    Overview,
    Windows,
    Tiling,
    Apps,
    /// ⌘C, ⌘V, ⌘Z…: the app in front gets its own key (`wol-app <verb>`).
    Commands,
    Screenshots,
    Accessibility,
    Media,
    System,
    Custom,
}

impl Group {
    pub const ALL: [Group; 11] = [
        Group::Search,
        Group::Overview,
        Group::Windows,
        Group::Tiling,
        Group::Apps,
        Group::Commands,
        Group::Screenshots,
        Group::Accessibility,
        Group::Media,
        Group::System,
        Group::Custom,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Group::Search => "Search",
            Group::Overview => "Overview",
            Group::Windows => "Windows",
            Group::Tiling => "Tiling",
            Group::Apps => "Apps",
            Group::Commands => "App commands",
            Group::Screenshots => "Screenshots",
            Group::Accessibility => "Accessibility",
            Group::Media => "Media and brightness",
            Group::System => "System",
            Group::Custom => "Custom shortcuts",
        }
    }
}

/// One row in Settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    /// Stable within one shipped conf: "exec wol-lock", "exec wol-lock#2", "custom-1".
    pub id: String,
    pub name: String,
    pub group: Group,
    pub default: Option<Combo>,
    /// Effective combo (None = turned off).
    pub combo: Option<Combo>,
    pub bind: Bind,
    /// Mouse drags, hardware media keys and release binds are shown but fixed.
    pub editable: bool,
}

/// Curated (name, group) for a shipped bind, None when the shipped conf gained a bind nobody named.
pub fn name_of(dispatcher: &str, args: &str) -> Option<(&'static str, Group)> {
    NAMES
        .iter()
        .find(|n| n.0 == dispatcher && n.1 == args)
        .map(|n| (n.2, n.3))
}

use Group as G;
/// (dispatcher, args, name, group) for every shipped bind (desktop conf + tiling set).
const NAMES: [(&str, &str, &str, Group); 105] = [
    ("exec", "wol-search", "Show Search", G::Search),
    ("exec", "foot", "Open Terminal", G::Apps),
    ("exec", "nautilus", "Open Files", G::Apps),
    ("exec", "wol-settings", "Open System Settings", G::Apps),
    (
        "exec",
        "wol shell toggle control-center",
        "Show Control Center",
        G::Accessibility,
    ),
    (
        "exec",
        "wol shell toggle notifications",
        "Show Notification Center",
        G::Accessibility,
    ),
    (
        "exec",
        "wol shell toggle session",
        "Show Shut Down options",
        G::System,
    ),
    ("exec", "wol-lock", "Lock Screen", G::System),
    (
        "exec",
        "wol theme toggle",
        "Switch between light and dark",
        G::System,
    ),
    ("exec", "wol shell toggle overview", "Overview", G::Overview),
    ("workspace", "e-1", "Move left a space", G::Overview),
    ("workspace", "e+1", "Move right a space", G::Overview),
    ("workspace", "1", "Switch to Desktop 1", G::Overview),
    ("workspace", "2", "Switch to Desktop 2", G::Overview),
    ("workspace", "3", "Switch to Desktop 3", G::Overview),
    ("workspace", "4", "Switch to Desktop 4", G::Overview),
    (
        "exec",
        "hyprctl dispatch movetoworkspace e-1",
        "Move window left a space",
        G::Overview,
    ),
    (
        "exec",
        "hyprctl dispatch movetoworkspace e+1",
        "Move window right a space",
        G::Overview,
    ),
    (
        "exec",
        "hyprctl dispatch movetoworkspace 1",
        "Move window to Desktop 1",
        G::Overview,
    ),
    (
        "exec",
        "hyprctl dispatch movetoworkspace 2",
        "Move window to Desktop 2",
        G::Overview,
    ),
    (
        "exec",
        "hyprctl dispatch movetoworkspace 3",
        "Move window to Desktop 3",
        G::Overview,
    ),
    (
        "exec",
        "hyprctl dispatch movetoworkspace 4",
        "Move window to Desktop 4",
        G::Overview,
    ),
    (
        "exec",
        "hyprctl dispatch movewindow mon:l",
        "Move window to the left display",
        G::Windows,
    ),
    (
        "exec",
        "hyprctl dispatch movewindow mon:r",
        "Move window to the right display",
        G::Windows,
    ),
    ("exec", "wol shell show switcher", "Switch apps", G::Windows),
    (
        "exec",
        "wol shell show switcher-prev",
        "Switch apps backwards",
        G::Windows,
    ),
    (
        "exec",
        "hyprctl wolsnap left",
        "Snap window to the left half",
        G::Windows,
    ),
    (
        "exec",
        "hyprctl wolsnap right",
        "Snap window to the right half",
        G::Windows,
    ),
    (
        "exec",
        "hyprctl wolsnap up",
        "Snap window up or zoom",
        G::Windows,
    ),
    (
        "exec",
        "hyprctl wolsnap down",
        "Snap window down or restore",
        G::Windows,
    ),
    ("exec", "wol-app quit", "Quit app", G::Windows),
    ("killactive", "", "Close window", G::Windows),
    ("exec", "wol-app hide", "Hide app", G::Windows),
    ("exec", "wol-app hide-others", "Hide other apps", G::Windows),
    ("exec", "wol-app minimise", "Minimize window", G::Windows),
    (
        "togglespecialworkspace",
        "hidden",
        "Show minimized windows",
        G::Windows,
    ),
    ("fullscreen", "0", "Enter full screen", G::Windows),
    ("togglefloating", "", "Float or tile window", G::Windows),
    ("movewindow", "", "Move window", G::Windows),
    ("resizewindow", "", "Resize window", G::Windows),
    (
        "exec",
        "wol-screenshot full",
        "Save picture of the screen",
        G::Screenshots,
    ),
    (
        "exec",
        "wol-screenshot region",
        "Save picture of selected area",
        G::Screenshots,
    ),
    (
        "exec",
        "wol-screenshot full --copy",
        "Copy picture of screen",
        G::Screenshots,
    ),
    (
        "exec",
        "wol-screenshot clip",
        "Copy picture of selected area",
        G::Screenshots,
    ),
    (
        "exec",
        "wol shell show screenshot",
        "Screenshot and recording options",
        G::Screenshots,
    ),
    (
        "exec",
        "wol-zoom toggle",
        "Turn zoom on or off",
        G::Accessibility,
    ),
    ("exec", "wol-zoom in", "Zoom in", G::Accessibility),
    ("exec", "wol-zoom out", "Zoom out", G::Accessibility),
    ("exec", "wol-volume up", "Volume up", G::Media),
    ("exec", "wol-volume down", "Volume down", G::Media),
    ("exec", "wol-volume mute", "Mute", G::Media),
    ("exec", "wol-volume micmute", "Mute microphone", G::Media),
    ("exec", "wol-backlight up", "Brightness up", G::Media),
    ("exec", "wol-backlight down", "Brightness down", G::Media),
    ("exec", "playerctl play-pause", "Play or pause", G::Media),
    ("exec", "playerctl next", "Next track", G::Media),
    ("exec", "playerctl previous", "Previous track", G::Media),
    ("movefocus", "l", "Focus the window on the left", G::Tiling),
    ("movefocus", "r", "Focus the window on the right", G::Tiling),
    ("movefocus", "u", "Focus the window above", G::Tiling),
    ("movefocus", "d", "Focus the window below", G::Tiling),
    ("movewindow", "l", "Move window left", G::Tiling),
    ("movewindow", "r", "Move window right", G::Tiling),
    ("movewindow", "u", "Move window up", G::Tiling),
    ("movewindow", "d", "Move window down", G::Tiling),
    ("resizeactive", "-40 0", "Make window narrower", G::Tiling),
    ("resizeactive", "40 0", "Make window wider", G::Tiling),
    ("resizeactive", "0 -40", "Make window shorter", G::Tiling),
    ("resizeactive", "0 40", "Make window taller", G::Tiling),
    ("layoutmsg", "togglesplit", "Rotate the split", G::Tiling),
    (
        "layoutmsg",
        "swapwithmaster",
        "Swap with the main window",
        G::Tiling,
    ),
    (
        "exec",
        "hyprctl switchxkblayout all next",
        "Next input source",
        G::System,
    ),
    ("pseudo", "", "Keep window size while tiled", G::Tiling),
    ("pin", "", "Keep window on top of all spaces", G::Windows),
    (
        "exec",
        "wol shell toggle force-quit",
        "Force Quit apps",
        G::System,
    ),
    (
        "exec",
        "wol shell toggle shortcuts",
        "Show keyboard shortcuts",
        G::System,
    ),
    (
        "exec",
        "wol shell toggle clipboard",
        "Show clipboard history",
        G::System,
    ),
    (
        "exec",
        "wol shell toggle themes",
        "Choose a theme",
        G::System,
    ),
    (
        "exec",
        "hyprpicker --autocopy --notify",
        "Pick a colour from the screen",
        G::System,
    ),
    (
        "exec",
        "wol toggle nightlight",
        "Turn Night Shift on or off",
        G::System,
    ),
    (
        "exec",
        "wol toggle dnd",
        "Turn Do Not Disturb on or off",
        G::System,
    ),
    (
        "exec",
        "wol toggle stay-awake",
        "Turn Stay Awake on or off",
        G::System,
    ),
    (
        "exec",
        "wol shell show menubar",
        "Move focus to the menu bar",
        G::System,
    ),
    (
        "exec",
        "wol shell show dock",
        "Move focus to the Dock",
        G::System,
    ),
    (
        "exec",
        "wol shell hide banners",
        "Dismiss notification banners",
        G::System,
    ),
    ("exec", "wol-app copy", "Copy", G::Commands),
    ("exec", "wol-app paste", "Paste", G::Commands),
    ("exec", "wol-app cut", "Cut", G::Commands),
    ("exec", "wol-app undo", "Undo", G::Commands),
    ("exec", "wol-app redo", "Redo", G::Commands),
    ("exec", "wol-app select-all", "Select All", G::Commands),
    ("exec", "wol-app save", "Save", G::Commands),
    ("exec", "wol-app open", "Open", G::Commands),
    ("exec", "wol-app print", "Print", G::Commands),
    ("exec", "wol-app find", "Find", G::Commands),
    ("exec", "wol-app find-next", "Find Next", G::Commands),
    ("exec", "wol-app new-tab", "New Tab", G::Commands),
    ("exec", "wol-app close", "Close tab or window", G::Commands),
    ("exec", "wol-app new", "New window", G::Commands),
    ("exec", "wol-app reload", "Reload", G::Commands),
    (
        "exec",
        "wol-app location",
        "Go to an address or location",
        G::Commands,
    ),
    ("exec", "wol-app zoom-in", "Zoom In", G::Commands),
    ("exec", "wol-app zoom-out", "Zoom Out", G::Commands),
    ("exec", "wol-app actual-size", "Actual Size", G::Commands),
    ("exec", "wol-app emoji", "Emoji & Symbols", G::Commands),
];

/// Global binds of a Lua configuration (`hyprland.lua`, `tiling.lua`).
pub fn parse_lua(lua: &str) -> Vec<Bind> {
    parse(&crate::theme::hypr::conf_binds(lua))
}

/// Shipped defaults (the desktop's hyprland.lua, then the tiling set), named and grouped, ids unique.
pub fn defaults(shipped: &str, tiling: &str) -> Vec<Shortcut> {
    let mut out: Vec<Shortcut> = Vec::new();
    let all = parse_lua(shipped)
        .into_iter()
        .map(|b| (b, false))
        .chain(parse_lua(tiling).into_iter().map(|b| (b, true)));
    for (b, tiles) in all {
        let base = format!("{} {}", b.dispatcher, b.args).trim().to_string();
        let n = out
            .iter()
            .filter(|s| s.id.split('#').next() == Some(&base))
            .count();
        let id = if n == 0 {
            base
        } else {
            format!("{base}#{}", n + 1)
        };
        let (name, group) = match name_of(&b.dispatcher, &b.args) {
            Some((n, g)) => (n.to_string(), if tiles { G::Tiling } else { g }),
            None if b.dispatcher == "exec" => (format!("Run {}", b.args), G::Apps),
            None => (
                format!("{} {}", b.dispatcher, b.args).trim().into(),
                G::Windows,
            ),
        };
        let editable =
            !b.flags.contains(['m', 'r']) && !b.combo.key.to_ascii_lowercase().starts_with("xf86");
        out.push(Shortcut {
            id,
            name,
            group: if tiles { G::Tiling } else { group },
            default: Some(b.combo.clone()),
            combo: Some(b.combo.clone()),
            bind: b,
            editable,
        });
    }
    out
}

/// Installed Lua files, else the repo copies.
pub fn load_defaults(read: impl Fn(&str) -> Option<String>) -> Vec<Shortcut> {
    defaults(
        &read(SHIPPED_PATH).unwrap_or_else(|| SHIPPED.into()),
        &read(TILING_PATH).unwrap_or_else(|| TILING.into()),
    )
}

/// A user shortcut that runs a command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Custom {
    pub name: String,
    pub combo: Combo,
    pub command: String,
}

/// What the user changed: per-id combo (None = off) and custom shortcuts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Overrides {
    pub changed: BTreeMap<String, Option<Combo>>,
    pub custom: Vec<Custom>,
}

/// Why an assignment was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The combo is taken by this shortcut (id, name); call again with `swap`.
    Conflict(String, String),
    /// The combo belongs to this custom shortcut (id, name) and the shortcut asking for it has
    /// no combo to hand over: a swap would leave the custom one keyless, and a custom shortcut
    /// without keys does not exist (it would be deleted). Change or remove the custom one first.
    CustomNeedsKeys(String, String),
    Unknown,
    Fixed,
    /// Custom shortcut without a name or with an unusable command.
    Invalid,
}

/// "custom-3" -> index 2.
fn custom_index(id: &str) -> Option<usize> {
    id.strip_prefix("custom-")?
        .parse::<usize>()
        .ok()?
        .checked_sub(1)
}

impl Overrides {
    pub fn parse(text: Option<&str>) -> Self {
        text.and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or_default()
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// Gives `id` the combo; a taken combo is refused unless `swap`, which hands the other shortcut `id`'s old combo.
    pub fn assign(
        &mut self,
        defaults: &[Shortcut],
        id: &str,
        combo: Option<Combo>,
        swap: bool,
    ) -> Result<(), Refused> {
        let custom = custom_index(id).filter(|i| *i < self.custom.len());
        match defaults.iter().find(|d| d.id == id) {
            Some(d) if !d.editable => return Err(Refused::Fixed),
            None if custom.is_none() => return Err(Refused::Unknown),
            _ => {}
        }
        let e = effective(defaults, self);
        let old = e.iter().find(|s| s.id == id).and_then(|s| s.combo.clone());
        if let Some(k) = &combo
            && let Some(other) = conflict(&e, k, id)
        {
            if old.is_none() && custom_index(&other.id).is_some_and(|i| i < self.custom.len()) {
                // QA-SET-35: `set(custom, None)` removes the entry. Refuse before anything
                // changes, swap or not (the Swap button is only a second try of the same call).
                return Err(Refused::CustomNeedsKeys(
                    other.id.clone(),
                    other.name.clone(),
                ));
            }
            if !swap {
                return Err(Refused::Conflict(other.id.clone(), other.name.clone()));
            }
            let other = other.id.clone();
            self.set(defaults, &other, old);
        }
        self.set(defaults, id, combo);
        Ok(())
    }

    fn set(&mut self, defaults: &[Shortcut], id: &str, combo: Option<Combo>) {
        if let Some(i) = custom_index(id).filter(|i| *i < self.custom.len()) {
            match combo {
                Some(k) => self.custom[i].combo = k,
                None => {
                    self.custom.remove(i);
                }
            }
            return;
        }
        let default = defaults
            .iter()
            .find(|d| d.id == id)
            .and_then(|d| d.default.clone());
        let same = match (&combo, &default) {
            (Some(a), Some(b)) => a.same(b),
            (None, None) => true,
            _ => false,
        };
        if same {
            self.changed.remove(id);
        } else {
            self.changed.insert(id.to_string(), combo);
        }
    }

    /// Back to the shipped combo.
    pub fn reset(&mut self, defaults: &[Shortcut], id: &str) {
        let _ = defaults;
        self.changed.remove(id);
    }

    /// Restore Defaults: shipped combos for every built-in shortcut; custom shortcuts stay.
    pub fn restore_defaults(&mut self) {
        self.changed.clear();
    }

    /// Adds a custom shortcut; refused when the combo is taken or the command is unusable.
    pub fn add_custom(&mut self, defaults: &[Shortcut], c: Custom) -> Result<(), Refused> {
        if c.name.trim().is_empty() || !crate::hyprprefs::is_valid_command(&c.command) {
            return Err(Refused::Invalid);
        }
        if let Some(o) = conflict(&effective(defaults, self), &c.combo, "") {
            return Err(Refused::Conflict(o.id.clone(), o.name.clone()));
        }
        self.custom.push(Custom {
            command: c.command.trim().into(),
            ..c
        });
        Ok(())
    }

    pub fn remove_custom(&mut self, id: &str) {
        if let Some(i) = custom_index(id).filter(|i| *i < self.custom.len()) {
            self.custom.remove(i);
        }
    }
}

/// Defaults with overrides applied, then custom shortcuts ("custom-N").
pub fn effective(defaults: &[Shortcut], o: &Overrides) -> Vec<Shortcut> {
    let mut v: Vec<Shortcut> = defaults
        .iter()
        .map(|d| Shortcut {
            combo: match o.changed.get(&d.id) {
                Some(c) if d.editable => c.clone(),
                _ => d.default.clone(),
            },
            ..d.clone()
        })
        .collect();
    for (i, c) in o.custom.iter().enumerate() {
        v.push(Shortcut {
            id: format!("custom-{}", i + 1),
            name: c.name.clone(),
            group: G::Custom,
            default: None,
            combo: Some(c.combo.clone()),
            bind: Bind {
                flags: String::new(),
                combo: c.combo.clone(),
                dispatcher: "exec".into(),
                args: c.command.clone(),
            },
            editable: true,
        });
    }
    v
}

/// The shortcut already using `combo` (other than `except`).
pub fn conflict<'a>(list: &'a [Shortcut], combo: &Combo, except: &str) -> Option<&'a Shortcut> {
    list.iter()
        .find(|s| s.id != except && s.combo.as_ref().is_some_and(|k| k.same(combo)))
}

/// Binds active in the session: tiling ones only while tiling is on.
pub fn active(list: &[Shortcut], tiling: bool) -> Vec<(Combo, Bind)> {
    list.iter()
        .filter(|s| tiling || s.group != G::Tiling)
        .filter_map(|s| Some((s.combo.clone()?, s.bind.clone())))
        .collect()
}

/// The Lua file: unbind changed shipped defaults, bind their new combos, tiling set while on, custom ones.
pub fn include(list: &[Shortcut], tiling: bool) -> String {
    use crate::theme::hypr::lua_str;
    let mut s = String::from(
        "-- Written by WolOS System Settings (Keyboard > Shortcuts). Change shortcuts there.\n",
    );
    let shipped = |x: &&Shortcut| x.group != G::Tiling && x.group != G::Custom;
    let moved: Vec<&Shortcut> = list
        .iter()
        .filter(shipped)
        .filter(|x| match (&x.combo, &x.default) {
            (Some(a), Some(b)) => !a.same(b),
            (None, Some(_)) => true,
            _ => false,
        })
        .collect();
    for x in &moved {
        if let Some(d) = &x.default {
            s.push_str(&format!("hl.unbind({})\n", lua_str(&d.lua_keys())));
        }
    }
    let bound = moved.iter().copied().chain(
        list.iter()
            .filter(|x| (tiling && x.group == G::Tiling) || x.group == G::Custom),
    );
    for x in bound {
        if let Some(k) = &x.combo {
            s.push_str(&x.bind.lua(k));
            s.push('\n');
        }
    }
    s
}

/// hyprctl batch moving the session from `old` to `new` active binds: unbinds first, then binds.
pub fn live(old: &[(Combo, Bind)], new: &[(Combo, Bind)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (k, _) in old.iter().filter(|x| !new.contains(x)) {
        let u = format!("keyword unbind {},{}", k.hypr_mods(), k.key);
        if !out.contains(&u) {
            out.push(u);
        }
    }
    out.extend(
        new.iter()
            .filter(|x| !old.contains(x))
            .map(|(k, b)| b.keyword(k)),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(mods: &str, key: &str) -> Combo {
        Combo::parse(mods, key).unwrap()
    }

    #[test]
    fn combos_parse_and_label() {
        for (mods, key, label, hypr) in [
            ("SUPER", "Space", "⌘Space", "SUPER"),
            ("SUPER SHIFT", "4", "⇧⌘4", "SUPER SHIFT"),
            ("CTRL SUPER", "Q", "⌃⌘Q", "SUPER CTRL"),
            ("SUPER_ALT", "H", "⌥⌘H", "SUPER ALT"),
            ("", "Print", "Print", ""),
            ("CTRL", "Left", "⌃←", "CTRL"),
            ("SUPER", "comma", "⌘,", "SUPER"),
            ("SUPER", "Return", "⌘↩", "SUPER"),
            ("SUPER", "Escape", "⌘⎋", "SUPER"),
            ("SUPER SHIFT", "Tab", "⇧⌘⇥", "SUPER SHIFT"),
            ("", "XF86AudioRaiseVolume", "Volume Up", ""),
            ("", "XF86AudioMicMute", "Mic Mute", ""),
            ("SUPER ALT", "equal", "⌥⌘=", "SUPER ALT"),
        ] {
            let x = c(mods, key);
            assert_eq!(
                (x.label().as_str(), x.hypr_mods().as_str()),
                (label, hypr),
                "{mods} {key}"
            );
        }
        assert_eq!(Combo::parse("HYPER", "x"), None);
        assert_eq!(Combo::parse("SUPER", ""), None);
        assert!(c("SUPER", "q").same(&c("SUPER", "Q")));
        assert!(!c("SUPER", "q").same(&c("SUPER SHIFT", "Q")));
    }

    #[test]
    fn recorded_key_presses() {
        assert_eq!(
            Combo::recorded(SUPER | SHIFT, "4"),
            Some(c("SUPER SHIFT", "4"))
        );
        assert_eq!(Combo::recorded(SUPER, "Super_L"), None, "bare modifier");
        assert_eq!(
            Combo::recorded(0, "a"),
            None,
            "plain letter would steal typing"
        );
        assert_eq!(Combo::recorded(0, "F5"), Some(c("", "F5")));
        assert_eq!(Combo::recorded(CTRL, "space"), Some(c("CTRL", "space")));
    }

    /// Every bind in the shipped Lua configuration is listed, named by the curated table (no
    /// fallback names).
    #[test]
    fn shipped_lua_lists_every_bind() {
        // Global bind lines (a submap's own binds are not shortcuts).
        let lowered = crate::theme::hypr::conf_binds(SHIPPED);
        assert!(
            !lowered.contains('?'),
            "a bind form conf_binds does not read"
        );
        let mut global = true;
        let lines = lowered
            .lines()
            .map(str::trim)
            .filter(|l| {
                if let Some(r) = l.strip_prefix("submap") {
                    global = r.trim_start_matches([' ', '=']).trim() == "reset";
                    return false;
                }
                global && l.starts_with("bind")
            })
            .count();
        let binds = parse_lua(SHIPPED);
        assert_eq!(binds.len(), lines, "a bind line failed to parse");
        let calls = SHIPPED
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("bind(") || l.starts_with("hl.bind("))
            .filter(|l| !l.starts_with("hl.bind(keys,"))
            .count();
        // The submap's own Esc bind is the one call that is not a shortcut (and the `bind` helper's
        // own body binds nothing).
        assert_eq!(calls, lines + 1, "every bind call is read");
        let d = defaults(SHIPPED, TILING);
        let tiling = TILING
            .lines()
            .filter(|l| l.trim_start().starts_with("hl.bind("))
            .count();
        assert_eq!(d.len(), lines + tiling);
        for b in binds.iter().chain(parse_lua(TILING).iter()) {
            assert!(
                name_of(&b.dispatcher, &b.args).is_some(),
                "unnamed shipped bind: {} {}",
                b.dispatcher,
                b.args
            );
        }
        let mut ids: Vec<&str> = d.iter().map(|s| s.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), d.len(), "ids unique");
        let spot = d.iter().find(|s| s.id == "exec wol-search").unwrap();
        assert_eq!(
            (
                spot.name.as_str(),
                spot.group,
                spot.combo.as_ref().unwrap().label()
            ),
            ("Show Search", Group::Search, "⌘Space".to_string())
        );
        assert!(d.iter().filter(|s| s.group == Group::Tiling).count() >= 8);
        // The shell's word for the warm display is Night Shift; the mic key has a name, not a keysym.
        let night = d
            .iter()
            .find(|s| s.id == "exec wol toggle nightlight")
            .unwrap();
        assert_eq!(night.name, "Turn Night Shift on or off");
        let mic = d
            .iter()
            .find(|s| s.id == "exec wol-volume micmute")
            .unwrap();
        assert_eq!(mic.combo.as_ref().unwrap().label(), "Mic Mute");
        assert!(
            d.iter()
                .filter_map(|s| s.combo.as_ref())
                .all(|k| !k.label().starts_with("XF86")),
            "every media key has a label"
        );
        let drag = d.iter().find(|s| s.bind.flags == "m").unwrap();
        assert!(!drag.editable);
        let vol = d
            .iter()
            .find(|s| s.bind.combo.key.starts_with("XF86"))
            .unwrap();
        assert!(!vol.editable);
        for g in [
            Group::Screenshots,
            Group::Overview,
            Group::Windows,
            Group::Apps,
            Group::Commands,
            Group::Accessibility,
        ] {
            assert!(d.iter().any(|s| s.group == g), "{g:?} empty");
        }
    }

    /// `Bind::lua` is the inverse of `conf_binds`: every shipped bind written back as Lua reads as
    /// the same bind, on its own key or another.
    #[test]
    fn every_shipped_bind_renders_as_lua() {
        let d = defaults(SHIPPED, TILING);
        let other = c("SUPER CTRL ALT", "K");
        for s in &d {
            let k = s.default.clone().unwrap();
            for combo in [&k, &other] {
                let lua = s.bind.lua(combo);
                let back = parse_lua(&lua);
                let want = Bind {
                    combo: combo.clone(),
                    ..s.bind.clone()
                };
                let sorted = |f: &str| {
                    let mut v: Vec<char> = f.chars().collect();
                    v.sort_unstable();
                    v
                };
                assert_eq!(back.len(), 1, "{lua}");
                assert_eq!(
                    (
                        sorted(&back[0].flags),
                        &back[0].combo,
                        &back[0].dispatcher,
                        &back[0].args
                    ),
                    (
                        sorted(&want.flags),
                        &want.combo,
                        &want.dispatcher,
                        &want.args
                    ),
                    "{lua}"
                );
            }
        }
        let b = Bind {
            flags: "el".into(),
            combo: c("", "XF86AudioRaiseVolume"),
            dispatcher: "exec".into(),
            args: "wol-volume \"up\"".into(),
        };
        assert_eq!(
            b.lua(&b.combo),
            "hl.bind(\"XF86AudioRaiseVolume\", hl.dsp.exec_cmd(\"wol-volume \\\"up\\\"\"), { repeating = true, locked = true })"
        );
        let odd = Bind {
            flags: String::new(),
            combo: c("SUPER", "P"),
            dispatcher: "pin".into(),
            args: String::new(),
        };
        assert_eq!(
            odd.lua(&odd.combo),
            "hl.bind(\"SUPER + P\", hl.dsp.exec_cmd(\"hyprctl dispatch pin\"))"
        );
    }

    #[test]
    fn parse_substitutes_variables_and_flags() {
        let conf = "$mod = SUPER\nbind = $mod SHIFT, 4, exec, wol-screenshot region\nbindel = , XF86AudioRaiseVolume, exec, wol-volume up\nbindm = $mod, mouse:272, movewindow\n# bind = x\nbind = $mod, A, exec, sh -c 'a, b'\n";
        let b = parse(conf);
        assert_eq!(b.len(), 4);
        assert_eq!(b[0].combo, c("SUPER SHIFT", "4"));
        assert_eq!(
            (b[0].dispatcher.as_str(), b[0].args.as_str()),
            ("exec", "wol-screenshot region")
        );
        assert_eq!(b[1].flags, "el");
        assert_eq!((b[2].flags.as_str(), b[2].args.as_str()), ("m", ""));
        assert_eq!(b[3].args, "sh -c 'a, b'", "args keep their commas");
        assert_eq!(
            b[0].value(&c("SUPER ALT", "4")),
            "SUPER ALT, 4, exec, wol-screenshot region"
        );
    }

    /// QA-SET-35: a built-in row turned Off, a custom shortcut on a combo, record that combo on
    /// the Off row and press Swap: the custom shortcut used to be deleted (`set(custom, None)`).
    #[test]
    fn swap_from_off_row_keeps_custom() {
        let d = defaults(SHIPPED, TILING);
        let mut o = Overrides::default();
        let off = "exec wol-search";
        o.assign(&d, off, None, false).unwrap();
        o.add_custom(
            &d,
            Custom {
                name: "Bee".into(),
                combo: c("SUPER", "B"),
                command: "foot".into(),
            },
        )
        .unwrap();
        let before = o.clone();
        for swap in [false, true] {
            match o.assign(&d, off, Some(c("SUPER", "B")), swap) {
                Err(Refused::CustomNeedsKeys(id, name)) => {
                    assert_eq!((id.as_str(), name.as_str()), ("custom-1", "Bee"))
                }
                r => panic!("swap={swap}: {r:?}"),
            }
            assert_eq!(o, before, "refusal changes nothing");
        }
        assert_eq!(o.custom.len(), 1);
        // A row that has a combo still swaps with a custom shortcut (it hands its combo over).
        o.assign(&d, "exec foot", Some(c("SUPER", "B")), true)
            .unwrap();
        assert_eq!(o.custom.len(), 1);
        assert_eq!(o.custom[0].combo, c("SUPER", "Return"));
    }

    /// QA-ACC-7: Control Center and Notification Center open from the keyboard, and the sheet lists
    /// both under Accessibility (they used to have only the menu bar's pointer path).
    #[test]
    fn both_panels_have_a_key() {
        let d = defaults(SHIPPED, TILING);
        for (args, name, combo) in [
            (
                "wol shell toggle control-center",
                "Show Control Center",
                c("CTRL SUPER", "C"),
            ),
            (
                "wol shell toggle notifications",
                "Show Notification Center",
                c("CTRL SUPER", "A"),
            ),
        ] {
            let s = d
                .iter()
                .find(|s| s.id == format!("exec {args}"))
                .unwrap_or_else(|| panic!("no shipped bind for {args}"));
            assert_eq!((s.name.as_str(), s.group), (name, Group::Accessibility));
            assert_eq!(s.combo.as_ref(), Some(&combo));
            assert!(s.editable);
        }
    }

    /// Snap: ⌘ and the four arrows reach wol-hypr's `wolsnap`, named on the
    /// sheet, and ⌘↑ is no longer the Overview (⌃↑ and the swipe are).
    #[test]
    fn super_arrows_snap_windows_and_control_up_stays_the_overview() {
        let d = defaults(SHIPPED, TILING);
        for (key, dir) in [
            ("Left", "left"),
            ("Right", "right"),
            ("Up", "up"),
            ("Down", "down"),
        ] {
            let s = d
                .iter()
                .find(|s| s.id == format!("exec hyprctl wolsnap {dir}"))
                .unwrap_or_else(|| panic!("no snap bind for {dir}"));
            assert_eq!(s.combo, Some(c("SUPER", key)), "{dir}");
            assert!(s.name.starts_with("Snap window"), "{}", s.name);
            assert_eq!(s.group, Group::Windows);
        }
        let overview = d
            .iter()
            .find(|s| s.id == "exec wol shell toggle overview")
            .unwrap();
        assert_eq!(overview.combo, Some(c("CTRL", "Up")));
        assert!(
            d.iter().all(|s| s.id != "exec wol shell toggle overview#2"),
            "one Overview key"
        );
    }

    /// QA-PWR-11: keys exist to send the focused window to another space and display, and the
    /// sheet names them (a shipped bind nobody named is listed as its raw command).
    #[test]
    fn shipped_binds_move_a_window_to_another_space_and_display() {
        let d = defaults(SHIPPED, TILING);
        let name = |args: &str| {
            d.iter()
                .find(|s| s.id == format!("exec {args}"))
                .unwrap_or_else(|| panic!("no shipped bind for {args}"))
                .name
                .clone()
        };
        assert_eq!(
            name("hyprctl dispatch movetoworkspace 3"),
            "Move window to Desktop 3"
        );
        assert_eq!(
            name("hyprctl dispatch movetoworkspace e+1"),
            "Move window right a space"
        );
        assert_eq!(
            name("hyprctl dispatch movewindow mon:r"),
            "Move window to the right display"
        );
        let combo = |args: &str| {
            d.iter()
                .find(|s| s.id == format!("exec {args}"))
                .and_then(|s| s.combo.clone())
                .unwrap()
        };
        assert_eq!(
            combo("hyprctl dispatch movetoworkspace 1"),
            c("CTRL ALT SHIFT", "1")
        );
        assert_eq!(
            combo("hyprctl dispatch movewindow mon:l"),
            c("CTRL ALT SUPER", "Left")
        );
        assert!(
            d.iter().all(|s| !s.name.starts_with("Run hyprctl")),
            "every hyprctl bind is named"
        );
    }

    #[test]
    fn conflicts_and_swap() {
        let d = defaults(SHIPPED, TILING);
        let mut o = Overrides::default();
        let spot = "exec wol-search";
        let term = "exec foot";
        let taken = c("SUPER", "Return");
        match o.assign(&d, spot, Some(taken.clone()), false) {
            Err(Refused::Conflict(id, name)) => {
                assert_eq!((id.as_str(), name.as_str()), (term, "Open Terminal"))
            }
            r => panic!("{r:?}"),
        }
        assert_eq!(o, Overrides::default(), "refusal changes nothing");
        o.assign(&d, spot, Some(taken.clone()), true).unwrap();
        let e = effective(&d, &o);
        let get = |id: &str| e.iter().find(|s| s.id == id).unwrap().combo.clone();
        assert_eq!(get(spot), Some(taken));
        assert_eq!(get(term), Some(c("SUPER", "Space")), "swapped");
        assert!(conflict(&e, &c("SUPER", "Space"), term).is_none());
        assert_eq!(
            conflict(&e, &c("SUPER", "space"), spot).unwrap().id,
            term,
            "case-insensitive"
        );
        o.reset(&d, spot);
        assert_eq!(
            effective(&d, &o)
                .iter()
                .find(|s| s.id == spot)
                .unwrap()
                .combo,
            Some(c("SUPER", "Space"))
        );
        o.assign(&d, spot, None, false).unwrap();
        assert_eq!(
            effective(&d, &o)
                .iter()
                .find(|s| s.id == spot)
                .unwrap()
                .combo,
            None,
            "turned off"
        );
        let drag = d.iter().find(|s| !s.editable).unwrap().id.clone();
        assert_eq!(o.assign(&d, &drag, None, false), Err(Refused::Fixed));
        assert_eq!(o.assign(&d, "nope", None, false), Err(Refused::Unknown));
        o.restore_defaults();
        assert!(o.changed.is_empty());
    }

    #[test]
    fn custom_shortcuts() {
        let d = defaults(SHIPPED, TILING);
        let mut o = Overrides::default();
        let htop = Custom {
            name: "Activity".into(),
            combo: c("SUPER CTRL", "U"),
            command: "foot -e htop".into(),
        };
        o.add_custom(&d, htop.clone()).unwrap();
        assert!(matches!(
            o.add_custom(
                &d,
                Custom {
                    combo: c("SUPER", "Space"),
                    ..htop.clone()
                }
            ),
            Err(Refused::Conflict(..))
        ));
        assert!(
            o.add_custom(
                &d,
                Custom {
                    command: "a; b".into(),
                    combo: c("SUPER CTRL", "Y"),
                    ..htop.clone()
                }
            )
            .is_err()
        );
        let e = effective(&d, &o);
        let last = e.last().unwrap();
        assert_eq!(
            (last.id.as_str(), last.group, last.bind.args.as_str()),
            ("custom-1", Group::Custom, "foot -e htop")
        );
        o.remove_custom("custom-1");
        assert!(o.custom.is_empty());
        assert_eq!(Overrides::parse(Some(&o.to_json())), o);
    }

    /// Simulates the compositor: shipped binds, then the Lua file's `hl.unbind`/`hl.bind` calls in
    /// order, equals `active`.
    #[test]
    fn include_round_trip() {
        let d = defaults(SHIPPED, TILING);
        let mut o = Overrides::default();
        o.assign(&d, "exec wol-search", Some(c("SUPER ALT", "Space")), false)
            .unwrap();
        o.assign(&d, "exec wol-settings", None, false).unwrap();
        o.add_custom(
            &d,
            Custom {
                name: "Top".into(),
                combo: c("SUPER CTRL", "U"),
                command: "foot -e top".into(),
            },
        )
        .unwrap();
        for tiling in [false, true] {
            let e = effective(&d, &o);
            let inc = include(&e, tiling);
            let mut session: Vec<(Combo, String, String)> = parse_lua(SHIPPED)
                .into_iter()
                .map(|b| (b.combo.clone(), b.dispatcher, b.args))
                .collect();
            for l in inc.lines() {
                if let Some(v) = l
                    .strip_prefix("hl.unbind(\"")
                    .and_then(|v| v.strip_suffix("\")"))
                {
                    let (m, k) = v.rsplit_once(" + ").unwrap_or(("", v));
                    let u = c(m, k);
                    session.retain(|s| !s.0.same(&u));
                } else if l.starts_with("hl.bind(") {
                    let b = &parse_lua(l)[0];
                    session.push((b.combo.clone(), b.dispatcher.clone(), b.args.clone()));
                } else {
                    assert!(l.starts_with("-- "), "{l}");
                }
            }
            let mut got: Vec<String> = session
                .iter()
                .map(|s| format!("{} {} {}", s.0.label(), s.1, s.2))
                .collect();
            let mut want: Vec<String> = active(&e, tiling)
                .iter()
                .map(|(k, b)| format!("{} {} {}", k.label(), b.dispatcher, b.args))
                .collect();
            got.sort();
            want.sort();
            assert_eq!(got, want, "tiling={tiling}\n{inc}");
        }
        assert!(
            include(&effective(&d, &Overrides::default()), false)
                .lines()
                .all(|l| l.starts_with("--")),
            "defaults write nothing"
        );
    }

    #[test]
    fn live_unbinds_then_binds() {
        let d = defaults(SHIPPED, TILING);
        let before = active(&effective(&d, &Overrides::default()), false);
        let mut o = Overrides::default();
        o.assign(&d, "exec wol-search", Some(c("SUPER", "Return")), true)
            .unwrap();
        let after = active(&effective(&d, &o), false);
        assert_eq!(
            live(&before, &after),
            [
                "keyword unbind SUPER,Space",
                "keyword unbind SUPER,Return",
                "keyword bind SUPER,Return,exec,wol-search",
                "keyword bind SUPER,Space,exec,foot",
            ]
        );
        assert!(live(&before, &before).is_empty());
        let tiled = active(&effective(&d, &Overrides::default()), true);
        let cmds = live(&before, &tiled);
        assert!(cmds.iter().all(|c| c.starts_with("keyword bind")) && cmds.len() >= 8);
        assert!(
            cmds.contains(&"keyword binde SUPER CTRL,left,resizeactive,-40 0".to_string()),
            "{cmds:?}"
        );
    }
}
