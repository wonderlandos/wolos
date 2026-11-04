// SPDX-License-Identifier: BSD-2-Clause
//! One-verify timings for docs/modules/trust.md: `cargo test -p woltrust --release --test timing
//! -- --ignored --nocapture`. Prints the median of 200 runs; asserts nothing about speed.
use std::time::Instant;
use woltrust::crypto::{Purpose, message, verify_hybrid};
use woltrust::make::Secret;

fn median(mut f: impl FnMut()) -> f64 {
    let mut t: Vec<f64> = (0..200)
        .map(|_| {
            let s = Instant::now();
            f();
            s.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    t.sort_by(f64::total_cmp);
    t[t.len() / 2]
}

#[test]
#[ignore = "timing, run by hand for docs/modules/trust.md"]
fn print_the_cost_of_one_verify() {
    let msg = message(Purpose::Package, &[7u8; 4096]);
    for (name, root) in [
        ("hybrid Ed25519 + ML-DSA-65", false),
        ("hybrid Ed25519 + SLH-DSA-SHA2-128s", true),
    ] {
        let s = Secret::from_seed(&[1; 32], root);
        let sig = s.sign(Purpose::Package, &[7u8; 4096]);
        let us = median(|| assert!(verify_hybrid(s.public(), &msg, &sig).is_ok()));
        println!("TIMING {name}: {us:.1} us");
    }
    let s = Secret::from_seed(&[1; 32], false);
    let sig = s.sign(Purpose::Package, &[7u8; 4096]);
    let ed = median(|| {
        use ed25519_dalek::Verifier;
        let k = ed25519_dalek::VerifyingKey::from_bytes(&s.public().ed25519).expect("key");
        assert!(
            k.verify(&msg, &ed25519_dalek::Signature::from_bytes(&sig.ed25519))
                .is_ok()
        );
    });
    println!("TIMING Ed25519 alone: {ed:.1} us");
    let (pki, vector) = {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("vectors");
        let pki: woltrust::vectors::PkiFile =
            serde_json::from_slice(&std::fs::read(dir.join("pki.json")).expect("pki"))
                .expect("parses");
        let v: woltrust::vectors::Vector =
            serde_json::from_slice(&std::fs::read(dir.join("valid-person.json")).expect("v"))
                .expect("parses");
        (pki, v)
    };
    let full = median(|| {
        let (out, _) = woltrust::vectors::run(&vector, &pki).expect("runs");
        assert_eq!(out.trust, woltrust::Trust::Verified);
    });
    println!("TIMING whole check incl. loading root and list: {full:.1} us");
    let (_, store) = woltrust::vectors::run(&vector, &pki).expect("runs");
    let chain = {
        let b = &pki.blobs;
        let d = |n: &str| woltrust::b64::decode(&b[n], 1 << 20).expect("blob");
        (d("chain-alice"), d("wsig-alice"), d("content"))
    };
    let one = median(|| {
        let out = woltrust::verify(
            &store,
            &woltrust::Input {
                purpose: Purpose::Package,
                content: &chain.2,
                app_id: Some("alice.notes"),
                wsig: Some(&chain.1),
                chain: Some(&chain.0),
                first_seen: None,
                now: woltrust::pki::NOW,
            },
        );
        assert_eq!(out.trust, woltrust::Trust::Verified);
    });
    println!("TIMING package + leaf + Authors CA + root pin (verify only): {one:.1} us");
}
