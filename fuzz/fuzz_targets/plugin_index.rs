// SPDX-License-Identifier: GPL-2.0-or-later
//! The plugin index (ADR 0031 W4): `wopkg::index::plugins::parse` of `index.json` bytes and the
//! bounded `.wolplug` unpack. Never panic; every listed entry keeps the entry rules, ids are
//! unique, and an unpacked tar stays under its cap.
#![no_main]
use libfuzzer_sys::fuzz_target;
use wopkg::index::plugins::{MAX_ENTRIES, parse, unpack};

fuzz_target!(|data: &[u8]| {
    if let Ok(i) = parse(data) {
        assert!(i.plugins.len() <= MAX_ENTRIES);
        for (n, e) in i.plugins.iter().enumerate() {
            assert!(e.is_valid(), "{e:?}");
            assert!(
                i.plugins[..n].iter().all(|p| p.id != e.id),
                "duplicate {}",
                e.id
            );
            let _ = e.permission_labels();
        }
        let _ = i.search(&String::from_utf8_lossy(&data[..data.len().min(16)]));
    }
    if let Ok(t) = unpack(data) {
        assert!(t.len() as u64 <= wol_core::package::WOLPLUG_MAX_TAR);
    }
});
