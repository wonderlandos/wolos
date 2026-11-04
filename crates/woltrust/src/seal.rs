// SPDX-License-Identifier: BSD-2-Clause
//! Sealed messages (ADR 0046, "Encryption"): bytes encrypted to the encryption keys of one
//! certificate with the hybrid KEM of [`wolcrypt::kem`] (X25519 plus ML-KEM-768, ChaCha20-Poly1305).
//! Both shared secrets go into the key, so a reader must break both halves; the whole header is
//! the associated data, so the label, the recipient and both KEM ciphertexts cannot be changed
//! without the open failing. Pure: the caller passes 64 fresh random bytes.
//!
//! Layout (all lengths fixed except the label): format `"wolos/seal/2" 0x00` (13) | label length
//! (1, 1..=64) | label (ASCII) | recipient certificate id (128 lowercase hex, rule "IDS") | X25519
//! ephemeral public key (32) | ML-KEM-768 ciphertext (1088) | AEAD ciphertext and 16-byte tag.
//! `wolos/seal/1` is the same with a 16-hex recipient serial: still opened, and still written for
//! a recipient with a legacy-length serial (the TEST authority). Key schedule:
//! `okm = SHA-512(format || "/kdf" 0x00 || ss_mlkem || ss_x25519 || x25519_ephemeral ||
//! x25519_recipient || SHA-512(mlkem_recipient) || SHA-512(header))`, key = okm[0..32], nonce =
//! okm[32..44]. Every key is used once.
use crate::cert::{Cert, is_serial};
use crate::crypto::{EncPublic, KEM_CT, X_PK};
use zeroize::Zeroizing;

pub use wolcrypt::kem::EncSecret;

/// The format label of new messages.
pub const FORMAT: &str = "wolos/seal/2";
/// The format of messages to a 16-hex serial.
pub const FORMAT_1: &str = "wolos/seal/1";
/// The label `wol encrypt` seals files under.
pub const FILE_LABEL: &str = "wolos/file/1";
/// Largest plaintext sealed or opened in one piece: 64 MiB.
pub const MAX_PLAINTEXT: usize = 64 << 20;
/// Poly1305 tag length.
pub const TAG: usize = 16;
const MAGIC_LEN: usize = FORMAT.len() + 1;
const MAX_LABEL: usize = 64;

/// Why a message could not be sealed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SealError {
    /// The certificate carries no encryption key (issued before the field existed).
    NoEncryptionKey,
    /// The label is empty, longer than 64 or not `[a-z0-9/._-]`.
    BadLabel,
    /// More than [`MAX_PLAINTEXT`].
    TooLarge,
    /// The certificate's encryption key is not a valid key.
    BadKey,
}

/// Why a sealed message could not be opened. The wire names are stable (the vectors use them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// Not a `wolos/seal/1` message, or cut short.
    Malformed,
    /// Sealed to another certificate.
    WrongRecipient,
    /// Sealed under another label than the one expected.
    WrongLabel,
    /// Larger than [`MAX_PLAINTEXT`] plus the header.
    TooLarge,
    /// A byte changed, or the keys are not the recipient's: nothing is returned.
    Damaged,
}

impl OpenError {
    /// Stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            OpenError::Malformed => "malformed",
            OpenError::WrongRecipient => "wrong-recipient",
            OpenError::WrongLabel => "wrong-label",
            OpenError::TooLarge => "too-large",
            OpenError::Damaged => "damaged",
        }
    }

    /// What a person reads.
    pub fn sentence(self) -> &'static str {
        match self {
            OpenError::Malformed => "This is not a sealed WolOS file.",
            OpenError::WrongRecipient => "This file was sealed to another certificate.",
            OpenError::WrongLabel => "This file was sealed for another use.",
            OpenError::TooLarge => "This sealed file is larger than 64 MB.",
            OpenError::Damaged => {
                "This sealed file was changed after it was sealed, or your key does not open it."
            }
        }
    }
}

impl SealError {
    /// What a person reads.
    pub fn sentence(self) -> &'static str {
        match self {
            SealError::NoEncryptionKey => {
                "This certificate has no encryption key; ask its owner to renew it."
            }
            SealError::BadLabel => "The label must be 1 to 64 characters a-z, 0-9, / . _ -.",
            SealError::TooLarge => "WolOS seals files of up to 64 MB.",
            SealError::BadKey => "This certificate's encryption key is damaged.",
        }
    }
}

/// The header fields of a sealed message, read without opening it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// `wolos/seal/2` or `wolos/seal/1`.
    pub format: &'static str,
    /// The label it was sealed under.
    pub label: String,
    /// The recipient certificate's id (128 hex; 16 hex in `wolos/seal/1`).
    pub recipient: String,
    /// Length of the header (where the AEAD part starts).
    pub len: usize,
}

fn label_ok(l: &str) -> bool {
    !l.is_empty()
        && l.len() <= MAX_LABEL
        && l.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'/' | b'.' | b'_' | b'-')
        })
}

/// Reads the header; `Malformed` when the bytes are not a `wolos/seal/2` or `wolos/seal/1`
/// message.
pub fn peek(sealed: &[u8]) -> Result<Header, OpenError> {
    let magic = sealed.get(..MAGIC_LEN).ok_or(OpenError::Malformed)?;
    let (format, rlen) = if magic == b"wolos/seal/2\0" {
        (FORMAT, wolcrypt::id::HEX_LEN)
    } else if magic == b"wolos/seal/1\0" {
        (FORMAT_1, 16)
    } else {
        return Err(OpenError::Malformed);
    };
    let n = usize::from(*sealed.get(MAGIC_LEN).ok_or(OpenError::Malformed)?);
    let at = MAGIC_LEN + 1;
    let label = sealed.get(at..at + n).ok_or(OpenError::Malformed)?;
    let label = std::str::from_utf8(label).map_err(|_| OpenError::Malformed)?;
    let serial = sealed
        .get(at + n..at + n + rlen)
        .ok_or(OpenError::Malformed)?;
    let serial = std::str::from_utf8(serial).map_err(|_| OpenError::Malformed)?;
    if !label_ok(label) || !is_serial(serial) {
        return Err(OpenError::Malformed);
    }
    let len = at + n + rlen + X_PK + KEM_CT;
    if sealed.len() < len + TAG {
        return Err(OpenError::Malformed);
    }
    Ok(Header {
        format,
        label: label.to_owned(),
        recipient: serial.to_owned(),
        len,
    })
}

/// Seals `plaintext` to the encryption keys of `cert` under `label`. `rnd` must be 64 fresh random
/// bytes (the ephemeral X25519 key and the ML-KEM message); the same `rnd` gives the same bytes.
pub fn seal(
    cert: &Cert,
    label: &str,
    plaintext: &[u8],
    rnd: &[u8; 64],
) -> Result<Vec<u8>, SealError> {
    let to = cert.enc.as_ref().ok_or(SealError::NoEncryptionKey)?;
    seal_to(to, &cert.body.serial, label, plaintext, rnd)
}

/// [`seal`] to raw encryption keys and a certificate id (`wolos/seal/2`), or a 16-hex serial
/// (`wolos/seal/1`).
pub fn seal_to(
    to: &EncPublic,
    serial: &str,
    label: &str,
    plaintext: &[u8],
    rnd: &[u8; 64],
) -> Result<Vec<u8>, SealError> {
    if !label_ok(label) {
        return Err(SealError::BadLabel);
    }
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(SealError::TooLarge);
    }
    if !is_serial(serial) {
        return Err(SealError::BadKey);
    }
    let format = if serial.len() == 16 { FORMAT_1 } else { FORMAT };
    let label_len = u8::try_from(label.len()).map_err(|_| SealError::BadLabel)?;
    let mut prefix = Vec::with_capacity(MAGIC_LEN + 1 + label.len() + serial.len());
    prefix.extend_from_slice(format.as_bytes());
    prefix.push(0);
    prefix.push(label_len);
    prefix.extend_from_slice(label.as_bytes());
    prefix.extend_from_slice(serial.as_bytes());
    wolcrypt::kem::seal(format, to, &prefix, plaintext, rnd).map_err(|_| SealError::BadKey)
}

/// Opens a message sealed to `serial` (whose secret keys are `keys`) under `label`. The plaintext
/// is returned only when every byte checks out.
pub fn open(
    keys: &EncSecret,
    serial: &str,
    label: &str,
    sealed: &[u8],
) -> Result<Zeroizing<Vec<u8>>, OpenError> {
    if sealed.len() > MAX_PLAINTEXT + 4096 {
        return Err(OpenError::TooLarge);
    }
    let h = peek(sealed)?;
    if h.recipient != serial {
        return Err(OpenError::WrongRecipient);
    }
    if h.label != label {
        return Err(OpenError::WrongLabel);
    }
    let prefix = h.len - X_PK - KEM_CT;
    wolcrypt::kem::open(h.format, keys, sealed, prefix).map_err(|_| OpenError::Damaged)
}

/// Bytes a sealed message adds to its plaintext for `label`.
pub fn overhead(label: &str) -> usize {
    overhead_to(label, wolcrypt::id::HEX_LEN)
}

/// Bytes a sealed message adds for `label` and a recipient of `recipient_len` hex characters
/// (16 for `wolos/seal/1`).
pub fn overhead_to(label: &str, recipient_len: usize) -> usize {
    MAGIC_LEN + 1 + label.len() + recipient_len + X_PK + KEM_CT + TAG
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_then_open_round_trips_and_binds_everything() {
        let me = EncSecret::from_seed(&[5; 32]);
        let serial = "0123456789abcdef";
        let msg = b"hello sealed world";
        let s = seal_to(me.public(), serial, FILE_LABEL, msg, &[9; 64]).unwrap();
        assert_eq!(s.len(), msg.len() + overhead_to(FILE_LABEL, 16));
        assert_eq!(&open(&me, serial, FILE_LABEL, &s).unwrap()[..], msg);
        assert_eq!(
            open(&me, "fedcba9876543210", FILE_LABEL, &s).err(),
            Some(OpenError::WrongRecipient)
        );
        assert_eq!(
            open(&me, serial, "wolos/other/1", &s).err(),
            Some(OpenError::WrongLabel)
        );
        let other = EncSecret::from_seed(&[6; 32]);
        assert_eq!(
            open(&other, serial, FILE_LABEL, &s).err(),
            Some(OpenError::Damaged)
        );
        for at in [0, 14, 20, 40, 100, 1180, s.len() - 1] {
            let mut t = s.clone();
            t[at] ^= 1;
            assert!(open(&me, serial, FILE_LABEL, &t).is_err(), "byte {at}");
        }
        assert_eq!(
            open(&me, serial, FILE_LABEL, &s[..s.len() - TAG - msg.len() - 1]).err(),
            Some(OpenError::Malformed)
        );
    }

    #[test]
    fn bad_labels_and_keys_are_refused_when_sealing() {
        let me = EncSecret::from_seed(&[5; 32]);
        let s = "0123456789abcdef";
        for l in ["", "Wolos/x", "a b", &"x".repeat(65)] {
            assert_eq!(
                seal_to(me.public(), s, l, b"", &[1; 64]).err(),
                Some(SealError::BadLabel),
                "{l}"
            );
        }
        let mut bad = me.public().clone();
        bad.x25519 = [0; 32];
        assert_eq!(
            seal_to(&bad, s, FILE_LABEL, b"", &[1; 64]).err(),
            Some(SealError::BadKey)
        );
        let mut bad = me.public().clone();
        *bad.ml_kem_768 = [0xff; crate::crypto::KEM_PK];
        assert_eq!(
            seal_to(&bad, s, FILE_LABEL, b"", &[1; 64]).err(),
            Some(SealError::BadKey)
        );
    }

    #[test]
    fn garbage_never_panics_when_opened() {
        let me = EncSecret::from_seed(&[5; 32]);
        for bytes in [
            &b""[..],
            b"wolos/seal/1",
            b"wolos/seal/1\0\x40",
            &[0u8; 2000][..],
        ] {
            assert!(open(&me, "0123456789abcdef", FILE_LABEL, bytes).is_err());
        }
    }
}
