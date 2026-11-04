// SPDX-License-Identifier: GPL-2.0-or-later
//! The revocation database (`woltrust::rkdb`): `Db::parse`, `RootSection::parse`, `apply` of the
//! second half onto the first, `merge`, `lookup` and the store's `check_rkdb` over any bytes. Never
//! panic; a parsed database is sorted and within its caps, every entry is found by its id, a merge
//! keeps every entry of both sides, and a delta that applies gives a file that parses.
#![no_main]
use libfuzzer_sys::fuzz_target;
use wol_core::woltrust::rkdb::{self, Db, Entries, RootSection};

fuzz_target!(|data: &[u8]| {
    if let Ok(db) = Db::parse(data) {
        assert!(db.issued < db.expires);
        assert!(db.entries.len() <= rkdb::MAX_ENTRIES);
        let all: Vec<_> = db.entries.iter().collect();
        assert!(all.windows(2).all(|w| w[0].id < w[1].id));
        for e in all.iter().take(64) {
            assert_eq!(db.entries.lookup(&e.id), Some(*e));
        }
        let m = rkdb::merge(&db.entries, &Entries::from_list(&all[..all.len() / 2]));
        assert!(rkdb::covers(&m, &db.entries));
        assert_eq!(m.len(), db.entries.len());
        let _ = db.to_list();
        let store = wol_core::woltrust::Store::default();
        assert!(store.check_rkdb(data, 1_770_000_000).is_err(), "no signer is known");
    }
    if let Ok((s, n)) = RootSection::parse(data) {
        assert!(n <= data.len());
        assert!(s.entries.len() <= rkdb::MAX_ROOT_ENTRIES);
    }
    if data.len() > 2 {
        let cut = usize::from(data[0]) * data.len() / 256;
        let (base, delta) = data[1..].split_at(cut.min(data.len() - 1));
        if let Ok(full) = rkdb::apply(base, delta) {
            assert!(Db::parse(&full).is_ok());
        }
    }
    let _ = rkdb::Checkpoint::parse(data);
});
