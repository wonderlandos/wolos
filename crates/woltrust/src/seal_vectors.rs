// SPDX-License-Identifier: BSD-2-Clause
//! Test vectors for `wolos/seal/1` (`vectors/seal/`): `keys.json` holds two TEST recipients whose
//! certificates carry encryption keys (a person and an organisation, issued by the TEST Authors
//! CA) and their public seeds; each vector names the recipient, the label, the plaintext and the 64
//! random bytes it was sealed with, the sealed bytes, who opens it and what must come out. Another
//! implementation reproduces the sealed bytes from `rnd` and must reach the same `expect`.
use crate::b64;
use crate::cert::{Cert, Kind, Level, Subject, Usage};
use crate::make::{self, Secret};
use crate::pki::{self, DAY, NOW, Pki, Spec};
use crate::seal::{self, EncSecret, FILE_LABEL, OpenError};
use crate::vectors::PkiFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use std::collections::BTreeMap;

/// What opening must give.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SealExpect {
    /// The plaintext (base64url) when it opens, else null.
    pub plaintext: Option<String>,
    /// The `OpenError` wire name when it does not open, else null.
    pub error: Option<String>,
}

/// One sealed-message vector.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SealVector {
    /// Always 1.
    pub format: u32,
    /// File name without `.json`.
    pub name: String,
    /// What is being tested.
    pub about: String,
    /// Blob name of the recipient certificate the message was sealed to.
    pub recipient: String,
    /// Label it was sealed under.
    pub label: String,
    /// The plaintext that was sealed (base64url).
    pub plaintext: String,
    /// The 64 random bytes it was sealed with (base64url).
    pub rnd: String,
    /// True when `sealed` is exactly `seal(recipient, label, plaintext, rnd)`; false for a
    /// tampered copy.
    pub reproducible: bool,
    /// The sealed bytes (base64url).
    pub sealed: String,
    /// Blob name of the opener's seed.
    pub opener_seed: String,
    /// Serial the opener holds the keys for.
    pub opener_serial: String,
    /// Label the opener expects.
    pub open_label: String,
    /// The result.
    pub expect: SealExpect,
}

fn blob(keys: &PkiFile, name: &str) -> Result<Vec<u8>, String> {
    let t = keys
        .blobs
        .get(name)
        .ok_or_else(|| format!("no blob {name}"))?;
    b64::decode(t, 1 << 20).ok_or_else(|| format!("blob {name} damaged"))
}

fn seed32(b: &[u8]) -> Result<[u8; 32], String> {
    b.try_into().map_err(|_| "seed is not 32 bytes".to_owned())
}

fn dec(s: &str) -> Result<Vec<u8>, String> {
    b64::decode(s, 1 << 20).ok_or_else(|| "value is not base64url".to_owned())
}

/// Opens the vector as recorded and returns what came out.
pub fn observe(v: &SealVector, keys: &PkiFile) -> Result<SealExpect, String> {
    let opener = EncSecret::from_seed(&seed32(&blob(keys, &v.opener_seed)?)?);
    Ok(
        match seal::open(&opener, &v.opener_serial, &v.open_label, &dec(&v.sealed)?) {
            Ok(p) => SealExpect {
                plaintext: Some(b64::encode(&p)),
                error: None,
            },
            Err(e) => SealExpect {
                plaintext: None,
                error: Some(e.as_str().to_owned()),
            },
        },
    )
}

/// Replays the vector: the recipient certificate parses with its encryption keys, a reproducible
/// vector seals to the same bytes, and opening gives `expect`.
pub fn check(v: &SealVector, keys: &PkiFile) -> Result<(), String> {
    let cert = Cert::parse(&blob(keys, &v.recipient)?).map_err(|e| format!("{}: {e}", v.name))?;
    if v.reproducible {
        let rnd: [u8; 64] = dec(&v.rnd)?
            .try_into()
            .map_err(|_| "rnd is not 64 bytes".to_owned())?;
        let again = seal::seal(&cert, &v.label, &dec(&v.plaintext)?, &rnd)
            .map_err(|e| format!("{}: {e:?}", v.name))?;
        if b64::encode(&again) != v.sealed {
            return Err(format!("{}: sealing again gives other bytes", v.name));
        }
    }
    let got = observe(v, keys)?;
    if got == v.expect {
        Ok(())
    } else {
        Err(format!("{}: expected {:?}, got {got:?}", v.name, v.expect))
    }
}

/// A TEST recipient: its certificate (with encryption keys) and seed.
pub struct Recipient {
    /// Public seed (TEST only).
    pub seed: [u8; 32],
    /// Its `.wolcert`.
    pub file: Vec<u8>,
    /// Its serial.
    pub serial: String,
}

/// A person or organisation like the TEST `alice` and `examplelabs`, renewed with encryption keys
/// (`test-alice-2`, `test-examplelabs-2`), issued by the TEST Authors CA.
pub fn recipient(p: &Pki, org: bool) -> Recipient {
    let (name, login, full, id, ns) = if org {
        (
            "test-examplelabs-2",
            "examplelabs",
            "Example Labs",
            2002,
            "examplelabs.",
        )
    } else {
        ("test-alice-2", "alice", "Alice Doe", 1001, "alice.")
    };
    let seed = pki::seed(name);
    let secret = Secret::from_seed(&seed, false);
    let spec = Spec {
        name,
        kind: if org { Kind::Org } else { Kind::Person },
        subject: Subject {
            github_id: Some(id),
            login: Some(login.into()),
            name: Some(full.into()),
            label: None,
            org: None,
        },
        namespaces: &[ns],
        usage: &[Usage::SignPackage, Usage::SignPlugin, Usage::SignExtension],
        issuer: p.authors.body.serial.clone(),
        not_before: NOW - 10 * DAY,
        not_after: NOW + 355 * DAY,
        level: Level::Verified,
    };
    let mut body = pki::body(&spec, &secret);
    body.keys = crate::key::KeyMaterial::new(seed, false).keys_text();
    let file = make::issue(&body, &p.authors.secret);
    Recipient {
        seed,
        file,
        serial: body.serial,
    }
}

fn rnd(name: &str) -> [u8; 64] {
    Sha512::digest(format!("woltrust/seal-rnd/{name}")).into()
}

struct Case<'a> {
    name: &'a str,
    about: &'a str,
    to_org: bool,
    plaintext: &'a [u8],
    tamper: Option<usize>,
    relabel: Option<&'a str>,
    opener_org: bool,
    opener_serial_org: bool,
    open_label: &'a str,
    want: Result<(), OpenError>,
}

const BASE: Case<'static> = Case {
    name: "",
    about: "",
    to_org: false,
    plaintext: b"test sealed message 1\n",
    tamper: None,
    relabel: None,
    opener_org: false,
    opener_serial_org: false,
    open_label: FILE_LABEL,
    want: Ok(()),
};

fn cases(big: &[u8]) -> Vec<Case<'_>> {
    let header_end = seal::overhead(FILE_LABEL) - seal::TAG;
    vec![
        Case {
            name: "sealed-to-person",
            about: "a file sealed to a person's certificate opens with that person's keys",
            ..BASE
        },
        Case {
            name: "sealed-to-org",
            about: "a file sealed to an organisation's certificate opens with its keys",
            to_org: true,
            opener_org: true,
            opener_serial_org: true,
            ..BASE
        },
        Case {
            name: "sealed-empty",
            about: "an empty file seals and opens",
            plaintext: b"",
            ..BASE
        },
        Case {
            name: "sealed-4k",
            about: "4 KiB of bytes seal and open",
            plaintext: big,
            ..BASE
        },
        Case {
            name: "wrong-recipient",
            about: "sealed to the person, opened by the organisation: refused before any key is used",
            opener_org: true,
            opener_serial_org: true,
            want: Err(OpenError::WrongRecipient),
            ..BASE
        },
        Case {
            name: "wrong-keys",
            about: "the organisation's keys presented under the person's serial: the open fails",
            opener_org: true,
            want: Err(OpenError::Damaged),
            ..BASE
        },
        Case {
            name: "wrong-label",
            about: "opened expecting another label",
            open_label: "wolos/other/1",
            want: Err(OpenError::WrongLabel),
            ..BASE
        },
        Case {
            name: "relabelled",
            about: "the label in the header changed to the one the opener expects: the open fails",
            relabel: Some("wolos/fils/1"),
            open_label: "wolos/fils/1",
            want: Err(OpenError::Damaged),
            ..BASE
        },
        Case {
            name: "tampered-x25519",
            about: "one bit flipped in the X25519 ephemeral key",
            tamper: Some(header_end - 1088 - 5),
            want: Err(OpenError::Damaged),
            ..BASE
        },
        Case {
            name: "tampered-ml-kem",
            about: "one bit flipped in the ML-KEM-768 ciphertext",
            tamper: Some(header_end - 300),
            want: Err(OpenError::Damaged),
            ..BASE
        },
        Case {
            name: "tampered-ciphertext",
            about: "one bit flipped in the encrypted bytes",
            tamper: Some(header_end + 3),
            want: Err(OpenError::Damaged),
            ..BASE
        },
        Case {
            name: "tampered-tag",
            about: "one bit flipped in the Poly1305 tag",
            tamper: Some(usize::MAX),
            want: Err(OpenError::Damaged),
            ..BASE
        },
        Case {
            name: "tampered-recipient",
            about: "the recipient serial in the header changed to the opener's",
            tamper: Some(usize::MAX - 1),
            opener_org: true,
            opener_serial_org: true,
            want: Err(OpenError::Damaged),
            ..BASE
        },
    ]
}

/// Builds `keys.json` and every seal vector. Deterministic.
pub fn generate() -> Result<(PkiFile, Vec<SealVector>), String> {
    let p = Pki::new();
    let alice = recipient(&p, false);
    let org = recipient(&p, true);
    let mut blobs = BTreeMap::new();
    for (n, b) in [
        ("alice-2", &alice.file),
        ("examplelabs-2", &org.file),
        ("authors", &p.authors.file),
        ("root-a", &p.root_a.file),
    ] {
        blobs.insert(n.to_owned(), b64::encode(b));
    }
    blobs.insert("alice-2-seed".into(), b64::encode(&alice.seed));
    blobs.insert("examplelabs-2-seed".into(), b64::encode(&org.seed));
    let keys = PkiFile {
        format: 1,
        name: "test-seal".into(),
        blobs,
    };
    let big: Vec<u8> = (0..4096u32)
        .map(|i| u8::try_from(i % 251).unwrap_or(0))
        .collect();
    let cases = cases(&big);
    let mut out = Vec::new();
    for c in &cases {
        let to = if c.to_org { &org } else { &alice };
        let cert = Cert::parse(&to.file).map_err(|e| e.to_owned())?;
        let r = rnd(c.name);
        let mut sealed =
            seal::seal(&cert, FILE_LABEL, c.plaintext, &r).map_err(|e| format!("{e:?}"))?;
        if let Some(l) = c.relabel {
            let at = seal::FORMAT.len() + 2;
            sealed.splice(at..at + l.len(), l.bytes());
        }
        match c.tamper {
            Some(usize::MAX) => {
                if let Some(b) = sealed.last_mut() {
                    *b ^= 1;
                }
            }
            Some(x) if x == usize::MAX - 1 => {
                let at = seal::FORMAT.len() + 2 + FILE_LABEL.len();
                sealed.splice(at..at + 16, org.serial.bytes());
            }
            Some(at) => sealed[at] ^= 1,
            None => {}
        }
        let v = SealVector {
            format: 1,
            name: c.name.into(),
            about: c.about.into(),
            recipient: if c.to_org { "examplelabs-2" } else { "alice-2" }.into(),
            label: FILE_LABEL.into(),
            plaintext: b64::encode(c.plaintext),
            rnd: b64::encode(&r),
            reproducible: c.tamper.is_none() && c.relabel.is_none(),
            sealed: b64::encode(&sealed),
            opener_seed: if c.opener_org {
                "examplelabs-2-seed"
            } else {
                "alice-2-seed"
            }
            .into(),
            opener_serial: if c.opener_serial_org {
                org.serial.clone()
            } else {
                alice.serial.clone()
            },
            open_label: c.open_label.into(),
            expect: SealExpect {
                plaintext: None,
                error: None,
            },
        };
        let got = observe(&v, &keys)?;
        let want = match c.want {
            Ok(()) => SealExpect {
                plaintext: Some(b64::encode(c.plaintext)),
                error: None,
            },
            Err(e) => SealExpect {
                plaintext: None,
                error: Some(e.as_str().into()),
            },
        };
        if got != want {
            return Err(format!("{}: wanted {want:?}, got {got:?}", c.name));
        }
        out.push(SealVector { expect: got, ..v });
    }
    Ok((keys, out))
}
