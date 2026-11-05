// SPDX-License-Identifier: BSD-2-Clause
//! ADR 0055: the key exchange order the engine library gives its TLS stack
//! (`src/engine/tls.rs`, built into libwolwebengine; compiled here on its own).
#[path = "../src/engine/tls.rs"]
mod tls;

const SECP256R1: u16 = 0x0017;
const SECP384R1: u16 = 0x0018;

/// The engine's provider at the pin lists X25519, P-256, P-384, then X25519MLKEM768 (rustls
/// 0.23.45 aws-lc-rs DEFAULT_KX_GROUPS without `prefer-post-quantum`).
#[test]
fn the_hybrid_group_moves_first_and_x25519_second() {
    let pin = vec![tls::X25519, SECP256R1, SECP384R1, tls::X25519MLKEM768];
    assert_eq!(
        tls::order(pin, |c| *c),
        [tls::X25519MLKEM768, tls::X25519, SECP256R1, SECP384R1]
    );
}

#[test]
fn ml_kem_alone_is_never_offered_and_nothing_is_added() {
    let all = vec![0x0201, tls::X25519, SECP256R1, tls::X25519MLKEM768, 0x0202];
    assert_eq!(
        tls::order(all, |c| *c),
        [tls::X25519MLKEM768, tls::X25519, SECP256R1]
    );
    // A provider without the hybrid group stays as it was (the library then says so).
    assert_eq!(
        tls::order(vec![tls::X25519, SECP256R1], |c| *c),
        [tls::X25519, SECP256R1]
    );
}
