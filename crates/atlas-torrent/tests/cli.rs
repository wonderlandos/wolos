// SPDX-License-Identifier: BSD-2-Clause
//! The helper's command line as the package ships it (no `tools` feature): what it refuses before
//! any network or stranger's data is touched.
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_atlas-torrent");

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-torrent-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// DL-SEC-01: outside a confined unit (this test's own session) `serve` refuses to serve, says
/// why, and leaves the refusal beside the socket for `launch`.
#[test]
fn serve_outside_a_unit_refuses() {
    let d = tmp("refuse");
    let run = d.join("run");
    std::fs::create_dir_all(&run).unwrap();
    let out = Command::new(BIN)
        .args(["serve", "--check-only", "--no-dht", "--downloads"])
        .arg(d.join("dl"))
        .arg("--state")
        .arg(d.join("st"))
        .arg("--socket")
        .arg(run.join("torrent.sock"))
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{err}");
    assert!(err.contains("cannot confine the torrent helper"), "{err}");
    let why = std::fs::read_to_string(run.join("torrent.refused")).unwrap();
    assert!(why.contains("missing: home"), "{why}");
    assert!(!run.join("torrent.sock").exists(), "no socket was opened");
    // the same without --check-only: it never gets to the socket either
    let out = Command::new(BIN)
        .args(["serve", "--no-dht", "--downloads"])
        .arg(d.join("dl"))
        .arg("--state")
        .arg(d.join("st"))
        .arg("--socket")
        .arg(run.join("torrent.sock"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(!run.join("torrent.sock").exists());
    let _ = std::fs::remove_dir_all(&d);
}

fn launch(downloads: &str, home: &str) -> (Option<i32>, String) {
    let d = tmp("launch");
    let out = Command::new(BIN)
        .args(["launch", "--downloads", downloads, "--state"])
        .arg(d.join("st"))
        .arg("--socket")
        .arg(d.join("run").join("torrent.sock"))
        .env("HOME", home)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&d);
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// DL-SEC-05: the home folder (or a folder holding it) is never bound as the downloads folder.
#[test]
fn launch_refuses_the_home_folder_as_downloads() {
    for dl in ["/home/p", "/home", "/"] {
        let (code, err) = launch(dl, "/home/p");
        assert_eq!(code, Some(1), "{dl}: {err}");
        assert!(
            err.contains("choose a download folder inside your home"),
            "{dl}: {err}"
        );
    }
}

/// DL-SEC-14: folder names systemd would misread are refused with the reason.
#[test]
fn launch_refuses_folders_systemd_cannot_carry() {
    for dl in ["/home/p/a:b", "/home/p/My Downloads", "/home/p/it's"] {
        let (code, err) = launch(dl, "/home/p");
        assert_eq!(code, Some(1), "{dl}: {err}");
        assert!(
            err.contains("characters the service manager cannot carry"),
            "{dl}: {err}"
        );
    }
}

/// DL-SEC-17: the test tools are not in the package build.
#[cfg(not(feature = "tools"))]
#[test]
fn the_test_tools_are_not_shipped() {
    for sub in ["seed", "make"] {
        let out = Command::new(BIN).arg(sub).output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stderr).contains("usage"));
    }
    let out = Command::new(BIN)
        .args(["serve", "--unconfined"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "--unconfined is unknown");
}
