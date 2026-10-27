//! Every daemon the shell and `wol` talk to starts at boot on the image. wol-admind shipped as a binary
//! only (no job, an rc.d script left off and pointing at /usr/local/sbin): every admin action failed.
use shared::jobs::JobSpec;
use std::{fs, path::PathBuf};

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

/// Binaries that bind a `shared::*::SOCKET` (crates/*/src/bin/<name>.rs).
fn daemons() -> Vec<String> {
    let mut v = vec![];
    for c in fs::read_dir(repo("crates")).unwrap().flatten() {
        let Ok(bins) = fs::read_dir(c.path().join("src/bin")) else {
            continue;
        };
        for b in bins.flatten() {
            let src = fs::read_to_string(b.path()).unwrap_or_default();
            if src
                .lines()
                .any(|l| l.contains("bind(") && l.contains("SOCKET"))
            {
                v.push(b.path().file_stem().unwrap().to_string_lossy().into_owned());
            }
        }
    }
    v.sort();
    v
}

/// Jobs wold starts at boot (image overlay).
fn boot_jobs() -> Vec<JobSpec> {
    let dir = repo("desktop/overlay/usr/local/etc/wolos/jobs");
    fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|f| {
            let t = fs::read_to_string(f.path()).unwrap();
            serde_json::from_str::<JobSpec>(&t)
                .unwrap_or_else(|e| panic!("{}: {e}", f.path().display()))
        })
        .collect()
}

fn installed(bin: &str) -> bool {
    read("image/build-rust.sh")
        .lines()
        .find(|l| l.trim_start().starts_with("for b in wold "))
        .is_some_and(|l| l.split_whitespace().any(|w| w.trim_end_matches(';') == bin))
}

#[test]
fn every_daemon_starts_at_boot() {
    let ds = daemons();
    assert_eq!(
        ds,
        ["wol-admind", "wol-login", "wold"],
        "new daemon: wire it below"
    );
    let jobs = boot_jobs();
    for d in &ds {
        assert!(installed(d), "{d}: not installed by image/build-rust.sh");
        let path = format!("/usr/local/bin/{d}");
        if d == "wold" {
            // wold is the supervisor: rc.d, enabled on live and installed systems.
            let rc = read("image/files/usr/local/etc/rc.d/wold");
            assert!(
                rc.contains(&path) && rc.contains("wold_enable:=YES"),
                "{rc}"
            );
            for f in ["image/rcconf.sh", "image/installerconfig"] {
                assert!(read(f).contains("wold_enable=YES"), "{f}");
            }
            continue;
        }
        let job = jobs
            .iter()
            .find(|j| j.program.first() == Some(&path))
            .unwrap_or_else(|| panic!("{d}: no wold job runs {path}"));
        assert!(job.run_at_load && job.keep_alive, "{d}: {job:?}");
    }
}

#[test]
fn admin_daemon_is_a_wold_job_only() {
    let job = boot_jobs()
        .into_iter()
        .find(|j| j.label == "org.wol.admin")
        .expect("org.wol.admin job");
    assert_eq!(job.program, ["/usr/local/bin/wol-admind"]);
    // One mechanism: no second (rc.d) launcher for the same socket.
    assert!(!repo("crates/api/rc.d/wol_admind").exists());
}
