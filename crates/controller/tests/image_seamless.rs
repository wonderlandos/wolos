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
        c.arg(repo("desktop/overlay/usr/local/bin/wol-session"))
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
    sh_n("desktop/overlay/usr/local/bin/wol-session");
}

#[test]
fn every_compositor_starts_on_black_with_the_lock_kept_across_crashes() {
    let desktop = read("desktop/overlay/usr/local/share/wolos/hypr/hyprland.conf");
    let black = Some("rgb(000000)".to_string());
    assert_eq!(value(&desktop, "misc", "background_color"), black);
    // A crashed wol-lock is replaced by its supervisor; the new one takes the lock over.
    assert_eq!(
        value(&desktop, "misc", "allow_session_lock_restore"),
        Some("true".into())
    );
    for f in ["login.conf", "setup.conf"] {
        let c = read(&format!("desktop/overlay/usr/local/share/wolos/hypr/{f}"));
        assert_eq!(value(&c, "misc", "background_color"), black, "{f}");
        // No pointer over the black frames: the window shows it after its first frame (kit::Reveal).
        assert_eq!(
            value(&c, "cursor", "invisible"),
            Some("true".into()),
            "{f}: cursor hidden until the window maps"
        );
    }
}

/// Runs rc.d/wolos_quiet start with a stub rc.subr and a fake `sysctl kern.console`.
fn quiet_start(kern_console: &str) -> String {
    let d = sandbox(
        &format!("quiet-{}", kern_console.len()),
        &[(
            "sysctl",
            &format!("#!/bin/sh\n[ \"$*\" = '-n kern.console' ] && echo '{kern_console}'\n"),
        )],
    );
    let stub = d.join("rc.subr");
    fs::write(
        &stub,
        "load_rc_config() { :; }\nrun_rc_command() { [ \"$1\" = start ] && $start_cmd; }\n",
    )
    .unwrap();
    let script =
        read("image/files/etc/rc.d/wolos_quiet").replace("/etc/rc.subr", &stub.to_string_lossy());
    let out = Command::new("sh")
        .args(["-c", &script, "wolos_quiet", "start"])
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// After the loader splash nothing but WolOS reaches the screen: rc text on a video console is
/// black on black without a cursor; a serial console (VM, headless) keeps readable logs.
#[test]
fn boot_text_never_reaches_the_screen() {
    use controller::login::BLANK_CONSOLE;
    assert_eq!(quiet_start("ttyv0,/ttyv0,"), BLANK_CONSOLE);
    assert_eq!(quiet_start("ttyv0,ttyu0,/ttyv0,ttyu0,"), BLANK_CONSOLE);
    assert_eq!(quiet_start("ttyu0,ttyv0,/ttyv0,ttyu0,"), "");
    let rc = read("image/files/etc/rc.d/wolos_quiet");
    // Before the first rc.d script that prints (hostid: "Setting hostuuid").
    let before = rc
        .lines()
        .find_map(|l| l.strip_prefix("# BEFORE:"))
        .expect("BEFORE line");
    for s in ["hostid", "dumpon", "sysctl", "fsck", "root"] {
        assert!(
            before.split_whitespace().any(|w| w == s),
            "wolos_quiet runs before {s}"
        );
    }
    sh_n("image/files/etc/rc.d/wolos_quiet");
    let b = read("image/build.sh");
    assert!(
        b.contains("\"$STAGE/etc/rc.d/wolos_quiet\""),
        "build.sh makes wolos_quiet executable"
    );
}

/// Kernel splash: the WolOS mark (not the FreeBSD logo) on black, for boot and shutdown.
#[test]
fn loader_shows_the_wolos_splash() {
    let b = read("image/build.sh");
    for want in [
        "boot_mute=\"YES\"",
        "splash=\"/boot/images/wolos.png\"",
        "shutdown_splash=\"/boot/images/wolos.png\"",
        "design/brand/wolos-splash.png\" \"$STAGE/boot/images/wolos.png\"",
    ] {
        assert!(b.contains(want), "image/build.sh lacks {want}");
    }
    // stand/common/gfx_fb.c: png bpp -> si_depth; vt(4) draws only depth 4 (8-bit RGBA), centred
    // at its own size, so it must fit the smallest screen.
    let png = fs::read(repo("design/brand/wolos-splash.png")).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let be = |i: usize| u32::from_be_bytes(png[i..i + 4].try_into().unwrap());
    let (w, h) = (be(16), be(20));
    let (depth, colour, interlace) = (png[24], png[25], png[28]);
    assert!(w <= 480 && h <= 480, "{w}x{h}");
    assert_eq!(
        (depth, colour, interlace),
        (8, 6, 0),
        "8-bit RGBA, not interlaced"
    );
}
