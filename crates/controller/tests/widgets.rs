//! `wol widgets`: list, add, remove, reload, validate against the recording fake and a temp home.
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use shared::widgets::{Builtin, Desktop, Kind, Size};
use std::path::{Path, PathBuf};

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

/// A temp home with `XDG_DATA_HOME` and a session; `files`: (path under home, text).
fn env(tag: &str, files: &[(&str, &str)]) -> FakeEnv {
    let home = std::env::temp_dir().join(format!("wol-widgets-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    for (p, text) in files {
        let p = home.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    FakeEnv {
        home: home.clone(),
        vars: vec![
            ("XDG_RUNTIME_DIR".into(), "/run/user/1000".into()),
            (
                "XDG_DATA_HOME".into(),
                home.join("share").to_string_lossy().into(),
            ),
        ],
        replies: vec![String::new()],
        ..Default::default()
    }
}

fn layout_file(env: &FakeEnv) -> PathBuf {
    env.home.join("wolos/widgets.json")
}

/// The last `widgets.json` written.
fn written(env: &FakeEnv) -> Desktop {
    let (_, text) = env
        .written
        .iter()
        .rev()
        .find(|(p, _)| p == &layout_file(env))
        .expect("widgets.json written");
    Desktop::parse(text).unwrap()
}

fn reloaded(env: &FakeEnv) -> bool {
    env.requests
        .iter()
        .any(|(sock, req)| sock == Path::new("/run/user/1000/wol-shell.sock") && req == b"reload\n")
}

const FRIENDS: &str = r#"{
  "name": "Game Friends",
  "sizes": ["small", "medium"],
  "source": {"command": ["./friends.sh"]},
  "layouts": {"small": {"type": "label", "text": "{{online.length}} online"},
              "medium": {"type": "label", "text": "{{total}} friends"}},
  "sample": {"online": [], "total": 3}
}"#;

const PLACED: &str = r#"{"version": 1, "widgets": [
  {"id": "clock", "kind": "clock", "size": "small", "col": 0, "row": 0},
  {"id": "weather", "kind": "weather", "size": "medium", "col": 1, "row": 0}
]}"#;

#[test]
fn list_shows_the_desktop_and_what_can_be_added() {
    let mut e = env(
        "list",
        &[
            ("wolos/widgets.json", PLACED),
            ("share/wolos/widgets/game-friends/widget.json", FRIENDS),
            ("share/wolos/widgets/broken/widget.json", "{"),
        ],
    );
    e.tty = true;
    assert_eq!(run("widgets", &mut e), 0, "{}", e.err);
    for want in [
        "clock",
        "weather",
        "medium",
        "1,0",
        "Clock",
        "Batteries",
        "Game Friends",
        "broken",
        "wol widgets validate broken",
    ] {
        assert!(e.out.contains(want), "{want} in\n{}", e.out);
    }
    let mut e = env(
        "list-json",
        &[
            ("wolos/widgets.json", PLACED),
            ("share/wolos/widgets/game-friends/widget.json", FRIENDS),
        ],
    );
    assert_eq!(run("widgets list --json", &mut e), 0, "{}", e.err);
    let v: serde_json::Value = serde_json::from_str(&e.out).unwrap();
    assert_eq!(v["placed"][1]["id"], "weather");
    let kinds: Vec<&str> = v["available"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"clock") && kinds.contains(&"custom:game-friends"));
    let friends = v["available"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["kind"] == "custom:game-friends")
        .unwrap();
    assert_eq!(friends["valid"], true);
    assert_eq!(friends["sizes"], serde_json::json!(["small", "medium"]));
}

#[test]
fn add_places_in_the_nearest_free_cell_and_reloads_the_shell() {
    let mut e = env(
        "add",
        &[
            ("wolos/widgets.json", PLACED),
            ("share/wolos/widgets/game-friends/widget.json", FRIENDS),
        ],
    );
    assert_eq!(run("widgets add clock", &mut e), 0, "{}", e.err);
    let d = written(&e);
    let w = d.get("clock-2").expect("second clock");
    assert_eq!(
        (w.kind.clone(), w.size),
        (Kind::Builtin(Builtin::Clock), Size::Small)
    );
    assert_eq!((w.col, w.row), (0, 1), "next free cell to the top-left");
    assert!(reloaded(&e));

    // A package, by id or kind; sizes it does not offer are refused.
    let mut e = env(
        "add-pkg",
        &[("share/wolos/widgets/game-friends/widget.json", FRIENDS)],
    );
    assert_eq!(
        run("widgets add game-friends medium", &mut e),
        0,
        "{}",
        e.err
    );
    let w = written(&e).widgets[0].clone();
    assert_eq!(
        (w.id.as_str(), w.kind, w.size),
        (
            "game-friends",
            Kind::Custom("game-friends".into()),
            Size::Medium
        )
    );
    let mut e = env(
        "add-bad",
        &[("share/wolos/widgets/game-friends/widget.json", FRIENDS)],
    );
    assert_ne!(run("widgets add custom:game-friends large", &mut e), 0);
    assert!(e.err.contains("small, medium"), "{}", e.err);
    assert_ne!(run("widgets add notes", &mut e), 0);
    assert!(e.err.contains("no widget"), "{}", e.err);
    assert_ne!(run("widgets add clock huge", &mut e), 0);
    assert!(e.written.is_empty());
}

#[test]
fn add_works_without_a_running_shell_and_refuses_to_overwrite_a_broken_file() {
    let mut e = env("offline", &[]);
    e.replies.clear();
    assert_eq!(run("widgets add battery large", &mut e), 0, "{}", e.err);
    assert_eq!(written(&e).widgets[0].size, Size::Large);
    assert!(e.out.contains("next time the desktop starts"), "{}", e.out);

    let mut e = env("broken", &[("wolos/widgets.json", "{ nope")]);
    assert_ne!(run("widgets add clock", &mut e), 0);
    assert!(e.err.contains("widgets.json"), "{}", e.err);
    assert!(
        e.written.is_empty(),
        "a file it cannot read is never replaced"
    );
}

#[test]
fn remove_takes_one_instance_by_id() {
    let mut e = env("remove", &[("wolos/widgets.json", PLACED)]);
    assert_eq!(run("widgets remove weather", &mut e), 0, "{}", e.err);
    let ids: Vec<String> = written(&e).widgets.into_iter().map(|w| w.id).collect();
    assert_eq!(ids, ["clock"]);
    assert!(reloaded(&e));
    let mut e = env("remove-none", &[("wolos/widgets.json", PLACED)]);
    assert_ne!(run("widgets remove music", &mut e), 0);
    assert!(e.err.contains("clock, weather"), "{}", e.err);
}

#[test]
fn reload_asks_the_shell() {
    let mut e = env("reload", &[]);
    assert_eq!(run("widgets reload", &mut e), 0, "{}", e.err);
    assert!(reloaded(&e));
}

#[test]
fn validate_names_every_problem_with_its_path() {
    let broken = r#"{"name": "Stocks", "sizes": ["small", "huge"],
        "layouts": {"small": {"type": "stak", "children": []}}}"#;
    let mut e = env(
        "validate",
        &[("share/wolos/widgets/stocks/widget.json", broken)],
    );
    assert_ne!(run("widgets validate stocks", &mut e), 0);
    for want in ["sizes[1]", "huge", "stak", "2 problems"] {
        assert!(e.out.contains(want), "{want} in\n{}", e.out);
    }
}

#[test]
fn validate_runs_the_source_and_checks_every_binding() {
    let script = "#!/bin/sh\necho '{\"online\": [{\"name\": \"Alice\"}]}'\n";
    let mut e = env(
        "validate-ok",
        &[
            ("share/wolos/widgets/game-friends/widget.json", FRIENDS),
            ("share/wolos/widgets/game-friends/friends.sh", script),
        ],
    );
    let sh = e.home.join("share/wolos/widgets/game-friends/friends.sh");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755)).unwrap();
    // The script's output has no `total`: the medium layout would show an empty label.
    assert_ne!(run("widgets validate game-friends", &mut e), 0);
    assert!(e.out.contains("{{total}}"), "{}", e.out);
    assert!(e.out.contains("source"), "{}", e.out);

    // By folder path, and with output that binds: valid.
    let fixed = "#!/bin/sh\necho '{\"online\": [], \"total\": 2}'\n";
    std::fs::write(&sh, fixed).unwrap();
    let dir = sh.parent().unwrap().to_string_lossy().to_string();
    let mut e2 = FakeEnv {
        home: e.home.clone(),
        vars: e.vars.clone(),
        ..Default::default()
    };
    assert_eq!(
        run(&format!("widgets validate {dir}"), &mut e2),
        0,
        "{}{}",
        e2.out,
        e2.err
    );
    assert!(e2.out.contains("Game Friends is valid"), "{}", e2.out);
}
