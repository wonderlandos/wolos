// SPDX-License-Identifier: BSD-2-Clause
//! Producers: key pairs from a 32-byte seed, certificates, `.wsig` files and revocation lists.
//! Used by `wol sign`, the hub's tools, the test vectors and the tests. Everything is
//! deterministic (the same seed and message give the same bytes), which is what lets the vectors
//! be reproduced byte for byte. The production CA keys never come through here: the intermediates
//! sign remotely and the roots at the ceremony.
use crate::cert::{self, Body};
use crate::crypto::{PublicKeys, Purpose, Second, Sig, Wsig, message};
use crate::{b64, revoked};
use fips204::ml_dsa_65;
use fips204::traits::{KeyGen as MlKeyGen, SerDes as MlSerDes, Signer as MlSigner};
use fips205::slh_dsa_sha2_128s;
use fips205::traits::{KeyGen as SlhKeyGen, SerDes as SlhSerDes, Signer as SlhSigner};
use fips205::{CryptoRng, RngCore};
use sha2::{Digest, Sha512};

/// A signing key pair derived from a seed.
pub struct Secret {
    seed: zeroize::Zeroizing<[u8; 32]>,
    root: bool,
    ed: ed25519_dalek::SigningKey,
    keys: PublicKeys,
}

fn derive(seed: &[u8; 32], what: &str) -> [u8; 64] {
    let mut h = Sha512::new();
    h.update(b"woltrust/");
    h.update(what.as_bytes());
    h.update([0]);
    h.update(seed);
    h.finalize().into()
}

fn first<const N: usize>(b: &[u8; 64]) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&b[..N]);
    out
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

impl Secret {
    /// The key pair for `seed`: Ed25519 plus SLH-DSA-SHA2-128s when `root`, else Ed25519 plus
    /// ML-DSA-65.
    pub fn from_seed(seed: &[u8; 32], root: bool) -> Secret {
        let ed = ed25519_dalek::SigningKey::from_bytes(&first::<32>(&derive(seed, "ed25519")));
        let second = if root {
            let d = derive(seed, "slh-dsa-sha2-128s");
            let (pk, _) = slh_dsa_sha2_128s::KG::keygen_from_seed(
                &first::<16>(&d),
                &d_part(&d, 16),
                &d_part(&d, 32),
            );
            Second::Slh(Box::new(pk.into_bytes()))
        } else {
            let (pk, _) = ml_dsa_65::KG::keygen_from_seed(&first::<32>(&derive(seed, "ml-dsa-65")));
            Second::MlDsa65(Box::new(pk.into_bytes()))
        };
        let keys = PublicKeys {
            ed25519: ed.verifying_key().to_bytes(),
            second,
        };
        Secret {
            seed: zeroize::Zeroizing::new(*seed),
            root,
            ed,
            keys,
        }
    }

    /// The public halves.
    pub fn public(&self) -> &PublicKeys {
        &self.keys
    }

    /// Signs `label || 0 || SHA-512(content)` with both halves.
    pub fn sign(&self, purpose: Purpose, content: &[u8]) -> Sig {
        self.sign_message(&message(purpose, content))
    }

    /// Signs an already built message (`crate::crypto::message_for`) with both halves.
    pub fn sign_message(&self, msg: &[u8]) -> Sig {
        use ed25519_dalek::Signer;
        let ed25519 = self.ed.sign(msg).to_bytes();
        let second = if self.root {
            let d = derive(&self.seed, "slh-dsa-sha2-128s");
            let (_, sk) = slh_dsa_sha2_128s::KG::keygen_from_seed(
                &first::<16>(&d),
                &d_part(&d, 16),
                &d_part(&d, 32),
            );
            sk.try_sign_with_rng(&mut NoRandom, msg, &[], false)
                .map(|s| s.to_vec())
                .unwrap_or_default()
        } else {
            let (_, sk) =
                ml_dsa_65::KG::keygen_from_seed(&first::<32>(&derive(&self.seed, "ml-dsa-65")));
            let mut rnd = Sha512::new();
            rnd.update(b"woltrust/rnd\0");
            rnd.update(&self.seed[..]);
            rnd.update(msg);
            let rnd: [u8; 64] = rnd.finalize().into();
            sk.try_sign_with_seed(&first::<32>(&rnd), msg, &[])
                .map(|s| s.to_vec())
                .unwrap_or_default()
        };
        Sig { ed25519, second }
    }
}

fn d_part(d: &[u8; 64], at: usize) -> [u8; 16] {
    let mut out = [0u8; 16];
    out.copy_from_slice(&d[at..at + 16]);
    out
}

/// The `Keys` text of a body for `secret`.
pub fn keys_text(secret: &Secret) -> cert::KeysText {
    let pk = secret.public();
    let mut t = cert::KeysText {
        ed25519: b64::encode(&pk.ed25519),
        ..cert::KeysText::default()
    };
    match &pk.second {
        Second::MlDsa65(k) => t.ml_dsa_65 = Some(b64::encode(&k[..])),
        Second::Slh(k) => t.slh_dsa_sha2_128s = Some(b64::encode(&k[..])),
    }
    t
}

/// `.wolcert` bytes for `body`, signed by `issuer` (the body's own key for a root).
pub fn issue(body: &Body, issuer: &Secret) -> Vec<u8> {
    let body_bytes = serde_json::to_vec(body).unwrap_or_default();
    let sig = issuer.sign(Purpose::Cert, &body_bytes);
    cert::encode(&body_bytes, &sig)
}

/// A `.wsig` for `content` by the certificate `serial`, signed by `secret`.
pub fn wsig(serial: &str, secret: &Secret, purpose: Purpose, content: &[u8]) -> Vec<u8> {
    let s = secret.sign(purpose, content);
    Wsig {
        cert: serial.to_owned(),
        ed25519: b64::encode(&s.ed25519),
        second: b64::encode(&s.second),
        origin: None,
    }
    .to_bytes()
}

/// A `.wsig` that also signs where the release comes from (`origin`: the exact bytes of an
/// [`crate::origin::Origin`]).
pub fn wsig_with_origin(
    serial: &str,
    secret: &Secret,
    purpose: Purpose,
    content: &[u8],
    origin: &[u8],
) -> Vec<u8> {
    let s = secret.sign_message(&crate::crypto::message_with_origin(
        purpose, content, origin,
    ));
    Wsig {
        cert: serial.to_owned(),
        ed25519: b64::encode(&s.ed25519),
        second: b64::encode(&s.second),
        origin: Some(b64::encode(origin)),
    }
    .to_bytes()
}

/// `(revoked.json, revoked.json.wsig)` for `list`, signed by `secret` (certificate `serial`).
pub fn revocation_list(
    list: &revoked::ListText,
    serial: &str,
    secret: &Secret,
) -> (Vec<u8>, Vec<u8>) {
    let bytes = serde_json::to_vec(list).unwrap_or_default();
    let sig = wsig(serial, secret, Purpose::Revocation, &bytes);
    (bytes, sig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::verify_hybrid;

    #[test]
    fn signing_is_deterministic_and_verifies() {
        for root in [false, true] {
            let s = Secret::from_seed(&[7; 32], root);
            let a = s.sign(Purpose::Package, b"hello");
            let b = s.sign(Purpose::Package, b"hello");
            assert_eq!(a, b, "root={root}");
            assert!(verify_hybrid(s.public(), &message(Purpose::Package, b"hello"), &a).is_ok());
            assert!(verify_hybrid(s.public(), &message(Purpose::Plugin, b"hello"), &a).is_err());
            assert!(verify_hybrid(s.public(), &message(Purpose::Package, b"hellp"), &a).is_err());
        }
    }
}
