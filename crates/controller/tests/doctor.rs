// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol doctor`: one list of plain-word checks over every area, and `wol doctor fix <id>` through
//! the path the owning verb uses (wol-admind for root changes, the person's own commands else).
use controller::cli::parse;
use controller::doctor::AREAS;
use controller::run::{FakeEnv, execute};
use shared::errors::ErrorCode;
use std::path::PathBuf;

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

fn json(env: &FakeEnv) -> serde_json::Value {
    serde_json::from_str(&env.out).unwrap_or_else(|e| panic!("{e}: {}", env.out))
}

const FIREWALL_OFF: &str = "systemctl show -p UnitFileState --value nftables.service";

#[test]
fn parses_area_and_fix() {
    let i = parse(&["doctor"]).unwrap();
    assert_eq!((i.action, i.get("area")), ("doctor.check", None));
    let i = parse(&["doctor", "sound", "--json"]).unwrap();
    assert_eq!((i.action, i.get("area")), ("doctor.check", Some("sound")));
    let i = parse(&["doctor", "fix", "security.firewall"]).unwrap();
    assert_eq!(
        (i.action, i.get("id")),
        ("doctor.fix", Some("security.firewall"))
    );
    assert!(parse(&["doctor", "kitchen"]).is_err());
}

#[test]
fn every_area_reports_checks_with_stable_ids() {
    let mut env = FakeEnv::default();
    run("doctor --json", &mut env);
    let v = json(&env);
    assert_eq!(v["checked"], 1000);
    let checks = v["checks"].as_array().unwrap();
    for a in AREAS {
        assert!(
            checks.iter().any(|c| c["area"] == *a),
            "no check in area {a}"
        );
    }
    let mut ids: Vec<&str> = checks.iter().map(|c| c["id"].as_str().unwrap()).collect();
    for c in checks {
        let (id, area) = (c["id"].as_str().unwrap(), c["area"].as_str().unwrap());
        assert!(id.starts_with(&format!("{area}.")), "{id}");
        assert!(
            ["ok", "warn", "fail", "skipped"].contains(&c["status"].as_str().unwrap()),
            "{c}"
        );
        assert!(!c["title"].as_str().unwrap().is_empty(), "{c}");
        if let Some(f) = c.get("fix") {
            assert!(f["title"].is_string() && f["command"].is_string(), "{c}");
            assert!(f["needs_admin"].is_boolean(), "{c}");
        }
    }
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n, "ids unique");
}

/// UX-LESSONS: titles and details are for a person; commands live only in `fix.command`.
#[test]
fn words_on_screen_carry_no_commands() {
    let mut env = FakeEnv {
        captures: vec![(FIREWALL_OFF.into(), "disabled\n".into())],
        ..Default::default()
    };
    run("doctor --json", &mut env);
    for c in json(&env)["checks"].as_array().unwrap() {
        for k in ["title", "detail"] {
            let t = c[k].as_str().unwrap();
            for bad in [
                "systemctl",
                "pacman",
                "wpctl",
                "sudo",
                "/usr",
                "/etc",
                "/dev",
                "wol ",
                "--",
                "probably",
                "maybe",
                "might",
            ] {
                assert!(!t.contains(bad), "{} {k}: {t:?} has {bad:?}", c["id"]);
            }
        }
    }
}

#[test]
fn one_area_only() {
    let mut env = FakeEnv::default();
    run("doctor security --json", &mut env);
    let v = json(&env);
    let checks = v["checks"].as_array().unwrap();
    assert!(!checks.is_empty());
    assert!(checks.iter().all(|c| c["area"] == "security"));
}

#[test]
fn firewall_fix_goes_through_admind_with_the_password_on_stdin() {
    let sock = PathBuf::from("/run/admin.sock");
    let mut env = FakeEnv {
        captures: vec![(FIREWALL_OFF.into(), "disabled\n".into())],
        vars: vec![("WOL_ADMIN_SOCKET".into(), sock.display().to_string())],
        replies: vec!["auth\n".into(), "ok\n".into()],
        stdin: vec!["s3cret-pw".into()],
        ..Default::default()
    };
    run("doctor security --json", &mut env);
    let v = json(&env);
    let fw = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "security.firewall")
        .unwrap()
        .clone();
    assert_eq!(fw["status"], "fail");
    assert_eq!(fw["fix"]["needs_admin"], true);
    assert_eq!(fw["fix"]["command"], "wol admin firewall on");
    env.out.clear();
    // The fake firewall stays off: the request went out, the check after says so.
    let code = run("doctor fix security.firewall --json", &mut env);
    let sent: Vec<String> = env
        .requests
        .iter()
        .map(|r| String::from_utf8_lossy(&r.1).into_owned())
        .collect();
    assert_eq!(
        sent,
        ["firewall\x001\0on\0", "firewall\x001\0on\0s3cret-pw"]
    );
    assert!(env.execs.is_empty(), "root changes only through wol-admind");
    let r = json(&env);
    assert_eq!(
        (r["id"].as_str(), r["ran"].as_bool()),
        (Some("security.firewall"), Some(true))
    );
    assert_eq!(r["ok"], false);
    assert_eq!(r["check"]["status"], "fail");
    assert_eq!(code, ErrorCode::Failed.exit());
}

#[test]
fn sound_service_fix_runs_as_the_person() {
    let mut env = FakeEnv::default();
    run("doctor fix sound.pipewire --json", &mut env);
    assert_eq!(
        env.execs,
        vec![
            [
                "systemctl",
                "--user",
                "start",
                "pipewire.service",
                "pipewire-pulse.service"
            ]
            .map(String::from)
            .to_vec()
        ]
    );
    assert!(
        env.requests.is_empty(),
        "no administrator for the person's own services"
    );
    assert_eq!(json(&env)["ran"], true);
}

#[test]
fn nothing_to_run_and_unknown_ids() {
    let mut env = FakeEnv::default();
    // Nobody signs in from afar here: ok, nothing runs.
    assert_eq!(run("doctor fix security.remote-login --json", &mut env), 0);
    let r = json(&env);
    assert_eq!(
        (r["ran"].as_bool(), r["ok"].as_bool()),
        (Some(false), Some(true))
    );
    assert!(env.execs.is_empty() && env.requests.is_empty());
    let mut env = FakeEnv::default();
    assert_eq!(
        run("doctor fix kitchen.sink", &mut env),
        ErrorCode::NotFound.exit()
    );
    assert_eq!(
        run("doctor fix security.nope", &mut env),
        ErrorCode::NotFound.exit()
    );
}

#[test]
fn text_report_names_the_fix_verb() {
    let mut env = FakeEnv {
        tty: true,
        captures: vec![(FIREWALL_OFF.into(), "disabled\n".into())],
        ..Default::default()
    };
    run("doctor security", &mut env);
    assert!(
        env.out
            .contains("fix: wol doctor fix security.firewall (Turn on the firewall)"),
        "{}",
        env.out
    );
    assert!(env.out.contains("to fix.\n"), "{}", env.out);
}
