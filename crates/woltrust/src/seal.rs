// SPDX-License-Identifier: BSD-2-Clause
//! Sealed messages, format `wolos/seal/1` (ADR 0046, "Encryption"): bytes encrypted to the
//! encryption keys of one certificate with a hybrid KEM, X25519 plus ML-KEM-768, and
//! ChaCha20-Poly1305. Both shared secrets go into the key, so a reader must break both halves;
//! the whole header is the associated data, so the label, the recipient and both KEM ciphertexts
//! cannot be changed without the open failing. Pure: the caller passes 64 fresh random bytes.
//!
//! Layout (all lengths fixed except the label):
//! `"wolos/seal/1" 0x00` (13) | label length (1, 1..=64) | label (ASCII) | recipient serial (16
//! lowercase hex) | X25519 ephemeral public key (32) | ML-KEM-768 ciphertext (1088) | AEAD
//! ciphertext and 16-byte tag. Key schedule: `okm = SHA-512("wolos/seal/1/kdf" 0x00 || ss_mlkem
//! || ss_x25519 || x25519_ephemeral || x25519_recipient || SHA-512(mlkem_recipient) ||
//! SHA-512(header))`, key = okm[0..32], nonce = okm[32..44]. Every key is used once.
use crate::cert::{Cert, is_serial};
use crate::crypto::{EncPublic, KEM_CT, KEM_PK, X_PK};
use curve25519_dalek::montgomery::MontgomeryPoint;
use fips203::ml_kem_768;
use fips203::traits::{Decaps, Encaps, KeyGen, SerDes};
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};
use sha2::{Digest, Sha512};
use zeroize::{Zeroize, Zeroizing};

/// The format label, first in every sealed message.
pub const FORMAT: &str = "wolos/seal/1";
/// The label `wol encrypt` seals files under.
pub const FILE_LABEL: &str = "wolos/file/1";
/// Largest plaintext sealed or opened in one piece: 64 MiB.
pub const MAX_PLAINTEXT: usize = 64 << 20;
/// Poly1305 tag length.
pub const TAG: usize = 16;
const MAGIC_LEN: usize = FORMAT.len() + 1;
const MAX_LABEL: usize = 64;

/// The secret half of a certificate's encryption keys, derived from the author's 32-byte seed.
pub struct EncSecret {
    x25519: Zeroizing<[u8; 32]>,
    dk: Zeroizing<[u8; ml_kem_768::DK_LEN]>,
    public: EncPublic,
}

fn derive(seed: &[u8; 32], what: &str) -> Zeroizing<[u8; 64]> {
    let mut h = Sha512::new();
    h.update(b"woltrust/");
    h.update(what.as_bytes());
    h.update([0]);
    h.update(seed);
    Zeroizing::new(h.finalize().into())
}

fn half(b: &[u8; 64], at: usize) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&b[at..at + 32]);
    out
}

impl EncSecret {
    /// The encryption key pair of `seed` (the same seed as the signing keys, another label).
    pub fn from_seed(seed: &[u8; 32]) -> EncSecret {
        let x = derive(seed, "x25519");
        let x25519 = Zeroizing::new(half(&x, 0));
        let m = derive(seed, "ml-kem-768");
        let (ek, dk) = ml_kem_768::KG::keygen_from_seed(half(&m, 0), half(&m, 32));
        let public = EncPublic {
            x25519: MontgomeryPoint::mul_base_clamped(*x25519).to_bytes(),
            ml_kem_768: Box::new(ek.into_bytes()),
        };
        EncSecret {
            x25519,
            dk: Zeroizing::new(dk.into_bytes()),
            public,
        }
    }

    /// The public halves (what goes into the certificate body).
    pub fn public(&self) -> &EncPublic {
        &self.public
    }
}

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
    /// The label it was sealed under.
    pub label: String,
    /// Serial of the recipient certificate.
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

/// Reads the header; `Malformed` when the bytes are not a `wolos/seal/1` message.
pub fn peek(sealed: &[u8]) -> Result<Header, OpenError> {
    let magic = sealed.get(..MAGIC_LEN).ok_or(OpenError::Malformed)?;
    if magic.get(..FORMAT.len()) != Some(FORMAT.as_bytes()) || magic.last() != Some(&0) {
        return Err(OpenError::Malformed);
    }
    let n = usize::from(*sealed.get(MAGIC_LEN).ok_or(OpenError::Malformed)?);
    let at = MAGIC_LEN + 1;
    let label = sealed.get(at..at + n).ok_or(OpenError::Malformed)?;
    let label = std::str::from_utf8(label).map_err(|_| OpenError::Malformed)?;
    let serial = sealed
        .get(at + n..at + n + 16)
        .ok_or(OpenError::Malformed)?;
    let serial = std::str::from_utf8(serial).map_err(|_| OpenError::Malformed)?;
    if !label_ok(label) || !is_serial(serial) {
        return Err(OpenError::Malformed);
    }
    let len = at + n + 16 + X_PK + KEM_CT;
    if sealed.len() < len + TAG {
        return Err(OpenError::Malformed);
    }
    Ok(Header {
        label: label.to_owned(),
        recipient: serial.to_owned(),
        len,
    })
}

fn kdf(
    ss_m: &[u8],
    ss_x: &[u8; 32],
    eph: &[u8; 32],
    to: &EncPublic,
    header: &[u8],
) -> Zeroizing<[u8; 64]> {
    let mut h = Sha512::new();
    h.update(FORMAT.as_bytes());
    h.update(b"/kdf\0");
    h.update(ss_m);
    h.update(ss_x);
    h.update(eph);
    h.update(to.x25519);
    h.update(Sha512::digest(&to.ml_kem_768[..]));
    h.update(Sha512::digest(header));
    Zeroizing::new(h.finalize().into())
}

fn aead(okm: &[u8; 64]) -> Option<(LessSafeKey, Nonce)> {
    let key = UnboundKey::new(&CHACHA20_POLY1305, &okm[..32]).ok()?;
    let nonce = Nonce::try_assume_unique_for_key(&okm[32..44]).ok()?;
    Some((LessSafeKey::new(key), nonce))
}

/// X25519 with the all-zero output (a low-order point) refused.
fn x25519(secret: &[u8; 32], public: &[u8; 32]) -> Option<Zeroizing<[u8; 32]>> {
    let ss = Zeroizing::new(MontgomeryPoint(*public).mul_clamped(*secret).to_bytes());
    (ss.iter().any(|b| *b != 0)).then_some(ss)
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

/// [`seal`] to raw encryption keys and a serial.
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
    let ek =
        ml_kem_768::EncapsKey::try_from_bytes(*to.ml_kem_768).map_err(|_| SealError::BadKey)?;
    let eph_secret = Zeroizing::new(half(rnd, 0));
    let eph = MontgomeryPoint::mul_base_clamped(*eph_secret).to_bytes();
    let ss_x = x25519(&eph_secret, &to.x25519).ok_or(SealError::BadKey)?;
    let (ss_m, ct) = ek.encaps_from_seed(&half(rnd, 32));
    let ss_m = Zeroizing::new(ss_m.into_bytes());
    let label_len = u8::try_from(label.len()).map_err(|_| SealError::BadLabel)?;
    let mut out = Vec::with_capacity(MAGIC_LEN + 1 + label.len() + 16 + X_PK + KEM_CT);
    out.extend_from_slice(FORMAT.as_bytes());
    out.push(0);
    out.push(label_len);
    out.extend_from_slice(label.as_bytes());
    out.extend_from_slice(serial.as_bytes());
    out.extend_from_slice(&eph);
    out.extend_from_slice(&ct.into_bytes());
    let okm = kdf(&ss_m[..], &ss_x, &eph, to, &out);
    let (key, nonce) = aead(&okm).ok_or(SealError::BadKey)?;
    let mut body = plaintext.to_vec();
    key.seal_in_place_append_tag(nonce, Aad::from(&out[..]), &mut body)
        .map_err(|_| SealError::TooLarge)?;
    out.extend_from_slice(&body);
    body.zeroize();
    Ok(out)
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
    let (header, rest) = sealed.split_at(h.len);
    let at = h.len - X_PK - KEM_CT;
    let mut eph = [0u8; X_PK];
    eph.copy_from_slice(&header[at..at + X_PK]);
    let mut ct = [0u8; KEM_CT];
    ct.copy_from_slice(&header[at + X_PK..]);
    let ct = ml_kem_768::CipherText::try_from_bytes(ct).map_err(|_| OpenError::Damaged)?;
    let dk = ml_kem_768::DecapsKey::try_from_bytes(*keys.dk).map_err(|_| OpenError::Damaged)?;
    let ss_m = Zeroizing::new(
        dk.try_decaps(&ct)
            .map_err(|_| OpenError::Damaged)?
            .into_bytes(),
    );
    let ss_x = x25519(&keys.x25519, &eph).ok_or(OpenError::Damaged)?;
    let okm = kdf(&ss_m[..], &ss_x, &eph, &keys.public, header);
    let (key, nonce) = aead(&okm).ok_or(OpenError::Damaged)?;
    let mut body = Zeroizing::new(rest.to_vec());
    let n = key
        .open_in_place(nonce, Aad::from(header), &mut body)
        .map_err(|_| OpenError::Damaged)?
        .len();
    body.truncate(n);
    Ok(body)
}

/// Bytes a sealed message adds to its plaintext for `label`.
pub fn overhead(label: &str) -> usize {
    MAGIC_LEN + 1 + label.len() + 16 + X_PK + KEM_CT + TAG
}

const _: () = assert!(KEM_PK == ml_kem_768::EK_LEN && KEM_CT == ml_kem_768::CT_LEN);

#[cfg(test)]
mod tests {
    use super::*;

    fn hex32(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap_or(0);
        }
        out
    }

    #[test]
    fn x25519_matches_rfc_7748_section_6_1() {
        let a = hex32("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
        let b = hex32("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb");
        let a_pub = MontgomeryPoint::mul_base_clamped(a).to_bytes();
        let b_pub = MontgomeryPoint::mul_base_clamped(b).to_bytes();
        assert_eq!(
            a_pub,
            hex32("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
        );
        assert_eq!(
            b_pub,
            hex32("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f")
        );
        let k = hex32("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742");
        assert_eq!(x25519(&a, &b_pub).map(|s| *s), Some(k));
        assert_eq!(x25519(&b, &a_pub).map(|s| *s), Some(k));
        assert!(x25519(&a, &[0u8; 32]).is_none(), "low-order point refused");
    }

    #[test]
    fn seal_then_open_round_trips_and_binds_everything() {
        let me = EncSecret::from_seed(&[5; 32]);
        let serial = "0123456789abcdef";
        let msg = b"hello sealed world";
        let s = seal_to(me.public(), serial, FILE_LABEL, msg, &[9; 64]).unwrap();
        assert_eq!(s.len(), msg.len() + overhead(FILE_LABEL));
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
        *bad.ml_kem_768 = [0xff; KEM_PK];
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
