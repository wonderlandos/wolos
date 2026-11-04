// SPDX-License-Identifier: GPL-2.0-or-later
//! desktop/overlay/usr/bin/wol-session against fakes (B3, QA-LIVE-33 / live-2 L2-35): a settings
//! folder the account cannot write still starts the session, on the WolOS defaults.
#[path = "support/repo.rs"]
mod repo;
use repo::repo;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

/// The compositor arguments wol-session ends with (a fake dbus-run-session logs them) and its stderr.
fn start(tag: &str, home_setup: impl Fn(&std::path::Path)) -> (String, String) {
    let d = std::env::temp_dir().join(format!("wol-session-b3-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("bin")).unwrap();
    let log = d.join("log");
    let fake = d.join("bin/dbus-run-session");
    fs::write(
        &fake,
        format!("#!/bin/sh\necho \"$*\" >> '{}'\n", log.display()),
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let home = d.join("home");
    fs::create_dir_all(&home).unwrap();
    home_setup(&home);
    let out = Command::new("sh")
        .arg(repo("desktop/overlay/usr/bin/wol-session"))
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("HOME", &home)
        .env("XDG_RUNTIME_DIR", &d)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("WOL_SESSION_LOCKED")
        .env("WOLOS_SHARE", "/usr/share/wolos")
        .output()
        .unwrap();
    // Give the folders back so the temp dir can be removed.
    for p in [".config", ".cache", ".local"] {
        let _ = fs::set_permissions(home.join(p), fs::Permissions::from_mode(0o755));
    }
    let args = fs::read_to_string(&log).unwrap_or_default();
    let _ = fs::remove_dir_all(&d);
    (args, String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn unwritable_config_starts_on_the_defaults() {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipped: root writes through the folder's mode");
        return;
    }
    // A normal home: the account's own config, no extra arguments (image_seamless pins the rest).
    let (args, _) = start("ok", |_| {});
    assert_eq!(args, "-- start-hyprland\n");
    // chmod 000 ~/.config ~/.cache (live-2 L2-35): the defaults' file, and the reason on stderr.
    let (args, err) = start("ro", |h| {
        for p in [".config", ".cache"] {
            fs::create_dir_all(h.join(p)).unwrap();
            fs::set_permissions(h.join(p), fs::Permissions::from_mode(0o000)).unwrap();
        }
    });
    assert_eq!(
        args, "-- start-hyprland -- --config /usr/share/wolos/hypr/hyprland.lua\n",
        "{err}"
    );
    assert!(err.contains("starting on the WolOS defaults"), "{err}");
}
