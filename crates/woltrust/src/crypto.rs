// SPDX-License-Identifier: BSD-2-Clause
//! The signed message and the hybrid check of ADR 0046. Every signature covers
//! `label || 0x00 || SHA-512(content)`; one label per purpose stops a signature made for one
//! purpose from being replayed as another. A signature has two halves and both must verify: Ed25519
//! plus ML-DSA-65 below the roots, Ed25519 plus SLH-DSA-SHA2-128s for the roots. There is no
//! "either" mode: the key set of the issuer fixes the pair.
use crate::b64;
use crate::cert::Usage;
use serde::{Deserialize, Serialize};

pub use wolcrypt::kem::{EncPublic, KEM_CT, KEM_PK, X_PK};
pub use wolcrypt::sign::{
    ED_PK, ED_SIG, Fail, ML_PK, ML_SIG, PublicKeys, SLH_PK, SLH_SIG, Second, Sig,
};

/// `label || 0x00 || SHA-512(content)` for a label outside [`Purpose`] (the request proof).
pub fn message_for(label: &str, content: &[u8]) -> Vec<u8> {
    wolcrypt::sign::message(label, content)
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
    /// A file certificate: what WolOS Hub checked about one release (`crate::filecert`).
    Release,
    /// A witness's detached signature on a published file (`crate::witness`).
    Witness,
}

impl Purpose {
    /// All eight, in label order.
    pub const ALL: [Purpose; 8] = [
        Purpose::Cert,
        Purpose::Package,
        Purpose::Plugin,
        Purpose::Extension,
        Purpose::Index,
        Purpose::Revocation,
        Purpose::Release,
        Purpose::Witness,
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
            Purpose::Release => "wolos/release/1",
            Purpose::Witness => "wolos/witness/1",
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
            // The Index intermediate signs file certificates, as it signs the index.
            Purpose::Release => Usage::SignIndex,
            Purpose::Witness => Usage::Witness,
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

/// Both halves must verify over `msg` ([`wolcrypt::sign::verify_message`]); the first failing
/// half is named.
pub fn verify_hybrid(keys: &PublicKeys, msg: &[u8], sig: &Sig) -> Result<(), Fail> {
    wolcrypt::sign::verify_message(keys, msg, sig)
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
    /// When a witness signed (`crate::witness`, Unix seconds); bound into its message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<u64>,
}

/// The message of a signature that carries an origin: `label || 0x00 || SHA-512(content) ||
/// SHA-512(origin)`.
pub fn message_with_origin(purpose: Purpose, content: &[u8], origin: &[u8]) -> Vec<u8> {
    wolcrypt::sign::message_with(purpose.label(), content, &[origin])
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
