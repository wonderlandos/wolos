// SPDX-License-Identifier: GPL-2.0-or-later
//! wold control socket (`shared::jobs::JobRequest`, `JobSpec`; audit B10): never panic; an
//! accepted request round-trips through its own line.
#![no_main]
use libfuzzer_sys::fuzz_target;
use shared::jobs::{JobRequest, JobSpec};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Some(r) = JobRequest::parse(&text) {
        let _ = r.needs_root();
        assert_eq!(JobRequest::parse(&r.line()), Some(r));
    }
    let _ = JobSpec::parse(&text);
});
