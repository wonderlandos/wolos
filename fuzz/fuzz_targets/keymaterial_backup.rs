// SPDX-License-Identifier: GPL-2.0-or-later
//! The key backup of `woltrust init` (`init::extract`, audit 2 L5) over any bytes, for every role.
//! Never panic; what comes out is within the caps of a `.wolkey` and a `.wolcert`, and the key
//! file's own name parses or is refused, never a panic.
#![no_main]
use libfuzzer_sys::fuzz_target;
use wol_core::woltrust::{init, wolkey};

fuzz_target!(|data: &[u8]| {
    for role in ["root-a", "root-b", "authors-ca", "packages-ca", "index", ""] {
        if let Ok((_, key, cert)) = init::extract(data, role) {
            assert!(key.len() <= wolkey::MAX_FILE);
            assert!(cert.len() <= wol_core::woltrust::MAX_CERT_FILE);
            let _ = wolkey::name_of(&key);
            let _ = wol_core::woltrust::Cert::parse(&cert);
        }
    }
});
