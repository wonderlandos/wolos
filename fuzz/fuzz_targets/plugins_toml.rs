// SPDX-License-Identifier: GPL-2.0-or-later
//! plugin.toml (`shared::plugins::parse`, the TOML subset; audit B10): never panic; an accepted
//! manifest written back with `to_toml` parses again.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Ok(m) = shared::plugins::parse(&text) {
        let _ = (m.kinds(), m.scripts(), m.files());
        let again = shared::plugins::parse(&m.to_toml());
        assert!(again.is_ok(), "to_toml output does not parse: {again:?}");
    }
    let _ = shared::plugins::check_hypr(&text);
});
