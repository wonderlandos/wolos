// SPDX-License-Identifier: GPL-2.0-or-later
//! The Arch Linux image (ADR 0016): mkarchiso profile `wolos-iso/profile`, pinned to one Arch Linux Archive
//! date, `[wolos]` before the Arch repos, linux-wolos only, the `[wolos]` repo build and the release
//! manifest. Scripts run against fakes; nothing here needs root or the network.
#[path = "support/repo.rs"]
mod repo;
use repo::read;

#[test]
fn every_hyprland_session_runs_at_scale_one() {
    // The installer, setup and login sessions use the desktop's monitor rule: an automatic scale
    // made the logical screen smaller than the welcome card in a VM.
    let want = "monitor = , preferred, auto, 1";
    for c in ["hyprland", "installer", "setup", "login"] {
        let s = read(&format!("desktop/overlay/usr/share/wolos/hypr/{c}.conf"));
        assert!(s.lines().any(|l| l == want), "{c}.conf lacks {want}");
    }
}

/// QA-SHEET-3, QA-VIS2-20: a greeter compositor (installer, Setup, login window) turns its output
/// off before it exits, in both config languages: the last scanout (the old card with black bars)
/// came back on screen during the hand-off to the next one. The installer had it, Setup did not.
#[test]
fn every_greeter_compositor_turns_the_output_off_before_it_exits() {
    for c in ["installer", "setup", "login"] {
        let conf = read(&format!("desktop/overlay/usr/share/wolos/hypr/{c}.conf"));
        assert!(
            conf.lines()
                .any(|l| l.starts_with("exec-once = /usr/bin/wol-")
                    && l.contains("hyprctl dispatch dpms off; hyprctl dispatch exit")),
            "{c}.conf: dpms off before exit"
        );
        let lua = read(&format!("desktop/overlay/usr/share/wolos/hypr/{c}.lua"));
        assert!(
            lua.lines().any(|l| l.contains("/usr/bin/wol-")
                && l.contains(
                    r#"hl.dsp.dpms({ action = \"off\" })"; hyprctl dispatch "hl.dsp.exit()""#
                )),
            "{c}.lua: dpms off before exit"
        );
    }
}

/// QA-FILES-1 / QA-ADR-15: every program the login window's power row runs (and the session menu's)
/// exists on the Arch image. `acpiconf` and `shutdown -p` were FreeBSD's and are gone; the verbs are
/// systemd's, which every image has (`systemd-ukify` and the units below `files/usr/lib/systemd`
/// need it).
#[test]
fn power_binaries_exist() {
    use controller::login::power_argv;
    use shared::login::Power;
    let img = read("files/usr/lib/systemd/system/wolos-boot-ok.service");
    assert!(img.contains("[Unit]"), "the image ships systemd units");
    for p in Power::ALL {
        let argv = power_argv(p);
        assert_eq!(argv.first(), Some(&"systemctl"), "{p:?}: {argv:?}");
        assert!(
            !argv
                .iter()
                .any(|a| a.contains("acpiconf") || a.starts_with("/sbin")),
            "{p:?}: FreeBSD command {argv:?}"
        );
    }
    // No source file of the login path names the FreeBSD commands any more.
    for f in [
        "crates/controller/src/login.rs",
        "crates/controller/src/bin/wol-login.rs",
    ] {
        let src = read(f);
        let code: String = src
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let code = code.split("#[cfg(test)]").next().unwrap_or(&code);
        for bad in ["acpiconf", "shutdown\", \"-p"] {
            assert!(!code.contains(bad), "{f} still runs {bad}");
        }
    }
}
