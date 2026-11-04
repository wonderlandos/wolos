// SPDX-License-Identifier: BSD-2-Clause
//! `wolos/seal/1`: every file in `vectors/seal/` replays, the generator reproduces them byte for
//! byte, the recipients' certificates (format 1 with encryption keys) still verify as verified
//! authors, and a body with half an encryption key is refused.
#![cfg(feature = "seal")]
use std::path::PathBuf;
use woltrust::pki::{DAY, NOW, Pki, chain_of};
use woltrust::seal_vectors::{SealVector, check, generate, recipient};
use woltrust::vectors::{PkiFile, to_text};
use woltrust::{Code, Input, Purpose, Store, Trust, make, verify};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vectors/seal")
}

fn load() -> (PkiFile, Vec<SealVector>) {
    let keys: PkiFile =
        serde_json::from_slice(&std::fs::read(dir().join("keys.json")).expect("keys.json"))
            .expect("keys.json parses");
    let mut out: Vec<SealVector> = std::fs::read_dir(dir())
        .expect("seal dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n != "keys.json"))
        .map(|p| serde_json::from_slice(&std::fs::read(p).expect("read")).expect("parses"))
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    (keys, out)
}

#[test]
fn every_recorded_seal_vector_replays() {
    let (keys, vs) = load();
    assert!(vs.len() >= 13, "{} seal vectors", vs.len());
    for v in &vs {
        check(v, &keys).unwrap_or_else(|e| panic!("{e}"));
    }
    for name in [
        "sealed-to-person",
        "sealed-to-org",
        "wrong-recipient",
        "tampered-ciphertext",
    ] {
        assert!(vs.iter().any(|v| v.name == name), "{name} is covered");
    }
}

#[test]
fn the_seal_generator_reproduces_the_files() {
    let (keys, vs) = generate().expect("generation succeeds");
    let on_disk = |n: &str| std::fs::read_to_string(dir().join(n)).expect("file");
    assert_eq!(to_text(&keys), on_disk("keys.json"));
    for v in &vs {
        assert_eq!(
            to_text(v),
            on_disk(&format!("{}.json", v.name)),
            "{}",
            v.name
        );
    }
    assert_eq!(load().1.len(), vs.len(), "no stale seal vector on disk");
}

#[test]
fn a_certificate_with_encryption_keys_still_verifies_as_an_author() {
    let p = Pki::new();
    let r = recipient(&p, false);
    let c = woltrust::Cert::parse(&r.file).expect("parses");
    assert!(c.enc.is_some(), "encryption keys decoded");
    assert!(c.body_bytes.len() <= woltrust::MAX_BODY);
    let km = woltrust::key::KeyMaterial::new(r.seed, false);
    let content = b"manifest";
    let wsig = make::wsig(&r.serial, &km.secret(), Purpose::Package, content);
    let l = woltrust::revoked::ListText {
        format: 1,
        serial: 3,
        issued: NOW - DAY,
        expires: NOW - DAY + woltrust::LIST_LIFETIME,
        entries: vec![],
    };
    let list = make::revocation_list(&l, &p.index.body.serial, &p.index.secret);
    let store = Store::load(
        &[&p.root_a.file],
        &[&p.authors.file, &p.index.file],
        &[list],
        NOW,
        true,
    );
    let chain = chain_of(&[&r.file]);
    let out = verify(
        &store,
        &Input {
            purpose: Purpose::Package,
            content,
            app_id: Some("alice.notes"),
            wsig: Some(&wsig),
            chain: Some(&chain),
            first_seen: None,
            now: NOW,
        },
    );
    assert_eq!((out.trust, out.code), (Trust::Origin, Code::Ok));
}

#[test]
fn a_body_with_half_an_encryption_key_is_refused() {
    let p = Pki::new();
    let r = recipient(&p, false);
    let c = woltrust::Cert::parse(&r.file).expect("parses");
    let mut b = c.body.clone();
    b.keys.ml_kem_768 = None;
    let bytes = serde_json::to_vec(&b).expect("json");
    assert!(woltrust::cert::Body::parse(&bytes).is_err());
}
