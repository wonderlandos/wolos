// SPDX-License-Identifier: BSD-2-Clause
//! Regression tests for TRUST audit 2 (`.claude/v012/trust/audit-2.md`), woltrust and wolcrypt
//! part (brief A). Each test first reproduces the audit's repro (R1-R5 or the finding's test idea)
//! and asserts the fixed behaviour. The vectors `emergency-index-in-db`,
//! `emergency-section-future-dated`, `list-eternal`, `witness-without-at-refused`,
//! `rkdb/rkdb-eternal` and `rkdb/rkdb-checkpoint-one-source` replay the same cases for other
//! implementations.
use woltrust::cert::Cert;
use woltrust::pki::{DAY, NOW, Pki};
use woltrust::revoked::{self, Entry, ListText, Reason, RevList, Seen, SeenEntry};
use woltrust::rkdb::{self, Checkpoint};
use woltrust::store::HeldDb;
use woltrust::vectors_ids::{Ids, db, entry};
use woltrust::{Code, Input, Purpose, Store, Trust, Wsig, make, witness};

fn store(p: &Pki, ids: &Ids) -> Store {
    let none: [(&[u8], &[u8]); 0] = [];
    Store::load(
        &[&p.root_a.file, &p.root_b.file],
        &[
            &p.index.file,
            &ids.index.file,
            &ids.witness.file,
            &ids.authors.file,
        ],
        &none,
        NOW,
        true,
    )
}

fn by_b(p: &Pki, issued: u64, entries: &[rkdb::Entry]) -> Vec<u8> {
    rkdb::RootSection::make(
        u32::try_from(issued).unwrap(),
        entries,
        &p.root_b.body.serial,
        &p.root_b.secret,
    )
    .unwrap()
}

/// What a file the derived Index signed (`Purpose::Index`) is worth in `s`.
fn index_file(s: &Store, ids: &Ids) -> (Trust, Code) {
    let wsig = make::wsig(
        &ids.index.body.serial,
        &ids.index.secret(),
        Purpose::Index,
        b"index.json",
    );
    let o = woltrust::verify(
        s,
        &Input {
            purpose: Purpose::Index,
            content: b"index.json",
            app_id: None,
            wsig: Some(&wsig),
            chain: None,
            first_seen: None,
            now: NOW,
        },
    );
    (o.trust, o.code)
}

#[test]
fn h1_a_section_withdrawing_the_index_is_kept_from_the_database_that_index_signed() {
    let p = Pki::new();
    let ids = Ids::new(&p);
    let section = by_b(
        &p,
        NOW - DAY,
        &[entry(
            &ids.index.body.serial,
            NOW - DAY,
            Reason::KeyCompromise,
        )],
    );
    let carried = db(&ids, 7, NOW - 3600, &[], Some(&section), [0; 32]);
    // R1: the database is refused as before ...
    let mut s = store(&p, &ids);
    assert_eq!(s.load_rkdb(&carried, NOW), Err(rkdb::Error::BadSignature));
    assert!(s.dbs.is_empty());
    // ... but the section is kept now (it was dropped with the database: 0 sections).
    assert_eq!(s.root_sections.len(), 1);
    assert_eq!(
        index_file(&s, &ids),
        (Trust::Withdrawn, Code::RevokedKeyCompromise)
    );
    // The same section alone (a bare `.rkdr`) passes too, and is recognised by its magic.
    assert_eq!(rkdb::kind_of(&section), Some(rkdb::FileKind::RootSection));
    assert_eq!(rkdb::kind_of(&carried), Some(rkdb::FileKind::Db));
    let mut alone = store(&p, &ids);
    alone
        .load_root_section(&section, NOW)
        .expect("a bare .rkdr loads");
    assert_eq!(index_file(&alone, &ids).0, Trust::Withdrawn);
    // Loading it twice keeps one copy.
    alone.load_root_section(&section, NOW).expect("again");
    assert_eq!(alone.root_sections.len(), 1);
    // A database the Index got wrong in another way (Index signature flipped) still delivers it.
    let mut broken = carried.clone();
    let last = broken.len() - 1;
    broken[last] ^= 1;
    let mut s2 = store(&p, &ids);
    assert!(s2.load_rkdb(&broken, NOW).is_err());
    assert_eq!(s2.root_sections.len(), 1);
    // A forged section (root B's bytes, claimed by the Index) is never salvaged.
    let mut forged = by_b(
        &p,
        NOW - DAY,
        &[entry(
            &ids.authors.body.serial,
            NOW - DAY,
            Reason::KeyCompromise,
        )],
    );
    let at = forged.len() - rkdb::ROOT_SIG;
    forged[at..at + 64].copy_from_slice(&rkdb::id_bytes(&ids.index.body.serial).unwrap());
    let with_forged = db(&ids, 8, NOW - 3600, &[], Some(&forged), [0; 32]);
    let mut s3 = store(&p, &ids);
    assert!(s3.load_rkdb(&with_forged, NOW).is_err());
    assert!(s3.root_sections.is_empty());
    assert!(s3.salvage_root_section(&with_forged, NOW).is_none());
}

#[test]
fn m1_eternal_lists_and_databases_are_refused() {
    // R3: `expires = u32::MAX` loaded and stayed fresh ten years on.
    let v = rkdb_vectors_named("rkdb-eternal");
    let eternal = woltrust::b64::decode(v.new.as_deref().unwrap(), 1 << 24).unwrap();
    assert_eq!(
        rkdb::Db::parse(&eternal).err(),
        Some(rkdb::Error::Malformed)
    );
    // At the bound itself (24 h + 24 h skew) a database still parses.
    let p = Pki::new();
    let ids = Ids::new(&p);
    let ok = rkdb::make(
        &rkdb::Make {
            serial: 1,
            issued: u32::try_from(NOW).unwrap(),
            expires: u32::try_from(NOW + rkdb::MAX_LIFETIME).unwrap(),
            prev_hash: [0; 32],
            entries: &rkdb::Entries::default(),
            root: None,
        },
        &ids.index.body.serial,
        &ids.index.secret(),
    );
    assert!(ok.is_ok());
    let over = rkdb::make(
        &rkdb::Make {
            serial: 1,
            issued: u32::try_from(NOW).unwrap(),
            expires: u32::try_from(NOW + rkdb::MAX_LIFETIME + 1).unwrap(),
            prev_hash: [0; 32],
            entries: &rkdb::Entries::default(),
            root: None,
        },
        &ids.index.body.serial,
        &ids.index.secret(),
    );
    assert_eq!(
        over.err(),
        Some(rkdb::Error::Malformed),
        "the producer refuses it too"
    );
    // A JSON list fresh for a hundred years.
    let list = |expires: u64| {
        format!(r#"{{"format":1,"serial":1,"issued":{NOW},"expires":{expires},"entries":[]}}"#)
    };
    assert!(revoked::parse(list(NOW + 100 * 365 * DAY).as_bytes()).is_err());
    assert!(revoked::parse(list(NOW + revoked::MAX_LIST_LIFETIME).as_bytes()).is_ok());
    assert!(revoked::parse(list(NOW + revoked::MAX_LIST_LIFETIME + 1).as_bytes()).is_err());
}

fn rkdb_vectors_named(name: &str) -> woltrust::rkdb_vectors::RkdbVector {
    woltrust::rkdb_vectors::generate()
        .unwrap()
        .into_iter()
        .find(|v| v.name == name)
        .unwrap()
}

#[test]
fn m2_one_checkpoint_never_raises_the_floor_and_checkpoints_need_a_witness() {
    let huge = Checkpoint {
        serial: 1 << 63,
        hash: [1; 32],
    };
    // The audit's test idea: one source at serial 2^63 froze every machine (floor raised).
    assert_eq!(rkdb::checkpoints(Some(huge), None, 5).0, 5);
    assert_eq!(rkdb::checkpoints(None, Some(huge), 5).0, 5);
    let other = Checkpoint {
        serial: 6,
        hash: [2; 32],
    };
    assert_eq!(rkdb::checkpoints(Some(huge), Some(other), 5).0, 5);
    assert_eq!(rkdb::checkpoints(Some(other), Some(other), 5).0, 6);
    // A checkpoint counts only with a witness signature beside it.
    let p = Pki::new();
    let ids = Ids::new(&p);
    let s = store(&p, &ids);
    let text = other.to_json();
    let sig = witness::sign(
        &ids.witness.body.serial,
        &ids.witness.secret(),
        text.as_bytes(),
        NOW - 60,
    );
    assert_eq!(
        witness::checkpoint(&s, text.as_bytes(), &sig, NOW),
        Some(other)
    );
    let forged = huge.to_json();
    assert_eq!(witness::checkpoint(&s, forged.as_bytes(), &sig, NOW), None);
    // Signed by the Index (not a witness): refused.
    let by_index = make::wsig(
        &ids.index.body.serial,
        &ids.index.secret(),
        Purpose::Index,
        text.as_bytes(),
    );
    assert_eq!(
        witness::checkpoint(&s, text.as_bytes(), &by_index, NOW),
        None
    );
}

#[test]
fn m3_a_flat_key_is_found_under_both_its_ids() {
    let p = Pki::new();
    let key = [42u8; 32];
    let short = woltrust::request::key_id(&key);
    let long = woltrust::request::legacy_key_id(&key);
    assert_eq!((short.len(), long.len()), (16, 128));
    for serial in [short, long] {
        let mut s = Store::load(
            &[&p.root_a.file],
            &[&p.index.file],
            &[] as &[(&[u8], &[u8])],
            NOW,
            true,
        );
        // Before: only `withdrawn(store, key_id)` was asked, so the 128-hex entry was missed.
        s.lists.push(RevList {
            list: ListText {
                format: 1,
                serial: 1,
                issued: NOW - DAY,
                expires: NOW,
                entries: vec![Entry {
                    serial: serial.clone(),
                    at: NOW - DAY,
                    reason: Reason::KeyCompromise,
                }],
            },
            signer: p.index.body.serial.clone(),
            signer_is_root: false,
        });
        assert_eq!(
            woltrust::verify::key_withdrawn(&s, &key),
            Some(Code::RevokedKeyCompromise),
            "{serial}"
        );
        assert_eq!(woltrust::verify::key_withdrawn(&s, &[43u8; 32]), None);
    }
}

#[test]
fn m4_the_memory_cap_is_loud_and_a_full_memory_reads_back() {
    // R5: 40 000 of 40 001 entries were kept and nothing said so.
    let id = |i: usize| wolcrypt::hash::hex(&wolcrypt::hash::sha512(&i.to_be_bytes()));
    let signer = id(usize::MAX);
    let mut seen = Seen {
        format: 1,
        entries: (0..revoked::MAX_SEEN_ENTRIES)
            .map(|i| SeenEntry {
                serial: id(i),
                at: NOW,
                reason: Reason::KeyCompromise,
                signer: signer.clone(),
                by_root: false,
            })
            .collect(),
    };
    let more = RevList {
        list: ListText {
            format: 1,
            serial: 9,
            issued: NOW,
            expires: NOW + DAY,
            entries: vec![
                Entry {
                    serial: id(0),
                    at: NOW,
                    reason: Reason::KeyCompromise,
                },
                Entry {
                    serial: id(revoked::MAX_SEEN_ENTRIES + 1),
                    at: NOW,
                    reason: Reason::Abuse,
                },
            ],
        },
        signer: signer.clone(),
        signer_is_root: false,
    };
    let r = seen.add(&more);
    assert_eq!(
        (r.added, r.refused),
        (0, 1),
        "held entry skipped, new one refused"
    );
    let mut s = Store::default();
    s.lists.push(more);
    let _ = s.seen(&seen);
    assert!(
        s.notes.iter().any(|n| n.contains("revocation memory full")),
        "{:?}",
        s.notes
    );
    // A full memory of 128-hex ids fits the read cap: it is never written too large to read.
    let bytes = seen.to_bytes();
    assert!(bytes.len() <= revoked::MAX_SEEN_BYTES, "{}", bytes.len());
    assert_eq!(
        Seen::parse(&bytes).map(|x| x.entries.len()),
        Ok(revoked::MAX_SEEN_ENTRIES)
    );
}

#[test]
fn m4_a_rolled_back_state_database_is_left_out_and_hard_entries_are_listed() {
    let p = Pki::new();
    let ids = Ids::new(&p);
    let bob = entry(&ids.bob.body.serial, NOW - 3 * DAY, Reason::KeyCompromise);
    let alice = entry(&p.alice.body.serial, NOW - 2 * DAY, Reason::Superseded);
    let one = db(&ids, 1, NOW - 2 * DAY, &[], None, [0; 32]);
    let h1 = rkdb::Db::parse(&one).unwrap().hash;
    let two = db(&ids, 2, NOW - 3600, &[bob, alice], None, h1);
    // The image holds serial 2; the state file was rolled back to serial 1.
    let mut s = store(&p, &ids);
    s.load_rkdb(&two, NOW).unwrap();
    // Before: `load_rkdb` took it as is (no accept); now the acceptance rules run.
    assert_eq!(s.load_rkdb_accepted(&one, NOW), Err(rkdb::Error::Older));
    assert_eq!(s.dbs.len(), 1);
    assert!(s.notes.iter().any(|n| n.contains("left out")));
    // A state database that keeps everything and is newer is taken.
    let h2 = rkdb::Db::parse(&two).unwrap().hash;
    let three = db(&ids, 3, NOW - 1800, &[bob, alice], None, h2);
    assert_eq!(s.load_rkdb_accepted(&three, NOW), Ok(()));
    // The hard entries a database adds over the one held, for the memory.
    let (n2, _) = s.check_rkdb(&two, NOW).unwrap();
    let (n1, _) = s.check_rkdb(&one, NOW).unwrap();
    let added = Store::hard_added(&n2, Some(&n1));
    assert_eq!(added.list.entries.len(), 1);
    assert_eq!(added.list.entries[0].serial, ids.bob.body.serial);
    assert_eq!(added.signer, ids.index.body.serial);
    assert!(Store::hard_added(&n2, Some(&n2)).list.entries.is_empty());
    let _: &HeldDb = &n2;
}

#[test]
fn m5_a_withdrawn_index_keeps_no_list_whatever_the_load_order() {
    let p = Pki::new();
    let l = ListText {
        format: 1,
        serial: 3,
        issued: NOW - DAY,
        expires: NOW - DAY + woltrust::LIST_LIFETIME,
        entries: Vec::new(),
    };
    let lists = [make::revocation_list(
        &l,
        &p.index.body.serial,
        &p.index.secret,
    )];
    let section = by_b(
        &p,
        NOW - DAY,
        &[entry(
            &p.index.body.serial,
            NOW - DAY,
            Reason::KeyCompromise,
        )],
    );
    let roots = [&p.root_a.file, &p.root_b.file];
    // Before: lists were judged before any section, so the Index's list stayed and fed freshness.
    let mut late = Store::load(&roots, &[&p.index.file], &lists, NOW, true);
    assert_eq!(late.lists.len(), 1);
    late.load_root_section(&section, NOW).unwrap();
    assert!(late.lists.is_empty(), "pruned once the section is in");
    let first = Store::load_with_sections(&roots, &[&p.index.file], &[&section], &lists, NOW, true);
    assert!(first.lists.is_empty(), "refused at load");
    assert_eq!(first.root_sections.len(), 1);
}

#[test]
fn l1_witness_signatures_carry_at_and_only_they_do() {
    let p = Pki::new();
    let ids = Ids::new(&p);
    let s = store(&p, &ids);
    // R2: a witness `.wsig` without `at` verified as wolos with a file-signature line.
    let no_at = make::wsig(
        &ids.witness.body.serial,
        &ids.witness.secret(),
        Purpose::Witness,
        &p.root_a.file,
    );
    let o = witness::verify(&s, &p.root_a.file, &no_at, NOW);
    assert_eq!(
        (o.trust, o.code),
        (Trust::Invalid, Code::SignatureMalformed)
    );
    let good = witness::sign(
        &ids.witness.body.serial,
        &ids.witness.secret(),
        &p.root_a.file,
        NOW,
    );
    assert_eq!(
        witness::verify(&s, &p.root_a.file, &good, NOW).trust,
        Trust::Wolos
    );
    // `at` on any other purpose is refused.
    let pkg = make::wsig(
        &p.alice.body.serial,
        &p.alice.secret,
        Purpose::Package,
        b"x",
    );
    let mut w = Wsig::parse(&pkg).unwrap();
    w.at = Some(NOW);
    let chain = Pki::chain(&[&p.alice, &p.authors]);
    let o = woltrust::verify(
        &s,
        &Input {
            purpose: Purpose::Package,
            content: b"x",
            app_id: Some("alice.notes"),
            wsig: Some(&w.to_bytes()),
            chain: Some(&chain),
            first_seen: None,
            now: NOW,
        },
    );
    assert_eq!(o.code, Code::SignatureMalformed);
    // A witness signature with an origin is refused.
    let mut wo = Wsig::parse(&good).unwrap();
    wo.origin = Some(woltrust::b64::encode(b"{}"));
    assert_eq!(
        witness::verify(&s, &p.root_a.file, &wo.to_bytes(), NOW).code,
        Code::SignatureMalformed
    );
}

#[test]
fn l2_a_root_section_from_the_future_is_refused() {
    let p = Pki::new();
    let ids = Ids::new(&p);
    let s = store(&p, &ids);
    let e = [entry(&ids.authors.body.serial, NOW, Reason::Ceased)];
    // R4: `issued = NOW + 300 d` was accepted.
    let future = by_b(&p, NOW + 300 * DAY, &e);
    let (sec, _) = rkdb::RootSection::parse(&future).unwrap();
    assert_eq!(
        s.check_root_section(&sec, NOW).err(),
        Some(rkdb::Error::Malformed)
    );
    let edge = by_b(&p, NOW + woltrust::verify::CLOCK_SKEW, &e);
    let (sec, _) = rkdb::RootSection::parse(&edge).unwrap();
    assert!(s.check_root_section(&sec, NOW).is_ok(), "inside the skew");
}

#[test]
fn l4_a_delta_with_a_huge_root_length_is_refused_without_wrapping() {
    let v = rkdb_vectors_named("rkdb-delta");
    let b = |x: &Option<String>| woltrust::b64::decode(x.as_deref().unwrap(), 1 << 24).unwrap();
    let (base, mut d) = (b(&v.held), b(&v.delta));
    let n = u32::from_be_bytes(d[54 + rkdb::HEADER..58 + rkdb::HEADER].try_into().unwrap());
    let at = 58 + rkdb::HEADER + usize::try_from(n).unwrap() * rkdb::ENTRY;
    d[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(rkdb::apply(&base, &d).err(), Some(rkdb::Error::Malformed));
    d.truncate(at + 2);
    assert_eq!(rkdb::apply(&base, &d).err(), Some(rkdb::Error::Malformed));
}

#[test]
fn check_intermediate_needs_both_roots_validity_and_no_withdrawal() {
    let p = Pki::new();
    let ids = Ids::new(&p);
    let s = store(&p, &ids);
    let index = Cert::parse(&ids.index.file).unwrap();
    assert_eq!(s.check_intermediate(&index, NOW), Ok(()));
    let mut lone = index.clone();
    lone.cosigs.clear();
    assert_eq!(s.check_intermediate(&lone, NOW), Err(Code::RootSignatures));
    assert_eq!(
        s.check_intermediate(&index, NOW + 10 * 365 * DAY),
        Err(Code::Expired)
    );
    let mut gone = store(&p, &ids);
    gone.load_root_section(
        &by_b(
            &p,
            NOW - DAY,
            &[entry(
                &ids.index.body.serial,
                NOW - DAY,
                Reason::KeyCompromise,
            )],
        ),
        NOW,
    )
    .unwrap();
    assert_eq!(
        gone.check_intermediate(&index, NOW),
        Err(Code::RevokedKeyCompromise)
    );
    let alice = Cert::parse(&p.alice.file).unwrap();
    assert!(
        s.check_intermediate(&alice, NOW).is_err(),
        "a leaf is not an intermediate"
    );
}

#[test]
fn the_detached_check_accepts_the_witness_vector() {
    let p = Pki::new();
    let ids = Ids::new(&p);
    let sig = witness::sign(
        &ids.witness.body.serial,
        &ids.witness.secret(),
        b"file",
        NOW,
    );
    let line = wolcrypt::detached::verify(
        b"file",
        &sig,
        &ids.witness.file,
        [&p.root_a.file, &p.root_b.file],
    )
    .expect("verifies");
    assert!(
        line.contains(&wolcrypt::hash::hex(&wolcrypt::hash::sha512(b"file"))),
        "{line}"
    );
}
