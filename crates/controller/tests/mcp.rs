// SPDX-License-Identifier: GPL-2.0-or-later
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

/// An `initialize` request from the client called `name`.
fn hello(id: u64, name: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": name, "version": "1"}}
    })
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
    // QA-PLUG-59: before `initialize` no tool is called or listed.
    assert_eq!(r["error"]["code"], -32002, "{r}");
    let r = rpc(
        &mut s,
        &mut e,
        json!({"jsonrpc":"2.0","id":5,"method":"tools/list"}),
    )
    .unwrap();
    assert_eq!(r["error"]["code"], -32002, "{r}");
    rpc(&mut s, &mut e, hello(6, "t")).unwrap();
    let r = rpc(
        &mut s,
        &mut e,
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"rm_rf"}}),
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
        // Folded to 120 columns (QA-PWR-6): the sentence is there word for word.
        let flat = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat(&help).contains(&flat(v.cmd.help)), "{}", v.tool());
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

/// The approve lines sent to the shell socket (other requests, e.g. `reload`, skipped), and the
/// administrator approvals asked through wold (`session.approve`, R2C-6).
fn approvals(env: &FakeEnv) -> Vec<Approval> {
    env.requests
        .iter()
        .filter_map(|r| {
            let l = String::from_utf8(r.1.clone()).unwrap();
            if r.0 == std::path::Path::new("/run/u/wol-shell.sock") {
                return Approval::parse(l.strip_prefix("approve ")?);
            }
            via_wold(&r.0, &l)
        })
        .collect()
}

/// An approval the helper asked wold to show (R2C-6).
fn via_wold(sock: &std::path::Path, line: &str) -> Option<Approval> {
    if sock != std::path::Path::new(shared::jobs::SOCKET) {
        return None;
    }
    match shared::portal::Call::decode(line.strip_prefix("call ")?.trim().as_bytes()).ok()? {
        shared::portal::Call::SessionApprove { approval } => Some(approval),
        _ => None,
    }
}

/// wold's reply carrying the sheet's answer.
fn wold(a: Answer) -> String {
    format!(
        "{}
",
        json!({"ok": true, "data": a})
    )
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

/// Audit C10-1: an `apps` grant from an `open` sheet never covers a change that leaves something
/// behind (here removing a web app): it still gets a sheet, without Always Allow being stored.
#[test]
fn an_open_grant_does_not_cover_web_apps() {
    let mut e = env("onceonly");
    std::fs::create_dir_all(e.home.join("wolos")).unwrap();
    std::fs::write(
        e.home.join("wolos/helpers.json"),
        r#"{"enabled":true,"deny":[],"always":{"Claude Desktop":["apps"]}}"#,
    )
    .unwrap();
    let mut s = init(&mut e);
    let r = call(&mut s, &mut e, "webapp_remove", json!({"name": "Mail"}));
    assert_eq!(r["isError"], true, "no answer here: {r}");
    let asked = approvals(&e);
    assert_eq!(asked.len(), 1, "asked on screen");
    assert!(matches!(
        &asked[0].request,
        ApprovalRequest::Change { capability, .. } if capability == "apps"
    ));
}

/// Audit C10-6: Always Allow belongs to the name and the program that started `wol mcp`. Another
/// program that copies the name gets the sheet, which names its own program.
#[test]
fn always_allow_is_bound_to_the_starting_program() {
    let start = |e: &mut FakeEnv, program: Option<&str>| {
        let mut s = Server::started_by(program.map(str::to_owned));
        rpc(
            &mut s,
            e,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"Claude Desktop","version":"1"}}}),
        )
        .unwrap();
        s
    };
    let mut e = env("bound");
    std::fs::create_dir_all(e.home.join("wolos")).unwrap();
    std::fs::write(
        e.home.join("wolos/helpers.json"),
        r#"{"enabled":true,"deny":[],"always":{"Claude Desktop (/usr/bin/a)":["appearance"],"Claude Desktop":["appearance"]}}"#,
    )
    .unwrap();
    let mut s = start(&mut e, Some("/usr/bin/a"));
    assert_eq!(s.helper(), "Claude Desktop (/usr/bin/a)");
    let r = call(&mut s, &mut e, "appearance_set", json!({"mode": "dark"}));
    assert_eq!(r["isError"], false, "{r}");
    assert!(approvals(&e).is_empty(), "program A was granted");
    // Same name, another program; and a program that could not be read: both are asked.
    for (program, shown) in [
        (Some("/usr/bin/b"), "Claude Desktop (/usr/bin/b)"),
        (None, "Claude Desktop (unknown program)"),
    ] {
        let mut e2 = env("bound2");
        std::fs::create_dir_all(e2.home.join("wolos")).unwrap();
        std::fs::copy(
            e.home.join("wolos/helpers.json"),
            e2.home.join("wolos/helpers.json"),
        )
        .unwrap();
        let mut s = start(&mut e2, program);
        let r = call(&mut s, &mut e2, "appearance_set", json!({"mode": "light"}));
        assert_eq!(r["isError"], true, "no answer here: {r}");
        let asked = approvals(&e2);
        assert_eq!(asked.len(), 1, "{program:?} gets the sheet");
        assert_eq!(asked[0].helper, shown);
    }
    // A long path keeps its end and the whole name fits what wold relays.
    let long = format!("/{}/claude", "d".repeat(300));
    let mut e3 = env("bound3");
    let s = start(&mut e3, Some(&long));
    let h = s.helper();
    assert!(h.len() <= shared::portal::MAX_HELPER_TEXT, "{h}");
    assert!(
        h.starts_with("Claude Desktop (…") && h.ends_with("/claude)"),
        "{h}"
    );
}

/// QA-PLUG-58: a JSON array is a batch: one array of answers (notifications give none); an empty
/// array is one Invalid Request.
#[test]
fn a_batch_gets_an_array_of_answers() {
    let mut e = env("batch");
    let mut s = init(&mut e);
    let batch = json!([
        {"jsonrpc":"2.0","id":1,"method":"ping"},
        {"jsonrpc":"2.0","method":"notifications/initialized"},
        {"jsonrpc":"2.0","id":2,"method":"nope"}
    ]);
    let r = s.handle(&batch.to_string(), &mut e).unwrap();
    let a = r.as_array().expect("an array");
    assert_eq!(a.len(), 2, "{r}");
    assert_eq!(a[0]["result"], json!({}));
    assert_eq!(a[1]["error"]["code"], -32601);
    let only_notes = json!([{"jsonrpc":"2.0","method":"notifications/initialized"}]);
    assert!(s.handle(&only_notes.to_string(), &mut e).is_none());
    let r = s.handle("[]", &mut e).unwrap();
    assert_eq!(r["error"]["code"], -32600);
}

/// QA-PLUG-59: a line over the cap is answered as too large without being parsed.
#[test]
fn an_oversize_line_is_refused() {
    let mut e = env("bigline");
    let mut s = init(&mut e);
    let r = s.handle(controller::mcp::TOO_LONG, &mut e).unwrap();
    assert_eq!(r["error"]["code"], -32600);
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("too large"),
        "{r}"
    );
    assert_eq!(controller::mcp::MAX_LINE, 1 << 20);
}

/// QA-PLUG-60: `wol mcp` does not serve root.
#[test]
fn mcp_refuses_root() {
    let mut e = env("root");
    e.root = true;
    let code = controller::mcp::serve(&mut e);
    assert_eq!(code, shared::errors::ErrorCode::Denied.exit());
    assert!(e.err.contains("not for root"), "{}", e.err);
}

/// QA-PLUG-79: `modules_enable` for a module that is not installed answers the sentence at
/// once; nobody is asked first.
#[test]
fn modules_enable_checks_installed_first() {
    let mut e = env("enable79");
    let dir = "/fake/modules";
    let record = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../files/usr/share/wolos/modules/ai-local.json");
    e.vars
        .push((controller::modules::DIR_VAR.into(), dir.into()));
    e.files.push((
        PathBuf::from(format!("{dir}/ai-local.json")),
        std::fs::read_to_string(record).unwrap(),
    ));
    e.captures.push(("pacman -Qq".into(), "bash\n".into()));
    let mut s = init(&mut e);
    let r = call(&mut s, &mut e, "modules_enable", json!({"id": "ai-local"}));
    assert_eq!(r["isError"], true, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("is not installed yet") && text.contains("wol modules install ai-local"),
        "{text}"
    );
    assert!(
        approvals(&e).is_empty(),
        "no sheet for a change with nothing to do"
    );
}

/// QA-PLUG-26: a client's self-update moves its program to a new versioned path; the grant given
/// before still holds, and another folder is still another program.
#[test]
fn a_grant_survives_the_clients_self_update() {
    for (n, (program, granted)) in [
        ("/home/bob/.local/share/claude/versions/2.1.3", true),
        ("/home/bob/.local/share/claude/versions/2.2.0-beta1", true),
        ("/home/bob/.local/share/other/versions/2.1.3", false),
    ]
    .into_iter()
    .enumerate()
    {
        let mut e = env(&format!("selfupdate{n}"));
        std::fs::create_dir_all(e.home.join("wolos")).unwrap();
        std::fs::write(
            e.home.join("wolos/helpers.json"),
            r#"{"enabled":true,"deny":[],"always":{"Claude Code (/home/bob/.local/share/claude/versions/*)":["appearance"]}}"#,
        )
        .unwrap();
        let mut s = Server::started_by(Some(program.to_owned()));
        rpc(&mut s, &mut e, hello(1, "Claude Code")).unwrap();
        let r = call(&mut s, &mut e, "appearance_set", json!({"mode": "dark"}));
        assert_eq!(approvals(&e).is_empty(), granted, "{program}: {r}");
    }
    assert_eq!(
        controller::mcp::unversioned("/opt/app-1.2/bin/v3.4.5/tool"),
        "/opt/app-1.2/bin/*/tool"
    );
    assert_eq!(controller::mcp::unversioned("/usr/bin/a"), "/usr/bin/a");
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

/// QA-PWR-20: a value the verb would refuse is reported as that, not as "No WolOS session is
/// showing" (the gate ran first and found nobody to ask).
#[test]
fn a_bad_value_is_not_blamed_on_a_missing_session() {
    let mut e = FakeEnv {
        home: temp_home("badvalue"),
        ..Default::default()
    };
    let mut s = init(&mut e);
    for (tool, args, want) in [
        (
            "audio_volume",
            json!({"percent": "abc"}),
            "argument 'percent' is a whole number from 0 to 100",
        ),
        (
            "audio_volume",
            json!({"percent": 300}),
            "argument 'percent' is a whole number from 0 to 100",
        ),
        (
            "display_brightness",
            json!({"percent": 0}),
            "argument 'percent' is a whole number from 1 to 100",
        ),
    ] {
        let r = call(&mut s, &mut e, tool, args);
        assert_eq!(r["isError"], true, "{r}");
        assert_eq!(text(&r), want, "{r}");
    }
    assert!(e.requests.is_empty() && e.execs.is_empty());
    // A good value still reaches the gate, which has no session to ask.
    let r = call(&mut s, &mut e, "audio_volume", json!({"percent": 40}));
    assert!(text(&r).contains("No WolOS session is showing"), "{r}");
}

/// Admin verbs: the shell asks for the password and runs it; the helper never does.
#[test]
fn admin_tool_runs_in_the_shell_after_the_password() {
    let mut e = env("admin");
    e.replies = vec![wold(Answer::Done)];
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
    // R2C-6: asked through wold, never on the shell socket directly.
    assert!(
        e.requests
            .iter()
            .all(|r| r.0 == std::path::Path::new(shared::jobs::SOCKET)),
        "{:?}",
        e.requests.iter().map(|r| &r.0).collect::<Vec<_>>()
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
            wold(Answer::Failed {
                message: "Wi-Fi hardware is off".into(),
            }),
            "Wi-Fi hardware is off",
            "failed",
        ),
        (wold(Answer::AllowOnce), "could not ask", "refused"),
        (wold(Answer::Deny), "declined", "declined"),
        (
            format!(
                "{}
",
                json!({"ok": false, "code": "unavailable", "message": shared::portal::BUSY})
            ),
            "already waiting",
            "refused",
        ),
        (
            format!(
                "{}
",
                json!({"ok": false, "code": "permission-denied", "message": "no"})
            ),
            "could not ask on screen (no)",
            "refused",
        ),
        // Audit S4-12: wold's reason is kept, not replaced by a generic line.
        (
            format!(
                "{}
",
                json!({"ok": false, "code": "invalid-argument", "message": "approval too long"})
            ),
            "could not ask on screen (approval too long)",
            "refused",
        ),
        (
            "{\"ok\": true, \"data\": 7}\n".to_string(),
            "could not ask on screen (wold's answer could not be read)",
            "refused",
        ),
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
        ("restore_activate", json!({"be": "wolos-b"})),
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

/// Optional modules (ADR 0032): list, status and `hw compute` are reads; `enable` asks with the
/// `apps` capability and a plain sentence, then runs the user step here; install and remove are not
/// tools until the shell can run their approval.
#[test]
fn modules_tools_follow_the_gate() {
    let mut e = env("modules");
    let dir = e.home.join("modules");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("dev.json"),
        r#"{"id":"dev","name":"Developer","summary":"Tools","category":"developer","size_mb":9,
            "packages":["git"],"post":["/usr/lib/wolos/modules-enable-dev"]}"#,
    )
    .unwrap();
    e.vars.push((
        controller::modules::DIR_VAR.into(),
        dir.to_string_lossy().into_owned(),
    ));
    e.captures = vec![("pacman -Qq".into(), "wolos-module-dev\n".into())];
    let mut s = init(&mut e);
    let names: Vec<String> = catalog::tools().iter().map(|v| v.tool()).collect();
    for t in [
        "modules",
        "modules_list",
        "modules_status",
        "modules_enable",
        "hw_compute",
    ] {
        assert!(names.contains(&t.to_string()), "{t}");
    }
    for t in ["modules_install", "modules_remove"] {
        assert!(
            !names.contains(&t.to_string()),
            "{t} needs an on-screen approval the shell runs first"
        );
    }
    let r = call(&mut s, &mut e, "modules_list", json!({}));
    assert_eq!(r["isError"], false, "{r}");
    let items = r["structuredContent"]["items"].as_array().unwrap();
    assert_eq!(items[0]["id"], "dev");
    assert_eq!(
        (
            items[0]["installed"].as_bool(),
            items[0]["enabled"].as_bool()
        ),
        (Some(true), Some(false))
    );
    assert!(approvals(&e).is_empty(), "reads never ask");

    e.replies = vec![Answer::AllowOnce.line()];
    let r = call(&mut s, &mut e, "modules_enable", json!({"id": "dev"}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["enabled"], true);
    assert_eq!(
        approvals(&e),
        [Approval {
            helper: "Claude Desktop".into(),
            tool: "modules enable".into(),
            request: ApprovalRequest::Change {
                capability: "apps".into(),
                what: "finish setting up the “dev” module".into(),
            },
        }]
    );
    assert_eq!(e.execs, [["/usr/lib/wolos/modules-enable-dev"]]);
    let saved = std::fs::read_to_string(e.home.join("state/wolos/modules.json")).unwrap();
    assert!(saved.contains("\"dev\""), "{saved}");

    let r = call(&mut s, &mut e, "hw_compute", json!({}));
    assert_eq!(r["isError"], false, "{r}");
    let backend = r["structuredContent"]["backend"].as_str().unwrap();
    assert!(
        ["cuda", "rocm", "vulkan", "cpu"].contains(&backend),
        "{backend}"
    );
    let a = audit(&e);
    assert_eq!(
        a.iter()
            .map(|x| x["tool"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["modules list", "modules enable", "hw compute"]
    );
    assert_eq!(a[1]["result"], "approved");
}

/// R2C-6: every administrator helper verb is asked through wold, which names the helper by its
/// kernel executable; the shell would refuse it from a same-account process directly.
#[test]
fn every_admin_tool_is_asked_through_wold() {
    for (tool, args) in [
        ("wifi_on", json!({})),
        ("wifi_off", json!({})),
        ("wifi_join", json!({"ssid": "Home", "password": "hunter22"})),
        ("bluetooth_on", json!({})),
        ("bluetooth_off", json!({})),
        ("restore_activate", json!({"be": "wolos-b"})),
    ] {
        let mut e = env("viawold");
        e.replies = vec![wold(Answer::Done)];
        let mut s = init(&mut e);
        let r = call(&mut s, &mut e, tool, args);
        assert_eq!(r["isError"], false, "{tool}: {r}");
        let a = approvals(&e);
        assert_eq!(a.len(), 1, "{tool}");
        assert!(a[0].request.is_admin(), "{tool}");
        assert!(
            e.requests
                .iter()
                .all(|q| q.0 == std::path::Path::new(shared::jobs::SOCKET)),
            "{tool}: nothing on the shell socket"
        );
    }
}

/// QA-PLUG-25: `initialize` is answered once; a second one cannot rename the client mid-session.
#[test]
fn a_second_initialize_is_refused() {
    let mut e = env("reinit");
    let mut s = init(&mut e);
    let r = rpc(&mut s, &mut e, hello(2, "Trusted Thing")).unwrap();
    assert_eq!(r["error"]["code"], -32600, "{r}");
    assert_eq!(s.client(), "Claude Desktop");
}

/// QA-PLUG-24: a percent that is not a whole number from 0 to 100 is a tool error before any
/// sheet: the sheet never reads "wants to set the volume to abc%".
#[test]
fn bad_percent_never_reaches_a_sheet() {
    for (tool, bad) in [
        ("audio_volume", json!("abc")),
        ("audio_volume", json!("150")),
        ("audio_volume", json!(-3)),
        ("display_brightness", json!("40%")),
    ] {
        let mut e = env("percent");
        e.replies = vec![Answer::AllowOnce.line()];
        let mut s = init(&mut e);
        let r = call(&mut s, &mut e, tool, json!({ "percent": bad }));
        assert_eq!(r["isError"], true, "{tool} {bad}: {r}");
        assert!(text(&r).contains(" to 100"), "{tool} {bad}: {r}");
        assert!(e.requests.is_empty(), "{tool} {bad}: a sheet was asked for");
    }
}
