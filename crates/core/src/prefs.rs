//! User desktop preferences written by Settings: shell prefs (menu bar, Dock), Hyprland keyboard/display
//! fragments, hypridle config. Pure render/parse; callers write files and reload.
use std::collections::HashMap;

/// `$XDG_CONFIG_HOME/wolos/shell.json`; the shell rereads it on `reload` (wol-shell.sock).
pub const SHELL_FILE: &str = "wolos/shell.json";
/// `$XDG_CONFIG_HOME/hypr/wolos/<name>.conf`, sourced by the user hyprland.conf.
pub const HYPR_DIR: &str = "hypr/wolos";
pub const HYPR_USER: &str = "hypr/hyprland.conf";
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
    /// Show recently used apps in the Dock.
    pub dock_show_recents: bool,
    /// Minimize effect speed as a percentage of its time scale (50 = 0.5).
    pub dock_minimize_speed_pct: u8,
    /// Spotlight: milliseconds a slower provider (the file index) waits after a keystroke before
    /// its rows may be merged in (`priority_index_delay_ms`).
    pub priority_index_delay_ms: u32,
    /// Spotlight: slower providers answering later than this after a keystroke are left out.
    pub spotlight_slow_timeout_ms: u32,
    /// Settings > Control Center: which optional modules show.
    pub control_center: ControlCenterPrefs,
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
            dock_show_recents: true,
            dock_minimize_speed_pct: 50,
            priority_index_delay_ms: 0,
            spotlight_slow_timeout_ms: 400,
            control_center: ControlCenterPrefs::default(),
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
    /// Minimize effect time scale, 0.1 to 2.0 (1.0 = the stock duration).
    pub fn minimize_time_scale(&self) -> f64 {
        f64::from(self.dock_minimize_speed_pct.clamp(10, 200)) / 100.0
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
    pub fn to_json(self) -> String {
        serde_json::to_string_pretty(&self).unwrap_or_default() + "\n"
    }
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

/// `input { }` block for `keyboard.conf`. None for an unknown layout.
pub fn keyboard_conf(layout: &str, repeat: usize) -> Option<String> {
    let (_, rate, delay) = REPEATS.get(repeat)?;
    let (kb, variant) = match layout.split_once('(') {
        Some((l, v)) => (l, v.trim_end_matches(')')),
        None => (layout, ""),
    };
    LAYOUTS.iter().any(|l| l.0 == layout).then(|| {
        format!(
            "# Written by WolOS Settings.\ninput {{\n    kb_layout = {kb}\n    kb_variant = {variant}\n    repeat_rate = {rate}\n    repeat_delay = {delay}\n}}\n"
        )
    })
}

/// Setup assistant `~/.config/wol/setup.json` (ADR 0004) -> `keyboard.conf`, normal repeat. None if unknown.
pub fn setup_keyboard(setup_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(setup_json).ok()?;
    keyboard_conf(v.get("keymap")?.as_str()?, 1)
}

/// Setup assistant choices, relative to `$XDG_CONFIG_HOME`; renamed to `SETUP_APPLIED` once applied.
pub const SETUP_FILE: &str = "wol/setup.json";
pub const SETUP_APPLIED: &str = "wol/setup.applied.json";

fn kv(text: &str) -> HashMap<&str, &str> {
    crate::system::pairs(text, '=')
}

/// (layout index, repeat index) from `keyboard.conf`; defaults (U.S., Normal).
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

/// `monitor = <rule>` lines for `displays.conf`, one per monitor (later rule for the same name wins).
pub fn displays_conf(current: Option<&str>, rule: &str) -> String {
    let name = rule.split(',').next().unwrap_or("");
    let mut out = String::from("# Written by WolOS Settings.\n");
    for l in current.unwrap_or("").lines() {
        if let Some(r) = l.trim().strip_prefix("monitor = ")
            && r.split(',').next() != Some(name)
        {
            out.push_str(&format!("monitor = {r}\n"));
        }
    }
    out.push_str(&format!("monitor = {rule}\n"));
    out
}

/// User hyprland.conf with a `source` line for the fragments; None when already present.
pub fn ensure_source(user_conf: Option<&str>, fragments_glob: &str) -> Option<String> {
    let line = format!("source = {fragments_glob}");
    let cur = user_conf.unwrap_or("source = /usr/share/wolos/hypr/hyprland.conf\n");
    if cur.lines().any(|l| l.trim() == line) {
        return None;
    }
    let mut s = cur.trim_end_matches('\n').to_string();
    s.push_str(&format!(
        "\n# WolOS Settings (keyboard, displays).\n{line}\n"
    ));
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
pub fn hypridle_conf(lock_secs: u32, lock_on_sleep: bool) -> String {
    let mut s = String::from(
        "# Written by WolOS Settings.\ngeneral {\n    lock_cmd = pgrep -x wol-lock || wol-lock\n",
    );
    if lock_on_sleep {
        s.push_str("    before_sleep_cmd = pgrep -x wol-lock || wol-lock\n");
    }
    s.push_str("    after_sleep_cmd = hyprctl dispatch dpms on\n}\n");
    if lock_secs > 0 {
        s.push_str(&format!(
            "listener {{\n    timeout = {lock_secs}\n    on-timeout = pgrep -x wol-lock || wol-lock\n}}\nlistener {{\n    timeout = {}\n    on-timeout = hyprctl dispatch dpms off\n    on-resume = hyprctl dispatch dpms on\n}}\n",
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
    fn spotlight_keys_default() {
        let d = ShellPrefs::default();
        assert_eq!(d.priority_index_delay_ms, 0);
        assert_eq!(d.spotlight_slow_timeout_ms, 400);
        let p = ShellPrefs::parse(Some(r#"{"priority_index_delay_ms":50}"#));
        assert_eq!(p.priority_index_delay_ms, 50);
    }

    #[test]
    fn dock_keys_default_and_clamp() {
        let d = ShellPrefs::default();
        assert_eq!(d.dock_autohide_delay_ms, 0);
        assert_eq!(d.dock_tile_size, 56);
        assert!(d.dock_show_recents);
        assert_eq!(d.dock_minimize_speed_pct, 50, "time scale 0.5");
        assert_eq!(d.minimize_time_scale(), 0.5);
        let old = ShellPrefs::parse(Some(r#"{"dock_autohide":true}"#));
        assert_eq!(old.dock_tile_size, 56);
        let wild = ShellPrefs::parse(Some(
            r#"{"dock_tile_size":4000,"dock_autohide_delay_ms":999999,"dock_minimize_speed_pct":0}"#,
        ));
        assert_eq!(wild.tile_size(), TILE_SIZE.1);
        assert_eq!(wild.autohide_delay_ms(), AUTOHIDE_DELAY_MAX_MS);
        assert_eq!(wild.minimize_time_scale(), 0.1);
    }

    #[test]
    fn control_center_modules_persist_and_default() {
        let old = ShellPrefs::parse(Some(r#"{"show_date":false}"#));
        assert_eq!(old.control_center, ControlCenterPrefs::default());
        let d = ControlCenterPrefs::default();
        assert!(d.now_playing && d.battery && d.screenshot && d.record && d.night_shift);
        assert!(
            !d.keyboard_brightness && !d.accessibility,
            "opt-in like macOS"
        );
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
            let c = keyboard_conf(LAYOUTS[l].0, r).unwrap();
            assert_eq!(parse_keyboard(Some(&c)), (l, r), "{c}");
        }
        assert!(
            keyboard_conf("us(intl)", 1)
                .unwrap()
                .contains("kb_variant = intl\n")
        );
        assert_eq!(keyboard_conf("xx; exec", 1), None);
        assert_eq!(keyboard_conf("us", 9), None);
        assert_eq!(parse_keyboard(None), (0, 1));
    }

    #[test]
    fn displays_and_source() {
        let a = displays_conf(None, "eDP-1,1920x1080@60.00,auto,1");
        let b = displays_conf(Some(&a), "HDMI-A-1,2560x1440@60.00,auto,1");
        let c = displays_conf(Some(&b), "eDP-1,1920x1080@60.00,auto,1.5");
        assert_eq!(c.matches("monitor = ").count(), 2);
        assert!(c.ends_with("monitor = eDP-1,1920x1080@60.00,auto,1.5\n"));
        let s = ensure_source(None, "/h/.config/hypr/wolos/*.conf").unwrap();
        assert!(s.starts_with("source = /usr/share/wolos/hypr/hyprland.conf\n"));
        assert_eq!(
            ensure_source(Some(&s), "/h/.config/hypr/wolos/*.conf"),
            None
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
        // Battery display off and sleep (power::policy) whatever the lock setting, Never too.
        for secs in [0, 300] {
            let c = hypridle_conf(secs, false);
            assert!(
                c.ends_with(&crate::power::policy::hypridle_listeners()),
                "{c}"
            );
        }
    }

    #[test]
    fn setup_keyboard_once() {
        let k = setup_keyboard(r#"{"keymap":"br","locale":"pt_BR.UTF-8","appearance":"auto"}"#)
            .unwrap();
        assert!(k.contains("kb_layout = br\n"), "{k}");
        assert!(
            setup_keyboard(r#"{"keymap":"us(intl)"}"#)
                .unwrap()
                .contains("kb_variant = intl")
        );
        assert_eq!(setup_keyboard(r#"{"keymap":"xx"}"#), None);
        assert_eq!(setup_keyboard("junk"), None);
    }
}
