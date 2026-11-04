// SPDX-License-Identifier: BSD-2-Clause
//! Every file in `vectors/` replays to the recorded result, the generator reproduces the files byte
//! for byte, and the vectors cover the cases ADR 0046 names. The hub's wasm build replays the same files.
use std::path::PathBuf;
use woltrust::vectors::{PkiFile, Vector, check};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vectors")
}

fn load() -> (PkiFile, Vec<Vector>) {
    let pki: PkiFile =
        serde_json::from_slice(&std::fs::read(dir().join("pki.json")).expect("pki.json is there"))
            .expect("pki.json parses");
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir())
        .expect("vectors dir is there")
        .flatten()
    {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "json")
            && p.file_name().is_some_and(|n| n != "pki.json")
        {
            let v: Vector = serde_json::from_slice(&std::fs::read(&p).expect("vector readable"))
                .expect("vector parses");
            assert_eq!(
                format!("{}.json", v.name),
                p.file_name().expect("name").to_string_lossy()
            );
            out.push(v);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    (pki, out)
}

#[test]
fn every_recorded_vector_replays_to_its_result() {
    let (pki, vectors) = load();
    assert!(vectors.len() >= 30, "{} vectors", vectors.len());
    for v in &vectors {
        check(v, &pki).unwrap_or_else(|e| panic!("{e}"));
    }
}

#[test]
fn the_generator_reproduces_the_files_byte_for_byte() {
    let (pki, vectors) = woltrust::vectors::generate().expect("generation succeeds");
    let on_disk = |name: &str| std::fs::read_to_string(dir().join(name)).expect("file readable");
    assert_eq!(woltrust::vectors::to_text(&pki), on_disk("pki.json"));
    for v in &vectors {
        assert_eq!(
            woltrust::vectors::to_text(v),
            on_disk(&format!("{}.json", v.name)),
            "{}",
            v.name
        );
    }
    let (_, files) = load();
    assert_eq!(files.len(), vectors.len(), "no stale vector file on disk");
}

#[test]
fn the_vectors_name_every_case_of_the_adr() {
    let (_, vectors) = load();
    let names: Vec<&str> = vectors.iter().map(|v| v.name.as_str()).collect();
    for want in [
        "valid-person",
        "valid-org",
        "valid-wolos",
        "bad-half-ed25519",
        "bad-half-second",
        "expired",
        "revoked-key-compromise",
        "revoked-abuse",
        "revoked-superseded",
        "revoked-ceased",
        "stale-list",
        "list-too-old",
        "namespace-mismatch",
        "wrong-usage",
        "chain-too-long",
        "chain-too-large",
        "test-root-refused",
    ] {
        assert!(names.contains(&want), "missing vector {want}");
    }
}

#[test]
fn only_the_valid_vectors_are_trusted() {
    use woltrust::Trust;
    let (_, vectors) = load();
    for v in &vectors {
        let good = matches!(v.expect.trust, Trust::Verified | Trust::Wolos);
        let named_valid = v.name.starts_with("valid-")
            || matches!(
                v.name.as_str(),
                "stale-list"
                    | "expired-but-timestamped"
                    | "revoked-superseded-older-file"
                    | "revoked-ceased-older-file"
                    | "revoked-intermediate-by-index-ignored"
                    | "root-a-revoked-by-itself-ignored"
            );
        assert_eq!(good, named_valid, "{}", v.name);
    }
}
