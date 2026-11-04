// SPDX-License-Identifier: BSD-2-Clause
//! PBKDF2-HMAC-SHA512 (RFC 8018, SP 800-132) within fixed bounds: new files use [`ROUNDS`];
//! fewer than [`MIN_ROUNDS`] or more than [`MAX_ROUNDS`] are refused, so a stored file cannot be
//! downgraded and a hostile one cannot cost unbounded time.
use zeroize::Zeroizing;

/// Rounds for new key files.
pub const ROUNDS: u32 = 600_000;
/// Fewest rounds accepted.
pub const MIN_ROUNDS: u32 = 210_000;
/// Most rounds accepted.
pub const MAX_ROUNDS: u32 = 10_000_000;

/// A 32-byte key from `pass` and `salt`; None when `rounds` is outside the bounds.
pub fn pbkdf2_sha512(pass: &[u8], salt: &[u8], rounds: u32) -> Option<Zeroizing<[u8; 32]>> {
    if !(MIN_ROUNDS..=MAX_ROUNDS).contains(&rounds) {
        return None;
    }
    let mut out = Zeroizing::new([0u8; 32]);
    raw(pass, salt, rounds, &mut out[..]);
    Some(out)
}

/// PBKDF2-HMAC-SHA512 without the bounds (test vectors, which use 1 to 4096 rounds).
pub fn raw(pass: &[u8], salt: &[u8], rounds: u32, out: &mut [u8]) {
    pbkdf2::pbkdf2_hmac::<sha2::Sha512>(pass, salt, rounds.max(1), out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hex;

    #[test]
    fn known_answers_and_bounds() {
        // PBKDF2-HMAC-SHA512, P = "password", S = "salt" (the RFC 6070 inputs with SHA-512).
        let mut out = [0u8; 64];
        raw(b"password", b"salt", 1, &mut out);
        assert_eq!(
            hex(&out),
            "867f70cf1ade02cff3752599a3a53dc4af34c7a669815ae5d513554e1c8cf252c02d470a285a0501bad999bfe943c08f050235d7d68b1da55e63f73b60a57fce"
        );
        raw(b"password", b"salt", 2, &mut out);
        assert_eq!(
            hex(&out),
            "e1d9c16aa681708a45f5c7c4e215ceb66e011a2e9f0040713f18aefdb866d53cf76cab2868a39b9f7840edce4fef5a82be67335c77a6068e04112754f27ccf4e"
        );
        assert!(pbkdf2_sha512(b"p", b"s", MIN_ROUNDS - 1).is_none());
        assert!(pbkdf2_sha512(b"p", b"s", MAX_ROUNDS + 1).is_none());
    }
}
