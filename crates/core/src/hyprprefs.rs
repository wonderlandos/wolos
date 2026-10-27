//! WolOS-managed Hyprland include (`hypr/wolos/settings.conf`): typed desktop prefs from System Settings
//! rendered as `category:key = value` one-liners (the same names `hyprctl keyword` takes), plus the minimal
//! command batch that applies a change live. Never touches the user's own hyprland.conf; never reloads.
use crate::shortcuts::{self, Overrides, Shortcut};
use crate::system::Host;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Settings' source of truth, relative to `$XDG_CONFIG_HOME`.
pub const PREFS_FILE: &str = "wolos/desktop.json";
/// Rendered include, sourced by the user conf through the `hypr/wolos/*.conf` glob (`prefs::ensure_source`).
pub const INCLUDE: &str = "hypr/wolos/settings.conf";
/// Named rules in the shipped hyprland.conf: everything floats; listed apps keep floating while tiling.
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
    /// Windows tile instead of floating (macOS-like floating is the default).
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
    /// Same values as the shipped hyprland.conf: writing defaults changes nothing.
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

    /// First run: carry over the older `hypr/wolos/keyboard.conf` (layout, repeat).
    pub fn from_legacy_keyboard(keyboard_conf: Option<&str>) -> Self {
        let mut p = Self::default();
        if let Some(t) = keyboard_conf {
            let (l, r) = crate::prefs::parse_keyboard(Some(t));
            p.layouts = vec![crate::prefs::LAYOUTS[l].0.to_string()];
            let (_, rate, delay) = crate::prefs::REPEATS[r];
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

    /// The include file: keywords, then session env and login items.
    pub fn include(&self) -> String {
        let mut s = String::from(
            "# Written by WolOS System Settings. Change these there; your own lines belong in hyprland.conf.\n",
        );
        for (k, v) in self.keywords() {
            s.push_str(&format!("{k} = {v}\n"));
        }
        s.push_str(&format!("env = XCURSOR_SIZE,{}\n", self.cursor_size));
        if let Some(l) = self
            .language
            .as_deref()
            .filter(|l| crate::region::is_valid_locale(l))
        {
            s.push_str(&format!("env = LANG,{l}\nenv = LC_ALL,{l}\n"));
        }
        for c in self.login_items.iter().filter(|c| is_valid_command(c)) {
            s.push_str(&format!("exec-once = {}\n", c.trim()));
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

/// The older Keyboard fragment, folded into the include on the first save.
const LEGACY_KEYBOARD: &str = "hypr/wolos/keyboard.conf";

fn read_in(home: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(home.join(rel)).ok()
}

/// Saved desktop prefs under `home` (`$XDG_CONFIG_HOME`); first run carries over the older keyboard fragment.
pub fn load_desktop(home: &Path) -> DesktopPrefs {
    match read_in(home, PREFS_FILE) {
        Some(t) => DesktopPrefs::parse(Some(&t)),
        None => DesktopPrefs::from_legacy_keyboard(read_in(home, LEGACY_KEYBOARD).as_deref()),
    }
}

pub fn load_overrides(home: &Path) -> Overrides {
    Overrides::parse(read_in(home, shortcuts::PREFS_FILE).as_deref())
}

/// Shipped shortcuts: the installed confs, else the copies built in.
pub fn shortcut_defaults(host: &dyn Host) -> Vec<Shortcut> {
    shortcuts::load_defaults(|p| host.read_text(p))
}

/// Writes prefs, both includes and the user conf's `source` line: one journal entry (`wol undo`,
/// Settings > Restore).
fn persist(home: &Path, d: &DesktopPrefs, o: &Overrides, list: &[Shortcut]) -> std::io::Result<()> {
    let glob = home.join(crate::prefs::HYPR_DIR).join("*.conf");
    let user = crate::prefs::HYPR_USER;
    let source =
        crate::prefs::ensure_source(read_in(home, user).as_deref(), &glob.to_string_lossy());
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

/// One apply at a time: each reads the saved state, diffs and writes it back.
static APPLYING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Submap with no binds of its own (shipped hyprland.conf): while Settings records a shortcut, WolOS
/// shortcuts pause so the keys reach it. Esc (non-consuming bind) always leaves it.
pub const RECORD_SUBMAP: &str = "wol-record";

/// Pauses (true) or resumes global shortcuts around a shortcut recording.
pub fn capture(host: &dyn Host, on: bool) -> bool {
    let to = if on { RECORD_SUBMAP } else { "reset" };
    host.run(&["hyprctl", "dispatch", "submap", to])
        .is_some_and(|o| o.trim() == "ok")
}

/// One `hyprctl --batch` for the running session (none when nothing changed). Best effort: without a
/// running Hyprland the files still apply at the next login.
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
            assert!(inc.contains(&format!("windowrule[wol-float]:enable = {}\n", want[0])));
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
        let legacy = crate::prefs::keyboard_conf("br", 2).unwrap();
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
            "animations:enabled = false\n",
            "env = XCURSOR_SIZE,24\n",
            "env = LANG,pt_BR.UTF-8\n",
            "exec-once = foot\n",
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

    /// `WOL_HYPRLAND=<path to Hyprland> cargo test -p core hyprland_accepts -- --ignored`: the real parser
    /// (0.56) accepts the shipped conf plus both generated includes (plugins stripped: not loadable here).
    #[test]
    #[ignore]
    fn hyprland_accepts_generated_includes() {
        use crate::shortcuts as sc;
        let bin = std::env::var("WOL_HYPRLAND").expect("WOL_HYPRLAND");
        let mut conf = String::new();
        let mut depth = 0;
        for l in sc::SHIPPED.lines() {
            let t = l.trim_start();
            if t.starts_with("plugin {") || depth > 0 {
                depth += t.matches('{').count();
                depth -= t.matches('}').count();
                continue;
            }
            if !t.starts_with("plugin =") && !t.starts_with("source =") && !t.contains("hyprbars") {
                conf.push_str(l);
                conf.push('\n');
            }
        }
        let p = DesktopPrefs {
            tiling: true,
            layout: Layout::Master,
            float_apps: vec!["org.gnome.Calculator".into()],
            layouts: vec!["us".into(), "br".into()],
            reduce_motion: true,
            language: Some("pt_BR.UTF-8".into()),
            login_items: vec!["foot --server".into()],
            ..Default::default()
        };
        let d = sc::defaults(sc::SHIPPED, sc::TILING);
        let mut o = sc::Overrides::default();
        o.assign(
            &d,
            "exec wol-spotlight",
            sc::Combo::parse("SUPER ALT", "Space"),
            false,
        )
        .unwrap();
        o.add_custom(
            &d,
            sc::Custom {
                name: "Top".into(),
                combo: sc::Combo::parse("SUPER CTRL", "T").unwrap(),
                command: "foot -e top".into(),
            },
        )
        .unwrap();
        conf.push_str(&p.include());
        conf.push_str(&sc::include(&sc::effective(&d, &o), true));
        let f = std::env::temp_dir().join(format!("wol-hypr-verify-{}.conf", std::process::id()));
        std::fs::write(&f, &conf).unwrap();
        let out = std::process::Command::new(bin)
            .args(["--verify-config", "-c"])
            .arg(&f)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let _ = std::fs::remove_file(&f);
        assert!(text.contains("config ok"), "{text}\n{conf}");
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
            h.join("hypr/wolos/keyboard.conf"),
            "input {\n    kb_layout = br\n    repeat_rate = 40\n    repeat_delay = 250\n}\n",
        )
        .unwrap();
        let host = FakeHost::new(true);
        host.command(
            "hyprctl -j clients",
            r#"[{"address":"0x1","class":"foot","floating":true,"workspace":{"id":1}}]"#,
        );
        // First load migrates the older keyboard fragment.
        let old = load_desktop(&h);
        assert_eq!(old.layouts, ["br"]);
        let new = DesktopPrefs {
            tiling: true,
            ..old.clone()
        };
        apply_desktop(&host, &h, &new).unwrap();
        assert_eq!(load_desktop(&h), new);
        let inc = std::fs::read_to_string(h.join(INCLUDE)).unwrap();
        assert!(inc.contains("windowrule[wol-float]:enable = false\n"));
        assert!(inc.contains("input:kb_layout = br\n"));
        assert!(!h.join("hypr/wolos/keyboard.conf").exists(), "migrated");
        let user = std::fs::read_to_string(h.join("hypr/hyprland.conf")).unwrap();
        assert!(user.contains(&format!("source = {}/hypr/wolos/*.conf", h.display())));
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
            sc.contains("bind = SUPER ALT, left, movefocus, l\n"),
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
        assert!(!capture(&FakeHost::new(true), true), "no Hyprland");
        let shipped = shortcuts::SHIPPED;
        let block =
            format!("submap = {RECORD_SUBMAP}\nbindn = , escape, submap, reset\nsubmap = reset\n");
        assert!(
            shipped.contains(&block),
            "shipped conf defines the record submap"
        );
    }

    #[test]
    fn apply_shortcuts_rebinds_live_and_persists() {
        let h = home("shortcuts");
        let host = FakeHost::new(true);
        let defaults = shortcut_defaults(&host);
        let shot = defaults
            .iter()
            .find(|s| s.name == "Show Spotlight search")
            .expect("spotlight shortcut")
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
        assert!(sc.contains("bind = SUPER SHIFT, K, "), "{sc}");
        let _ = std::fs::remove_dir_all(h);
    }
}
