//! `wopkg_context`: paths and network policy for one caller, from a JSON config with defaults
//! for the person running it. Holds no privilege.
use crate::ffi::{Error, Result};
use crate::index::{Kind, Record, segment::Segment};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub home: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub apps_dir: Option<PathBuf>,
    pub pacman_local: Option<PathBuf>,
    pub flatpak_dirs: Option<Vec<PathBuf>>,
    pub wol_records: Option<PathBuf>,
    pub keys_dir: Option<PathBuf>,
    pub wold_socket: Option<PathBuf>,
    /// Release state the system publishes (`/usr/lib/os-release`-like JSON, `release.rs`).
    pub release_file: Option<PathBuf>,
    #[serde(default)]
    pub allow_loopback_http: bool,
    pub jobs: Option<usize>,
}

pub struct Context {
    pub home: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
    pub data: PathBuf,
    pub apps: PathBuf,
    pub pacman_local: PathBuf,
    pub flatpak_dirs: Vec<PathBuf>,
    pub wol_records: PathBuf,
    pub keys: PathBuf,
    pub wold: PathBuf,
    pub release_file: PathBuf,
    pub jobs: usize,
    pub net: crate::fetch::Net,
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

impl Context {
    pub fn new(c: Config) -> Result<Context> {
        let home = c
            .home
            .or_else(|| env_dir("HOME"))
            .ok_or_else(|| Error::invalid("wopkg needs a home folder."))?;
        let cache = c.cache_dir.unwrap_or_else(|| {
            env_dir("XDG_CACHE_HOME")
                .unwrap_or(home.join(".cache"))
                .join("wopkg")
        });
        let state = c.state_dir.unwrap_or_else(|| {
            env_dir("XDG_STATE_HOME")
                .unwrap_or(home.join(".local/state"))
                .join("wopkg")
        });
        let data = c.data_dir.unwrap_or_else(|| {
            env_dir("XDG_DATA_HOME")
                .unwrap_or(home.join(".local/share"))
                .join("wopkg")
        });
        Ok(Context {
            apps: c.apps_dir.unwrap_or(home.join("Applications")),
            pacman_local: c.pacman_local.unwrap_or("/var/lib/pacman/local".into()),
            flatpak_dirs: c.flatpak_dirs.unwrap_or(vec![
                home.join(".local/share/flatpak"),
                "/var/lib/flatpak".into(),
            ]),
            wol_records: c.wol_records.unwrap_or("/usr/share/wolos/packages".into()),
            keys: c.keys_dir.unwrap_or("/usr/share/wolos/keys".into()),
            wold: c.wold_socket.unwrap_or(shared::jobs::SOCKET.into()),
            release_file: c
                .release_file
                .unwrap_or("/usr/lib/wolos/release.json".into()),
            jobs: c.jobs.unwrap_or(8).clamp(1, 32),
            net: crate::fetch::Net::new(c.allow_loopback_http),
            home,
            cache,
            state,
            data,
        })
    }

    pub fn index_dir(&self) -> PathBuf {
        self.cache.join("index")
    }

    pub fn segments(&self) -> Vec<Segment> {
        let mut names: Vec<PathBuf> = std::fs::read_dir(self.index_dir())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "seg"))
            .collect();
        names.sort();
        names.iter().filter_map(|p| Segment::open(p)).collect()
    }

    pub fn installed(&self) -> Installed {
        let names = |d: &Path| -> Vec<String> {
            std::fs::read_dir(d)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        };
        let pacman = names(&self.pacman_local)
            .iter()
            .filter_map(|n| pacman_name(n).map(str::to_owned))
            .collect();
        let flatpak = self
            .flatpak_dirs
            .iter()
            .flat_map(|d| names(&d.join("app")))
            .collect();
        let wol = names(&self.wol_records).into_iter().collect();
        let appimages = crate::appimage::Registry::load(self)
            .apps
            .into_iter()
            .map(|a| a.name)
            .collect();
        let recipes = names(&self.data.join("recipes")).into_iter().collect();
        Installed {
            pacman,
            flatpak,
            wol,
            appimages,
            recipes,
        }
    }
}

/// `name-pkgver-pkgrel` (a pacman local db entry) -> name.
pub fn pacman_name(entry: &str) -> Option<&str> {
    let mut it = entry.rmatch_indices('-');
    it.next()?;
    let (i, _) = it.next()?;
    Some(&entry[..i]).filter(|n| !n.is_empty())
}

#[derive(Clone, Debug, Default)]
pub struct Installed {
    pub pacman: HashSet<String>,
    pub flatpak: HashSet<String>,
    pub wol: HashSet<String>,
    pub appimages: HashSet<String>,
    pub recipes: HashSet<String>,
}

impl Installed {
    pub fn has(&self, r: &Record) -> bool {
        match r.kind {
            Kind::Pacman | Kind::Aur => self.pacman.contains(&r.id),
            Kind::Flatpak => self.flatpak.contains(&r.id),
            Kind::Wol => self.wol.contains(&r.id),
            Kind::Appimage => self.appimages.contains(&r.name),
            Kind::Recipe => self.recipes.contains(&r.id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacman_entry_names() {
        let cases = [
            ("firefox-140.0-1", Some("firefox")),
            ("linux-wolos-1:1.0+7.3-2", Some("linux-wolos")),
            ("python-pyqt5-sip-12.17.0-2", Some("python-pyqt5-sip")),
            ("ALPM_DB_VERSION", None),
            ("x-1", None),
        ];
        for (e, want) in cases {
            assert_eq!(pacman_name(e), want, "{e}");
        }
    }

    #[test]
    fn defaults_follow_home() {
        let c = Context::new(Config {
            home: Some("/h".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(c.apps, Path::new("/h/Applications"));
        assert!(c.index_dir().ends_with("wopkg/index"));
    }
}
