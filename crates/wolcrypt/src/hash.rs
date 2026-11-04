// SPDX-License-Identifier: BSD-2-Clause
//! Hashes: SHA-512 (ids, signed messages, key schedules), SHA-512/256 (the revocation database
//! chain, checkpoints), SHA-256 (the `.wol` member hashes of ADR 0011). Lowercase hex helpers.
use sha2::Digest;
use std::string::String;
use std::vec::Vec;

/// SHA-512 output length.
pub const SHA512_LEN: usize = 64;

/// SHA-512 of `data`.
pub fn sha512(data: &[u8]) -> [u8; 64] {
    sha2::Sha512::digest(data).into()
}

/// SHA-512 over several parts, as if concatenated.
pub fn sha512_parts(parts: &[&[u8]]) -> [u8; 64] {
    let mut h = sha2::Sha512::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// SHA-512/256 (FIPS 180-4: its own initial values, truncated to 32 bytes) of `data`.
pub fn sha512_256(data: &[u8]) -> [u8; 32] {
    sha2::Sha512_256::digest(data).into()
}

/// SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    sha2::Sha256::digest(data).into()
}

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(D[usize::from(b >> 4)]));
        s.push(char::from(D[usize::from(b & 15)]));
    }
    s
}

/// Bytes of lowercase hex text; None for odd length, uppercase or any other character.
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return None;
    }
    let v = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    b.chunks(2)
        .map(|p| Some(v(p[0])? << 4 | v(p[1])?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fips_180_4_abc() {
        assert_eq!(
            hex(&sha512(b"abc")),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        assert_eq!(
            hex(&sha512_256(b"abc")),
            "53048e2681941ef99b2e29b76b4c7dabe4c2d0c634fc6d46e0e2f13107e7af23"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(sha512_parts(&[b"a", b"bc"]), sha512(b"abc"));
    }

    #[test]
    fn hex_round_trips_and_refuses_uppercase() {
        assert_eq!(
            unhex(&hex(&[0, 1, 0xab, 0xff])),
            Some(vec![0, 1, 0xab, 0xff])
        );
        assert_eq!(unhex("AB"), None);
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
    }
}
