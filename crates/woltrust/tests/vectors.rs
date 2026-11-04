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
            && p.file_name()
                .is_some_and(|n| n != "pki.json" && n != "lines.json")
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
        "valid-trusted",
        "trusted-superseded-downgrade",
        "trusted-abuse",
        "trusted-origin-revoked",
    ] {
        assert!(names.contains(&want), "missing vector {want}");
    }
}

#[test]
fn only_the_valid_vectors_are_trusted() {
    use woltrust::Trust;
    let (_, vectors) = load();
    for v in &vectors {
        let good = matches!(
            v.expect.trust,
            Trust::Trusted | Trust::Origin | Trust::Wolos
        );
        let named_valid = v.name.starts_with("valid-")
            || matches!(
                v.name.as_str(),
                "list-too-old" | "no-list" | "list-bad-signature"
            )
            || matches!(
                v.name.as_str(),
                "stale-list"
                    | "expired-but-timestamped"
                    | "revoked-superseded-older-file"
                    | "revoked-ceased-older-file"
                    | "revoked-intermediate-by-index-ignored"
                    | "root-a-revoked-by-itself-ignored"
                    | "trusted-superseded-downgrade"
                    | "trusted-ceased-older-file"
                    | "version-entry-ignored-for-certificates"
                    | "emergency-other-chain-unaffected"
                    | "emergency-forged-by-intermediate"
                    | "rkdb-index-cannot-revoke-a-ca"
                    | "witness-valid"
                    | "emergency-section-future-dated"
                    | "list-eternal"
            );
        assert_eq!(good, named_valid, "{}", v.name);
    }
}

#[test]
fn every_certificate_line_is_recorded() {
    let want = woltrust::vectors::to_text(&woltrust::vectors_ids::cert_lines());
    let have = std::fs::read_to_string(dir().join("lines.json")).expect("lines.json");
    assert_eq!(want, have);
    let lines: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&have).expect("parses");
    assert!(lines["root-a"].ends_with("<ed25519+slh-dsa-sha2-128s>"));
    assert!(lines["authors-2"].contains("<issued-by: test-root-a | test-root-b>"));
    assert!(lines["alice-trusted"].contains("<trusted> <origin-cert: "));
    assert!(lines["org"].starts_with("<org/examplelabs> <github:2002>"));
    assert!(lines["bob"].contains("<issued-by: TEST WolOS Authors>"));
    assert!(lines["example-root-b"].starts_with("<WolOS Project - hello@wolos.org> <root> "));
    assert!(lines["example-authors-ca"].contains(
        "<issued-by: Júlia Klee - Kleedolinux - WolOS.org - 10/04/2009 - 9 de Julho 1932 - São Paulo | WolOS Project - hello@wolos.org>"
    ));
}

#[test]
fn a_seal_to_a_derived_id_is_format_2_and_names_the_full_id() {
    let p = woltrust::pki::Pki::new();
    let ids = woltrust::vectors_ids::Ids::new(&p);
    let bob = woltrust::Cert::parse(&ids.bob.file).expect("bob");
    let s = woltrust::seal::seal(&bob, woltrust::seal::FILE_LABEL, b"hi", &[4; 64]).expect("seals");
    let h = woltrust::seal::peek(&s).expect("header");
    assert_eq!(
        (h.format, h.recipient.as_str()),
        ("wolos/seal/2", bob.body.serial.as_str())
    );
    assert_eq!(
        s.len(),
        2 + woltrust::seal::overhead(woltrust::seal::FILE_LABEL)
    );
    let k = ids.bob.km.enc();
    assert_eq!(
        &woltrust::seal::open(&k, &bob.body.serial, woltrust::seal::FILE_LABEL, &s).expect("opens")
            [..],
        b"hi"
    );
}
