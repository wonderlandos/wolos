// SPDX-License-Identifier: BSD-2-Clause
//! Optional modules (ADR 0032, docs/modules/modules.md): AI, developer, creative and console
//! stacks installed after the base. One record per module in `/usr/share/wolos/modules/<id>.json`;
//! the module itself is the meta package `wolos-module-<id>` (wolos-pkgbuilds). pacman is the
//! truth for "installed"; `$XDG_STATE_HOME/wolos/modules.json` holds what the user enabled (the
//! user-side `post` step ran); `/var/lib/wolos/modules-pending` lists ids the installer ticked but
//! could not install offline, drained by firstboot. Pure: callers pass what they read.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Registry directory: one `<id>.json` per module.
pub const DIR: &str = "/usr/share/wolos/modules";
/// Ids the installer queued for first boot, one per line.
pub const PENDING_FILE: &str = "/var/lib/wolos/modules-pending";
/// Per-user enablement, relative to the state home.
pub const STATE_FILE: &str = "wolos/modules.json";
/// Meta package name prefix.
pub const PACKAGE_PREFIX: &str = "wolos-module-";
/// The modules this release ships records for (contract tests).
pub const IDS: [&str; 6] = [
    "ai-claude",
    "ai-local",
    "ai-sessions",
    "dev",
    "creative",
    "console",
];

/// One registry record (`/usr/share/wolos/modules/<id>.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Module {
    pub id: String,
    pub name: String,
    pub summary: String,
    /// Icon name (freedesktop theme name or a WolOS symbolic).
    pub icon: String,
    /// `ai` | `developer` | `creative` | `console`.
    pub category: String,
    /// Download size the installer shows ("Downloads N MB").
    pub size_mb: u32,
    /// Arch and `[wolos]` packages the meta package depends on (documentation; pacman decides).
    pub packages: Vec<String>,
    /// Flatpak app ids installed user-side by `enable`.
    pub flatpaks: Vec<String>,
    /// wopkg recipe names installed user-side by `enable` (ADR 0021).
    pub recipes: Vec<String>,
    /// mise tools (`mise use -g <tool>`) installed user-side by `enable`.
    pub mise: Vec<String>,
    /// Enable step as argv, run as the user, idempotent; empty = nothing to run.
    pub post: Vec<String>,
    /// Installable from the ISO without network.
    pub offline: bool,
    /// Requirements in words: `network`, `gpu`, ...
    pub needs: Vec<String>,
}

/// `[a-z][a-z0-9-]{0,31}`: a safe file stem and package suffix.
pub fn valid_id(id: &str) -> bool {
    let mut c = id.chars();
    matches!(c.next(), Some(f) if f.is_ascii_lowercase())
        && id.len() <= 32
        && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == '-')
}

impl Module {
    /// Parses one record; the id must be valid and the name present.
    pub fn parse(text: &str) -> Result<Module, String> {
        let m: Module = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if !valid_id(&m.id) {
            return Err(format!("bad module id '{}'", m.id));
        }
        if m.name.trim().is_empty() {
            return Err(format!("module '{}' has no name", m.id));
        }
        if m.post.iter().any(|a| a.is_empty()) {
            return Err(format!("module '{}': post has an empty word", m.id));
        }
        Ok(m)
    }

    /// `wolos-module-<id>`.
    pub fn package(&self) -> String {
        format!("{PACKAGE_PREFIX}{}", self.id)
    }

    /// Whether `enable` has anything to do as the user.
    pub fn has_user_step(&self) -> bool {
        !(self.post.is_empty()
            && self.recipes.is_empty()
            && self.mise.is_empty()
            && self.flatpaks.is_empty())
    }
}

/// Every valid record in `dir`, sorted by id. `list` gives the directory's full paths, `read` a
/// file's text. A record whose file stem is not its id, or that does not parse, is skipped.
pub fn load(
    dir: &Path,
    list: &dyn Fn(&Path) -> Vec<String>,
    read: &dyn Fn(&Path) -> Option<String>,
) -> Vec<Module> {
    let mut out: Vec<Module> = list(dir)
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| {
            let stem = p.file_stem()?.to_str()?.to_owned();
            let m = Module::parse(&read(&p)?).ok()?;
            (m.id == stem).then_some(m)
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    out
}

/// Per-user enablement file (`{"enabled": ["dev"]}`). Malformed = nothing enabled.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Enabled {
    pub enabled: Vec<String>,
}

impl Enabled {
    pub fn parse(text: Option<&str>) -> Self {
        text.and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    pub fn has(&self, id: &str) -> bool {
        self.enabled.iter().any(|e| e == id)
    }

    /// Marks `id` enabled or not; true when the file changed.
    pub fn set(&mut self, id: &str, on: bool) -> bool {
        let had = self.has(id);
        self.enabled.retain(|e| e != id);
        if on {
            self.enabled.push(id.to_owned());
            self.enabled.sort();
        }
        had != on
    }
}

/// `$XDG_STATE_HOME/wolos/modules.json`.
pub fn state_file(state_home: &Path) -> PathBuf {
    state_home.join(STATE_FILE)
}

/// `/var/lib/wolos/modules-pending`.
pub fn pending_file() -> PathBuf {
    PathBuf::from(PENDING_FILE)
}

/// Ids in the pending file: one per line, blank lines and `#` comments skipped, invalid ids dropped.
pub fn parse_pending(text: Option<&str>) -> Vec<String> {
    let mut v: Vec<String> = text
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && valid_id(l))
        .map(str::to_owned)
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Package names from `pacman -Qq` output (one per line; `pacman -Q` lines keep the first word).
pub fn installed_packages(pacman_q: &str) -> Vec<String> {
    pacman_q
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

/// What the system says now, read by the caller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// Installed package names (`pacman -Qq`).
    pub installed: Vec<String>,
    pub enabled: Enabled,
    /// Ids queued for first boot.
    pub pending: Vec<String>,
}

/// One row of `wol modules list --json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub category: String,
    pub size_mb: u32,
    /// `wolos-module-<id>`.
    pub package: String,
    /// The meta package is installed (pacman).
    pub installed: bool,
    /// The user ran the enable step (`post`, recipes, mise).
    pub enabled: bool,
    /// Queued by the installer; firstboot installs it.
    pub pending: bool,
    /// `enable` has a user-side step; false = installed is all there is.
    pub needs_enable: bool,
    pub offline: bool,
    pub needs: Vec<String>,
}

impl Status {
    /// One word for tables: `enabled` | `installed` | `pending` | `available`.
    pub fn state(&self) -> &'static str {
        match (self.installed, self.enabled, self.pending) {
            (true, true, _) => "enabled",
            (true, false, _) => "installed",
            (false, _, true) => "pending",
            _ => "available",
        }
    }
}

pub fn status(m: &Module, f: &Facts) -> Status {
    let package = m.package();
    Status {
        id: m.id.clone(),
        name: m.name.clone(),
        summary: m.summary.clone(),
        category: m.category.clone(),
        size_mb: m.size_mb,
        installed: f.installed.contains(&package),
        enabled: f.enabled.has(&m.id),
        pending: f.pending.contains(&m.id),
        needs_enable: m.has_user_step(),
        offline: m.offline,
        needs: m.needs.clone(),
        package,
    }
}

/// `wol modules list --json`: every module in registry order.
pub fn list(modules: &[Module], f: &Facts) -> Vec<Status> {
    modules.iter().map(|m| status(m, f)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEV: &str = r#"{"id":"dev","name":"Developer","summary":"Toolchains","category":"developer",
        "size_mb":900,"packages":["git","mise"],"recipes":[],"post":["/usr/lib/wolos/modules-enable-dev"]}"#;
    const CONSOLE: &str =
        r#"{"id":"console","name":"Console","category":"console","packages":["starship"]}"#;

    #[test]
    fn parse_validates_id_and_name() {
        let m = Module::parse(DEV).unwrap();
        assert_eq!(
            (m.id.as_str(), m.size_mb, m.package().as_str()),
            ("dev", 900, "wolos-module-dev")
        );
        assert!(m.has_user_step());
        assert!(!Module::parse(CONSOLE).unwrap().has_user_step());
        for bad in [
            r#"{"id":"Dev","name":"x"}"#,
            r#"{"id":"a b","name":"x"}"#,
            r#"{"id":"dev"}"#,
            r#"{"id":"dev","name":"x","post":[""]}"#,
            "{nope",
        ] {
            assert!(Module::parse(bad).is_err(), "{bad}");
        }
        for (id, ok) in [
            ("ai-claude", true),
            ("x9", true),
            ("9x", false),
            ("", false),
            ("a_b", false),
        ] {
            assert_eq!(valid_id(id), ok, "{id}");
        }
    }

    #[test]
    fn load_reads_valid_records_sorted_and_skips_the_rest() {
        let dir = Path::new("/usr/share/wolos/modules");
        let files = [
            ("/usr/share/wolos/modules/dev.json", DEV),
            ("/usr/share/wolos/modules/console.json", CONSOLE),
            ("/usr/share/wolos/modules/other.json", DEV), // stem != id
            ("/usr/share/wolos/modules/bad.json", "{"),
            ("/usr/share/wolos/modules/README", "text"),
        ];
        let list = |p: &Path| -> Vec<String> {
            assert_eq!(p, dir);
            files.iter().map(|f| f.0.to_owned()).collect()
        };
        let read = |p: &Path| -> Option<String> {
            files
                .iter()
                .find(|f| Path::new(f.0) == p)
                .map(|f| f.1.to_owned())
        };
        let ms = load(dir, &list, &read);
        let ids: Vec<&str> = ms.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["console", "dev"]);
        assert!(load(dir, &|_| vec![], &read).is_empty());
    }

    #[test]
    fn status_from_pacman_state_and_pending() {
        let dev = Module::parse(DEV).unwrap();
        let console = Module::parse(CONSOLE).unwrap();
        let mut f = Facts {
            installed: installed_packages("git 2.47.0-1\nwolos-module-dev 1-1\n"),
            enabled: Enabled::parse(Some(r#"{"enabled":["dev"]}"#)),
            pending: parse_pending(Some("# queued\nconsole\n\nBad Id\nconsole\n")),
        };
        assert_eq!(f.pending, ["console"]);
        let s = list(&[dev.clone(), console.clone()], &f);
        assert_eq!(
            (s[0].state(), s[0].installed, s[0].enabled),
            ("enabled", true, true)
        );
        assert_eq!(
            (s[1].state(), s[1].pending, s[1].needs_enable),
            ("pending", true, false)
        );
        assert_eq!(s[0].package, "wolos-module-dev");
        f.enabled.set("dev", false);
        assert_eq!(status(&dev, &f).state(), "installed");
        f.pending.clear();
        assert_eq!(status(&console, &f).state(), "available");
        let j = serde_json::to_value(&s[0]).unwrap();
        assert_eq!(j["installed"], true);
        assert_eq!(j["size_mb"], 900);
    }

    #[test]
    fn enabled_file_round_trips_and_fails_closed() {
        let mut e = Enabled::parse(Some("{broken"));
        assert!(e.enabled.is_empty());
        assert!(e.set("dev", true));
        assert!(!e.set("dev", true), "idempotent");
        assert!(e.set("ai-claude", true));
        assert_eq!(e.enabled, ["ai-claude", "dev"]);
        assert_eq!(Enabled::parse(Some(&e.to_json())), e);
        assert!(e.set("dev", false));
        assert!(!e.has("dev"));
        assert_eq!(
            state_file(Path::new("/home/a/.local/state")),
            Path::new("/home/a/.local/state/wolos/modules.json")
        );
        assert_eq!(pending_file(), Path::new(PENDING_FILE));
        assert_eq!(parse_pending(None), Vec::<String>::new());
    }
}
