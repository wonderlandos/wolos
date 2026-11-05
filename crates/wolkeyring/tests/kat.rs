// SPDX-License-Identifier: BSD-2-Clause
//! Known-answer tests of every primitive the keyring stands on (ADR 0054 "Risks"): the vectors
//! come from the standards, not from this code.
//! - ML-KEM-768 key generation: NIST ACVP (FIPS 203), 25 cases, `tests/vectors/acvp-keygen.json`.
//! - ML-KEM-768 keygen + encaps + decaps + implicit rejection: Go's accumulated test (crypto/mlkem,
//!   BSD-3-Clause), 100 iterations in debug builds and 10 000 in release.
//! - ML-DSA-65 key generation: NIST ACVP (FIPS 204), 25 cases.
//! - ChaCha20-Poly1305: RFC 8439 section 2.8.2, through wolcrypt::aead.
//! - PBKDF2-HMAC-SHA512: the published 1-iteration vector, through wolcrypt::kdf.
//! - X25519: RFC 7748 section 6.1, through wolcrypt::kem.
//! - Argon2id: RFC 9106 section 5.3, through wolkeyring::kdf.
use serde::Deserialize;

#[derive(Deserialize)]
struct Kem {
    #[serde(rename = "tcId")]
    tc: u32,
    d: String,
    z: String,
    ek_sha256: String,
    dk_sha256: String,
}

#[derive(Deserialize)]
struct Dsa {
    #[serde(rename = "tcId")]
    tc: u32,
    seed: String,
    pk_sha256: String,
    sk_sha256: String,
}

#[derive(Deserialize)]
struct File {
    ml_kem_768: Vec<Kem>,
    ml_dsa_65: Vec<Dsa>,
}

fn vectors() -> File {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/acvp-keygen.json");
    serde_json::from_slice(&std::fs::read(p).expect("vector file")).expect("vector json")
}

fn h32(s: &str) -> [u8; 32] {
    wolcrypt::hash::unhex(s)
        .expect("hex")
        .try_into()
        .expect("32 bytes")
}

fn sha256_hex(b: &[u8]) -> String {
    wolcrypt::hash::hex(&wolcrypt::hash::sha256(b))
}

#[test]
fn ml_kem_768_key_generation_matches_nist_acvp() {
    use fips203::ml_kem_768;
    use fips203::traits::{KeyGen, SerDes};
    let v = vectors();
    assert_eq!(v.ml_kem_768.len(), 25);
    for t in v.ml_kem_768 {
        let (ek, dk) = ml_kem_768::KG::keygen_from_seed(h32(&t.d), h32(&t.z));
        assert_eq!(
            sha256_hex(&ek.into_bytes()),
            t.ek_sha256,
            "tcId {} ek",
            t.tc
        );
        assert_eq!(
            sha256_hex(&dk.into_bytes()),
            t.dk_sha256,
            "tcId {} dk",
            t.tc
        );
    }
}

#[test]
fn ml_dsa_65_key_generation_matches_nist_acvp() {
    use fips204::ml_dsa_65;
    use fips204::traits::{KeyGen, SerDes};
    let v = vectors();
    assert_eq!(v.ml_dsa_65.len(), 25);
    for t in v.ml_dsa_65 {
        let (pk, sk) = ml_dsa_65::KG::keygen_from_seed(&h32(&t.seed));
        assert_eq!(
            sha256_hex(&pk.into_bytes()),
            t.pk_sha256,
            "tcId {} pk",
            t.tc
        );
        assert_eq!(
            sha256_hex(&sk.into_bytes()),
            t.sk_sha256,
            "tcId {} sk",
            t.tc
        );
    }
}

#[test]
fn ml_kem_768_matches_go_accumulated() {
    use fips203::ml_kem_768;
    use fips203::traits::{Decaps, Encaps, KeyGen, SerDes};
    use sha3::Shake128;
    use sha3::digest::{ExtendableOutput, Update, XofReader};
    let (n, expected) = if cfg!(debug_assertions) {
        (
            100,
            "1114b1b6699ed191734fa339376afa7e285c9e6acf6ff0177d346696ce564415",
        )
    } else {
        (
            10_000,
            "8a518cc63da366322a8e7a818c7a0d63483cb3528d34a4cf42f35d5ad73f22fc",
        )
    };
    let mut s = Shake128::default().finalize_xof();
    let mut o = Shake128::default();
    for _ in 0..n {
        let (mut d, mut z, mut m) = ([0u8; 32], [0u8; 32], [0u8; 32]);
        s.read(&mut d);
        s.read(&mut z);
        let (ek, dk) = ml_kem_768::KG::keygen_from_seed(d, z);
        o.update(&ek.clone().into_bytes());
        s.read(&mut m);
        let (k, ct) = ek.encaps_from_seed(&m);
        o.update(&ct.clone().into_bytes());
        o.update(&k.clone().into_bytes());
        assert_eq!(dk.try_decaps(&ct).expect("decaps"), k);
        let mut ct1 = [0u8; ml_kem_768::CT_LEN];
        s.read(&mut ct1);
        let k1 = dk
            .try_decaps(&ml_kem_768::CipherText::try_from_bytes(ct1).expect("ct"))
            .expect("implicit rejection");
        o.update(&k1.into_bytes());
    }
    let mut out = [0u8; 32];
    o.finalize_xof().read(&mut out);
    assert_eq!(wolcrypt::hash::hex(&out), expected, "{n} iterations");
}

#[test]
fn chacha20_poly1305_matches_rfc_8439() {
    let key: [u8; 32] = core::array::from_fn(|i| 0x80 + i as u8);
    let nonce: [u8; 12] = wolcrypt::hash::unhex("070000004041424344454647")
        .unwrap()
        .try_into()
        .unwrap();
    let ad = wolcrypt::hash::unhex("50515253c0c1c2c3c4c5c6c7").unwrap();
    let mut buf = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.".to_vec();
    wolcrypt::aead::seal(&key, &nonce, &ad, &mut buf);
    assert_eq!(
        wolcrypt::hash::hex(&buf),
        "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d63dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b3692ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc3ff4def08e4b7a9de576d26586cec64b6116\
         1ae10b594f09e26a7e902ecbd0600691"
    );
    assert!(wolcrypt::aead::open(&key, &nonce, &ad, &mut buf).is_some());
}

#[test]
fn pbkdf2_hmac_sha512_matches_the_published_vector() {
    let mut out = [0u8; 64];
    wolcrypt::kdf::raw(b"password", b"salt", 1, &mut out);
    assert_eq!(
        wolcrypt::hash::hex(&out),
        "867f70cf1ade02cff3752599a3a53dc4af34c7a669815ae5d513554e1c8cf252c02d470a285a0501bad999bfe943c08f050235d7d68b1da55e63f73b60a57fce"
    );
}

#[test]
fn x25519_matches_rfc_7748() {
    let a = h32("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
    let b = h32("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb");
    let pb = wolcrypt::kem::x25519_public(&b);
    assert_eq!(
        *wolcrypt::kem::x25519(&a, &pb).unwrap(),
        h32("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742")
    );
}

#[test]
fn argon2id_matches_rfc_9106_section_5_3() {
    // Password 32 x 0x01, salt 16 x 0x02, secret 8 x 0x03, associated data 12 x 0x04,
    // m = 32 KiB, t = 3, p = 4, 32-byte tag.
    let mut out = [0u8; 32];
    wolkeyring::kdf::argon2_raw(
        &[1; 32],
        &[2; 16],
        Some((&[3; 8], &[4; 12])),
        32,
        3,
        4,
        &mut out,
    )
    .unwrap();
    assert_eq!(
        wolcrypt::hash::hex(&out),
        "0d640df58d78766c08c037a34a8b53c9d01ef0452d75b65eb52520e96b01e659"
    );
}
