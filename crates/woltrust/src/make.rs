// SPDX-License-Identifier: BSD-2-Clause
//! Producers: key pairs from a 32-byte seed, certificates, `.wsig` files and revocation lists.
//! Used by `wol sign`, the hub's tools, the test vectors and the tests. Everything is
//! deterministic (the same seed and message give the same bytes), which is what lets the vectors
//! be reproduced byte for byte. The production CA keys never come through here: the intermediates
//! sign remotely and the roots at the ceremony.
use crate::cert::{self, Body};
use crate::crypto::{PublicKeys, Purpose, Second, Sig, Wsig};
use crate::{b64, revoked};
use wolcrypt::sign::{Scheme, SigningKey};

/// A signing key pair derived from a seed ([`wolcrypt::sign::SigningKey`]; wiped on drop).
pub struct Secret {
    key: SigningKey,
}

impl Secret {
    /// The key pair for `seed`: Ed25519 plus SLH-DSA-SHA2-128s when `root`, else Ed25519 plus
    /// ML-DSA-65.
    pub fn from_seed(seed: &[u8; 32], root: bool) -> Secret {
        let scheme = if root { Scheme::Root } else { Scheme::Signing };
        Secret {
            key: SigningKey::from_seed(seed, scheme),
        }
    }

    /// The public halves.
    pub fn public(&self) -> &PublicKeys {
        self.key.public()
    }

    /// Signs `label || 0 || SHA-512(content)` with both halves.
    pub fn sign(&self, purpose: Purpose, content: &[u8]) -> Sig {
        self.key.sign(purpose.label(), content)
    }

    /// Signs an already built message (`crate::crypto::message_for`) with both halves.
    pub fn sign_message(&self, msg: &[u8]) -> Sig {
        self.key.sign_message(msg)
    }

    /// The certificate id of these keys with `enc` (rule "IDS": SHA-512 over the public keys in
    /// order), as 128 hex.
    pub fn id(&self, enc: Option<&crate::crypto::EncPublic>) -> String {
        wolcrypt::id::to_hex(&wolcrypt::id::of_keys(self.public(), enc))
    }
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

/// `.wolcert` bytes for an intermediate signed by `issuer` (the root `body.issuer` names) and
/// co-signed by each of `cosigners` (`(root id, key)`; the other root).
pub fn issue_cosigned(body: &Body, issuer: &Secret, cosigners: &[(&str, &Secret)]) -> Vec<u8> {
    let body_bytes = serde_json::to_vec(body).unwrap_or_default();
    let sig = issuer.sign(Purpose::Cert, &body_bytes);
    let cosigs: Vec<cert::CoSig> = cosigners
        .iter()
        .map(|(id, k)| cert::CoSig {
            issuer: (*id).to_owned(),
            sig: k.sign(Purpose::Cert, &body_bytes),
        })
        .collect();
    cert::encode_with(&body_bytes, &sig, &cosigs)
}

/// A `.wsig` for `content` by the certificate `serial`, signed by `secret`.
pub fn wsig(serial: &str, secret: &Secret, purpose: Purpose, content: &[u8]) -> Vec<u8> {
    let s = secret.sign(purpose, content);
    Wsig {
        cert: serial.to_owned(),
        ed25519: b64::encode(&s.ed25519),
        second: b64::encode(&s.second),
        origin: None,
        at: None,
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
        at: None,
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
    use crate::crypto::{message, verify_hybrid};

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
