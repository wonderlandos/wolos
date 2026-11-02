// SPDX-License-Identifier: GPL-2.0-or-later
//! wopkg's calls to wold (`shared::packages::PackageCall`, read as root; R2C-21): never panic;
//! anything accepted is valid, names its grant and round-trips through its encoding.
#![no_main]
use libfuzzer_sys::fuzz_target;
use shared::packages::PackageCall;

fuzz_target!(|data: &[u8]| {
    let _ = PackageCall::is_package_call(&String::from_utf8_lossy(data));
    if let Ok(c) = PackageCall::decode(data) {
        assert!(c.validate().is_ok());
        assert!(!c.grant().is_empty());
        assert_eq!(PackageCall::decode(c.encode().as_bytes()), Ok(c));
    }
});
