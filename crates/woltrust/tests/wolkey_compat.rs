// SPDX-License-Identifier: BSD-2-Clause
//! The move from ring to wolcrypt (lead decision 2026-11-04 "WOLCRYPT") keeps every byte: a
//! `.wolkey` written with ring's PBKDF2 and ChaCha20-Poly1305 before the move opens, and locking
//! the same key with the same random bytes writes the same file again.
use woltrust::key::KeyMaterial;
use woltrust::wolkey;

const RING_MADE: &[u8] = include_bytes!("../vectors/wolkey/ring-made.wolkey");

#[test]
fn a_key_file_ring_wrote_opens_and_is_rewritten_byte_for_byte() {
    let km = wolkey::unlock(RING_MADE, "0123456789abcdef", "correct horse battery")
        .expect("ring-made file opens");
    let want = KeyMaterial::new([0x42; 32], false);
    assert_eq!(km.secret().public(), want.secret().public());
    let rnd: [u8; 28] = core::array::from_fn(|i| i as u8);
    let again = wolkey::lock(
        &want,
        "0123456789abcdef",
        "correct horse battery",
        &rnd,
        wolkey::MIN_ROUNDS,
    )
    .expect("locks");
    assert_eq!(again, RING_MADE);
    assert_eq!(
        wolkey::unlock(RING_MADE, "0123456789abcdef", "wrong horse battery").err(),
        Some(wolkey::KeyError::WrongPassphrase)
    );
}
