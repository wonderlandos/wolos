//! Everyday userland from the real-GPU run: sudo for administrators, zsh for every account, user
//! folders, unlocked keyring, printing, removable media and default apps (Firefox the browser).
//! Scripts run on the host with fake FreeBSD tools.
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
    ["image/packages-base.txt", "desktop/packages.txt"]
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
        "sudo",
        "zsh",
        "zsh-autosuggestions",
        "zsh-syntax-highlighting",
        "zsh-completions",
        "xdg-user-dirs",
        "gnome-keyring",
        "cups",
        "cups-filters",
        "avahi-app",
        "fusefs-ntfs",
        "fusefs-exfat",
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
}

#[test]
fn administrators_can_sudo() {
    // Setup made an admin (group wheel) but the stock sudoers has no %wheel rule.
    let p = "desktop/overlay/usr/local/etc/sudoers.d/wolos";
    let rules: Vec<String> = read(p)
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(String::from)
        .collect();
    assert_eq!(rules, ["%wheel ALL=(ALL:ALL) ALL"]);
    assert!(
        read("image/build.sh").contains(r#"chmod 440 "$STAGE/usr/local/etc/sudoers.d/wolos""#),
        "sudoers drop-in must be 0440"
    );
    // The 15.1 sudo package's sudoers ends with `@includedir /usr/local/etc/sudoers.d` (checked on
    // the installed VM). visudo validates the syntax where the host has it.
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
    assert_eq!(wol_core::settings::users::LOGIN_SHELL, "/usr/local/bin/zsh");
    let b = read("image/build.sh");
    let live = b.lines().find(|l| l.contains("useradd live")).unwrap();
    assert!(live.contains("-s /usr/local/bin/zsh"), "{live}");
    // root stays on /bin/sh (base system): a broken /usr/local must not lock out recovery.
    for f in ["image/build.sh", "image/installerconfig", "image/rcconf.sh"] {
        let s = read(f);
        assert!(
            !s.contains("usermod root") && !s.contains("chsh"),
            "{f} changes root's shell"
        );
    }
    // Users' own files: only skel (new accounts) and system-level rc files, never ~/.zshrc.
    let skel = read("desktop/overlay/usr/share/skel/dot.zshrc");
    assert!(
        skel.lines().all(|l| l.starts_with('#')),
        "skel .zshrc is comments only"
    );
    // zsh's newuser module sources SITESCRIPT_DIR/newuser before the wizard script (strings of
    // zsh-5.9.2.pkg newuser.so: /usr/local/share/zsh/scripts, then .../5.9.2/scripts).
    let newuser = read("desktop/overlay/usr/local/share/zsh/scripts/newuser");
    assert!(
        newuser.lines().all(|l| l.starts_with('#')) && !newuser.contains("zsh-newuser-install"),
        "no zsh first-run wizard"
    );
}

#[test]
fn system_zshrc_loads_plugins_theme_colours_and_keys() {
    let rc = "desktop/overlay/usr/local/etc/zshrc";
    let text = read(rc);
    // FreeBSD ports paths (pkg-plist of shells/zsh-autosuggestions, shells/zsh-syntax-highlighting).
    for p in [
        "${WOLOS_ZSH_PLUGINS:-/usr/local/share}",
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
    for f in [rc, "desktop/overlay/usr/local/etc/zprofile"] {
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
        .arg(repo("desktop/overlay/usr/local/bin/wol-session"))
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
    let hypr = read("desktop/overlay/usr/local/share/wolos/hypr/hyprland.conf");
    assert!(
        hypr.lines()
            .any(|l| l.starts_with("exec-once = gnome-keyring-daemon --start --components=secrets")),
        "wol-login's login keyring daemon is finished by the session (D-Bus)"
    );
}

/// Runs image/rcconf.sh on a temp root with a fake sysrc; returns (sysrc log, auto_master).
fn run_rcconf() -> (String, String) {
    let d = sandbox(
        "rcconf",
        &[("sysrc", "#!/bin/sh\necho \"$@\" >> \"$LOG\"\n")],
    );
    let root = d.join("root");
    fs::create_dir_all(root.join("etc")).unwrap();
    fs::write(
        root.join("etc/auto_master"),
        "/net\t\t-hosts\t\t-nobrowse,nosuid,intr\n#/media\t\t-media\t\t-nosuid\n",
    )
    .unwrap();
    for _ in 0..2 {
        let st = Command::new("sh")
            .arg(repo("image/rcconf.sh"))
            .arg(&root)
            .env("PATH", path_with(&d))
            .env("LOG", d.join("log"))
            .status()
            .unwrap();
        assert!(st.success());
    }
    (
        fs::read_to_string(d.join("log")).unwrap(),
        fs::read_to_string(root.join("etc/auto_master")).unwrap(),
    )
}

#[test]
fn printing_and_removable_media_are_on_in_live_and_installed_systems() {
    let (log, master) = run_rcconf();
    let inst = read("image/installerconfig");
    for kv in [
        "cupsd_enable=YES",
        "avahi_daemon_enable=YES",
        "autofs_enable=YES",
    ] {
        assert!(log.contains(kv), "rcconf.sh lacks {kv}:\n{log}");
        assert!(inst.contains(kv), "installerconfig lacks {kv}");
    }
    let media: Vec<&str> = master.lines().filter(|l| l.starts_with("/media")).collect();
    assert_eq!(media, ["/media\t\t-wolmedia\t-nosuid,noatime"], "{master}");
    assert!(
        inst.contains("-wolmedia"),
        "installerconfig lacks the /media map"
    );
    for s in [
        "image/rcconf.sh",
        "image/installerconfig",
        "image/build.sh",
        "desktop/overlay/etc/autofs/special_wolmedia",
        "desktop/overlay/usr/local/bin/wol-session",
    ] {
        sh_n(s);
    }
}

#[test]
fn usb_sticks_mount_writable_for_wolos_accounts() {
    // Stock /etc/autofs/special_media lines (FreeBSD 15.1) per filesystem.
    let stock = "#!/bin/sh\n[ $# -eq 0 ] && { echo da0s1; echo da1p1; exit 0; }\ncase $1 in\n\
        fat) echo '-fstype=msdosfs,sync   :/dev/da0s1' ;;\n\
        exfat) echo '-mountprog=/usr/local/sbin/mount.exfat,fstype=exfat,sync     :/dev/da0s1' ;;\n\
        ntfs) echo '-mountprog=/usr/local/bin/ntfs-3g,fstype=ntfs,sync  :/dev/da0s1' ;;\n\
        ufs) echo '-fstype=ufs,async  :/dev/da0p1' ;;\n\
        *) echo 'Cannot mount' >&2; exit 1 ;;\nesac\n";
    let d = sandbox("media", &[("special_media", stock)]);
    let run = |args: &[&str]| {
        Command::new("sh")
            .arg(repo("desktop/overlay/etc/autofs/special_wolmedia"))
            .args(args)
            .env("WOLOS_SPECIAL_MEDIA", d.join("bin/special_media"))
            .output()
            .unwrap()
    };
    for (key, want) in [
        (
            "fat",
            "-fstype=msdosfs,gid=5,mask=436,dirmask=509,sync   :/dev/da0s1",
        ),
        (
            "exfat",
            "-mountprog=/usr/local/sbin/mount.exfat,fstype=exfat,allow_other,gid=5,umask=002,sync     :/dev/da0s1",
        ),
        (
            "ntfs",
            "-mountprog=/usr/local/bin/ntfs-3g,fstype=ntfs,allow_other,gid=5,umask=002,sync  :/dev/da0s1",
        ),
        ("ufs", "-fstype=ufs,async  :/dev/da0p1"),
    ] {
        let o = run(&[key]);
        assert!(o.status.success());
        assert_eq!(String::from_utf8_lossy(&o.stdout).trim_end(), want, "{key}");
    }
    assert_eq!(String::from_utf8_lossy(&run(&[]).stdout), "da0s1\nda1p1\n");
    assert!(!run(&["none"]).status.success(), "errors pass through");
    let mode = fs::metadata(repo("desktop/overlay/etc/autofs/special_wolmedia"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111, "automountd executes the map");
}

#[test]
fn firefox_and_the_dock_apps_open_their_files_by_default() {
    let mime = read("desktop/overlay/usr/local/share/applications/mimeapps.list");
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
