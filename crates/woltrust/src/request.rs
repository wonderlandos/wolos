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

/// What the CA copies from a request into the certificate, all covered by the proof (audit H2):
/// a body that differs in any of them does not verify against the proof.
#[derive(Clone, Copy, Debug)]
pub struct Claimed<'a> {
    /// GitHub id.
    pub github_id: u64,
    /// Person or organisation.
    pub kind: Kind,
    /// GitHub login.
    pub login: &'a str,
    /// Display name.
    pub name: &'a str,
    /// The organisation a person publishes for, if any.
    pub org: Option<&'a crate::cert::OrgRef>,
    /// The namespaces asked for, in order.
    pub namespaces: &'a [String],
    /// Every public key.
    pub keys: &'a KeysText,
}

impl<'a> Claimed<'a> {
    /// The claims of a body the CA is asked to sign (None without a GitHub id, login or name).
    pub fn of_body(b: &'a Body) -> Option<Claimed<'a>> {
        Some(Claimed {
            github_id: b.subject.github_id?,
            kind: b.kind,
            login: b.subject.login.as_deref()?,
            name: b.subject.name.as_deref()?,
            org: b.subject.org.as_ref(),
            namespaces: &b.namespaces,
            keys: &b.keys,
        })
    }

    /// The claims of a request file.
    pub fn of_request(r: &'a Request) -> Option<Claimed<'a>> {
        Some(Claimed {
            github_id: r.subject.github_id?,
            kind: r.kind,
            login: r.subject.login.as_deref()?,
            name: r.subject.name.as_deref()?,
            org: r.subject.org.as_ref(),
            namespaces: &r.namespaces,
            keys: &r.keys,
        })
    }

    /// The bytes a proof signs: the JSON array `[github_id, kind, login, name, org, namespaces,
    /// ed25519, ml-dsa-65, slh-dsa-sha2-128s, x25519, ml-kem-768]` (absent values `null`, `org` as
    /// `{"github_id","login"}`), so no field can bleed into the next.
    pub fn content(&self) -> Vec<u8> {
        let org = self
            .org
            .map(|o| serde_json::json!({"github_id": o.github_id, "login": o.login}));
        let k = self.keys;
        serde_json::to_vec(&serde_json::json!([
            self.github_id,
            kind_name(self.kind),
            self.login,
            self.name,
            org,
            self.namespaces,
            k.ed25519,
            k.ml_dsa_65,
            k.slh_dsa_sha2_128s,
            k.x25519,
            k.ml_kem_768,
        ]))
        .unwrap_or_default()
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Person => "person",
        Kind::Org => "org",
        Kind::Root => "root",
        Kind::Intermediate => "intermediate",
        Kind::Signer => "signer",
    }
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

/// Signs the proof of `claimed` with `secret` (the keys' own secret).
pub fn prove(secret: &Secret, claimed: &Claimed) -> Proof {
    let msg = crypto::message_for(LABEL, &claimed.content());
    let s = secret.sign_message(&msg);
    Proof {
        ed25519: b64::encode(&s.ed25519),
        second: b64::encode(&s.second),
    }
}

/// Checks a proof against a body (person or organisation): the body's own keys must have signed
/// every field the CA copied: GitHub id, kind, login, name, organisation, namespaces and keys. A
/// body that names another login or namespace than was asked for is refused.
pub fn check(body: &Body, keys: &PublicKeys, proof: &Proof) -> Result<(), &'static str> {
    let claimed = Claimed::of_body(body).ok_or("the body names no GitHub id, login or name")?;
    let ed = b64::decode(&proof.ed25519, crypto::ED_SIG).ok_or("proof malformed")?;
    let second = b64::decode(&proof.second, crypto::SLH_SIG).ok_or("proof malformed")?;
    let sig = Sig {
        ed25519: ed.try_into().map_err(|_| "proof malformed")?,
        second,
    };
    let msg = crypto::message_for(LABEL, &claimed.content());
    crypto::verify_hybrid(keys, &msg, &sig)
        .map_err(|_| "the proof does not verify: the body differs from what was requested")
}

/// The request file of the hub-index schema (`action: issue`). The struct itself is strict;
/// [`parse`] drops the top-level fields it does not name before reading it, so the hub's copy
/// (`requests/<id>.json`, with `id`, `approved_by`, `approved_at` and `verified_at`, which the CA
/// reads itself) reads as the request. Nothing ignored is signed or trusted: the proof covers
/// what the CA copies ([`Claimed`], [`Request::check_proof`]).
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

/// Largest request file read: keys and proof are about 7 KB of base64, the hub's additions a few
/// hundred bytes.
pub const MAX_REQUEST: usize = 64 * 1024;

/// Why a request file is refused, with the stable string the request vectors check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// Larger than [`MAX_REQUEST`].
    TooLarge,
    /// Not JSON, or a named field missing or of the wrong type.
    Damaged,
    /// Not `format` 1 with `action` `issue`, or not a person or organisation with a GitHub id.
    Unsupported,
    /// The keys are malformed, or the proof of possession does not verify with them.
    BadProof,
}

impl RequestError {
    /// The stable name (`request-too-large`, `request-damaged`, `request-unsupported`,
    /// `request-bad-proof`).
    pub fn as_str(self) -> &'static str {
        match self {
            RequestError::TooLarge => "request-too-large",
            RequestError::Damaged => "request-damaged",
            RequestError::Unsupported => "request-unsupported",
            RequestError::BadProof => "request-bad-proof",
        }
    }
}

/// The top-level fields of a [`Request`]; [`parse`] ignores every other one.
pub const FIELDS: [&str; 9] = [
    "format",
    "action",
    "kind",
    "subject",
    "namespaces",
    "keys",
    "proof",
    "months",
    "requested_at",
];

/// Reads a request file within [`MAX_REQUEST`]. Unknown top-level fields are ignored (the hub's
/// additions); inside `subject`, `keys` and `proof` every field must be known.
pub fn parse(bytes: &[u8]) -> Result<Request, RequestError> {
    if bytes.len() > MAX_REQUEST {
        return Err(RequestError::TooLarge);
    }
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| RequestError::Damaged)?;
    let known: serde_json::Map<String, serde_json::Value> = v
        .as_object()
        .ok_or(RequestError::Damaged)?
        .iter()
        .filter(|(k, _)| FIELDS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    serde_json::from_value(serde_json::Value::Object(known)).map_err(|_| RequestError::Damaged)
}

impl Request {
    /// The request's own keys signed its GitHub id, kind and keys (label [`LABEL`]): the same
    /// check `woltrust issue --proof` makes against the body the CA builds from it.
    pub fn check_proof(&self) -> Result<(), RequestError> {
        if self.format != 1
            || self.action != "issue"
            || !matches!(self.kind, Kind::Person | Kind::Org)
        {
            return Err(RequestError::Unsupported);
        }
        let claimed = Claimed::of_request(self).ok_or(RequestError::Unsupported)?;
        let keys = crate::cert::decode_keys(&self.keys).map_err(|_| RequestError::BadProof)?;
        let half = |t: &str, max| b64::decode(t, max).ok_or(RequestError::BadProof);
        let sig = Sig {
            ed25519: half(&self.proof.ed25519, crypto::ED_SIG)?
                .try_into()
                .map_err(|_| RequestError::BadProof)?,
            second: half(&self.proof.second, crypto::SLH_SIG)?,
        };
        let msg = crypto::message_for(LABEL, &claimed.content());
        crypto::verify_hybrid(&keys, &msg, &sig).map_err(|_| RequestError::BadProof)
    }
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
                text: None,
            },
            namespaces: vec!["alice.".into()],
            usage: vec![Usage::SignPackage],
            keys,
            issuer: "fedcba9876543210".into(),
            not_before: 1,
            not_after: 2,
            level: Level::Verified,
            verified_at: None,
            class: None,
            origin_cert: None,
        }
    }

    #[test]
    fn a_proof_binds_every_field_the_ca_copies() {
        let s = Secret::from_seed(&[8; 32], false);
        let keys = crate::make::keys_text(&s);
        let b = body(keys.clone());
        let p = prove(&s, &Claimed::of_body(&b).unwrap());
        assert!(check(&b, s.public(), &p).is_ok());
        // Audit H2: the CA cannot add a namespace, change the login or name, or add an org.
        let mut other = b.clone();
        other.namespaces.push("mozilla.".into());
        assert!(check(&other, s.public(), &p).is_err(), "extra namespace");
        let mut other = b.clone();
        other.subject.login = Some("bob".into());
        assert!(check(&other, s.public(), &p).is_err(), "other login");
        let mut other = b.clone();
        other.subject.name = Some("Bob".into());
        assert!(check(&other, s.public(), &p).is_err(), "other name");
        let mut other = b.clone();
        other.subject.org = Some(crate::cert::OrgRef {
            github_id: 7,
            login: "mozilla".into(),
        });
        assert!(check(&other, s.public(), &p).is_err(), "org added");
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
