// SPDX-License-Identifier: GPL-2.0-or-later
//! `.wolcert` files and certificate bodies (ADR 0046, `woltrust::cert`): `Cert::parse` and
//! `Body::parse` over any bytes. Never panic; whatever parses keeps its rules (a serial, a class
//! only on an author, a trusted certificate naming its origin, no invisible characters in names).
#![no_main]
use libfuzzer_sys::fuzz_target;
use wol_core::woltrust::cert::{Body, Cert, Class, Kind, is_invisible, is_serial};

fuzz_target!(|data: &[u8]| {
    let mut bodies = Vec::new();
    if let Ok(c) = Cert::parse(data) {
        assert!(data.len() <= wol_core::woltrust::MAX_CERT_FILE);
        bodies.push(c.body);
    }
    if let Ok((b, _, _)) = Body::parse(data) {
        bodies.push(b);
    }
    for b in bodies {
        assert!(is_serial(&b.serial) && is_serial(&b.issuer));
        assert!(b.not_before < b.not_after);
        let author = matches!(b.kind, Kind::Person | Kind::Org);
        assert!(author || b.class.is_none());
        if b.class == Some(Class::Trusted) {
            assert!(b.origin_cert.as_deref().is_some_and(is_serial));
        }
        let shown = [&b.subject.name, &b.subject.text];
        assert!(shown.iter().all(|s| {
            s.as_deref()
                .is_none_or(|t| !t.chars().any(|c| c.is_control() || is_invisible(c)))
        }));
        assert!(b.subject.text.is_none() || b.kind == Kind::Root);
    }
});
