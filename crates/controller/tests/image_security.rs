// SPDX-License-Identifier: GPL-2.0-or-later
//! Security defaults the image ships (wol_core::security, docs/modules/security.md): the
//! firewall on from the first boot, kernel hardening, the system log off the network and the
//! compositor's privacy rules.
#[path = "support/repo.rs"]
mod repo;
use repo::read;
use wol_core::security::{
    HARDENING, HARDENING_FILE, HARDENING_MASK, HYPR_PERMISSIONS, hypr_permissions,
};

#[test]
fn compositor_privacy_rules_ship_and_load() {
    let rel = format!("desktop/overlay{HYPR_PERMISSIONS}");
    assert_eq!(read(&rel), hypr_permissions(), "{rel}: regenerate");
    let conf = read("desktop/overlay/usr/share/wolos/hypr/hyprland.conf");
    let source = format!("source = {HYPR_PERMISSIONS}");
    let at = conf.find(&source).expect("hyprland.conf sources the rules");
    let plugin = conf.find("\nplugin = ").unwrap();
    assert!(at < plugin, "rules before the plugin lines");
    for p in conf.lines().filter_map(|l| l.strip_prefix("plugin = ")) {
        assert!(
            p.starts_with(wol_core::security::PLUGIN_DIR) && p.ends_with(".so"),
            "{p}: plugins load only from the plugin folder"
        );
    }
}

/// `security::HARDENING_FILE` is the file the image really ships (it once named /etc/sysctl.d,
/// where nothing ships), it sets every `security::HARDENING` value, and nothing in the image masks
/// it with a same-name /etc file.
#[test]
fn hardening_file_is_the_shipped_one() {
    let rel = format!("files{HARDENING_FILE}");
    assert!(
        HARDENING_FILE.starts_with("/usr/lib/sysctl.d/"),
        "{HARDENING_FILE}: the shipped default lives in /usr/lib/sysctl.d"
    );
    let conf = read(&rel);
    for (k, v, _) in HARDENING {
        let kv = format!("{k} = {v}");
        assert!(conf.lines().any(|l| l == kv), "{rel} lacks {kv}");
    }
    for root in ["files", "desktop/overlay"] {
        let mask = format!("{root}{HARDENING_MASK}");
        assert!(
            !repo::repo(&mask).exists(),
            "{mask} would replace {HARDENING_FILE} whole"
        );
    }
    assert_eq!(
        HARDENING_MASK.rsplit('/').next(),
        HARDENING_FILE.rsplit('/').next(),
        "the mask is the same name in /etc/sysctl.d"
    );
}

/// sysctl.d order (systemd-sysctl): every file from /etc, /run and /usr/lib, one per name, applied
/// in byte order of the file name, the last value wins. wol-admind's `power::SYSCTL_LOCAL` must
/// sort after every shipped file, or the default would override a local value (`90-wolos-local`
/// once sorted before `90-wolos`, `-` < `.`), and must not share a shipped name (it would mask
/// that file whole).
#[test]
fn local_sysctl_sorts_after_the_shipped_defaults() {
    use wol_core::power::SYSCTL_LOCAL;
    let name = |p: &str| p.rsplit('/').next().unwrap().to_owned();
    assert!(
        SYSCTL_LOCAL.starts_with("/etc/sysctl.d/"),
        "{SYSCTL_LOCAL}: a local change lives in /etc"
    );
    let local = name(SYSCTL_LOCAL);
    let mut shipped: Vec<String> = std::fs::read_dir(repo::repo("files/usr/lib/sysctl.d"))
        .expect("files/usr/lib/sysctl.d")
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".conf"))
        .collect();
    shipped.sort();
    assert!(shipped.contains(&name(HARDENING_FILE)), "{shipped:?}");
    for f in &shipped {
        assert!(
            f.as_bytes() < local.as_bytes(),
            "{f} sorts after {local}: its defaults would override local values"
        );
    }
    assert!(
        b"90-wolos-local.conf".as_slice() < b"90-wolos.conf".as_slice(),
        "the old name sorted first"
    );
    for root in ["files", "desktop/overlay"] {
        let rel = format!("{root}{SYSCTL_LOCAL}");
        assert!(
            !repo::repo(&rel).exists(),
            "{rel}: the local file is the machine's, not the image's"
        );
    }
}
