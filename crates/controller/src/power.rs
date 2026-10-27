//! `wol power`: status, apply (devd on AC changes, rc.d `wolos_power` at boot and resume), mode,
//! sleep, and the suspend hook that locks every session first. Policy in `wol_core::power::policy`.
use crate::cli::Invocation;
use crate::run::{Env, admin_send, emit_json, fail};
use shared::admin::{AdminAction, AdminRequest};
use shared::errors::ErrorCode;
use std::cell::RefCell;
use std::path::Path;
use wol_core::power::policy::{self, Facts, Idle, Mode, State};

pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let json = inv.flag("json") || !env.stdout_tty();
    match inv.action {
        "power.status" => status(env, json),
        "power.apply" => apply(env),
        "power.mode" => {
            let Some(m) = inv.get("mode").and_then(Mode::parse) else {
                return fail(env, ErrorCode::Usage, "mode: auto, low or high");
            };
            let req = AdminRequest {
                action: AdminAction::PowerMode,
                args: vec![m.name().into()],
            };
            admin_send(env, &req)
        }
        "power.sleep" => session_power(env, "sleep"),
        "power.restart" => session_power(env, "restart"),
        "power.shutdown" => session_power(env, "shutdown"),
        "power.idle" => match inv.get("step").and_then(Idle::parse) {
            Some(i) => idle(env, i),
            None => fail(env, ErrorCode::Usage, "step: display-off or sleep"),
        },
        "power.suspend" => suspend(env),
        other => fail(env, ErrorCode::Failed, &format!("no handler for {other}")),
    }
}

/// Read-only probes through the environment.
fn facts(env: &mut dyn Env) -> Facts {
    let cell = RefCell::new(env);
    policy::facts(&|argv: &[&str]| cell.borrow_mut().capture(argv))
}

fn state(env: &dyn Env) -> State {
    State::parse(env.read_file(Path::new(policy::STATE_FILE)).as_deref())
}

fn status(env: &mut dyn Env, json: bool) -> i32 {
    let st = policy::status(&facts(env), &state(env));
    if json {
        emit_json(env, &serde_json::to_value(&st).unwrap_or_default());
        return 0;
    }
    let source = match (st.source, st.battery) {
        (policy::Source::Battery, Some(b)) => format!("battery, {b}%"),
        (policy::Source::Battery, None) => "battery".into(),
        (policy::Source::Ac, Some(b)) => format!("power adapter, battery {b}%"),
        (policy::Source::Ac, None) => "power adapter".into(),
    };
    let cpu = match (st.cpu, st.epp) {
        (policy::Cpu::Epp, Some(e)) => format!("Speed Shift, energy preference {e}"),
        (policy::Cpu::Powerd, _) => "powerd".into(),
        _ => "fixed (nothing to steer)".into(),
    };
    let mut t = format!("source: {source}\nmode: {}\ncpu: {cpu}\n", st.mode.name());
    if let Some(c) = &st.cx_lowest {
        t.push_str(&format!("idle states: down to {c}\n"));
    }
    if let Some(l) = st.lid {
        t.push_str(&format!("lid: {}\n", wol_core::power::lid_value(l)));
    }
    t.push_str(&format!(
        "sleep: {}\n",
        if st.sleep { "S3" } else { "not supported" }
    ));
    if !st.wifi_power_save.is_empty() {
        t.push_str(&format!(
            "wifi power save: {}\n",
            st.wifi_power_save.join(", ")
        ));
    }
    env.out(&t);
    0
}

/// Root: the plan for the saved mode and the power source now; the laptop lid default. Never fails
/// boot or devd: problems are printed, the exit status is 0.
fn apply(env: &mut dyn Env) -> i32 {
    if !env.is_root() {
        env.err("wol: power apply needs root (wol power mode changes the mode)\n");
        return ErrorCode::Denied.exit();
    }
    let f = facts(env);
    let mode = state(env).mode;
    let mut argv = policy::plan(&f, mode).argv();
    let local = env.read_file(Path::new(wol_core::power::SYSCTL_LOCAL));
    if let Some((k, v)) = policy::lid_default(&f, local.as_deref()) {
        argv.push(vec!["sysctl".into(), format!("{k}={v}")]);
    }
    for a in &argv {
        match env.exec(a) {
            Ok(0) => env.out(&format!("wolos_power: {}\n", a.join(" "))),
            _ => env.out(&format!("wolos_power: {} failed\n", a.join(" "))),
        }
    }
    env.out(&format!(
        "wolos_power: {} on {}\n",
        mode.name(),
        match f.source {
            policy::Source::Ac => "AC",
            policy::Source::Battery => "battery",
        }
    ));
    0
}

/// Sleep now (S3). Accounts in group operator may (`/dev/acpi` is root:operator 0664); the lock is
/// on screen before the machine sleeps (rc.suspend -> `wolos_power suspend`).
/// Sleep, restart or shut down: root runs it, everyone else asks wol-admind (no password for
/// the person at the computer). Sleep locks every session first (rc.suspend: `power suspend`).
fn session_power(env: &mut dyn Env, verb: &str) -> i32 {
    if env.is_root() {
        let argv = wol_core::admin::session_power_argv(verb).unwrap_or_default();
        let argv: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
        return crate::run::exec(env, &argv);
    }
    let req = AdminRequest {
        action: AdminAction::SessionPower,
        args: vec![verb.into()],
    };
    admin_send(env, &req)
}

/// hypridle battery listeners: display off, then sleep, on battery only.
fn idle(env: &mut dyn Env, i: Idle) -> i32 {
    let f = facts(env);
    if !policy::idle_acts(i, f.source, f.sleep) {
        return 0;
    }
    match i {
        Idle::DisplayOff => crate::run::exec(
            env,
            &[
                "hyprctl".into(),
                "dispatch".into(),
                "dpms".into(),
                "off".into(),
            ],
        ),
        Idle::Sleep => session_power(env, "sleep"),
    }
}

/// Root, from rc.suspend (KEYWORD suspend) before the kernel sleeps: every desktop session that
/// asks for the password after sleep (Settings > Lock Screen) gets its lock now. rc.suspend then
/// syncs and waits 3 s before it lets the kernel continue, so the lock is drawn first.
fn suspend(env: &mut dyn Env) -> i32 {
    if !env.is_root() {
        env.err("wol: power suspend needs root\n");
        return ErrorCode::Denied.exit();
    }
    let passwd = env.read_file(Path::new("/etc/passwd")).unwrap_or_default();
    for s in policy::sessions(&|p| env.list_dir(Path::new(p))) {
        let home = policy::home_of(&passwd, s.uid);
        let conf = home.and_then(|h| {
            env.read_file(
                &Path::new(&h)
                    .join(".config")
                    .join(wol_core::prefs::IDLE_FILE),
            )
        });
        if !wol_core::prefs::parse_hypridle(conf.as_deref()).1 {
            continue;
        }
        let r = env.request(Path::new(&s.socket), policy::LOCK_REQUEST);
        env.out(&format!(
            "wolos_power: lock uid {}: {}\n",
            s.uid,
            r.as_deref().map_or("no answer", str::trim)
        ));
    }
    0
}
