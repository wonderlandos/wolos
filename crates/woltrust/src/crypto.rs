// SPDX-License-Identifier: BSD-2-Clause
//! The signed message and the hybrid check of ADR 0046. Every signature covers
//! `label || 0x00 || SHA-512(content)`; one label per purpose stops a signature made for one
//! purpose from being replayed as another. A signature has two halves and both must verify: Ed25519
//! plus ML-DSA-65 below the roots, Ed25519 plus SLH-DSA-SHA2-128s for the roots. There is no
//! "either" mode: the key set of the issuer fixes the pair.
use crate::b64;
use crate::cert::Usage;
use fips204::ml_dsa_65;
use fips204::traits::{SerDes as MlSerDes, Verifier as MlVerifier};
use fips205::slh_dsa_sha2_128s;
use fips205::traits::{SerDes as SlhSerDes, Verifier as SlhVerifier};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};

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

/// X25519 public key length.
pub const X_PK: usize = 32;
/// ML-KEM-768 encapsulation key length (FIPS 203).
pub const KEM_PK: usize = 1184;
/// ML-KEM-768 ciphertext length (FIPS 203).
pub const KEM_CT: usize = 1088;

/// The encryption keys of one certificate: both halves of the hybrid KEM of `wolos/seal/1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncPublic {
    /// X25519 public key.
    pub x25519: [u8; X_PK],
    /// ML-KEM-768 encapsulation key.
    pub ml_kem_768: Box<[u8; KEM_PK]>,
}

/// `label || 0x00 || SHA-512(content)` for a label outside [`Purpose`] (the request proof).
pub fn message_for(label: &str, content: &[u8]) -> Vec<u8> {
    let label = label.as_bytes();
    let mut m = Vec::with_capacity(label.len() + 1 + 64);
    m.extend_from_slice(label);
    m.push(0);
    m.extend_from_slice(&Sha512::digest(content));
    m
}

/// What a signature is for. The label is part of the signed message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// A certificate signed by its issuer.
    Cert,
    /// The `SHA256` member of a `.wol`.
    Package,
    /// The `SHA256` member of a `.wolplug`.
    Plugin,
    /// The `SHA256` member of an extension.
    Extension,
    /// The hub index.
    Index,
    /// A revocation list.
    Revocation,
}

impl Purpose {
    /// All six, in label order.
    pub const ALL: [Purpose; 6] = [
        Purpose::Cert,
        Purpose::Package,
        Purpose::Plugin,
        Purpose::Extension,
        Purpose::Index,
        Purpose::Revocation,
    ];

    /// The label signed in front of the content hash.
    pub fn label(self) -> &'static str {
        match self {
            Purpose::Cert => "wolos/cert/1",
            Purpose::Package => "wolos/package/1",
            Purpose::Plugin => "wolos/plugin/1",
            Purpose::Extension => "wolos/extension/1",
            Purpose::Index => "wolos/index/1",
            Purpose::Revocation => "wolos/revocation/1",
        }
    }

    /// The usage a certificate needs to sign for this purpose.
    pub fn usage(self) -> Usage {
        match self {
            Purpose::Cert => Usage::SignCerts,
            Purpose::Package => Usage::SignPackage,
            Purpose::Plugin => Usage::SignPlugin,
            Purpose::Extension => Usage::SignExtension,
            Purpose::Index => Usage::SignIndex,
            Purpose::Revocation => Usage::SignRevocations,
        }
    }

    /// Inverse of [`Purpose::label`].
    pub fn from_label(label: &str) -> Option<Purpose> {
        Purpose::ALL.into_iter().find(|p| p.label() == label)
    }
}

/// `label || 0x00 || SHA-512(content)`: the bytes both halves sign (under 100 bytes).
pub fn message(purpose: Purpose, content: &[u8]) -> Vec<u8> {
    message_for(purpose.label(), content)
}

/// The second public key of a certificate: fixed by the kind of key set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Second {
    /// ML-DSA-65 (intermediates, signers, people, organisations).
    MlDsa65(Box<[u8; ML_PK]>),
    /// SLH-DSA-SHA2-128s (roots).
    Slh(Box<[u8; SLH_PK]>),
}

/// The public keys of one certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicKeys {
    /// Ed25519 public key.
    pub ed25519: [u8; ED_PK],
    /// The post-quantum half.
    pub second: Second,
}

/// A hybrid signature as raw bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sig {
    /// Ed25519 signature.
    pub ed25519: [u8; ED_SIG],
    /// ML-DSA-65 or SLH-DSA-SHA2-128s signature, by the signer's key set.
    pub second: Vec<u8>,
}

/// Why a hybrid check failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// The Ed25519 half did not verify.
    Ed25519,
    /// The post-quantum half did not verify (or has the wrong length or key).
    Second,
}

/// Both halves must verify over `msg`; the first failing half is named.
pub fn verify_hybrid(keys: &PublicKeys, msg: &[u8], sig: &Sig) -> Result<(), Fail> {
    let ed_ok = ed25519_dalek::VerifyingKey::from_bytes(&keys.ed25519).is_ok_and(|k| {
        k.verify_strict(msg, &ed25519_dalek::Signature::from_bytes(&sig.ed25519))
            .is_ok()
    });
    let second_ok = match &keys.second {
        Second::MlDsa65(pk) => verify_ml(pk, msg, &sig.second),
        Second::Slh(pk) => verify_slh(pk, msg, &sig.second),
    };
    // Both halves are always evaluated, so the cost does not tell which one failed.
    match (ed_ok, second_ok) {
        (true, true) => Ok(()),
        (false, _) => Err(Fail::Ed25519),
        (true, false) => Err(Fail::Second),
    }
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

/// The wire form of a signature next to a `.wol` (`SHA256.wsig`) or a list (`revoked.json.wsig`):
/// `{"cert":"<signer serial>","ed25519":"<b64url>","second":"<b64url>"}`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Wsig {
    /// Serial of the signing certificate (the leaf for a package, the Index or a root for a list).
    pub cert: String,
    /// Ed25519 half, base64url.
    pub ed25519: String,
    /// Post-quantum half, base64url.
    pub second: String,
    /// Where the release comes from (`crate::origin::Origin`): base64url of the exact signed
    /// bytes. Absent: no origin, and the signed message is the plain one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

/// The message of a signature that carries an origin: `label || 0x00 || SHA-512(content) ||
/// SHA-512(origin)`.
pub fn message_with_origin(purpose: Purpose, content: &[u8], origin: &[u8]) -> Vec<u8> {
    let mut m = message(purpose, content);
    m.extend_from_slice(&Sha512::digest(origin));
    m
}

impl Wsig {
    /// Parses a `.wsig` of at most [`crate::MAX_WSIG`] bytes.
    pub fn parse(bytes: &[u8]) -> Result<Wsig, &'static str> {
        if bytes.len() > crate::MAX_WSIG {
            return Err("signature file too large");
        }
        let w: Wsig = serde_json::from_slice(bytes).map_err(|_| "signature file damaged")?;
        if !crate::cert::is_serial(&w.cert) {
            return Err("signer serial malformed");
        }
        Ok(w)
    }

    /// The raw halves; None when a half is not canonical base64url of a plausible length.
    pub fn sig(&self) -> Option<Sig> {
        let ed = b64::decode(&self.ed25519, ED_SIG)?;
        let second = b64::decode(&self.second, SLH_SIG)?;
        Some(Sig {
            ed25519: ed.try_into().ok()?,
            second,
        })
    }

    /// Serialised form (key order as declared, no spaces).
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_is_label_zero_hash() {
        let m = message(Purpose::Package, b"abc");
        assert_eq!(m.len(), "wolos/package/1".len() + 1 + 64);
        assert!(m.starts_with(b"wolos/package/1\0"));
        assert_ne!(m, message(Purpose::Plugin, b"abc"));
    }

    #[test]
    fn six_labels_are_distinct_and_round_trip() {
        for p in Purpose::ALL {
            assert_eq!(Purpose::from_label(p.label()), Some(p));
        }
        assert_eq!(Purpose::from_label("wolos/package/2"), None);
    }

    #[test]
    fn wsig_with_unknown_field_is_refused() {
        let text = br#"{"cert":"0123456789abcdef","ed25519":"","second":"","x":1}"#;
        assert!(Wsig::parse(text).is_err());
    }
}
