// SPDX-License-Identifier: BSD-2-Clause
//! The checks of each layer (signatures.md, "The chain of trust"). Records are read as JSON
//! values, so the signed bytes are exactly the fields the signer wrote ([`crate::canon`]).
use crate::canon;
use ed25519_dalek::{Signature, VerifyingKey};
use serde_json::{Map, Value};
use std::fmt;

/// The layer that failed, named as the protocol's security page names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Root,
    Tld,
    Domain,
    Manifest,
    Content,
}

impl Layer {
    pub fn as_str(self) -> &'static str {
        match self {
            Layer::Root => "root",
            Layer::Tld => "tld",
            Layer::Domain => "domain",
            Layer::Manifest => "manifest",
            Layer::Content => "content",
        }
    }
}

/// A check that failed: nothing past it is used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub layer: Layer,
    /// What was checked (`home.fed`, `.fed`, a block address).
    pub subject: String,
    pub reason: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} check failed for {}: {}",
            self.layer.as_str(),
            self.subject,
            self.reason
        )
    }
}

fn fail<T>(layer: Layer, subject: &str, reason: impl Into<String>) -> Result<T, Failure> {
    Err(Failure {
        layer,
        subject: subject.to_owned(),
        reason: reason.into(),
    })
}

fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let b = s.as_bytes();
    if b.len() != N * 2 {
        return None;
    }
    let nib = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = (nib(b[2 * i])? << 4) | nib(b[2 * i + 1])?;
    }
    Some(out)
}

/// Whether `sig_hex` is `public_hex`'s Ed25519 signature of `msg` (both hex, as the protocol
/// writes them).
pub fn signature_ok(public_hex: &str, msg: &[u8], sig_hex: &str) -> bool {
    let (Some(pk), Some(sig)) = (unhex::<32>(public_hex), unhex::<64>(sig_hex)) else {
        return false;
    };
    let Ok(key) = VerifyingKey::from_bytes(&pk) else {
        return false;
    };
    // Strict: no small-order keys, no malleable signatures (one valid signature per message).
    key.verify_strict(msg, &Signature::from_bytes(&sig)).is_ok()
}

/// The content address of `bytes`: BLAKE3, lower-case hex.
pub fn address(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Whether `s` reads as a content address (64 hex digits), so it is safe in a request path.
pub fn is_address(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `bytes` hash to `want`, or the failure for `layer`.
pub fn check_address(layer: Layer, bytes: &[u8], want: &str) -> Result<(), Failure> {
    let got = address(bytes);
    if got != want {
        return fail(
            layer,
            want,
            format!("the bytes hash to {got}, not to their address"),
        );
    }
    Ok(())
}

pub(crate) fn text<'a>(m: &'a Map<String, Value>, k: &str) -> &'a str {
    m.get(k).and_then(Value::as_str).unwrap_or("")
}

/// A record's own Ed25519 signature by `key`.
fn signed_by(layer: Layer, subject: &str, record: &Value, key: &str) -> Result<(), Failure> {
    let m = record.as_object().map_or_else(
        || fail(layer, subject, "the record is not a JSON object"),
        Ok,
    )?;
    let alg = text(m, "signature_algorithm");
    if alg != "ed25519" {
        return fail(
            layer,
            subject,
            format!("signature algorithm {alg:?} is not ed25519"),
        );
    }
    let Some(sig) = m.get("signature").and_then(Value::as_str) else {
        return fail(layer, subject, "the record is unsigned");
    };
    let msg = canon::signable(record).unwrap_or_default();
    if !signature_ok(key, &msg, sig) {
        return fail(
            layer,
            subject,
            "the signature does not verify against the key that must have made it",
        );
    }
    Ok(())
}

/// The oldest root zone WolOS accepts: the one this build was made against (2026-11-04). An
/// older zone, however well signed, is a replay.
pub const MIN_ROOT_VERSION: u64 = 1_783_402_193;

/// A root zone that verified against the pinned root key, with every TLD record in it.
#[derive(Debug, Clone)]
pub struct Zone {
    pub version: u64,
    /// The BLAKE3 address of the zone's bytes: two zones of one version must be the same zone.
    pub digest: String,
    /// `generated_at` in seconds since 1970, when it reads as a time: no record is judged at an
    /// earlier moment than this, so a clock set back cannot bring an expired record back.
    pub generated_at: Option<i64>,
    zone: Map<String, Value>,
}

impl Zone {
    /// Reads and verifies a root zone: it names `root_key` as its key, it is signed by it, and so
    /// is each TLD record.
    pub fn verify(bytes: &[u8], root_key: &str) -> Result<Zone, Failure> {
        let subject = "the root zone";
        let v: Value = serde_json::from_slice(bytes).map_or_else(
            |e| fail(Layer::Root, subject, format!("not JSON ({e})")),
            Ok,
        )?;
        let Some(m) = v.as_object() else {
            return fail(Layer::Root, subject, "not a JSON object");
        };
        if text(m, "network") != "federate" {
            return fail(Layer::Root, subject, "not a Federate root zone");
        }
        if text(m, "root_public_key") != root_key {
            return fail(
                Layer::Root,
                subject,
                "it names another root key than the one WolOS trusts",
            );
        }
        signed_by(Layer::Root, subject, &v, root_key)?;
        let Some(version) = m.get("root_version").and_then(Value::as_u64) else {
            return fail(Layer::Root, subject, "no root_version");
        };
        if version < MIN_ROOT_VERSION {
            return fail(
                Layer::Root,
                subject,
                format!(
                    "version {version} is older than the zone this WolOS was built with ({MIN_ROOT_VERSION})"
                ),
            );
        }
        let Some(tlds) = m.get("tlds").and_then(Value::as_object) else {
            return fail(Layer::Root, subject, "no TLD table");
        };
        for (name, rec) in tlds {
            let subject = format!(".{name}");
            signed_by(Layer::Tld, &subject, rec, root_key)?;
            if rec.get("tld").and_then(Value::as_str) != Some(name.as_str()) {
                return fail(Layer::Tld, &subject, "the record is for another TLD");
            }
        }
        if !m.get("domains").is_some_and(Value::is_object) {
            return fail(Layer::Root, subject, "no domain table");
        }
        Ok(Zone {
            version,
            digest: address(bytes),
            generated_at: crate::time::unix_seconds(text(m, "generated_at")),
            zone: m.clone(),
        })
    }

    pub fn tld(&self, name: &str) -> Option<&Map<String, Value>> {
        self.zone.get("tlds")?.get(name)?.as_object()
    }

    pub fn tld_names(&self) -> Vec<String> {
        self.zone
            .get("tlds")
            .and_then(Value::as_object)
            .map(|t| t.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// The root-managed domain record for `domain`, as the zone holds it (verified by [`domain`]).
    pub fn domain(&self, domain: &str) -> Option<&Value> {
        self.zone.get("domains")?.get(domain)
    }
}

/// A domain record: signed by the TLD's operator key, for `label.tld` under `tld`.
pub fn domain(record: &Value, fqdn: &str, tld: &str, operator_key: &str) -> Result<(), Failure> {
    let Some(m) = record.as_object() else {
        return fail(Layer::Domain, fqdn, "the record is not a JSON object");
    };
    let (d, l, t) = (text(m, "domain"), text(m, "label"), text(m, "tld"));
    if d != fqdn || format!("{l}.{t}") != fqdn || t != tld {
        return fail(Layer::Domain, fqdn, "the record is for another name");
    }
    signed_by(Layer::Domain, fqdn, record, operator_key)
}

/// A manifest: its bytes hash to the address the domain record names, it is for `fqdn`, and it
/// is signed by the owner key the domain record names.
pub fn manifest(
    bytes: &[u8],
    address: &str,
    fqdn: &str,
    owner_key: &str,
) -> Result<Value, Failure> {
    check_address(Layer::Manifest, bytes, address).map_err(|f| Failure {
        subject: fqdn.to_owned(),
        ..f
    })?;
    let v: Value = serde_json::from_slice(bytes).map_or_else(
        |e| fail(Layer::Manifest, fqdn, format!("not JSON ({e})")),
        Ok,
    )?;
    let Some(m) = v.as_object() else {
        return fail(Layer::Manifest, fqdn, "not a JSON object");
    };
    if text(m, "domain") != fqdn {
        return fail(Layer::Manifest, fqdn, "the manifest is for another name");
    }
    if text(m, "owner_public_key") != owner_key {
        return fail(
            Layer::Manifest,
            fqdn,
            "the manifest's signer is not the owner the domain record names",
        );
    }
    signed_by(Layer::Manifest, fqdn, &v, owner_key)?;
    if !m.get("files").is_some_and(Value::is_object) {
        return fail(Layer::Manifest, fqdn, "no file table");
    }
    Ok(v)
}

/// The file a request path names in a manifest, and its block address (protocol.md "Path
/// mapping": `/` is the entry, `/x` is `x` or `x/index.html`, `/x/` is `x/index.html`).
pub fn file_for(manifest: &Value, path: &str) -> Option<(String, String)> {
    let files = manifest.get("files")?.as_object()?;
    let decoded = percent_decode(path)?;
    let trimmed = decoded.trim_start_matches('/');
    let candidates = if trimmed.is_empty() {
        vec![manifest.get("entry")?.as_str()?.to_owned()]
    } else if trimmed.ends_with('/') {
        vec![format!("{trimmed}index.html")]
    } else {
        vec![trimmed.to_owned(), format!("{trimmed}/index.html")]
    };
    candidates.into_iter().find_map(|c| {
        files
            .get(&c)
            .and_then(Value::as_str)
            .map(|h| (c, h.to_owned()))
    })
}

/// A URL path with its `%XX` escapes decoded (the engine sends `a%20b.html` for `a b.html`).
/// `None` for a malformed escape, a NUL, or bytes that are not UTF-8: no file is named that way.
pub fn percent_decode(path: &str) -> Option<String> {
    let b = path.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            let v = u8::from_str_radix(h, 16).ok()?;
            if !h.bytes().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    if out.contains(&0) {
        return None;
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_reads_both_cases_and_nothing_else() {
        assert_eq!(unhex::<2>("0aFf"), Some([0x0a, 0xff]));
        assert_eq!(unhex::<2>("0aF"), None);
        assert_eq!(unhex::<2>("0aFg"), None);
    }

    /// BLAKE3's published vector for the empty input.
    #[test]
    fn the_address_is_blake3() {
        assert_eq!(
            address(b""),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert!(is_address(&address(b"x")));
        assert!(!is_address("../../etc/passwd"));
    }

    #[test]
    fn path_mapping_follows_the_protocol() {
        let m: Value = serde_json::from_str(r#"{"entry":"index.html","files":{"index.html":"a","about":"b","docs/index.html":"c"}}"#).unwrap();
        assert_eq!(file_for(&m, "/"), Some(("index.html".into(), "a".into())));
        assert_eq!(file_for(&m, "/about"), Some(("about".into(), "b".into())));
        assert_eq!(
            file_for(&m, "/docs"),
            Some(("docs/index.html".into(), "c".into()))
        );
        assert_eq!(
            file_for(&m, "/docs/"),
            Some(("docs/index.html".into(), "c".into()))
        );
        assert_eq!(file_for(&m, "/nope"), None);
    }

    #[test]
    fn escaped_paths_are_decoded_and_bad_escapes_name_nothing() {
        let m: Value = serde_json::from_str(
            r#"{"entry":"index.html","files":{"a b.html":"s","café.html":"u","index.html":"i"}}"#,
        )
        .unwrap();
        assert_eq!(
            file_for(&m, "/a%20b.html"),
            Some(("a b.html".into(), "s".into()))
        );
        assert_eq!(
            file_for(&m, "/caf%C3%A9.html"),
            Some(("café.html".into(), "u".into()))
        );
        for bad in ["/index.html%00", "/%FF.html", "/%zz", "/%4", "/%+4.html"] {
            assert_eq!(file_for(&m, bad), None, "{bad}");
        }
        assert_eq!(percent_decode("/x%2Fy").as_deref(), Some("/x/y"));
    }

    /// RFC 8032 test 1 is valid; a small-order key (the identity point) with the identity as R
    /// and S = 0 "verifies" any message under a lax check, and is refused by the strict one.
    #[test]
    fn verification_is_strict() {
        let pk = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
        let sig = concat!(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155",
            "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        );
        assert!(signature_ok(pk, b"", sig));
        let identity = format!("01{}", "00".repeat(31));
        let forged = format!("{identity}{}", "00".repeat(32));
        assert!(!signature_ok(&identity, b"any record at all", &forged));
    }
}
