// SPDX-License-Identifier: GPL-2.0-or-later
//! libWolkit pictures (PNG, JPEG, EXIF orientation; WK-image) under libFuzzer: a picture file is
//! untrusted; reading its header and decoding it never panic, never allocate past the limits
//! checked from the header, and a picture that decodes has the size its header promised.
#![no_main]
use libfuzzer_sys::fuzz_target;
use wolkit::image::{decode, exif};

fuzz_target!(|data: &[u8]| {
    let _ = exif::orientation(data);
    let Ok(h) = decode::probe(data) else {
        return;
    };
    if let Ok(p) = decode::decode(data, 256) {
        assert_eq!((p.width, p.height), (h.width, h.height), "the header's size");
        assert!(p.levels[0].w <= 256 && p.levels[0].h <= 256, "the cap");
    }
});
