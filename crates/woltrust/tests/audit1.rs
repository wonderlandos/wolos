// SPDX-License-Identifier: BSD-2-Clause
//! Regression tests for TRUST audit 1 (`.claude/v012/trust/audit-1.md`), one per reproducer of
//! `repro/src/main.rs`, each asserting the fixed behaviour. H1 (legacy key) and the memory file of
//! M1 are tested in `wol_core::trust`; the CA half of H2 in `tests/cli.rs` and `vectors/request/`.
use woltrust::pki::{DAY, NOW, Pki};
use woltrust::revoked::{Entry, ListText, Reason, Seen};
use woltrust::{Code, Input, Purpose, Store, Trust, make};

fn list(p: &Pki, serial: u64, issued: u64, entries: Vec<Entry>) -> (Vec<u8>, Vec<u8>) {
    let l = ListText {
        format: 1,
        serial,
        issued,
        expires: issued + woltrust::LIST_LIFETIME,
        entries,
    };
    make::revocation_list(&l, &p.index.body.serial, &p.index.secret)
}

fn check(store: &Store, p: &Pki, first_seen: Option<u64>, now: u64) -> (Trust, Code) {
    let content = b"release signed long before the revocation";
    let wsig = make::wsig(
        &p.alice.body.serial,
        &p.alice.secret,
        Purpose::Package,
        content,
    );
    let chain = Pki::chain(&[&p.alice, &p.authors]);
    let o = woltrust::verify(
        store,
        &Input {
            purpose: Purpose::Package,
            content,
            app_id: Some("alice.notes"),
            wsig: Some(&wsig),
            chain: Some(&chain),
            first_seen,
            now,
        },
    );
    (o.trust, o.code)
}

fn entry(p: &Pki, reason: Reason, at: u64) -> Entry {
    Entry {
        serial: p.alice.body.serial.clone(),
        at,
        reason,
    }
}

fn load(p: &Pki, lists: &[&(Vec<u8>, Vec<u8>)], now: u64) -> Store {
    let pairs: Vec<(&[u8], &[u8])> = lists
        .iter()
        .map(|(a, b)| (a.as_slice(), b.as_slice()))
        .collect();
    Store::load(&[&p.root_a.file], &[&p.index.file], &pairs, now, true)
}

/// A1-A3 held before and still hold: a hard revocation hits old files, stale lists and replays.
#[test]
fn a1_a3_a_hard_revocation_is_final() {
    let p = Pki::new();
    let l1 = list(&p, 1, NOW - 10 * DAY, vec![]);
    let l2 = list(
        &p,
        2,
        NOW - 2 * DAY,
        vec![entry(&p, Reason::KeyCompromise, NOW - DAY)],
    );
    let s = load(&p, &[&l2], NOW);
    assert_eq!(
        check(&s, &p, Some(NOW - 400 * DAY), NOW),
        (Trust::Withdrawn, Code::RevokedKeyCompromise)
    );
    assert_eq!(check(&s, &p, None, NOW + 50 * DAY).0, Trust::Withdrawn);
    let s = load(&p, &[&l1, &l2], NOW);
    assert_eq!(check(&s, &p, None, NOW).0, Trust::Withdrawn);
}

/// A4/A5 (M1): a higher list without the entry, or only an older list, no longer un-revokes once
/// the machine remembers what it saw.
#[test]
fn a4_a5_the_memory_keeps_a_seen_revocation() {
    let p = Pki::new();
    let l1 = list(&p, 1, NOW - 10 * DAY, vec![]);
    let l2 = list(
        &p,
        2,
        NOW - 2 * DAY,
        vec![entry(&p, Reason::KeyCompromise, NOW - DAY)],
    );
    let l3 = list(&p, 3, NOW - DAY, vec![]);
    let seen = load(&p, &[&l2], NOW).seen(&Seen::default());
    for held in [vec![&l1, &l3], vec![&l1]] {
        let mut s = load(&p, &held, NOW);
        assert_eq!(check(&s, &p, None, NOW).0, Trust::Origin, "the gap itself");
        s.remember(&seen);
        assert_eq!(
            check(&s, &p, None, NOW),
            (Trust::Withdrawn, Code::RevokedKeyCompromise)
        );
    }
}

/// A6 (M2): the Index expired since it signed the list; the list still counts.
#[test]
fn a6_an_expired_index_keeps_what_it_withdrew() {
    let p = Pki::new();
    let l2 = list(
        &p,
        2,
        NOW - 2 * DAY,
        vec![entry(&p, Reason::KeyCompromise, NOW - DAY)],
    );
    let late = p.index.body.not_after + 1;
    let s = load(&p, &[&l2], late);
    assert_eq!(s.lists.len(), 1, "{:?}", s.notes);
    assert_eq!(check(&s, &p, Some(NOW), late).0, Trust::Withdrawn);
    // A list dated in the future is not loaded.
    let future = list(&p, 5, NOW + 3 * DAY, vec![]);
    assert!(load(&p, &[&future], NOW).lists.is_empty());
}

/// A7: a soft revocation with `at` in the future takes effect when the list was issued.
#[test]
fn a7_a_future_at_does_not_disable_a_soft_revocation() {
    let p = Pki::new();
    let l4 = list(
        &p,
        4,
        NOW - DAY,
        vec![entry(&p, Reason::Superseded, NOW + 365 * DAY)],
    );
    let s = load(&p, &[&l4], NOW);
    assert_eq!(
        check(&s, &p, Some(NOW), NOW),
        (Trust::Withdrawn, Code::RevokedSuperseded)
    );
    // A file the index saw before the list was issued stays valid.
    assert_eq!(check(&s, &p, Some(NOW - 2 * DAY), NOW).0, Trust::Origin);
}

/// C1/C2 (M3): offline with roots only, or with an old image list, the signature keeps its value
/// and is flagged "not checked for withdrawn certificates".
#[test]
fn c1_c2_offline_verification_keeps_its_value() {
    let p = Pki::new();
    let s = load(&p, &[], NOW);
    assert_eq!(check(&s, &p, None, NOW), (Trust::Origin, Code::ListTooOld));
    let l5 = list(&p, 1, NOW - 38 * DAY, vec![]);
    let s = load(&p, &[&l5], NOW);
    assert_eq!(check(&s, &p, None, NOW), (Trust::Origin, Code::ListTooOld));
}

/// C3 (L3): a clock up to 24 hours behind a certificate's start still counts; more does not.
#[test]
fn c3_a_day_of_clock_skew_is_allowed() {
    let p = Pki::new();
    let l3 = list(&p, 3, p.alice.body.not_before, vec![]);
    let s = load(&p, &[&l3], NOW);
    let nb = p.alice.body.not_before;
    assert_eq!(check(&s, &p, None, nb - 60).1, Code::Ok);
    assert_eq!(check(&s, &p, None, nb - 2 * DAY).1, Code::NotYetValid);
}

/// D1-D3 held and still hold: labels separate purposes, a proof is no file signature, both
/// halves are required.
#[test]
fn d1_d3_signatures_stay_separate_and_hybrid() {
    let p = Pki::new();
    let body = woltrust::Cert::parse(&p.alice.file).unwrap().body_bytes;
    let cert_sig = p.authors.secret.sign(Purpose::Cert, &body);
    let pk = p.authors.secret.public();
    assert!(
        woltrust::crypto::verify_hybrid(
            pk,
            &woltrust::crypto::message(Purpose::Package, &body),
            &cert_sig
        )
        .is_err()
    );
    let proof_msg = woltrust::crypto::message_for(woltrust::request::LABEL, b"x");
    let sig = p.alice.secret.sign_message(&proof_msg);
    assert!(Purpose::ALL.iter().all(|pp| {
        woltrust::crypto::verify_hybrid(
            p.alice.secret.public(),
            &woltrust::crypto::message(*pp, b"x"),
            &sig,
        )
        .is_err()
    }));
    let mut half = p.alice.secret.sign(Purpose::Package, b"m");
    half.second = vec![0; half.second.len()];
    assert!(
        woltrust::crypto::verify_hybrid(
            p.alice.secret.public(),
            &woltrust::crypto::message(Purpose::Package, b"m"),
            &half
        )
        .is_err()
    );
}

/// D4 (L1): a name with a bidi override or a zero-width mark is refused.
#[test]
fn d4_invisible_characters_are_refused_in_names() {
    let p = Pki::new();
    let body = woltrust::Cert::parse(&p.alice.file).unwrap().body_bytes;
    for bad in ["Alice \u{202E}evitaN", "Al\u{200B}ice", "Alice\u{2066}x"] {
        let mut b: serde_json::Value = serde_json::from_slice(&body).unwrap();
        b["subject"]["name"] = serde_json::Value::String(bad.into());
        assert!(
            woltrust::Body::parse(&serde_json::to_vec(&b).unwrap()).is_err(),
            "{bad:?}"
        );
    }
}

/// D5 (H2): the proof binds the login, the name and the namespaces, not only the id and keys.
#[test]
fn d5_the_proof_binds_what_the_ca_copies() {
    let km = woltrust::key::KeyMaterial::new([5; 32], false);
    let p = Pki::new();
    let mut body = p.alice.body.clone();
    body.keys = km.keys_text();
    let claimed = woltrust::request::Claimed::of_body(&body).unwrap();
    let proof = woltrust::request::prove(&km.secret(), &claimed);
    let keys = woltrust::Body::parse(&serde_json::to_vec(&body).unwrap())
        .unwrap()
        .1;
    assert!(woltrust::request::check(&body, &keys, &proof).is_ok());
    body.subject.login = Some("mallory".into());
    assert!(woltrust::request::check(&body, &keys, &proof).is_err());
}
