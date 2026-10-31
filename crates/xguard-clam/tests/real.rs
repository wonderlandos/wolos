// SPDX-License-Identifier: GPL-2.0-or-later
//! The real library, with a small real signature file. With `XGUARD_CLAM_DB` set: the full database.
#![cfg(feature = "libclamav")]
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_wol-xguard-clam");
pub const EICAR: &str = r"X5O!P%@AP[4\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";

fn dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("xg-clam-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("db")).unwrap();
    std::fs::write(
        d.join("db/t.ndb"),
        "Test.Clam.Marker:0:*:5847554152442d434c414d2d4d41524b4552\n",
    )
    .unwrap();
    d
}

fn once(db: &str, path: &std::path::Path) -> (i32, String) {
    let o = Command::new(BIN)
        .args(["scan", "--db", db])
        .arg(path)
        .output()
        .unwrap();
    (
        o.status.code().unwrap(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
    )
}

#[test]
fn a_signature_file_loads_and_the_library_finds_what_it_describes() {
    let d = dir("once");
    let (bad, good) = (d.join("bad"), d.join("good"));
    std::fs::write(&bad, "aXGUARD-CLAM-MARKERz").unwrap();
    std::fs::write(&good, "nothing here").unwrap();
    let db = d.join("db");
    let (code, out) = once(db.to_str().unwrap(), &bad);
    assert_eq!(code, 1, "{out}");
    assert!(out.starts_with("found Test.Clam.Marker"), "{out}");
    assert!(out.contains("signatures=1"), "{out}");
    assert_eq!(once(db.to_str().unwrap(), &good).0, 0);
}

#[test]
fn a_missing_database_is_an_answer_of_its_own() {
    let d = dir("none");
    let (code, _) = once("/nonexistent", &d.join("db/t.ndb"));
    assert_eq!(code, 3);
}

#[test]
fn the_full_database_finds_the_test_file_inside_an_archive() {
    let Ok(db) = std::env::var("XGUARD_CLAM_DB") else {
        eprintln!("XGUARD_CLAM_DB not set: not tested here");
        return;
    };
    let d = dir("full");
    let f = d.join("eicar.com");
    std::fs::write(&f, EICAR).unwrap();
    let (code, out) = once(&db, &f);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("Eicar"), "{out}");
    let gz = d.join("eicar.tar.gz");
    let made = Command::new("tar")
        .arg("-czf")
        .arg(&gz)
        .arg("-C")
        .arg(&d)
        .arg("eicar.com")
        .status()
        .unwrap();
    assert!(made.success());
    assert_eq!(once(&db, &gz).0, 1, "inside a tar.gz");
}
