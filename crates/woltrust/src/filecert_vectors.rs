// SPDX-License-Identifier: BSD-2-Clause
//! Test vectors for file certificates (`vectors/filecert/`): each holds the file certificate, the
//! SHA-256 of the file it is checked against, the store (inline base64url: roots, intermediates,
//! lists) and what [`crate::filecert::verify`] must give: `checked`, `withdrawn-version` or a
//! `file-cert-*` error. Keys come from the TEST authority (public seeds).
use crate::b64;
use crate::cert::Class;
use crate::filecert::{self, Body, FileKind, Source};
use crate::make;
use crate::pki::{DAY, NOW, Pki};
use crate::report::Verdict;
use crate::revoked::{Entry, ListText, Reason};
use crate::store::Store;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One file certificate vector.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileCertVector {
    /// Always 1.
    pub format: u32,
    /// File name without `.json`.
    pub name: String,
    /// What is being tested.
    pub about: String,
    /// The file certificate (base64url).
    pub filecert: String,
    /// SHA-256 of the file it is checked against.
    pub file_sha256: String,
    /// Pinned roots (base64url `.wolcert`).
    pub roots: Vec<String>,
    /// Intermediates (base64url `.wolcert`).
    pub intermediates: Vec<String>,
    /// Revocation lists: `[revoked.json, revoked.json.wsig]`, base64url each.
    pub lists: Vec<[String; 2]>,
    /// `checked`, `withdrawn-version` or the error name.
    pub expect: String,
}

fn dec(s: &str) -> Result<Vec<u8>, String> {
    b64::decode(s, 1 << 20).ok_or_else(|| "value is not base64url".to_owned())
}

/// Runs the vector (TEST roots allowed) and returns the result name.
pub fn observe(v: &FileCertVector) -> Result<String, String> {
    let all = |xs: &[String]| xs.iter().map(|s| dec(s)).collect::<Result<Vec<_>, _>>();
    let lists = v
        .lists
        .iter()
        .map(|[l, s]| Ok((dec(l)?, dec(s)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let store = Store::load(&all(&v.roots)?, &all(&v.intermediates)?, &lists, NOW, true);
    Ok(
        match filecert::verify(&store, &dec(&v.filecert)?, &v.file_sha256) {
            Ok(c) => c.as_str().to_owned(),
            Err(e) => e.as_str().to_owned(),
        },
    )
}

/// Replays the vector.
pub fn check(v: &FileCertVector) -> Result<(), String> {
    let got = observe(v)?;
    if got == v.expect {
        Ok(())
    } else {
        Err(format!("{}: expected {}, got {got}", v.name, v.expect))
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The file the vectors vouch for, and its body.
fn body(p: &Pki) -> (String, Body) {
    let manifest = b"0000  alice.notes/plugin.toml\n";
    let file = b"test published file 1\n";
    let b = Body {
        format: 1,
        serial: "a1b2c3d4e5f60718".into(),
        file_sha256: hex(&Sha256::digest(file)),
        manifest_sha256: hex(&Sha256::digest(manifest)),
        id: "alice.notes".into(),
        version: "1.2".into(),
        kind: FileKind::Package,
        origin: Some(Source {
            repo: "https://github.com/alice/notes".into(),
            tag: "v1.2".into(),
            commit: "0123456789abcdef0123456789abcdef01234567".into(),
        }),
        cert: p.alice.body.serial.clone(),
        class: Some(Class::Origin),
        verdict: Verdict::Pass,
        findings: 0,
        checked_at: NOW - DAY,
    };
    (b.file_sha256.clone(), b)
}

/// Builds every file certificate vector. Deterministic.
pub fn generate() -> Result<Vec<FileCertVector>, String> {
    let p = Pki::new();
    let (sha, b) = body(&p);
    let good = filecert::make(&b, &p.index.body.serial, &p.index.secret);
    let list = |entries: Vec<Entry>| {
        let l = ListText {
            format: 1,
            serial: 21,
            issued: NOW - DAY,
            expires: NOW - DAY + crate::LIST_LIFETIME,
            entries,
        };
        let (a, s) = make::revocation_list(&l, &p.index.body.serial, &p.index.secret);
        [b64::encode(&a), b64::encode(&s)]
    };
    let withdraw = |serial: &str, reason| {
        list(vec![Entry {
            serial: serial.to_owned(),
            at: NOW - DAY / 2,
            reason,
        }])
    };
    let mut tampered: serde_json::Value =
        serde_json::from_slice(&good).map_err(|e| e.to_string())?;
    let mut other = b.clone();
    other.verdict = Verdict::Flag;
    tampered["body"] = serde_json::Value::String(b64::encode(
        &serde_json::to_vec(&other).map_err(|e| e.to_string())?,
    ));
    let tampered = serde_json::to_vec(&tampered).map_err(|e| e.to_string())?;
    let by_author = filecert::make(&b, &p.alice.body.serial, &p.alice.secret);
    let roots = vec![b64::encode(&p.root_a.file)];
    let inter = vec![b64::encode(&p.index.file), b64::encode(&p.authors.file)];
    type Case<'a> = (
        &'a str,
        &'a str,
        &'a [u8],
        String,
        Vec<[String; 2]>,
        Vec<String>,
        &'a str,
    );
    let cases: Vec<Case> = vec![
        (
            "filecert-valid",
            "a file certificate by the Index for this exact file: checked by WolOS Hub",
            &good,
            sha.clone(),
            vec![list(vec![])],
            inter.clone(),
            "checked",
        ),
        (
            "filecert-withdrawn-version",
            "the version withdrawn alone (withdrawn-version by its serial); the author's certificate is untouched",
            &good,
            sha.clone(),
            vec![withdraw(&b.serial, Reason::WithdrawnVersion)],
            inter.clone(),
            "withdrawn-version",
        ),
        (
            "filecert-other-reason-ignored",
            "a key-compromise entry with the file certificate's serial is about a certificate, not a version",
            &good,
            sha.clone(),
            vec![withdraw(&b.serial, Reason::KeyCompromise)],
            inter.clone(),
            "checked",
        ),
        (
            "filecert-tampered",
            "the verdict in the body changed after signing",
            &tampered,
            sha.clone(),
            vec![list(vec![])],
            inter.clone(),
            "file-cert-bad-signature",
        ),
        (
            "filecert-other-file",
            "a valid file certificate offered for another file",
            &good,
            "0".repeat(64),
            vec![list(vec![])],
            inter.clone(),
            "file-cert-other-file",
        ),
        (
            "filecert-by-author",
            "signed by the author's own certificate, not the Index",
            &by_author,
            sha.clone(),
            vec![list(vec![])],
            inter.clone(),
            "file-cert-signer-unknown",
        ),
        (
            "filecert-index-unknown",
            "the Index is not in the store",
            &good,
            sha,
            vec![],
            vec![b64::encode(&p.authors.file)],
            "file-cert-signer-unknown",
        ),
    ];
    let mut out = Vec::new();
    for (name, about, fc, file_sha256, lists, intermediates, want) in cases {
        let v = FileCertVector {
            format: 1,
            name: name.into(),
            about: about.into(),
            filecert: b64::encode(fc),
            file_sha256,
            roots: roots.clone(),
            intermediates,
            lists,
            expect: want.into(),
        };
        let got = observe(&v)?;
        if got != want {
            return Err(format!("{name}: wanted {want}, got {got}"));
        }
        out.push(v);
    }
    Ok(out)
}
