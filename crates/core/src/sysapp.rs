// SPDX-License-Identifier: BSD-2-Clause
//! The permission catalog of system web apps (ADR 0048), in core because three programs read it: the
//! web-app host (`webengine::bridge`), wold (which checks every call again) and the tests. Also the
//! little of `app.toml` that wold needs ([`declared`]).
//!
//! The catalog:: every name an `app.toml` may list, the
//! bridge methods it opens and the consent grant behind it. A method is a `wol` verb's action
//! name (`audio.doctor`), its params the verb's named arguments, its result the verb's `--json`
//! output, so the bridge adds no privileged surface: wold runs the same verb, after checking the
//! method, the caller's app id and the grant.

/// One permission an app may declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permission {
    /// `area.read` or `area.control` style name, the word an app lists.
    pub name: &'static str,
    /// Bridge methods it opens (the verbs' action names), each in exactly one permission.
    pub methods: &'static [&'static str],
    /// The consent capability (`crate::consent::CAPABILITIES`) that grants it to an app that
    /// is not part of the system image; `None`: only system apps (installed with the sealed image)
    /// hold it, and they hold it because they declare it.
    pub consent: Option<&'static str>,
    /// Its methods change the machine for every account: wold shows the administrator sheet.
    pub admin: bool,
    /// What Settings > Privacy says it allows, in plain words.
    pub about: &'static str,
}

const fn p(
    name: &'static str,
    methods: &'static [&'static str],
    consent: Option<&'static str>,
    admin: bool,
    about: &'static str,
) -> Permission {
    Permission {
        name,
        methods,
        consent,
        admin,
        about,
    }
}

/// Every permission an app may list, in the order Settings shows them.
pub const PERMISSIONS: &[Permission] = &[
    p(
        "system.read",
        &[
            "about",
            "sys",
            "hw.status",
            "power.status",
            "battery",
            "boot.status",
            "modules.status",
        ],
        None,
        false,
        "See this computer's model, storage, power and hardware",
    ),
    p(
        "update.read",
        &["update.status", "update.check"],
        None,
        false,
        "See which updates are ready",
    ),
    p(
        "packages.update",
        &["update.apply"],
        Some("packages-update"),
        true,
        "Install updates, after you approve each time",
    ),
    p(
        "security.read",
        &["security.status", "firewall.status"],
        None,
        false,
        "See the firewall, remote login and malware check",
    ),
    p(
        "network.read",
        &["network", "wifi.get", "bluetooth.get"],
        Some("network-info"),
        false,
        "See whether you are online and which network you use",
    ),
    p(
        "audio.read",
        &["audio.get", "audio.doctor"],
        Some("audio-info"),
        false,
        "See your sound devices and the sound check",
    ),
    p(
        "audio.control",
        &["audio.volume", "audio.mute", "audio.unmute", "audio.reset"],
        Some("audio-control"),
        false,
        "Change the volume and reset sound settings",
    ),
    p(
        "games.read",
        &["games.doctor"],
        None,
        false,
        "See what games need on this computer",
    ),
    // The Doctor app's verbs (`wol doctor`, from the Doctor web app's branch).
    p(
        "doctor.read",
        &["doctor.check"],
        None,
        false,
        "Check this computer for problems",
    ),
    p(
        "doctor.fix",
        &["doctor.fix"],
        None,
        true,
        "Fix a problem the check found, after you approve",
    ),
    p(
        "notifications",
        &["notify"],
        Some("notifications"),
        false,
        "Show notifications",
    ),
];

/// Topics a page may subscribe to: `app` (the `wolos.app` record changed: theme, accent,
/// locale), `permissions` (the granted list changed), `progress` (a long call's
/// `{method, done, total, label}`).
pub const TOPICS: &[&str] = &["app", "permissions", "progress"];

/// The permission called `name`.
pub fn permission(name: &str) -> Option<&'static Permission> {
    PERMISSIONS.iter().find(|p| p.name == name)
}

/// The permission that opens `method`.
pub fn permission_of(method: &str) -> Option<&'static Permission> {
    PERMISSIONS.iter().find(|p| p.methods.contains(&method))
}

/// A well-formed permission name: lower-case words joined by one dot at most.
pub fn is_name(name: &str) -> bool {
    let word = |w: &str| !w.is_empty() && w.bytes().all(|b| b.is_ascii_lowercase());
    match name.split_once('.') {
        Some((a, b)) => word(a) && word(b),
        None => word(name),
    }
}

/// Where the image installs system web apps: `<SYSTEM_DIR>/<slug>/app.toml`.
pub const SYSTEM_DIR: &str = "/usr/share/wolos/webapps";
/// Where an app of others installs (a package, ADR 0048): `<OPT_DIR>/<slug>/app.toml`.
pub const OPT_DIR: &str = "/opt/wolos/webapps";
/// Prefix of a web app's id (`webengine::process::APP_ID_PREFIX`).
pub const APP_ID_PREFIX: &str = "org.wolos.webapp.";

/// The slug of a web app id (`org.wolos.webapp.doctor` -> `doctor`), when it is well formed.
pub fn slug_of(app_id: &str) -> Option<&str> {
    let s = app_id.strip_prefix(APP_ID_PREFIX)?;
    let ok = !s.is_empty()
        && s != "kit"
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    ok.then_some(s)
}

/// What an `app.toml` declares that wold uses: its id and its permission names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared {
    pub id: String,
    pub permissions: Vec<String>,
}

/// Reads `id` and `permissions` of an `app.toml` (other keys are the host's business). An error is a
/// sentence for a log.
pub fn declared(text: &str) -> Result<Declared, String> {
    let t: toml::Table = text.parse().map_err(|e| format!("app.toml: {e}"))?;
    let id = t
        .get("id")
        .and_then(toml::Value::as_str)
        .ok_or("app.toml: id is missing")?
        .to_owned();
    let permissions = match t.get("permissions") {
        None => Vec::new(),
        Some(toml::Value::Array(a)) => a
            .iter()
            .map(|v| v.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .ok_or("app.toml: permissions is a list of names")?,
        Some(_) => return Err("app.toml: permissions is a list of names".into()),
    };
    Ok(Declared { id, permissions })
}

/// The scope unit's app id, for a cgroup path component `app-wol-<app id>-<pid>-<n>.scope` made by
/// the host (`webengine::process::scope_unit`) for the process `pid`; `None` for any other name.
pub fn app_of_scope(unit: &str, pid: i32) -> Option<String> {
    let rest = unit.strip_prefix("app-wol-")?.strip_suffix(".scope")?;
    let (head, n) = rest.rsplit_once('-')?;
    let (app, p) = head.rsplit_once('-')?;
    let hex = !n.is_empty() && n.bytes().all(|b| b.is_ascii_hexdigit());
    (hex && p.parse::<i32>().ok() == Some(pid) && slug_of(app).is_some()).then(|| app.to_owned())
}

/// The web app a process belongs to, from its `/proc/<pid>/cgroup` text: the first path component
/// that is the app scope of `pid` ([`app_of_scope`]).
pub fn app_of_cgroup(cgroup: &str, pid: i32) -> Option<String> {
    cgroup
        .lines()
        .flat_map(|l| l.rsplit(':').next().unwrap_or("").split('/'))
        .find_map(|c| app_of_scope(c, pid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_well_formed_and_methods_have_one_owner() {
        let mut seen = std::collections::BTreeSet::new();
        for p in PERMISSIONS {
            assert!(is_name(p.name), "{}", p.name);
            assert!(seen.insert(p.name), "{} twice", p.name);
            assert!(!p.methods.is_empty(), "{} opens nothing", p.name);
            for m in p.methods {
                assert_eq!(permission_of(m).map(|q| q.name), Some(p.name), "{m}");
            }
        }
        assert!(!is_name("System.read") && !is_name("a.b.c") && !is_name(".a"));
    }

    #[test]
    fn the_app_is_told_by_its_scope_and_pid() {
        let u = "app-wol-org.wolos.webapp.doctor-4242-1f.scope";
        assert_eq!(
            app_of_scope(u, 4242).as_deref(),
            Some("org.wolos.webapp.doctor")
        );
        assert_eq!(app_of_scope(u, 4243), None, "another pid's scope");
        for bad in [
            "app-wol-org.wolos.webapp.kit-4242-1f.scope",
            "app-wol-org.other.thing-4242-1f.scope",
            "app-wol-org.wolos.webapp.doctor-4242.scope",
            "app-wol-org.wolos.webapp.doctor-4242-zz.scope",
            "app-wol-org.wolos.webapp.Doctor-4242-1f.scope",
            "app-gnome-foo-4242.scope",
        ] {
            assert_eq!(app_of_scope(bad, 4242), None, "{bad}");
        }
        let cg = format!("0::/user.slice/user-1000.slice/user@1000.service/app.slice/{u}\n");
        assert_eq!(
            app_of_cgroup(&cg, 4242).as_deref(),
            Some("org.wolos.webapp.doctor")
        );
        assert_eq!(
            app_of_cgroup("0::/user.slice/session-2.scope\n", 4242),
            None
        );
        assert_eq!(slug_of("org.wolos.webapp.my-notes"), Some("my-notes"));
        assert_eq!(slug_of("org.wolos.webapp."), None);
    }

    #[test]
    fn a_manifest_gives_its_id_and_permissions() {
        let d = declared("id='org.wolos.webapp.t'\nname='T'\npermissions=['audio.read']\nfoo=1\n")
            .unwrap();
        assert_eq!(d.id, "org.wolos.webapp.t");
        assert_eq!(d.permissions, ["audio.read"]);
        assert!(declared("name='T'").is_err());
        assert_eq!(
            declared("id='x'").unwrap().permissions,
            Vec::<String>::new()
        );
    }

    #[test]
    fn consent_capabilities_exist() {
        for p in PERMISSIONS {
            if let Some(c) = p.consent {
                assert!(crate::consent::CAPABILITIES.contains(&c), "{c}");
            }
        }
    }
}
