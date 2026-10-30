//! wold call decoder and Kit host wire (ADR 0020) under libFuzzer. Same invariants as
//! `crates/shared/tests/portal_fuzz.rs`: never panic; anything accepted is valid and round-trips.
#![no_main]
use libfuzzer_sys::fuzz_target;
use shared::portal::{Answer, Call};
use shared::view::{FromHost, ToHost};

fuzz_target!(|data: &[u8]| {
    if let Ok(c) = Call::decode(data) {
        assert!(c.validate().is_ok());
        assert_eq!(Call::decode(c.encode().as_bytes()), Ok(c));
    }
    let text = String::from_utf8_lossy(data);
    if let Some(m) = ToHost::parse(&text) {
        assert_eq!(ToHost::parse(&m.to_line()), Some(m));
    }
    if let Some(e) = FromHost::parse(&text) {
        assert_eq!(FromHost::parse(&e.to_line()), Some(e));
    }
    let a = Answer::parse(&text);
    assert_eq!(a.ok, a.code.is_none());
});
