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
//! and the password, in wold and the shell). A verb the gate has no request for is refused, never
//! run without a password.
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
    let Some(cmd) = TREE.iter().find(|c| c.action == method) else {
        return fault(
            ErrorCode::NotFound,
            "This app asked for something WolOS does not offer.",
            &format!("no verb has the action {method:?}"),
        );
    };
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
        match approve::decide(env, app, &verb.tool(), rule, &inv, secret) {
            Verdict::Done(what) => return json!({"ok": {"status": "done", "request": what}}),
            Verdict::Stop(_, msg) => return fault(ErrorCode::Denied, &msg, ""),
            // Cannot happen for an Admin rule (decide hands it to the shell), but never run it here.
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

    #[test]
    fn an_administrator_verb_without_an_approval_request_is_not_run() {
        let mut env = FakeEnv::default();
        let v = call(
            "org.wolos.webapp.t",
            "update.apply",
            r#"{"be":"wolos-b","release":"2026/11/02"}"#,
            &mut env,
        );
        assert_eq!(v["err"]["code"], "unavailable", "{v}");
        assert!(
            env.execs.is_empty() && env.requests.is_empty(),
            "nothing ran or was asked"
        );
    }

    #[test]
    fn exit_statuses_map_back_to_codes() {
        assert_eq!(code_of(77), ErrorCode::Denied);
        assert_eq!(code_of(66), ErrorCode::NotFound);
        assert_eq!(code_of(1), ErrorCode::Failed);
        assert_eq!(code_of(99), ErrorCode::Failed);
    }
}
