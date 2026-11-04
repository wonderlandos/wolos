// SPDX-License-Identifier: GPL-2.0-or-later
//! Revocation lists and the memory of seen revocations (`woltrust::revoked`): `parse` and
//! `Seen::parse` over any bytes, then the memory folded into a store and asked about a serial.
//! Never panic; a list that parses is within its caps; adding a list to the memory never makes
//! it shorter.
#![no_main]
use libfuzzer_sys::fuzz_target;
use wol_core::woltrust::revoked::{self, RevList, Seen};

fuzz_target!(|data: &[u8]| {
    if let Ok(l) = revoked::parse(data) {
        assert!(l.entries.len() <= wol_core::woltrust::MAX_REVOKED_ENTRIES);
        assert!(l.issued < l.expires);
        let mut seen = Seen::default();
        let rl = RevList {
            list: l,
            signer: "0123456789abcdef".into(),
            signer_is_root: false,
        };
        let before = seen.entries.len();
        seen.add(&rl);
        assert!(seen.entries.len() >= before);
        assert!(seen.entries.iter().all(|e| e.at <= rl.list.issued));
        let again = Seen::parse(&seen.to_bytes()).expect("the memory reads back");
        assert_eq!(again, seen);
    }
    if let Ok(s) = Seen::parse(data) {
        let mut store = wol_core::woltrust::Store::default();
        store.remember(&s);
        let _ = wol_core::woltrust::verify::withdrawn(&store, "0123456789abcdef");
    }
});
