// SPDX-License-Identifier: GPL-2.0-or-later
//! Key files (audit 2 L5): `wolkey::name_of` and `wolkey::unlock` of a `.wolkey`, and
//! `KeyMaterial::parse` of plain key material, over any bytes. Never panic; `unlock` under a name
//! the file does not carry is refused before any key derivation; parsed key material writes back
//! to text that parses to the same key.
#![no_main]
use libfuzzer_sys::fuzz_target;
use wol_core::woltrust::key::KeyMaterial;
use wol_core::woltrust::wolkey::{self, KeyError};

fuzz_target!(|data: &[u8]| {
    let name = wolkey::name_of(data);
    let other = "0".repeat(128);
    if name.as_deref().is_ok_and(|n| n != other) {
        assert_eq!(
            wolkey::unlock(data, &other, "a passphrase").err(),
            Some(KeyError::WrongName)
        );
    } else if name.is_err() {
        assert!(wolkey::unlock(data, &other, "a passphrase").is_err());
    }
    if let Ok(k) = KeyMaterial::parse(data) {
        let back = KeyMaterial::parse(&k.to_text()).expect("written key material parses");
        assert_eq!(back.key_id(), k.key_id());
    }
});
