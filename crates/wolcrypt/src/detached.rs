// SPDX-License-Identifier: BSD-2-Clause
//! A witness's detached signature, checked with nothing but wolcrypt (`wolcrypt verify-detached`,
//! the C ABI's `wolcrypt_verify_detached`, the wasm build behind wolos.org/verify): the file, its
//! `.wsig`, the witness certificate and the two pinned root certificates go in; the signature
//! line comes out. Checked: both roots are self-signed roots whose id is SHA-512 of their keys
//! (16-hex serials only for roots labelled `test...`); the witness is an intermediate with usage
//! `witness` alone, its id SHA-512 of its keys, signed by one root and co-signed by the other; the
//! `.wsig` names it, carries `at` inside the witness's validity, and both halves verify over
//! `"wolos/witness/1" 0x00 || SHA-512(file) || SHA-512(at as decimal text)`. Not checked here:
//! revocation (crate woltrust does that with the lists); this is the offline proof that the file
//! is the one WolOS published.
use crate::sign::{self, ED_SIG, PublicKeys, Second, Sig};
use serde::Deserialize;
use std::format;
use std::string::{String, ToString};
use std::vec::Vec;

/// Why a detached signature does not verify.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A file is not the JSON it should be.
    Malformed,
    /// A root is not a self-signed root, or its id does not match its keys.
    Root,
    /// The witness certificate is not a witness of these two roots.
    Witness,
    /// The signature does not verify, or names another witness, or lies outside its validity.
    Signature,
}

impl Error {
    /// Stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Error::Malformed => "malformed",
            Error::Root => "root",
            Error::Witness => "witness",
            Error::Signature => "signature",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SigText {
    ed25519: String,
    second: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CoSigText {
    issuer: String,
    ed25519: String,
    second: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    format: u32,
    body: String,
    sig: SigText,
    #[serde(default)]
    sigs: Vec<CoSigText>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Keys {
    ed25519: String,
    #[serde(rename = "ml-dsa-65")]
    ml_dsa_65: Option<String>,
    #[serde(rename = "slh-dsa-sha2-128s")]
    slh: Option<String>,
    x25519: Option<String>,
    #[serde(rename = "ml-kem-768")]
    ml_kem_768: Option<String>,
}

/// The subject: only the label and text matter here; the rest of a body's fields are named so a
/// typo is refused, as by crate woltrust.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Subject {
    label: Option<String>,
    text: Option<String>,
    github_id: Option<u64>,
    login: Option<String>,
    name: Option<String>,
    org: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    serial: String,
    kind: String,
    subject: Subject,
    #[serde(default)]
    namespaces: Vec<String>,
    usage: Vec<String>,
    keys: Keys,
    issuer: String,
    not_before: u64,
    not_after: u64,
    level: String,
    verified_at: Option<u64>,
    class: Option<String>,
    origin_cert: Option<String>,
    terms: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wsig {
    cert: String,
    ed25519: String,
    second: String,
    at: u64,
}

struct Cert {
    body: Body,
    body_bytes: Vec<u8>,
    keys: PublicKeys,
    sig: Sig,
    cosigs: Vec<(String, Sig)>,
}

fn sig_of(ed: &str, second: &str) -> Option<Sig> {
    Some(Sig {
        ed25519: crate::b64::decode(ed, ED_SIG)?.try_into().ok()?,
        second: crate::b64::decode(second, sign::SLH_SIG)?,
    })
}

fn parse_cert(raw: &[u8]) -> Result<Cert, Error> {
    if raw.len() > 40 * 1024 {
        return Err(Error::Malformed);
    }
    let env: Envelope = serde_json::from_slice(raw).map_err(|_| Error::Malformed)?;
    let body_bytes = crate::b64::decode(&env.body, 8192).ok_or(Error::Malformed)?;
    let body: Body = serde_json::from_slice(&body_bytes).map_err(|_| Error::Malformed)?;
    if env.format != 1 {
        return Err(Error::Malformed);
    }
    let ed: [u8; 32] = crate::b64::decode(&body.keys.ed25519, 32)
        .and_then(|v| v.try_into().ok())
        .ok_or(Error::Malformed)?;
    let second = match (&body.keys.ml_dsa_65, &body.keys.slh) {
        (Some(k), None) => Second::MlDsa65(Box::new(
            crate::b64::decode(k, sign::ML_PK)
                .and_then(|v| v.try_into().ok())
                .ok_or(Error::Malformed)?,
        )),
        (None, Some(k)) => Second::Slh(Box::new(
            crate::b64::decode(k, sign::SLH_PK)
                .and_then(|v| v.try_into().ok())
                .ok_or(Error::Malformed)?,
        )),
        _ => return Err(Error::Malformed),
    };
    let sig = sig_of(&env.sig.ed25519, &env.sig.second).ok_or(Error::Malformed)?;
    let cosigs = env
        .sigs
        .iter()
        .map(|c| Some((c.issuer.clone(), sig_of(&c.ed25519, &c.second)?)))
        .collect::<Option<Vec<_>>>()
        .ok_or(Error::Malformed)?;
    Ok(Cert {
        body,
        body_bytes,
        keys: PublicKeys {
            ed25519: ed,
            second,
        },
        sig,
        cosigs,
    })
}

/// A WolOS CA names no person: no GitHub fields, no class, no origin, no terms.
fn names_no_person(b: &Body) -> bool {
    let s = &b.subject;
    s.github_id.is_none()
        && s.login.is_none()
        && s.name.is_none()
        && s.org.is_none()
        && b.verified_at.is_none()
        && b.class.is_none()
        && b.origin_cert.is_none()
        && b.terms.is_none()
}

fn is_test(c: &Cert) -> bool {
    c.body
        .subject
        .label
        .as_deref()
        .is_some_and(|l| l.starts_with("test"))
}

fn id_ok(c: &Cert) -> bool {
    let id = crate::id::to_hex(&crate::id::of_keys(&c.keys, None));
    c.body.serial == id || (is_test(c) && c.body.serial.len() == 16)
}

fn cert_message(c: &Cert) -> Vec<u8> {
    sign::message("wolos/cert/1", &c.body_bytes)
}

/// Checks `wsig` over `file` by `witness` under the pinned `roots`; returns the signature line
/// `<WolOS Witness A|B> <sha512:HASH> <YYYY-MM-DD> <wolos.org/trust/ID>`.
pub fn verify(
    file: &[u8],
    wsig: &[u8],
    witness: &[u8],
    roots: [&[u8]; 2],
) -> Result<String, Error> {
    let r: Vec<Cert> = roots
        .iter()
        .map(|b| parse_cert(b))
        .collect::<Result<_, _>>()?;
    for c in &r {
        let root = c.body.kind == "root"
            && matches!(c.keys.second, Second::Slh(_))
            && c.body.issuer == c.body.serial
            && c.body.level == "wolos"
            && names_no_person(&c.body)
            && id_ok(c)
            && sign::verify_message(&c.keys, &cert_message(c), &c.sig).is_ok();
        if !root {
            return Err(Error::Root);
        }
    }
    if r[0].body.serial == r[1].body.serial {
        return Err(Error::Root);
    }
    let w = parse_cert(witness)?;
    let shape = w.body.kind == "intermediate"
        && w.body.usage == ["witness"]
        && w.body.level == "wolos"
        && w.body.keys.x25519.is_none()
        && w.body.keys.ml_kem_768.is_none()
        && w.body.namespaces.is_empty()
        && w.body.subject.text.is_none()
        && names_no_person(&w.body)
        && matches!(w.keys.second, Second::MlDsa65(_))
        && crate::id::is_hex(&w.body.serial)
        && id_ok(&w);
    if !shape {
        return Err(Error::Witness);
    }
    let (issuer, other) = match r.iter().position(|c| c.body.serial == w.body.issuer) {
        Some(0) => (&r[0], &r[1]),
        Some(_) => (&r[1], &r[0]),
        None => return Err(Error::Witness),
    };
    let msg = cert_message(&w);
    let co = w
        .cosigs
        .iter()
        .find(|(i, _)| *i == other.body.serial)
        .ok_or(Error::Witness)?;
    if sign::verify_message(&issuer.keys, &msg, &w.sig).is_err()
        || sign::verify_message(&other.keys, &msg, &co.1).is_err()
    {
        return Err(Error::Witness);
    }
    if wsig.len() > 16 * 1024 {
        return Err(Error::Malformed);
    }
    let s: Wsig = serde_json::from_slice(wsig).map_err(|_| Error::Malformed)?;
    let sig = sig_of(&s.ed25519, &s.second).ok_or(Error::Malformed)?;
    let inside = w.body.not_before <= s.at && s.at < w.body.not_after;
    let at_text = s.at.to_string();
    let m = sign::message_with("wolos/witness/1", file, &[at_text.as_bytes()]);
    if s.cert != w.body.serial || !inside || sign::verify_message(&w.keys, &m, &sig).is_err() {
        return Err(Error::Signature);
    }
    let label = w.body.subject.label.as_deref().unwrap_or_default();
    let name = format!(
        "{}WolOS Witness {}",
        if is_test(&w) { "TEST " } else { "" },
        if label.ends_with("-b") { "B" } else { "A" }
    );
    Ok(format!(
        "<{name}> <sha512:{}> <{}> <wolos.org/trust/{}>",
        crate::hash::hex(&crate::hash::sha512(file)),
        crate::text::date(s.at),
        w.body.serial
    ))
}
