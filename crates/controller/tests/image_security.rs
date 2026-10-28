//! Security defaults the image ships (wol_core::security, archicture/modules/security.md): the
//! firewall on from the first boot, kernel hardening, the system log off the network and the
//! compositor's privacy rules.
use std::fs;
use std::path::{Path, PathBuf};
use wol_core::security::{HYPR_PERMISSIONS, hypr_permissions};

fn repo(p: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|_| panic!("{p} missing"))
}

#[test]
fn compositor_privacy_rules_ship_and_load() {
    let rel = format!("desktop/overlay{HYPR_PERMISSIONS}");
    assert_eq!(read(&rel), hypr_permissions(&[]), "{rel}: regenerate");
    let conf = read("desktop/overlay/usr/share/wolos/hypr/hyprland.conf");
    // Arch layout: the overlay moved from /usr/local to /usr; wol_core still names /usr/local
    // until its paths follow (ADR 0016).
    let arch = |p: &str| p.replace("/usr/local/", "/usr/");
    let source = format!("source = {}", arch(HYPR_PERMISSIONS));
    let at = conf.find(&source).expect("hyprland.conf sources the rules");
    let plugin = conf.find("\nplugin = ").unwrap();
    assert!(at < plugin, "rules before the plugin lines");
    for p in conf.lines().filter_map(|l| l.strip_prefix("plugin = ")) {
        assert!(
            p.starts_with(&arch(wol_core::security::PLUGIN_DIR)) && p.ends_with(".so"),
            "{p}: plugins load only from the plugin folder"
        );
    }
}
