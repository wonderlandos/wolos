// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol shard` (docs/modules/shards.md): every verb maps to the `wopkg shard` argv it promises,
//! the exit status comes back unchanged (off a terminal with the captured JSON, also for `wol mcp`), the catalogue gives helpers read access to list/check and
//! a user-level change for add/update, and, when the wopkg CLI is built (`make` in wopkg/, or
//! `WOPKG_BIN`), the app template after `wopkg shard add` keeps RCR-040 and RCR-041 with the same
//! scanners `rules_crystal.rs` runs over the checkout.
#[path = "support/crystal_scan.rs"]
mod crystal_scan;

use controller::catalog::{Access, find_tool};
use controller::cli::parse;
use controller::mcp::ToolEnv;
use controller::run::{FakeEnv, execute};
use std::path::{Path, PathBuf};

fn argv(line: &str) -> Vec<String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    controller::shard::argv(&parse(&words).unwrap_or_else(|e| panic!("{line}: {e}")))
        .unwrap_or_else(|e| panic!("{line}: {e}"))
}

fn w(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn every_verb_maps_to_its_wopkg_argv() {
    for (line, want) in [
        ("shard", "wopkg shard list"),
        ("shard list", "wopkg shard list"),
        ("shard list web server", "wopkg shard list web server"),
        ("shard add wolkit", "wopkg shard add wolkit"),
        ("shard add wolkit app", "wopkg shard add wolkit ./app"),
        (
            "shard add wolkit /src/app --local",
            "wopkg shard add wolkit /src/app --local",
        ),
        ("shard update", "wopkg shard update"),
        ("shard update wolkit", "wopkg shard update wolkit"),
        ("shard update ./app", "wopkg shard update ./app"),
        ("shard update wolkit app", "wopkg shard update wolkit ./app"),
        ("shard check", "wopkg shard check"),
        ("shard check . --json", "wopkg shard check . --json"),
        (
            "shard check app --local --json",
            "wopkg shard check ./app --json --local",
        ),
    ] {
        assert_eq!(argv(line), w(want), "{line}");
    }
    let inv = parse(&["shard", "add", "../x"]).unwrap();
    assert!(
        controller::shard::argv(&inv).is_err(),
        "a folder is not a shard name"
    );
    assert!(parse(&["shard", "add"]).is_err(), "add needs a shard");
    assert!(
        parse(&["shard", "list", "--local"]).is_err(),
        "list has no --local"
    );
}

#[test]
fn the_exit_status_of_wopkg_comes_back() {
    // On a terminal wopkg prints itself; the status is passed through.
    let mut env = FakeEnv {
        tty: true,
        exec_codes: vec![1],
        ..Default::default()
    };
    assert_eq!(execute(&parse(&["shard", "check"]).unwrap(), &mut env), 1);
    assert_eq!(env.execs, [w("wopkg shard check")]);
    assert_eq!(
        execute(&parse(&["shard", "add", "wolkit"]).unwrap(), &mut env),
        0
    );
    assert_eq!(env.execs[1], w("wopkg shard add wolkit"));
    assert!(env.out.is_empty(), "nothing captured on a terminal");
}

const FINDINGS: &str = r#"{"ok":false,"index":true,"findings":[{"rule":"RCR-040","line":7,"file":"shard.yml","message":"wolkit: not pinned by `commit:`"}]}"#;

/// Off a terminal (a pipe, `--json`, `wol mcp`) the JSON document is captured and printed through
/// the env, and a non-zero exit still comes back with it.
#[test]
fn off_a_terminal_the_json_output_and_the_status_both_come_back() {
    let mut env = FakeEnv {
        exec_codes: vec![1],
        captures: vec![("wopkg shard check --json".into(), FINDINGS.into())],
        ..Default::default()
    };
    assert_eq!(execute(&parse(&["shard", "check"]).unwrap(), &mut env), 1);
    assert_eq!(env.execs, [w("wopkg shard check --json")]);
    assert_eq!(env.out, FINDINGS);
    // `--json` given on a terminal: not doubled.
    let mut env = FakeEnv {
        tty: true,
        captured: Some("{}".into()),
        ..Default::default()
    };
    assert_eq!(
        execute(&parse(&["shard", "list", "--json"]).unwrap(), &mut env),
        0
    );
    assert_eq!(env.execs, [w("wopkg shard list --json")]);
    assert_eq!(env.out, "{}");
}

/// `wol mcp`: the tool env keeps child stdout off the protocol, so the document must arrive in
/// its captured output, with the failing status.
#[test]
fn a_helper_gets_the_findings_and_the_failure() {
    let mut inner = FakeEnv {
        tty: true,
        exec_codes: vec![1],
        captures: vec![("wopkg shard check ./app --json".into(), FINDINGS.into())],
        ..Default::default()
    };
    let mut tool = ToolEnv::new(&mut inner, "mcp:test".into());
    let code = execute(&parse(&["shard", "check", "app"]).unwrap(), &mut tool);
    assert_eq!(code, 1);
    let v: serde_json::Value = serde_json::from_str(&tool.out).unwrap();
    assert_eq!(v["findings"][0]["rule"], "RCR-040");
    drop(tool);
    assert_eq!(inner.execs, [w("wopkg shard check ./app --json")]);
}

#[test]
fn helpers_may_look_and_check_and_pin_as_the_person() {
    for (tool, access) in [
        ("shard", Access::Read),
        ("shard_list", Access::Read),
        ("shard_check", Access::Read),
        ("shard_add", Access::Change),
        ("shard_update", Access::Change),
    ] {
        let v = find_tool(tool).unwrap_or_else(|| panic!("{tool} is not a tool"));
        assert_eq!(v.access(), Some(access), "{tool}");
    }
}

// ---- the app template after `wopkg shard add` (needs the built wopkg CLI) ----

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn wopkg_bin() -> Option<PathBuf> {
    std::env::var_os("WOPKG_BIN")
        .map(PathBuf::from)
        .or_else(|| Some(repo().join("wopkg/build/wopkg")))
        .filter(|p| p.is_file())
}

#[test]
fn the_app_template_after_shard_add_keeps_rcr_040_and_041() {
    let Some(bin) = wopkg_bin() else {
        if std::env::var_os("WOLOS_REQUIRE_CRYSTAL_LINT").is_some() {
            panic!("wopkg/build/wopkg is not built (make -C wopkg) and WOPKG_BIN is not set");
        }
        eprintln!(
            "skipped: wopkg/build/wopkg is not built (make -C wopkg) and WOPKG_BIN is not set"
        );
        return;
    };
    let fixture = repo().join("wopkg/spec/fixtures/shards");
    let tmp = std::env::temp_dir().join(format!("wol-shard-template-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let (cache, app) = (tmp.join("cache/shard-index"), tmp.join("app"));
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::create_dir_all(&app).unwrap();
    for f in ["index.json", "index.json.sig"] {
        std::fs::copy(fixture.join(f), cache.join(f)).unwrap();
    }
    let tmpl = std::fs::read_to_string(repo().join("wolos-crystal/templates/app/shard.yml.tmpl"))
        .unwrap()
        .replace("{{name}}", "demo")
        .replace("{{class_name}}", "Demo");
    std::fs::write(app.join("shard.yml"), &tmpl).unwrap();
    let config = serde_json::json!({
        "home": tmp, "cache_dir": tmp.join("cache"), "state_dir": tmp.join("state"),
        "keys_dir": fixture.join("keys"),
    });
    let out = std::process::Command::new(&bin)
        .args(["shard", "add", "wolkit"])
        .arg(&app)
        .args(["--local", "--json"])
        .env("WOPKG_CONFIG", config.to_string())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let yml = std::fs::read_to_string(app.join("shard.yml")).unwrap();
    let lock = std::fs::read_to_string(app.join("shard.lock")).unwrap();
    assert!(yml.starts_with(&tmpl), "the template is kept:\n{yml}");
    let deps = crystal_scan::parse_shard_yml(&yml).1;
    assert_eq!(deps.len(), 1, "{yml}");
    assert_eq!(crystal_scan::scan_shard_yml(&yml), vec![], "RCR-040");
    assert_eq!(
        crystal_scan::scan_shard_lock(&yml, Some(&lock)),
        vec![],
        "RCR-041"
    );
    let locked = crystal_scan::parse_shard_lock(&lock);
    assert_eq!(locked.keys().collect::<Vec<_>>(), ["cryswol", "wolkit"]);
    // and `check` agrees, with the list
    let out = std::process::Command::new(&bin)
        .args(["shard", "check"])
        .arg(&app)
        .arg("--json")
        .env("WOPKG_CONFIG", config.to_string())
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        (v["ok"].as_bool(), v["index"].as_bool()),
        (Some(true), Some(true)),
        "{v}"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}
