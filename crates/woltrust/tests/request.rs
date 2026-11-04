// SPDX-License-Identifier: BSD-2-Clause
//! Certificate requests: every file in `vectors/request/` replays to its result, the generator
//! reproduces them byte for byte, and the hub's copy of a request (with `id`, `approved_by`,
//! `approved_at`, `verified_at`) reads as the request itself.
#![cfg(feature = "seal")]
use std::path::PathBuf;
use woltrust::request_vectors::{RequestVector, check, generate};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vectors/request")
}

fn load() -> Vec<RequestVector> {
    let mut out: Vec<RequestVector> = std::fs::read_dir(dir())
        .expect("request dir")
        .flatten()
        .map(|e| {
            let v: RequestVector =
                serde_json::from_slice(&std::fs::read(e.path()).expect("readable"))
                    .expect("vector parses");
            assert_eq!(
                format!("{}.json", v.name),
                e.file_name().to_string_lossy(),
                "file name"
            );
            v
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[test]
fn every_recorded_request_vector_replays() {
    let vs = load();
    assert!(vs.len() >= 16, "{} request vectors", vs.len());
    for v in &vs {
        check(v).unwrap_or_else(|e| panic!("{e}"));
    }
    for (name, accepted) in [
        ("request-hub-copy", true),
        ("request-hub-copy-member", true),
        ("request-other-github-id", false),
        ("request-other-encryption-key", false),
        ("request-org-added-later", false),
        ("request-extra-namespace", false),
        ("request-other-login", false),
    ] {
        let v = vs.iter().find(|v| v.name == name).expect(name);
        assert_eq!(v.expect.accepted, accepted, "{name}");
    }
}

#[test]
fn the_request_generator_reproduces_the_files() {
    let vs = generate().expect("generation succeeds");
    for v in &vs {
        let on_disk =
            std::fs::read_to_string(dir().join(format!("{}.json", v.name))).expect("file readable");
        assert_eq!(woltrust::vectors::to_text(v), on_disk, "{}", v.name);
    }
    assert_eq!(load().len(), vs.len(), "no stale request vector on disk");
}

#[test]
fn the_hub_fields_are_ignored_and_the_proof_still_binds() {
    let hub = load()
        .into_iter()
        .find(|v| v.name == "request-hub-copy")
        .expect("hub copy");
    for field in [
        "\"id\"",
        "\"approved_by\"",
        "\"approved_at\"",
        "\"verified_at\"",
    ] {
        assert!(hub.request.contains(field), "{field} is in the hub's copy");
    }
    let r = woltrust::request::parse(hub.request.as_bytes()).expect("parses");
    assert!(r.check_proof().is_ok());
    let big = format!(
        "{}{}",
        hub.request,
        " ".repeat(woltrust::request::MAX_REQUEST)
    );
    assert_eq!(
        woltrust::request::parse(big.as_bytes())
            .unwrap_err()
            .as_str(),
        "request-too-large"
    );
}
