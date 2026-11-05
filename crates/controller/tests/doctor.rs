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

// ---- QA-DOC-4/5/2 (the Doctor's setup round): what a fix can ask, what is not a problem ----

use shared::hardware::{Device, HardwareSnapshot, Kind, State};

fn missing(kind: Kind, name: &str, packages: &[&str]) -> Device {
    Device {
        kind,
        name: name.into(),
        state: State::NeedsFirmware,
        missing_packages: packages.iter().map(|p| p.to_string()).collect(),
        ..Default::default()
    }
}

/// The checks of one area, as the list prints them.
fn area(env: &mut FakeEnv, area: &str) -> Vec<serde_json::Value> {
    env.out.clear();
    run(&format!("doctor {area} --json"), env);
    json(env)["checks"].as_array().unwrap().clone()
}

/// QA-DOC-4: a laptop missing Wi-Fi, Bluetooth and Ethernet firmware needed nine requests and the
/// sheet takes eight: "this fix cannot be asked on screen". The fix is cut to what one sheet can
/// ask and the detail says the rest comes after "Check again".
#[test]
fn a_hardware_fix_of_many_packages_fits_the_sheet() {
    let pk = |n: usize, p: &str| (0..n).map(|i| format!("{p}-{i}")).collect::<Vec<_>>();
    let mut devs = Vec::new();
    for (kind, name, p) in [
        (Kind::Wifi, "Wi-Fi", "wifi-fw"),
        (Kind::Bluetooth, "Bluetooth", "bt-fw"),
        (Kind::Ethernet, "Ethernet", "eth-fw"),
    ] {
        let list = pk(3, p);
        devs.push(missing(
            kind,
            name,
            &list.iter().map(String::as_str).collect::<Vec<_>>(),
        ));
    }
    let mut env = FakeEnv {
        hw: vec![HardwareSnapshot { devices: devs }],
        ..Default::default()
    };
    let c = area(&mut env, "network")
        .into_iter()
        .find(|c| c["id"] == "network.devices")
        .unwrap();
    assert_eq!(c["status"], "fail");
    assert!(
        c["detail"].as_str().unwrap().contains("Check again"),
        "the rest is promised: {c}"
    );
    env.hw = vec![HardwareSnapshot {
        devices: (0..3)
            .map(|i| {
                let list = pk(3, &format!("fw{i}"));
                missing(
                    [Kind::Wifi, Kind::Bluetooth, Kind::Ethernet][i],
                    "x",
                    &list.iter().map(String::as_str).collect::<Vec<_>>(),
                )
            })
            .collect(),
    }];
    let Ok(Some(shared::shell::ApprovalRequest::DoctorFix { steps, .. })) =
        controller::doctor::approval(&mut env, "network.devices")
    else {
        panic!("the fix is askable on screen")
    };
    assert_eq!(steps.len(), shared::shell::MAX_DOCTOR_STEPS, "{steps:?}");
}

/// QA-DOC-5: a virtual machine or a desktop with no sleep state is not a problem to fix, a fresh
/// install has not looked for updates yet, and a camera is not a sound device.
#[test]
fn doctor_does_not_warn_about_how_a_machine_is_built() {
    let mut env = FakeEnv {
        hw: vec![HardwareSnapshot {
            devices: vec![
                missing(Kind::Webcam, "Integrated Camera", &["cam-fw"]),
                Device {
                    kind: Kind::Audio,
                    name: "Speakers".into(),
                    ..Default::default()
                },
            ],
        }],
        captures: vec![("systemd-detect-virt --vm".into(), "kvm\n".into())],
        ..Default::default()
    };
    let power = area(&mut env, "power");
    let sleep = power.iter().find(|c| c["id"] == "power.sleep").unwrap();
    assert_ne!(sleep["status"], "warn", "{sleep}");
    let updates = area(&mut env, "updates");
    let looked = updates
        .iter()
        .find(|c| c["id"] == "updates.checked")
        .unwrap();
    assert_ne!(
        looked["status"], "warn",
        "the first daily look has not happened yet: {looked}"
    );
    let sound = area(&mut env, "sound");
    let dev = sound.iter().find(|c| c["id"] == "sound.devices").unwrap();
    assert_ne!(
        dev["status"], "fail",
        "a camera needing firmware is not a sound problem: {dev}"
    );
    let display = area(&mut env, "display");
    assert!(
        display
            .iter()
            .any(|c| c["id"] == "display.cameras" && c["status"] == "fail"),
        "the camera has its own row under the screens: {display:?}"
    );
}

/// QA-PWR-16: a list printed into a pipe (stdout is not a terminal) is JSON, as `--json` is; at a
/// terminal it is the table.
#[test]
fn theme_list_is_json_when_piped_and_a_table_at_a_terminal() {
    let mut piped = FakeEnv::default();
    assert!(!piped.tty);
    run("theme list", &mut piped);
    let v: serde_json::Value = serde_json::from_str(&piped.out).expect(&piped.out);
    assert!(v.as_array().is_some_and(|a| !a.is_empty()), "{v}");
    assert!(v[0]["id"].is_string() && v[0]["variant"].is_string(), "{v}");
    let mut term = FakeEnv {
        tty: true,
        ..Default::default()
    };
    run("theme list", &mut term);
    assert!(
        term.out.contains("wol-dark") && !term.out.trim_start().starts_with('['),
        "{}",
        term.out
    );
}

/// ADR 0055: plain `wol doctor` (and so the Doctor app) shows a system whose TLS policy is not
/// applied, from the files alone; the network probes run only for `wol doctor tls`.
#[test]
fn a_classical_system_shows_in_plain_doctor_and_probes_only_when_asked() {
    let mut env = FakeEnv::default();
    run("doctor --json", &mut env);
    let v = json(&env);
    let checks = v["checks"].as_array().unwrap();
    let p = checks
        .iter()
        .find(|c| c["id"] == "security.tls-policy")
        .expect("security.tls-policy in plain doctor");
    assert_eq!(
        p["status"], "fail",
        "no policy lines in the fake's files: {p}"
    );
    assert!(checks.iter().any(|c| c["id"] == "security.browser-tls"));
    assert!(
        !checks.iter().any(|c| c["area"] == "tls"),
        "no network probe in plain doctor"
    );
    assert!(
        env.captured_argvs
            .iter()
            .all(|a| a[0] != "curl" && a[0] != "gnutls-cli")
    );
    // Asked by name: the probes, and only them.
    let mut env = FakeEnv::default();
    run("doctor tls --json", &mut env);
    let ids: Vec<String> = json(&env)["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, ["tls.openssl", "tls.gnutls", "tls.wolos"]);

/// `memory.saver` (ADR 0061): fault-around, the sharing rule and why, and the page-out state of
/// the apps with a budget, in plain words; a second account with sharing still on is worth a look.
#[test]
fn memory_saver_reports_fault_around_sharing_and_the_page_out() {
    const ONE: &str = "root:x:0:0::/root:/bin/bash\nalice:x:1000:1000::/home/alice:/bin/zsh\n";
    const TWO: &str = "root:x:0:0::/root:/bin/bash\nalice:x:1000:1000::/home/alice:/bin/zsh\nbob:x:1001:1001::/home/bob:/bin/zsh\n";
    let status = concat!(
        r#"{"enabled":true,"saved":0,"apps":["#,
        r#"{"app":"discord","state":"saving","saved":1,"memory":2,"swap":0,"pageouts":3},"#,
        r#"{"app":"code","state":"saving","saved":1,"memory":2,"swap":0}]}"#
    );
    let files = |passwd: &str, run: &str| -> Vec<(PathBuf, String)> {
        vec![
            (
                "/sys/kernel/debug/fault_around_bytes".into(),
                "4096\n".into(),
            ),
            ("/sys/kernel/mm/ksm/run".into(), run.into()),
            ("/sys/kernel/mm/ksm/pages_sharing".into(), "5000\n".into()),
            ("/etc/passwd".into(), passwd.into()),
            ("/var/run/wol-memsaver.json".into(), status.into()),
        ]
    };
    let saver = |env: &mut FakeEnv| {
        run("doctor memory --json", env);
        let v = json(env);
        let c = v["checks"].as_array().unwrap();
        assert_eq!(c.len(), 1, "{v}");
        assert_eq!(c[0]["id"], "memory.saver");
        (
            c[0]["status"].as_str().unwrap().to_owned(),
            c[0]["detail"].as_str().unwrap().to_owned(),
        )
    };
    let mut env = FakeEnv {
        files: files(ONE, "1\n"),
        ..Default::default()
    };
    let (st, d) = saver(&mut env);
    assert_eq!(st, "ok", "{d}");
    assert!(d.contains("file pages they touch (4 KB at a time)"), "{d}");
    assert!(
        d.contains("shared (20 MB now), because this computer has one account"),
        "{d}"
    );
    assert!(
        d.contains("Discord is in the background and gave its memory back 4 times."),
        "{d}"
    );
    assert!(
        !d.contains("Code"),
        "only apps with a budget are named: {d}"
    );

    // A second account since the boot: sharing is still on, a restart turns it off.
    let mut env = FakeEnv {
        files: files(TWO, "1\n"),
        ..Default::default()
    };
    let (st, d) = saver(&mut env);
    assert_eq!(st, "warn", "{d}");
    assert!(
        d.contains("still shared though this computer now has 2 accounts. Restart"),
        "{d}"
    );
    // Two accounts, off as the rule says: fine, and the reason is said.
    let mut env = FakeEnv {
        files: files(TWO, "0\n"),
        ..Default::default()
    };
    let (st, d) = saver(&mut env);
    assert_eq!(st, "ok", "{d}");
    assert!(
        d.contains("has 2 accounts, and sharing would let one person learn"),
        "{d}"
    );

    // As a person: debugfs is unreadable, the boot step's result is read instead.
    let mut f = files(ONE, "1\n");
    f.remove(0);
    let show = "systemctl show wolos-mm.service -p LoadState -p ActiveState -p Result";
    let mut env = FakeEnv {
        files: f.clone(),
        captures: vec![(
            show.into(),
            "LoadState=loaded\nActiveState=failed\nResult=exit-code\n".into(),
        )],
        ..Default::default()
    };
    let (st, d) = saver(&mut env);
    assert_eq!(st, "warn", "{d}");
    assert!(
        d.contains("weren’t applied when the computer started"),
        "{d}"
    );
    let mut env = FakeEnv {
        files: f,
        captures: vec![(
            show.into(),
            "LoadState=loaded\nActiveState=active\nResult=success\n".into(),
        )],
        ..Default::default()
    };
    assert_eq!(saver(&mut env).0, "ok");

    // Memory Saver switched off: said, not a problem.
    let mut f = files(ONE, "1\n");
    f.push((
        env.home.join("wolos/memory-saver.json"),
        r#"{"enabled":false,"nap":false,"exclude":[]}"#.into(),
    ));
    let mut env = FakeEnv {
        files: f,
        ..Default::default()
    };
    let (st, d) = saver(&mut env);
    assert_eq!(st, "ok", "{d}");
    assert!(
        d.contains("Memory Saver is off, so apps keep their memory."),
        "{d}"
    );
}
