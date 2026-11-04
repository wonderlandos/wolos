// SPDX-License-Identifier: BSD-2-Clause
//! wolcrypt: every cryptographic operation WolOS performs, behind one small API
//! (docs/modules/wolcrypt.md). Pure: bytes and fresh random bytes go in, bytes come out; there is
//! no file, network, clock or random-generator access, so the system, the hub Worker (wasm), the
//! browser and C callers (the `wolcrypt-ffi` crate, `wolcrypt.h`) share one implementation.
//!
//! - [`hash`]: SHA-512, SHA-512/256, SHA-256.
//! - [`id`]: identifiers derived from public material (rule "IDS" of ADR 0046): a certificate id is
//!   SHA-512 over its public keys in a fixed order, a file id is SHA-512 over its `SHA256` member.
//! - [`sign`]: hybrid signatures, label-bound, both halves always; there is no classical-only entry
//!   point.
//! - [`kem`]: the hybrid KEM X25519 + ML-KEM-768 with a bound key schedule and ChaCha20-Poly1305.
//! - [`aead`]: ChaCha20-Poly1305 with associated data.
//! - [`kdf`]: PBKDF2-HMAC-SHA512 within fixed bounds.
//! - [`ct`]: constant-time comparison.
//! - [`detached`]: a witness's detached signature on a published file, checked offline.
//!
//! Secret types ([`sign::SigningKey`], [`kem::EncSecret`]) wipe themselves on drop.
//!
//! no_std-friendly: the code uses `core` and the `alloc` types only (`Vec`, `String`, `Box`), and
//! every dependency is built without its `std` feature, so a `no_std` build is `#![no_std]` and
//! `extern crate alloc` away; the workspace rule RR-044 (no `extern crate`) keeps it a std crate
//! today. It builds for `wasm32-unknown-unknown`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod aead;
pub mod b64;
pub mod ct;
pub mod detached;
pub mod hash;
pub mod id;
pub mod kdf;
pub mod kem;
pub mod sign;
pub mod text;

/// The wiping wrapper the secret types use, for callers that hold a secret copy of their own
/// (the C ABI's seed parameters, audit 2 L3) without a dependency of their own.
pub use zeroize::Zeroizing;

/// The wolcrypt API version (the C ABI's `wolcrypt_version()` returns it as `major << 16 | minor`).
pub const VERSION: (u16, u16) = (1, 0);

/// Derives 64 bytes for one purpose from a 32-byte seed: `SHA-512("woltrust/" || what || 0x00 ||
/// seed)`. The prefix is the one the first keys were made with (crate woltrust), kept so every
/// existing key and vector derives to the same bytes.
pub fn derive(seed: &[u8; 32], what: &str) -> zeroize::Zeroizing<[u8; 64]> {
    use sha2::Digest;
    let mut h = sha2::Sha512::new();
    h.update(b"woltrust/");
    h.update(what.as_bytes());
    h.update([0]);
    h.update(seed);
    zeroize::Zeroizing::new(h.finalize().into())
}

pub(crate) fn take<const N: usize>(b: &[u8], at: usize) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&b[at..at + N]);
    out
}
