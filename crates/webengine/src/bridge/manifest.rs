// SPDX-License-Identifier: BSD-2-Clause
//! `app.toml`, a system web app's manifest (ADR 0048): one folder `webapps/<slug>/` holds it,
//! the entry page, the icon and the app's own static files. Unknown keys are refused, so a typo
//! never passes as a default.
use super::catalog;
use crate::policy::Policy;
use crate::process::APP_ID_PREFIX;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Where the image installs system web apps: `<SYSTEM_DIR>/<slug>/`, the kit at `<SYSTEM_DIR>/kit/`.
pub use wol_core::sysapp::SYSTEM_DIR;
/// Where an app of others (a package from the hub, ADR 0048) installs: `<OPT_DIR>/<slug>/`. There is no kit there.
pub use wol_core::sysapp::OPT_DIR;
/// The shared kit's folder name next to the apps (never an app slug).
pub const KIT: &str = "kit";
pub const FILE: &str = "app.toml";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// `org.wolos.webapp.<slug>`, the slug being the folder's name.
    pub id: String,
    /// The window title and launcher name, plain English until the locale system lands.
    pub name: String,
    /// `major.minor.patch`.
    pub version: String,
    /// The page the window opens, relative to the folder (`index.html`).
    #[serde(default = "entry")]
    pub entry: String,
    /// The launcher and window icon, relative to the folder (`.svg` or `.png`).
    pub icon: String,
    /// One line for the launcher's tooltip and the Store.
    #[serde(default)]
    pub summary: String,
    /// Names from [`catalog::PERMISSIONS`].
    #[serde(default)]
    pub permissions: Vec<String>,
}

fn entry() -> String {
    "index.html".into()
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, String> {
        toml::from_str(text).map_err(|e| format!("{FILE}: {e}"))
    }

    /// The slug of [`Manifest::id`].
    pub fn slug(&self) -> &str {
        self.id.strip_prefix(APP_ID_PREFIX).unwrap_or("")
    }

    /// Every rule that needs no files: id, version, paths, permissions.
    pub fn check(&self) -> Result<(), String> {
        let slug = self.slug();
        if slug.is_empty()
            || slug == KIT
            || !slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || slug.starts_with('-')
        {
            return Err(format!(
                "id {:?}: {APP_ID_PREFIX}<slug>, slug a-z 0-9 -",
                self.id
            ));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 40 {
            return Err("name: 1 to 40 characters".into());
        }
        if self.summary.chars().count() > 120 {
            return Err("summary: at most 120 characters".into());
        }
        // Both reach a desktop entry line: no line breaks, no control characters, no escapes.
        if [&self.name, &self.summary]
            .iter()
            .any(|t| t.chars().any(|c| c.is_control() || c == '\\'))
        {
            return Err("name and summary: one line of plain text".into());
        }
        let v: Vec<&str> = self.version.split('.').collect();
        if v.len() != 3
            || !v
                .iter()
                .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(format!("version {:?}: major.minor.patch", self.version));
        }
        rel(&self.entry, &[".html"]).map_err(|e| format!("entry: {e}"))?;
        rel(&self.icon, &[".svg", ".png"]).map_err(|e| format!("icon: {e}"))?;
        for (i, p) in self.permissions.iter().enumerate() {
            if catalog::permission(p).is_none() {
                return Err(format!("permission {p:?} is not in the catalog"));
            }
            if self.permissions[..i].contains(p) {
                return Err(format!("permission {p:?} twice"));
            }
        }
        Ok(())
    }

    /// Reads and checks `<dir>/app.toml`: the folder is named after the slug and the entry and icon exist.
    pub fn load(dir: &Path) -> Result<Manifest, String> {
        let text = std::fs::read_to_string(dir.join(FILE))
            .map_err(|e| format!("{}: {e}", dir.join(FILE).display()))?;
        let m = Manifest::parse(&text)?;
        m.check()?;
        let folder = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if folder != m.slug() {
            return Err(format!("folder {folder:?} is not the slug {:?}", m.slug()));
        }
        for f in [&m.entry, &m.icon] {
            if !dir.join(f).is_file() {
                return Err(format!("{f}: no such file"));
            }
        }
        Ok(m)
    }

    /// The runtime policy (ADR 0042) of an app installed in `dir` with the kit in `kit`: its own
    /// folder and the kit, read-only, and no network.
    pub fn policy(&self, dir: &Path, kit: &Path) -> Policy {
        let folder =
            |p: &Path| format!("file://{}/", p.display().to_string().trim_end_matches('/'));
        Policy {
            origins: vec![folder(dir), folder(kit)],
            ..Policy::default()
        }
    }
}

/// The kit an app whose folder sits in `root` loads: the image's own, `<SYSTEM_DIR>/kit/`, for an app under
/// `<OPT_DIR>` (a package from the hub never ships a kit, and one that put a `kit` folder next to the apps
/// would not be believed); else the `kit` next to the apps (the image's own apps, and a checkout's `webapps/`
/// in development). Both arguments are canonical paths.
pub fn kit_for(root: &Path, system: &Path, opt: &Path) -> PathBuf {
    if root == opt {
        system.join(KIT)
    } else {
        root.join(KIT)
    }
}

impl Manifest {
    /// The launcher entry of an app installed in `dir` (`/usr/share/applications/<id>.desktop`):
    /// the runtime's host with the manifest, the app's icon, and the app id as the window class, so
    /// the shell matches the window to its launcher and the Memory Saver to its scope.
    pub fn desktop_entry(&self, dir: &Path) -> String {
        let dir = dir.display().to_string();
        let dir = dir.trim_end_matches('/');
        let mut s = format!("[Desktop Entry]\nType=Application\nName={}\n", self.name);
        if !self.summary.is_empty() {
            s.push_str(&format!("Comment={}\n", self.summary));
        }
        s.push_str(&format!(
            "Exec={HOST} {dir}/{FILE}\nIcon={dir}/{}\nTerminal=false\nCategories=System;\nStartupWMClass={}\nX-WolOS-WebApp={}\n",
            self.icon, self.id, self.version
        ));
        s
    }
}

/// The runtime's host process (ADR 0042), one per app id.
pub const HOST: &str = "/usr/bin/wol-webhost";

/// A plain relative path inside the folder, ending in one of `ext`.
fn rel(path: &str, ext: &[&str]) -> Result<(), String> {
    let ok = !path.is_empty()
        && !path.starts_with('/')
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != ".." && !s.contains('\\'))
        && ext.iter().any(|e| path.ends_with(e));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{path:?}: a path inside the folder ending in {ext:?}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
id = "org.wolos.webapp.hello"
name = "Hello"
version = "0.1.0"
icon = "icon.svg"
permissions = ["system.read"]
"#;

    #[test]
    fn a_good_manifest_passes_with_defaults() {
        let m = Manifest::parse(GOOD).unwrap();
        assert_eq!(m.entry, "index.html");
        assert_eq!(m.slug(), "hello");
        m.check().unwrap();
    }

    #[test]
    fn bad_manifests_are_refused() {
        let with = |from: &str, to: &str| Manifest::parse(&GOOD.replace(from, to));
        assert!(with("icon =", "colour = \"x\"\nicon =").is_err());
        for (from, to) in [
            ("webapp.hello", "webapp.Hello"),
            ("webapp.hello", "webapp.kit"),
            ("org.wolos.webapp.hello", "hello"),
            ("0.1.0", "1.0"),
            ("icon.svg", "../icon.svg"),
            ("icon.svg", "icon.gif"),
            ("system.read", "root.everything"),
            ("\"system.read\"", "\"system.read\", \"system.read\""),
        ] {
            assert!(with(from, to).unwrap().check().is_err(), "{to}");
        }
    }

    #[test]
    fn the_desktop_entry_launches_the_host_with_the_manifest() {
        let m = Manifest::parse(GOOD).unwrap();
        assert_eq!(
            m.desktop_entry(Path::new("/usr/share/wolos/webapps/hello/")),
            "[Desktop Entry]\nType=Application\nName=Hello\n\
             Exec=/usr/bin/wol-webhost /usr/share/wolos/webapps/hello/app.toml\n\
             Icon=/usr/share/wolos/webapps/hello/icon.svg\nTerminal=false\nCategories=System;\n\
             StartupWMClass=org.wolos.webapp.hello\nX-WolOS-WebApp=0.1.0\n"
        );
        let mut bad = m.clone();
        bad.name = "Hello\nExec=x".into();
        assert!(bad.check().is_err());
    }

    #[test]
    fn an_app_under_opt_loads_the_kit_of_the_image() {
        let (system, opt) = (Path::new("/usr/share/wolos/webapps"), Path::new("/opt/wolos/webapps"));
        assert_eq!(kit_for(system, system, opt), system.join("kit"));
        assert_eq!(kit_for(opt, system, opt), Path::new("/usr/share/wolos/webapps/kit"));
        // a checkout in development keeps its own kit
        assert_eq!(kit_for(Path::new("/src/wolos/webapps"), system, opt), Path::new("/src/wolos/webapps/kit"));
        let m = Manifest::parse(GOOD).unwrap();
        let p = m.policy(Path::new("/opt/wolos/webapps/hello"), &kit_for(opt, system, opt));
        assert_eq!(
            p.origins,
            ["file:///opt/wolos/webapps/hello/", "file:///usr/share/wolos/webapps/kit/"]
        );
    }

    #[test]
    fn the_policy_names_the_folder_and_the_kit_only() {
        let m = Manifest::parse(GOOD).unwrap();
        let p = m.policy(
            Path::new("/usr/share/wolos/webapps/hello"),
            Path::new("/usr/share/wolos/webapps/kit/"),
        );
        assert_eq!(
            p.origins,
            [
                "file:///usr/share/wolos/webapps/hello/",
                "file:///usr/share/wolos/webapps/kit/"
            ]
        );
        assert!(p.features.is_empty());
    }
}
