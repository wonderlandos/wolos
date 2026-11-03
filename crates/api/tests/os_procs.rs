// SPDX-License-Identifier: GPL-2.0-or-later
//! Real process trees (own test binary: the lib tests reap with waitpid(-1)).
use std::time::{Duration, Instant};

#[test]
fn stop_tree_stops_root_and_children() {
    let mut child = std::process::Command::new("sh")
        .args(["-c", "sleep 30 & wait"])
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let t = Instant::now();
    api::os::stop_tree(child.id() as i32);
    assert!(!child.wait().unwrap().success());
    assert!(t.elapsed() < Duration::from_secs(5));
}

/// Audit S4-26: a job never inherits wold's raised descriptor limit. Run under `prlimit
/// --nofile=4096:` (as wold.service sets it), a job started by OsRunner sees soft 1024 and the
/// hard limit unchanged. Skipped without prlimit(1) or when the hard limit is below 4096.
#[test]
fn jobs_start_from_the_usual_descriptor_limit() {
    assert_eq!(api::os::child_nofile(4096, 524_288), (1024, 524_288));
    assert_eq!(
        api::os::child_nofile(512, 4096),
        (512, 4096),
        "never raised"
    );
    if let Some(out) = std::env::var_os("WOL_NOFILE_PROBE") {
        use api::supervisor::Runner;
        let runner = api::os::OsRunner {
            qos: Box::new(api::qos::Sched),
            pid_dir: std::env::temp_dir().join(format!("wol-nofile-pids-{}", std::process::id())),
        };
        let spec = shared::jobs::JobSpec::parse(&format!(
            r#"{{"label":"nofile-probe","program":["/bin/sh","-c","ulimit -Sn > {0}.tmp; ulimit -Hn >> {0}.tmp; mv {0}.tmp {0}"]}}"#,
            std::path::Path::new(&out).display()
        ))
        .unwrap();
        runner.spawn(&spec).unwrap();
        return;
    }
    let out = std::env::temp_dir().join(format!("wol-nofile-{}", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let exe = std::env::current_exe().unwrap();
    let st = std::process::Command::new("prlimit")
        .arg("--nofile=4096:")
        .arg("--")
        .arg(&exe)
        .args(["--exact", "jobs_start_from_the_usual_descriptor_limit"])
        .env("WOL_NOFILE_PROBE", &out)
        .stdout(std::process::Stdio::null())
        .status();
    if !st.is_ok_and(|s| s.success()) {
        println!("skipped: no prlimit or a hard limit below 4096");
        return;
    }
    let t = Instant::now();
    while !out.exists() {
        assert!(t.elapsed() < Duration::from_secs(5), "the job never wrote");
        std::thread::yield_now();
    }
    let got = std::fs::read_to_string(&out).unwrap();
    let mut l = got.lines();
    assert_eq!(l.next(), Some("1024"), "soft limit of the job: {got}");
    assert_ne!(l.next(), Some("1024"), "hard limit left alone: {got}");
    let _ = std::fs::remove_file(&out);
}
