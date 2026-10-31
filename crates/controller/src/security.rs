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
        "security.scan" | "security.allow" => {
            let verb = &inv.action["security.".len()..];
            let argv = vec![
                "wol-xguard".to_string(),
                verb.to_string(),
                inv.get("path").unwrap_or_default().to_string(),
            ];
            match env.exec(&argv) {
                Ok(code) => code,
                Err(_) => fail(env, ErrorCode::Failed, "the malware check is not installed"),
            }
        }
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

/// One line for `wol security`; a line per file stopped.
fn malware_lines(s: &wol_core::malware::Status) -> String {
    let age = match s.signatures_age_days {
        Some(0) => "signatures from today".to_string(),
        Some(d) => format!("signatures {d} days old"),
        None => "no signatures".to_string(),
    };
    let mut t = format!("malware check: {}, {age}\n", on_off(s.on, "unknown"));
    for b in &s.blocked {
        t.push_str(&format!("stopped: {} ({})\n", b.path, b.name));
    }
    t
}

fn status(env: &mut dyn Env, action: &str, json: bool) -> i32 {
    let (st, check) = {
        let (now, uid) = (env.now_secs(), env.capture(&["id", "-u"]));
        let uid = uid.and_then(|u| u.trim().parse().ok()).unwrap_or(u32::MAX);
        let cell = RefCell::new(&mut *env);
        let probe = |argv: &[&str]| cell.borrow_mut().capture(argv);
        let st = wol_core::security::status(&probe);
        let read = |p: &str| cell.borrow().read_file(std::path::Path::new(p));
        (st, wol_core::malware::status(&probe, &read, uid, now))
    };
    if json {
        let v = if action == "firewall.status" {
            serde_json::json!({
                "on": st.firewall,
                "running": st.firewall_running,
                "rules": wol_core::sharing::FIREWALL_RULES,
            })
        } else {
            let mut v = serde_json::to_value(&st).unwrap_or_default();
            v["malware"] = serde_json::to_value(&check).unwrap_or_default();
            v
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
        t.push_str(&malware_lines(&check));
    }
    env.out(&t);
    0
}
