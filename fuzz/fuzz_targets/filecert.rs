// SPDX-License-Identifier: GPL-2.0-or-later
//! File certificates (`woltrust::filecert`, label `wolos/release/1`): `verify` over any bytes
//! against the TEST store. Never panic; nothing random verifies as checked.
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;
use wol_core::woltrust::pki::{NOW, Pki};
use wol_core::woltrust::{Store, filecert};

static STORE: OnceLock<Store> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let store = STORE.get_or_init(|| {
        let p = Pki::new();
        Store::load(
            &[&p.root_a.file],
            &[&p.index.file],
            &[(&b""[..], &b""[..])],
            NOW,
            true,
        )
    });
    let sha = "0".repeat(64);
    let _ = filecert::verify(store, data, &sha);
});
