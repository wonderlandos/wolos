// SPDX-License-Identifier: GPL-2.0-or-later
//! The Arch Linux image (ADR 0016): mkarchiso profile `wolos-iso/profile`, pinned to one Arch Linux Archive
//! date, `[wolos]` before the Arch repos, linux-wolos only, the `[wolos]` repo build and the release
//! manifest. Scripts run against fakes; nothing here needs root or the network.
#[path = "support/repo.rs"]
mod repo;
use repo::{read, repo};

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

/// Dry run 0.13.0 live-logout (QA-LIVE-32 class): a compositor that starts hypridle stops it
/// before it exits. hypridle aborts (SIGABRT, a core dump in the journal) when its display goes;
/// the desktop's Log Out stops it first (`shell::clipboard::before_end`), the login window did
/// not, so every sign-in left an abort from the login window's own hypridle. Only its own
/// (`-u` the compositor's account): never another account's.
#[test]
fn a_compositor_stops_its_hypridle_before_it_exits() {
    // The desktop's Log Out stops the unit (a pkill came back: Restart=on-failure, 2 s).
    let unit = read("files/usr/lib/systemd/user/wolos-hypridle.service");
    assert!(unit.contains("Restart=on-failure"), "{unit}");
    let shell = read("crates/shell/src/clipboard.rs");
    assert!(
        shell.contains(r#"["systemctl", "--user", "stop", "wolos-hypridle.service"]"#),
        "Log Out stops wolos-hypridle.service through systemd"
    );
    let stop = r#"pkill -x -u "$(id -u)" hypridle"#;
    for c in ["installer", "setup", "login"] {
        for (ext, exit) in [("conf", "hyprctl dispatch exit"), ("lua", "hl.dsp.exit()")] {
            let f = format!("desktop/overlay/usr/share/wolos/hypr/{c}.{ext}");
            let text = read(&f);
            if !text.contains("hypridle -c") {
                continue;
            }
            let line = text
                .lines()
                .find(|l| l.contains("/usr/bin/wol-") && l.contains(exit))
                .unwrap_or_else(|| panic!("{f}: no exit line"));
            let (at_stop, at_exit) = (line.find(stop), line.find(exit));
            assert!(
                matches!((at_stop, at_exit), (Some(s), Some(e)) if s < e),
                "{f}: hypridle stopped before the exit: {line}"
            );
        }
    }
}

/// QA-FDAY-1 (dry run 0.13.0, p1-live-try): the Try word went to the screen's VT only, so a
/// computer whose console is a serial port (a VM, a headless box: where wol-login's boot marks
/// already go, `wol-login` `emit`) never showed it. It goes to /dev/console too when that is a
/// serial port (the last name in the kernel's active consoles), after the compositor ended.
#[test]
fn the_try_word_reaches_a_serial_console_too() {
    let s = read("files/usr/lib/wolos/installer-session");
    let code: Vec<&str> = s
        .lines()
        .map(str::trim_start)
        .filter(|l| !l.starts_with('#'))
        .collect();
    let at = |needle: &str| {
        code.iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("installer-session has no line with {needle}"))
    };
    let console = at(r#"> /dev/console"#);
    assert!(code[console].contains("$msg"), "{}", code[console]);
    assert!(at("/sys/class/tty/console/active") < console);
    assert!(at("Hyprland -c") < console && console < at("systemctl start --no-block wold.service"));
    let ok = std::process::Command::new("sh")
        .arg("-n")
        .arg(repo("files/usr/lib/wolos/installer-session"))
        .status()
        .unwrap();
    assert!(ok.success());
}

/// QA-SHEET-2, QA-VIS2-4 (the installer hand-off): the installer's compositor leaves the console
/// it found. The boot log was on the framebuffer, and the old Welcome card came back with black
/// bars, between the card fading out and the live desktop. Writing the blank bytes while the VT is in
/// graphics mode does nothing, so `installer-session` writes them in text mode, to the VT the login
/// window uses, before the compositor takes it (wol-login's BLANK_CONSOLE on CONSOLE); the Try
/// word comes after the compositor is gone, drawn on the same blank console.
#[test]
fn the_installer_blanks_the_console_before_its_compositor_starts() {
    use controller::login::{BLANK_CONSOLE, CONSOLE};
    let s = read("files/usr/lib/wolos/installer-session");
    let code: Vec<&str> = s
        .lines()
        .map(str::trim_start)
        .filter(|l| !l.starts_with('#'))
        .collect();
    let at = |needle: &str| {
        code.iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("installer-session has no line with {needle}"))
    };
    // The same bytes as wol-login writes, as printf escapes.
    let printf = BLANK_CONSOLE.replace('\x1b', "\\033");
    let blank = at(&format!("printf '{printf}' > \"$tty\""));
    assert!(
        code.contains(&format!("tty={CONSOLE}").as_str()),
        "the login window's VT"
    );
    assert!(
        blank < at("Hyprland -c"),
        "blank first: after the compositor has the VT the write is lost"
    );
    // The Try word is drawn after the compositor ended, then the desktop is asked for.
    assert!(at("Hyprland -c") < at("Starting the desktop"));
    assert!(at("Starting the desktop") < at("systemctl start --no-block wold.service"));
    // The compositor is the installer's own config, which turns the output off before it exits
    // (`every_greeter_compositor_turns_the_output_off_before_it_exits`).
    assert!(code.iter().any(|l| l.contains("installer.lua")));
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
