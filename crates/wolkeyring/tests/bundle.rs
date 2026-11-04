// SPDX-License-Identifier: BSD-2-Clause
//! Export bundles `wolos/keyring-bundle/1` (ADR 0054 section 7): to a keyring identity or a
//! passphrase, signed by the exporter; what a changed byte, another recipient, a swapped
//! signer or a stripped half does.
mod support;
use support::*;
use wolkeyring::Error;
use wolkeyring::bundle::{self, Identity, Payload, PublicId, To, With};
use wolkeyring::item::Class;
use wolkeyring::kdf::Kdf;
use wolkeyring::rand::Counter;

fn payload() -> Payload {
    let (mut k, _, mut rng) = new_keyring(1);
    k.put(
        &mut rng,
        meta("site", &[("url", "a")], Class::WhenUnlocked),
        b"pw",
    )
    .unwrap();
    k.export_payload(100, None).unwrap()
}

fn kdf() -> Kdf {
    Kdf::Pbkdf2 {
        rounds: wolcrypt::kdf::MIN_ROUNDS,
    }
}

#[test]
fn a_bundle_to_a_keyring_opens_there_only() {
    let mut rng = Counter::new(1);
    let me = Identity::from_seed(&[1; 32]);
    let you = Identity::from_seed(&[2; 32]);
    let them = Identity::from_seed(&[3; 32]);
    let p = payload();
    let b = bundle::seal(&mut rng, &me, &p, To::Recipient(&you.public()), b"").unwrap();
    let o = bundle::open(&b, With::Identity(&you), b"").unwrap();
    assert_eq!(o.payload, p);
    assert_eq!(o.signer, me.public().sign);
    assert_eq!(
        bundle::open(&b, With::Identity(&them), b"").err(),
        Some(Error::WrongFactors)
    );
    assert_eq!(
        bundle::open(&b, With::Passphrase("x"), b"").err(),
        Some(Error::WrongFactors)
    );
    assert!(!bundle::needs_passphrase(&b).unwrap());
}

#[test]
fn every_changed_byte_is_refused() {
    let mut rng = Counter::new(2);
    let me = Identity::from_seed(&[1; 32]);
    let you = Identity::from_seed(&[2; 32]);
    let b = bundle::seal(&mut rng, &me, &payload(), To::Recipient(&you.public()), b"").unwrap();
    // Every 97th byte, and the last ones: header, KEM halves, ciphertext, tag.
    let positions: Vec<usize> = (0..b.len())
        .step_by(97)
        .chain(b.len() - 3..b.len())
        .collect();
    for i in positions {
        let mut x = b.clone();
        x[i] ^= 0x01;
        assert!(
            bundle::open(&x, With::Identity(&you), b"").is_err(),
            "byte {i}"
        );
    }
}

#[test]
fn a_passphrase_bundle_opens_with_that_passphrase_only() {
    let mut rng = Counter::new(3);
    let me = Identity::from_seed(&[1; 32]);
    let p = payload();
    let b = bundle::seal(
        &mut rng,
        &me,
        &p,
        To::Passphrase("a long enough phrase", kdf()),
        b"",
    )
    .unwrap();
    assert!(bundle::needs_passphrase(&b).unwrap());
    assert_eq!(
        bundle::open(&b, With::Passphrase("a long enough phrase"), b"")
            .unwrap()
            .payload,
        p
    );
    assert_eq!(
        bundle::open(&b, With::Passphrase("a long enough phrasf"), b"").err(),
        Some(Error::WrongFactors)
    );
    assert!(matches!(
        bundle::seal(&mut rng, &me, &p, To::Passphrase("short", kdf()), b""),
        Err(Error::Policy(_))
    ));
    assert!(matches!(
        bundle::seal(
            &mut rng,
            &me,
            &p,
            To::Passphrase("a long enough phrase", Kdf::Pbkdf2 { rounds: 1 }),
            b""
        ),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn a_bundle_bound_to_one_transfer_does_not_open_in_another() {
    let mut rng = Counter::new(4);
    let me = Identity::from_seed(&[1; 32]);
    let you = Identity::from_seed(&[2; 32]);
    let b = bundle::seal(
        &mut rng,
        &me,
        &payload(),
        To::Recipient(&you.public()),
        b"transfer A",
    )
    .unwrap();
    assert!(bundle::open(&b, With::Identity(&you), b"transfer B").is_err());
    assert!(bundle::open(&b, With::Identity(&you), b"").is_err());
    assert!(bundle::open(&b, With::Identity(&you), b"transfer A").is_ok());
}

#[test]
fn identities_round_trip_and_a_classical_only_identity_is_refused() {
    let me = Identity::from_seed(&[1; 32]);
    let line = me.public().to_line();
    assert!(line.starts_with("wolos-keyring-id/1 "));
    assert_eq!(PublicId::from_line(&line).unwrap(), me.public());
    // Only the X25519 and Ed25519 halves (no ML-KEM, no ML-DSA): refused.
    let b = me.public().to_bytes();
    let classical = [&b[..32], &b[32 + 1184..32 + 1184 + 32]].concat();
    let l = format!("wolos-keyring-id/1 {}", wolkeyring::wire::b64(&classical));
    assert!(PublicId::from_line(&l).is_err());
    assert_eq!(me.public().fingerprint_text().len(), 39);
}

#[test]
fn not_a_bundle_and_later_modes_are_named() {
    assert_eq!(
        bundle::needs_passphrase(b"hello").err(),
        Some(Error::Malformed("not a keyring bundle"))
    );
    let mut x = bundle::MAGIC.to_vec();
    x.push(9);
    assert!(matches!(
        bundle::needs_passphrase(&x),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn move_end_to_end_with_real_items() {
    use wolkeyring::pairing::{Receiver, Sender};
    let (a, _, _) = new_keyring(10);
    let mut a = a;
    let mut ra = Counter::new(11);
    a.put(
        &mut ra,
        meta("vpn", &[("vpn", "u1")], Class::AfterFirstUnlock),
        b"wg",
    )
    .unwrap();
    let (mut b, _, mut rb) = new_keyring(12);
    let (mut recv, m1) = Receiver::start(&mut rb).unwrap();
    let (mut send, m2) = Sender::start(&mut ra, &m1).unwrap();
    let (m3, s1) = recv.reveal(&m2).unwrap();
    assert_eq!(send.check(&m3).unwrap(), s1);
    let bundle = send
        .seal(
            &mut ra,
            &a.identity().unwrap(),
            &a.export_payload(5, None).unwrap(),
        )
        .unwrap();
    let opened = recv.open(&bundle).unwrap();
    let ((added, _, _), _) = b.import(&mut rb, &opened.payload).unwrap();
    assert_eq!(added, 1);
}
