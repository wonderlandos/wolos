// SPDX-License-Identifier: GPL-2.0-or-later
//! Per-app web policies (`wol_core::webpolicy`): root's pacman hook acts on what a policy file
//! says, so the decoder never panics, and what it accepts keeps its contracts: every plan line is
//! five one-word fields, every patch target is a plain path under the patch roots, and applying a
//! patch to any bytes either refuses or edits exactly the bytes it names. The input is split at
//! the first NUL into the policy and a file to patch.
#![no_main]
use libfuzzer_sys::fuzz_target;
use wol_core::webpolicy;

fuzz_target!(|data: &[u8]| {
    let (pol, file) = match data.iter().position(|b| *b == 0) {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (data, &b""[..]),
    };
    let Ok(p) = webpolicy::parse(pol) else {
        return;
    };
    for line in webpolicy::plan(&p).lines().skip(1) {
        let f: Vec<&str> = line.split(' ').collect();
        assert_eq!(f.len(), 5, "{line}");
        assert!(f.iter().all(|w| !w.is_empty()), "{line}");
    }
    for x in &p.patches {
        assert!(webpolicy::valid_target(&x.file), "{}", x.file);
        if let Ok(out) = x.apply(file) {
            assert_eq!(webpolicy::sha256_hex(file), x.sha256);
            assert_eq!(out.len() + x.find.len(), file.len() + x.replacement().len());
        }
    }
});
