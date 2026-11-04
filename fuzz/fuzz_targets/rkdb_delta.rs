// SPDX-License-Identifier: GPL-2.0-or-later
//! Deltas of the revocation database (`rkdb::delta`, audit 2 L5). Input: `u32 base length || base
//! || target`. Never panic; a delta that is made applies onto its base and gives the target byte
//! for byte, and is refused onto any other file.
#![no_main]
use libfuzzer_sys::fuzz_target;
use wol_core::woltrust::rkdb;

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let n = usize::try_from(u32::from_be_bytes([data[0], data[1], data[2], data[3]]))
        .unwrap_or(usize::MAX)
        .min(data.len() - 4);
    let (base, target) = data[4..].split_at(n);
    if let Ok(d) = rkdb::delta(base, target) {
        assert_eq!(rkdb::apply(base, &d).as_deref(), Ok(target), "a delta rebuilds its target");
        assert!(rkdb::apply(target, &d).is_err(), "never onto the target itself");
    }
    if let Ok(full) = rkdb::apply(base, target) {
        assert!(rkdb::Db::parse(&full).is_ok());
    }
});
