//! Everyday userland from the real-GPU run: sudo for administrators, zsh for every account, user
//! folders, unlocked keyring, printing, removable media and default apps (Firefox the browser).
//! Scripts run on the host with fakes.
use std::{fs, os::unix::fs::PermissionsExt, path::Path, path::PathBuf, process::Command};

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn packages() -> Vec<String> {
    ["image/arch/packages.x86_64", "desktop/packages.txt"]
        .iter()
        .flat_map(|f| {
            read(f)
                .lines()
                .filter_map(|l| l.split('#').next())
                .flat_map(|l| l.split_whitespace().map(String::from).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        })
        .collect()
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
fn image_ships_the_everyday_userland() {
    let pkgs = packages();
    for p in [
        "zsh-autosuggestions",
        "zsh-syntax-highlighting",
        "zsh-completions",
        "xdg-user-dirs",
        "gnome-keyring",
        "cups",
        "cups-filters",
        "avahi",
        "nss-mdns",
        "udisks2",
        "gvfs",
        "ntfs-3g",
        "exfatprogs",
        "nautilus",
        "geary",
        "amberol",
        "shotwell",
        "gnome-calendar",
        "gnote",
        "gnome-calculator",
        "gnome-clocks",
        "papers",
        "celluloid",
        "mpv",
        "file-roller",
        "baobab",
        "gnome-text-editor",
        "eog",
        "firefox",
    ] {
        assert!(pkgs.iter().any(|x| x == p), "packages.txt lacks {p}");
    }
    // Weather is the WolOS desktop widget: gnome-weather needs geoclue and gjs + spidermonkey (~55 MB).
    assert!(!pkgs.iter().any(|x| x == "gnome-weather"));
    // sudo-rs and zsh come with wolos-base (ADR 0017).
    let base = read("image/repo/wolos-base/PKGBUILD");
    assert!(base.contains("'sudo-rs'") && base.contains("'zsh'"));
}

#[test]
fn administrators_can_sudo() {
    // Setup made an admin (group wheel) but the stock sudoers has no %wheel rule.
    let p = "desktop/overlay/etc/sudoers.d/wolos";
    let rules: Vec<String> = read(p)
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(String::from)
        .collect();
    assert_eq!(rules, ["%wheel ALL=(ALL:ALL) ALL"]);
    assert!(
        read("image/repo/wolos-system/PKGBUILD")
            .contains(r#"chmod 440 "$pkgdir/etc/sudoers.d/wolos""#),
        "sudoers drop-in must be 0440"
    );
    // sudo-rs reads /etc/sudoers, whose Arch default ends with `@includedir /etc/sudoers.d`.
    // visudo validates the syntax where the host has it.
    if let Ok(out) = Command::new("visudo").arg("-csf").arg(repo(p)).output() {
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn every_account_logs_in_to_zsh_and_root_keeps_sh() {
    assert!(wol_core::settings::users::LOGIN_SHELL.ends_with("/bin/zsh"));
    let b = read("image/build.sh");
    let live = b.lines().find(|l| l.starts_with("u live ")).unwrap();
    assert!(live.ends_with(" /usr/bin/zsh"), "{live}");
    // root keeps bash (base system): a broken desktop package must not lock out recovery.
    assert!(
        !b.contains("usermod root") && !b.contains("chsh"),
        "image/build.sh changes root's shell"
    );
    // Users' own files: only skel (new accounts) and system-level rc files, never ~/.zshrc.
    let skel = read("desktop/overlay/usr/share/skel/dot.zshrc");
    assert!(
        skel.lines().all(|l| l.starts_with('#')),
        "skel .zshrc is comments only"
    );
    // zsh's newuser module sources SITESCRIPT_DIR/newuser before the wizard script.
    let newuser = read("desktop/overlay/usr/share/zsh/scripts/newuser");
    assert!(
        newuser.lines().all(|l| l.starts_with('#')) && !newuser.contains("zsh-newuser-install"),
        "no zsh first-run wizard"
    );
}

#[test]
fn system_zshrc_loads_plugins_theme_colours_and_keys() {
    let rc = "desktop/overlay/etc/zshrc";
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
    for f in [rc, "desktop/overlay/etc/zprofile"] {
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
    let preset = read("image/files/usr/lib/systemd/system-preset/50-wolos.preset");
    for u in ["cups.socket", "avahi-daemon.service", "udisks2.service"] {
        assert!(preset.lines().any(|l| l == format!("enable {u}")), "{u}");
    }
    sh_n("desktop/overlay/usr/bin/wol-session");
}

#[test]
fn firefox_and_the_dock_apps_open_their_files_by_default() {
    let mime = read("desktop/overlay/usr/share/applications/mimeapps.list");
    for t in [
        "x-scheme-handler/http=firefox.desktop;",
        "x-scheme-handler/https=firefox.desktop;",
        "text/html=firefox.desktop;",
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
    assert!(!mime.to_lowercase().contains("helium"));
    assert!(mime.starts_with('#') && mime.contains("\n[Default Applications]\n"));
}
