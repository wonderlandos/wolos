//! `wol firewall on|off|status` and `wol security`: the firewall through wol-admind, and what
//! Privacy & Security shows (wol_core::security).
use crate::cli::Invocation;
use crate::run::{Env, admin_send, emit_json, fail};
use shared::admin::{AdminAction, AdminRequest};
use shared::errors::ErrorCode;
use std::cell::RefCell;

pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let json = inv.flag("json") || !env.stdout_tty();
    match inv.action {
        "firewall.on" | "firewall.off" => {
            let req = AdminRequest {
                action: AdminAction::Firewall,
                args: vec![inv.action["firewall.".len()..].into()],
            };
            admin_send(env, &req)
        }
        "firewall.status" | "security.status" => status(env, inv.action, json),
        other => fail(env, ErrorCode::Failed, &format!("no handler for {other}")),
    }
}

fn on_off(v: Option<bool>, unknown: &str) -> String {
    match v {
        Some(true) => "on".into(),
        Some(false) => "off".into(),
        None => unknown.into(),
    }
}

fn status(env: &mut dyn Env, action: &str, json: bool) -> i32 {
    let st = {
        let cell = RefCell::new(&mut *env);
        wol_core::security::status(&|argv: &[&str]| cell.borrow_mut().capture(argv))
    };
    if json {
        let v = if action == "firewall.status" {
            serde_json::json!({
                "on": st.firewall,
                "running": st.firewall_running,
                "rules": wol_core::sharing::PF_RULES,
            })
        } else {
            serde_json::to_value(&st).unwrap_or_default()
        };
        emit_json(env, &v);
        return 0;
    }
    let running = match st.firewall_running {
        Some(true) => ", filtering now",
        Some(false) => ", not filtering now",
        None => "",
    };
    let mut t = format!(
        "firewall: {}{running} (nothing comes in unasked)\n",
        on_off(st.firewall, "unknown")
    );
    if action == "security.status" {
        t.push_str(&format!(
            "remote login: {}\nsystem log on the network: {}\n",
            on_off(st.remote_login, "off"),
            if st.system_log_private { "no" } else { "yes" }
        ));
        for c in &st.hardening {
            let mark = if c.ok() { "ok" } else { "differs" };
            t.push_str(&format!(
                "{}: {} (want {}, {mark})\n",
                c.key,
                c.now.as_deref().unwrap_or("?"),
                c.want
            ));
        }
    }
    env.out(&t);
    0
}
