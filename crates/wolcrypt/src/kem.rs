// SPDX-License-Identifier: BSD-2-Clause
//! The hybrid KEM of sealed messages (ADR 0046, "Encryption"): X25519 + ML-KEM-768 to a static
//! recipient key, ChaCha20-Poly1305 with the whole header as associated data. A reader must break
//! both halves. Key schedule, bound to everything public:
//! `okm = SHA-512(format || "/kdf" 0x00 || ss_mlkem || ss_x25519 || x25519_ephemeral ||
//! x25519_recipient || SHA-512(mlkem_recipient) || SHA-512(header))`, key = okm[0..32], nonce =
//! okm[32..44]; every key is used once. An all-zero X25519 result (a low-order point) is refused.
//!
//! The caller owns the header layout: [`seal`] writes `prefix || x25519_ephemeral (32) ||
//! ml_kem_ciphertext (1088) || aead ciphertext || tag (16)`, where `prefix` names the format, label
//! and recipient (crate woltrust, `wolos/seal/1` and `wolos/seal/2`).
use crate::{derive, take};
use curve25519_dalek::montgomery::MontgomeryPoint;
use fips203::ml_kem_768;
use fips203::traits::{Decaps as _, Encaps as _, KeyGen as _, SerDes as _};
use std::boxed::Box;
use std::vec::Vec;
use zeroize::Zeroizing;

/// X25519 public key length.
pub const X_PK: usize = 32;
/// ML-KEM-768 encapsulation key length.
pub const KEM_PK: usize = ml_kem_768::EK_LEN;
/// ML-KEM-768 ciphertext length.
pub const KEM_CT: usize = ml_kem_768::CT_LEN;
/// Bytes after the prefix and before the AEAD part.
pub const KEM_PART: usize = X_PK + KEM_CT;

const _: () = assert!(KEM_PK == 1184 && KEM_CT == 1088);

/// The public encryption keys of one certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncPublic {
    /// X25519 public key.
    pub x25519: [u8; X_PK],
    /// ML-KEM-768 encapsulation key.
    pub ml_kem_768: Box<[u8; KEM_PK]>,
}

/// The secret encryption keys, derived from the 32-byte seed; wiped on drop.
pub struct EncSecret {
    x25519: Zeroizing<[u8; 32]>,
    dk: Zeroizing<[u8; ml_kem_768::DK_LEN]>,
    public: EncPublic,
}

impl EncSecret {
    /// X25519 from `derive(seed, "x25519")[0..32]`, ML-KEM-768 from `derive(seed, "ml-kem-768")`
    /// (d = bytes 0..32, z = 32..64).
    pub fn from_seed(seed: &[u8; 32]) -> EncSecret {
        let x = derive(seed, "x25519");
        let x25519 = Zeroizing::new(take::<32>(&x[..], 0));
        let m = derive(seed, "ml-kem-768");
        // Audit 2 L3: the d and z seeds are wiped copies (the call takes them by value).
        let d = Zeroizing::new(take::<32>(&m[..], 0));
        let z = Zeroizing::new(take::<32>(&m[..], 32));
        let (ek, dk) = ml_kem_768::KG::keygen_from_seed(*d, *z);
        EncSecret {
            public: EncPublic {
                x25519: MontgomeryPoint::mul_base_clamped(*x25519).to_bytes(),
                ml_kem_768: Box::new(ek.into_bytes()),
            },
            x25519,
            dk: Zeroizing::new(dk.into_bytes()),
        }
    }

    /// The public halves.
    pub fn public(&self) -> &EncPublic {
        &self.public
    }
}

/// Why sealing or opening failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A recipient key is not a valid key (or gives the all-zero X25519 result).
    BadKey,
    /// The message is cut short.
    Malformed,
    /// A byte changed, or the keys are not the recipient's.
    Damaged,
}

/// X25519 with the all-zero output refused.
pub fn x25519(secret: &[u8; 32], public: &[u8; 32]) -> Option<Zeroizing<[u8; 32]>> {
    let ss = Zeroizing::new(MontgomeryPoint(*public).mul_clamped(*secret).to_bytes());
    ss.iter().any(|b| *b != 0).then_some(ss)
}

/// The X25519 public key of a secret scalar (clamped).
pub fn x25519_public(secret: &[u8; 32]) -> [u8; 32] {
    MontgomeryPoint::mul_base_clamped(*secret).to_bytes()
}

fn schedule(
    format: &str,
    ss_m: &[u8],
    ss_x: &[u8; 32],
    eph: &[u8; 32],
    to: &EncPublic,
    header: &[u8],
) -> Zeroizing<[u8; 64]> {
    let mlkem_h = crate::hash::sha512(&to.ml_kem_768[..]);
    let header_h = crate::hash::sha512(header);
    Zeroizing::new(crate::hash::sha512_parts(&[
        format.as_bytes(),
        b"/kdf\0",
        ss_m,
        &ss_x[..],
        &eph[..],
        &to.x25519[..],
        &mlkem_h,
        &header_h,
    ]))
}

/// The AEAD key (okm[0..32]) and nonce (okm[32..44]) as wiped copies (audit 2 L3).
fn key_nonce(
    okm: &[u8; 64],
) -> (
    Zeroizing<[u8; crate::aead::KEY]>,
    Zeroizing<[u8; crate::aead::NONCE]>,
) {
    (
        Zeroizing::new(take::<{ crate::aead::KEY }>(&okm[..], 0)),
        Zeroizing::new(take::<{ crate::aead::NONCE }>(&okm[..], 32)),
    )
}

/// Seals `plaintext` to `to`: `prefix || ephemeral || ciphertext || aead`. `rnd` is 64 fresh random
/// bytes (the ephemeral X25519 scalar, then the ML-KEM message); the same `rnd` gives the same
/// bytes.
pub fn seal(
    format: &str,
    to: &EncPublic,
    prefix: &[u8],
    plaintext: &[u8],
    rnd: &[u8; 64],
) -> Result<Vec<u8>, Error> {
    let ek = ml_kem_768::EncapsKey::try_from_bytes(*to.ml_kem_768).map_err(|_| Error::BadKey)?;
    let eph_secret = Zeroizing::new(take::<32>(rnd, 0));
    let eph = x25519_public(&eph_secret);
    let ss_x = x25519(&eph_secret, &to.x25519).ok_or(Error::BadKey)?;
    let m = Zeroizing::new(take::<32>(rnd, 32));
    let (ss_m, ct) = ek.encaps_from_seed(&m);
    let ss_m = Zeroizing::new(ss_m.into_bytes());
    let mut out = Vec::with_capacity(prefix.len() + KEM_PART + plaintext.len() + crate::aead::TAG);
    out.extend_from_slice(prefix);
    out.extend_from_slice(&eph);
    out.extend_from_slice(&ct.into_bytes());
    let okm = schedule(format, &ss_m[..], &ss_x, &eph, to, &out);
    let (key, nonce) = key_nonce(&okm);
    let mut body = Zeroizing::new(plaintext.to_vec());
    crate::aead::seal(&key, &nonce, &out, &mut body);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Opens a message whose prefix is `prefix_len` bytes long (the caller parsed and checked it).
/// The plaintext is returned only when every byte checks out.
pub fn open(
    format: &str,
    keys: &EncSecret,
    sealed: &[u8],
    prefix_len: usize,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    let len = prefix_len.checked_add(KEM_PART).ok_or(Error::Malformed)?;
    if sealed.len() < len + crate::aead::TAG {
        return Err(Error::Malformed);
    }
    let (header, rest) = sealed.split_at(len);
    let eph = take::<32>(header, prefix_len);
    let ct = take::<KEM_CT>(header, prefix_len + X_PK);
    let ct = ml_kem_768::CipherText::try_from_bytes(ct).map_err(|_| Error::Damaged)?;
    let dk = ml_kem_768::DecapsKey::try_from_bytes(*keys.dk).map_err(|_| Error::Damaged)?;
    let ss_m = Zeroizing::new(dk.try_decaps(&ct).map_err(|_| Error::Damaged)?.into_bytes());
    let ss_x = x25519(&keys.x25519, &eph).ok_or(Error::Damaged)?;
    let okm = schedule(format, &ss_m[..], &ss_x, &eph, &keys.public, header);
    let (key, nonce) = key_nonce(&okm);
    let mut body = Zeroizing::new(rest.to_vec());
    crate::aead::open(&key, &nonce, header, &mut body).ok_or(Error::Damaged)?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::unhex;

    #[test]
    fn x25519_matches_rfc_7748_section_6_1() {
        let h = |s: &str| -> [u8; 32] { unhex(s).unwrap().try_into().unwrap() };
        let a = h("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
        let b = h("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb");
        assert_eq!(
            x25519_public(&a),
            h("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
        );
        let k = h("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742");
        assert_eq!(x25519(&a, &x25519_public(&b)).map(|s| *s), Some(k));
        assert!(x25519(&a, &[0u8; 32]).is_none(), "low-order point refused");
    }

    #[test]
    fn seal_open_binds_prefix_and_keys() {
        let me = EncSecret::from_seed(&[5; 32]);
        let s = seal("wolos/test/1", me.public(), b"PREFIX", b"hi", &[9; 64]).unwrap();
        assert_eq!(&open("wolos/test/1", &me, &s, 6).unwrap()[..], b"hi");
        assert!(open("wolos/test/2", &me, &s, 6).is_err(), "format bound");
        for at in [0, 6, 40, 1100, s.len() - 1] {
            let mut t = s.clone();
            t[at] ^= 1;
            assert!(open("wolos/test/1", &me, &t, 6).is_err(), "byte {at}");
        }
        let other = EncSecret::from_seed(&[6; 32]);
        assert_eq!(
            open("wolos/test/1", &other, &s, 6).err(),
            Some(Error::Damaged)
        );
        assert_eq!(
            open("wolos/test/1", &me, &s[..20], 6).err(),
            Some(Error::Malformed)
        );
    }
}
