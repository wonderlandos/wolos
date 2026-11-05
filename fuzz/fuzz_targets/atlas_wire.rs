// SPDX-License-Identifier: GPL-2.0-or-later
//! Atlas Host Protocol lines (`shared::atlas`, ADR 0043 D4): what the browser sends the bridge and
//! what the bridge answers. Never panic; an accepted line re-encodes to a line that decodes to the
//! same value; a refusal never names more than the id it read.
#![no_main]
use libfuzzer_sys::fuzz_target;
use shared::atlas::{MAX_LINE, Reply, Request};

fuzz_target!(|data: &[u8]| {
    match Request::decode(data) {
        Ok(r) => {
            let line = r.encode();
            assert!(line.len() <= MAX_LINE + 1);
            assert_eq!(Request::decode(line.as_bytes()), Ok(r));
        }
        Err(Reply::Error { text, .. }) => assert!(!text.contains('/')),
        Err(other) => panic!("a refusal is an error reply: {other:?}"),
    }
    if let Some(r) = Reply::decode(data) {
        assert_eq!(Reply::decode(r.encode().as_bytes()), Some(r));
    }
});
