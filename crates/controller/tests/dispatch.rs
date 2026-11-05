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

/// QA-LIVE-21: Super+Esc and the other shell keybinds go through `wol shell`. With the shell
/// gone (the black desktop) the key itself starts the shell's user unit again, without waiting
/// for it; a `reload` (a settings change told to a shell that may not run) starts nothing.
#[test]
fn shell_keybind_starts_the_shell_when_it_is_gone() {
    let gone = || FakeEnv {
        vars: vec![("XDG_RUNTIME_DIR".into(), "/run/u".into())],
        ..Default::default()
    };
    let start = vec![
        "systemctl".to_owned(),
        "--user".into(),
        "start".into(),
        "--no-block".into(),
        "wolos-shell.service".into(),
    ];
    for line in [
        "shell toggle session",
        "shell show session",
        "shell hide osd",
    ] {
        let mut env = gone();
        assert_eq!(
            run(line, &mut env),
            69,
            "{line}: still an error for a script"
        );
        assert_eq!(env.execs, std::slice::from_ref(&start), "{line}");
        assert!(env.err.contains("starting it"), "{line}: {}", env.err);
    }
    let mut env = gone();
    assert_eq!(run("shell reload", &mut env), 69);
    assert!(env.execs.is_empty(), "{:?}", env.execs);
    // A shell that answers is left alone.
    let mut env = FakeEnv {
        vars: vec![("XDG_RUNTIME_DIR".into(), "/run/u".into())],
        replies: vec![String::new()],
        ..Default::default()
    };
    assert_eq!(run("shell toggle session", &mut env), 0);
    assert!(env.execs.is_empty());
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

/// QA-SET-63 (SET-R2-10): the session start restyles the apps for the theme in force without
/// saving one. `wol theme set "$(wol theme)"` at login wrote the theme file (Auto became a fixed
/// Light, for good) and journaled it, so Restore offered 'Appearance · Terminal · Undo' after
/// Setup. `wol theme apply` writes no theme file and no journal entry.
#[test]
fn theme_apply_restyles_without_saving_a_theme() {
    let home = temp_home("theme-apply");
    let mut env = FakeEnv {
        home: home.clone(),
        ..Default::default()
    };
    assert_eq!(run("theme apply", &mut env), 0, "{}", env.err);
    assert_eq!(env.out, "wol-light\n");
    assert!(
        !home.join(shared::theme::ACTIVE_FILE).exists(),
        "no theme file: Auto stays Auto"
    );
    let j = wol_core::journal::Journal::new(&home.join("state"), &home);
    assert!(j.entries().is_empty(), "nothing to undo: {:?}", j.entries());
    let css = std::fs::read_to_string(home.join("gtk-4.0/gtk.css")).unwrap();
    assert!(css.contains("--accent-bg-color: #7452f5;"), "{css}");
    // A saved theme is applied as it is, still without a new entry.
    assert_eq!(run("theme dark", &mut env), 0);
    let n = j.entries().len();
    env.out.clear();
    assert_eq!(run("theme apply", &mut env), 0);
    assert_eq!(env.out, "wol-dark\n");
    assert_eq!(j.entries().len(), n);
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
    assert!(css.contains("--accent-bg-color: #7452f5;"), "{css}");
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
    assert!(css.contains("--accent-bg-color: #7452f5;"));
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
        // The Lua call (`hyprctl eval` of an `hl.config`) names the option `plugin.hyprbars.…`.
        env.err.contains("wol: hl.config(") && env.err.contains("plugin.hyprbars.bar_color"),
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

/// The compositor's title strip follows the theme, so it is not dark on a light desktop (live:
/// `hyprctl eval` of the theme file's `hl.config` calls).
#[test]
fn theme_set_colours_the_title_strip() {
    let home = temp_home("strip");
    let mut env = FakeEnv {
        home: home.clone(),
        captured: Some("ok\n".into()),
        ..Default::default()
    };
    let set = |key: &str, colour: &str| {
        format!("hl.config({{ [\"plugin.hyprbars.{key}\"] = \"{colour}\" }})")
    };
    let bar = |env: &FakeEnv| {
        env.captured_argvs
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
        env.captured_argvs.iter().any(|a| a[..]
            == [
                "hyprctl".to_string(),
                "eval".into(),
                set("col.text", "rgba(4c4f69ff)")
            ]),
        "{:?}",
        env.captured_argvs
    );
    env.captured_argvs.clear();
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
    // Atlas (GTK mode) reads exactly these: GTK 3 theme, portal accent and scheme.
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
        exec_codes: vec![0, 0, 0, 0, 1],
        files: vec![cmdline.clone()],
        captures: vec![bootctl.clone()],
        ..Default::default()
    };
    assert_eq!(run("update apply wolos-b 2026/11/02 --yes", &mut env), 1);
    let ran: Vec<String> = env.execs.iter().map(|e| e.join(" ")).collect();
    let tx = "/run/wolos/top/@wolos-tx";
    let clean = [
        "find /run/wolos/top -maxdepth 1 -name @wolos-tx -exec umount -R {} ;".to_string(),
        "find /run/wolos/top -maxdepth 1 -name @wolos-tx -exec btrfs subvolume delete {} +"
            .to_string(),
    ];
    assert_eq!(ran[..2], clean, "a scratch copy a cut left goes first");
    // QA-LIVE4-7: and what a cut left of the slot's seal record.
    assert_eq!(
        ran[2],
        "find /efi -mindepth 2 -maxdepth 2 ( -path /efi/wolos-b/verity.new -o -path \
/efi/wolos-b/verity*.wol-tmp ) -delete"
    );
    assert_eq!(ran[3], format!("btrfs subvolume snapshot / {tx}"));
    assert_eq!(ran[4], format!("wol layers fold {tx}"));
    // QA-STORE-3: the fold failed before the swap: only the scratch copy goes; wolos-b (the
    // earlier restore point), its entries and its image stay.
    assert_eq!(
        ran[5..],
        [
            clean[0].clone(),
            clean[1].clone(),
            "rm -f /efi/wolos-b/verity.new /var/lib/wolos/update-building".to_owned(),
        ],
        "failed: the restore point is kept"
    );
    assert!(
        env.err.contains(wol_core::update::pkg::SLOT_FAILED),
        "{}",
        env.err
    );
    // QA-STORE-55: the person reads sentences, not the commands.
    assert!(
        !env.err.contains("btrfs") && !env.err.contains("/run/wolos"),
        "{}",
        env.err
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
            .contains("mv /run/wolos/top/@wolos-tx /run/wolos/top/@wolos-b\n"),
        "{}",
        env.out
    );
    assert!(
        env.out
            .contains("unshare --mount --propagation private --fork --kill-child=SIGKILL \
                wol update chroot /run/wolos/top/@wolos-tx -- /usr/bin/pacman -Syu --noconfirm\n")
    );
    // QA-STORE-76: an empty argument reads as '' (not a bare "bootctl set-default "); QA-LIVE4-7:
    // everything on the disk before "updated".
    assert!(
        env.out.ends_with("bootctl set-default ''\nsync\n"),
        "{}",
        env.out
    );
}

/// QA-STORE-76: `wol --version` prints the version, not the help; `wol layers` names its
/// commands instead of "did you mean 'layers'?". QA-PWR-6 (wd-settings) made that the rule for
/// every group: the group word alone is the help for the group, exit 0, listing its commands.
#[test]
fn version_and_group_words() {
    let mut env = FakeEnv::default();
    assert_eq!(run("--version", &mut env), 0);
    assert_eq!(env.out, format!("wol {}\n", env!("CARGO_PKG_VERSION")));
    let inv = parse(&["layers"]).unwrap();
    assert_eq!((inv.action, inv.get("command")), ("help", Some("layers")));
    let mut env = FakeEnv::default();
    assert_eq!(run("layers", &mut env), 0);
    for sub in ["layers list", "layers fold", "layers clear"] {
        assert!(env.out.contains(sub), "{sub}: {}", env.out);
    }
    let e = parse(&["layer"]).unwrap_err().to_string();
    assert!(e.contains("did you mean 'layers'"), "{e}");
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

/// QA-STORE-1: nothing pending after a refresh that worked is "up to date", never "pacman -Qu
/// failed". Since rel/0.12.1 the check reads the daily check's findings (wol-admind ran
/// `update-check now`; `pacman -Qu` never runs): a stamp from this request with an empty list is
/// `[]`; a stamp from before it (the refresh did not happen) is a failure, never `[]`.
#[test]
fn pacman_qu_exit1_means_nothing_pending() {
    // FakeEnv's clock reads 1000.
    let found = |checked: &str| FakeEnv {
        replies: vec!["ok\n".into()],
        files: vec![
            (
                PathBuf::from("/var/lib/wolos/updates/checked"),
                checked.into(),
            ),
            (PathBuf::from("/var/lib/wolos/updates/pkg"), String::new()),
        ],
        // A `pacman -Qu` run would say this; the check never asks it.
        statuses: vec![("pacman -Qu".into(), 1, String::new())],
        ..Default::default()
    };
    let mut env = found("1000\n");
    assert_eq!(run("update check --json", &mut env), 0, "{}", env.err);
    assert_eq!(env.out.trim(), "[]");
    assert!(
        env.execs.iter().all(|a| a[0] != "pacman"),
        "{:?}",
        env.execs
    );
    let mut env = found("5\n");
    assert_ne!(run("update check --json", &mut env), 0);
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
        files: vec![
            (
                PathBuf::from("/var/lib/wolos/updates/checked"),
                "1000\n".into(),
            ),
            (
                PathBuf::from("/var/lib/wolos/updates/pkg"),
                "wol-shell 0.12.1-1 -> 0.12.2-1\nlinux-wolos 6.17-1 -> 6.18-1\n".into(),
            ),
        ],
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

/// QA-NET-8 (B7): `wol wifi join` that fails exits non-zero with its cause: no Wi-Fi device,
/// a wrong password, an 802.1X network. `hidden` reaches NetworkManager as `hidden yes`.
#[test]
fn wifi_join_err_exits_nonzero() {
    use wol_core::system::fake::FakeHost;
    // No Wi-Fi device (NetworkManager answers, lists none).
    let h = std::sync::Arc::new(FakeHost::linux());
    h.command("nmcli -t -f TYPE dev", "ethernet\n");
    let mut env = FakeEnv {
        host: Some(h),
        stdin: vec!["foo12345".into()],
        ..Default::default()
    };
    let code = run("wifi join Foo", &mut env);
    assert_ne!(code, 0, "{}", env.err);
    assert!(env.err.contains("no Wi-Fi device"), "{}", env.err);
    assert!(!env.err.contains("password"), "{}", env.err);
    // An 802.1X network: said so, no password read.
    let h = std::sync::Arc::new(FakeHost::linux());
    h.command(
        "nmcli -t -f IN-USE,SIGNAL,SECURITY,SSID dev wifi list --rescan auto",
        " :70:WPA2 802.1X:Campus\n",
    );
    let mut env = FakeEnv {
        host: Some(h),
        stdin: vec!["password1".into()],
        ..Default::default()
    };
    assert_ne!(run("wifi join Campus", &mut env), 0);
    assert!(env.err.contains("802.1X"), "{}", env.err);
    // Hidden: the profile asks for the network by name.
    let h = std::sync::Arc::new(FakeHost::linux());
    h.slow(
        "nmcli connection add type wifi con-name Attic ifname * ssid Attic wifi-sec.key-mgmt wpa-psk 802-11-wireless.hidden yes",
        "",
    );
    let mut env = FakeEnv {
        host: Some(h.clone()),
        stdin: vec!["password1".into()],
        ..Default::default()
    };
    // The fake answers only the argv with `802-11-wireless.hidden yes`.
    assert_eq!(run("wifi join Attic hidden", &mut env), 0, "{}", env.err);
    let mut env = FakeEnv {
        host: Some(h),
        stdin: vec!["password1".into()],
        ..Default::default()
    };
    assert_ne!(
        run("wifi join Attic", &mut env),
        0,
        "not hidden: another argv"
    );
}

/// QA-STORE-50: a one-copy computer updated in place that fails has no slot to roll back: no
/// btrfs step runs and the sentence says some packages may already be updated.
#[test]
fn one_copy_failure_no_rollback() {
    let mut env = FakeEnv {
        root: true,
        exec_codes: vec![1],
        files: vec![(
            std::path::PathBuf::from(wol_core::update::SINGLE_COPY),
            String::new(),
        )],
        ..Default::default()
    };
    assert_eq!(run("update apply --yes", &mut env), 1);
    let ran: Vec<String> = env.execs.iter().map(|e| e.join(" ")).collect();
    assert_eq!(
        ran,
        ["/usr/bin/pacman -Syuw --noconfirm"],
        "nothing after the failure"
    );
    assert!(
        env.err.contains(wol_core::update::pkg::IN_PLACE_FAILED),
        "{}",
        env.err
    );
    assert!(!env.err.contains("removed"), "{}", env.err);
}

/// QA-LIVE4-8: after a cut, the next `wol update apply` succeeds at once: its first steps clear
/// a scratch copy the cut left (`find` runs nothing when there is none), and it ends by removing
/// its mark.
#[test]
fn retry_after_cut_succeeds() {
    let mut env = FakeEnv {
        root: true,
        files: vec![(
            std::path::PathBuf::from("/proc/cmdline"),
            "root=LABEL=wolos rootflags=subvol=@wolos-a rw\n".into(),
        )],
        captures: vec![(
            "bootctl list --json=short".into(),
            r#"[{"id":"wolos-a.conf","isSelected":true},{"id":"wolos-b.conf"}]"#.into(),
        )],
        ..Default::default()
    };
    assert_eq!(run("update apply --yes", &mut env), 0, "{}", env.err);
    let ran: Vec<String> = env.execs.iter().map(|e| e.join(" ")).collect();
    assert!(
        ran[1].contains("-name @wolos-tx -exec btrfs subvolume delete"),
        "{ran:?}"
    );
    assert!(ran[3].starts_with("btrfs subvolume snapshot / "), "{ran:?}");
    assert_eq!(ran[ran.len() - 3], "rm -f /var/lib/wolos/update-building");
    // QA-LIVE4-7: "restart to use it" only once everything is on the disk.
    assert_eq!(ran[ran.len() - 1], "sync");
    assert!(env.out.contains("restart"), "{}", env.out);
}

/// QA-SHADOW-44: a pacman that cannot start (missing in a broken slot) is a sentence and 127,
/// not a silent 127.
#[test]
fn pacman_exec_err_has_sentence() {
    let mut env = FakeEnv {
        exec_codes: vec![i32::MIN],
        ..Default::default()
    };
    assert_eq!(run("pkg -- -Q", &mut env), 127);
    assert!(env.err.contains("could not be started"), "{}", env.err);
    assert!(env.err.contains("Nothing was changed"), "{}", env.err);
}

/// QA-LIVE4-5: as a standard account `bootctl list` fails (the ESP is root's); `wol restore list`
/// reads the list boot-ok and esp-sync left in /run/wolos/slots.json and shows both slots.
#[test]
fn restore_list_reads_the_slot_list_as_a_user() {
    let list = concat!(
        r#"[{"id":"wolos-a.conf","isSelected":true,"isDefault":true},"#,
        r#"{"id":"wolos-b.conf","isSelected":false,"isDefault":false,"version":"3-2026.11.02"}]"#
    );
    let mut e = FakeEnv {
        files: vec![(
            std::path::PathBuf::from(wol_core::update::be::SLOTS_FILE),
            list.into(),
        )],
        ..Default::default()
    };
    assert_eq!(run("restore list --json", &mut e), 0, "{}", e.err);
    let v: serde_json::Value = serde_json::from_str(e.out.trim()).unwrap();
    let names: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["name"].as_str())
        .collect();
    assert_eq!(names, ["wolos-a", "wolos-b"], "{}", e.out);
}

/// QA-STORE-46, RH-192: `wol update chroot` (run by the slot build under `unshare`) binds the copy
/// onto itself and the shared package cache into it, then runs the command in it; pacman's own
/// flags pass through untouched. Only a slot copy, only root.
#[test]
fn update_chroot_binds_the_copy_and_the_cache() {
    let tx = "/run/wolos/top/@wolos-tx";
    let private = |own: &str| FakeEnv {
        root: true,
        captures: vec![
            (format!("realpath -e {tx}"), format!("{tx}\n")),
            ("readlink /proc/self/ns/mnt".into(), format!("{own}\n")),
            (
                "readlink /proc/1/ns/mnt".into(),
                "mnt:[4026531841]\n".into(),
            ),
        ],
        ..Default::default()
    };
    let line = format!("update chroot {tx} -- /usr/bin/pacman -Syu --noconfirm --ignore=gimp");
    let mut env = private("mnt:[4026532999]");
    assert_eq!(run(&line, &mut env), 0, "{}", env.err);
    let ran: Vec<String> = env.execs.iter().map(|e| e.join(" ")).collect();
    assert_eq!(
        ran,
        [
            format!("mount --bind {tx} {tx}"),
            format!("mkdir -p {tx}/var/cache/pacman/pkg"),
            format!("mount --bind /var/cache/pacman/pkg {tx}/var/cache/pacman/pkg"),
            format!("arch-chroot {tx} /usr/bin/pacman -Syu --noconfirm --ignore=gimp"),
        ]
    );
    // Review 3: in the system's own mount namespace (no `unshare`) it mounts nothing.
    let mut shared = private("mnt:[4026531841]");
    assert_ne!(run(&line, &mut shared), 0);
    assert!(shared.execs.is_empty(), "{:?}", shared.execs);
    assert!(
        shared.err.contains("private mount namespace"),
        "{}",
        shared.err
    );
    // Only the scratch copy, never a slot or another path, and not through a link.
    for other in [
        "/run/wolos/top/@wolos-a",
        "/run/wolos/top/@wolos-tx/../@wolos-a",
        "/",
    ] {
        let mut e = private("mnt:[4026532999]");
        assert_eq!(
            run(&format!("update chroot {other} -- ls"), &mut e),
            64,
            "{other}"
        );
        assert!(e.execs.is_empty());
    }
    let mut link = private("mnt:[4026532999]");
    link.captures[0].1 = "/home/eve/x\n".into();
    assert_ne!(run(&line, &mut link), 0);
    assert!(link.execs.is_empty());
    let mut user = FakeEnv::default();
    assert_eq!(run(&line, &mut user), 77);
    // Not in `wol help`: the slot build's own step.
    let mut help = FakeEnv::default();
    run("help", &mut help);
    assert!(!help.out.contains("update chroot"), "{}", help.out);
}

/// QA-STORE-9/68: `wol update channel` says the channel the system follows: stable, also when a
/// file from before names a testing channel; --json lists the offered ones.
#[test]
fn update_channel_reports_stable() {
    for file in [None, Some("beta\n"), Some("stable\n")] {
        let mut env = FakeEnv::default();
        if let Some(t) = file {
            env.files
                .push((PathBuf::from("/etc/wolos/channel"), t.into()));
        }
        assert_eq!(run("update channel", &mut env), 0);
        assert_eq!(env.out, "stable\n", "{file:?}");
    }
    let mut env = FakeEnv::default();
    assert_eq!(run("update channel --json", &mut env), 0);
    let v: serde_json::Value = serde_json::from_str(&env.out).unwrap();
    assert_eq!(v["channel"], "stable");
    assert_eq!(v["offered"], serde_json::json!(["stable"]));
}

/// QA-PLUG-23: `wol open` starts the app and returns; the app has no pipe of the caller's (a
/// guest agent or `su -l bob -c` waits for every holder of its stdout), and a launcher that stays
/// up as the app does not hold `wol open` either.
#[test]
fn open_returns_and_shares_no_pipe_with_the_app() {
    use controller::run::{Env, RealEnv};
    let dir = temp_home("detached");
    let out = dir.join("fds");
    let pid = dir.join("pid");
    let script = format!(
        "echo $$ > {}; a=$(readlink /proc/$$/fd/0 /proc/$$/fd/1 /proc/$$/fd/2); echo \"$a\" > {}; exec sleep 60",
        pid.display(),
        out.display()
    );
    let start = std::time::Instant::now();
    let code = RealEnv
        .exec_detached(&["sh".into(), "-c".into(), script])
        .unwrap();
    assert_eq!(code, 0);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(15),
        "waited for the app"
    );
    // The launcher is still running (it was waited for 5 s) and had written its descriptors long
    // before: none is ours.
    let seen = std::fs::read_to_string(&out).unwrap_or_default();
    assert_eq!(seen, "/dev/null\n/dev/null\n/dev/null\n", "{seen}");
    if let Ok(p) = std::fs::read_to_string(&pid) {
        let _ = std::process::Command::new("kill").arg(p.trim()).status();
    }
    // A launcher that exits answers with its own status.
    assert_eq!(
        RealEnv
            .exec_detached(&["sh".into(), "-c".into(), "exit 3".into()])
            .unwrap(),
        3
    );
}
