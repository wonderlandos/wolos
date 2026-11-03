// SPDX-License-Identifier: GPL-2.0-or-later
//! libWolkit style sheets (`.wolcss`, ADR 0039) under libFuzzer: the compiler never panics on any
//! text, and a sheet it accepts through the public entry point is a table the loader accepts.
#![no_main]
use libfuzzer_sys::fuzz_target;
use wolkit::style::wolcss::Builder;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    let mut b = Builder::new();
    b.add("fuzz.wolcss", src);
    let _ = b.finish();
    if let Ok(table) = wolkit::style::compile(src) {
        assert!(
            wolkit::style::load(&table).is_ok(),
            "the compiler made a table the loader refuses"
        );
    }
});
