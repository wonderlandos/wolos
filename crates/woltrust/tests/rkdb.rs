// SPDX-License-Identifier: BSD-2-Clause
//! The revocation database (lead decision 2026-11-04 "RKDB"): every vector in `vectors/rkdb/`
//! replays, the generator reproduces them byte for byte, a delta rebuilds the full file exactly,
//! and the measured costs at a million entries (run with `--ignored --nocapture`, release).
use std::path::PathBuf;
use std::time::Instant;
use woltrust::revoked::Reason;
use woltrust::rkdb::{self, Entries, Entry};
use woltrust::rkdb_vectors::{RkdbVector, check, generate};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vectors/rkdb")
}

#[test]
fn every_rkdb_vector_replays_and_the_generator_reproduces_them() {
    let made = generate().expect("generates");
    assert!(made.len() >= 7);
    for v in &made {
        let p = dir().join(format!("{}.json", v.name));
        let text = std::fs::read_to_string(&p).expect("vector on disk");
        assert_eq!(woltrust::vectors::to_text(v), text, "{}", v.name);
        let back: RkdbVector = serde_json::from_str(&text).expect("parses");
        check(&back).unwrap_or_else(|e| panic!("{e}"));
    }
    let on_disk = std::fs::read_dir(dir()).expect("dir").count();
    assert_eq!(on_disk, made.len(), "no stale vector file");
}

#[test]
fn a_delta_rebuilds_the_full_file_byte_for_byte() {
    let v: Vec<RkdbVector> = generate().expect("generates");
    let d = v
        .iter()
        .find(|x| x.name == "rkdb-delta")
        .expect("delta vector");
    let full = v
        .iter()
        .find(|x| x.name == "rkdb-three-entries")
        .expect("full");
    let b = |s: &Option<String>| woltrust::b64::decode(s.as_deref().unwrap(), 1 << 24).unwrap();
    let rebuilt = rkdb::apply(&b(&d.held), &b(&d.delta)).expect("applies");
    assert_eq!(rebuilt, b(&full.new));
    // Onto another base it is refused.
    assert_eq!(
        rkdb::apply(&b(&full.new), &b(&d.delta)).err(),
        Some(rkdb::Error::BaseMismatch)
    );
}

fn million() -> Vec<Entry> {
    (0..1_000_000u32)
        .map(|i| {
            let h = wolcrypt::hash::sha512(&i.to_be_bytes());
            Entry {
                id: h,
                at: 1_700_000_000 + i,
                reason: if i % 7 == 0 {
                    Reason::KeyCompromise
                } else {
                    Reason::Superseded
                },
            }
        })
        .collect()
}

#[test]
#[ignore = "benchmark: cargo test --release -p woltrust --test rkdb -- --ignored --nocapture"]
fn bench_one_million_entries() {
    let p = woltrust::pki::Pki::new();
    let ids = woltrust::vectors_ids::Ids::new(&p);
    let list = million();
    let t = Instant::now();
    let set = Entries::from_list(&list);
    let sort = t.elapsed();
    let file = rkdb::make(
        &rkdb::Make {
            serial: 1,
            issued: 1_770_000_000,
            expires: 1_770_086_400,
            prev_hash: [0; 32],
            entries: &set,
            root: None,
        },
        &ids.index.body.serial,
        &ids.index.secret(),
    )
    .expect("makes");
    let t = Instant::now();
    let db = rkdb::Db::parse(&file).expect("parses");
    let load = t.elapsed();
    let none: [(&[u8], &[u8]); 0] = [];
    let store = woltrust::Store::load(
        &[p.root_a.file.clone(), p.root_b.file.clone()],
        std::slice::from_ref(&ids.index.file),
        &none,
        1_770_000_100,
        true,
    );
    let t = Instant::now();
    store.check_rkdb(&file, 1_770_000_100).expect("verifies");
    let verify = t.elapsed();
    let t = Instant::now();
    let mut found = 0;
    for e in list.iter().step_by(1000) {
        found += usize::from(db.entries.lookup(&e.id).is_some());
    }
    let lookup = t.elapsed() / 1000;
    assert_eq!(found, 1000);
    let other = Entries::from_list(&list[..500_000]);
    let t = Instant::now();
    let m = rkdb::merge(&db.entries, &other);
    let merge = t.elapsed();
    assert_eq!(m.len(), 1_000_000);
    println!(
        "1e6 entries, {} bytes: sort+encode {sort:?}, parse+check {load:?}, \
         verify (parse + signature) {verify:?}, lookup {lookup:?} each, merge with 5e5 {merge:?}",
        file.len()
    );
}
