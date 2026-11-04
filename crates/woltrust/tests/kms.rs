// SPDX-License-Identifier: BSD-2-Clause
//! The KMS spike test of docs/trust/kms.md: a signature made by the key service verifies here.
//! Ignored by default (it needs files from a key that costs money); run it with
//! `WOLTRUST_KMS_DIR=<dir> cargo test -p woltrust --test kms -- --ignored --nocapture`, where the
//! directory holds `ml.pub.der`, `ml.sig`, `ed.pub.der`, `ed.sig` and `msg.bin` (the message
//! `label || 0x00 || SHA-512(content)` that was signed with `--message-type RAW`).
use fips204::ml_dsa_65;
use fips204::traits::{SerDes, Verifier};

fn read(name: &str) -> Vec<u8> {
    let dir = std::env::var("WOLTRUST_KMS_DIR").expect("WOLTRUST_KMS_DIR names the directory");
    std::fs::read(std::path::Path::new(&dir).join(name)).expect("file is there")
}

/// The raw key is the tail of a DER SubjectPublicKeyInfo: its BIT STRING is the last `n` bytes.
fn tail(der: &[u8], n: usize) -> &[u8] {
    &der[der.len() - n..]
}

#[test]
#[ignore = "needs a KMS key and its output; see docs/trust/kms.md"]
fn kms_ml_dsa_65_and_ed25519_signatures_verify_with_our_code() {
    let msg = read("msg.bin");
    assert!(msg.len() < 100, "the signed message is the short form");
    let pk: [u8; ml_dsa_65::PK_LEN] = tail(&read("ml.pub.der"), ml_dsa_65::PK_LEN)
        .try_into()
        .expect("1952 bytes");
    let sig = read("ml.sig");
    println!(
        "ML_DSA_65 signature length: {} (expected {})",
        sig.len(),
        ml_dsa_65::SIG_LEN
    );
    let sig: [u8; ml_dsa_65::SIG_LEN] = sig.try_into().expect("a raw 3309-byte signature");
    let key = ml_dsa_65::PublicKey::try_from_bytes(pk).expect("key decodes");
    assert!(
        key.verify(&msg, &sig, &[]),
        "ML-DSA-65 with an empty context"
    );

    let ed: [u8; 32] = tail(&read("ed.pub.der"), 32).try_into().expect("32 bytes");
    let es: [u8; 64] = read("ed.sig").try_into().expect("a raw 64-byte signature");
    let k = ed25519_dalek::VerifyingKey::from_bytes(&ed).expect("key decodes");
    k.verify_strict(&msg, &ed25519_dalek::Signature::from_bytes(&es))
        .expect("Ed25519 verifies");
}
