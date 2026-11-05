// SPDX-License-Identifier: BSD-2-Clause
//! The engine's TLS key exchange order (ADR 0055). At the pin the engine's rustls is built without
//! `prefer-post-quantum`, so its provider lists the hybrid post-quantum group X25519MLKEM768 last
//! and sends a key share only for X25519: every connection is classical unless the server asks
//! again. [`order`] puts X25519MLKEM768 first and X25519 right after it (rustls then sends both
//! shares: the hybrid group's X25519 half is shared), keeps the other groups behind them, and drops
//! a lone ML-KEM group (never offered without a classical half). Dependency-free, so
//! `tests/engine_tls.rs` checks it without the engine; `lib.rs` applies it to the provider.

/// X25519MLKEM768 (draft-ietf-tls-ecdhe-mlkem).
pub const X25519MLKEM768: u16 = 0x11ec;
/// X25519.
pub const X25519: u16 = 0x001d;
/// ML-KEM-768 and ML-KEM-1024 alone (draft-connolly-tls-mlkem-key-agreement).
pub const MLKEM_ALONE: [u16; 3] = [0x0200, 0x0201, 0x0202];

/// `groups` with the hybrid group first and X25519 second, the rest in their order, no lone ML-KEM.
/// `code` gives an item's TLS codepoint.
pub fn order<T>(groups: Vec<T>, code: impl Fn(&T) -> u16) -> Vec<T> {
    let mut first = Vec::new();
    let mut second = Vec::new();
    let mut rest = Vec::new();
    for g in groups {
        match code(&g) {
            X25519MLKEM768 => first.push(g),
            X25519 => second.push(g),
            c if MLKEM_ALONE.contains(&c) => {}
            _ => rest.push(g),
        }
    }
    first.extend(second);
    first.extend(rest);
    first
}
