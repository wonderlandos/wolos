// SPDX-License-Identifier: BSD-2-Clause
//! Shared builders for the keyring tests.
#![allow(dead_code)] // helpers that each test file uses a part of
use std::collections::BTreeMap;
use wolkeyring::factor::{Factor, Kind, Presented};
use wolkeyring::item::{Class, Meta};
use wolkeyring::kdf::Kdf;
use wolkeyring::rand::Counter;
use wolkeyring::store::Keyring;
use wolkeyring::wire;
use zeroize::Zeroizing;

pub const PASSWORD: &str = "correct horse battery";
pub const RECOVERY: &str = "0123456789ABCDEFGHJKMNPQRS";

pub fn typed(id: u8, kind: Kind, input: &str) -> (Factor, Zeroizing<[u8; 32]>) {
    let f = Factor {
        id,
        kind,
        label: format!("{kind:?}"),
        kdf: Some(Kdf::Pbkdf2 {
            rounds: wolcrypt::kdf::MIN_ROUNDS,
        }),
        salt: Some(wire::b64(&[id; 16])),
        sealed: None,
        credential: None,
        hmac_salt: None,
    };
    let s = f
        .secret(&Presented::Typed(Zeroizing::new(input.into())))
        .unwrap();
    (f, s)
}

pub fn sealed(id: u8, kind: Kind, bytes: [u8; 32]) -> (Factor, Zeroizing<[u8; 32]>) {
    let f = Factor {
        id,
        kind,
        label: format!("{kind:?}"),
        kdf: None,
        salt: None,
        sealed: Some(wire::b64(b"systemd-creds blob")),
        credential: None,
        hmac_salt: None,
    };
    let s = f.secret(&Presented::Bytes(Zeroizing::new(bytes))).unwrap();
    (f, s)
}

pub fn pw() -> (u8, Presented) {
    (1, Presented::Typed(Zeroizing::new(PASSWORD.into())))
}

pub fn rec() -> (u8, Presented) {
    (2, Presented::Typed(Zeroizing::new(RECOVERY.into())))
}

pub fn tpm() -> (u8, Presented) {
    (3, Presented::Bytes(Zeroizing::new([7; 32])))
}

/// Password (1), recovery (2), TPM (3); rules {1}, {2}, {3}.
pub fn new_keyring(seed: u8) -> (Keyring, Vec<u8>, Counter) {
    let mut rng = Counter::new(seed);
    let f = vec![
        typed(1, Kind::Password, PASSWORD),
        typed(2, Kind::Recovery, RECOVERY),
        sealed(3, Kind::Tpm, [7; 32]),
    ];
    let (k, file) = Keyring::create(&mut rng, "login", f, &[vec![1], vec![2], vec![3]]).unwrap();
    (k, file, rng)
}

pub fn meta(label: &str, attrs: &[(&str, &str)], class: Class) -> Meta {
    Meta {
        id: String::new(),
        collection: "login".into(),
        label: label.into(),
        attrs: attrs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect::<BTreeMap<_, _>>(),
        content_type: "text/plain".into(),
        created: 10,
        modified: 10,
        class,
        device_only: false,
        acl: Vec::new(),
    }
}
