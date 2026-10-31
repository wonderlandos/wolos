//! Memory Saver (ADR 0022): background apps give their memory back and get it back the moment
//! they are used again (App Nap + memory compressor). Policy only; wold applies it to cgroups.
//! Inspired by memory-sentry (yanlvl99): per-role ceilings with CPU feedback, here with PSI too.
pub mod app;
pub mod ceiling;
pub mod role;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The user's choices, `$XDG_CONFIG_HOME/wolos/memory-saver.json`; wol-realtime forwards them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub enabled: bool,
    pub nap: bool,
    /// App ids (`shared::memsaver::app_id`) that keep their memory.
    pub exclude: Vec<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            enabled: true,
            nap: false,
            exclude: Vec::new(),
        }
    }
}

pub const FILE: &str = "wolos/memory-saver.json";

pub fn path(config_home: &Path) -> PathBuf {
    config_home.join(FILE)
}

/// Missing or unreadable file: the defaults (on, no App Nap, nothing excluded).
pub fn load(config_home: &Path) -> Prefs {
    std::fs::read(path(config_home))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(config_home: &Path, p: &Prefs) -> std::io::Result<()> {
    let f = path(config_home);
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = f.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(p).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(tmp, f)
}

impl Prefs {
    /// Exclude or include one app (by window class or app id). True when something changed.
    pub fn set_excluded(&mut self, app: &str, excluded: bool) -> bool {
        let id = shared::memsaver::app_id(app);
        let had = self.exclude.contains(&id);
        if excluded && !had {
            self.exclude.push(id);
            self.exclude.sort();
        } else if !excluded && had {
            self.exclude.retain(|e| *e != id);
        }
        had != excluded
    }
}

/// Human size for Settings: "820 MB", "1.4 GB" (decimal units, as Storage shows them).
pub fn size(bytes: u64) -> String {
    const MB: f64 = 1e6;
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else {
        format!("{:.0} MB", (b / MB).max(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_by_default_without_app_nap() {
        let dir = std::env::temp_dir().join(format!("wol-memsaver-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&dir), Prefs::default());
        assert!(Prefs::default().enabled && !Prefs::default().nap);
        let mut p = Prefs::default();
        assert!(p.set_excluded("Spotify", true));
        assert!(!p.set_excluded("spotify", true), "already excluded");
        assert!(p.set_excluded("Code", true));
        assert_eq!(p.exclude, vec!["code", "spotify"]);
        p.nap = true;
        save(&dir, &p).unwrap();
        assert_eq!(load(&dir), p);
        assert!(p.set_excluded("code", false));
        assert_eq!(p.exclude, vec!["spotify"]);
        // A partial or older file keeps the defaults for what it lacks.
        std::fs::write(path(&dir), r#"{"nap": true}"#).unwrap();
        assert!(load(&dir).enabled);
        std::fs::write(path(&dir), "garbage").unwrap();
        assert_eq!(load(&dir), Prefs::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sizes_read_like_storage() {
        for (b, want) in [
            (0, "1 MB"),
            (820_000_000, "820 MB"),
            (1_400_000_000, "1.4 GB"),
        ] {
            assert_eq!(size(b), want);
        }
    }
}
