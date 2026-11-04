// SPDX-License-Identifier: BSD-2-Clause
//! File certificates (what WolOS Hub checked about one release): every file in
//! `vectors/filecert/` replays, the generator reproduces them byte for byte, and the cases the
//! lead named are there (valid, withdrawn version, tampered).
use std::path::PathBuf;
use woltrust::filecert_vectors::{FileCertVector, check, generate};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vectors/filecert")
}

fn load() -> Vec<FileCertVector> {
    let mut out: Vec<FileCertVector> = std::fs::read_dir(dir())
        .expect("filecert dir")
        .flatten()
        .map(|e| {
            serde_json::from_slice(&std::fs::read(e.path()).expect("readable"))
                .expect("vector parses")
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[test]
fn every_recorded_filecert_vector_replays() {
    let vs = load();
    for v in &vs {
        check(v).unwrap_or_else(|e| panic!("{e}"));
    }
    for (name, want) in [
        ("filecert-valid", "checked"),
        ("filecert-withdrawn-version", "withdrawn-version"),
        ("filecert-tampered", "file-cert-bad-signature"),
    ] {
        let v = vs.iter().find(|v| v.name == name).expect(name);
        assert_eq!(v.expect, want, "{name}");
    }
}

#[test]
fn the_filecert_generator_reproduces_the_files() {
    let vs = generate().expect("generation succeeds");
    for v in &vs {
        let on_disk =
            std::fs::read_to_string(dir().join(format!("{}.json", v.name))).expect("file readable");
        assert_eq!(woltrust::vectors::to_text(v), on_disk, "{}", v.name);
    }
    assert_eq!(load().len(), vs.len(), "no stale file certificate vector");
}
