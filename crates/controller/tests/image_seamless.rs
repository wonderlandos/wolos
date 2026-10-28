//! One picture from power-on to the desktop (ADR 0012): loader splash, black console, locked start,
//! compositor colours. Scripts run against fakes; configs are checked line by line.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo(p: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn sandbox(tag: &str, fakes: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-seamless-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("bin")).unwrap();
    for (name, body) in fakes {
        let p = d.join("bin").join(name);
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    d
}

fn sh_n(p: &str) {
    let st = Command::new("sh").arg("-n").arg(repo(p)).status().unwrap();
    assert!(st.success(), "sh -n {p}");
}

/// `key = value` lines of one Hyprland section (`misc {` .. `}`), whitespace trimmed.
fn section(conf: &str, name: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut inside = false;
    for l in conf.lines().map(str::trim) {
        if l.starts_with(&format!("{name} {{")) {
            inside = true;
        } else if inside && l == "}" {
            inside = false;
        } else if inside && let Some((k, v)) = l.split_once('=') {
            let v = v.split('#').next().unwrap_or("").trim();
            out.push((k.trim().to_string(), v.to_string()));
        }
    }
    out
}

fn value(conf: &str, sect: &str, key: &str) -> Option<String> {
    section(conf, sect)
        .into_iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

#[test]
fn locked_start_hands_hyprland_the_lock_screen() {
    let d = sandbox(
        "session",
        &[(
            "dbus-run-session",
            "#!/bin/sh\necho \"$* | locked=${WOL_SESSION_LOCKED:-} crash=${WOLOS_CRASH_LOCK_CMD:-}\" >> \"$HOME/log\"\n",
        )],
    );
    let home = d.join("home");
    for locked in [true, false] {
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        let mut c = Command::new("sh");
        c.arg(repo("desktop/overlay/usr/bin/wol-session"))
            .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
            .env("HOME", &home)
            .env("XDG_RUNTIME_DIR", &d)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env_remove("WOL_SESSION_LOCKED")
            .env_remove("WOLOS_CRASH_LOCK_CMD");
        if locked {
            c.env("WOL_SESSION_LOCKED", "1");
        }
        assert!(c.status().unwrap().success());
        let log = fs::read_to_string(home.join("log")).unwrap();
        // The variable never reaches apps; a crash restart always relaunches the lock screen.
        let want = if locked {
            "-- start-hyprland -- --locked-cmd wol-lock --startup | locked= crash=wol-lock --startup\n"
        } else {
            "-- start-hyprland | locked= crash=wol-lock --startup\n"
        };
        assert_eq!(log, want, "locked={locked}");
    }
    sh_n("desktop/overlay/usr/bin/wol-session");
}

#[test]
fn every_compositor_starts_on_black_with_the_lock_kept_across_crashes() {
    let desktop = read("desktop/overlay/usr/share/wolos/hypr/hyprland.conf");
    let black = Some("rgb(000000)".to_string());
    assert_eq!(value(&desktop, "misc", "background_color"), black);
    // A crashed wol-lock is replaced by its supervisor; the new one takes the lock over.
    assert_eq!(
        value(&desktop, "misc", "allow_session_lock_restore"),
        Some("true".into())
    );
    for f in ["login.conf", "setup.conf"] {
        let c = read(&format!("desktop/overlay/usr/share/wolos/hypr/{f}"));
        assert_eq!(value(&c, "misc", "background_color"), black, "{f}");
        // No pointer over the black frames: the window shows it after its first frame (kit::Reveal).
        assert_eq!(
            value(&c, "cursor", "invisible"),
            Some("true".into()),
            "{f}: cursor hidden until the window maps"
        );
    }
}

/// Boot splash: the WolOS mark (plymouth theme image) is a small 8-bit RGBA PNG.
#[test]
fn splash_is_the_wolos_mark() {
    let w = read("image/repo/wolos-system/PKGBUILD");
    assert!(w.contains("design/brand/wolos-splash.png") || w.contains("$d/brand/wolos-splash.png"));
    let png = fs::read(repo("design/brand/wolos-splash.png")).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let be = |i: usize| u32::from_be_bytes(png[i..i + 4].try_into().unwrap());
    let (w, h) = (be(16), be(20));
    assert!(w <= 480 && h <= 480, "{w}x{h}");
    assert_eq!((png[24], png[25]), (8, 6), "8-bit RGBA");
}

/// Hyprland draws no title bars: GTK 3 apps without a header bar (Shotwell) lost their traffic lights.
#[test]
fn every_gtk3_window_draws_the_kit_traffic_lights() {
    let s = read("desktop/overlay/usr/bin/wol-session");
    assert!(s.contains("export GTK_CSD=${GTK_CSD:-1}"), "{s}");
}
