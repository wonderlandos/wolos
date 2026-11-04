// SPDX-License-Identifier: GPL-2.0-or-later
//! The Doctor never answers with nothing (QA-100 WEB-24, H3-2; journey qa100-doctor-failure-report):
//! when a check crashes, when a fix does not work, when wol-admind refuses, when the id is wrong,
//! and when the web app asks while a check fails, the answer is a report: what was checked, what
//! failed, the error text and what to do, in the text, in `--json` (with the exit status) and in
//! the reply a web app gets.
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
    serde_json::from_str(&env.out).unwrap_or_else(|e| panic!("{e}: {:?}", env.out))
}

const FIREWALL: &str = "systemctl show -p UnitFileState --value nftables.service";

fn firewall_off() -> FakeEnv {
    FakeEnv {
        captures: vec![(FIREWALL.into(), "disabled\n".into())],
        vars: vec![(
            "WOL_ADMIN_SOCKET".into(),
            PathBuf::from("/run/admin.sock").display().to_string(),
        )],
        ..Default::default()
    }
}

fn row<'a>(v: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no row {id} in {v}"))
}

/// A check that crashes used to take `wol doctor` down with it: no report at all, for any area.
/// Now the area is one failed row that says what was being checked and what the error was; every
/// other area still reports; the exit status says "not ready".
#[test]
fn a_check_that_crashes_is_a_failed_row_and_the_other_areas_still_report() {
    let mut env = FakeEnv {
        crash: vec![FIREWALL.into()],
        ..Default::default()
    };
    let code = run("doctor --json", &mut env);
    let v = json(&env);
    assert_eq!(code, ErrorCode::Failed.exit());
    assert_eq!(v["ready"], false, "{v}");
    let r = row(&v, "security.check");
    assert_eq!(
        (r["status"].as_str(), r["area"].as_str()),
        (Some("fail"), Some("security"))
    );
    assert!(
        r["error"].as_str().unwrap().contains("crashed"),
        "the error text: {r}"
    );
    let detail = r["detail"].as_str().unwrap();
    assert!(
        detail.contains("Security") && detail.contains("Check again"),
        "what was checked and what to do: {detail}"
    );
    for a in AREAS.iter().filter(|a| **a != "security") {
        assert!(
            v["checks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["area"] == *a),
            "area {a} is still in the report"
        );
    }
}

#[test]
fn a_check_that_crashes_is_in_the_text_report_too() {
    let mut env = FakeEnv {
        tty: true,
        crash: vec![FIREWALL.into()],
        ..Default::default()
    };
    let code = run("doctor security", &mut env);
    assert_eq!(code, ErrorCode::Failed.exit());
    assert!(env.out.contains("FAIL"), "{}", env.out);
    assert!(env.out.contains("Security"), "{}", env.out);
    assert!(
        env.out.contains("crashed"),
        "the error text is on the screen: {}",
        env.out
    );
    assert!(env.out.contains("1 problem to fix."), "{}", env.out);
}

/// The Doctor app asks `doctor.check` through the bridge; wol exits 1 when a check fails, and the
/// bridge turned that into "It did not work." with the report dropped (QA-WEB-24): the app showed
/// its error state exactly when the computer had a problem.
#[test]
fn the_app_gets_the_report_when_a_check_fails() {
    let mut env = firewall_off();
    let v = controller::webapp::call("org.wolos.webapp.doctor", "doctor.check", "{}", &mut env);
    assert!(v.get("err").is_none(), "an answer, not a refusal: {v}");
    assert_eq!(v["ok"]["ready"], false, "{v}");
    assert_eq!(row(&v["ok"], "security.firewall")["status"], "fail");
}

#[test]
fn the_app_gets_the_report_when_a_check_crashes() {
    let mut env = FakeEnv {
        crash: vec![FIREWALL.into()],
        ..Default::default()
    };
    let v = controller::webapp::call("org.wolos.webapp.doctor", "doctor.check", "{}", &mut env);
    assert_eq!(row(&v["ok"], "security.check")["status"], "fail", "{v}");
}

/// A fix that did not work: the JSON says what failed, the error text wol-admind gave, and what to
/// do. It used to be `{"ran":true,"ok":false,"check":..}` and nothing else.
#[test]
fn a_fix_that_wol_admind_refused_says_why_in_json() {
    let mut env = firewall_off();
    env.replies = vec!["failed The firewall service would not start.\n".into()];
    let code = run("doctor fix security.firewall --json", &mut env);
    assert_ne!(code, 0);
    let r = json(&env);
    assert_eq!(
        (r["ran"].as_bool(), r["ok"].as_bool()),
        (Some(true), Some(false)),
        "{r}"
    );
    assert_eq!(r["error"], "The firewall service would not start.", "{r}");
    let message = r["message"].as_str().unwrap();
    assert!(
        message.contains("did not work") && message.contains("would not start"),
        "{message}"
    );
    assert!(
        r["next"]
            .as_str()
            .is_some_and(|n| n.contains("Check again")),
        "what to do: {r}"
    );
    assert_eq!(
        r["check"]["id"], "security.firewall",
        "the row as it is now: {r}"
    );
    assert_eq!(r["exit"].as_i64(), Some(i64::from(code)), "{r}");
}

#[test]
fn a_fix_when_wol_admind_is_not_running_says_so_in_json() {
    let mut env = firewall_off(); // no reply queued: the socket refuses
    let code = run("doctor fix security.firewall --json", &mut env);
    assert_eq!(code, ErrorCode::Unavailable.exit());
    let r = json(&env);
    assert!(
        r["error"]
            .as_str()
            .unwrap()
            .contains("administrator service"),
        "{r}"
    );
    assert_eq!(r["ok"], false);
}

#[test]
fn a_fix_that_fails_in_the_text_report_says_why_and_what_to_do() {
    let mut env = firewall_off();
    env.tty = true;
    env.replies = vec!["failed The firewall service would not start.\n".into()];
    let code = run("doctor fix security.firewall", &mut env);
    assert_ne!(code, 0);
    assert!(env.err.contains("would not start"), "{}", env.err);
    assert!(env.err.contains("is not fixed"), "{}", env.err);
    assert!(
        env.err.contains("Check again") || env.err.contains("wol doctor security"),
        "{}",
        env.err
    );
}

/// A fix that runs as the person and exits non-zero keeps its output as the error text.
#[test]
fn a_user_fix_that_fails_keeps_its_output_as_the_error() {
    let mut env = FakeEnv {
        exec_codes: vec![1],
        captures: vec![(
            "systemctl --user start pipewire.service pipewire-pulse.service".into(),
            "Failed to start pipewire.service: Unit not found.\n".into(),
        )],
        ..Default::default()
    };
    let code = run("doctor fix sound.pipewire --json", &mut env);
    assert_ne!(code, 0);
    let r = json(&env);
    assert_eq!(r["ok"], false, "{r}");
    assert!(
        r["error"].as_str().unwrap().contains("Unit not found"),
        "{r}"
    );
    assert!(
        r["message"].as_str().unwrap().contains("did not work"),
        "{r}"
    );
}

/// Never an empty answer for a bad id either: `--json` prints the report on stdout as well.
#[test]
fn an_unknown_check_id_is_a_json_answer_too() {
    let mut env = FakeEnv::default();
    let code = run("doctor fix security.nope --json", &mut env);
    assert_eq!(code, ErrorCode::NotFound.exit());
    let r = json(&env);
    assert_eq!(
        (r["ran"].as_bool(), r["ok"].as_bool()),
        (Some(false), Some(false)),
        "{r}"
    );
    assert!(
        r["error"].as_str().unwrap().contains("security.nope"),
        "{r}"
    );
    assert!(r["next"].is_string(), "{r}");
    assert!(
        env.err.contains("security.nope"),
        "stderr keeps the sentence: {}",
        env.err
    );
}
