// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol webapp-call <app id> <method> [<params JSON>]` (hidden, ADR 0048): one bridge call of a
//! system web app, run the way `wol mcp` runs a tool. wold starts it, as the account of the app's
//! host process, after it has checked the caller, the method and the grant; this is the part that
//! knows the command tree: the method is a verb's action name, the params its named arguments, and
//! the reply is one JSON line, `{"ok":<the verb's --json output>}` or `{"err":{"code","message",
//! "hint"}}` (`code` is a `shared::errors::ErrorCode` name). The same parser (`cli::parse`) and
//! handlers (`run::dispatch`) answer, so no verb exists for apps that does not exist for people.
//!
//! An administrator change is asked through the helper gate (`approve::decide`: the approval sheet
//! and the password, in wold and the shell), as for `wol mcp`: `update.install` is Update Now's
//! sheet, `doctor.fix` the check's wol-admind requests (or, for a fix of the person's own services,
//! the consent sheet, then the verb runs here). A verb the gate has no request for is refused,
//! never run without a password.
use crate::approve::{self, Verdict};
use crate::catalog::{self, Access};
use crate::cli::{self, TREE};
use crate::mcp::ToolEnv;
use crate::run::Env;
use serde_json::{Map, Value, json};
use shared::errors::ErrorCode;

/// Largest params text and reply wold takes (`shared::portal::MAX_LINE`).
pub const MAX_PARAMS: usize = shared::portal::MAX_LINE;

fn fault(code: ErrorCode, message: &str, hint: &str) -> Value {
    json!({"err": {"code": code.name(), "message": message, "hint": hint}})
}

/// Maps a verb's exit status to its error code name (`ErrorCode::exit`).
fn code_of(exit: i32) -> ErrorCode {
    ErrorCode::ALL
        .into_iter()
        .find(|c| c.exit() == exit && *c != ErrorCode::Ok)
        .unwrap_or(ErrorCode::Failed)
}

/// The reply value for one call (the caller prints it as a line).
pub fn call(app: &str, method: &str, params: &str, env: &mut dyn Env) -> Value {
    if params.len() > MAX_PARAMS {
        return fault(
            ErrorCode::Usage,
            "The request was too large.",
            "params over 60 KiB",
        );
    }
    let args: Map<String, Value> = match serde_json::from_str(if params.trim().is_empty() {
        "{}"
    } else {
        params
    }) {
        Ok(Value::Object(m)) => m,
        _ => {
            return fault(
                ErrorCode::Usage,
                "The request was not understood.",
                "params must be a JSON object",
            );
        }
    };
    // Only the catalog's methods (`wol_core::sysapp`): never a verb no app may hold.
    let Some(cmd) = TREE
        .iter()
        .find(|c| c.action == method)
        .filter(|_| wol_core::sysapp::permission_of(method).is_some())
    else {
        return fault(
            ErrorCode::NotFound,
            "This app asked for something WolOS does not offer.",
            &format!("no verb has the action {method:?}"),
        );
    };
    // `sys` lists its topics as text when it has none: the page asked for data, so it must name one
    // (QA-WEB-34).
    if method == "sys"
        && args
            .get("topic")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return fault(
            ErrorCode::Usage,
            "The request was not understood.",
            "sys needs a topic",
        );
    }
    // A check that does not exist is `not-found` before the gate asks anyone for anything, and its
    // name is not echoed into a sentence (QA-WEB-31).
    if method == "doctor.fix" {
        let id = args.get("id").and_then(Value::as_str).unwrap_or_default();
        if !crate::doctor::exists(env, id) {
            return fault(
                ErrorCode::NotFound,
                "WolOS has no check with that name.",
                &format!("no check {id:?}"),
            );
        }
    }
    let verb = catalog::Verb {
        cmd,
        helper: catalog::HELPER_VERBS.iter().find(|h| h.path == cmd.path),
    };
    let (argv, secret) = match catalog::argv(&verb, &args) {
        Ok(x) => x,
        Err(e) => return fault(ErrorCode::Usage, "The request was not understood.", &e),
    };
    let inv = match cli::parse(&argv) {
        Ok(i) => i,
        Err(e) => {
            return fault(
                ErrorCode::Usage,
                "The request was not understood.",
                &e.to_string(),
            );
        }
    };
    // An administrator change: the sheet and the password, or nothing.
    if wol_core::sysapp::permission_of(method).is_some_and(|p| p.admin) {
        let Some(rule) = catalog::rule_of(&inv).filter(|r| r.access == Access::Admin) else {
            return fault(
                ErrorCode::Unavailable,
                "WolOS cannot ask for your password for this yet, so nothing changed.",
                &format!("{method} has no approval request"),
            );
        };
        // What the verb's own --json would have said, read before the change (the check after).
        let before = (method == "update.install").then(|| crate::run::update_install_result(env));
        match approve::decide(env, app, &verb.tool(), rule, &inv, secret) {
            Verdict::Done(what) => {
                return json!({"ok": match method {
                    "update.install" => before.unwrap_or(Value::Null),
                    "doctor.fix" => crate::doctor::after(env, inv.get("id").unwrap_or_default()),
                    _ => json!({"status": "done", "request": what}),
                }});
            }
            Verdict::Stop(_, msg) => return fault(ErrorCode::Denied, &msg, ""),
            // Allowed on the consent sheet: a fix of the person's own services runs below (the
            // verb sends no wol-admind request for an app). Nothing else gets here.
            Verdict::Run { .. } if method == "doctor.fix" => {}
            Verdict::Run { .. } => {
                return fault(
                    ErrorCode::Unavailable,
                    "WolOS cannot ask for your password for this yet, so nothing changed.",
                    "",
                );
            }
        }
    }
    let mut te = ToolEnv::new(env, format!("webapp:{app}"));
    let code = crate::run::dispatch(&inv, &mut te);
    let (out, err) = (te.out, te.err);
    let read = wol_core::sysapp::permission_of(method).is_some_and(|p| !p.admin);
    // The Doctor's check exits 1 when something needs a fix, and its fix exits non-zero when it did
    // not work: both print the report as JSON first, and the report is the answer (QA-WEB-24: the
    // app showed "It did not work." exactly when the computer had a problem). Only a refusal that
    // never ran (`ran: false` with an `error`) stays an error. Any other read verb whose stdout is
    // one JSON object and stderr is empty answers the same way (`report_of`).
    let doctor = matches!(method, "doctor.check" | "doctor.fix");
    let report = if code == 0 {
        None
    } else if doctor {
        serde_json::from_str::<Value>(out.trim())
            .ok()
            .filter(|r| r.is_object() && !(r["ran"] == false && r.get("error").is_some()))
    } else if read {
        report_of(&out, &err)
    } else {
        None
    };
    if let Some(report) = report {
        return json!({"ok": report});
    }
    if code != 0 {
        let msg = err.trim().trim_start_matches("wol: ").trim().to_string();
        let msg = if msg.is_empty() {
            "It did not work.".to_string()
        } else {
            msg
        };
        return fault(code_of(code), &msg, "");
    }
    let text = out.trim_end();
    json!({"ok": serde_json::from_str::<Value>(text).unwrap_or_else(|_| Value::String(text.to_string()))})
}

/// The JSON report a read verb printed though it exited non-zero, when that is all it said: a
/// check's exit status says what it found (`wol doctor` exits 1 when a check fails, QA-WEB-24),
/// and the report is the answer. None when stderr has words or stdout is not one JSON object.
fn report_of(out: &str, err: &str) -> Option<Value> {
    if !err.trim().is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(out.trim_end())
        .ok()
        .filter(Value::is_object)
}

/// The verb: prints the reply line, exit status 0 whenever a reply was printed (the reply says).
pub fn run(inv: &cli::Invocation, env: &mut dyn Env) -> i32 {
    let reply = call(
        inv.get("app").unwrap_or_default(),
        inv.get("method").unwrap_or_default(),
        inv.get("params").unwrap_or_default(),
        env,
    );
    env.out(&format!("{reply}\n"));
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::FakeEnv;

    fn ask(method: &str, params: &str) -> Value {
        call(
            "org.wolos.webapp.t",
            method,
            params,
            &mut FakeEnv::default(),
        )
    }

    #[test]
    fn a_read_verb_answers_with_its_json_output() {
        let mut env = FakeEnv::default();
        let v = call("org.wolos.webapp.t", "about", "", &mut env);
        assert_eq!(v, json!({"ok": {"os": "WolOS", "kernel": "15.1"}}));
        // The params are the verb's named arguments.
        let v = call(
            "org.wolos.webapp.t",
            "sys",
            r#"{"topic":"about"}"#,
            &mut env,
        );
        assert_eq!(v["ok"]["os"], "WolOS", "{v}");
    }

    /// QA-WEB-24: `wol doctor` exits 1 when a check fails, so Doctor showed "It did not work."
    /// exactly when the computer had a problem. A read verb's JSON report is the answer whatever
    /// its exit status says; a failing read with nothing on stdout is still a fault.
    #[test]
    fn a_check_that_finds_a_problem_answers_with_its_report() {
        let mut env = FakeEnv {
            captures: vec![(
                "systemctl show -p UnitFileState --value nftables.service".into(),
                "disabled\n".into(),
            )],
            ..Default::default()
        };
        let v = call(
            "org.wolos.webapp.doctor",
            "doctor.check",
            r#"{"area":"security"}"#,
            &mut env,
        );
        assert_eq!(v["ok"]["ready"], false, "{v}");
        let fw = v["ok"]["checks"]
            .as_array()
            .and_then(|c| c.iter().find(|c| c["id"] == "security.firewall"))
            .cloned()
            .unwrap_or_default();
        assert_eq!(fw["status"], "fail", "{v}");
        assert_eq!(report_of("", "wol: no such area\n"), None);
        assert_eq!(report_of("not json", ""), None);
        assert_eq!(report_of("[1]", ""), None, "an object only");
        assert_eq!(
            report_of("{\"ready\":false}\n", ""),
            Some(json!({"ready": false}))
        );
    }

    #[test]
    fn what_is_not_a_verb_or_not_an_object_is_refused_with_a_code() {
        assert_eq!(ask("rm.rf", "{}")["err"]["code"], "not-found");
        assert_eq!(ask("sys", "[1]")["err"]["code"], "usage");
        assert_eq!(ask("sys", r#"{"nope":1}"#)["err"]["code"], "usage");
        assert_eq!(
            ask("sys", &"x".repeat(MAX_PARAMS + 1))["err"]["code"],
            "usage"
        );
    }

    /// QA-WEB-34 and QA-WEB-31: `sys` without a topic is `usage` (it printed its topic list as
    /// text), and a check that does not exist is `not-found` whatever its name holds, with no
    /// password sheet and nothing run.
    #[test]
    fn sys_needs_a_topic_and_an_unknown_check_is_not_found() {
        for p in ["{}", r#"{"topic":""}"#, r#"{"topic":7}"#] {
            assert_eq!(ask("sys", p)["err"]["code"], "usage", "{p}");
        }
        for id in [
            "x; rm -rf /",
            "fire\nwall",
            "kitchen.sink",
            "security.nope",
            "",
        ] {
            let mut env = FakeEnv::default();
            let v = call(
                "org.wolos.webapp.t",
                "doctor.fix",
                &json!({ "id": id }).to_string(),
                &mut env,
            );
            assert_eq!(v["err"]["code"], "not-found", "{id:?}: {v}");
            let msg = v["err"]["message"].as_str().unwrap();
            assert!(!msg.contains(id.trim()) || id.is_empty(), "{msg}");
            assert!(!msg.contains('\n'), "{msg:?}");
            assert!(env.execs.is_empty() && env.requests.is_empty(), "{id:?}");
        }
    }

    #[test]
    fn a_verb_no_permission_opens_is_not_run() {
        // The slot update stays for people (root, --yes); no app may hold it.
        let mut env = FakeEnv::default();
        let v = call(
            "org.wolos.webapp.t",
            "update.apply",
            r#"{"be":"wolos-b","release":"2026/11/02"}"#,
            &mut env,
        );
        assert_eq!(v["err"]["code"], "not-found", "{v}");
        assert!(
            env.execs.is_empty() && env.requests.is_empty(),
            "nothing ran or was asked"
        );
    }

    fn session() -> FakeEnv {
        FakeEnv {
            vars: vec![("XDG_RUNTIME_DIR".into(), "/run/u".into())],
            ..Default::default()
        }
    }

    fn wold(a: shared::shell::Answer) -> String {
        format!("{}\n", json!({"ok": true, "data": a}))
    }

    /// The one approval asked through wold (`session.approve`), never on the shell socket.
    fn asked(env: &FakeEnv) -> Vec<shared::shell::Approval> {
        env.requests
            .iter()
            .map(|(sock, line)| {
                assert_eq!(sock, std::path::Path::new(shared::jobs::SOCKET));
                let l = String::from_utf8(line.clone()).unwrap();
                let call = l.strip_prefix("call ").unwrap();
                match shared::portal::Call::decode(call.trim().as_bytes()).unwrap() {
                    shared::portal::Call::SessionApprove { approval } => approval,
                    c => panic!("{c:?}"),
                }
            })
            .collect()
    }

    #[test]
    fn install_updates_is_update_nows_sheet_and_answers_applied_and_restart() {
        use shared::shell::{Answer, ApprovalRequest};
        let mut env = session();
        env.replies = vec![wold(Answer::Done)];
        let v = call("org.wolos.webapp.t", "update.install", "", &mut env);
        assert_eq!(v["ok"]["applied"], 0, "{v}");
        assert_eq!(v["ok"]["restart"], false, "{v}");
        let a = asked(&env);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].helper, "org.wolos.webapp.t");
        assert_eq!(
            a[0].request,
            ApprovalRequest::Packages {
                verb: "update".into(),
                names: vec![]
            }
        );
        // wold waits for the sheet and for wol-admind's long step: so does wol.
        assert!(
            env.waits[0] > shared::shell::APPROVAL_WAIT_SECS + 600,
            "{:?}",
            env.waits
        );
        assert!(env.execs.is_empty() && env.admin.lock().unwrap().sent.is_empty());
        // Declined on screen: nothing ran, and the app hears it.
        let mut env = session();
        env.replies = vec![wold(Answer::Deny)];
        let v = call("org.wolos.webapp.t", "update.install", "", &mut env);
        assert_eq!(v["err"]["code"], "denied", "{v}");
        // No session showing: refused, never run on a cached password.
        let mut env = FakeEnv::default();
        let v = call("org.wolos.webapp.t", "update.install", "", &mut env);
        assert_eq!(v["err"]["code"], "denied", "{v}");
        assert!(env.requests.is_empty() && env.admin.lock().unwrap().sent.is_empty());
    }

    /// QA-ACC-6: with Settings > Accessibility > Timeouts at x5 the sheet stays open five times as
    /// long, so the wait for its answer is that much longer (wold reads the same file).
    #[test]
    fn a_patient_account_waits_longer_for_the_sheet() {
        use shared::shell::Answer;
        let waits = |text: Option<&str>| {
            let mut env = session();
            if let Some(t) = text {
                let f = env.home.join(shared::patience::FILE);
                env.files.push((f, t.into()));
            }
            env.replies = vec![wold(Answer::Done)];
            call("org.wolos.webapp.t", "update.install", "", &mut env);
            env.waits[0]
        };
        let normal = waits(None);
        assert_eq!(waits(Some("5\n")), normal + 200, "50 s -> 250 s");
        assert_eq!(
            waits(Some("never")),
            normal - shared::shell::APPROVAL_TIMEOUT_SECS + shared::patience::NEVER_SECS
        );
        assert_eq!(waits(Some("bogus")), normal);
    }

    const FIREWALL_OFF: &str = "systemctl show -p UnitFileState --value nftables.service";

    #[test]
    fn a_doctor_fix_is_the_checks_admin_requests_on_the_password_sheet() {
        use shared::shell::{Answer, ApprovalRequest};
        let mut env = session();
        env.captures = vec![(FIREWALL_OFF.into(), "disabled\n".into())];
        env.replies = vec![wold(Answer::Done)];
        let v = call(
            "org.wolos.webapp.doctor",
            "doctor.fix",
            r#"{"id":"security.firewall"}"#,
            &mut env,
        );
        // The answer is `wol doctor fix --json`'s shape, from the check after the fix.
        assert_eq!(v["ok"]["id"], "security.firewall", "{v}");
        assert_eq!(v["ok"]["ran"], true, "{v}");
        assert_eq!(v["ok"]["check"]["id"], "security.firewall", "{v}");
        let a = asked(&env);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].tool, "doctor_fix");
        assert_eq!(
            a[0].request,
            ApprovalRequest::DoctorFix {
                id: "security.firewall".into(),
                title: "Turn on the firewall".into(),
                steps: vec![vec!["firewall".into(), "on".into()]],
            }
        );
        assert!(
            env.admin.lock().unwrap().sent.is_empty() && env.execs.is_empty(),
            "the shell runs it, not wol"
        );
        // A check that does not exist asks nothing and is not found (QA-WEB-31).
        let mut env = session();
        let v = call(
            "org.wolos.webapp.doctor",
            "doctor.fix",
            r#"{"id":"security.nope"}"#,
            &mut env,
        );
        assert_eq!(v["err"]["code"], "not-found", "{v}");
        assert!(env.requests.is_empty());
    }

    /// A helper's or an app's `wol doctor fix` never sends a wol-admind request itself, even when
    /// the check turned into an administrator fix after its consent sheet.
    #[test]
    fn a_fix_run_for_an_app_sends_no_admin_request() {
        let firewall_off = || FakeEnv {
            captures: vec![(FIREWALL_OFF.into(), "disabled\n".into())],
            ..Default::default()
        };
        let mut env = firewall_off();
        let inv = cli::parse(&["doctor", "fix", "--json", "security.firewall"]).unwrap();
        let mut te = ToolEnv::new(&mut env, "webapp:org.wolos.webapp.doctor".into());
        let code = crate::run::dispatch(&inv, &mut te);
        assert_eq!(code, ErrorCode::Denied.exit());
        assert!(env.requests.is_empty() && env.admin.lock().unwrap().sent.is_empty());
        // A person at `wol` still gets wol-admind (the password on stdin).
        let mut env = firewall_off();
        crate::run::dispatch(&inv, &mut env);
        assert!(!env.requests.is_empty(), "wol-admind asked");
    }

    #[test]
    fn exit_statuses_map_back_to_codes() {
        assert_eq!(code_of(77), ErrorCode::Denied);
        assert_eq!(code_of(66), ErrorCode::NotFound);
        assert_eq!(code_of(1), ErrorCode::Failed);
        assert_eq!(code_of(99), ErrorCode::Failed);
    }
}
