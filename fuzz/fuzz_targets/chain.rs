// SPDX-License-Identifier: GPL-2.0-or-later
//! `CHAIN` members and the whole check (`woltrust::cert::parse_chain`, `woltrust::verify`): the
//! input is split at the first NUL into a `CHAIN` and a `SHA256.wsig`, judged against the TEST
//! store. Never panic; a changed chain or signature never makes a file trusted, made by WolOS or
//! of proven origin unless it is exactly a valid TEST signature.
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;
use wol_core::woltrust::pki::{NOW, Pki};
use wol_core::woltrust::{Input, Purpose, Store, Trust, cert};

static STORE: OnceLock<(Store, Vec<u8>)> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let (store, good) = STORE.get_or_init(|| {
        let p = Pki::new();
        let s = Store::load(
            &[&p.root_a.file],
            &[&p.authors.file, &p.index.file],
            &[(&b""[..], &b""[..])],
            NOW,
            true,
        );
        let w = wol_core::woltrust::make::wsig(
            &p.alice.body.serial,
            &p.alice.secret,
            Purpose::Package,
            b"fuzz content",
        );
        (s, w)
    });
    let _ = cert::parse_chain(data);
    let (chain, wsig) = match data.iter().position(|b| *b == 0) {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (data, &good[..]),
    };
    let o = wol_core::woltrust::verify(
        store,
        &Input {
            purpose: Purpose::Package,
            content: b"fuzz content",
            app_id: Some("alice.notes"),
            wsig: Some(wsig),
            chain: Some(chain),
            first_seen: None,
            now: NOW,
        },
    );
    if wsig != &good[..] {
        assert!(!matches!(
            o.trust,
            Trust::Wolos | Trust::Trusted | Trust::Origin
        ));
    }
});
