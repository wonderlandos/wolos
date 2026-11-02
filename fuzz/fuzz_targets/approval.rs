// SPDX-License-Identifier: GPL-2.0-or-later
//! Approval sheet wire (`shared::shell::Approval`, `Answer`; audit B10): never panic; an accepted
//! request round-trips through its socket line.
#![no_main]
use libfuzzer_sys::fuzz_target;
use shared::shell::{Answer, Approval};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Some(a) = Approval::parse(&text) {
        let _ = (a.request.describe(), a.request.is_admin());
        let line = a.line();
        let payload = line.split_once(' ').map(|(_, p)| p).unwrap_or_default();
        assert_eq!(Approval::parse(payload), Some(a));
    }
    if let Some(a) = Answer::parse(&text) {
        assert_eq!(Answer::parse(&a.line()), Some(a));
    }
});
