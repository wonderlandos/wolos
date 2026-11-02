// SPDX-License-Identifier: GPL-2.0-or-later
//! flatpak-wolos: Flatpak from the WolOS fork's `wolos` branch at the commit in `ref`, built from a
//! tarball like linux-wolos, replacing Arch's flatpak and listed in the [wolos] repo.
use std::{fs, path::PathBuf};
fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
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
    let build = read("tools/build-rust.sh");
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
    for d in ["crates", "desktop", "files"] {
        scan(&repo(d), &mut hits);
    }
    assert!(hits.is_empty(), "{hits:?}");
}
