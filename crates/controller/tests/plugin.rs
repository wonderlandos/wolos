// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol plugin`: new -> check -> run -> enable -> pack -> remove round trip in a temp home, and the
//! reference plugin `plugins/shell/org.wolos.stats` checks clean from its checkout folder.
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use serde_json::Value;
use shared::errors::ErrorCode;
use std::path::{Path, PathBuf};

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    env.out.clear();
    env.err.clear();
    execute(&parse(&argv).unwrap_or_else(|e| panic!("{line}: {e}")), env)
}

fn json(env: &FakeEnv) -> Value {
    serde_json::from_str(&env.out).unwrap_or_else(|e| panic!("not JSON: {e}\n{}", env.out))
}

/// A temp home with `XDG_DATA_HOME`, a runtime dir and one shell reply per call.
fn env(tag: &str) -> FakeEnv {
    let home = std::env::temp_dir().join(format!("wol-plugin-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    FakeEnv {
        home: home.clone(),
        vars: vec![
            ("HOME".into(), home.to_string_lossy().into()),
            ("XDG_RUNTIME_DIR".into(), "/run/user/1000".into()),
            (
                "XDG_DATA_HOME".into(),
                home.join("share").to_string_lossy().into(),
            ),
        ],
        replies: vec![String::new(); 8],
        ..Default::default()
    }
}

fn plugin_dir(env: &FakeEnv, id: &str) -> PathBuf {
    env.home.join("share/wolos/plugins").join(id)
}

fn stats_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/shell/org.wolos.stats")
}

#[test]
fn new_check_run_enable_pack_remove_round_trip() {
    let mut e = env("roundtrip");
    assert_eq!(
        run(
            "plugin new alice.demo menubar,widget,tile,hotkey,theme --json",
            &mut e
        ),
        0,
        "{}",
        e.err
    );
    let r = json(&e);
    assert_eq!(r["valid"], true, "{r}");
    let dir = plugin_dir(&e, "alice.demo");
    for f in ["plugin.toml", "bar.lua", "card.lua", "theme.json"] {
        assert!(dir.join(f).is_file(), "{f}");
    }
    assert_eq!(
        run("plugin new alice.demo", &mut e),
        ErrorCode::Invalid.exit(),
        "exists already"
    );
    assert!(e.err.contains("exists already"), "{}", e.err);

    assert_eq!(
        run("plugin check alice.demo --json", &mut e),
        0,
        "{}",
        e.out
    );
    let r = json(&e);
    assert_eq!(r["id"], "alice.demo");
    assert_eq!(r["manifest"]["permissions"], serde_json::json!(["toggles"]));
    let kinds: Vec<&str> = ["menubar", "widget", "tile", "toggle", "hotkey", "theme"].to_vec();
    for k in kinds {
        assert!(!r["manifest"][k].as_array().unwrap().is_empty(), "{k}");
    }
    e.tty = true;
    assert_eq!(run("plugin check alice.demo", &mut e), 0);
    assert!(e.out.starts_with("OK alice.demo 0.1"), "{}", e.out);
    assert!(e.out.contains("menubar bar, widget card"), "{}", e.out);
    assert!(e.out.contains("Switch its own toggles"), "{}", e.out);
    e.tty = false;

    // The folder path works too.
    assert_eq!(run(&format!("plugin check {}", dir.display()), &mut e), 0);

    // Headless render: the bar item, its popover, then a click on the card.
    assert_eq!(run("plugin run alice.demo menubar", &mut e), 0, "{}", e.out);
    let r = json(&e);
    assert_eq!(r["kind"], "menubar");
    assert_eq!(r["entry"], "bar");
    assert_eq!(r["node"]["type"], "row", "{r}");
    assert_eq!(r["popover"]["type"], "stack");
    assert_eq!(r["refresh"], 60);
    assert_eq!(
        run("plugin run alice.demo widget card", &mut e),
        0,
        "{}",
        e.out
    );
    let r = json(&e);
    assert_eq!(r["node"]["children"][0]["text"], "Count: 0");
    assert_eq!(
        run("plugin run alice.demo widget card 1", &mut e),
        0,
        "{}",
        e.out
    );
    let r = json(&e);
    assert_eq!(r["clicked"], 1);
    assert_eq!(r["node"]["children"][0]["text"], "Count: 1");
    // State went to the state home and a fresh run sees it.
    let state = e.home.join("state/wolos/plugins/alice.demo/state.json");
    assert_eq!(std::fs::read_to_string(&state).unwrap(), r#"{"count":1}"#);
    assert_eq!(run("plugin run alice.demo widget", &mut e), 0);
    assert_eq!(json(&e)["node"]["children"][0]["text"], "Count: 1");
    assert_eq!(
        run("plugin run alice.demo search", &mut e),
        ErrorCode::NotFound.exit(),
        "{}",
        e.err
    );
    assert!(
        e.err.contains("no [[search]] entry") && e.err.contains("menubar bar"),
        "{}",
        e.err
    );
    assert_eq!(
        run("plugin run alice.demo widget nope", &mut e),
        ErrorCode::NotFound.exit()
    );
    assert_eq!(
        run("plugin run nope menubar", &mut e),
        ErrorCode::NotFound.exit()
    );
    assert!(e.err.contains("wol plugin new nope"), "{}", e.err);

    // On, off, list.
    assert_eq!(run("plugin list --json", &mut e), 0);
    let r = json(&e);
    assert_eq!(r["plugins"][0]["id"], "alice.demo");
    assert_eq!(r["plugins"][0]["enabled"], false);
    assert_eq!(r["plugins"][0]["valid"], true);
    assert_eq!(
        run("plugin enable alice.demo --json", &mut e),
        0,
        "{}",
        e.err
    );
    let r = json(&e);
    assert_eq!(r["granted"], serde_json::json!(["toggles"]));
    assert_eq!(r["reloaded"], true);
    let enabled = e.home.join("wolos/plugins.json");
    assert!(
        std::fs::read_to_string(&enabled)
            .unwrap()
            .contains("\"toggles\"")
    );
    assert!(
        e.requests
            .iter()
            .any(|(s, req)| s == Path::new("/run/user/1000/wol-shell.sock") && req == b"reload\n")
    );
    assert_eq!(run("plugin list --json", &mut e), 0);
    assert_eq!(json(&e)["plugins"][0]["enabled"], true);
    e.tty = true;
    assert_eq!(run("plugin enable alice.demo", &mut e), 0);
    assert_eq!(
        e.out,
        "alice.demo is on. Granted: Switch its own toggles.\n"
    );
    assert_eq!(run("plugin disable alice.demo", &mut e), 0);
    assert_eq!(e.out, "alice.demo is off.\n");
    e.tty = false;
    assert_eq!(
        run("plugin enable nope", &mut e),
        ErrorCode::NotFound.exit()
    );

    // Pack: a tar next to the folder, then zstd.
    assert_eq!(run("plugin pack alice.demo --json", &mut e), 0, "{}", e.err);
    let r = json(&e);
    let file = PathBuf::from(r["file"].as_str().unwrap());
    assert_eq!(file, plugin_dir(&e, "alice.demo-0.1.wolplug"));
    assert_eq!(r["signed"], false);
    let tar = file.with_extension("tar");
    assert!(
        tar.is_file(),
        "the tar waits for zstd (the fake env runs nothing)"
    );
    let members = wol_core::package::members(&tar).unwrap();
    let names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
    assert!(
        names.contains(&"alice.demo/plugin.toml") && names.last() == Some(&"SHA256"),
        "{names:?}"
    );
    let zstd = e.execs.iter().find(|a| a[0] == "zstd").expect("zstd ran");
    assert_eq!(&zstd[1..6], ["-q", "-f", "-19", "--rm", "-o"]);
    assert_eq!(zstd[6], file.to_string_lossy());
    assert_eq!(zstd[7], tar.to_string_lossy());
    assert_eq!(
        r["sha256"].as_str().unwrap().lines().count(),
        names.len() - 1
    );

    // A broken script: check says which file and what, run fails the same way, pack refuses.
    std::fs::write(
        dir.join("bar.lua"),
        "return wol.widget { render = function() end }",
    )
    .unwrap();
    assert_eq!(
        run("plugin check alice.demo --json", &mut e),
        ErrorCode::Invalid.exit()
    );
    let r = json(&e);
    assert_eq!(r["valid"], false);
    assert_eq!(
        r["problems"][0],
        "bar.lua: The script must end with return wol.menubar { ... }."
    );
    assert_eq!(
        run("plugin pack alice.demo", &mut e),
        ErrorCode::Invalid.exit()
    );
    assert!(e.err.contains("fix the plugin first"), "{}", e.err);
    std::fs::write(
        dir.join("bar.lua"),
        "return wol.menubar { render = function() return wol.label { 1 .. nil } end }",
    )
    .unwrap();
    assert_eq!(
        run("plugin run alice.demo menubar", &mut e),
        ErrorCode::Invalid.exit()
    );
    let r = json(&e);
    assert!(r["error"].as_str().unwrap().contains("concatenate"), "{r}");
    std::fs::write(dir.join("plugin.toml"), "schema = 1\n").unwrap();
    assert_eq!(
        run("plugin check alice.demo --json", &mut e),
        ErrorCode::Invalid.exit()
    );
    assert_eq!(json(&e)["problems"][0], "plugin.toml needs id.");
    assert_eq!(run("plugin list --json", &mut e), 0);
    assert_eq!(json(&e)["plugins"][0]["valid"], false);

    // Remove: folder and state gone, nothing enabled.
    assert_eq!(run("plugin remove alice.demo", &mut e), 0, "{}", e.err);
    assert!(!dir.exists());
    assert!(!state.exists());
    assert_eq!(
        run("plugin remove alice.demo", &mut e),
        ErrorCode::NotFound.exit()
    );
    assert_eq!(run("plugin list --json", &mut e), 0);
    assert_eq!(json(&e)["plugins"].as_array().unwrap().len(), 0);
    let _ = std::fs::remove_dir_all(&e.home);
}

#[test]
fn new_refuses_bad_ids_and_kinds() {
    let mut e = env("bad");
    assert_eq!(run("plugin new Demo", &mut e), ErrorCode::Usage.exit());
    assert!(e.err.contains("alice.demo"), "{}", e.err);
    assert_eq!(
        run("plugin new alice.demo sidebar", &mut e),
        ErrorCode::Usage.exit()
    );
    assert!(e.err.contains("unknown kind 'sidebar'"), "{}", e.err);
    assert!(!plugin_dir(&e, "alice.demo").exists());
    let _ = std::fs::remove_dir_all(&e.home);
}

/// Every scaffold kind renders headless, so an agent's first `run` after `new` works.
#[test]
fn every_scaffold_entry_runs() {
    let mut e = env("kinds");
    assert_eq!(
        run(
            "plugin new alice.all menubar,widget,panel,search --json",
            &mut e
        ),
        0,
        "{}",
        e.out
    );
    for (kind, entry) in [("menubar", "bar"), ("widget", "card"), ("panel", "panel")] {
        assert_eq!(
            run(&format!("plugin run alice.all {kind} {entry}"), &mut e),
            0,
            "{kind}: {}",
            e.out
        );
        assert!(json(&e)["node"].is_object(), "{kind}");
    }
    assert_eq!(
        run("plugin run alice.all search search hello", &mut e),
        0,
        "{}",
        e.out
    );
    let r = json(&e);
    assert_eq!(r["hits"][0]["title"], "Echo: hello");
    assert_eq!(
        run("plugin run alice.all panel panel 1", &mut e),
        0,
        "{}",
        e.out
    );
    assert_eq!(json(&e)["clicked"], 1);
    let _ = std::fs::remove_dir_all(&e.home);
}

#[test]
fn reference_stats_plugin_checks_clean_from_its_checkout() {
    let mut e = env("stats");
    let dir = stats_dir();
    assert!(dir.join("plugin.toml").is_file(), "{}", dir.display());
    assert_eq!(
        run(&format!("plugin check {} --json", dir.display()), &mut e),
        0,
        "{}",
        e.out
    );
    let r = json(&e);
    assert_eq!(r["id"], "org.wolos.stats");
    assert_eq!(r["manifest"]["permissions"], serde_json::json!(["run"]));
    assert_eq!(
        r["files"],
        serde_json::json!(["bar.lua", "card.lua", "plugin.toml"])
    );
    // Headless, `wol sys stats` may or may not exist on this box: the bar item renders either way.
    assert_eq!(
        run(&format!("plugin run {} menubar", dir.display()), &mut e),
        0,
        "{}",
        e.out
    );
    let r = json(&e);
    assert!(
        r["node"]["type"] == "row" || r["node"]["type"] == "label",
        "{r}"
    );
    assert_eq!(r["popover"]["type"], "stack");
    assert!(
        r["events"][0]
            .as_str()
            .unwrap()
            .starts_with("run: wol sys stats --json"),
        "{r}"
    );
    assert_eq!(
        run(&format!("plugin run {} widget", dir.display()), &mut e),
        0,
        "{}",
        e.out
    );
    // Installed as a user plugin it would be refused: the prefix is WolOS's.
    let user = plugin_dir(&e, "org.wolos.stats");
    std::fs::create_dir_all(&user).unwrap();
    for f in ["plugin.toml", "bar.lua", "card.lua"] {
        std::fs::copy(dir.join(f), user.join(f)).unwrap();
    }
    assert_eq!(
        run("plugin check org.wolos.stats --json", &mut e),
        ErrorCode::Invalid.exit()
    );
    assert!(
        json(&e)["problems"][0]
            .as_str()
            .unwrap()
            .contains("reserved")
    );
    let _ = std::fs::remove_dir_all(&e.home);
}
