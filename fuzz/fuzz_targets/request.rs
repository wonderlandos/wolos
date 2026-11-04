// SPDX-License-Identifier: GPL-2.0-or-later
//! Certificate requests (`woltrust::request`): `parse` and `check_proof` over any bytes, the
//! hub's added fields ignored. Never panic; a request whose proof verifies names a person or an
//! organisation with a GitHub id, login and name.
#![no_main]
use libfuzzer_sys::fuzz_target;
use wol_core::woltrust::Kind;
use wol_core::woltrust::request;

fuzz_target!(|data: &[u8]| {
    if let Ok(r) = request::parse(data) {
        if r.check_proof().is_ok() {
            assert!(matches!(r.kind, Kind::Person | Kind::Org));
            assert!(r.subject.github_id.is_some() && r.subject.login.is_some());
        }
    }
});
