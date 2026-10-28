//! Helper-facing CLI verbs against a recording fake: settings journal + undo, Downloads tidy,
//! restore points (bectl fixtures), admin password flow, named scripts under the helper policy.
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use serde_json::Value;
use shared::admin::AdminReply;
use std::path::PathBuf;
use std::sync::Arc;

fn temp_home(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-helpers-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    env.out.clear();
    env.err.clear();
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

fn json_out(env: &FakeEnv) -> Value {
    serde_json::from_str(&env.out).unwrap_or_else(|e| panic!("{e}: {}", env.out))
}

#[test]
fn appearance_accent_and_undo_round_trip() {
    let mut e = FakeEnv {
        home: temp_home("appearance"),
        ..Default::default()
    };
    assert_eq!(run("accent set blue --json", &mut e), 0, "{}", e.err);
    assert_eq!(
        std::fs::read_to_string(e.home.join("wolos/theme")).unwrap(),
        "wol-auto-blue\n"
    );
    assert_eq!(run("appearance set dark --json", &mut e), 0, "{}", e.err);
    let v = json_out(&e);
    assert_eq!(
        (
            v["mode"].as_str(),
            v["accent"].as_str(),
            v["theme"].as_str()
        ),
        (Some("dark"), Some("blue"), Some("wol-dark-blue"))
    );
    assert_eq!(run("history --json", &mut e), 0);
    let h = json_out(&e);
    assert_eq!(h.as_array().unwrap().len(), 2);
    assert_eq!(h[0]["by"], "cli");
    assert_eq!(run("undo", &mut e), 0, "{}", e.err);
    assert_eq!(
        std::fs::read_to_string(e.home.join("wolos/theme")).unwrap(),
        "wol-auto-blue\n"
    );
    assert_eq!(json_out(&e)["undid"], 2, "piped output is JSON");
    // Third-party apps follow the undo (gsettings through exec).
    assert!(
        e.execs
            .iter()
            .any(|a| a.first().is_some_and(|x| x == "gsettings")),
        "{:?}",
        e.execs
    );
    e.tty = true;
    assert_eq!(run("undo 3", &mut e), 0, "redo");
    assert_eq!(e.out, "undid #3 (theme); redo: wol undo 4\n");
    assert_eq!(
        std::fs::read_to_string(e.home.join("wolos/theme")).unwrap(),
        "wol-dark-blue\n"
    );
    assert_eq!(run("undo x", &mut e), 64);
}

#[test]
fn theme_set_is_journaled_too() {
    let mut e = FakeEnv {
        home: temp_home("themeset"),
        ..Default::default()
    };
    assert_eq!(run("theme set nord", &mut e), 0, "{}", e.err);
    assert_eq!(run("theme dark", &mut e), 0, "{}", e.err);
    assert_eq!(run("undo", &mut e), 0, "{}", e.err);
    assert_eq!(
        std::fs::read_to_string(e.home.join("wolos/theme")).unwrap(),
        "nord\n"
    );
}

#[test]
fn focus_and_wallpaper() {
    let mut e = FakeEnv {
        home: temp_home("focus"),
        vars: vec![("XDG_RUNTIME_DIR".into(), "/run/u".into())],
        replies: vec![String::new()],
        ..Default::default()
    };
    assert_eq!(run("focus on --json", &mut e), 0);
    assert_eq!(json_out(&e)["do_not_disturb"], true);
    assert!(e.home.join("wol/dnd").exists());
    let img = e.home.join("Pictures/sea.png");
    std::fs::create_dir_all(img.parent().unwrap()).unwrap();
    std::fs::write(&img, "png").unwrap();
    assert_eq!(
        run("wallpaper set ~/Pictures/sea.png --json", &mut e),
        0,
        "{}",
        e.err
    );
    assert_eq!(json_out(&e)["path"], img.display().to_string());
    assert_eq!(
        e.requests.last().unwrap().1,
        b"reload\n",
        "shell rereads the wallpaper"
    );
    assert_eq!(run("wallpaper set /etc/passwd", &mut e), 65);
    assert_eq!(run("undo", &mut e), 0);
    assert_eq!(run("undo", &mut e), 0);
    assert!(!e.home.join("wol/dnd").exists());
}

#[test]
fn downloads_tidy_dry_run_then_plan_then_undo() {
    let mut e = FakeEnv {
        home: temp_home("tidy"),
        ..Default::default()
    };
    let dl = e.home.join("Downloads");
    std::fs::create_dir_all(&dl).unwrap();
    for f in ["a.png", "b.pdf", "c.part", "notes"] {
        std::fs::write(dl.join(f), f).unwrap();
    }
    assert_eq!(run("files tidy --json", &mut e), 0, "{}", e.err);
    let plan = json_out(&e);
    assert_eq!(plan["dry_run"], true);
    assert_eq!(plan["moves"].as_array().unwrap().len(), 2);
    assert!(dl.join("a.png").exists(), "dry run moves nothing");
    assert_eq!(
        run("files tidy 0000000000000000", &mut e),
        65,
        "stale plan refused"
    );
    let id = plan["plan"].as_str().unwrap().to_string();
    assert_eq!(
        run(&format!("files tidy {id} --json"), &mut e),
        0,
        "{}",
        e.err
    );
    assert_eq!(json_out(&e)["moved"], 2);
    assert!(dl.join("Images/a.png").exists() && dl.join("Documents/b.pdf").exists());
    assert!(dl.join("c.part").exists() && dl.join("notes").exists());
    assert_eq!(run("undo", &mut e), 0, "{}", e.err);
    assert!(dl.join("a.png").exists() && dl.join("b.pdf").exists());
}

#[test]
fn downloads_outside_home_refused() {
    let mut e = FakeEnv {
        home: temp_home("tidyscope"),
        ..Default::default()
    };
    std::fs::write(e.home.join("user-dirs.dirs"), "XDG_DOWNLOAD_DIR=\"/tmp\"\n").unwrap();
    assert_eq!(run("files tidy", &mut e), 66, "{}", e.err);
    assert!(e.err.contains("inside your home"));
}

/// Running wolos-a; an update waits in wolos-b for its first boot.
const BOOTCTL: &str = r#"[
 {"id":"wolos-a.conf","version":"2026/10/20","isSelected":true,"isDefault":false},
 {"id":"wolos-b+3.conf","version":"2026/11/02","isSelected":false,"isDefault":true,"triesLeft":3}
]"#;

fn slots_env(tag: &str) -> FakeEnv {
    FakeEnv {
        home: temp_home(tag),
        captures: vec![("bootctl list --json=short".into(), BOOTCTL.into())],
        ..Default::default()
    }
}

#[test]
fn restore_list_marks_the_rollback_target() {
    let mut e = slots_env("restorelist");
    assert_eq!(run("restore list --json", &mut e), 0, "{}", e.err);
    let v = json_out(&e);
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert_eq!(
        v[0]["rollback"], true,
        "pending update: rollback keeps the running slot"
    );
    let mut none = FakeEnv {
        home: temp_home("nobe"),
        ..Default::default()
    };
    assert_eq!(run("restore list", &mut none), 69);
    assert!(none.err.contains("root slots"));
}

#[test]
fn rollback_goes_through_wol_admind_with_the_password() {
    let mut e = slots_env("rollback");
    e.replies = vec!["auth\n".into(), "ok\n".into()];
    e.stdin = vec!["s3cret".into()];
    assert_eq!(run("update rollback", &mut e), 0, "{}", e.err);
    let sent: Vec<String> = e
        .requests
        .iter()
        .map(|r| String::from_utf8_lossy(&r.1).into_owned())
        .collect();
    assert_eq!(sent[0], "boot-environment\x001\x00wolos-a\x00");
    assert_eq!(sent[1], "boot-environment\x001\x00wolos-a\x00s3cret");
    assert!(e.execs.is_empty(), "not root: never runs bootctl itself");
    let mut e = FakeEnv {
        home: temp_home("activate"),
        captures: vec![(
            "bootctl list --json=short".into(),
            r#"[{"id":"wolos-a.conf","isSelected":true}]"#.into(),
        )],
        ..Default::default()
    };
    assert_eq!(run("restore activate wolos-b", &mut e), 66);
    assert_eq!(run("restore activate a;b", &mut e), 64);
    assert!(e.requests.is_empty());
}

#[test]
fn first_update_has_no_previous_slot_to_replace() {
    let mut e = FakeEnv {
        home: temp_home("firstupdate"),
        root: true,
        files: vec![(
            std::path::PathBuf::from("/proc/cmdline"),
            "rootflags=subvol=@wolos-a\n".into(),
        )],
        captures: vec![(
            "bootctl list --json=short".into(),
            r#"[{"id":"wolos-a.conf","isSelected":true}]"#.into(),
        )],
        ..Default::default()
    };
    assert_eq!(run("update apply --yes", &mut e), 0, "{}", e.err);
    let ran: Vec<String> = e.execs.iter().map(|a| a.join(" ")).collect();
    assert_eq!(ran[0], "btrfs subvolume snapshot / /run/wolos/top/@wolos-b");
    assert!(!ran.iter().any(|r| r.contains("delete")), "{ran:?}");
    assert_eq!(ran.last().unwrap(), "bootctl set-default wolos-b.conf");
    assert!(e.out.contains("updated into wolos-b"));
}

/// NetworkManager lets the person at the computer switch the radio: no wol-admind, no password.
#[test]
fn wifi_off_needs_no_password() {
    let host = Arc::new(wol_core::system::fake::FakeHost::linux());
    host.command("nmcli radio wifi off", "")
        .command("nmcli radio wifi", "disabled\n");
    let mut e = FakeEnv {
        home: temp_home("wifi"),
        host: Some(host),
        ..Default::default()
    };
    assert_eq!(run("wifi off --json", &mut e), 0, "{}", e.err);
    assert_eq!(json_out(&e)["on"], false);
    assert!(e.admin.lock().unwrap().sent.is_empty());
    let mut e = FakeEnv {
        home: temp_home("wifi2"),
        host: Some(Arc::new(wol_core::system::fake::FakeHost::linux())),
        ..Default::default()
    };
    assert_ne!(run("wifi on", &mut e), 0, "no NetworkManager");
}

#[test]
fn volume_and_brightness_validate() {
    let host = Arc::new(wol_core::system::fake::FakeHost::new(false));
    host.command("wpctl set-volume @DEFAULT_AUDIO_SINK@ 40%", "")
        .command("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.40\n");
    let mut e = FakeEnv {
        home: temp_home("vol"),
        host: Some(host.clone()),
        ..Default::default()
    };
    assert_eq!(run("audio volume 40 --json", &mut e), 0, "{}", e.err);
    assert_eq!(json_out(&e)["volume"], 40);
    assert_eq!(run("audio volume 101", &mut e), 64);
    assert_eq!(run("display brightness 0", &mut e), 64);
    assert!(
        host.ran()
            .iter()
            .any(|r| r == "wpctl set-volume @DEFAULT_AUDIO_SINK@ 40%")
    );
}

/// A named script the user chose Always Allow for (the approval sheet saves this).
fn always(home: &std::path::Path, helper: &str, caps: &[&str]) {
    std::fs::create_dir_all(home.join("wolos")).unwrap();
    let p = serde_json::json!({"enabled": true, "deny": [], "always": {helper: caps}});
    std::fs::write(home.join("wolos/helpers.json"), p.to_string()).unwrap();
}

#[test]
fn open_notify_windows() {
    let mut e = FakeEnv {
        home: temp_home("open"),
        vars: vec![("WOL_HELPER".into(), "morning".into())],
        ..Default::default()
    };
    always(
        &e.home,
        "script:morning",
        &["apps", "notify", "window-list"],
    );
    let apps = e.home.join("share/applications");
    std::fs::create_dir_all(&apps).unwrap();
    std::fs::write(
        apps.join("org.gnome.Evolution.desktop"),
        "[Desktop Entry]\nName=Mail\nExec=evolution\n",
    )
    .unwrap();
    e.vars.push((
        "XDG_DATA_DIRS".into(),
        e.home.join("share").display().to_string(),
    ));
    assert_eq!(run("open Mail", &mut e), 0, "{}", e.err);
    assert_eq!(
        e.execs[0],
        [
            "gio",
            "launch",
            &apps
                .join("org.gnome.Evolution.desktop")
                .display()
                .to_string()
        ]
    );
    assert_eq!(run("open https://wolos.org", &mut e), 0);
    assert_eq!(e.execs[1], ["xdg-open", "https://wolos.org"]);
    assert_eq!(run("open javascript:alert(1)", &mut e), 69);
    assert_eq!(run("open Nonexistent", &mut e), 66);
    assert_eq!(run("notify Hello world", &mut e), 0);
    assert_eq!(
        e.execs[2],
        ["notify-send", "--app-name=morning", "--", "Hello", "world"]
    );
    e.captures = vec![
        (
            "hyprctl -j clients".into(),
            r#"[{"address":"0x1","class":"foot","title":"~","workspace":{"name":"1"}}]"#.into(),
        ),
        (
            "hyprctl dispatch focuswindow class:^(foot)$".into(),
            "ok\n".into(),
        ),
    ];
    assert_eq!(run("windows --json", &mut e), 0);
    assert_eq!(json_out(&e)[0]["app"], "foot");
    assert_eq!(run("windows focus foot", &mut e), 0, "{}", e.err);
    assert_eq!(run("windows focus foot;x", &mut e), 64);
    // Named script = helper: its changes are in the audit log.
    let log = std::fs::read_to_string(e.home.join("state/wolos/actions.jsonl")).unwrap();
    assert!(
        log.contains(r#""helper":"script:morning""#) && log.contains(r#""tool":"open""#),
        "{log}"
    );
}

#[test]
fn named_scripts_obey_the_off_switch() {
    let mut e = FakeEnv {
        home: temp_home("scriptoff"),
        vars: vec![("WOL_HELPER".into(), "cron".into())],
        ..Default::default()
    };
    // The user (no WOL_HELPER) turns helpers off.
    let user = FakeEnv {
        home: e.home.clone(),
        ..Default::default()
    };
    let mut user = user;
    assert_eq!(run("helpers off", &mut user), 0, "{}", user.err);
    assert_eq!(run("focus on", &mut e), 77);
    assert!(e.err.contains("turned off"));
    assert!(!e.home.join("wol/dnd").exists());
    assert_eq!(
        run("shell reload", &mut e),
        69,
        "non-helper verbs are not gated (no session here)"
    );
    assert_eq!(run("helpers --json", &mut user), 0);
    let s = json_out(&user);
    assert_eq!(s["enabled"], false);
    assert_eq!(s["helpers"][0]["name"], "script:cron");
    assert_eq!(s["helpers"][0]["refused"], 1);
}

/// Scripts ask on screen like MCP clients: deny changes nothing, allow runs it, no session refuses.
#[test]
fn scripts_ask_on_screen_like_mcp_clients() {
    use shared::shell::{Answer, Approval, ApprovalRequest};
    let mut e = FakeEnv {
        home: temp_home("scriptask"),
        vars: vec![
            ("WOL_HELPER".into(), "cron".into()),
            ("XDG_RUNTIME_DIR".into(), "/run/u".into()),
        ],
        ..Default::default()
    };
    e.replies = vec![Answer::Deny.line()];
    assert_eq!(run("focus on", &mut e), 77);
    assert!(e.err.contains("declined"), "{}", e.err);
    assert!(!e.home.join("wol/dnd").exists());
    let line = String::from_utf8(e.requests[0].1.clone()).unwrap();
    let a = Approval::parse(line.strip_prefix("approve ").unwrap()).unwrap();
    assert_eq!(
        (a.helper.as_str(), a.request),
        (
            "script:cron",
            ApprovalRequest::Change {
                capability: "focus".into(),
                what: "turn Do Not Disturb on".into()
            }
        )
    );
    e.replies = vec![Answer::AllowOnce.line()];
    assert_eq!(run("focus on", &mut e), 0, "{}", e.err);
    assert_eq!(run("focus --json", &mut e), 0);
    assert_eq!(json_out(&e)["do_not_disturb"], true);
    let log = std::fs::read_to_string(e.home.join("state/wolos/actions.jsonl")).unwrap();
    assert!(
        log.contains(r#""result":"declined""#) && log.contains(r#""result":"approved""#),
        "{log}"
    );
    e.vars.retain(|v| v.0 != "XDG_RUNTIME_DIR");
    assert_eq!(run("focus off", &mut e), 69, "no session: unavailable");
    assert!(e.err.contains("No WolOS session"));
}

/// A script's administrator verb goes to the password sheet, never to a cached authorization.
#[test]
fn script_admin_verbs_run_only_in_the_shell() {
    use shared::shell::Answer;
    let mut e = FakeEnv {
        home: temp_home("scriptadmin"),
        vars: vec![
            ("WOL_HELPER".into(), "cron".into()),
            ("XDG_RUNTIME_DIR".into(), "/run/u".into()),
        ],
        ..Default::default()
    };
    e.admin.lock().unwrap().replies = vec![AdminReply::Ok];
    e.replies = vec![Answer::Done.line()];
    assert_eq!(run("wifi off", &mut e), 0, "{}", e.err);
    assert!(
        e.admin.lock().unwrap().sent.is_empty(),
        "the script never talks to wol-admind"
    );
    assert!(String::from_utf8_lossy(&e.requests[0].1).contains(r#""action":"wifi","on":false"#));
}

#[test]
fn helpers_forget_takes_back_always_allow() {
    let mut e = FakeEnv {
        home: temp_home("forget"),
        ..Default::default()
    };
    always(&e.home, "script:morning", &["apps"]);
    assert_eq!(run("helpers --json", &mut e), 0);
    assert_eq!(json_out(&e)["always"][0]["helper"], "script:morning");
    assert_eq!(
        run("helpers forget script:morning --json", &mut e),
        0,
        "{}",
        e.err
    );
    assert_eq!(json_out(&e)["always"], serde_json::json!([]));
}

#[test]
fn api_json_is_the_catalogue() {
    let mut e = FakeEnv::default();
    assert_eq!(run("api --json", &mut e), 0);
    let v = json_out(&e);
    assert_eq!(v["exit_codes"]["64"], "usage");
    let verbs = v["verbs"].as_array().unwrap();
    let off = verbs.iter().find(|x| x["tool"] == "wifi_off").unwrap();
    assert_eq!(
        (off["admin"].as_bool(), off["capability"].as_str()),
        (Some(true), Some("network"))
    );
    assert!(
        verbs
            .iter()
            .any(|x| x["command"] == "wol mcp" && x["helpers"] == false)
    );
}
