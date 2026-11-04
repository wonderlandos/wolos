// SPDX-License-Identifier: BSD-2-Clause
//! Certificate requests (ADR 0046, `wol cert request`): an author makes keys on their own machine
//! and sends the public halves with a proof of possession, a hybrid signature with the new keys
//! under the label `wolos/request/1`. The proof covers the GitHub id, the kind and every public key
//! (signing and encryption), so it can be checked again from the body the CA builds, without the
//! request file: `woltrust issue --proof` does exactly that. The request file is the hub-index
//! `schema/request.schema.json` shape.
use crate::b64;
use crate::cert::{Body, KeysText, Kind};
use crate::crypto::{self, PublicKeys, Sig};
use crate::make::Secret;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The label of a proof of possession.
pub const LABEL: &str = "wolos/request/1";

/// 16 hex characters naming a public Ed25519 key: the first 8 bytes of its SHA-256.
pub fn key_id(ed25519: &[u8; 32]) -> String {
    Sha256::digest(ed25519)[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The bytes a proof signs: one line each for the GitHub id, the kind and the five key fields
/// (base64url as in the body, empty when absent), in this order.
pub fn proof_content(github_id: u64, kind: Kind, keys: &KeysText) -> Vec<u8> {
    let kind = match kind {
        Kind::Person => "person",
        Kind::Org => "org",
        Kind::Root => "root",
        Kind::Intermediate => "intermediate",
        Kind::Signer => "signer",
    };
    let o = |v: &Option<String>| v.clone().unwrap_or_default();
    format!(
        "{github_id}\n{kind}\n{}\n{}\n{}\n{}\n{}\n",
        keys.ed25519,
        o(&keys.ml_dsa_65),
        o(&keys.slh_dsa_sha2_128s),
        o(&keys.x25519),
        o(&keys.ml_kem_768)
    )
    .into_bytes()
}

/// The proof file (`--proof PROOF.json`, the request's `proof` member).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    /// Ed25519 half, base64url.
    pub ed25519: String,
    /// ML-DSA-65 half, base64url.
    pub second: String,
}

/// Signs the proof for `github_id`, `kind` and `keys` with `secret` (the keys' own secret).
pub fn prove(secret: &Secret, github_id: u64, kind: Kind, keys: &KeysText) -> Proof {
    let msg = crypto::message_for(LABEL, &proof_content(github_id, kind, keys));
    let s = secret.sign_message(&msg);
    Proof {
        ed25519: b64::encode(&s.ed25519),
        second: b64::encode(&s.second),
    }
}

/// Checks a proof against a body (person or organisation): the body's own keys must have signed
/// its GitHub id, kind and keys.
pub fn check(body: &Body, keys: &PublicKeys, proof: &Proof) -> Result<(), &'static str> {
    let id = body
        .subject
        .github_id
        .ok_or("the body names no GitHub id")?;
    let ed = b64::decode(&proof.ed25519, crypto::ED_SIG).ok_or("proof malformed")?;
    let second = b64::decode(&proof.second, crypto::SLH_SIG).ok_or("proof malformed")?;
    let sig = Sig {
        ed25519: ed.try_into().map_err(|_| "proof malformed")?,
        second,
    };
    let msg = crypto::message_for(LABEL, &proof_content(id, body.kind, &body.keys));
    crypto::verify_hybrid(keys, &msg, &sig).map_err(|_| "the proof of possession does not verify")
}

/// The request file of the hub-index schema (`action: issue`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Always 1.
    pub format: u32,
    /// Always `issue`.
    pub action: String,
    /// `person` or `org`.
    pub kind: Kind,
    /// GitHub id, login and name.
    pub subject: crate::cert::Subject,
    /// Asked-for prefixes.
    pub namespaces: Vec<String>,
    /// Public keys, encryption keys included.
    pub keys: KeysText,
    /// Proof of possession.
    pub proof: Proof,
    /// Months of validity asked for.
    pub months: u32,
    /// Unix seconds.
    pub requested_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cert::{Level, Subject, Usage};

    fn body(keys: KeysText) -> Body {
        Body {
            serial: "0123456789abcdef".into(),
            kind: Kind::Person,
            subject: Subject {
                github_id: Some(42),
                login: Some("alice".into()),
                name: Some("Alice".into()),
                label: None,
                org: None,
            },
            namespaces: vec!["alice.".into()],
            usage: vec![Usage::SignPackage],
            keys,
            issuer: "fedcba9876543210".into(),
            not_before: 1,
            not_after: 2,
            level: Level::Verified,
            verified_at: None,
        }
    }

    #[test]
    fn a_proof_binds_the_id_kind_and_every_key() {
        let s = Secret::from_seed(&[8; 32], false);
        let keys = crate::make::keys_text(&s);
        let p = prove(&s, 42, Kind::Person, &keys);
        let b = body(keys.clone());
        assert!(check(&b, s.public(), &p).is_ok());
        let mut other = b.clone();
        other.subject.github_id = Some(43);
        assert!(check(&other, s.public(), &p).is_err());
        let mut other = b.clone();
        other.kind = Kind::Org;
        assert!(check(&other, s.public(), &p).is_err());
        let mut other = b.clone();
        other.keys.x25519 = Some("AAAA".into());
        assert!(check(&other, s.public(), &p).is_err());
        let thief = Secret::from_seed(&[9; 32], false);
        assert!(check(&b, thief.public(), &p).is_err());
        assert_eq!(key_id(&s.public().ed25519).len(), 16);
    }
}
