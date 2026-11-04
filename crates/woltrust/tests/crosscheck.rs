// SPDX-License-Identifier: BSD-2-Clause
//! The ML-DSA-65 verifier is checked against a second implementation (a dev-dependency, never
//! shipped): real signatures from the vectors must pass in both, and damaged ones must fail in both.
use ml_dsa::{EncodedSignature, EncodedVerifyingKey, MlDsa65, Signature, VerifyingKey};
use woltrust::cert::Cert;
use woltrust::crypto::{Purpose, Second, message, verify_hybrid};

fn pki_blob(name: &str) -> Vec<u8> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("vectors");
    let pki: woltrust::vectors::PkiFile =
        serde_json::from_slice(&std::fs::read(dir.join("pki.json")).expect("pki.json"))
            .expect("parses");
    woltrust::b64::decode(&pki.blobs[name], 1 << 20).expect("blob")
}

fn other_accepts(pk: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    let Ok(pk) = EncodedVerifyingKey::<MlDsa65>::try_from(pk) else {
        return false;
    };
    let Ok(enc) = EncodedSignature::<MlDsa65>::try_from(sig) else {
        return false;
    };
    let Some(sig) = Signature::<MlDsa65>::decode(&enc) else {
        return false;
    };
    VerifyingKey::<MlDsa65>::decode(&pk).verify_with_context(msg, &[], &sig)
}

#[test]
fn both_ml_dsa_implementations_agree_on_real_and_damaged_signatures() {
    let authors = Cert::parse(&pki_blob("authors")).expect("authors parses");
    let alice = Cert::parse(&pki_blob("alice")).expect("alice parses");
    let Second::MlDsa65(pk) = &authors.keys.second else {
        panic!("the Authors CA has an ML-DSA-65 key");
    };
    let msg = message(Purpose::Cert, &alice.body_bytes);
    assert!(
        other_accepts(&pk[..], &msg, &alice.sig.second),
        "second implementation accepts"
    );
    assert!(
        verify_hybrid(&authors.keys, &msg, &alice.sig).is_ok(),
        "ours accepts"
    );
    for at in (0..alice.sig.second.len()).step_by(97) {
        let mut bad = alice.sig.clone();
        bad.second[at] ^= 0x10;
        assert!(
            !other_accepts(&pk[..], &msg, &bad.second),
            "second rejects flip at {at}"
        );
        assert!(
            verify_hybrid(&authors.keys, &msg, &bad).is_err(),
            "ours rejects flip at {at}"
        );
    }
    let wrong = message(Purpose::Package, &alice.body_bytes);
    assert!(!other_accepts(&pk[..], &wrong, &alice.sig.second));
    assert!(verify_hybrid(&authors.keys, &wrong, &alice.sig).is_err());
}
