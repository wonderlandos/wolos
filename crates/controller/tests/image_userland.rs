// SPDX-License-Identifier: GPL-2.0-or-later
//! Everyday userland from the real-GPU run: sudo for administrators, zsh for every account, user
//! folders, unlocked keyring, printing, removable media and default apps (Helium the browser).
//! Scripts run on the host with fakes.
#![allow(dead_code)] // siblings.rs helpers this file does not call
#[path = "support/siblings.rs"]
mod siblings;

use std::{fs, os::unix::fs::PermissionsExt, path::Path, path::PathBuf, process::Command};
fn repo(p: &str) -> PathBuf {
    siblings::root().join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

/// Fresh temp dir with executable fakes `(name, body)` in `bin/`.
fn sandbox(tag: &str, fakes: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-userland-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("bin")).unwrap();
    for (name, body) in fakes {
        let p = d.join("bin").join(name);
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    d
}

fn path_with(d: &Path) -> String {
    format!("{}:/usr/bin:/bin", d.join("bin").display())
}

fn sh_n(p: &str) {
    let st = Command::new("sh").arg("-n").arg(repo(p)).status().unwrap();
    assert!(st.success(), "sh -n {p}");
}

fn zsh() -> Option<&'static str> {
    let ok = Command::new("zsh").arg("-fc").arg(":").status();
    ok.is_ok_and(|s| s.success()).then_some("zsh")
}

#[test]
fn every_account_logs_in_to_zsh_and_root_keeps_sh() {
    assert!(wol_core::settings::users::LOGIN_SHELL.ends_with("/bin/zsh"));
    let b = read("build.sh");
    let live = b.lines().find(|l| l.starts_with("u live ")).unwrap();
    assert!(live.ends_with(" /usr/bin/zsh"), "{live}");
    // root keeps bash (base system): a broken desktop package must not lock out recovery.
    assert!(
        !b.contains("usermod root") && !b.contains("chsh"),
        "build.sh changes root's shell"
    );
    // Users' own files: only skel (new accounts) and system-level rc files, never ~/.zshrc.
    let skel = read("desktop/overlay/usr/share/skel/dot.zshrc");
    assert!(
        skel.lines().all(|l| l.starts_with('#')),
        "skel .zshrc is comments only"
    );
    // Arch's zsh reads /etc/zsh/; zshenv is the one file there no package owns. zsh's newuser
    // script calls zsh-newuser-install when it is defined: a no-op means no first-run wizard.
    let env = read("desktop/overlay/etc/zsh/zshenv");
    assert!(
        env.contains("zsh-newuser-install() { :; }"),
        "no zsh first-run wizard"
    );
    for gone in ["etc/zshrc", "etc/zprofile", "usr/share/zsh/scripts/newuser"] {
        assert!(
            !repo(&format!("desktop/overlay/{gone}")).exists(),
            "{gone}: not read, or zsh's"
        );
    }
    let Some(zsh) = zsh() else { return };
    let d = sandbox("zshenv", &[]);
    fs::write(d.join("rc"), "WOL_RC=loaded\n").unwrap();
    let run = |flags: &str| {
        let probe = format!(
            "source {}; print -r -- \"rc=${{WOL_RC:-none}} $(whence -w zsh-newuser-install)\"",
            repo("desktop/overlay/etc/zsh/zshenv").display()
        );
        let o = Command::new(zsh)
            .args([flags, "-c", &probe])
            .env("WOLOS_ZSHRC", d.join("rc"))
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    assert_eq!(run("-fi"), "rc=loaded zsh-newuser-install: function");
    assert_eq!(
        run("-f"),
        "rc=none zsh-newuser-install: function",
        "scripts skip the setup"
    );
}

#[test]
fn system_zshrc_loads_plugins_theme_colours_and_keys() {
    let rc = "desktop/overlay/usr/share/wolos/zsh/zshrc";
    let text = read(rc);
    // Arch package paths (zsh-autosuggestions, zsh-syntax-highlighting).
    for p in [
        "${WOLOS_ZSH_PLUGINS:-/usr/share/zsh/plugins}",
        "zsh-autosuggestions/zsh-autosuggestions.zsh",
        "zsh-syntax-highlighting/zsh-syntax-highlighting.zsh",
        "wolos/toolkits/zsh/colors.zsh",
    ] {
        assert!(text.contains(p), "zshrc lacks {p}");
    }
    let Some(zsh) = zsh() else {
        eprintln!("zsh not on this host: zshrc run skipped");
        return;
    };
    for f in [rc, "desktop/overlay/etc/zsh/zshenv"] {
        let st = Command::new(zsh).arg("-n").arg(repo(f)).status().unwrap();
        assert!(st.success(), "zsh -n {f}");
    }
    let d = sandbox("zshrc", &[]);
    let (home, plugins) = (d.join("home"), d.join("share"));
    for (rel, body) in [
        (
            "zsh-autosuggestions/zsh-autosuggestions.zsh",
            "WOL_AS=$ZSH_AUTOSUGGEST_HIGHLIGHT_STYLE",
        ),
        (
            "zsh-syntax-highlighting/zsh-syntax-highlighting.zsh",
            "WOL_SH=${WOL_AS:+after-autosuggestions}",
        ),
    ] {
        fs::create_dir_all(plugins.join(rel).parent().unwrap()).unwrap();
        fs::write(plugins.join(rel), body).unwrap();
    }
    let colors = home.join(".config/wolos/toolkits/zsh/colors.zsh");
    fs::create_dir_all(colors.parent().unwrap()).unwrap();
    fs::write(
        &colors,
        "WOLOS_PROMPT_ACCENT='#7c5cff'\nWOLOS_PROMPT_SUGGEST=244\n",
    )
    .unwrap();
    let probe = format!(
        "source {}; print -r -- \"P=$PROMPT\"; print -r -- \"AS=$WOL_AS SH=$WOL_SH\"; \
         for k in '^[[H' '^[[F' '^[[3~' '^[[1;5C' '^[[1;5D' '^[[A'; do bindkey \"$k\"; done",
        repo(rc).display()
    );
    let out = Command::new(zsh)
        .args(["-f", "-i", "-c", &probe])
        .env_clear()
        .env("HOME", &home)
        .env("TERM", "foot")
        .env("PATH", "/usr/bin:/bin")
        .env("WOLOS_ZSH_PLUGINS", &plugins)
        .output()
        .unwrap();
    let (so, se) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(out.status.success(), "{se}");
    assert!(
        !se.contains("command not found") && !se.contains("no such"),
        "{se}"
    );
    assert!(
        so.contains("%F{#7c5cff}"),
        "theme accent not in prompt:\n{so}"
    );
    assert!(
        so.contains("${vcs_info_msg_0_}"),
        "no git branch in prompt:\n{so}"
    );
    assert!(
        so.contains("AS=fg=244 SH=after-autosuggestions"),
        "plugins:\n{so}"
    );
    for (k, w) in [
        ("^[[H", "beginning-of-line"),
        ("^[[F", "end-of-line"),
        ("^[[3~", "delete-char"),
        ("^[[1;5C", "forward-word"),
        ("^[[1;5D", "backward-word"),
        ("^[[A", "up-line-or-beginning-search"),
    ] {
        assert!(so.contains(&format!("\"{k}\" {w}")), "{k} -> {w}:\n{so}");
    }
    // Only the completion cache is written; the user's rc files are never created.
    assert!(!home.join(".zshrc").exists());
    assert!(home.join(".cache/zsh").is_dir());
}

#[test]
fn session_creates_user_folders_and_finishes_the_keyring() {
    let d = sandbox(
        "session",
        &[
            (
                "xdg-user-dirs-update",
                "#!/bin/sh\necho dirs >> \"$HOME/log\"\n",
            ),
            (
                "dbus-run-session",
                "#!/bin/sh\necho session >> \"$HOME/log\"\n",
            ),
        ],
    );
    let home = d.join("home");
    fs::create_dir_all(&home).unwrap();
    let st = Command::new("sh")
        .arg(repo("desktop/overlay/usr/bin/wol-session"))
        .env("PATH", path_with(&d))
        .env("HOME", &home)
        .env("XDG_RUNTIME_DIR", &d)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .status()
        .unwrap();
    assert!(st.success());
    assert_eq!(
        fs::read_to_string(home.join("log")).unwrap(),
        "dirs\nsession\n"
    );
    let hypr = read("desktop/overlay/usr/share/wolos/hypr/hyprland.conf");
    assert!(
        hypr.lines()
            .any(|l| l.starts_with("exec-once = gnome-keyring-daemon --start --components=secrets")),
        "wol-login's login keyring daemon is finished by the session (D-Bus)"
    );
}

/// Printing and removable media are on in live and installed systems (the WolOS preset).
#[test]
fn printing_and_removable_media_are_on() {
    let preset = read("files/usr/lib/systemd/system-preset/50-wolos.preset");
    for u in ["cups.socket", "avahi-daemon.service", "udisks2.service"] {
        assert!(preset.lines().any(|l| l == format!("enable {u}")), "{u}");
    }
    sh_n("desktop/overlay/usr/bin/wol-session");
}

#[test]
fn helium_and_the_dock_apps_open_their_files_by_default() {
    let mime = read("desktop/overlay/usr/share/applications/mimeapps.list");
    for t in [
        "x-scheme-handler/http=helium.desktop;",
        "x-scheme-handler/https=helium.desktop;",
        "text/html=helium.desktop;",
        "application/xhtml+xml=helium.desktop;",
        "x-scheme-handler/mailto=org.gnome.Geary.desktop;",
        "inode/directory=org.gnome.Nautilus.desktop;",
        "application/pdf=org.gnome.Papers.desktop;",
        "text/plain=org.gnome.TextEditor.desktop;",
        "image/png=org.gnome.eog.desktop;",
        "audio/flac=io.bassi.Amberol.desktop;",
        "video/mp4=io.github.celluloid_player.Celluloid.desktop;",
        "application/zip=org.gnome.FileRoller.desktop;",
    ] {
        assert!(mime.lines().any(|l| l == t), "mimeapps.list lacks {t}");
    }
    assert!(!mime.to_lowercase().contains("firefox"));
    assert!(mime.starts_with('#') && mime.contains("\n[Default Applications]\n"));
}

/// e2e D1: web apps named the package (`helium-browser`) as their program and never opened. Web
/// apps, the browser theme files and the default-browser entry all name the program the
/// `helium-browser` package really installs.
#[test]
fn web_apps_and_the_default_browser_run_the_program_the_package_installs() {
    let bin = wol_core::webapps::BROWSER;
    assert_eq!(bin, wol_core::theme::helium::HELIUM.bin);
    let pkg = read("helium-browser/PKGBUILD");
    assert!(
        pkg.lines()
            .any(|l| l.trim() == format!("install -Dm755 helium.sh \"$pkgdir{bin}\"")),
        "helium-browser/PKGBUILD does not install {bin}"
    );
    assert!(
        !pkg.contains("/usr/bin/helium-browser"),
        "a second launcher name would hide which one the image has"
    );
    let entry = read("helium-browser/helium.desktop");
    let exec = entry
        .lines()
        .find_map(|l| l.strip_prefix("Exec="))
        .and_then(|e| e.split_whitespace().next())
        .unwrap();
    assert_eq!(Some(exec), bin.rsplit('/').next(), "helium.desktop Exec");
    let mime = read("desktop/overlay/usr/share/applications/mimeapps.list");
    assert!(
        mime.lines()
            .any(|l| l == "x-scheme-handler/https=helium.desktop;")
    );
}

/// e2e D2: Night Light's program was not in the image, so the toggle, Ctrl+Super+N and the
/// Control Center tile did nothing. The image installs it (wolos-iso profile) for the session's
/// user unit (wolos-hyprsunset.service, part of wolos-session.target since QA-CRASH-5) to start.
#[test]
fn night_light_has_its_program_in_the_image() {
    let p = wol_core::toggles::NIGHT_LIGHT_PROGRAM;
    let pkgs = read("profile/packages.desktop");
    assert!(
        pkgs.lines()
            .any(|l| l.split('#').next().unwrap_or("").trim() == p),
        "profile/packages.desktop lacks {p}"
    );
    let unit = read("files/usr/lib/systemd/user/wolos-hyprsunset.service");
    assert!(
        unit.lines()
            .any(|l| l.starts_with("ExecStart=") && l.contains(&format!("exec {p}"))),
        "{unit}"
    );
    assert!(
        read("files/usr/lib/systemd/user/wolos-session.target")
            .contains("wolos-hyprsunset.service")
    );
}

/// The live user has no password: a lock screen after five idle minutes could not be left. The
/// idle daemon's unit does not run on the live medium.
#[test]
fn the_live_desktop_never_locks_itself() {
    let unit = read("files/usr/lib/systemd/user/wolos-hypridle.service");
    assert!(
        unit.lines()
            .any(|l| l.trim() == "ConditionPathExists=!/etc/wolos-live"),
        "{unit}"
    );
    let hypr = read("desktop/overlay/usr/share/wolos/hypr/hyprland.conf");
    assert!(
        !hypr.lines().any(|l| l.contains("exec hypridle")),
        "hypridle runs from its unit only"
    );
}
