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

/// MANUAL 0.12.0: `wol appearance set dark` left the open terminal light. The session links foot.ini
/// to the shipped (light) default before the first theme; the theme now takes that link over, and
/// running foot terminals of this account switch sections by signal.
#[test]
fn appearance_reaches_the_open_terminal() {
    let home = temp_home("foot");
    std::fs::create_dir_all(home.join("foot")).unwrap();
    std::os::unix::fs::symlink("/usr/share/wolos/foot/foot.ini", home.join("foot/foot.ini"))
        .unwrap();
    let mut env = FakeEnv {
        home: home.clone(),
        ..Default::default()
    };
    let uid = shared::sys::euid().to_string();
    let signal = |env: &FakeEnv| {
        env.execs
            .iter()
            .find(|a| a.first().is_some_and(|x| x == "pkill"))
            .cloned()
    };
    assert_eq!(run("appearance set dark", &mut env), 0, "{}", env.err);
    let foot = std::fs::read_to_string(home.join("foot/foot.ini")).unwrap();
    assert!(foot.contains("initial-color-theme=dark\n"), "{foot}");
    assert_eq!(
        signal(&env).unwrap(),
        [
            "pkill",
            "--signal",
            "USR1",
            "--uid",
            &uid,
            "--exact",
            "foot|footclient"
        ]
    );
    env.execs.clear();
    assert_eq!(run("appearance set light", &mut env), 0, "{}", env.err);
    assert_eq!(signal(&env).unwrap()[2], "USR2");
    // A foot.ini of the person's own: kept, and their terminals are left alone.
    std::fs::remove_file(home.join("foot/foot.ini")).unwrap();
    std::fs::write(home.join("foot/foot.ini"), "mine").unwrap();
    env.execs.clear();
    assert_eq!(run("appearance set dark", &mut env), 0, "{}", env.err);
    assert_eq!(signal(&env), None);
    assert_eq!(
        std::fs::read_to_string(home.join("foot/foot.ini")).unwrap(),
        "mine"
    );
}

/// MANUAL 0.12.0: `wol appearance set dark` printed about ten "ok" lines (hyprctl's answer to
/// each keyword). Those stay quiet; an error hyprctl answers, or no compositor, is said.
#[test]
fn appearance_keeps_hyprctl_quiet_but_its_errors() {
    let home = temp_home("quiet");
    let mut env = FakeEnv {
        home: home.clone(),
        captured: Some("ok\n".into()),
        ..Default::default()
    };
    assert_eq!(run("appearance set dark", &mut env), 0, "{}", env.err);
    assert!(!env.captured_argvs.is_empty());
    assert!(
        !env.out.contains("\nok\n") && !env.out.starts_with("ok"),
        "{}",
        env.out
    );
    assert!(!env.err.contains("ok"), "{}", env.err);
    env.captured = Some("config option <plugin:hyprbars:bar_color> does not exist.\n".into());
    env.err.clear();
    assert_eq!(run("appearance set light", &mut env), 0);
    assert!(
        env.err.contains("wol: plugin:hyprbars:bar_color "),
        "{}",
        env.err
    );
    assert!(env.err.contains("does not exist"), "{}", env.err);
    env.captured = None;
    env.captured_argvs.clear();
    env.err.clear();
    assert_eq!(run("appearance set dark", &mut env), 0);
    assert_eq!(env.captured_argvs.len(), 1, "stops after the first failure");
    assert_eq!(
        env.err.matches("hyprctl did not answer").count(),
        1,
        "{}",
        env.err
    );
}

/// The compositor's title strip follows the theme, so it is not dark on a light desktop.
#[test]
fn theme_set_colours_the_title_strip() {
    let home = temp_home("strip");
    let mut env = FakeEnv {
        home: home.clone(),
        captured: Some("ok\n".into()),
        ..Default::default()
    };
    let bar = |env: &FakeEnv| {
        env.captured_argvs
            .iter()
            .find(|a| a[..2] == ["hyprctl", "keyword"] && a[2] == "plugin:hyprbars:bar_color")
            .map(|a| a[3].clone())
    };
    assert_eq!(
        run("theme set catppuccin-latte", &mut env),
        0,
        "{}",
        env.err
    );
    assert_eq!(bar(&env).as_deref(), Some("rgba(eff1f5e6)"));
    assert!(
        env.captured_argvs.iter().any(|a| a[..]
            == [
                "hyprctl",
                "keyword",
                "plugin:hyprbars:col.text",
                "rgba(4c4f69ff)"
            ]),
        "{:?}",
        env.captured_argvs
    );
    env.captured_argvs.clear();
    assert_eq!(run("theme set lumon", &mut env), 0, "{}", env.err);
    assert_eq!(bar(&env).as_deref(), Some("rgba(16242de6)"));
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
            // A failed step may leave the copy bound for pacman: a mounted subvolume stays.
            "umount -R /run/wolos/top/@wolos-b",
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
