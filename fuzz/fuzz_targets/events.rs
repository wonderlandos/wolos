// SPDX-License-Identifier: GPL-2.0-or-later
//! wold event bus line (`shared::events::Event`, audit B10): never panic; an accepted event
//! re-encodes to a line that decodes to the same event.
#![no_main]
use libfuzzer_sys::fuzz_target;
use shared::events::Event;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Some(e) = Event::parse(&text) {
        let _ = e.audience();
        assert_eq!(Event::parse(&e.to_line()), Some(e));
    }
});
