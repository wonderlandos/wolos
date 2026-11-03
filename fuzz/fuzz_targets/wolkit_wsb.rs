// SPDX-License-Identifier: GPL-2.0-or-later
//! libWolkit style tables (`WSB1`, ADR 0039) under libFuzzer: a table from outside is untrusted;
//! loading it never panics and never reads or allocates beyond the bounds, and a table that loads
//! writes back to bytes that load to the same table.
#![no_main]
use libfuzzer_sys::fuzz_target;
use wolkit::style::wsb;

fuzz_target!(|data: &[u8]| {
    let _ = wolkit::style::load(data);
    if let Ok(t) = wsb::load(data) {
        let bytes = wsb::write(&t).expect("a loaded table writes back");
        assert_eq!(wsb::load(&bytes), Ok(t), "a table changed on a round trip");
    }
});
