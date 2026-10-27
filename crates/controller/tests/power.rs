//! `wol power`: status, apply (root), mode (wol-admind), battery idle steps, the suspend lock.
use controller::cli::{POWER_MODES, parse};
use controller::run::{FakeEnv, execute};
use std::path::PathBuf;
use wol_core::power::policy::Mode;

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

fn laptop(on_battery: bool) -> FakeEnv {
    let c = |a: &str, b: &str| (a.to_string(), b.to_string());
    FakeEnv {
        captures: vec![
            c("sysctl -n hw.ncpu", "2\n"),
            c("sysctl -n dev.hwpstate_intel.0.epp", "50\n"),
            c(
                "sysctl -n hw.acpi.acline",
                if on_battery { "0\n" } else { "1\n" },
            ),
            c("sysctl -n hw.acpi.battery.life", "41\n"),
            c("sysctl -n hw.acpi.cpu.cx_lowest", "C1\n"),
            c("sysctl -n hw.acpi.lid_switch_state", "NONE\n"),
            c("sysctl -n hw.acpi.supported_sleep_state", "S3 S4 S5\n"),
        ],
        ..FakeEnv::default()
    }
}

#[test]
fn power_modes_are_the_policy_modes() {
    assert_eq!(POWER_MODES, Mode::ALL.map(|m| m.name()));
}

#[test]
fn status_json() {
    let mut env = laptop(true);
    assert_eq!(run("power status --json", &mut env), 0);
    let v: serde_json::Value = serde_json::from_str(&env.out).unwrap();
    assert_eq!(v["source"], "battery");
    assert_eq!(v["mode"], "auto");
    assert_eq!(v["battery"], 41);
    assert_eq!(v["epp"], 75);
    assert_eq!(v["cx_lowest"], "Cmax");
    assert_eq!(v["lid"], "Nothing");
    assert_eq!(v["charge_limit"], serde_json::Value::Null);
    let mut env = laptop(false);
    env.tty = true;
    env.files.push((
        PathBuf::from("/usr/local/etc/wolos/power.json"),
        "{\"mode\":\"low\"}\n".into(),
    ));
    assert_eq!(run("power", &mut env), 0);
    assert!(
        env.out.contains("source: power adapter, battery 41%\n"),
        "{}",
        env.out
    );
    assert!(env.out.contains("mode: low\n"));
    assert!(env.out.contains("energy preference 85"));
}

#[test]
fn apply_runs_the_plan_as_root_only() {
    let mut env = laptop(true);
    assert_ne!(run("power apply", &mut env), 0);
    assert!(env.execs.is_empty());
    env.root = true;
    assert_eq!(run("power apply", &mut env), 0);
    let ran: Vec<String> = env.execs.iter().map(|a| a.join(" ")).collect();
    assert_eq!(
        ran,
        [
            "sysctl dev.hwpstate_intel.0.epp=75 dev.hwpstate_intel.1.epp=75 hw.acpi.cpu.cx_lowest=Cmax",
            "sysctl hw.acpi.lid_switch_state=S3",
        ]
    );
    assert!(
        env.out.contains("wolos_power: auto on battery\n"),
        "{}",
        env.out
    );
    // The user chose "Nothing" for the lid (wol-admind lid): kept.
    let mut env = laptop(false);
    env.root = true;
    env.files.push((
        PathBuf::from("/etc/sysctl.conf.local"),
        "hw.acpi.lid_switch_state=NONE\n".into(),
    ));
    env.exec_codes = vec![1];
    assert_eq!(run("power apply", &mut env), 0, "never fails boot or devd");
    assert_eq!(env.execs.len(), 1);
    assert!(env.out.contains("failed\n"));
}

#[test]
fn mode_goes_through_wol_admind() {
    let mut env = FakeEnv {
        replies: vec!["ok\n".into()],
        ..FakeEnv::default()
    };
    assert_eq!(run("power mode low", &mut env), 0);
    assert_eq!(env.requests[0].1, b"power-mode\x001\x00low\x00");
    assert!(parse(&["power", "mode", "turbo"]).is_err());
}

/// Sleep, Restart and Shut Down work from any account at the computer (wol-admind, no password);
/// root (rc scripts) runs them directly.
#[test]
fn session_power_goes_through_wol_admind() {
    for (line, wire) in [
        ("power sleep", &b"session-power\x001\x00sleep\x00"[..]),
        ("power restart", b"session-power\x001\x00restart\x00"),
        ("power shutdown", b"session-power\x001\x00shutdown\x00"),
    ] {
        let mut env = FakeEnv {
            replies: vec!["ok\n".into()],
            ..FakeEnv::default()
        };
        assert_eq!(run(line, &mut env), 0, "{line}");
        assert_eq!(env.requests[0].1, wire, "{line}");
        assert!(env.execs.is_empty());
    }
    let mut env = FakeEnv {
        root: true,
        ..FakeEnv::default()
    };
    assert_eq!(run("power shutdown", &mut env), 0);
    assert_eq!(env.execs[0].join(" "), "shutdown -p now");
}

#[test]
fn idle_steps_act_on_battery_only() {
    let mut env = laptop(false);
    assert_eq!(run("power idle display-off", &mut env), 0);
    assert_eq!(run("power idle sleep", &mut env), 0);
    assert!(env.execs.is_empty(), "AC: nothing");
    let mut env = laptop(true);
    env.replies = vec!["ok\n".into()];
    assert_eq!(run("power idle display-off", &mut env), 0);
    assert_eq!(run("power idle sleep", &mut env), 0);
    let ran: Vec<String> = env.execs.iter().map(|a| a.join(" ")).collect();
    assert_eq!(ran, ["hyprctl dispatch dpms off"]);
    assert_eq!(env.requests[0].1, b"session-power\x001\x00sleep\x00");
}

#[test]
fn suspend_locks_sessions_that_ask_for_the_password_after_sleep() {
    let f = |p: &str, t: &str| (PathBuf::from(p), t.to_string());
    let mut env = FakeEnv {
        root: true,
        files: vec![
            f(
                "/etc/passwd",
                "ann:*:1001:1001:Ann:/home/ann:/bin/sh\nbob:*:1002:1002:Bob:/home/bob:/bin/sh\n",
            ),
            f("/var/run/user/911/hypr/g/.socket.sock", ""),
            f("/var/run/user/1001/hypr/a/.socket.sock", ""),
            f("/var/run/user/1002/hypr/b/.socket.sock", ""),
            // Bob turned "require password after sleep" off (no before_sleep_cmd).
            f(
                "/home/bob/.config/hypr/hypridle.conf",
                &wol_core::prefs::hypridle_conf(300, false),
            ),
        ],
        replies: vec!["ok\n".into()],
        ..FakeEnv::default()
    };
    assert_eq!(run("power suspend", &mut env), 0);
    assert_eq!(env.requests.len(), 1, "{:?}", env.requests);
    assert_eq!(
        env.requests[0],
        (
            PathBuf::from("/var/run/user/1001/hypr/a/.socket.sock"),
            b"dispatch exec pgrep -x wol-lock || wol-lock".to_vec()
        )
    );
    assert!(env.out.contains("lock uid 1001: ok"), "{}", env.out);
    let mut user = FakeEnv::default();
    assert_ne!(run("power suspend", &mut user), 0);
}

#[test]
fn system_idle_config_has_the_battery_steps() {
    let f = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../desktop/overlay/usr/local/share/wolos/hypr/hypridle.conf");
    let conf = std::fs::read_to_string(f).unwrap();
    assert!(
        conf.ends_with(&wol_core::power::policy::hypridle_listeners()),
        "{conf}"
    );
    assert_eq!(wol_core::prefs::parse_hypridle(Some(&conf)), (300, true));
}
