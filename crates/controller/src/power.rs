// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol power`: status, apply (udev on AC changes, systemd `wolos-power` at boot and resume), mode,
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
        (policy::Cpu::Epp, Some(e)) => format!("power profiles, energy preference {e}"),
        _ => "fixed (nothing to steer)".into(),
    };
    let mut t = format!("source: {source}\nmode: {}\ncpu: {cpu}\n", st.mode.name());
    if let Some(l) = st.lid {
        t.push_str(&format!("lid: {}\n", wol_core::power::lid_value(l)));
    }
    t.push_str(&format!(
        "sleep: {}\n",
        if st.sleep { "suspend" } else { "not supported" }
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

/// Root: the plan for the saved mode and the power source now. Never fails boot or udev: problems
/// are printed, the exit status is 0.
fn apply(env: &mut dyn Env) -> i32 {
    if !env.is_root() {
        env.err("wol: power apply needs root (wol power mode changes the mode)\n");
        return ErrorCode::Denied.exit();
    }
    let f = facts(env);
    let mode = state(env).mode;
    let argv = policy::plan(&f, mode).argv();
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

/// Sleep, restart or shut down: root runs it, everyone else asks wol-admind (no password for
/// the person at the computer). Sleep locks every session first (system-sleep hook: `power suspend`).
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

/// How long the sleep hook waits for one session shell to take its lock (docs/modules/power.md).
pub const LOCK_WAIT_SECS: u32 = 3;

/// Root, from the systemd-sleep hook (`pre`) before the kernel sleeps: every desktop session that
/// asks for the password after sleep (Settings > Lock Screen) gets its lock now. The hook then
/// waits before it lets the kernel continue, so the lock is drawn first.
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
        // QA-FILES-8: a foreign or hand-edited hypridle.conf still locks; only Settings' own
        // "no password after sleep" skips the lock.
        if !wol_core::prefs::lock_on_sleep(conf.as_deref()) {
            continue;
        }
        // Audit S4-21: a hung shell costs the sleep LOCK_WAIT_SECS, not the generic 30 s.
        let r = env.request_wait(Path::new(&s.socket), policy::LOCK_REQUEST, LOCK_WAIT_SECS);
        env.out(&format!(
            "wolos_power: lock uid {}: {}\n",
            s.uid,
            r.as_deref().map_or("no answer", str::trim)
        ));
    }
    0
}

#[cfg(test)]
mod tests {
    use crate::run::FakeEnv;
    use std::path::PathBuf;

    /// QA-FILES-8: an account whose hypridle.conf Settings did not write (hand-edited, another
    /// locker's, empty) is locked before sleep like everyone else; Settings' "no password after
    /// sleep" is the one way out.
    #[test]
    fn foreign_hypridle_still_locks() {
        let f = |p: &str, t: &str| (PathBuf::from(p), t.to_string());
        let mut env = FakeEnv {
            root: true,
            files: vec![
                f(
                    "/etc/passwd",
                    "ann:*:1001:1001:Ann:/home/ann:/bin/sh\nbob:*:1002:1002:Bob:/home/bob:/bin/sh\n",
                ),
                f("/run/user/1001/hypr/a/.socket.sock", ""),
                f("/run/user/1002/hypr/b/.socket.sock", ""),
                f(
                    "/home/ann/.config/hypr/hypridle.conf",
                    "general {\n    lock_cmd = hyprlock\n}\nlistener {\n    timeout = 120\n    on-timeout = hyprlock\n}\n",
                ),
                f(
                    "/home/bob/.config/hypr/hypridle.conf",
                    &wol_core::prefs::hypridle_conf(300, false),
                ),
            ],
            replies: vec!["ok\n".into(), "ok\n".into()],
            ..FakeEnv::default()
        };
        assert_eq!(super::suspend(&mut env), 0);
        let asked: Vec<&PathBuf> = env.requests.iter().map(|(p, _)| p).collect();
        assert_eq!(
            asked,
            [&PathBuf::from("/run/user/1001/hypr/a/.socket.sock")],
            "{}",
            env.out
        );
    }
}
