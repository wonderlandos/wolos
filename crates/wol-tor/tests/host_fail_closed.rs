// SPDX-License-Identifier: GPL-2.0-or-later
//! The `wol-tor host` verb never starts Atlas when its Tor profile is not in place (ADR 0053 D5).
use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-tor-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn host(runtime: &std::path::Path, mark: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_wol-tor"))
        .args(["host", "--group", "t", "--", "/bin/sh", "-c"])
        .arg(format!("touch {}", mark.display()))
        .env("XDG_RUNTIME_DIR", runtime)
        .output()
        .unwrap()
}

#[test]
fn atlas_does_not_start_without_the_profile() {
    let run = scratch("noprofile");
    let mark = run.join("atlas-started");
    let out = host(&run, &mark);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Tor profile is not in place"), "{err}");
    assert!(!mark.exists(), "Atlas must not have run");
    let _ = std::fs::remove_dir_all(&run);
}

#[test]
fn the_written_profile_has_the_files_and_keys_before_atlas_is_launched() {
    let run = scratch("written");
    let dir = run.join("profile");
    let out = Command::new(env!("CARGO_BIN_EXE_wol-tor"))
        .args(["policies", "--write"])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("policies/managed/tor.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(doc["ProxySettings"]["ProxyMode"], "fixed_servers");
    assert_eq!(doc["BrowserThemeColor"], "#6A4DF4");
    assert!(dir.join("fonts.conf").is_file());
    // With the profile in the place `host` reads, the profile check passes and the next gate
    // (the isolated namespace) decides; a failure now must not be about the profile.
    let paths_profile = run.join("wol-tor-profile/t");
    std::fs::create_dir_all(paths_profile.parent().unwrap()).unwrap();
    std::fs::rename(&dir, &paths_profile).unwrap();
    let mark = run.join("atlas-started");
    let out = host(&run, &mark);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("Tor profile is not in place"), "{err}");
    let _ = std::fs::remove_dir_all(&run);
}

#[test]
fn keep_tor_ready_is_off_by_default_and_off_writes_no_unit() {
    let cfg = scratch("keepready");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_wol-tor"))
            .arg("keep-ready")
            .args(args)
            .env("XDG_CONFIG_HOME", &cfg)
            .env("HOME", &cfg)
            .output()
            .unwrap()
    };
    let out = run(&["status"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "off");
    assert!(
        !cfg.join("systemd").exists() && !cfg.join("wol-tor").exists(),
        "reading the switch writes nothing"
    );
    let off = run(&["off"]);
    assert!(
        off.status.success(),
        "{}",
        String::from_utf8_lossy(&off.stderr)
    );
    assert!(!cfg.join("systemd/user/wol-tor-cache.timer").exists());
    assert!(!cfg.join("systemd/user/wol-tor-cache.service").exists());
    assert_eq!(
        String::from_utf8_lossy(&run(&["status"]).stdout).trim(),
        "off"
    );
    let bad = run(&["maybe"]);
    assert!(!bad.status.success());
    let _ = std::fs::remove_dir_all(&cfg);
}

#[test]
fn cache_run_refuses_without_the_unit_switch() {
    let run = scratch("cacherun");
    let out = Command::new(env!("CARGO_BIN_EXE_wol-tor"))
        .arg("cache-run")
        .env("RUNTIME_DIRECTORY", &run)
        .env_remove("WOL_TOR_KEEP_READY")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("keep-ready on"));
    let _ = std::fs::remove_dir_all(&run);
}
