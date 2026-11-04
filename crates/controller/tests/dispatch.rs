// SPDX-License-Identifier: GPL-2.0-or-later
//! Every action against a recording fake (IaraOS DispatchTests).
use controller::cli::{TREE, parse};
use controller::run::{FakeEnv, execute};
use std::path::PathBuf;

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

fn temp_home(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-ctl-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn every_action_has_a_handler() {
    for c in TREE {
        let mut env = FakeEnv {
            home: temp_home("all"),
            ..Default::default()
        };
        let mut argv: Vec<&str> = c.path.to_vec();
        for a in c.args.iter().filter(|a| !a.optional) {
            argv.push(a.choices.first().copied().unwrap_or("x"));
        }
        let inv = parse(&argv).unwrap();
        execute(&inv, &mut env);
        assert!(!env.err.contains("no handler"), "{}: {}", c.action, env.err);
    }
}

#[test]
fn shell_panels_send_one_line_to_the_shell_socket() {
    for (line, wire) in [
        ("shell toggle switcher", "toggle switcher\n"),
        ("shell toggle notifications", "toggle notifications\n"),
        ("shell toggle session", "toggle session\n"),
        ("shell show session", "show session\n"),
        ("shell reload", "reload\n"),
    ] {
        let mut env = FakeEnv {
            vars: vec![("XDG_RUNTIME_DIR".into(), "/run/u".into())],
            replies: vec![String::new()],
            ..Default::default()
        };
        assert_eq!(run(line, &mut env), 0, "{line}");
        assert_eq!(
            env.requests,
            vec![(
                PathBuf::from("/run/u/wol-shell.sock"),
                wire.as_bytes().to_vec()
            )]
        );
    }
    let mut env = FakeEnv {
        vars: vec![("XDG_RUNTIME_DIR".into(), "/run/u".into())],
        ..Default::default()
    };
    assert_eq!(
        run("shell toggle session", &mut env),
        69,
        "shell not running"
    );
}

#[test]
fn theme_light_dark_toggle_write_the_active_file() {
    let home = temp_home("theme");
    let mut env = FakeEnv {
        home: home.clone(),
        ..Default::default()
    };
    let active = home.join(shared::theme::ACTIVE_FILE);
    for (line, want) in [
        ("theme dark", "wol-dark"),
        ("theme toggle", "wol-light"),
        ("theme toggle", "wol-dark"),
        ("theme light", "wol-light"),
    ] {
        assert_eq!(run(line, &mut env), 0, "{line}: {}", env.err);
        assert_eq!(
            std::fs::read_to_string(&active).unwrap().trim(),
            want,
            "{line}"
        );
    }
    env.out.clear();
    run("theme", &mut env);
    assert_eq!(env.out, "wol-light\n");
    assert_eq!(run("theme set no-such-theme", &mut env), 66);
    env.out.clear();
    run("theme css", &mut env);
    assert!(env.out.contains("--accent-text") && env.out.contains("--radius-m"));
}

#[test]
fn theme_set_restyles_third_party_apps() {
    let home = temp_home("apps");
    let mut env = FakeEnv {
        home: home.clone(),
        ..Default::default()
    };
    assert_eq!(run("theme dark", &mut env), 0, "{}", env.err);
    let css = std::fs::read_to_string(home.join("gtk-4.0/gtk.css")).unwrap();
    assert!(css.contains("--accent-bg-color: #8b6cff;"), "{css}");
    assert!(home.join("wolos/toolkits/qt6ct/colors/WolOS.conf").exists());
    let set = |k: &str, v: &str| {
        env.execs
            .iter()
            .any(|a| a.len() == 5 && a[0] == "gsettings" && a[3] == k && a[4] == v)
    };
    assert!(set("color-scheme", "prefer-dark"), "{:?}", env.execs);
    assert!(set("button-layout", ":minimize,maximize,close"));
    env.execs.clear();
    assert_eq!(run("theme light", &mut env), 0);
    let css = std::fs::read_to_string(home.join("gtk-4.0/gtk.css")).unwrap();
    assert!(css.contains("--accent-bg-color: #7c5cff;"));
    assert!(
        env.execs
            .iter()
            .any(|a| a.last().map(String::as_str) == Some("default"))
    );
}

/// The compositor's title strip follows the theme, so it is not dark on a light desktop (live:
/// `hyprctl eval` of the theme file's `hl.config` calls).
#[test]
fn theme_set_colours_the_title_strip() {
    let home = temp_home("strip");
    let mut env = FakeEnv {
        home: home.clone(),
        ..Default::default()
    };
    let set = |key: &str, colour: &str| {
        format!("hl.config({{ [\"plugin.hyprbars.{key}\"] = \"{colour}\" }})")
    };
    let bar = |env: &FakeEnv| {
        env.execs
            .iter()
            .find(|a| {
                a[..2] == ["hyprctl", "eval"] && a[2].contains("\"plugin.hyprbars.bar_color\"")
            })
            .map(|a| a[2].clone())
    };
    assert_eq!(
        run("theme set catppuccin-latte", &mut env),
        0,
        "{}",
        env.err
    );
    assert_eq!(bar(&env), Some(set("bar_color", "rgba(eff1f5e6)")));
    assert!(
        env.execs.iter().any(|a| a[..]
            == [
                "hyprctl".to_string(),
                "eval".into(),
                set("col.text", "rgba(4c4f69ff)")
            ]),
        "{:?}",
        env.execs
    );
    env.execs.clear();
    assert_eq!(run("theme set lumon", &mut env), 0, "{}", env.err);
    assert_eq!(bar(&env), Some(set("bar_color", "rgba(16242de6)")));
}

/// Upgrade to the Lua configuration: the first `wol theme set` (session start) carries the 0.56
/// `.conf` files over, removes them (a dangling link too) and reloads the compositor once; once
/// they are gone it does nothing.
#[test]
fn theme_set_carries_the_old_compositor_files_over_once() {
    let home = temp_home("retire");
    let wolos = home.join("hypr/wolos");
    std::fs::create_dir_all(&wolos).unwrap();
    std::fs::write(
        wolos.join("keyboard.conf"),
        "input {\n    kb_layout = br\n    repeat_rate = 40\n}\n",
    )
    .unwrap();
    std::fs::write(
        wolos.join("displays.conf"),
        "monitor = eDP-1,1920x1080@60.00,auto,1.5\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(home.join("gone/theme.conf"), wolos.join("theme.conf")).unwrap();
    let mut env = FakeEnv {
        home: home.clone(),
        ..Default::default()
    };
    let reloads = |env: &FakeEnv| {
        env.execs
            .iter()
            .filter(|a| a[..] == ["hyprctl", "reload"])
            .count()
    };
    assert_eq!(run("theme set wol-dark", &mut env), 0, "{}", env.err);
    let mut removed = env.removed.clone();
    removed.sort();
    assert_eq!(
        removed,
        ["displays.conf", "keyboard.conf", "theme.conf"].map(|f| wolos.join(f))
    );
    let wrote = |f: &str| {
        env.written
            .iter()
            .find(|w| w.0 == wolos.join(f))
            .map(|w| w.1.clone())
            .unwrap_or_else(|| panic!("{f} written"))
    };
    let kb = wrote("keyboard.lua");
    assert!(
        kb.contains("kb_layout = \"br\",") && kb.contains("repeat_rate = 40,"),
        "{kb}"
    );
    assert_eq!(
        wrote("displays.lua"),
        "-- Written by WolOS Settings.\nhl.monitor({ output = \"eDP-1\", mode = \"1920x1080@60.00\", position = \"auto\", scale = \"1.5\" })\n"
    );
    assert_eq!(reloads(&env), 1);
    for f in &removed {
        std::fs::remove_file(f).unwrap();
    }
    let mut env = FakeEnv {
        home: home.clone(),
        ..Default::default()
    };
    assert_eq!(run("theme set wol-dark", &mut env), 0, "{}", env.err);
    assert_eq!((reloads(&env), env.removed.len()), (0, 0), "once");
}

/// `wol theme set <id>` in a fresh HOME writes every toolkit's files from that theme.
#[test]
fn theme_set_writes_every_toolkit() {
    let home = temp_home("toolkits");
    let data = home.join("data");
    let mut env = FakeEnv {
        home: home.clone(),
        vars: vec![("XDG_DATA_HOME".into(), data.display().to_string())],
        ..Default::default()
    };
    assert_eq!(run("theme set retro-82", &mut env), 0, "{}", env.err);
    let read = |p: std::path::PathBuf| {
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    };
    let tk = home.join("wolos/toolkits");
    for (p, want) in [
        (home.join("gtk-4.0/gtk.css"), "WolOS retro-82"),
        (home.join("gtk-3.0/gtk.css"), "WolOS retro-82"),
        (home.join("qt6ct/qt6ct.conf"), "style=kvantum"),
        (home.join("Kvantum/WolOS/WolOS.kvconfig"), "WolOS retro-82"),
        (home.join("foot/foot.ini"), "WolOS retro-82"),
        (tk.join("firefox/wolos-chrome.css"), "WolOS retro-82"),
        (tk.join("zsh/colors.zsh"), "WOLOS_THEME=retro-82"),
        (
            data.join("gtksourceview-5/styles/wolos-dark.xml"),
            "WolOS retro-82",
        ),
        (
            data.join("gtksourceview-5/styles/wolos-light.xml"),
            "WolOS wol-light",
        ),
    ] {
        assert!(read(p.clone()).contains(want), "{}: {want}", p.display());
    }
    // WolOS-<id>-gtk3: libhandy strips a -dark suffix (wol_core::theme::apps::gtk3_theme_name).
    let gtk3 = read(data.join("themes/WolOS-retro-82-gtk3/gtk-3.0/gtk.css"));
    assert!(gtk3.contains("@define-color accent_bg_color #faa968;"));
    for css in [gtk3, read(home.join("gtk-3.0/gtk.css"))] {
        for bad in ["-gtk-icon-size", ":root", "var(", "color-mix("] {
            assert!(!css.contains(bad), "GTK 3 CSS uses GTK 4-only {bad}");
        }
    }
    assert!(
        env.execs
            .iter()
            .any(|a| a.len() == 5 && a[3] == "gtk-theme" && a[4] == "WolOS-retro-82-gtk3"),
        "a new GTK 3 theme name per WolOS theme: running GTK 3 apps reload"
    );
    // Helium (Chromium GTK mode) reads exactly these: GTK 3 theme, portal accent and scheme.
    for (k, v) in [
        ("accent-color", "orange"),
        ("color-scheme", "prefer-dark"),
        ("button-layout", ":minimize,maximize,close"),
    ] {
        assert!(
            env.execs
                .iter()
                .any(|a| a.len() == 5 && a[3] == k && a[4] == v),
            "{k}={v}: {:?}",
            env.execs
        );
    }
    assert!(
        read(tk.join("firefox/wolos-chrome.css"))
            .contains("--toolbox-bgcolor: #05182e !important;")
    );
    assert!(
        env.execs
            .iter()
            .any(|a| a.len() == 5 && a[2] == "org.gnome.TextEditor" && a[4] == "wolos-dark"),
        "{:?}",
        env.execs
    );
}

#[test]
fn sys_prints_json_when_not_a_tty() {
    let mut env = FakeEnv::default();
    assert_eq!(run("sys about", &mut env), 0);
    assert!(env.out.trim_start().starts_with('{'));
    let mut env = FakeEnv {
        tty: true,
        ..Default::default()
    };
    run("sys about", &mut env);
    assert!(env.out.contains("os: WolOS"));
    let mut env = FakeEnv::default();
    assert_eq!(run("sys abot", &mut env), 64);
    assert!(env.err.contains("did you mean 'about'"));
}

#[test]
fn jobs_talk_to_wold() {
    let mut env = FakeEnv {
        replies: vec![
            r#"[{"label":"a","state":"running","pid":3,"starts":1,"lastExit":null}]"#.into(),
        ],
        tty: true,
        ..Default::default()
    };
    assert_eq!(run("jobs", &mut env), 0);
    assert_eq!(
        env.requests[0],
        (PathBuf::from(shared::jobs::SOCKET), b"list\n".to_vec())
    );
    assert!(env.out.contains("running"));
    let mut env = FakeEnv {
        replies: vec![r#"{"ok":false,"error":"needs root"}"#.into()],
        ..Default::default()
    };
    assert_eq!(run("stop a", &mut env), 77);
    assert_eq!(env.requests[0].1, b"stop a\n");
    // E3: root's request failing with "denied" or "root" in the text is a failure, not a refusal.
    for error in ["permission denied: /var/lib/wol/x", "cannot chroot"] {
        let mut env = FakeEnv {
            replies: vec![format!(r#"{{"ok":false,"error":"{error}"}}"#)],
            root: true,
            ..Default::default()
        };
        assert_eq!(run("start a", &mut env), 1, "{error}");
        assert!(env.err.contains(error));
    }
    let mut env = FakeEnv::default();
    assert_eq!(run("jobs", &mut env), 69);
}

#[test]
fn admin_asks_for_password_only_when_needed() {
    let mut env = FakeEnv {
        replies: vec!["auth\n".into(), "ok\n".into()],
        stdin: vec!["pw".into()],
        ..Default::default()
    };
    assert_eq!(run("admin idle 300", &mut env), 0);
    assert_eq!(env.requests[0].1, b"idle\x001\x00300\x00");
    assert_eq!(env.requests[1].1, b"idle\x001\x00300\x00pw");
    for (replies, stdin, code) in [
        (vec!["notadmin\n"], vec![], 77),
        (vec!["invalid\n"], vec![], 64),
        (vec!["auth\n"], vec![], 77),
        (vec![], vec![], 69),
    ] {
        let mut env = FakeEnv {
            replies: replies.into_iter().map(String::from).collect(),
            stdin,
            ..Default::default()
        };
        assert_eq!(run("admin revoke", &mut env), code);
    }
    let mut env = FakeEnv::default();
    assert_eq!(run("admin rm-rf", &mut env), 64);
    assert_eq!(
        run("admin idle forever", &mut env),
        64,
        "validated before sending"
    );
    assert!(env.requests.is_empty());
}

#[test]
fn update_apply_needs_yes_and_root_and_rolls_back() {
    let mut env = FakeEnv::default();
    assert_eq!(run("update apply", &mut env), 64);
    assert_eq!(run("update apply --yes", &mut env), 77);
    assert!(env.execs.is_empty());
    // Running wolos-a; wolos-b holds the previous restore point; pacman fails in the new slot.
    let cmdline = (
        std::path::PathBuf::from("/proc/cmdline"),
        "root=LABEL=wolos rootflags=subvol=@wolos-a rw\n".to_string(),
    );
    let bootctl = (
        "bootctl list --json=short".to_string(),
        r#"[{"id":"wolos-a.conf","isSelected":true},{"id":"wolos-b.conf"}]"#.to_string(),
    );
    let mut env = FakeEnv {
        root: true,
        exec_codes: vec![0, 0, 0, 1],
        files: vec![cmdline.clone()],
        captures: vec![bootctl.clone()],
        ..Default::default()
    };
    assert_eq!(run("update apply wolos-b 2026/11/02 --yes", &mut env), 1);
    let ran: Vec<String> = env.execs.iter().map(|e| e.join(" ")).collect();
    assert_eq!(ran[0], "btrfs subvolume delete /run/wolos/top/@wolos-b");
    assert_eq!(ran[1], "btrfs subvolume snapshot / /run/wolos/top/@wolos-b");
    assert_eq!(ran[2], "wol layers fold /run/wolos/top/@wolos-b");
    assert!(ran[3].starts_with("sed -i "), "{ran:?}");
    assert_eq!(
        ran[4..],
        [
            "btrfs subvolume delete /run/wolos/top/@wolos-b",
            "rm -f /efi/loader/entries/wolos-b+3.conf /efi/EFI/wolos/wolos-b.efi"
        ],
        "failed: the new slot is removed"
    );
    // Never the running slot; the other one by default.
    let mut env = FakeEnv {
        files: vec![cmdline],
        captures: vec![bootctl],
        ..Default::default()
    };
    assert_eq!(run("update plan wolos-a", &mut env), 64);
    run("update plan", &mut env);
    assert!(
        env.out
            .contains("btrfs subvolume snapshot / /run/wolos/top/@wolos-b\n"),
        "{}",
        env.out
    );
    assert!(
        env.out
            .contains("arch-chroot /run/wolos/top/@wolos-b /usr/bin/pacman -Syu --noconfirm\n")
    );
}

#[test]
fn run_and_gpu_delegate_to_their_binaries() {
    let mut env = FakeEnv::default();
    run("run Game.wol -- -x", &mut env);
    run("gpu env", &mut env);
    run("gpu hyprconf", &mut env);
    assert_eq!(
        env.execs,
        [
            vec!["wol-run", "Game.wol", "-x"],
            vec!["wol-realtime", "gpu", "env"],
            vec!["wol-realtime", "gpu", "hyprconf"]
        ]
    );
}

#[test]
fn bundle_plan_resolves_for_this_host() {
    let arch = wol_core::bundle::host_arch();
    let info = format!(
        r#"{{"id":"org.x","version":"1","targets":[{{"abi":"linux","arch":"{arch}","entry":"l"}},{{"abi":"freebsd-native","arch":"{arch}","entry":"n"}}]}}"#
    );
    let mut env = FakeEnv {
        files: vec![(PathBuf::from("G.wol/Info.json"), info)],
        ..Default::default()
    };
    assert_eq!(run("port G.wol", &mut env), 0);
    assert!(env.out.contains("\"entry\":\"n\""), "{}", env.out);
    let mut env = FakeEnv::default();
    assert_eq!(run("port Missing.wol", &mut env), 66);
}

/// QA-STORE-1: `pacman -Qu` exits 1 with nothing on either stream when no update is waiting. After
/// a refresh that worked that is "up to date", not "pacman -Qu failed"; an exit 1 that says why is
/// still a failure, with pacman's own sentence.
#[test]
fn pacman_qu_exit1_means_nothing_pending() {
    let qu = |stderr: &str| FakeEnv {
        replies: vec!["ok\n".into()],
        statuses: vec![("pacman -Qu".into(), 1, stderr.into())],
        captures: vec![("pacman -Qu".into(), "".into())],
        ..Default::default()
    };
    let mut env = qu("");
    assert_eq!(run("update check --json", &mut env), 0, "{}", env.err);
    assert_eq!(env.out.trim(), "[]");
    let mut env = qu("error: failed to init transaction (unable to lock database)\n");
    assert_ne!(run("update check --json", &mut env), 0);
    assert!(env.err.contains("unable to lock database"), "{}", env.err);
    assert!(!env.out.contains("[]"), "{}", env.out);
}

/// QA-FDAY F10: a refresh that could not be written on a full disk says so, not "are you online?".
#[test]
fn check_on_a_full_disk_names_the_disk() {
    let df = |avail: u64| FakeEnv {
        captures: vec![
            ("pacman -Qu".into(), "".into()),
            (
                "df -B1 --output=avail /var".into(),
                format!("Avail\n{avail}\n"),
            ),
        ],
        ..Default::default()
    };
    let mut env = df(0);
    assert_ne!(run("update check --json", &mut env), 0);
    assert!(env.err.contains("no room left"), "{}", env.err);
    assert!(env.err.contains("Settings > Storage"), "{}", env.err);
    let mut env = df(40 << 30);
    assert_ne!(run("update check --json", &mut env), 0);
    assert!(env.err.contains("online"), "{}", env.err);
}

/// QA-STORE-15 / QA-STORE-13: the text a person reads is a sentence, and an update built in the
/// other slot says it needs the restart.
#[test]
fn update_check_and_install_speak_in_sentences() {
    let mut env = FakeEnv {
        tty: true,
        replies: vec!["ok\n".into()],
        captures: vec![(
            "pacman -Qu".into(),
            "wol-shell 0.12.1-1 -> 0.12.2-1\nlinux-wolos 6.17-1 -> 6.18-1\n".into(),
        )],
        ..Default::default()
    };
    assert_eq!(run("update check", &mut env), 0);
    assert_eq!(env.out, "2 updates available: wol-shell, linux-wolos\n");
    let line = controller::run::installed_line;
    assert_eq!(
        line(1, false, false),
        "Installed 1 update. Restart to finish.\n"
    );
    assert_eq!(line(3, true, false), "Installed 3 updates.\n");
    assert_eq!(
        line(2, true, true),
        "Installed 2 updates. Restart to use the new kernel.\n"
    );
}

/// QA-HUNT-3: 77 is "needs the administrator's password". A job's own error text with "denied" or
/// "root" in it (root's `permission denied: /var/lib/...`) is a failure (1), not that refusal.
#[test]
fn denied_text_is_not_exit_77() {
    for error in [
        "permission denied: /var/lib/wol/x",
        "cannot remove root dir",
    ] {
        let reply = format!(r#"{{"ok":false,"error":"{error}"}}"#);
        let mut env = FakeEnv {
            replies: vec![reply.clone()],
            root: true,
            ..Default::default()
        };
        assert_eq!(run("start a", &mut env), 1, "{error}");
        assert!(env.err.contains(error), "{}", env.err);
        // The refusal wold itself makes for a person without the right is still 77.
        let mut env = FakeEnv {
            replies: vec![reply],
            ..Default::default()
        };
        assert_eq!(run("stop a", &mut env), 77, "{error}");
    }
}

const SLOTS: &str = r#"[
 {"id":"wolos-a.conf","version":"2026/10/20","isSelected":true,"isDefault":false},
 {"id":"wolos-b+3.conf","version":"2026/11/02","isSelected":false,"isDefault":true,"triesLeft":3}
]"#;

/// QA-STORE-5: a boot entry whose root subvolume is gone (an interrupted update) is not a restore
/// point, in the list and for `restore activate`.
#[test]
fn restore_list_drops_a_slot_without_its_subvolume() {
    let env = |top: &str| FakeEnv {
        captures: vec![
            ("bootctl list --json=short".into(), SLOTS.into()),
            ("ls /run/wolos/top".into(), top.into()),
        ],
        ..Default::default()
    };
    let mut e = env("@wolos-a\n");
    assert_eq!(run("restore list --json", &mut e), 0, "{}", e.err);
    let v: serde_json::Value = serde_json::from_str(e.out.trim()).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1, "{}", e.out);
    assert_eq!(v[0]["name"], "wolos-a");
    let mut e = env("@wolos-a\n");
    assert_eq!(run("restore activate wolos-b", &mut e), 66, "{}", e.err);
    let mut e = env("@wolos-a\n@wolos-b\n");
    assert_eq!(run("restore list --json", &mut e), 0);
    assert!(e.out.contains("wolos-b"), "{}", e.out);
}

/// QA-STORE-5: one copy of WolOS says so, instead of "does not boot WolOS root slots".
#[test]
fn restore_list_one_copy_sentence() {
    let mut e = FakeEnv {
        files: vec![(PathBuf::from("/etc/wolos/single-copy"), String::new())],
        ..Default::default()
    };
    assert_eq!(run("restore list", &mut e), 69);
    assert!(e.err.contains("one copy of WolOS"), "{}", e.err);
    assert!(!e.err.contains("root slots"), "{}", e.err);
}

/// QA-100 LIVE3-19: a failed administrator change says why. wol-admind's `failed <sentence>` reply
/// reaches the person; a plain `failed` keeps the generic line.
#[test]
fn admin_failure_names_cause() {
    let line = shared::admin::failed_line("The change failed: modprobe: ERROR: No such device");
    let mut env = FakeEnv {
        replies: vec![format!("{line}\n")],
        ..Default::default()
    };
    assert_eq!(run("admin hw-load coretemp", &mut env), 1);
    assert!(
        env.err
            .contains("wol admin: The change failed: modprobe: ERROR: No such device"),
        "{}",
        env.err
    );
    let mut plain = FakeEnv {
        replies: vec!["failed\n".into()],
        ..Default::default()
    };
    assert_eq!(run("admin hw-load coretemp", &mut plain), 1);
    assert_eq!(plain.err, "wol admin: The change failed\n");
}

/// QA-100: `wol admin reports on|off` and `wol admin auto-update metered on|off` reach wol-admind as
/// those requests (Settings > Privacy and Software Update send the same ones); bad words never leave.
#[test]
fn admin_reports_and_metered_requests() {
    use shared::admin::{AdminAction, decode};
    let sent = |line: &str| {
        let mut env = FakeEnv {
            replies: vec!["ok\n".into()],
            ..Default::default()
        };
        let code = run(line, &mut env);
        let req = env
            .requests
            .first()
            .and_then(|(_, b)| decode(b))
            .map(|(r, _)| (r.action, r.args));
        (code, req)
    };
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        sent("admin reports on"),
        (0, Some((AdminAction::Reports, args(&["on"]))))
    );
    assert_eq!(
        sent("admin reports off"),
        (0, Some((AdminAction::Reports, args(&["off"]))))
    );
    assert_eq!(
        sent("admin auto-update metered on"),
        (0, Some((AdminAction::AutoUpdate, args(&["metered", "on"]))))
    );
    let (code, req) = sent("admin reports maybe");
    assert_ne!(code, 0);
    assert_eq!(req, None, "refused before it was sent");
    let (code, req) = sent("admin auto-update metered maybe");
    assert_ne!(code, 0);
    assert_eq!(req, None);
}
