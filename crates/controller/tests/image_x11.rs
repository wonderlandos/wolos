// SPDX-License-Identifier: GPL-2.0-or-later
//! XWayland and X11 integration (audit X1, v0.12): every conf, environment and package line the
//! X11 fixes rely on. X11 apps follow the monitor scale (wol-x11-env), Java and Qt find their way,
//! only this account's X11 clients connect, X11 pop-ups do not steal focus, fullscreen X11 games
//! present immediately, and VirtualBox gets resize, clipboard and drag-and-drop.
#[path = "support/siblings.rs"]
mod siblings;

use std::path::PathBuf;

fn repo(p: &str) -> PathBuf {
    siblings::root().join(p)
}

fn read(p: &str) -> String {
    std::fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

const HYPR: &str = "desktop/overlay/usr/share/wolos/hypr";
const BIN: &str = "desktop/overlay/usr/bin";

fn conf(name: &str) -> String {
    read(&format!("{HYPR}/{name}.conf"))
}

/// Lines without their trailing `  # comment` and surrounding blanks; comment lines dropped.
fn code(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.split("  #").next().unwrap_or(l).trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

fn has_line(text: &str, want: &str) -> bool {
    code(text).iter().any(|l| l == want)
}

/// Package names of the image lists (first word of each line, comments dropped).
fn packages() -> Vec<String> {
    ["profile/packages.x86_64", "profile/packages.desktop"]
        .iter()
        .flat_map(|p| {
            read(p)
                .lines()
                .filter_map(|l| l.split('#').next()?.split_whitespace().next())
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A `windowrule { ... }` block by its `name =`.
fn rule_block(text: &str, name: &str) -> String {
    let head = format!("name = {name}");
    let at = text
        .find(&head)
        .unwrap_or_else(|| panic!("no windowrule {name}"));
    text[at..].split('}').next().unwrap().to_string()
}

#[test]
fn session_env_lets_java_and_qt_run_under_xwayland() {
    let h = conf("hyprland");
    // X1-4: AWT knows the compositor does not reparent (grey JetBrains/Swing windows otherwise).
    assert!(
        has_line(&h, "env = _JAVA_AWT_WM_NONREPARENTING,1"),
        "hyprland.conf"
    );
    let env = read("desktop/overlay/usr/lib/environment.d/60-wolos-flatpak.conf");
    assert!(
        env.lines().any(|l| l == "_JAVA_AWT_WM_NONREPARENTING=1"),
        "environment.d (Flatpak Java): {env}"
    );
    // X1-8: Qt without its Wayland plugin falls back to xcb instead of aborting.
    assert!(
        has_line(&h, "env = QT_QPA_PLATFORM,wayland;xcb"),
        "hyprland.conf"
    );
    assert!(!has_line(&h, "env = QT_QPA_PLATFORM,wayland"));
}

#[test]
fn x11_scale_step_runs_on_every_load_and_after_reload() {
    let h = conf("hyprland");
    // X1-2: `exec` (not exec-once) so `hyprctl reload` re-runs it.
    assert!(has_line(&h, "exec = wol-x11-env"), "hyprland.conf");
    assert!(
        !code(&h)
            .iter()
            .any(|l| l.starts_with("exec-once = wol-x11-env"))
    );
    // X1-11: the systemd user manager gets the session variables whichever D-Bus daemon runs.
    assert!(has_line(
        &h,
        "exec-once = dbus-update-activation-environment --systemd --all"
    ));
    let s = read(&format!("{BIN}/wol-x11-env"));
    assert!(s.starts_with("#!/bin/sh\n# SPDX-License-Identifier: GPL-2.0-or-later\n"));
    assert!(s.contains("\nset -eu\n"));
    for want in [
        "hyprctl -j monitors",
        "xrdb -nocpp -merge -",
        "Xft.dpi: %s",
        "Xcursor.size: %s",
        "Xcursor.theme: %s",
        "dbus-update-activation-environment --systemd",
        "\"GDK_SCALE=$gdk\"",
        "\"STEAM_FORCE_DESKTOPUI_SCALING=$steam\"",
        "--cursor-size",
    ] {
        assert!(s.contains(want), "wol-x11-env lacks {want}");
    }
    // Settings' cursor size goes through the same step (no second xrdb snippet).
    let sync = wol_core::hyprprefs::cursor_sync(32);
    assert!(sync.contains(&vec![
        wol_core::hyprprefs::X11_ENV.to_string(),
        "--cursor-size".into(),
        "32".into()
    ]));
    assert!(!sync.iter().flatten().any(|a| a.contains("xrdb")));
}

/// Program -> packages that provide it (any one listed is enough).
fn provider(bin: &str) -> &'static [&'static str] {
    match bin {
        // glib2 comes with gsettings-desktop-schemas (its dependency).
        "gsettings" => &["glib2", "gsettings-desktop-schemas"],
        "xrdb" => &["xorg-xrdb"],
        "xhost" => &["xorg-xhost"],
        "hyprctl" => &["hyprland-wolos"],
        other => panic!("no package known for {other}: add it here and to packages.desktop"),
    }
}

/// X1-3: every program the cursor sync and the X11 step run comes from a shipped package.
#[test]
fn cursor_sync_and_x11_env_programs_are_shipped() {
    let pkgs = packages();
    let shipped = |p: &str| pkgs.iter().any(|x| x == p);
    let scripts = ["wol-x11-env"];
    let mut bins: Vec<String> = wol_core::hyprprefs::cursor_sync(24)
        .into_iter()
        .map(|argv| argv[0].clone())
        .collect();
    bins.extend(["xrdb", "hyprctl", "xhost"].map(String::from));
    for b in bins {
        if scripts.contains(&b.as_str()) {
            assert!(
                repo(&format!("{BIN}/{b}")).is_file(),
                "{b} not in the overlay"
            );
            continue;
        }
        let p = provider(&b);
        assert!(
            p.iter().any(|x| shipped(x)),
            "{b}: none of {p:?} in the image"
        );
    }
}

/// X1-6: Xwayland has no auth cookie; only this account's X11 clients may connect.
#[test]
fn xwayland_admits_only_this_account() {
    let h = conf("hyprland");
    let l = code(&h)
        .into_iter()
        .find(|l| l.contains("xhost"))
        .expect("xhost line");
    assert!(l.starts_with("exec-once = "), "{l}");
    // Grant first, then close: the closing call must still be able to connect.
    let grant = l.find("xhost +si:localuser:").expect("grant");
    let close = l.rfind("xhost -'").expect("access control on");
    assert!(grant < close, "{l}");
    assert!(l.contains("xhost -local:"), "{l}");
    assert!(packages().iter().any(|p| p == "xorg-xhost"));
}

/// X1-7 (decision): no X11 `_NET_ACTIVE_WINDOW` focus stealing; the window turns urgent instead.
#[test]
fn activation_requests_do_not_take_focus() {
    let h = conf("hyprland");
    let misc = h.split("\nmisc {").nth(1).expect("misc block");
    let misc = misc.split('}').next().unwrap();
    assert!(has_line(misc, "focus_on_activate = false"), "{misc}");
}

/// X1-5: one helper starts the VirtualBox guest services in every compositor session.
#[test]
fn virtualbox_guest_gets_resize_clipboard_and_dnd() {
    for c in ["hyprland", "installer", "setup", "login"] {
        let s = conf(c);
        assert!(has_line(&s, "exec-once = wol-vbox-guest"), "{c}.conf");
        assert!(
            !s.contains("VBoxClient"),
            "{c}.conf calls VBoxClient directly"
        );
    }
    let s = read(&format!("{BIN}/wol-vbox-guest"));
    assert!(s.starts_with("#!/bin/sh\n# SPDX-License-Identifier: GPL-2.0-or-later\n"));
    assert!(s.contains("\nset -eu\n"));
    for want in [
        "= oracle ] || exit 0",
        "resize=--vmsvga\n",
        "resize=--vmsvga-session",
        "\"$resize\" --clipboard --draganddrop",
    ] {
        assert!(s.contains(want), "wol-vbox-guest lacks {want}");
    }
    assert!(packages().iter().any(|p| p == "virtualbox-guest-utils"));
}

/// X1-9, X1-17: the xwayland block states its options; fullscreen X11 games present at once.
#[test]
fn xwayland_block_and_immediate_rules() {
    let perf = conf("wolos-perf");
    let xw = perf.split("xwayland {").nth(1).expect("xwayland block");
    let xw = xw.split('}').next().unwrap();
    for want in [
        "enabled = true",
        "force_zero_scaling = true",
        "use_nearest_neighbor = true",
        "create_abstract_socket = false",
    ] {
        assert!(has_line(xw, want), "xwayland block lacks {want}");
    }
    let wine = rule_block(&perf, "wol-wine-immediate");
    assert!(has_line(&wine, r"match:class = (?i)\.exe$"), "{wine}");
    assert!(has_line(&wine, "immediate = on"), "{wine}");
    let fs = rule_block(&perf, "wol-xwayland-fullscreen-immediate");
    for want in [
        "match:xwayland = true",
        "match:fullscreen = true",
        "immediate = on",
    ] {
        assert!(has_line(&fs, want), "{fs}");
    }
    // Steam's gamescope launch option and fixed-resolution games need the micro-compositor.
    assert!(packages().iter().any(|p| p == "gamescope"));
}

/// X1-16: pacman Electron apps and VS Code get the ozone hint from a linked default. Since ADR
/// 0042 the link goes to the web-app runtime profile (tests/webruntime.rs runs the linker).
#[test]
fn electron_flags_defaults_are_linked() {
    let s = read(&format!("{BIN}/wol-session"));
    assert!(
        s.lines()
            .any(|l| l.contains(r#"sh "$share/webruntime/link.sh" "$cfg" "$share""#)),
        "wol-session links the Electron flag files through the runtime profile"
    );
    let link = read("desktop/overlay/usr/share/wolos/webruntime/link.sh");
    let lp = link
        .lines()
        .find(|l| l.starts_with("for f in electron-flags.conf"))
        .expect("the linker names the flag files");
    for f in ["electron-flags.conf", "code-flags.conf"] {
        let t = read(&format!("desktop/overlay/usr/share/wolos/{f}"));
        assert!(has_line(&t, "--ozone-platform-hint=auto"), "{f}");
        assert!(
            lp.split_whitespace().any(|w| w.trim_end_matches(';') == f),
            "the linker does not link {f}"
        );
    }
}
