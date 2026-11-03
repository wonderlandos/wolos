// SPDX-License-Identifier: BSD-2-Clause
//! The person's appearance settings, read from the files the shell watches and kept current with
//! inotify (plan section 5.1): the active theme id (`$XDG_CONFIG_HOME/wolos/theme`, dark or light
//! and the accent through `core`'s theme catalog and its Auto day/night switch), the Increase
//! Contrast flag (`$XDG_CONFIG_HOME/wol/contrast`, present or not) and the desktop preferences
//! (`$XDG_CONFIG_HOME/wolos/desktop.json`: reduce motion, reduce transparency). A change swaps the
//! engine's environment and repaints; computed styles keep token references, so nothing restyles
//! from scratch.
//!
//! Only built-in themes and their accent variants are resolved here (no directory walk on the
//! app's thread); an id the catalog does not know draws the default of its appearance.
//!
//! The window chrome's hooks live here too: the glass app-id suffix the compositor keys on, and
//! the object path the window's menus are published at for the menu bar.

use super::sys;
use crate::env::Environment;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The contrast flag, relative to the config home (the shell's `kit::style::CONTRAST_FILE`).
pub const CONTRAST_FILE: &str = "wol/contrast";
/// The active theme id file (`shared::theme::ACTIVE_FILE`).
pub const THEME_FILE: &str = shared::theme::ACTIVE_FILE;
/// The desktop preferences (`core::hyprprefs::PREFS_FILE`).
pub const PREFS_FILE: &str = wol_core::hyprprefs::PREFS_FILE;
/// Largest theme id file read.
const MAX_ID: u64 = 4096;

/// The app-id suffix of a toplevel that asks the compositor for glass behind it (frozen; the
/// compositor's own copy is `realtime::glass::APP_SUFFIX`, its window rule keys on it).
pub const GLASS_SUFFIX: &str = ".glass";

/// Whether a window gets glass behind it: its root computes `backdrop: glass` and the person has
/// not turned on Reduce Transparency (then the token colour stays, as the Kit does).
pub fn wants_glass(root: crate::style::value::Backdrop, env: &Environment) -> bool {
    root == crate::style::value::Backdrop::Glass && !env.reduce_transparency
}

/// The app id the toplevel carries: the app's own, with [`GLASS_SUFFIX`] when it wants glass.
/// An id that already ends in it is not suffixed twice; an empty id stays empty (no rule matches).
pub fn toplevel_app_id(app_id: &str, glass: bool) -> String {
    let base = base_app_id(app_id);
    if glass && !base.is_empty() {
        format!("{base}{GLASS_SUFFIX}")
    } else {
        base.to_owned()
    }
}

/// The app's own id from a toplevel's app id (the glass suffix dropped).
pub fn base_app_id(app_id: &str) -> &str {
    match app_id.strip_suffix(GLASS_SUFFIX) {
        Some(base) if !base.is_empty() => base,
        _ => app_id,
    }
}

/// The D-Bus object path the app's menus are published at, the GApplication form of its own id
/// (`/org/example/App`): the shell's `ui::transient::object_path`, frozen.
pub fn menu_object_path(app_id: &str) -> String {
    format!("/{}", base_app_id(app_id).replace('.', "/"))
}

/// The appearance inputs and what they resolve to.
#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    pub env: Environment,
    /// The resolved theme's 13 token colours as 0xRRGGBBAA, in `shared::theme::TOKENS` order
    /// (0 for a token the theme does not set).
    pub tokens: [u32; 13],
    /// Seconds to the next Auto switch, when the theme follows the time of day.
    pub auto_switch_in: Option<u32>,
}

fn hex(s: &str) -> Option<u32> {
    let h = s.trim().strip_prefix('#')?;
    let v = u32::from_str_radix(h, 16).ok()?;
    match h.len() {
        6 => Some((v << 8) | 0xff),
        8 => Some(v),
        _ => None,
    }
}

fn read_id(path: &Path) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    let mut s = String::new();
    f.take(MAX_ID).read_to_string(&mut s).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

/// Read everything under `config_home` at local time `hms`.
pub fn read(config_home: &Path, hms: (u8, u8, u8)) -> Appearance {
    use wol_core::theme::{ThemeCatalog, accent, active};
    let saved = read_id(&config_home.join(THEME_FILE));
    let auto = accent::is_auto(saved.as_deref());
    // Outside Auto the theme itself says dark or light; prefer_dark only picks Auto's half.
    let prefer_dark = auto && accent::auto_is_dark(hms.0);
    let eff = accent::effective(saved.as_deref(), prefer_dark);
    let id = active::effective(eff.as_deref(), prefer_dark);
    let cat = accent::variants()
        .into_iter()
        .fold(ThemeCatalog::builtin(), ThemeCatalog::with);
    let variant_default = if prefer_dark {
        shared::theme::Variant::Dark
    } else {
        shared::theme::Variant::Light
    }
    .default_theme();
    let theme = cat.resolve(&id).or_else(|_| cat.resolve(variant_default));
    let mut env = Environment::default();
    let mut tokens = [0u32; 13];
    if let Ok(t) = &theme {
        env.dark = t.variant == shared::theme::Variant::Dark;
        for (i, name) in shared::theme::TOKENS.iter().enumerate() {
            if let Some(v) = t.tokens.get(*name).and_then(|s| hex(s)) {
                tokens[i] = v;
            }
        }
        if let Some(a) = t.tokens.get("accent").and_then(|s| hex(s)) {
            env.accent_rgb = a >> 8;
        }
    }
    env.contrast = config_home.join(CONTRAST_FILE).exists();
    let prefs = wol_core::hyprprefs::load_desktop(config_home);
    env.reduce_motion = prefs.reduce_motion;
    env.reduce_transparency = prefs.reduce_transparency;
    Appearance {
        env,
        tokens,
        auto_switch_in: auto.then(|| accent::secs_to_next_switch(hms.0, hms.1, hms.2).max(1)),
    }
}

/// The watcher: inotify on the three files' directories and a timer for the Auto switch.
pub struct Watch {
    home: PathBuf,
    ino: sys::Inotify,
    timer: sys::Timer,
    pub current: Appearance,
}

impl Watch {
    /// Start watching under `config_home`. Directories that do not exist yet are watched through
    /// the config home itself, so creating them is seen.
    pub fn new(config_home: &Path) -> std::io::Result<Watch> {
        let ino = sys::Inotify::new()?;
        for d in [
            config_home.to_path_buf(),
            config_home.join("wolos"),
            config_home.join("wol"),
        ] {
            // A missing directory is fine: its parent's watch sees it appear.
            let _ = ino.watch(&d);
        }
        let timer = sys::Timer::new()?;
        let current = read(config_home, sys::local_time());
        let w = Watch {
            home: config_home.to_path_buf(),
            ino,
            timer,
            current,
        };
        w.arm();
        Ok(w)
    }

    fn arm(&self) {
        let first = self
            .current
            .auto_switch_in
            .map(|s| std::time::Duration::from_secs(u64::from(s)));
        // A timer that cannot be set only delays the next Auto switch to the next file change.
        let _ = self.timer.set(first, std::time::Duration::ZERO);
    }

    pub fn fds(&self) -> [std::os::fd::RawFd; 2] {
        [self.ino.fd(), self.timer.fd()]
    }

    /// Look at what changed; the new appearance when it differs.
    pub fn poll(&mut self) -> Option<&Appearance> {
        let files = self.ino.drain();
        let clock = self.timer.take() > 0;
        if !files && !clock {
            return None;
        }
        if files {
            // Directories created since the start are watched from now on.
            let _ = self.ino.watch(&self.home.join("wolos"));
            let _ = self.ino.watch(&self.home.join("wol"));
        }
        let next = read(&self.home, sys::local_time());
        let changed = next != self.current;
        self.current = next;
        self.arm();
        changed.then_some(&self.current)
    }
}
