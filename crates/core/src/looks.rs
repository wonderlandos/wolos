//! Effects & Looks: how windows look (corners, blur, transparency, borders, shadows, gaps, motion) as
//! Hyprland keywords in the managed include (`hyprprefs`), and Looks presets that set them all at once.
//! Presets are data files: shipped in [`SYSTEM_DIR`], the user's own in [`USER_DIR`] (import/export).
//! [`Looks::default`] equals the shipped hyprland.conf: writing defaults changes nothing.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Shipped presets (`<id>.json`).
pub const SYSTEM_DIR: &str = "/usr/local/share/wolos/looks";
/// The user's presets, relative to `$XDG_CONFIG_HOME`.
pub const USER_DIR: &str = "wolos/looks";
/// Preset id of the shipped defaults.
pub const DEFAULT_ID: &str = "wolos";
/// Largest preset file read or imported.
pub const MAX_FILE: u64 = 64 * 1024;

/// How windows open and close.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Motion {
    /// Grow from the centre (macOS-like, the default).
    #[default]
    Zoom,
    /// Spring past full size and settle.
    Pop,
    /// Slide in from the nearest edge.
    Slide,
}

/// How desktops (workspaces) change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Switch {
    #[default]
    Slide,
    Vertical,
    Fade,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Looks {
    /// Space around windows in px (between windows: half).
    pub gaps: u8,
    /// Corner radius in px.
    pub rounding: u8,
    pub blur: bool,
    /// Blur radius ("strength").
    pub blur_size: u8,
    /// Blur quality (passes).
    pub blur_passes: u8,
    /// Colour saturation behind glass, 0..=1.
    pub vibrancy: f32,
    /// Window opacity, focused and unfocused, 0.5..=1.
    pub active_opacity: f32,
    pub inactive_opacity: f32,
    /// Menu bar, Dock and Control Center glass: multiplies the theme's glass alpha (1 = theme).
    pub glass: f32,
    /// Border width in px (0 = none).
    pub border_size: u8,
    /// Focused-window border: one colour, or up to three for a gradient (`#rrggbb[aa]`).
    pub border_colors: Vec<String>,
    /// Gradient angle in degrees.
    pub border_angle: u16,
    pub inactive_border: String,
    pub shadow: bool,
    /// Shadow size in px.
    pub shadow_range: u8,
    /// Dim windows that are not focused.
    pub dim_inactive: bool,
    /// 0..=1.
    pub dim_strength: f32,
    /// Animations on (Reduce Motion turns them off whatever this says).
    pub animations: bool,
    pub motion: Motion,
    pub switch: Switch,
    /// Animation speed, 0.5 (slower) ..= 2 (faster); 1 = shipped durations.
    pub speed: f32,
}

impl Default for Looks {
    fn default() -> Self {
        Self {
            gaps: 12,
            rounding: 12,
            blur: true,
            blur_size: 8,
            blur_passes: 3,
            vibrancy: 0.2,
            active_opacity: 1.0,
            inactive_opacity: 1.0,
            glass: 1.0,
            border_size: 0,
            border_colors: vec!["#8b6cff".into()],
            border_angle: 45,
            inactive_border: CLEAR.into(),
            shadow: true,
            shadow_range: 30,
            dim_inactive: false,
            dim_strength: 0.2,
            animations: true,
            motion: Motion::Zoom,
            switch: Switch::Slide,
            speed: 1.0,
        }
    }
}

/// Allowed ranges (the UI's slider bounds come from here).
pub mod range {
    pub const GAPS: (u8, u8) = (0, 40);
    pub const ROUNDING: (u8, u8) = (0, 24);
    pub const BLUR_SIZE: (u8, u8) = (1, 16);
    pub const BLUR_PASSES: (u8, u8) = (1, 4);
    pub const OPACITY: (f32, f32) = (0.5, 1.0);
    pub const GLASS: (f32, f32) = (0.5, 1.2);
    pub const BORDER: (u8, u8) = (0, 6);
    pub const SHADOW: (u8, u8) = (0, 60);
    pub const DIM: (f32, f32) = (0.05, 0.6);
    pub const SPEED: (f32, f32) = (0.5, 2.0);
    pub const BORDER_COLORS: usize = 3;
}

/// A colour that draws nothing (no border on other windows).
pub const CLEAR: &str = "#00000000";

/// `#rgb`, `#rrggbb` or `#rrggbbaa`.
pub fn is_color(s: &str) -> bool {
    s.strip_prefix('#')
        .is_some_and(|h| matches!(h.len(), 3 | 6 | 8) && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Hyprland colour: `#8b6cff` -> `rgba(8b6cffff)`; None for anything else.
pub fn hypr_color(s: &str) -> Option<String> {
    if !is_color(s) {
        return None;
    }
    let h = s[1..].to_ascii_lowercase();
    let full = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect::<String>() + "ff",
        6 => h + "ff",
        _ => h,
    };
    Some(format!("rgba({full})"))
}

fn round2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

fn clamp_f(v: f32, (lo, hi): (f32, f32)) -> f32 {
    if v.is_finite() {
        round2(v.clamp(lo, hi))
    } else {
        lo
    }
}

/// Seconds formatted the way Hyprland reads animation speed (deciseconds, one decimal).
fn ds(base: f32, speed: f32) -> String {
    format!("{:.1}", (base / speed).max(0.1))
}

impl Looks {
    /// Every value inside its range, floats to two decimals, colours valid (bad ones dropped or
    /// replaced by the default): what is saved, applied and compared.
    pub fn normalized(&self) -> Self {
        let d = Self::default();
        let u = |v: u8, (lo, hi): (u8, u8)| v.clamp(lo, hi);
        let mut colors: Vec<String> = self
            .border_colors
            .iter()
            .filter(|c| is_color(c))
            .take(range::BORDER_COLORS)
            .map(|c| c.to_ascii_lowercase())
            .collect();
        if colors.is_empty() {
            colors = d.border_colors.clone();
        }
        Self {
            gaps: u(self.gaps, range::GAPS),
            rounding: u(self.rounding, range::ROUNDING),
            blur: self.blur,
            blur_size: u(self.blur_size, range::BLUR_SIZE),
            blur_passes: u(self.blur_passes, range::BLUR_PASSES),
            vibrancy: clamp_f(self.vibrancy, (0.0, 1.0)),
            active_opacity: clamp_f(self.active_opacity, range::OPACITY),
            inactive_opacity: clamp_f(self.inactive_opacity, range::OPACITY),
            glass: clamp_f(self.glass, range::GLASS),
            border_size: u(self.border_size, range::BORDER),
            border_colors: colors,
            border_angle: self.border_angle % 360,
            inactive_border: if is_color(&self.inactive_border) {
                self.inactive_border.to_ascii_lowercase()
            } else {
                d.inactive_border
            },
            shadow: self.shadow,
            shadow_range: u(self.shadow_range, range::SHADOW),
            dim_inactive: self.dim_inactive,
            dim_strength: clamp_f(self.dim_strength, range::DIM),
            animations: self.animations,
            motion: self.motion,
            switch: self.switch,
            speed: clamp_f(self.speed, range::SPEED),
        }
    }

    /// Same look once normalized (presets compare this way: "Custom" when none matches).
    pub fn same(&self, other: &Self) -> bool {
        self.normalized() == other.normalized()
    }

    /// Hyprland keywords in apply order. Accessibility wins: `reduce_transparency` turns blur off
    /// and windows opaque, `reduce_motion` turns animations off.
    pub fn keywords(
        &self,
        reduce_motion: bool,
        reduce_transparency: bool,
    ) -> Vec<(String, String)> {
        let l = self.normalized();
        let kv = |k: &str, v: String| (k.to_string(), v);
        let b = |v: bool| if v { "true" } else { "false" }.to_string();
        let f = |v: f32| format!("{v:.2}");
        let opaque = reduce_transparency;
        let mut border: Vec<String> = l
            .border_colors
            .iter()
            .filter_map(|c| hypr_color(c))
            .collect();
        if border.len() > 1 {
            border.push(format!("{}deg", l.border_angle));
        }
        let (style_in, curve_in) = match l.motion {
            Motion::Zoom => ("popin 90%", "wolOut"),
            Motion::Pop => ("popin 80%", "spring"),
            Motion::Slide => ("slide", "wolOut"),
        };
        let style_out = match l.motion {
            Motion::Slide => "slide",
            _ => "popin 90%",
        };
        let ws = match l.switch {
            Switch::Slide => "slide",
            Switch::Vertical => "slidevert",
            Switch::Fade => "fade",
        };
        let s = l.speed;
        let anim = |name: &str, base: f32, curve: &str, style: &str| {
            let tail = if style.is_empty() {
                String::new()
            } else {
                format!(",{style}")
            };
            kv(
                "animation",
                format!("{name},1,{},{curve}{tail}", ds(base, s)),
            )
        };
        vec![
            kv("general:gaps_in", (l.gaps / 2).to_string()),
            kv("general:gaps_out", l.gaps.to_string()),
            kv("decoration:rounding", l.rounding.to_string()),
            kv("decoration:blur:enabled", b(l.blur && !opaque)),
            kv("decoration:blur:size", l.blur_size.to_string()),
            kv("decoration:blur:passes", l.blur_passes.to_string()),
            kv("decoration:blur:vibrancy", f(l.vibrancy)),
            kv(
                "decoration:active_opacity",
                f(if opaque { 1.0 } else { l.active_opacity }),
            ),
            kv(
                "decoration:inactive_opacity",
                f(if opaque { 1.0 } else { l.inactive_opacity }),
            ),
            kv("general:border_size", l.border_size.to_string()),
            kv("general:col.active_border", border.join(" ")),
            kv(
                "general:col.inactive_border",
                hypr_color(&l.inactive_border).unwrap_or_default(),
            ),
            kv("decoration:shadow:enabled", b(l.shadow)),
            kv("decoration:shadow:range", l.shadow_range.to_string()),
            kv("decoration:dim_inactive", b(l.dim_inactive)),
            kv("decoration:dim_strength", f(l.dim_strength)),
            kv("animations:enabled", b(l.animations && !reduce_motion)),
            anim("windowsIn", 2.2, curve_in, style_in),
            anim("windowsOut", 1.8, "wolIn", style_out),
            anim("windowsMove", 4.0, "spring", ""),
            anim("fade", 3.0, "ease", ""),
            anim("layers", 3.0, "ease", "fade"),
            anim("workspaces", 5.0, "ease", ws),
            anim("specialWorkspace", 4.0, "ease", "slidevert"),
        ]
    }

    /// Rough GPU cost: blur work, see-through windows (blur behind every window) and shadow size.
    /// [`Cost::Heavy`] earns a quiet hint on integrated graphics.
    pub fn cost(&self) -> Cost {
        let l = self.normalized();
        let blur = if l.blur {
            u32::from(l.blur_passes) * u32::from(l.blur_size)
        } else {
            0
        };
        let see_through = l.blur && l.inactive_opacity.min(l.active_opacity) < 0.95;
        let score = blur
            + if see_through { 16 } else { 0 }
            + if l.shadow {
                u32::from(l.shadow_range) / 4
            } else {
                0
            };
        match score {
            0..=20 => Cost::Light,
            21..=44 => Cost::Moderate,
            _ => Cost::Heavy,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cost {
    Light,
    Moderate,
    Heavy,
}

/// One Looks preset file: `{"name": "Glass", "description": "...", <Looks fields>}`. Missing fields
/// take the WolOS defaults, so a preset only lists what it changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    /// File stem; not stored in the file.
    #[serde(skip)]
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The user's own (import, save) rather than shipped.
    #[serde(skip)]
    pub user: bool,
    #[serde(flatten)]
    pub looks: Looks,
}

/// Why a preset file was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresetError {
    TooLarge,
    NotAPreset(String),
    Io(String),
}

impl std::fmt::Display for PresetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => write!(f, "This file is too large to be a Looks preset."),
            Self::NotAPreset(_) => write!(f, "This file isn’t a Looks preset."),
            Self::Io(e) => write!(f, "The file couldn’t be read or written ({e})."),
        }
    }
}

/// Longest description a Looks card shows (two short lines); longer ones end at a word with "…".
pub const DESCRIPTION_CHARS: usize = 44;

/// One line of at most `max` characters, never cut inside a word: a longer text ends with "…".
fn clean_text(s: &str, max: usize) -> String {
    let s: String = s
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max {
        return s;
    }
    // Room for the ellipsis; keep whole words only.
    let mut out = String::new();
    for w in s.split(' ') {
        let len = out.chars().count() + usize::from(!out.is_empty()) + w.chars().count();
        if len + 1 > max {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    let out = out.trim_end_matches([',', ';', ':', '.']);
    format!("{out}…")
}

/// Name shown in Settings: one line, at most 40 characters, no control characters.
fn clean_name(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(40)
        .collect::<String>()
        .trim()
        .to_string()
}

/// File stem for a preset name: lowercase ASCII letters, digits and dashes.
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-').chars().take(40).collect::<String>();
    if s.is_empty() { "look".into() } else { s }
}

impl Preset {
    /// Parses and normalizes preset JSON; a nameless preset takes `id` as its name.
    pub fn parse(id: &str, json: &str) -> Result<Self, PresetError> {
        let mut p: Preset =
            serde_json::from_str(json).map_err(|e| PresetError::NotAPreset(e.to_string()))?;
        p.id = id.to_string();
        p.name = clean_name(&p.name);
        if p.name.is_empty() {
            p.name = id.to_string();
        }
        p.description = clean_text(&p.description, DESCRIPTION_CHARS);
        p.looks = p.looks.normalized();
        Ok(p)
    }

    pub fn to_json(&self) -> String {
        let mut p = self.clone();
        p.looks = p.looks.normalized();
        serde_json::to_string_pretty(&p).unwrap_or_default() + "\n"
    }
}

/// Preset directories in load order: shipped, then the user's (`config_home`/[`USER_DIR`]).
pub fn dirs(config_home: &Path) -> Vec<(PathBuf, bool)> {
    vec![
        (PathBuf::from(SYSTEM_DIR), false),
        (config_home.join(USER_DIR), true),
    ]
}

/// Every preset in `dirs` (`(dir, user)`), sorted with the WolOS default first, then shipped
/// presets, then the user's, by name. A user preset with a shipped id replaces it. Unreadable,
/// oversized or invalid files are skipped.
pub fn load(dirs: &[(PathBuf, bool)]) -> Vec<Preset> {
    let mut out: Vec<Preset> = Vec::new();
    for (dir, user) in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        for f in files {
            let Some(id) = f.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(p) = read_file(&f, id) else { continue };
            let p = Preset { user: *user, ..p };
            out.retain(|x| x.id != p.id);
            out.push(p);
        }
    }
    out.sort_by_key(|p| (p.id != DEFAULT_ID, p.user, p.name.to_lowercase()));
    out
}

fn read_file(path: &Path, id: &str) -> Result<Preset, PresetError> {
    let meta = std::fs::metadata(path).map_err(|e| PresetError::Io(e.to_string()))?;
    if meta.len() > MAX_FILE {
        return Err(PresetError::TooLarge);
    }
    let text = std::fs::read_to_string(path).map_err(|e| PresetError::Io(e.to_string()))?;
    Preset::parse(id, &text)
}

/// First preset that looks the same as `looks`; None = "Custom".
pub fn matching<'a>(presets: &'a [Preset], looks: &Looks) -> Option<&'a Preset> {
    presets.iter().find(|p| p.looks.same(looks))
}

/// Copies a preset file into the user's presets (`config_home`/[`USER_DIR`]) under a free id
/// derived from its name; returns the stored preset. The file is validated and normalized first.
pub fn import(config_home: &Path, file: &Path) -> Result<Preset, PresetError> {
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("look");
    let p = read_file(file, stem)?;
    save_user(config_home, &p.name, &p.description, &p.looks)
}

/// Saves `looks` as a user preset named `name` (a free id: `name`, `name-2`, ...).
pub fn save_user(
    config_home: &Path,
    name: &str,
    description: &str,
    looks: &Looks,
) -> Result<Preset, PresetError> {
    let dir = config_home.join(USER_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| PresetError::Io(e.to_string()))?;
    let base = slug(name);
    let id = (1..)
        .map(|n| {
            if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            }
        })
        .find(|id| !dir.join(format!("{id}.json")).exists() && id != DEFAULT_ID)
        .unwrap_or(base);
    let p = Preset {
        id: id.clone(),
        name: clean_name(name),
        description: clean_text(description, DESCRIPTION_CHARS),
        user: true,
        looks: looks.normalized(),
    };
    write_atomic(&dir.join(format!("{id}.json")), &p.to_json())?;
    Ok(p)
}

/// Writes the current look to `file` as a preset others can import.
pub fn export(file: &Path, name: &str, looks: &Looks) -> Result<(), PresetError> {
    let p = Preset {
        id: String::new(),
        name: clean_name(name),
        description: String::new(),
        user: true,
        looks: looks.normalized(),
    };
    write_atomic(file, &p.to_json())
}

/// Removes a user preset (shipped ones cannot be removed).
pub fn remove_user(config_home: &Path, id: &str) -> Result<(), PresetError> {
    if slug(id) != id {
        return Err(PresetError::NotAPreset(id.into()));
    }
    std::fs::remove_file(config_home.join(USER_DIR).join(format!("{id}.json")))
        .map_err(|e| PresetError::Io(e.to_string()))
}

fn write_atomic(p: &Path, text: &str) -> Result<(), PresetError> {
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, text)
        .and_then(|_| std::fs::rename(&tmp, p))
        .map_err(|e| PresetError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kw(l: &Looks, k: &str) -> String {
        l.keywords(false, false)
            .into_iter()
            .find(|(x, _)| x == k)
            .map(|(_, v)| v)
            .unwrap_or_else(|| panic!("{k}"))
    }

    const SHIPPED_CONF: &str =
        include_str!("../../../desktop/overlay/usr/local/share/wolos/hypr/hyprland.conf");

    /// Writing the defaults changes nothing: every value equals the shipped hyprland.conf.
    #[test]
    fn defaults_equal_the_shipped_conf() {
        // "a, 1, 4, b" == "a,1,4.0,b": compare comma fields, numbers as numbers.
        let norm = |v: &str| -> Vec<String> {
            v.split(',')
                .map(|f| {
                    let f = f.trim().replace(' ', "");
                    f.parse::<f64>().map_or(f, |n| format!("{n}"))
                })
                .collect()
        };
        let conf: Vec<(String, String)> = SHIPPED_CONF
            .lines()
            .filter_map(|l| l.split('#').next()?.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect();
        // Hyprland's own defaults: not in the shipped conf (borders are 0 px wide there).
        let unset = [
            "dim_inactive",
            "dim_strength",
            "col.active_border",
            "col.inactive_border",
            "active_opacity",
            "inactive_opacity",
        ];
        for (k, v) in Looks::default().keywords(false, false) {
            let leaf = k.rsplit(':').next().unwrap();
            if unset.contains(&leaf) {
                continue;
            }
            let want = norm(&v);
            let found = conf.iter().any(|(ck, cv)| {
                let got = norm(cv);
                if k == "animation" {
                    ck == "animation" && got == want
                } else {
                    ck == leaf && got == want
                }
            });
            assert!(found, "{k} = {v} differs from the shipped conf");
        }
        let d = Looks::default();
        assert_eq!(kw(&d, "decoration:active_opacity"), "1.00");
        assert_eq!(kw(&d, "decoration:dim_inactive"), "false");
    }

    #[test]
    fn keywords_table() {
        let l = Looks {
            gaps: 20,
            rounding: 0,
            blur_size: 12,
            blur_passes: 2,
            active_opacity: 0.95,
            inactive_opacity: 0.8,
            border_size: 2,
            border_colors: vec!["#33CCFF".into(), "#00ff99ee".into()],
            border_angle: 405,
            inactive_border: "#595959aa".into(),
            dim_inactive: true,
            dim_strength: 0.3,
            motion: Motion::Slide,
            switch: Switch::Fade,
            speed: 2.0,
            ..Default::default()
        };
        for (k, want) in [
            ("general:gaps_in", "10"),
            ("general:gaps_out", "20"),
            ("decoration:rounding", "0"),
            ("decoration:blur:size", "12"),
            ("decoration:blur:passes", "2"),
            ("decoration:active_opacity", "0.95"),
            ("decoration:inactive_opacity", "0.80"),
            ("general:border_size", "2"),
            (
                "general:col.active_border",
                "rgba(33ccffff) rgba(00ff99ee) 45deg",
            ),
            ("general:col.inactive_border", "rgba(595959aa)"),
            ("decoration:dim_inactive", "true"),
            ("decoration:dim_strength", "0.30"),
        ] {
            assert_eq!(kw(&l, k), want, "{k}");
        }
        let anims: Vec<String> = l
            .keywords(false, false)
            .into_iter()
            .filter(|(k, _)| k == "animation")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(anims[0], "windowsIn,1,1.1,wolOut,slide");
        assert_eq!(anims[1], "windowsOut,1,0.9,wolIn,slide");
        assert_eq!(anims[5], "workspaces,1,2.5,ease,fade");
        let one = Looks {
            border_colors: vec!["#fff".into()],
            ..Default::default()
        };
        assert_eq!(
            kw(&one, "general:col.active_border"),
            "rgba(ffffffff)",
            "one colour, no angle"
        );
    }

    #[test]
    fn accessibility_wins() {
        let l = Looks {
            active_opacity: 0.8,
            ..Default::default()
        };
        let get = |rm, rt, k: &str| {
            l.keywords(rm, rt)
                .into_iter()
                .find(|(x, _)| x == k)
                .unwrap()
                .1
        };
        assert_eq!(get(false, true, "decoration:blur:enabled"), "false");
        assert_eq!(get(false, true, "decoration:active_opacity"), "1.00");
        assert_eq!(get(true, false, "animations:enabled"), "false");
        assert_eq!(get(false, false, "animations:enabled"), "true");
        let off = Looks {
            animations: false,
            ..Default::default()
        };
        assert_eq!(kw(&off, "animations:enabled"), "false");
    }

    #[test]
    fn normalize_clamps_and_cleans() {
        let l = Looks {
            gaps: 200,
            blur_passes: 9,
            vibrancy: f32::NAN,
            active_opacity: 0.1,
            speed: 0.333,
            border_colors: vec!["red".into(), "#12".into()],
            inactive_border: "nope".into(),
            ..Default::default()
        }
        .normalized();
        assert_eq!((l.gaps, l.blur_passes), (40, 4));
        assert_eq!((l.vibrancy, l.active_opacity, l.speed), (0.0, 0.5, 0.5));
        assert_eq!(l.border_colors, Looks::default().border_colors);
        assert_eq!(l.inactive_border, Looks::default().inactive_border);
        assert!(
            Looks {
                vibrancy: 0.2000001,
                ..Default::default()
            }
            .same(&Looks::default())
        );
    }

    #[test]
    fn cost_grows_with_blur_transparency_and_shadows() {
        assert_eq!(Looks::default().cost(), Cost::Moderate);
        let flat = Looks {
            blur: false,
            shadow: false,
            ..Default::default()
        };
        assert_eq!(flat.cost(), Cost::Light);
        let heavy = Looks {
            blur_passes: 4,
            blur_size: 12,
            inactive_opacity: 0.8,
            ..Default::default()
        };
        assert_eq!(heavy.cost(), Cost::Heavy);
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wol-looks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn long_descriptions_end_at_a_word() {
        let long = "Tight gaps and crisp edges: room for more windows.";
        let d = clean_text(long, DESCRIPTION_CHARS);
        assert_eq!(d, "Tight gaps and crisp edges: room for more…");
        assert!(d.chars().count() <= DESCRIPTION_CHARS);
        assert_eq!(
            clean_text("Soft  glass\nand motion.", 44),
            "Soft glass and motion."
        );
        assert_eq!(clean_text("Short.", 44), "Short.");
        let p =
            Preset::parse("x", &format!(r#"{{"name": "X", "description": "{long}"}}"#)).unwrap();
        assert!(p.description.ends_with("more…"));
    }

    /// Shipped presets: valid, distinct, named, and the default equals [`Looks::default`].
    #[test]
    fn shipped_presets() {
        let dir = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../desktop/overlay/usr/local/share/wolos/looks"
        ));
        let mut n = 0;
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            let text = std::fs::read_to_string(&p).unwrap();
            let id = p.file_stem().unwrap().to_str().unwrap();
            assert_eq!(slug(id), id, "{id}: file name is a slug");
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            let fields = serde_json::to_value(Looks::default()).unwrap();
            for k in v.as_object().unwrap().keys() {
                assert!(
                    ["name", "description"].contains(&k.as_str()) || fields.get(k).is_some(),
                    "{id}: unknown key {k}"
                );
            }
            let pr = Preset::parse(id, &text).unwrap();
            assert!(!pr.description.is_empty(), "{id}: description");
            assert!(
                !pr.description.ends_with('…'),
                "{id}: description fits a card ({DESCRIPTION_CHARS} characters)"
            );
            assert_eq!(
                serde_json::to_value(pr.looks.normalized()).unwrap(),
                serde_json::to_value(&pr.looks).unwrap(),
                "{id}: values inside their ranges"
            );
            n += 1;
        }
        assert!(n >= 6, "at least six presets, found {n}");
        let all = load(&[(dir, false)]);
        assert_eq!(all[0].id, DEFAULT_ID);
        assert!(all[0].looks.same(&Looks::default()));
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert!(!a.looks.same(&b.looks), "{} looks like {}", a.id, b.id);
                assert_ne!(a.name, b.name);
            }
        }
        assert_eq!(matching(&all, &Looks::default()).unwrap().id, DEFAULT_ID);
        let custom = Looks {
            gaps: 3,
            ..Default::default()
        };
        assert!(matching(&all, &custom).is_none(), "Custom");
    }

    #[test]
    fn import_export_save_remove() {
        let home = tmp("io");
        let file = home.join("Neon Dreams.json");
        let mine = Looks {
            rounding: 4,
            border_size: 3,
            ..Default::default()
        };
        export(&file, "Neon Dreams", &mine).unwrap();
        let p = import(&home, &file).unwrap();
        assert_eq!(
            (p.id.as_str(), p.name.as_str()),
            ("neon-dreams", "Neon Dreams")
        );
        assert!(p.user && p.looks.same(&mine));
        let again = import(&home, &file).unwrap();
        assert_eq!(again.id, "neon-dreams-2", "never overwrites");
        let all = load(&dirs(&home)[1..]);
        assert_eq!(all.len(), 2);
        remove_user(&home, "neon-dreams-2").unwrap();
        assert_eq!(load(&dirs(&home)[1..]).len(), 1);
        assert!(remove_user(&home, "../x").is_err());
        let bad = home.join("bad.json");
        std::fs::write(&bad, "[1,2]").unwrap();
        assert!(matches!(
            import(&home, &bad),
            Err(PresetError::NotAPreset(_))
        ));
        std::fs::write(&bad, " ".repeat(MAX_FILE as usize + 1)).unwrap();
        assert_eq!(import(&home, &bad), Err(PresetError::TooLarge));
        let partial = Preset::parse("p", r#"{"name":"P\nQ","gaps":4}"#).unwrap();
        assert_eq!(partial.name, "PQ");
        assert_eq!(partial.looks.rounding, Looks::default().rounding);
        assert_eq!(slug("  Hello, World!  "), "hello-world");
        assert_eq!(slug("¿?"), "look");
        let _ = std::fs::remove_dir_all(home);
    }
}
