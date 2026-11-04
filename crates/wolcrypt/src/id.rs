// SPDX-License-Identifier: BSD-2-Clause
//! Derived identifiers (ADR 0046, lead decision 2026-11-04 "IDS"): nothing is named by a random
//! number. A certificate's id is SHA-512 over its public keys in a fixed order:
//! `ed25519 || ml-dsa-65 || x25519 || ml-kem-768` (signing keys), `ed25519 || slh-dsa-sha2-128s`
//! (roots), absent keys omitted. A file's id is SHA-512 over its `SHA256` member (the hash list the
//! author signed). Written as 128 lowercase hex characters.
//!
//! The parts have fixed lengths and each key set has its own total (64 bytes for a root, 1984 for
//! signing keys, 3200 with encryption keys, 32 for a bare Ed25519 key), so no two key sets
//! concatenate to the same bytes. Anyone can recompute an id from the public keys; claiming one
//! takes the private keys (a signature) and the authority's certificate. Finding other keys with a
//! given id is a second preimage of SHA-512 (about 2^512 classical work, 2^256 with Grover's
//! algorithm); two key sets with the same id is a collision (2^256 classical, at best about 2^171
//! quantum with BHT, which needs more memory than that is worth, so 2^256 is the planning figure).
use crate::sign::{PublicKeys, Second};
use std::string::String;

/// Id length in bytes.
pub const LEN: usize = 64;
/// Id length in hex characters.
pub const HEX_LEN: usize = 128;

/// SHA-512 over the public keys in id order.
pub fn of_keys(keys: &PublicKeys, enc: Option<&crate::kem::EncPublic>) -> [u8; LEN] {
    let second: &[u8] = match &keys.second {
        Second::MlDsa65(k) => &k[..],
        Second::Slh(k) => &k[..],
    };
    match enc {
        Some(e) => {
            crate::hash::sha512_parts(&[&keys.ed25519, second, &e.x25519, &e.ml_kem_768[..]])
        }
        None => crate::hash::sha512_parts(&[&keys.ed25519, second]),
    }
}

/// The id of a bare Ed25519 key (WolOS's previous flat keys): SHA-512 of its 32 bytes.
pub fn of_ed25519(key: &[u8; 32]) -> [u8; LEN] {
    crate::hash::sha512(key)
}

/// The id of a file: SHA-512 of its `SHA256` member.
pub fn of_file(manifest: &[u8]) -> [u8; LEN] {
    crate::hash::sha512(manifest)
}

/// 128 lowercase hex characters.
pub fn to_hex(id: &[u8; LEN]) -> String {
    crate::hash::hex(id)
}

/// True for 128 lowercase hex characters.
pub fn is_hex(s: &str) -> bool {
    s.len() == HEX_LEN
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The bytes of a 128-hex id.
pub fn from_hex(s: &str) -> Option<[u8; LEN]> {
    if !is_hex(s) {
        return None;
    }
    crate::hash::unhex(s)?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sign::{Scheme, SigningKey};

    #[test]
    fn ids_follow_the_key_order_and_differ_per_key_set() {
        let k = SigningKey::from_seed(&[1; 32], Scheme::Signing);
        let e = crate::kem::EncSecret::from_seed(&[1; 32]);
        let with = of_keys(k.public(), Some(e.public()));
        let without = of_keys(k.public(), None);
        assert_ne!(with, without);
        let mut cat = Vec::new();
        cat.extend_from_slice(&k.public().ed25519);
        cat.extend_from_slice(k.public().second.as_bytes());
        assert_eq!(without, crate::hash::sha512(&cat));
        cat.extend_from_slice(&e.public().x25519);
        cat.extend_from_slice(&e.public().ml_kem_768[..]);
        assert_eq!(with, crate::hash::sha512(&cat));
        let r = SigningKey::from_seed(&[1; 32], Scheme::Root);
        assert_ne!(of_keys(r.public(), None), without);
        let h = to_hex(&with);
        assert!(is_hex(&h));
        assert_eq!(from_hex(&h), Some(with));
        assert_eq!(from_hex(&h[..16]), None);
    }
}
