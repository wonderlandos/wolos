// SPDX-License-Identifier: BSD-2-Clause
//! Hybrid signatures (ADR 0046). Every signature covers `label || 0x00 || SHA-512(content)` (plus
//! further SHA-512 digests for bound extras, [`message_with`]), so a signature made for one purpose
//! is never valid for another and every signer signs under 200 bytes. A signature has two halves
//! and both must verify: Ed25519 + ML-DSA-65 for signing keys, Ed25519 + SLH-DSA-SHA2-128s for
//! roots. There is no classical-only entry point: [`verify`] and [`verify_message`] take the whole
//! key set and the whole signature, and evaluate both halves every time.
//!
//! Keys derive from one 32-byte seed with distinct labels ([`crate::derive`]); signing is
//! deterministic, so the same seed and message give the same bytes (the vectors rely on it).
use crate::{derive, take};
use fips204::ml_dsa_65;
use fips204::traits::{KeyGen as _, SerDes as _, Signer as _, Verifier as _};
use fips205::slh_dsa_sha2_128s;
use fips205::traits::{KeyGen as _, SerDes as _, Signer as _, Verifier as _};
use fips205::{CryptoRng, RngCore};
use std::vec::Vec;
use zeroize::Zeroizing;

/// Ed25519 public key length.
pub const ED_PK: usize = 32;
/// Ed25519 signature length.
pub const ED_SIG: usize = 64;
/// ML-DSA-65 public key length.
pub const ML_PK: usize = ml_dsa_65::PK_LEN;
/// ML-DSA-65 signature length.
pub const ML_SIG: usize = ml_dsa_65::SIG_LEN;
/// SLH-DSA-SHA2-128s public key length.
pub const SLH_PK: usize = slh_dsa_sha2_128s::PK_LEN;
/// SLH-DSA-SHA2-128s signature length.
pub const SLH_SIG: usize = slh_dsa_sha2_128s::SIG_LEN;

/// Which key set: fixes the post-quantum half.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    /// Ed25519 + ML-DSA-65: intermediates, signers, people, organisations.
    Signing,
    /// Ed25519 + SLH-DSA-SHA2-128s: the roots (hash-based, 20 years).
    Root,
}

/// The post-quantum public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Second {
    /// ML-DSA-65.
    MlDsa65(Box<[u8; ML_PK]>),
    /// SLH-DSA-SHA2-128s.
    Slh(Box<[u8; SLH_PK]>),
}

impl Second {
    /// The raw key bytes.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Second::MlDsa65(k) => &k[..],
            Second::Slh(k) => &k[..],
        }
    }
}

/// The public half of a signing key set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicKeys {
    /// Ed25519 public key.
    pub ed25519: [u8; ED_PK],
    /// The post-quantum half.
    pub second: Second,
}

/// A hybrid signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sig {
    /// Ed25519 half.
    pub ed25519: [u8; ED_SIG],
    /// ML-DSA-65 or SLH-DSA-SHA2-128s half.
    pub second: Vec<u8>,
}

/// Which half failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// The Ed25519 half.
    Ed25519,
    /// The post-quantum half (or its length, or the key).
    Second,
}

/// `label || 0x00 || SHA-512(content)`.
pub fn message(label: &str, content: &[u8]) -> Vec<u8> {
    message_with(label, content, &[])
}

/// `label || 0x00 || SHA-512(content) || SHA-512(extra_1) || ...`: a signature that also binds
/// further records (a release's origin).
pub fn message_with(label: &str, content: &[u8], extras: &[&[u8]]) -> Vec<u8> {
    let mut m = Vec::with_capacity(label.len() + 1 + 64 * (1 + extras.len()));
    m.extend_from_slice(label.as_bytes());
    m.push(0);
    m.extend_from_slice(&crate::hash::sha512(content));
    for e in extras {
        m.extend_from_slice(&crate::hash::sha512(e));
    }
    m
}

/// Both halves must verify over `msg`; both are always evaluated, so the cost does not tell which
/// failed. The first failing half is named.
pub fn verify_message(keys: &PublicKeys, msg: &[u8], sig: &Sig) -> Result<(), Fail> {
    let ed_ok = ed25519_dalek::VerifyingKey::from_bytes(&keys.ed25519).is_ok_and(|k| {
        k.verify_strict(msg, &ed25519_dalek::Signature::from_bytes(&sig.ed25519))
            .is_ok()
    });
    let second_ok = match &keys.second {
        Second::MlDsa65(pk) => verify_ml(pk, msg, &sig.second),
        Second::Slh(pk) => verify_slh(pk, msg, &sig.second),
    };
    match (ed_ok, second_ok) {
        (true, true) => Ok(()),
        (false, _) => Err(Fail::Ed25519),
        (true, false) => Err(Fail::Second),
    }
}

/// [`verify_message`] over [`message`]`(label, content)`.
pub fn verify(keys: &PublicKeys, label: &str, content: &[u8], sig: &Sig) -> Result<(), Fail> {
    verify_message(keys, &message(label, content), sig)
}

fn verify_ml(pk: &[u8; ML_PK], msg: &[u8], sig: &[u8]) -> bool {
    let Ok(sig) = <&[u8; ML_SIG]>::try_from(sig) else {
        return false;
    };
    ml_dsa_65::PublicKey::try_from_bytes(*pk).is_ok_and(|k| k.verify(msg, sig, &[]))
}

fn verify_slh(pk: &[u8; SLH_PK], msg: &[u8], sig: &[u8]) -> bool {
    let Ok(sig) = <&[u8; SLH_SIG]>::try_from(sig) else {
        return false;
    };
    slh_dsa_sha2_128s::PublicKey::try_from_bytes(*pk).is_ok_and(|k| k.verify(msg, sig, &[]))
}

/// Fills with zeros: SLH-DSA is called with `hedged = false`, which never reads the generator.
struct NoRandom;

impl RngCore for NoRandom {
    fn next_u32(&mut self) -> u32 {
        0
    }
    fn next_u64(&mut self) -> u64 {
        0
    }
    fn fill_bytes(&mut self, out: &mut [u8]) {
        out.fill(0);
    }
    fn try_fill_bytes(&mut self, out: &mut [u8]) -> Result<(), fips205::RngError> {
        out.fill(0);
        Ok(())
    }
}

impl CryptoRng for NoRandom {}

/// A signing key set derived from a 32-byte seed. The seed and the Ed25519 key are wiped on drop;
/// the post-quantum secret key is re-derived for each signature and wiped after it.
pub struct SigningKey {
    seed: Zeroizing<[u8; 32]>,
    scheme: Scheme,
    ed: ed25519_dalek::SigningKey,
    keys: PublicKeys,
}

fn slh_parts(seed: &[u8; 32]) -> ([u8; 16], [u8; 16], [u8; 16], Zeroizing<[u8; 64]>) {
    let d = derive(seed, "slh-dsa-sha2-128s");
    (
        take::<16>(&d[..], 0),
        take::<16>(&d[..], 16),
        take::<16>(&d[..], 32),
        d,
    )
}

impl SigningKey {
    /// The key set for `seed`: Ed25519 from `derive(seed, "ed25519")`, ML-DSA-65 from
    /// `derive(seed, "ml-dsa-65")` or, for [`Scheme::Root`], SLH-DSA-SHA2-128s from
    /// `derive(seed, "slh-dsa-sha2-128s")` (16 + 16 + 16 bytes).
    pub fn from_seed(seed: &[u8; 32], scheme: Scheme) -> SigningKey {
        let ed_seed = Zeroizing::new(take::<32>(&derive(seed, "ed25519")[..], 0));
        let ed = ed25519_dalek::SigningKey::from_bytes(&ed_seed);
        let second = match scheme {
            Scheme::Root => {
                let (a, b, c, _d) = slh_parts(seed);
                let (pk, _) = slh_dsa_sha2_128s::KG::keygen_from_seed(&a, &b, &c);
                Second::Slh(Box::new(pk.into_bytes()))
            }
            Scheme::Signing => {
                let s = Zeroizing::new(take::<32>(&derive(seed, "ml-dsa-65")[..], 0));
                let (pk, _) = ml_dsa_65::KG::keygen_from_seed(&s);
                Second::MlDsa65(Box::new(pk.into_bytes()))
            }
        };
        let keys = PublicKeys {
            ed25519: ed.verifying_key().to_bytes(),
            second,
        };
        SigningKey {
            seed: Zeroizing::new(*seed),
            scheme,
            ed,
            keys,
        }
    }

    /// The public halves.
    pub fn public(&self) -> &PublicKeys {
        &self.keys
    }

    /// Which key set.
    pub fn scheme(&self) -> Scheme {
        self.scheme
    }

    /// Signs [`message`]`(label, content)` with both halves.
    pub fn sign(&self, label: &str, content: &[u8]) -> Sig {
        self.sign_message(&message(label, content))
    }

    /// Signs an already built message ([`message`], [`message_with`]) with both halves.
    /// ML-DSA-65 takes its 32 random bytes from `SHA-512("woltrust/rnd" 0x00 || seed || msg)`
    /// (deterministic, secret-dependent: the hedged form of FIPS 204 with a derived seed).
    pub fn sign_message(&self, msg: &[u8]) -> Sig {
        use ed25519_dalek::Signer as _;
        let ed25519 = self.ed.sign(msg).to_bytes();
        let second = match self.scheme {
            Scheme::Root => {
                let (a, b, c, _d) = slh_parts(&self.seed);
                let (_, sk) = slh_dsa_sha2_128s::KG::keygen_from_seed(&a, &b, &c);
                sk.try_sign_with_rng(&mut NoRandom, msg, &[], false)
                    .map(|s| s.to_vec())
                    .unwrap_or_default()
            }
            Scheme::Signing => {
                let s = Zeroizing::new(take::<32>(&derive(&self.seed, "ml-dsa-65")[..], 0));
                let (_, sk) = ml_dsa_65::KG::keygen_from_seed(&s);
                let rnd = Zeroizing::new(crate::hash::sha512_parts(&[
                    b"woltrust/rnd\0",
                    &self.seed[..],
                    msg,
                ]));
                sk.try_sign_with_seed(&take::<32>(&rnd[..], 0), msg, &[])
                    .map(|s| s.to_vec())
                    .unwrap_or_default()
            }
        };
        Sig { ed25519, second }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_schemes_sign_deterministically_and_bind_the_label() {
        for scheme in [Scheme::Signing, Scheme::Root] {
            let k = SigningKey::from_seed(&[7; 32], scheme);
            let a = k.sign("wolos/package/1", b"hello");
            assert_eq!(a, k.sign("wolos/package/1", b"hello"));
            assert!(verify(k.public(), "wolos/package/1", b"hello", &a).is_ok());
            assert!(verify(k.public(), "wolos/plugin/1", b"hello", &a).is_err());
            assert!(verify(k.public(), "wolos/package/1", b"hellp", &a).is_err());
            let mut half = a.clone();
            half.second[0] ^= 1;
            assert_eq!(
                verify(k.public(), "wolos/package/1", b"hello", &half),
                Err(Fail::Second)
            );
            let mut half = a.clone();
            half.ed25519[0] ^= 1;
            assert_eq!(
                verify(k.public(), "wolos/package/1", b"hello", &half),
                Err(Fail::Ed25519)
            );
        }
    }
}
