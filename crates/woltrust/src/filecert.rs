// SPDX-License-Identifier: BSD-2-Clause
//! The file certificate (lead decisions 2026-11-04; "release statement" in the first order): one
//! signed record per published version, what WolOS Hub checked about exactly that file. The Index
//! intermediate signs it with label `wolos/release/1`; the index publishes it next to the entry and
//! the Store verifies it offline beside the author's own signature. A bad version is withdrawn
//! alone, by the file certificate's serial with reason `withdrawn-version`, without touching the
//! author's certificate.
//!
//! File: `{"format":1,"body":"<b64url>","cert":"<signer serial>","sig":{"ed25519":"..","second":".."}}`.
//! Body (the exact signed bytes, at most 4 KiB): `{"format":1,"serial","file_sha256",
//! "manifest_sha256","id","version","kind","origin"?,"cert","class"?,"verdict","findings",
//! "checked_at","rkdb_serial"?,"terms"?}`. Rule "IDS": `serial` is the file id, SHA-512 of the
//! file's `SHA256` member (128 hex), recomputed by [`verify_member`] wherever the member is at
//! hand; 16 hex in the TEST authority's older vectors.
use crate::b64;
use crate::cert::{Class, Kind, Usage, is_serial};
use crate::crypto::{self, Purpose, Sig};
use crate::report::Verdict;
use crate::store::Store;
use serde::{Deserialize, Serialize};

/// Largest file certificate file.
pub const MAX_FILE: usize = 16 * 1024;
/// Largest body.
pub const MAX_BODY: usize = 4096;

/// What kind of file it vouches for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
#[serde(deny_unknown_fields)]
pub enum FileKind {
    /// A `.wol`.
    Package,
    /// A `.wolplug`.
    Plugin,
    /// An extension.
    Extension,
}

/// Where the release came from, as the hub saw it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// `https://github.com/OWNER/NAME`.
    pub repo: String,
    /// The release tag.
    pub tag: String,
    /// The 40-hex commit.
    pub commit: String,
}

/// The signed body.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Body {
    /// Always 1.
    pub format: u32,
    /// 16 hex: the file certificate's own serial (what a `withdrawn-version` entry names).
    pub serial: String,
    /// SHA-256 of the whole file as published (64 lowercase hex).
    pub file_sha256: String,
    /// SHA-256 of the file's `SHA256` member, the hash list the author signed.
    pub manifest_sha256: String,
    /// The app, plugin or extension id.
    pub id: String,
    /// Its version.
    pub version: String,
    /// What it is.
    pub kind: FileKind,
    /// Where it came from, when the release carried a signed origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Source>,
    /// Serial of the author's signing certificate.
    pub cert: String,
    /// That certificate's class when the hub checked it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<Class>,
    /// The hub's automatic check.
    pub verdict: Verdict,
    /// How many findings it had.
    pub findings: u32,
    /// When the hub checked it, Unix seconds.
    pub checked_at: u64,
    /// The serial of the revocation database the hub held when it signed (lead decision "RKDB"):
    /// a machine that verifies this file certificate never accepts an older database again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rkdb_serial: Option<u64>,
    /// The trust terms the release was published under, when the hub records them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms: Option<crate::cert::Terms>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SigText {
    ed25519: String,
    second: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FileText {
    format: u32,
    body: String,
    cert: String,
    sig: SigText,
}

/// Why a file certificate does not count, with the stable string the vectors check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileCertError {
    /// Too large, not JSON, unknown fields, or a field out of range.
    Malformed,
    /// The signer is not an intermediate this store knows.
    SignerUnknown,
    /// The signer may not sign file certificates (needs `sign-index`), is not valid at
    /// `checked_at`, does not reach a pinned root, or was withdrawn.
    WrongSigner,
    /// A signature half does not verify.
    BadSignature,
    /// It vouches for another file (the SHA-256 differs).
    OtherFile,
}

impl FileCertError {
    /// Stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            FileCertError::Malformed => "file-cert-malformed",
            FileCertError::SignerUnknown => "file-cert-signer-unknown",
            FileCertError::WrongSigner => "file-cert-wrong-signer",
            FileCertError::BadSignature => "file-cert-bad-signature",
            FileCertError::OtherFile => "file-cert-other-file",
        }
    }
}

/// A file certificate that verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Checked {
    /// WolOS Hub checked this file: "Checked by WolOS Hub on <checked_at>".
    Checked(Body),
    /// This version was withdrawn (`withdrawn-version`, at the given time): "This version was
    /// withdrawn". The Store does not offer it.
    Withdrawn(Body, u64),
}

impl Checked {
    /// Stable wire name: `checked` or `withdrawn-version`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Checked::Checked(_) => "checked",
            Checked::Withdrawn(..) => "withdrawn-version",
        }
    }

    /// The body.
    pub fn body(&self) -> &Body {
        match self {
            Checked::Checked(b) | Checked::Withdrawn(b, _) => b,
        }
    }
}

fn hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn short(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}

impl Body {
    fn check(&self) -> Result<(), FileCertError> {
        let ok = self.format == crate::FORMAT
            && is_serial(&self.serial)
            && is_serial(&self.cert)
            && self.terms.as_ref().is_none_or(crate::cert::Terms::is_valid)
            && hex64(&self.file_sha256)
            && hex64(&self.manifest_sha256)
            && short(&self.id, 128)
            && short(&self.version, 64)
            && self.origin.as_ref().is_none_or(|o| {
                o.repo.starts_with("https://github.com/")
                    && short(&o.repo, 200)
                    && short(&o.tag, 128)
                    && o.commit.len() == 40
                    && o.commit.bytes().all(|b| b.is_ascii_hexdigit())
            });
        if ok {
            Ok(())
        } else {
            Err(FileCertError::Malformed)
        }
    }
}

/// Signs `body` as the intermediate `serial` with `secret` (the hub's sign job; tests).
pub fn make(body: &Body, serial: &str, secret: &crate::make::Secret) -> Vec<u8> {
    let bytes = serde_json::to_vec(body).unwrap_or_default();
    let s = secret.sign(Purpose::Release, &bytes);
    serde_json::to_vec(&FileText {
        format: crate::FORMAT,
        body: b64::encode(&bytes),
        cert: serial.to_owned(),
        sig: SigText {
            ed25519: b64::encode(&s.ed25519),
            second: b64::encode(&s.second),
        },
    })
    .unwrap_or_default()
}

/// [`verify`], and with the file's `SHA256` member at hand: the member's SHA-256 is the body's
/// `manifest_sha256` and, for a derived serial, the serial is the file id SHA-512(member) (rule
/// "IDS"), else [`FileCertError::OtherFile`].
pub fn verify_member(
    store: &Store,
    bytes: &[u8],
    file_sha256: &str,
    member: &[u8],
) -> Result<Checked, FileCertError> {
    let c = verify(store, bytes, file_sha256)?;
    let b = c.body();
    let id = wolcrypt::id::to_hex(&wolcrypt::id::of_file(member));
    let ok = b.manifest_sha256 == wolcrypt::hash::hex(&wolcrypt::hash::sha256(member))
        && (crate::cert::is_legacy_serial(&b.serial) || b.serial == id);
    if ok {
        Ok(c)
    } else {
        Err(FileCertError::OtherFile)
    }
}

/// Verifies a file certificate for the file whose SHA-256 is `file_sha256` (lowercase hex): the
/// signer is an intermediate of `store` with `sign-index`, valid when it signed (`checked_at`),
/// reaching a pinned root and not withdrawn; both halves verify; it names this file. Then a
/// `withdrawn-version` entry for its serial, in any list or memory held, makes it
/// [`Checked::Withdrawn`]. Pure: no clock is needed.
pub fn verify(store: &Store, bytes: &[u8], file_sha256: &str) -> Result<Checked, FileCertError> {
    if bytes.len() > MAX_FILE {
        return Err(FileCertError::Malformed);
    }
    let f: FileText = serde_json::from_slice(bytes).map_err(|_| FileCertError::Malformed)?;
    let body_bytes = b64::decode(&f.body, MAX_BODY).ok_or(FileCertError::Malformed)?;
    let ed = b64::decode(&f.sig.ed25519, crypto::ED_SIG).ok_or(FileCertError::Malformed)?;
    let second = b64::decode(&f.sig.second, crypto::SLH_SIG).ok_or(FileCertError::Malformed)?;
    let sig = Sig {
        ed25519: ed.try_into().map_err(|_| FileCertError::Malformed)?,
        second,
    };
    if f.format != crate::FORMAT || !is_serial(&f.cert) {
        return Err(FileCertError::Malformed);
    }
    let body: Body = serde_json::from_slice(&body_bytes).map_err(|_| FileCertError::Malformed)?;
    body.check()?;
    let signer = store
        .intermediates
        .iter()
        .find(|c| c.body.serial == f.cert)
        .ok_or(FileCertError::SignerUnknown)?;
    if signer.body.kind != Kind::Intermediate || !signer.body.usage.contains(&Usage::SignIndex) {
        return Err(FileCertError::WrongSigner);
    }
    let path = crate::verify::path(signer, &[], store).map_err(|_| FileCertError::WrongSigner)?;
    if !path.iter().all(|c| c.valid_at(body.checked_at))
        || crate::verify::revocation(&path, store, None).is_some()
    {
        return Err(FileCertError::WrongSigner);
    }
    let msg = crypto::message(Purpose::Release, &body_bytes);
    crypto::verify_hybrid(&signer.keys, &msg, &sig).map_err(|_| FileCertError::BadSignature)?;
    if body.file_sha256 != file_sha256 {
        return Err(FileCertError::OtherFile);
    }
    let withdrawn = crate::verify::version_withdrawn(store, &body.serial);
    Ok(match withdrawn {
        Some(at) => Checked::Withdrawn(body, at),
        None => Checked::Checked(body),
    })
}
