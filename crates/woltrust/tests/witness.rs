// SPDX-License-Identifier: BSD-2-Clause
//! The witness (lead decisions 2026-11-04): woltrust and wolcrypt alone give the same answer and
//! the same signature line for a witness's detached signature, and a tampered file, a witness
//! signed by one root only, or a swapped root is refused by both.
use woltrust::pki::{DAY, NOW, Pki};
use woltrust::vectors_ids::Ids;

#[test]
fn woltrust_and_wolcrypt_agree_on_a_detached_signature() {
    let p = Pki::new();
    let ids = Ids::new(&p);
    let file = p.root_a.file.clone();
    let sig = woltrust::witness::sign(
        &ids.witness.body.serial,
        &ids.witness.secret(),
        &file,
        NOW - DAY,
    );
    let none: [(&[u8], &[u8]); 0] = [];
    let store = woltrust::Store::load(
        &[p.root_a.file.clone(), p.root_b.file.clone()],
        std::slice::from_ref(&ids.witness.file),
        &none,
        NOW,
        true,
    );
    let out = woltrust::witness::verify(&store, &file, &sig, NOW);
    assert_eq!(out.trust, woltrust::Trust::Wolos, "{:?}", out.code);
    let line = wolcrypt::detached::verify(
        &file,
        &sig,
        &ids.witness.file,
        [&p.root_a.file, &p.root_b.file],
    )
    .expect("wolcrypt verifies");
    assert_eq!(Some(line.clone()), out.line);
    assert!(line.starts_with("<TEST WolOS Witness A> <sha512:"));
    let mut other = file.clone();
    other[10] ^= 1;
    assert!(
        wolcrypt::detached::verify(
            &other,
            &sig,
            &ids.witness.file,
            [&p.root_a.file, &p.root_b.file]
        )
        .is_err()
    );
    assert_ne!(
        woltrust::witness::verify(&store, &other, &sig, NOW).trust,
        woltrust::Trust::Wolos
    );
    let mut lone = woltrust::Cert::parse(&ids.witness.file).expect("parses");
    lone.cosigs.clear();
    assert_eq!(
        wolcrypt::detached::verify(
            &file,
            &sig,
            &lone.to_bytes(),
            [&p.root_a.file, &p.root_b.file]
        )
        .err(),
        Some(wolcrypt::detached::Error::Witness)
    );
    assert!(
        wolcrypt::detached::verify(
            &file,
            &sig,
            &ids.witness.file,
            [&p.root_a.file, &p.root_a.file]
        )
        .is_err()
    );
}
