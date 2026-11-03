// SPDX-License-Identifier: BSD-2-Clause
//! The permission catalog of system web apps (ADR 0048): every name an `app.toml` may list, the
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
    /// The consent capability (`wol_core::consent::CAPABILITIES`) that grants it to an app that
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
    fn consent_capabilities_exist() {
        for p in PERMISSIONS {
            if let Some(c) = p.consent {
                assert!(wol_core::consent::CAPABILITIES.contains(&c), "{c}");
            }
        }
    }
}
