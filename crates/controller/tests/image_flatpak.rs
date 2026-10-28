//! flatpak-wolos: Flatpak from the WolOS fork's `wolos` branch at the commit in `ref`, built from a
//! tarball like linux-wolos, replacing Arch's flatpak and listed in the [wolos] repo.
use std::{fs, path::PathBuf, process::Command};

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}
fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

#[test]
fn recipe_builds_the_fork_at_the_recorded_commit() {
    let r = read("image/repo/flatpak-wolos/ref");
    let commit = r.trim();
    assert!(
        commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit()),
        "ref is a full commit id: {commit:?}"
    );
    let p = read("image/repo/flatpak-wolos/PKGBUILD");
    for want in [
        "pkgname=flatpak-wolos",
        "url='https://github.com/wonderlandos/flatpak'",
        "$url/archive/$_commit.tar.gz",
        "$(<\"$startdir/ref\")",
        "conflicts=(flatpak)",
        "-D system_bubblewrap=bwrap",
        "-D system_dbus_proxy=xdg-dbus-proxy",
    ] {
        assert!(p.contains(want), "PKGBUILD lacks {want}");
    }
    assert!(!p.contains("git+"), "a tarball, not a clone");
    let dir = repo("image/repo/flatpak-wolos");
    let out = Command::new("bash")
        .arg("-c")
        .arg("startdir=$PWD; source ./PKGBUILD >/dev/null; echo \"${provides[*]}|$_commit\"")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = String::from_utf8_lossy(&out.stdout);
    let (provides, pinned) = out.trim().split_once('|').unwrap();
    assert_eq!(pinned, commit);
    // Same epoch and upstream version as Arch's package, so dependants keep resolving.
    assert!(
        provides.split(' ').any(|p| p == "flatpak=1:1.16.6"),
        "{provides}"
    );
}

#[test]
fn the_repo_builds_it() {
    let t = read("image/repo/packages");
    let line = t
        .lines()
        .find(|l| l.starts_with("flatpak-wolos "))
        .expect("flatpak-wolos in image/repo/packages");
    let f: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(f[1], "image/repo/flatpak-wolos");
    assert!(f[3..].contains(&"image/repo/flatpak-wolos/"), "{line}");
}

/// Every session finds the consent helper: systemd's environment.d, read for user sessions.
#[test]
fn sessions_point_flatpak_at_the_consent_helper() {
    let t = read("desktop/overlay/usr/lib/environment.d/60-wolos-flatpak.conf");
    let line = format!(
        "{}={}",
        wol_core::flatpak::HELPER_ENV,
        wol_core::flatpak::HELPER
    );
    assert!(t.lines().any(|l| l == line), "{t}");
    let build = read("image/build-rust.sh");
    let list = build
        .lines()
        .find(|l| l.starts_with("for b in wold"))
        .unwrap();
    assert!(
        list.split_whitespace()
            .any(|b| b.trim_end_matches(';') == "wol-flatpak-consent"),
        "the helper is built and installed: {list}"
    );
}

/// Flathub is there from the first boot, system-wide, with its signing key.
#[test]
fn flathub_is_a_system_remote() {
    let t = read("desktop/overlay/etc/flatpak/remotes.d/flathub.flatpakrepo");
    assert!(t.starts_with("[Flatpak Repo]\n"), "{t}");
    for want in ["Url=https://dl.flathub.org/repo/", "GPGKey="] {
        assert!(t.lines().any(|l| l.starts_with(want)), "lacks {want}");
    }
    let key = t.lines().find_map(|l| l.strip_prefix("GPGKey=")).unwrap();
    assert!(key.len() > 1000, "a whole key, not a stub");
}

/// X11 apps run through XWayland at 1:1 (the per-app scale makes them sharp, not upscaled);
/// Electron/Chromium apps pick Wayland when they can.
#[test]
fn x11_compat_and_electron_wayland_hint() {
    let hypr = read("desktop/overlay/usr/share/wolos/hypr/wolos-perf.conf");
    let xw = hypr.split("xwayland {").nth(1).expect("xwayland block");
    let xw = xw.split('}').next().unwrap();
    assert!(xw.contains("enabled = true"), "{xw}");
    assert!(xw.contains("force_zero_scaling = true"), "{xw}");
    let env = read("desktop/overlay/usr/lib/environment.d/60-wolos-flatpak.conf");
    assert!(
        env.lines()
            .any(|l| l == "ELECTRON_OZONE_PLATFORM_HINT=auto")
    );
}

/// Apps that offer Wayland and fallback X11 keep Flatpak's default (native Wayland): nothing in
/// WolOS overrides their sockets.
fn scan(dir: &std::path::Path, hits: &mut Vec<String>) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            if !p.ends_with("target") && !p.ends_with(".git") && !p.ends_with("golden") {
                scan(&p, hits);
            }
        } else if let Ok(t) = fs::read_to_string(&p) {
            for bad in [
                "--socket=fallback-x11",
                "--nosocket=wayland",
                "sockets=!wayland",
            ] {
                if t.contains(bad) && !p.ends_with("image_flatpak.rs") {
                    hits.push(format!("{}: {bad}", p.display()));
                }
            }
        }
    }
}

#[test]
fn wayland_capable_flatpaks_are_never_overridden() {
    let mut hits = Vec::new();
    for d in ["crates", "desktop", "image/files"] {
        scan(&repo(d), &mut hits);
    }
    assert!(hits.is_empty(), "{hits:?}");
}
