// SPDX-License-Identifier: BSD-2-Clause
//! The `.wolcert` file (format 1, ADR 0046): `{"format":1,"body":"<b64url>","sig":{"ed25519":"..",
//! "second":".."}}`. The body is the exact signed bytes, never re-serialised, so there is no
//! canonical-JSON step. Parsing is bounded (file 24 KiB, body 4 KiB) and every field is checked
//! against the rules of its kind, so a certificate that parses is well formed.
use crate::b64;
use crate::crypto::{
    ED_PK, ED_SIG, EncPublic, KEM_PK, ML_PK, ML_SIG, PublicKeys, SLH_PK, SLH_SIG, Second, Sig, X_PK,
};
use crate::{FORMAT, MAX_BODY, MAX_CERT_FILE};
use serde::{Deserialize, Serialize};

/// What a certificate is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
#[serde(deny_unknown_fields)]
pub enum Kind {
    /// A pinned root; signs intermediates and revocations only.
    Root,
    /// An intermediate: Authors CA, Packages CA or Index.
    Intermediate,
    /// A person (one GitHub account).
    Person,
    /// An organisation (one GitHub organisation).
    Org,
    /// A signer of WolOS's own packages (issued by the Packages CA).
    Signer,
}

/// How a person sees the maker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
#[serde(deny_unknown_fields)]
pub enum Level {
    /// Made by WolOS.
    Wolos,
    /// A certified author or organisation.
    Verified,
}

/// Which of a person's or organisation's two certificates this is (lead decision 2026-11-04):
/// `origin` is issued automatically at GitHub sign-in with the CLA signed, and proves who
/// published and that the id is theirs; `trusted` is issued after an administrator approved
/// them. A body without `class` is an origin certificate: trust must be written in signed bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
#[serde(deny_unknown_fields)]
pub enum Class {
    /// Identity and namespace, proven at sign-in.
    Origin,
    /// Approved by a WolOS Hub administrator.
    Trusted,
}

/// What a certificate may sign.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub enum Usage {
    /// Other certificates.
    SignCerts,
    /// The hub index.
    SignIndex,
    /// Revocation lists.
    SignRevocations,
    /// `.wol` packages.
    SignPackage,
    /// `.wolplug` plugins.
    SignPlugin,
    /// Extensions.
    SignExtension,
}

/// Who a certificate names. Persons and organisations carry the numeric GitHub id (stable across
/// renames); the login and name are shown text. Roots and intermediates carry a label.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Subject {
    /// Numeric GitHub id of the account or organisation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_id: Option<u64>,
    /// GitHub login as shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Label of a root, intermediate or signer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The GitHub organisation a person publishes for, when any (shown as origin; a release
    /// from that organisation's repositories counts as theirs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org: Option<OrgRef>,
    /// A root's human-readable subject, signed with it and shown as who vouches (at most 200
    /// characters, no control characters; roots only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// A GitHub organisation by its permanent numeric id, with the login as shown.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrgRef {
    /// Numeric GitHub id of the organisation.
    pub github_id: u64,
    /// Its login.
    pub login: String,
}

/// The public keys as written in a body (base64url).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeysText {
    /// Ed25519 public key.
    pub ed25519: String,
    /// ML-DSA-65 public key (everything below the roots).
    #[serde(rename = "ml-dsa-65", default, skip_serializing_if = "Option::is_none")]
    pub ml_dsa_65: Option<String>,
    /// SLH-DSA-SHA2-128s public key (roots).
    #[serde(
        rename = "slh-dsa-sha2-128s",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub slh_dsa_sha2_128s: Option<String>,
    /// X25519 encryption key (optional, always together with `ml-kem-768`): the classical half
    /// of the sealed-message KEM. Additive to format 1: a body without it is as before.
    #[serde(rename = "x25519", default, skip_serializing_if = "Option::is_none")]
    pub x25519: Option<String>,
    /// ML-KEM-768 encapsulation key (optional, always together with `x25519`): the
    /// post-quantum half of the sealed-message KEM.
    #[serde(
        rename = "ml-kem-768",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub ml_kem_768: Option<String>,
}

/// The signed body of a certificate.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Body {
    /// 16 lowercase hex characters, random.
    pub serial: String,
    /// What this certificate is.
    pub kind: Kind,
    /// Who it names.
    pub subject: Subject,
    /// App-id prefixes it may sign (leaves) or issue (intermediates, roots).
    pub namespaces: Vec<String>,
    /// What it may sign.
    pub usage: Vec<Usage>,
    /// Its public keys.
    pub keys: KeysText,
    /// Serial of the issuer (its own serial for a root).
    pub issuer: String,
    /// Start of validity, Unix seconds.
    pub not_before: u64,
    /// End of validity (exclusive), Unix seconds.
    pub not_after: u64,
    /// How the maker is shown.
    pub level: Level,
    /// When the hub verified the subject (the GitHub account and the request), Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<u64>,
    /// Origin or trusted (people and organisations only); absent means origin ([`Body::class`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<Class>,
    /// For a trusted certificate: the serial of the same subject's origin certificate, so
    /// withdrawing that one withdraws this one too. Required for `trusted`, refused otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_cert: Option<String>,
}

/// Most characters of a root's subject text.
pub const MAX_TEXT: usize = 200;

/// The id prefix that only the Packages CA may issue for.
pub const RESERVED: &str = "org.wolos.";
/// Namespace of an Authors CA: every ordinary prefix.
pub const ANY: &str = "*";

/// 16 lowercase hex characters.
pub fn is_serial(s: &str) -> bool {
    s.len() == 16
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_login(s: &str) -> bool {
    !s.is_empty() && s.len() <= 39 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A character that changes how text around it is shown without being shown itself: bidi
/// embeddings, overrides and isolates, zero-width marks and joiners, the soft hyphen, the BOM
/// (audit L1/D4: "Alice U+202E evitaN" must not reorder a shown name).
pub fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

fn is_name(s: &str) -> bool {
    !s.trim().is_empty()
        && s.chars().count() <= 128
        && !s.chars().any(|c| c.is_control() || is_invisible(c))
}

/// A prefix like `alice.` or `org.wolos.`: dot-separated lowercase segments, a final dot.
pub fn is_prefix(s: &str) -> bool {
    let Some(body) = s.strip_suffix('.') else {
        return false;
    };
    s.len() <= 64
        && body.split('.').all(|seg| {
            !seg.is_empty()
                && !seg.starts_with('-')
                && !seg.ends_with('-')
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        })
}

/// True when `q` can be issued to ordinary authors: it neither lies inside nor contains
/// [`RESERVED`] (a prefix such as `org.` would cover it).
pub fn is_ordinary(q: &str) -> bool {
    !RESERVED.starts_with(q) && !q.starts_with(RESERVED)
}

/// True when some namespace of `issuer` covers `q`. `*` covers every ordinary prefix; any other
/// entry covers the prefixes that start with it. `q == "*"` needs `*` itself.
pub fn covers(issuer: &[String], q: &str) -> bool {
    issuer.iter().any(|p| {
        if p == ANY {
            q == ANY || is_ordinary(q)
        } else {
            q != ANY && q.starts_with(p.as_str())
        }
    })
}

fn has_usage(u: &[Usage], one: Usage) -> bool {
    u.contains(&one)
}

impl Body {
    /// Rules of the kind (see ADR 0046): who may carry which usage, level, namespace and key set.
    fn check(&self) -> Result<(), &'static str> {
        use Usage::{
            SignCerts, SignExtension, SignIndex, SignPackage, SignPlugin, SignRevocations,
        };
        if !is_serial(&self.serial) || !is_serial(&self.issuer) {
            return Err("serial malformed");
        }
        if self.not_before >= self.not_after {
            return Err("validity is empty");
        }
        if self.usage.is_empty() || self.usage.len() > 6 || self.namespaces.len() > 8 {
            return Err("usage or namespaces out of range");
        }
        let s = &self.subject;
        if let Some(l) = &s.label
            && !is_label(l)
        {
            return Err("label malformed");
        }
        if let Some(l) = &s.login
            && !is_login(l)
        {
            return Err("login malformed");
        }
        if let Some(n) = &s.name
            && !is_name(n)
        {
            return Err("name malformed");
        }
        if let Some(o) = &s.org
            && (self.kind != Kind::Person || !is_login(&o.login))
        {
            return Err("only a person names an organisation, by a valid login");
        }
        if let Some(t) = &s.text
            && (self.kind != Kind::Root
                || t.trim().is_empty()
                || t.chars().count() > MAX_TEXT
                || t.chars().any(|c| c.is_control() || is_invisible(c)))
        {
            return Err("only a root carries a subject text, of 1 to 200 characters");
        }
        let author = matches!(self.kind, Kind::Person | Kind::Org);
        match (self.class, &self.origin_cert) {
            (Some(_), _) if !author => return Err("only an author certificate has a class"),
            (Some(Class::Trusted), Some(o)) if is_serial(o) && *o != self.serial => {}
            (Some(Class::Trusted), _) => {
                return Err("a trusted certificate names its origin certificate");
            }
            (_, Some(_)) => return Err("only a trusted certificate names an origin certificate"),
            (_, None) => {}
        }
        if self.verified_at.is_some_and(|t| t >= self.not_after) {
            return Err("verified after the end of validity");
        }
        let ns_ok = self.namespaces.iter().all(|n| n == ANY || is_prefix(n));
        if !ns_ok {
            return Err("namespace malformed");
        }
        let only = |allowed: &[Usage]| self.usage.iter().all(|u| allowed.contains(u));
        let leaf_usage = [SignPackage, SignPlugin, SignExtension];
        match self.kind {
            Kind::Root => {
                if s.label.is_none() || s.github_id.is_some() {
                    return Err("a root is named by a label");
                }
                if !only(&[SignCerts, SignRevocations]) || self.level != Level::Wolos {
                    return Err("a root signs certificates and revocations for WolOS");
                }
                if self.issuer != self.serial || self.keys.slh_dsa_sha2_128s.is_none() {
                    return Err("a root is self-issued with an SLH-DSA key");
                }
                if self.namespaces.is_empty() {
                    return Err("a root needs namespaces to delegate");
                }
            }
            Kind::Intermediate => {
                if s.label.is_none() || s.github_id.is_some() {
                    return Err("an intermediate is named by a label");
                }
                if !only(&[SignCerts, SignIndex, SignRevocations]) || self.level != Level::Wolos {
                    return Err("an intermediate signs certificates, the index or revocations");
                }
                if has_usage(&self.usage, SignCerts) == self.namespaces.is_empty() {
                    return Err("only a certificate issuer carries namespaces");
                }
            }
            Kind::Person | Kind::Org => {
                if s.github_id.is_none() || s.login.is_none() || s.name.is_none() {
                    return Err("an author needs a GitHub id, login and name");
                }
                if !only(&leaf_usage) || self.level != Level::Verified {
                    return Err("an author signs packages, plugins and extensions as verified");
                }
                if self.namespaces.is_empty()
                    || !self.namespaces.iter().all(|n| n != ANY && is_ordinary(n))
                {
                    return Err("an author's namespaces are ordinary prefixes");
                }
            }
            Kind::Signer => {
                if s.label.is_none() || s.github_id.is_some() {
                    return Err("a signer is named by a label");
                }
                if !only(&leaf_usage) || self.level != Level::Wolos {
                    return Err("a signer signs packages, plugins and extensions as WolOS");
                }
                if self.namespaces.is_empty()
                    || !self.namespaces.iter().all(|n| n.starts_with(RESERVED))
                {
                    return Err("a signer's namespaces are inside org.wolos.");
                }
            }
        }
        if self.kind != Kind::Root && self.keys.ml_dsa_65.is_none() {
            return Err("this kind carries an ML-DSA-65 key");
        }
        if self.keys.ml_dsa_65.is_some() == self.keys.slh_dsa_sha2_128s.is_some() {
            return Err("exactly one post-quantum key");
        }
        if self.keys.x25519.is_some() != self.keys.ml_kem_768.is_some() {
            return Err("an encryption key has both halves or none");
        }
        Ok(())
    }

    /// Origin or trusted; an author body without `class` is origin. None for roots,
    /// intermediates and WolOS signers.
    pub fn class(&self) -> Option<Class> {
        matches!(self.kind, Kind::Person | Kind::Org).then(|| self.class.unwrap_or(Class::Origin))
    }

    /// Parses and checks a body on its own (the bytes `woltrust issue` is asked to sign): the
    /// same rules as inside a certificate, the keys decoded.
    pub fn parse(bytes: &[u8]) -> Result<(Body, PublicKeys, Option<EncPublic>), &'static str> {
        if bytes.len() > MAX_BODY {
            return Err("certificate body too large");
        }
        let body: Body = serde_json::from_slice(bytes).map_err(|_| "certificate body damaged")?;
        body.check()?;
        let keys = decode_keys(&body.keys)?;
        let enc = decode_enc(&body.keys)?;
        Ok((body, keys, enc))
    }
}

pub(crate) fn decode_keys(k: &KeysText) -> Result<PublicKeys, &'static str> {
    let ed = b64::decode(&k.ed25519, ED_PK).ok_or("key malformed")?;
    let second = match (&k.ml_dsa_65, &k.slh_dsa_sha2_128s) {
        (Some(k), None) => {
            let raw = b64::decode(k, ML_PK).ok_or("key malformed")?;
            Second::MlDsa65(Box::new(raw.try_into().map_err(|_| "key length")?))
        }
        (None, Some(k)) => {
            let raw = b64::decode(k, SLH_PK).ok_or("key malformed")?;
            Second::Slh(Box::new(raw.try_into().map_err(|_| "key length")?))
        }
        _ => return Err("exactly one post-quantum key"),
    };
    Ok(PublicKeys {
        ed25519: ed.try_into().map_err(|_| "key length")?,
        second,
    })
}

fn decode_enc(k: &KeysText) -> Result<Option<EncPublic>, &'static str> {
    let (Some(x), Some(m)) = (&k.x25519, &k.ml_kem_768) else {
        return Ok(None);
    };
    let x = b64::decode(x, X_PK).ok_or("encryption key malformed")?;
    let m = b64::decode(m, KEM_PK).ok_or("encryption key malformed")?;
    Ok(Some(EncPublic {
        x25519: x.try_into().map_err(|_| "encryption key length")?,
        ml_kem_768: Box::new(m.try_into().map_err(|_| "encryption key length")?),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    format: u32,
    body: String,
    sig: SigText,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SigText {
    ed25519: String,
    second: String,
}

/// A parsed certificate. Its signature is not yet checked: the chain code does that.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cert {
    /// The decoded body.
    pub body: Body,
    /// The exact signed bytes.
    pub body_bytes: Vec<u8>,
    /// Decoded public keys.
    pub keys: PublicKeys,
    /// Decoded encryption keys, when the body carries them (sealed messages, `crate::seal`).
    pub enc: Option<EncPublic>,
    /// The issuer's signature over the body (label `wolos/cert/1`).
    pub sig: Sig,
}

impl Cert {
    /// Parses and checks one `.wolcert`; the error is a short English reason for a log.
    pub fn parse(bytes: &[u8]) -> Result<Cert, &'static str> {
        if bytes.len() > MAX_CERT_FILE {
            return Err("certificate file too large");
        }
        let env: Envelope = serde_json::from_slice(bytes).map_err(|_| "certificate damaged")?;
        if env.format != FORMAT {
            return Err("unknown certificate format");
        }
        let body_bytes = b64::decode(&env.body, MAX_BODY).ok_or("certificate body malformed")?;
        let body: Body =
            serde_json::from_slice(&body_bytes).map_err(|_| "certificate body damaged")?;
        body.check()?;
        let keys = decode_keys(&body.keys)?;
        let enc = decode_enc(&body.keys)?;
        let sig_ed = b64::decode(&env.sig.ed25519, ED_SIG).ok_or("signature malformed")?;
        let sig_second = b64::decode(&env.sig.second, SLH_SIG).ok_or("signature malformed")?;
        if sig_second.len() != ML_SIG && sig_second.len() != SLH_SIG {
            return Err("signature length");
        }
        Ok(Cert {
            body,
            body_bytes,
            keys,
            enc,
            sig: Sig {
                ed25519: sig_ed.try_into().map_err(|_| "signature length")?,
                second: sig_second,
            },
        })
    }

    /// The `.wolcert` file bytes for this body and signature (deterministic).
    pub fn to_bytes(&self) -> Vec<u8> {
        encode(&self.body_bytes, &self.sig)
    }

    /// True when `now` lies inside the validity (end exclusive).
    pub fn valid_at(&self, t: u64) -> bool {
        self.body.not_before <= t && t < self.body.not_after
    }
}

/// `.wolcert` bytes from a body's exact bytes and the issuer's signature.
pub fn encode(body_bytes: &[u8], sig: &Sig) -> Vec<u8> {
    #[derive(Serialize)]
    struct Out<'a> {
        format: u32,
        body: String,
        sig: &'a SigText,
    }
    let text = SigText {
        ed25519: b64::encode(&sig.ed25519),
        second: b64::encode(&sig.second),
    };
    let out = Out {
        format: FORMAT,
        body: b64::encode(body_bytes),
        sig: &text,
    };
    serde_json::to_vec(&out).unwrap_or_default()
}

/// Splits a `CHAIN` member (a JSON array of `.wolcert` objects) into certificates, leaf first.
/// At most [`crate::MAX_CHAIN_CERTS`] certificates and [`crate::MAX_CHAIN_BYTES`] bytes; a root
/// does not belong in a chain.
pub fn parse_chain(bytes: &[u8]) -> Result<Vec<Cert>, ChainError> {
    if bytes.len() > crate::MAX_CHAIN_BYTES {
        return Err(ChainError::TooLarge);
    }
    let items: Vec<serde_json::Value> =
        serde_json::from_slice(bytes).map_err(|_| ChainError::Malformed("chain damaged"))?;
    if items.len() > crate::MAX_CHAIN_CERTS {
        return Err(ChainError::TooLong);
    }
    let mut out = Vec::with_capacity(items.len());
    for item in &items {
        let raw = serde_json::to_vec(item).map_err(|_| ChainError::Malformed("chain damaged"))?;
        let c = Cert::parse(&raw).map_err(ChainError::Malformed)?;
        if c.body.kind == Kind::Root {
            return Err(ChainError::Malformed("a root does not belong in a chain"));
        }
        if out.iter().any(|o: &Cert| o.body.serial == c.body.serial) {
            return Err(ChainError::Malformed("a certificate appears twice"));
        }
        out.push(c);
    }
    Ok(out)
}

/// Why a `CHAIN` member was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainError {
    /// More than 64 KiB.
    TooLarge,
    /// More than four certificates.
    TooLong,
    /// Anything else, with a reason for a log.
    Malformed(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_follow_the_naming_rules() {
        for ok in ["alice.", "example-labs.", "a1.b2.", "org.wolosx."] {
            assert!(is_prefix(ok), "{ok}");
        }
        for bad in ["", ".", "alice", "Alice.", "-a.", "a-.", "a..b.", "a b."] {
            assert!(!is_prefix(bad), "{bad}");
        }
    }

    #[test]
    fn the_reserved_prefix_is_never_ordinary() {
        for q in ["org.", "org.wolos.", "org.wolos.notes.", "o"] {
            assert!(!is_ordinary(q), "{q}");
        }
        assert!(is_ordinary("alice."));
        assert!(is_ordinary("org.wolosx."));
        assert!(covers(&["*".into()], "alice."));
        assert!(!covers(&["*".into()], "org.wolos."));
        assert!(covers(&["org.wolos.".into()], "org.wolos.sub."));
        assert!(!covers(&["org.wolos.".into()], "*"));
    }

    #[test]
    fn garbage_never_panics() {
        for bytes in [
            &b""[..],
            b"{}",
            b"[]",
            b"null",
            b"{\"format\":1}",
            &[0xff; 40][..],
        ] {
            assert!(Cert::parse(bytes).is_err());
            assert!(parse_chain(bytes).is_err() || bytes == b"[]");
        }
    }
}
