// SPDX-License-Identifier: BSD-2-Clause
//! The store `wolos/keyring/1` (ADR 0054 section 2-4): what opens, what does not, and what a
//! changed byte, a dropped record or a downgraded header does.
mod support;
use std::collections::BTreeMap;
use support::*;
use wolkeyring::Error;
use wolkeyring::item::{AppId, Class};
use wolkeyring::store::Keyring;

fn q(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

/// A keyring with two items written to its file.
fn with_items() -> (Vec<u8>, String, String) {
    let (mut k, mut file, mut rng) = new_keyring(1);
    let (a, r1) = k
        .put(
            &mut rng,
            meta("site", &[("url", "a.example")], Class::WhenUnlocked),
            b"hunter2",
        )
        .unwrap();
    let (b, r2) = k
        .put(
            &mut rng,
            meta("vpn", &[("vpn", "u1")], Class::AfterFirstUnlock),
            b"wgkey",
        )
        .unwrap();
    file.extend(r1);
    file.extend(r2);
    (file, a, b)
}

#[test]
fn every_rule_opens_and_the_items_come_back() {
    let (file, a, b) = with_items();
    for p in [pw(), rec(), tpm()] {
        let mut k = Keyring::parse(&file).unwrap();
        assert!(!k.first_unlocked());
        k.unlock(&[p]).unwrap();
        assert_eq!(&k.secret(&a, None).unwrap()[..], b"hunter2");
        assert_eq!(&k.secret(&b, None).unwrap()[..], b"wgkey");
        assert_eq!(k.search(&q(&[("url", "a.example")])).len(), 1);
        assert_eq!(k.search(&q(&[("url", "b.example")])).len(), 0);
    }
}

#[test]
fn a_wrong_password_opens_nothing() {
    let (file, _, _) = with_items();
    let mut k = Keyring::parse(&file).unwrap();
    let bad = (
        1,
        wolkeyring::factor::Presented::Typed(zeroize::Zeroizing::new("wrong password".into())),
    );
    assert_eq!(k.unlock(&[bad]), Err(Error::WrongFactors));
    assert!(!k.first_unlocked());
}

#[test]
fn a_changed_record_byte_is_found() {
    let (file, _, _) = with_items();
    let last = file.len() - 5;
    let mut bad = file.clone();
    bad[last] ^= 0x40;
    let mut k = Keyring::parse(&bad).unwrap();
    assert_eq!(k.unlock(&[pw()]), Err(Error::Damaged("record")));
}

#[test]
fn a_dropped_or_reordered_record_is_found() {
    let (mut k, mut file, mut rng) = new_keyring(2);
    let head = file.len();
    let mut recs = Vec::new();
    for i in 0..3 {
        let (_, r) = k
            .put(
                &mut rng,
                meta(
                    &format!("i{i}"),
                    &[("n", &i.to_string())],
                    Class::WhenUnlocked,
                ),
                b"s",
            )
            .unwrap();
        recs.push(r);
    }
    // Drop the middle one.
    let mut dropped = file[..head].to_vec();
    dropped.extend(&recs[0]);
    dropped.extend(&recs[2]);
    assert!(matches!(
        Keyring::parse(&dropped).unwrap().unlock(&[pw()]),
        Err(Error::Damaged(_))
    ));
    // Swap two.
    let mut swapped = file[..head].to_vec();
    swapped.extend(&recs[1]);
    swapped.extend(&recs[0]);
    assert!(matches!(
        Keyring::parse(&swapped).unwrap().unlock(&[pw()]),
        Err(Error::Damaged(_))
    ));
    for r in &recs {
        file.extend(r);
    }
    assert_eq!(
        Keyring::parse(&file)
            .map(|mut k| k.unlock(&[pw()]).map(|_| k.len()))
            .unwrap(),
        Ok(3)
    );
}

fn edit_header(file: &[u8], f: impl Fn(&mut serde_json::Value)) -> Vec<u8> {
    let n = u32::from_be_bytes([file[0], file[1], file[2], file[3]]) as usize;
    let mut h: serde_json::Value = serde_json::from_slice(&file[4..4 + n]).unwrap();
    f(&mut h);
    let hb = serde_json::to_vec(&h).unwrap();
    let mut out = (hb.len() as u32).to_be_bytes().to_vec();
    out.extend(hb);
    out.extend(&file[4 + n..]);
    out
}

#[test]
fn a_changed_header_is_found_at_unlock() {
    let (file, _, _) = with_items();
    let bad = edit_header(&file, |h| h["settings"]["lock_after_secs"] = 0.into());
    let mut k = Keyring::parse(&bad).unwrap();
    assert_eq!(k.unlock(&[pw()]), Err(Error::Damaged("keyring header")));
}

#[test]
fn a_downgraded_kdf_or_format_is_refused() {
    let (file, _, _) = with_items();
    let weak = edit_header(&file, |h| h["factors"][0]["kdf"]["rounds"] = 1000.into());
    assert!(matches!(Keyring::parse(&weak), Err(Error::Unsupported(_))));
    let fewer = edit_header(&file, |h| {
        h["factors"][0]["kdf"]["rounds"] = (wolcrypt::kdf::MIN_ROUNDS + 1).into()
    });
    assert_eq!(
        Keyring::parse(&fewer).unwrap().unlock(&[pw()]),
        Err(Error::WrongFactors)
    );
    let old = edit_header(&file, |h| h["format"] = "wolos/keyring/0".into());
    assert!(matches!(Keyring::parse(&old), Err(Error::Unsupported(_))));
    let classical = edit_header(&file, |h| h["format"] = "gnome-keyring".into());
    assert!(matches!(
        Keyring::parse(&classical),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn locking_keeps_only_after_first_unlock_secrets() {
    let (file, a, b) = with_items();
    let mut k = Keyring::parse(&file).unwrap();
    k.unlock(&[pw()]).unwrap();
    k.lock();
    assert_eq!(k.secret(&a, None), Err(Error::Locked));
    assert_eq!(&k.secret(&b, None).unwrap()[..], b"wgkey");
    assert_eq!(
        k.search(&q(&[("url", "a.example")])).len(),
        1,
        "the index stays"
    );
    k.seal();
    assert_eq!(k.secret(&b, None), Err(Error::Locked));
    assert!(k.search(&q(&[])).is_empty());
}

#[test]
fn an_app_off_the_access_list_is_denied_until_allowed() {
    let (mut k, _, mut rng) = new_keyring(3);
    let helium = AppId {
        exe: "/usr/bin/helium".into(),
        sha256: Some("aa".into()),
        app: None,
        signer: None,
    };
    let other = AppId {
        exe: "/usr/bin/python3".into(),
        sha256: Some("bb".into()),
        app: None,
        signer: None,
    };
    let mut m = meta("site", &[("url", "x")], Class::WhenUnlocked);
    m.acl.push(helium.clone());
    let (id, _) = k.put(&mut rng, m, b"pw").unwrap();
    assert_eq!(&k.secret(&id, Some(&helium)).unwrap()[..], b"pw");
    assert_eq!(k.secret(&id, Some(&other)), Err(Error::Denied));
    k.allow(&mut rng, &id, other.clone()).unwrap();
    assert_eq!(&k.secret(&id, Some(&other)).unwrap()[..], b"pw");
}

#[test]
fn the_chip_alone_cannot_export_or_change_the_policy() {
    let (file, _, _) = with_items();
    let mut k = Keyring::parse(&file).unwrap();
    k.unlock(&[tpm()]).unwrap();
    assert!(!k.strong());
    assert!(matches!(k.export_payload(1, None), Err(Error::Policy(_))));
    let mut rng = wolkeyring::rand::Counter::new(9);
    let f = vec![typed(1, wolkeyring::factor::Kind::Password, PASSWORD)];
    assert!(matches!(
        k.set_policy(&mut rng, f, &[vec![1]]),
        Err(Error::Policy(_))
    ));
    k.unlock(&[pw()]).unwrap();
    assert!(k.strong());
    assert_eq!(k.export_payload(1, None).unwrap().items.len(), 2);
}

#[test]
fn a_policy_change_rewrites_and_old_factors_stop_working() {
    let (file, a, _) = with_items();
    let mut k = Keyring::parse(&file).unwrap();
    k.unlock(&[pw()]).unwrap();
    let mut rng = wolkeyring::rand::Counter::new(10);
    // Password + PIN-with-chip; the recovery code is removed.
    let f = vec![
        typed(1, wolkeyring::factor::Kind::Password, PASSWORD),
        sealed(3, wolkeyring::factor::Kind::Tpm, [7; 32]),
        typed(4, wolkeyring::factor::Kind::Pin, "246810"),
    ];
    let new = k.set_policy(&mut rng, f, &[vec![1], vec![3, 4]]).unwrap();
    let pin = (
        4,
        wolkeyring::factor::Presented::Typed(zeroize::Zeroizing::new("246810".into())),
    );
    let mut k2 = Keyring::parse(&new).unwrap();
    assert_eq!(k2.unlock(&[rec()]), Err(Error::WrongFactors));
    assert_eq!(
        k2.unlock(&[tpm()]),
        Err(Error::WrongFactors),
        "the chip alone is no longer a rule"
    );
    k2.unlock(&[tpm(), pin]).unwrap();
    assert_eq!(&k2.secret(&a, None).unwrap()[..], b"hunter2");
    // A PIN alone is refused as a rule.
    let f = vec![
        typed(1, wolkeyring::factor::Kind::Password, PASSWORD),
        typed(4, wolkeyring::factor::Kind::Pin, "246810"),
    ];
    k.unlock(&[pw()]).unwrap();
    assert!(matches!(
        k.set_policy(&mut rng, f, &[vec![1], vec![4]]),
        Err(Error::Policy(_))
    ));
}

#[test]
fn records_of_an_older_epoch_do_not_open_after_compaction() {
    let (mut k, file, mut rng) = new_keyring(4);
    let (_, r) = k
        .put(
            &mut rng,
            meta("a", &[("n", "1")], Class::WhenUnlocked),
            b"1",
        )
        .unwrap();
    let compacted = k.compact(&mut rng).unwrap();
    let mut k2 = Keyring::parse(&compacted).unwrap();
    k2.unlock(&[pw()]).unwrap();
    assert_eq!(k2.len(), 1);
    // The old file's record appended to the new file (a replay) does not open.
    let mut replay = compacted.clone();
    replay.extend(&r);
    assert!(matches!(
        Keyring::parse(&replay).unwrap().unlock(&[pw()]),
        Err(Error::Damaged(_))
    ));
    drop(file);
}

#[test]
fn import_merges_newer_and_keeps_newer_local() {
    let (mut a, _, mut ra) = new_keyring(6);
    let (mut b, _, mut rb) = new_keyring(7);
    let mut m = meta("site", &[("url", "x")], Class::WhenUnlocked);
    m.modified = 20;
    a.put(&mut ra, m.clone(), b"new").unwrap();
    a.put(
        &mut ra,
        meta("only-a", &[("url", "y")], Class::WhenUnlocked),
        b"y",
    )
    .unwrap();
    let mut dev = meta("device", &[("url", "z")], Class::WhenUnlocked);
    dev.device_only = true;
    a.put(&mut ra, dev, b"z").unwrap();
    m.modified = 15;
    b.put(&mut rb, m.clone(), b"old").unwrap();
    let p = a.export_payload(30, None).unwrap();
    assert_eq!(p.items.len(), 2, "this-device-only stays home");
    let ((added, replaced, kept), _) = b.import(&mut rb, &p).unwrap();
    assert_eq!((added, replaced, kept), (1, 1, 0));
    let id = b.search(&q(&[("url", "x")]))[0].id.clone();
    assert_eq!(&b.secret(&id, None).unwrap()[..], b"new");
    let ((added, replaced, kept), _) = b.import(&mut rb, &p).unwrap();
    assert_eq!(
        (added, replaced, kept),
        (0, 0, 2),
        "a second import changes nothing"
    );
}

#[test]
fn bounds_are_enforced_before_allocation() {
    let (mut k, _, mut rng) = new_keyring(8);
    let big = vec![0u8; wolkeyring::item::MAX_SECRET + 1];
    assert_eq!(
        k.put(&mut rng, meta("a", &[], Class::WhenUnlocked), &big)
            .err(),
        Some(Error::Limit("secret size"))
    );
    let mut huge = vec![0u8; 4];
    huge[..4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(Keyring::parse(&huge).is_err());
    assert!(Keyring::parse(&[]).is_err());
}
