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
    // "nope" is no id (`new` would refuse it), so it is not told to start one (QA-PLUG-72).
    assert!(e.err.contains("not a plugin folder"), "{}", e.err);

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

    // Pack: a tar in the working folder, then zstd. QA-PLUG-11: never into the hidden plugin
    // folder (where it sat beside the plugins): the working folder, else the home folder.
    let work = e.home.join("work");
    std::fs::create_dir_all(&work).unwrap();
    e.vars.push(("PWD".into(), work.to_string_lossy().into()));
    assert_eq!(run("plugin pack alice.demo --json", &mut e), 0, "{}", e.err);
    let r = json(&e);
    let file = PathBuf::from(r["file"].as_str().unwrap());
    assert_eq!(file, work.join("alice.demo-0.1.wolplug"));
    assert!(!plugin_dir(&e, "alice.demo-0.1.wolplug").exists());
    assert_eq!(r["signed"], false);
    let zstd = e.execs.iter().find(|a| a[0] == "zstd").expect("zstd ran");
    let tar = PathBuf::from(&zstd[7]);
    // Audit S4-23: a unique name next to the target, never a neighbour's.
    let tar_name = tar.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        tar_name.starts_with("alice.demo-0.1.wolplug.") && tar_name.ends_with(".tar"),
        "{tar_name}"
    );
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
    assert_eq!(&zstd[1..6], ["-q", "-f", "-19", "--rm", "-o"]);
    assert_eq!(zstd[6], file.to_string_lossy());
    assert_eq!(zstd[7], tar.to_string_lossy());
    assert_eq!(
        r["sha256"].as_str().unwrap().lines().count(),
        names.len() - 1
    );
    // Audit S4-23: an existing target is replaced only with --yes; a neighbour that the old
    // `with_extension("tar")` name hit is never touched.
    let neighbour = file.with_extension("tar");
    std::fs::write(&neighbour, "mine").unwrap();
    std::fs::write(&file, "old pack").unwrap();
    assert_eq!(
        run("plugin pack alice.demo", &mut e),
        ErrorCode::Usage.exit()
    );
    assert!(e.err.contains("--yes"), "{}", e.err);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "old pack");
    assert_eq!(run("plugin pack alice.demo --yes", &mut e), 0, "{}", e.err);
    assert_eq!(std::fs::read_to_string(&neighbour).unwrap(), "mine");
    let _ = std::fs::remove_file(&neighbour);
    let _ = std::fs::remove_file(&file);

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
    // QA-PLUG-37: no TTY, so the answer is JSON.
    assert_eq!(json(&e)["id"], "alice.demo", "{}", e.out);
    assert!(
        json(&e)["removed"]
            .as_str()
            .unwrap()
            .ends_with("alice.demo")
    );
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
        run("plugin new alice.all menubar,widget --json", &mut e),
        0,
        "{}",
        e.out
    );
    // `[[panel]]` and `[[search]]` are planned (QA-PLUG-4): not scaffolded (the Lua side of
    // both stays covered by wol-lua's tests).
    for (kind, entry) in [("menubar", "bar"), ("widget", "card")] {
        assert_eq!(
            run(&format!("plugin run alice.all {kind} {entry}"), &mut e),
            0,
            "{kind}: {}",
            e.out
        );
        assert!(json(&e)["node"].is_object(), "{kind}");
    }
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

/// Audit C8-2: `check` (and `new`, `pack`) run the plugin's Lua with inert services: a
/// `wol.run` in render runs nothing, and no state or toggle flag is written, before any grant.
#[test]
fn check_runs_nothing_and_writes_nothing() {
    let mut e = env("inert");
    assert_eq!(
        run("plugin new alice.inert menubar", &mut e),
        0,
        "{}",
        e.out
    );
    let dir = plugin_dir(&e, "alice.inert");
    let mark = e.home.join("ran");
    let toml = std::fs::read_to_string(dir.join("plugin.toml")).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        toml.replace("min_wolos", "permissions = [\"run\"]\nmin_wolos"),
    )
    .unwrap();
    std::fs::write(
        dir.join("bar.lua"),
        format!(
            "return wol.menubar {{ render = function() wol.run {{ \"touch\", \"{}\" }}; wol.state.set(\"n\", 1); return wol.label {{ \"x\" }} end }}\n",
            mark.display()
        ),
    )
    .unwrap();
    assert_eq!(
        run("plugin check alice.inert --json", &mut e),
        0,
        "{}",
        e.out
    );
    assert!(!mark.exists(), "check ran the plugin's command");
    assert!(!e.home.join("state").exists(), "check wrote state");
    assert_eq!(
        run("plugin pack alice.inert --json", &mut e),
        0,
        "{}",
        e.err
    );
    assert!(!mark.exists(), "pack ran the plugin's command");
    // `run` is the explicit way to run it, as the person.
    assert_eq!(
        run("plugin run alice.inert menubar", &mut e),
        0,
        "{}",
        e.out
    );
    assert!(
        json(&e)["events"][0]
            .as_str()
            .unwrap()
            .starts_with("run: touch")
    );
    assert!(mark.exists());
    let _ = std::fs::remove_dir_all(&e.home);
}

/// Audit X6-1: planned kinds are refused by `new` with the sentence `check` gives.
#[test]
fn new_refuses_planned_kinds() {
    let mut e = env("planned");
    for k in ["look", "hypr", "panel", "search"] {
        assert_eq!(
            run(&format!("plugin new alice.p {k}"), &mut e),
            ErrorCode::Usage.exit()
        );
        assert!(e.err.contains("is planned"), "{}", e.err);
    }
    assert!(!plugin_dir(&e, "alice.p").exists());
    let _ = std::fs::remove_dir_all(&e.home);
}

/// Audit X6-2 and X6-1: `wol plugin add` installs a folder or a `.wolplug` into this account's
/// plugin folder (unsigned only with --yes, never turned on); an enabled `[[webapp]]` is a
/// launcher until the plugin is turned off or removed.
#[test]
fn add_installs_and_webapps_follow_enable() {
    let mut e = env("add");
    // The source folder lives outside the plugin folder (a checkout).
    let src_root = e.home.join("src");
    let src = src_root.join("alice.web");
    std::fs::create_dir_all(&src_root).unwrap();
    let mut maker = env("add-maker");
    assert_eq!(
        run("plugin new alice.web menubar,webapp", &mut maker),
        0,
        "{}",
        maker.out
    );
    let made = plugin_dir(&maker, "alice.web");
    std::fs::create_dir_all(&src).unwrap();
    for f in ["plugin.toml", "bar.lua"] {
        std::fs::copy(made.join(f), src.join(f)).unwrap();
    }

    let line = format!("plugin add {}", src.display());
    assert_eq!(run(&line, &mut e), ErrorCode::Usage.exit());
    assert!(
        e.err.contains("not signed") && e.err.contains("--yes"),
        "{}",
        e.err
    );
    assert!(!plugin_dir(&e, "alice.web").exists());
    assert_eq!(run(&format!("{line} --yes --json"), &mut e), 0, "{}", e.err);
    let r = json(&e);
    assert_eq!(
        (r["id"].as_str(), r["enabled"].as_bool()),
        (Some("alice.web"), Some(false))
    );
    assert!(plugin_dir(&e, "alice.web").join("bar.lua").is_file());
    assert_eq!(
        run("plugin add alice.nothing", &mut e),
        ErrorCode::Unavailable.exit()
    );
    assert!(e.err.contains("App Store"), "{}", e.err);

    let launcher = e
        .home
        .join("share/applications/wolos-webapp-plugin-alice-web-web.desktop");
    assert!(!launcher.exists());
    assert_eq!(run("plugin enable alice.web", &mut e), 0, "{}", e.err);
    let text = std::fs::read_to_string(&launcher).unwrap();
    assert!(text.contains("X-WolOS-Plugin=alice.web"), "{text}");
    assert_eq!(run("plugin disable alice.web", &mut e), 0);
    assert!(!launcher.exists());
    assert_eq!(run("plugin enable alice.web", &mut e), 0);
    assert_eq!(run("plugin remove alice.web", &mut e), 0, "{}", e.err);
    assert!(!launcher.exists(), "remove takes its launchers too");

    // A .wolplug made by `pack` (the fake env leaves the tar; real zstd compresses it).
    assert_eq!(
        run("plugin pack alice.web --json", &mut maker),
        0,
        "{}",
        maker.err
    );
    let file = PathBuf::from(json(&maker)["file"].as_str().unwrap());
    let zstd = maker.execs.iter().find(|a| a[0] == "zstd").unwrap().clone();
    let real = std::process::Command::new("zstd")
        .args(&zstd[1..])
        .status()
        .is_ok_and(|s| s.success());
    if real {
        let line = format!("plugin add {}", file.display());
        assert_eq!(run(&line, &mut e), ErrorCode::Usage.exit(), "unsigned");
        assert_eq!(run(&format!("{line} --yes"), &mut e), 0, "{}", e.err);
        assert!(plugin_dir(&e, "alice.web").join("plugin.toml").is_file());
        std::fs::write(&file, b"not zstd").unwrap();
        assert_eq!(
            run(&format!("{line} --yes"), &mut e),
            ErrorCode::Invalid.exit()
        );
        assert!(e.err.contains("damaged"), "{}", e.err);
    } else {
        eprintln!("no zstd here: the .wolplug half is skipped");
    }
    let _ = std::fs::remove_dir_all(&e.home);
    let _ = std::fs::remove_dir_all(&maker.home);
}

/// QA-PLUG-2: `wol plugin run` of a render that waits on `wol.run` gets the command's own answer
/// (here its timeout sentence after 3 s, more than the shell's 2 s service credit), not "took too long", and the run succeeds.
#[test]
fn run_waits_on_services_without_took_too_long() {
    let mut e = env("slowrun");
    assert_eq!(
        run("plugin new alice.slow menubar --json", &mut e),
        0,
        "{}",
        e.out
    );
    let dir = plugin_dir(&e, "alice.slow");
    let toml = std::fs::read_to_string(dir.join("plugin.toml")).unwrap();
    let toml = if toml.contains("permissions = []") {
        toml.replace("permissions = []", "permissions = [\"run\"]")
    } else {
        toml.replacen("[[menubar]]", "permissions = [\"run\"]\n\n[[menubar]]", 1)
    };
    std::fs::write(dir.join("plugin.toml"), toml).unwrap();
    std::fs::write(
        dir.join("bar.lua"),
        "return wol.menubar { render = function()\n  \
         local ok, out = wol.run { 'sleep', '6', timeout = 3 }\n  \
         local n = 0 for i = 1, 20000 do n = n + i end\n  \
         return wol.label { tostring(ok) .. ' ' .. tostring(out) }\nend }\n",
    )
    .unwrap();
    let code = run("plugin run alice.slow menubar bar", &mut e);
    assert_eq!(code, 0, "{}{}", e.out, e.err);
    let r = json(&e);
    let text = r["node"].to_string();
    assert!(text.contains("false"), "{text}");
    assert!(!text.contains("too long"), "{text}");
    let _ = std::fs::remove_dir_all(&e.home);
}

/// QA-PLUG-63: `wol plugin list` names why a plugin the shell turned off is off, and turning it
/// on again clears the reason.
#[test]
fn list_names_why_a_plugin_was_turned_off() {
    let mut e = env("turnedoff");
    assert_eq!(
        run("plugin new alice.crash menubar --json", &mut e),
        0,
        "{}",
        e.out
    );
    assert_eq!(
        run("plugin enable alice.crash --json", &mut e),
        0,
        "{}",
        e.err
    );
    let state = e.home.join("state");
    let why = "alice.crash was turned off after 3 failures in a row: boom";
    // What the shell does on the third failure: off, and the reason kept.
    assert_eq!(
        run("plugin disable alice.crash --json", &mut e),
        0,
        "{}",
        e.err
    );
    wol_core::plugins::note_turned_off(&state, "alice.crash", why).unwrap();
    assert_eq!(run("plugin list --json", &mut e), 0, "{}", e.err);
    let r = json(&e);
    let p = r["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "alice.crash")
        .unwrap()
        .clone();
    assert_eq!(
        (p["enabled"].clone(), p["problem"].clone()),
        (Value::Bool(false), Value::String(why.into())),
        "{p}"
    );
    assert_eq!(
        run("plugin enable alice.crash --json", &mut e),
        0,
        "{}",
        e.err
    );
    assert_eq!(wol_core::plugins::turned_off(&state, "alice.crash"), None);
    let _ = std::fs::remove_dir_all(&e.home);
}

/// QA-PLUG-70: `enable` runs the check `wol plugin check` runs: a plugin whose script fails is
/// not turned on (it used to be "on", then the shell failed it three times and turned it off).
#[test]
fn enable_runs_check() {
    let mut e = env("enablecheck");
    assert_eq!(
        run("plugin new alice.crash menubar --json", &mut e),
        0,
        "{}",
        e.out
    );
    let dir = plugin_dir(&e, "alice.crash");
    std::fs::write(
        dir.join("bar.lua"),
        "return wol.menubar { render = function() error('boom') end }\n",
    )
    .unwrap();
    assert_eq!(
        run("plugin enable alice.crash", &mut e),
        ErrorCode::Invalid.exit(),
        "{}",
        e.out
    );
    assert!(
        e.err.contains("boom") && e.err.contains("not turned on"),
        "{}",
        e.err
    );
    assert!(!wol_core::plugins::load_enabled(&e.home).is_enabled("alice.crash"));
    let _ = std::fs::remove_dir_all(&e.home);
}

/// QA-PLUG-67: `enable` says what it grants, `run` by name, on a terminal and in JSON.
#[test]
fn enable_lists_grants() {
    let mut e = env("grants");
    assert_eq!(
        run("plugin new alice.dk dock --json", &mut e),
        0,
        "{}",
        e.out
    );
    // The Dock scaffold runs `foot`: it asks for `run`.
    assert_eq!(run("plugin enable alice.dk --json", &mut e), 0, "{}", e.err);
    assert_eq!(json(&e)["granted"], serde_json::json!(["run"]));
    // On a terminal: the sentence.
    e.tty = true;
    assert_eq!(run("plugin enable alice.dk", &mut e), 0, "{}", e.err);
    assert!(e.out.contains("Granted:"), "{}", e.out);
    assert!(e.out.to_lowercase().contains("run"), "{}", e.out);
    let _ = std::fs::remove_dir_all(&e.home);
}

/// QA-PLUG-72: the not-found answer suggests `wol plugin new` only for an id; a path (MCP
/// `plugin_check {path: "/etc"}`) is told it is not a plugin folder.
#[test]
fn not_found_sentence_only_for_id_shaped() {
    let mut e = env("notfound");
    for what in ["/etc", "../../../etc", "NotAnId"] {
        assert_eq!(
            run(&format!("plugin check {what}"), &mut e),
            ErrorCode::NotFound.exit(),
            "{what}"
        );
        assert!(!e.err.contains("wol plugin new"), "{what}: {}", e.err);
    }
    assert_eq!(
        run("plugin check alice.nope", &mut e),
        ErrorCode::NotFound.exit()
    );
    assert!(e.err.contains("wol plugin new alice.nope"), "{}", e.err);
    let _ = std::fs::remove_dir_all(&e.home);
}

/// QA-PLUG-19: `wol plugin add ./x.wolplug` installs (zstd ran in / and could not find the
/// relative file, which read as "damaged"); a file that is not a package says so.
#[test]
fn add_reads_a_relative_file_and_names_junk() {
    let mut e = env("addrel");
    assert_eq!(
        run("plugin new alice.rel menubar --json", &mut e),
        0,
        "{}",
        e.out
    );
    let out = e.home.join("pkg");
    std::fs::create_dir_all(&out).unwrap();
    assert_eq!(
        run(
            &format!(
                "plugin pack alice.rel {}/alice.rel.wolplug --json",
                out.display()
            ),
            &mut e
        ),
        0,
        "{}{}",
        e.out,
        e.err
    );
    // The fake env only records pack's zstd step: compress the tar it wrote here.
    let tar = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .path();
    let file = out.join("alice.rel.wolplug");
    let st = std::process::Command::new("zstd")
        .args(["-q", "-f", "-o"])
        .arg(&file)
        .arg(&tar)
        .status()
        .unwrap();
    assert!(st.success());
    std::fs::remove_dir_all(plugin_dir(&e, "alice.rel")).unwrap();
    // The same file by a plain relative name in this test's working folder.
    let rel = format!("wol-addrel-{}.wolplug", std::process::id());
    std::fs::copy(&file, &rel).unwrap();
    let code = run(&format!("plugin add {rel} --yes --json"), &mut e);
    let _ = std::fs::remove_file(&rel);
    assert_eq!(code, 0, "{}{}", e.out, e.err);
    assert!(plugin_dir(&e, "alice.rel").join("plugin.toml").is_file());
    let junk = e.home.join("junk.wolplug");
    std::fs::write(&junk, vec![7u8; 4096]).unwrap();
    assert_ne!(
        run(&format!("plugin add {} --yes", junk.display()), &mut e),
        0
    );
    assert!(e.err.contains("not a plugin package"), "{}", e.err);
    let _ = std::fs::remove_dir_all(&e.home);
}

/// QA-PLUG-22: a plugin that asks for a newer WolOS than this one says so in `check` and `list`.
#[test]
fn check_warns_when_min_wolos_is_newer() {
    let mut e = env("minwolos");
    e.files.push((
        PathBuf::from("/usr/lib/wolos-release.json"),
        r#"{"version":"0.12.1","snapshot":"2026/11/02"}"#.into(),
    ));
    assert_eq!(
        run("plugin new alice.future menubar --json", &mut e),
        0,
        "{}",
        e.out
    );
    let dir = plugin_dir(&e, "alice.future");
    let toml = std::fs::read_to_string(dir.join("plugin.toml")).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        toml.replace("min_wolos = \"0.12\"", "min_wolos = \"9.0\""),
    )
    .unwrap();
    assert_eq!(
        run("plugin check alice.future --json", &mut e),
        0,
        "{}",
        e.out
    );
    assert_eq!(json(&e)["needs_wolos"], "9.0");
    e.tty = true;
    assert_eq!(run("plugin check alice.future", &mut e), 0, "{}", e.out);
    assert!(
        e.out
            .contains("Needs WolOS 9.0; this computer runs 0.12.1."),
        "{}",
        e.out
    );
    e.tty = false;
    assert_eq!(run("plugin list --json", &mut e), 0);
    let r = json(&e);
    let p = r["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "alice.future")
        .unwrap()
        .clone();
    assert_eq!(
        p["problem"], "Needs WolOS 9.0; this computer runs 0.12.1.",
        "{p}"
    );
    let _ = std::fs::remove_dir_all(&e.home);
}

/// QA-PLUG-10: the first `wol.notify` of a fresh `wol plugin run` is allowed (0.12.1 answered
/// "Too many notifications" on it), also next to an entry that loops forever.
#[test]
fn first_notify_of_a_fresh_run_is_allowed() {
    let mut e = env("notify");
    let dir = plugin_dir(&e, "bob.evil");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        "schema = 1\nid = \"bob.evil\"\nname = \"Evil\"\nversion = \"1\"\nlicense = \"MIT\"\n\
         permissions = [\"notifications\"]\n[[widget]]\nid = \"loop\"\nscript = \"loop.lua\"\n\
         [[widget]]\nid = \"n\"\nscript = \"n.lua\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("loop.lua"),
        "return wol.widget { render = function() while true do end end }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("n.lua"),
        "return wol.widget { render = function() local ok, m = wol.notify('a', 'b'); return wol.label { tostring(ok) .. tostring(m) } end }\n",
    )
    .unwrap();
    for _ in 0..3 {
        assert_eq!(
            run("plugin run bob.evil widget n --json", &mut e),
            0,
            "{}{}",
            e.out,
            e.err
        );
        let r = json(&e);
        assert!(r["node"].to_string().contains("truenil"), "{r}");
    }
    let _ = std::fs::remove_dir_all(&e.home);
}

/// QA-PLUG-36: the input of `plugin run` for a bar item or card is a part number; "abc" is
/// refused with a sentence instead of being dropped without a word. QA-PLUG-46: a bad id gets
/// the person's own account name as the example.
#[test]
fn run_input_must_be_a_number_and_a_bad_id_names_the_account() {
    let mut e = env("input46");
    assert_eq!(
        run("plugin new alice.demo menubar,widget", &mut e),
        0,
        "{}",
        e.err
    );
    let code = run("plugin run alice.demo widget card abc", &mut e);
    assert_eq!(code, ErrorCode::Usage.exit(), "{}{}", e.out, e.err);
    assert!(e.err.contains("\"abc\" is not a part number"), "{}", e.err);
    assert_eq!(
        run("plugin run alice.demo widget card 1", &mut e),
        0,
        "{}",
        e.err
    );
    // A search script's input is the query text, any text.
    // The account name is the example, lowercase, and the made-up tail follows the bad id.
    e.vars.push(("USER".into(), "Bob".into()));
    let code = run("plugin new BadId", &mut e);
    assert_eq!(code, ErrorCode::Usage.exit());
    assert!(e.err.contains("like \"bob.badid\""), "{}", e.err);
    assert!(!e.err.contains("alice"), "{}", e.err);
    e.vars.retain(|(k, _)| k != "USER");
    run("plugin new Bad_Id", &mut e);
    assert!(e.err.contains("like \"alice.bad-id\""), "{}", e.err);
}

/// QA-PLUG-40: `plugin list` says when plugins.json does not read (everything is off because of
/// it), in text on stderr and in the JSON.
#[test]
fn list_names_an_unreadable_plugins_json() {
    let mut e = env("badjson40");
    assert_eq!(run("plugin new alice.demo menubar", &mut e), 0);
    assert_eq!(run("plugin enable alice.demo", &mut e), 0, "{}", e.err);
    let file = e.home.join("wolos/plugins.json");
    std::fs::write(&file, "{ junk").unwrap();
    assert_eq!(run("plugin list --json", &mut e), 0);
    let j = json(&e);
    assert!(
        j["settings_problem"]
            .as_str()
            .unwrap()
            .contains("plugins.json"),
        "{j}"
    );
    assert_eq!(j["plugins"][0]["enabled"], false);
    // Turning a plugin on again keeps the broken file beside the new one.
    assert_eq!(run("plugin enable alice.demo", &mut e), 0, "{}", e.err);
    assert_eq!(
        std::fs::read_to_string(e.home.join("wolos/plugins.json.bad")).unwrap(),
        "{ junk"
    );
    assert_eq!(run("plugin list --json", &mut e), 0);
    assert!(json(&e)["settings_problem"].is_null());
}

/// QA-PLUG-12: the person allows a site for a turned-on plugin with `wol plugin allow-host`;
/// `plugin run` then reads it (the refusal named the command), and a helper cannot.
#[test]
fn allow_host_lets_a_turned_on_plugin_read_one_site() {
    let mut e = env("allowhost");
    assert_eq!(run("plugin new alice.web menubar", &mut e), 0, "{}", e.err);
    // Not on: nothing to allow.
    let code = run("plugin allow-host alice.web api.example.invalid", &mut e);
    assert_eq!(code, ErrorCode::NotFound.exit(), "{}", e.err);
    assert!(e.err.contains("wol plugin enable alice.web"), "{}", e.err);
    // On, but the manifest never asked for the web.
    assert_eq!(run("plugin enable alice.web", &mut e), 0, "{}", e.err);
    let code = run("plugin allow-host alice.web api.example.invalid", &mut e);
    assert_eq!(code, ErrorCode::Invalid.exit(), "{}", e.err);
    assert!(e.err.contains("web permission"), "{}", e.err);
    // A plugin that asks for http.
    let dir = plugin_dir(&e, "alice.web");
    let toml = std::fs::read_to_string(dir.join("plugin.toml")).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        toml.replacen("min_wolos", "permissions = [\"http\"]\nmin_wolos", 1),
    )
    .unwrap();
    std::fs::write(
        dir.join("bar.lua"),
        "return wol.menubar { render = function()\n  \
         local b, m = wol.http_get('https://api.example.invalid/x')\n  \
         return wol.label { tostring(b) .. ' / ' .. tostring(m) }\nend }\n",
    )
    .unwrap();
    assert_eq!(run("plugin enable alice.web", &mut e), 0, "{}", e.err);
    // Nothing allowed yet: the run answers with the command that allows it.
    assert_eq!(run("plugin run alice.web menubar", &mut e), 0, "{}", e.err);
    let text = json(&e)["node"]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        text.contains("wol plugin allow-host alice.web api.example.invalid"),
        "{text}"
    );
    assert_eq!(run("plugin allow-host alice.web --json", &mut e), 0);
    assert_eq!(json(&e)["hosts"], serde_json::json!([]));
    // Bad names are refused; a whole address is cut to its host.
    for bad in ["a/b", "-x.com", "x@y.com", "a_b.com"] {
        let code = run(&format!("plugin allow-host alice.web {bad}"), &mut e);
        assert_eq!(code, ErrorCode::Usage.exit(), "{bad}: {}", e.err);
    }
    let code = run("plugin allow-host alice.web a..b", &mut e);
    assert_eq!(code, ErrorCode::Usage.exit(), "{}", e.err);
    assert_eq!(
        run(
            "plugin allow-host alice.web https://API.Example.invalid/path --json",
            &mut e
        ),
        0,
        "{}",
        e.err
    );
    let j = json(&e);
    assert_eq!(
        j["hosts"],
        serde_json::json!(["api.example.invalid"]),
        "{j}"
    );
    assert_eq!(j["added"], "api.example.invalid");
    // The run now gets past the grant (the web itself is not reached from a test: the sentence
    // is no longer the grant's).
    assert_eq!(run("plugin run alice.web menubar", &mut e), 0, "{}", e.err);
    let text = json(&e)["node"]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!text.contains("allow-host"), "{text}");
    // It is a person's verb, not a helper's: under WOL_HELPER (a plugin's `wol.run`, a script, an
    // assistant's shell) it is refused and logged, and nothing is written.
    let before = std::fs::read_to_string(e.home.join("wolos/plugins.json")).unwrap();
    for helper in [("WOL_HELPER", "alice.web"), ("CLAUDECODE", "1")] {
        let mut h = env("allowhost-helper");
        h.home = e.home.clone();
        h.vars = e.vars.clone();
        h.vars.push((helper.0.into(), helper.1.into()));
        let code = run("plugin allow-host alice.web evil.example.invalid", &mut h);
        assert_eq!(code, ErrorCode::Denied.exit(), "{helper:?}: {}", h.err);
        assert!(h.err.contains("is for people"), "{}", h.err);
        assert_eq!(
            run("plugin enable alice.web", &mut h),
            ErrorCode::Denied.exit()
        );
    }
    assert_eq!(
        std::fs::read_to_string(e.home.join("wolos/plugins.json")).unwrap(),
        before
    );
    assert!(controller::catalog::find_tool("plugin_allow_host").is_none());
    assert!(
        !controller::catalog::HELPER_VERBS
            .iter()
            .any(|v| v.path == ["plugin", "allow-host"])
    );
    let _ = std::fs::remove_dir_all(&e.home);
}
