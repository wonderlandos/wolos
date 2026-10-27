//! Security defaults the image ships (wol_core::security, archicture/modules/security.md): the
//! firewall on from the first boot, kernel hardening, the system log off the network and the
//! compositor's privacy rules.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use wol_core::security::{HARDENING, HYPR_PERMISSIONS, RC_DEFAULTS, hypr_permissions};

fn repo(p: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|_| panic!("{p} missing"))
}

fn lines(s: &str) -> Vec<&str> {
    s.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .collect()
}

/// image/rcconf.sh (live root) with a fake sysrc: the rc.conf settings it writes.
fn rcconf_log() -> String {
    let d = std::env::temp_dir().join(format!("wol-security-rcconf-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("bin")).unwrap();
    fs::create_dir_all(d.join("root/etc")).unwrap();
    fs::write(d.join("root/etc/auto_master"), "").unwrap();
    let sysrc = d.join("bin/sysrc");
    fs::write(&sysrc, "#!/bin/sh\necho \"$@\" >> \"$LOG\"\n").unwrap();
    fs::set_permissions(&sysrc, fs::Permissions::from_mode(0o755)).unwrap();
    let st = Command::new("sh")
        .arg(repo("image/rcconf.sh"))
        .arg(d.join("root"))
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("LOG", d.join("log"))
        .status()
        .unwrap();
    assert!(st.success());
    let log = fs::read_to_string(d.join("log")).unwrap();
    let _ = fs::remove_dir_all(&d);
    log
}

#[test]
fn firewall_and_quiet_services_are_on_in_live_and_installed_systems() {
    let log = rcconf_log();
    let inst = read("image/installerconfig");
    for (k, v, why) in RC_DEFAULTS {
        let kv = format!("{k}={v}");
        assert!(log.contains(&kv), "rcconf.sh lacks {kv} ({why}):\n{log}");
        assert!(inst.contains(&kv), "installerconfig lacks {kv} ({why})");
    }
    for f in [log.as_str(), inst.as_str()] {
        assert!(
            !f.contains("sshd_enable=YES"),
            "no inbound remote login by default"
        );
    }
    assert!(repo(&format!("desktop/overlay{}", wol_core::sharing::PF_RULES)).is_file());
}

#[test]
fn kernel_hardening_is_set_at_boot() {
    let f = read("desktop/overlay/etc/sysctl.conf.local");
    let set = lines(&f);
    for (k, v, why) in HARDENING {
        assert!(
            set.contains(&format!("{k}={v}").as_str()),
            "sysctl.conf.local lacks {k}={v} ({why})"
        );
        let at = f.find(&format!("\n{k}={v}")).unwrap();
        let comment = f[..at].lines().last().unwrap_or("");
        assert!(comment.starts_with("# "), "{k}: one-line reason above it");
    }
}

#[test]
fn compositor_privacy_rules_ship_and_load() {
    let rel = format!("desktop/overlay{HYPR_PERMISSIONS}");
    assert_eq!(read(&rel), hypr_permissions(&[]), "{rel}: regenerate");
    let conf = read("desktop/overlay/usr/local/share/wolos/hypr/hyprland.conf");
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
