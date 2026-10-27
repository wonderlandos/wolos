//! `wol mcp` protocol (JSON-RPC 2.0 over stdio) against a recording fake: initialize, tools/list
//! generated from the catalogue, tools/call happy path, policy refusals, and approvals: changes
//! ask on screen first, administrator changes run only in the shell.
use controller::catalog;
use controller::cli::{TREE, parse};
use controller::mcp::{Server, VERSIONS};
use controller::run::{FakeEnv, execute};
use serde_json::{Value, json};
use shared::shell::{Answer, Approval, ApprovalRequest};
use std::path::PathBuf;

fn temp_home(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-mcp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn env(tag: &str) -> FakeEnv {
    FakeEnv {
        home: temp_home(tag),
        vars: vec![("XDG_RUNTIME_DIR".into(), "/run/u".into())],
        ..Default::default()
    }
}

fn rpc(s: &mut Server, env: &mut FakeEnv, msg: Value) -> Option<Value> {
    s.handle(&msg.to_string(), env)
}

fn init(env: &mut FakeEnv) -> Server {
    let mut s = Server::default();
    let r = rpc(
        &mut s,
        env,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"Claude Desktop","version":"1"}}}),
    )
    .unwrap();
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    assert!(
        rpc(
            &mut s,
            env,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .is_none()
    );
    s
}

fn call(s: &mut Server, env: &mut FakeEnv, name: &str, args: Value) -> Value {
    let r = rpc(
        s,
        env,
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":name,"arguments":args}}),
    )
    .unwrap();
    assert_eq!(r["id"], 7);
    r["result"].clone()
}

fn audit(env: &FakeEnv) -> Vec<Value> {
    std::fs::read_to_string(env.home.join("state/wolos/actions.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn initialize_negotiates_and_describes_the_server() {
    let mut e = env("init");
    let s = init(&mut e);
    assert!(s.initialized);
    assert_eq!(s.client(), "Claude Desktop");
    let mut s = Server::default();
    let r = rpc(
        &mut s,
        &mut e,
        json!({"jsonrpc":"2.0","id":"a","method":"initialize","params":{"protocolVersion":"1999-01-01"}}),
    )
    .unwrap();
    let res = &r["result"];
    assert_eq!(
        res["protocolVersion"], VERSIONS[0],
        "unknown version: our newest"
    );
    assert_eq!(res["serverInfo"]["name"], "wolos");
    assert_eq!(res["capabilities"]["tools"]["listChanged"], false);
    assert!(res["instructions"].as_str().unwrap().contains("approval"));
    assert_eq!(r["id"], "a");
}

#[test]
fn protocol_errors() {
    let mut e = env("errors");
    let mut s = Server::default();
    let parse_err = s.handle("{nope", &mut e).unwrap();
    assert_eq!(
        (parse_err["error"]["code"].as_i64(), &parse_err["id"]),
        (Some(-32700), &Value::Null)
    );
    let r = rpc(
        &mut s,
        &mut e,
        json!({"jsonrpc":"2.0","id":2,"method":"resources/list"}),
    )
    .unwrap();
    assert_eq!(r["error"]["code"], -32601);
    let r = rpc(
        &mut s,
        &mut e,
        json!({"jsonrpc":"1.0","id":3,"method":"ping"}),
    )
    .unwrap();
    assert_eq!(r["error"]["code"], -32600);
    let r = rpc(
        &mut s,
        &mut e,
        json!({"jsonrpc":"2.0","id":4,"method":"ping"}),
    )
    .unwrap();
    assert_eq!(r["result"], json!({}));
    let r = rpc(
        &mut s,
        &mut e,
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"rm_rf"}}),
    )
    .unwrap();
    assert_eq!(r["error"]["code"], -32602);
    assert!(
        rpc(&mut s, &mut e, json!({"jsonrpc":"2.0","id":9,"result":{}})).is_none(),
        "client responses get no reply"
    );
}

#[test]
fn tools_list_is_the_catalogue() {
    let mut e = env("list");
    let mut s = init(&mut e);
    let r = rpc(
        &mut s,
        &mut e,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .unwrap();
    let tools = r["result"]["tools"].as_array().unwrap();
    let want: Vec<String> = catalog::tools().iter().map(|v| v.tool()).collect();
    let got: Vec<String> = tools
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(got, want);
    assert!(got.len() >= 35, "{}", got.len());
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
        assert!(!t["description"].as_str().unwrap().is_empty());
    }
    let wifi = tools.iter().find(|t| t["name"] == "wifi_join").unwrap();
    assert_eq!(wifi["inputSchema"]["required"], json!(["ssid"]));
    assert!(wifi["inputSchema"]["properties"]["password"].is_object());
    assert!(
        wifi["description"]
            .as_str()
            .unwrap()
            .contains("password prompt on screen")
    );
    let appearance = tools
        .iter()
        .find(|t| t["name"] == "appearance_set")
        .unwrap();
    assert_eq!(
        appearance["inputSchema"]["properties"]["mode"]["enum"],
        json!(["light", "dark", "auto"])
    );
    assert_eq!(
        tools.iter().find(|t| t["name"] == "theme_get").unwrap()["annotations"]["readOnlyHint"],
        true
    );
    for hidden in [
        "shell_toggle",
        "admin",
        "jobs_start",
        "helpers_off",
        "update_apply",
        "mcp",
    ] {
        assert!(
            !got.iter().any(|g| g == hidden),
            "{hidden} must not be a tool"
        );
    }
}

/// Catalogue/CLI parity: every tool is a CLI command that parses, has help and a handler.
#[test]
fn every_tool_parses_has_help_and_a_handler() {
    for v in catalog::tools() {
        let mut argv: Vec<&str> = v.cmd.path.to_vec();
        for a in v.cmd.args.iter().filter(|a| !a.optional) {
            argv.push(a.choices.first().copied().unwrap_or("x"));
        }
        let inv = parse(&argv).unwrap_or_else(|e| panic!("{}: {e}", v.tool()));
        assert!(!v.cmd.help.is_empty());
        let help =
            controller::cli::help(&v.cmd.path.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(help.contains(v.cmd.help), "{}", v.tool());
        let mut e = FakeEnv {
            home: temp_home("parity"),
            ..Default::default()
        };
        execute(&inv, &mut e);
        assert!(!e.err.contains("no handler"), "{}: {}", v.tool(), e.err);
    }
    assert!(TREE.iter().any(|c| c.path == ["mcp"]));
}

#[test]
fn theme_get_happy_path() {
    let mut e = env("themeget");
    let mut s = init(&mut e);
    let r = call(&mut s, &mut e, "theme_get", json!({}));
    assert_eq!(r["isError"], false, "{r}");
    let sc = &r["structuredContent"];
    assert!(sc["theme"].as_str().unwrap().starts_with("wol-"), "{sc}");
    assert_eq!(sc["mode"], "auto");
    assert_eq!(sc["accent"], "violet");
    let text: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(&text, sc);
    let a = audit(&e);
    assert_eq!(
        (
            a[0]["helper"].as_str(),
            a[0]["tool"].as_str(),
            a[0]["result"].as_str()
        ),
        (Some("Claude Desktop"), Some("theme get"), Some("ok"))
    );
}

/// The approve lines sent to the shell socket (other requests, e.g. `reload`, skipped).
fn approvals(env: &FakeEnv) -> Vec<Approval> {
    env.requests
        .iter()
        .filter(|r| r.0 == std::path::Path::new("/run/u/wol-shell.sock"))
        .filter_map(|r| {
            let l = String::from_utf8(r.1.clone()).unwrap();
            Approval::parse(l.strip_prefix("approve ")?)
        })
        .collect()
}

fn text(r: &Value) -> &str {
    r["content"][0]["text"].as_str().unwrap()
}

#[test]
fn dark_mode_change_is_journaled_undoable_and_logged() {
    let mut e = env("dark");
    e.replies = vec![Answer::AllowOnce.line()];
    let mut s = init(&mut e);
    let r = call(&mut s, &mut e, "appearance_set", json!({"mode": "dark"}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["mode"], "dark");
    assert_eq!(r["structuredContent"]["undo"], 1);
    assert_eq!(
        std::fs::read_to_string(e.home.join("wolos/theme")).unwrap(),
        "wol-dark\n"
    );
    let h = call(&mut s, &mut e, "history", json!({}));
    let items = h["structuredContent"]["items"].as_array().unwrap();
    assert_eq!(
        (items[0]["by"].as_str(), items[0]["what"].as_str()),
        (Some("mcp:Claude Desktop"), Some("theme"))
    );
    e.replies = vec![Answer::AllowOnce.line()];
    let u = call(&mut s, &mut e, "undo", json!({}));
    assert_eq!(u["structuredContent"]["undid"], 1, "{u}");
    assert!(!e.home.join("wolos/theme").exists(), "back to Auto");
    let a = audit(&e);
    assert_eq!(
        a.iter()
            .map(|x| x["tool"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["appearance set", "history", "undo"]
    );
    assert_eq!(a[0]["args"], json!(["dark"]));
    assert_eq!(a[0]["result"], "approved");
}

/// A change asks the shell first: who, what in plain words, which capability; then runs here.
#[test]
fn change_asks_on_screen_then_runs_once_allowed() {
    let mut e = env("ask");
    e.replies = vec![Answer::AllowOnce.line()];
    let mut s = init(&mut e);
    let r = call(&mut s, &mut e, "appearance_set", json!({"mode": "dark"}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(
        approvals(&e),
        [Approval {
            helper: "Claude Desktop".into(),
            tool: "appearance set".into(),
            request: ApprovalRequest::Change {
                capability: "appearance".into(),
                what: "switch to Dark appearance".into(),
            },
        }]
    );
    let a = audit(&e);
    assert_eq!(a.len(), 1, "one record per call");
    assert_eq!(
        (a[0]["result"].as_str(), a[0]["detail"].as_str()),
        (Some("approved"), Some("allowed once"))
    );
    let r = call(&mut s, &mut e, "theme_get", json!({}));
    assert_eq!(r["isError"], false);
    assert_eq!(approvals(&e).len(), 1, "reads never ask");
}

/// Deny, no answer in time, a full queue, or a shell that cannot answer: nothing changes.
#[test]
fn every_refusal_changes_nothing() {
    for (reply, outcome, says) in [
        (Answer::Deny.line(), "declined", "declined"),
        (Answer::TimedOut.line(), "declined", "No answer"),
        (Answer::Busy.line(), "refused", "already waiting"),
        (String::new(), "refused", "could not ask"),
    ] {
        let mut e = env("refusals");
        e.replies = vec![reply.clone()];
        let mut s = init(&mut e);
        let r = call(&mut s, &mut e, "appearance_set", json!({"mode": "dark"}));
        assert_eq!(r["isError"], true, "{reply}: {r}");
        assert!(text(&r).contains(says), "{reply}: {r}");
        assert!(text(&r).contains("Nothing changed"), "{reply}: {r}");
        assert!(!e.home.join("wolos/theme").exists(), "{reply}");
        assert_eq!(audit(&e)[0]["result"], outcome, "{reply}");
    }
}

/// Always Allow (saved by the shell) skips the sheet for that helper and capability only.
#[test]
fn always_allow_skips_the_sheet() {
    let mut e = env("always");
    std::fs::create_dir_all(e.home.join("wolos")).unwrap();
    std::fs::write(
        e.home.join("wolos/helpers.json"),
        r#"{"enabled":true,"deny":[],"always":{"Claude Desktop":["appearance"]}}"#,
    )
    .unwrap();
    let mut s = init(&mut e);
    let r = call(&mut s, &mut e, "appearance_set", json!({"mode": "dark"}));
    assert_eq!(r["isError"], false, "{r}");
    assert!(approvals(&e).is_empty());
    assert_eq!(audit(&e)[0]["detail"], "always allowed");
    let r = call(&mut s, &mut e, "audio_mute", json!({}));
    assert_eq!(
        r["isError"], true,
        "sound was never granted: asks (no answer here)"
    );
    assert_eq!(approvals(&e).len(), 1);
}

/// AllowAlways from the shell runs it now (the shell saved the grant).
#[test]
fn allow_always_runs_now() {
    let mut e = env("allowalways");
    e.replies = vec![Answer::AllowAlways.line()];
    let mut s = init(&mut e);
    let r = call(&mut s, &mut e, "focus_on", json!({}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(audit(&e)[0]["detail"], "always allowed from now on");
}

#[test]
fn bad_arguments_are_tool_errors() {
    let mut e = env("badargs");
    let mut s = init(&mut e);
    let r = call(&mut s, &mut e, "appearance_set", json!({"mode": "sepia"}));
    assert_eq!(r["isError"], true);
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("must be one of"),
        "{r}"
    );
    let r = call(&mut s, &mut e, "audio_volume", json!({}));
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("missing argument 'percent'")
    );
    let r = call(&mut s, &mut e, "audio_volume", json!({"percent": 300}));
    assert_eq!(r["isError"], true);
    assert!(!e.home.join("wolos/theme").exists());
}

/// Admin verbs: the shell asks for the password and runs it; the helper never does.
#[test]
fn admin_tool_runs_in_the_shell_after_the_password() {
    let mut e = env("admin");
    e.replies = vec![Answer::Done.line()];
    let mut s = init(&mut e);
    let r = call(
        &mut s,
        &mut e,
        "wifi_join",
        json!({"ssid": "Home", "password": "hunter22"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["status"], "done");
    assert!(text(&r).contains("join the Wi-Fi network “Home”"), "{r}");
    let a = approvals(&e);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].helper, "Claude Desktop");
    assert_eq!(
        a[0].request,
        ApprovalRequest::WifiJoin {
            ssid: "Home".into(),
            password: Some("hunter22".into())
        }
    );
    assert!(e.execs.is_empty(), "nothing executed here");
    assert!(
        e.admin.lock().unwrap().sent.is_empty(),
        "no wol-admind request from the helper"
    );
    let log = std::fs::read_to_string(e.home.join("state/wolos/actions.jsonl")).unwrap();
    assert!(
        log.contains("\"approved\"") && !log.contains("hunter22"),
        "{log}"
    );
}

/// Failed in the shell: the helper hears why. An answer meant for user-level changes never makes
/// the helper run an administrator change itself.
#[test]
fn admin_failures_and_wrong_answers() {
    for (reply, says, outcome) in [
        (
            Answer::Failed {
                message: "Wi-Fi hardware is off".into(),
            }
            .line(),
            "Wi-Fi hardware is off",
            "failed",
        ),
        (Answer::AllowOnce.line(), "could not ask", "refused"),
        (Answer::Deny.line(), "declined", "declined"),
    ] {
        let mut e = env("adminfail");
        e.replies = vec![reply.clone()];
        let mut s = init(&mut e);
        let r = call(&mut s, &mut e, "wifi_off", json!({}));
        assert_eq!(r["isError"], true, "{reply}: {r}");
        assert!(text(&r).contains(says), "{reply}: {r}");
        assert!(e.execs.is_empty() && e.admin.lock().unwrap().sent.is_empty());
        assert_eq!(audit(&e)[0]["result"], outcome, "{reply}");
    }
}

#[test]
fn changes_without_a_session_are_refused() {
    for (tool, args) in [
        ("restore_activate", json!({"be": "wol-before-update-1"})),
        ("appearance_set", json!({"mode": "dark"})),
    ] {
        let mut e = FakeEnv {
            home: temp_home("nosession"),
            ..Default::default()
        };
        let mut s = init(&mut e);
        let r = call(&mut s, &mut e, tool, args);
        assert_eq!(r["isError"], true);
        assert!(text(&r).contains("No WolOS session is showing"), "{r}");
        assert!(e.requests.is_empty() && e.execs.is_empty());
        assert!(!e.home.join("wolos/theme").exists());
        assert_eq!(audit(&e)[0]["result"], "refused");
    }
}

#[test]
fn policy_off_switch_and_denied_capability() {
    let mut e = env("policy");
    let mut s = init(&mut e);
    std::fs::create_dir_all(e.home.join("wolos")).unwrap();
    std::fs::write(
        e.home.join("wolos/helpers.json"),
        r#"{"enabled":true,"deny":["appearance"]}"#,
    )
    .unwrap();
    let r = call(&mut s, &mut e, "appearance_set", json!({"mode": "dark"}));
    assert_eq!(r["isError"], true);
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Change appearance"),
        "{r}"
    );
    assert!(!e.home.join("wolos/theme").exists());
    assert_eq!(
        call(&mut s, &mut e, "theme_get", json!({}))["isError"],
        false,
        "reads still allowed"
    );
    std::fs::write(e.home.join("wolos/helpers.json"), r#"{"enabled":false}"#).unwrap();
    let r = call(&mut s, &mut e, "theme_get", json!({}));
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("turned off")
    );
    let a = audit(&e);
    assert_eq!(a.iter().filter(|x| x["result"] == "refused").count(), 2);
}

#[test]
fn helpers_cannot_undo_their_own_permissions() {
    let mut e = env("selfundo");
    assert_eq!(
        execute(&parse(&["helpers", "deny", "files"]).unwrap(), &mut e),
        0,
        "{}",
        e.err
    );
    e.replies = vec![Answer::AllowOnce.line()];
    let mut s = init(&mut e);
    let r = call(&mut s, &mut e, "undo", json!({}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("own permissions")
    );
    let p = std::fs::read_to_string(e.home.join("wolos/helpers.json")).unwrap();
    assert!(p.contains("files"));
}

#[test]
fn serve_reads_lines_until_eof() {
    let mut e = env("serve");
    e.stdin = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","clientInfo":{"name":"t"}}}).to_string(),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string(),
        String::new(),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"theme_get","arguments":{}}}).to_string(),
    ];
    assert_eq!(execute(&parse(&["mcp"]).unwrap(), &mut e), 0);
    let lines: Vec<Value> = e
        .out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "{}", e.out);
    assert_eq!(lines[1]["id"], 2);
    assert_eq!(lines[1]["result"]["isError"], false);
}
