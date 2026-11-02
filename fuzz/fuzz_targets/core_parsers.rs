// SPDX-License-Identifier: GPL-2.0-or-later
//! Text parsers in wol_core that root reads from files or programs (R2C-21): the release
//! signature hex (`update::check::unhex`, CORE-7), pacman.conf's `DBPath` and the folded-layers
//! list (`update::layers`, read by `wol layers clear` at boot), and `pw-top` output
//! (`audio::parse_pw_top_xruns`). Never panic; what they return keeps their contracts. The input
//! is split at NUL bytes into the parts the two-argument parsers take.
#![no_main]
use libfuzzer_sys::fuzz_target;
use wol_core::audio::parse_pw_top_xruns;
use wol_core::update::{check, layers};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut parts = text.splitn(3, '\0');
    let (a, b, c) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
    );
    if let Some(bytes) = check::unhex(a) {
        assert_eq!(bytes.len() * 2, a.len());
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()), "{a}");
    }
    let db = layers::db_path(a);
    assert!(
        db.starts_with('/') && !db.split('/').any(|p| p == ".."),
        "{db}"
    );
    for name in layers::to_clear(a, b) {
        assert!(layers::is_layer_name(&name), "{name}");
        assert!(b.lines().any(|l| l.trim() == name));
    }
    let next = layers::next_index(a, b, c);
    assert!(next >= 1);
    // Never a name the counter or a folded list already holds (u32::MAX: saturated, unreachable).
    assert!(
        next == u32::MAX
            || !a
                .lines()
                .chain(c.lines())
                .any(|l| l.trim() == layers::name(next))
    );
    for step in layers::clear_steps(&layers::to_clear(a, b)) {
        assert!(!step.is_empty());
    }
    let _ = layers::Record::parse(a);
    let _ = parse_pw_top_xruns(&text);
});
