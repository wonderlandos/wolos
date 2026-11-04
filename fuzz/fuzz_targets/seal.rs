// SPDX-License-Identifier: GPL-2.0-or-later
//! Sealed messages (`wolos/seal/1`, `woltrust::seal`): `peek` and `open` over any bytes with a
//! fixed TEST recipient. Never panic; nothing opens unless it was sealed to that recipient.
#![no_main]
use libfuzzer_sys::fuzz_target;
use wol_core::woltrust::seal::{self, EncSecret, FILE_LABEL};

fuzz_target!(|data: &[u8]| {
    let _ = seal::peek(data);
    let me = EncSecret::from_seed(&[9; 32]);
    if let Ok(plain) = seal::open(&me, "0123456789abcdef", FILE_LABEL, data) {
        assert!(plain.len() < data.len(), "the overhead is never negative");
    }
});
