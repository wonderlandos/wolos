// SPDX-License-Identifier: GPL-2.0-or-later
//! `.wolplug` checks (ADR 0031 W4): `wol_core::package::members_in` and `verify_wolplug` over any
//! decompressed tar. Never panic; whatever verifies keeps every rule it promises (one folder, the
//! listed files present, plugin.toml parses again, the 1 MB cap).
#![no_main]
use libfuzzer_sys::fuzz_target;
use wol_core::package::{Trust, members_in, public_of, verify_wolplug};

fuzz_target!(|data: &[u8]| {
    let _ = members_in(data);
    // A fixed trusted key: a signature that verifies is a real find, not noise.
    let keys = [public_of(&[7; 32])];
    if let Ok(v) = verify_wolplug(data, &keys) {
        let total: usize = v.files.iter().map(|f| f.1.len()).sum();
        assert!(total as u64 <= shared::plugins::MAX_TOTAL_SIZE);
        assert!(
            v.files
                .iter()
                .all(|(r, _)| shared::plugins::is_plugin_path(r))
        );
        for f in v.manifest.files() {
            assert!(v.files.iter().any(|(r, _)| r == f), "{f} named but missing");
        }
        assert!(shared::plugins::parse(&v.manifest.to_toml()).is_ok());
        assert!(matches!(v.trust, Trust::Signed(0) | Trust::Unsigned));
    }
});
