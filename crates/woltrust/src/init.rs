// SPDX-License-Identifier: BSD-2-Clause
//! `woltrust init` (ADR 0046, "Keys and setup"): the whole WolOS authority made in one go, on the
//! lead's own machine, with no ceremony. Root A and root B (Ed25519 + SLH-DSA-SHA2-128s, 20 years)
//! and the three intermediates (Authors CA, Packages CA, Index; Ed25519 + ML-DSA-65, 3 years,
//! signed by root A) are generated from fresh random seeds. Out come: the public certificates (for
//! the image), the two intermediate keys the hub pipeline needs, each a `.wolkey` locked with a
//! random CI passphrase, and one backup file with all five keys locked with the passphrase the
//! lead typed. Nothing is written in the clear except the CI passphrase, which is a GitHub secret.
use crate::b64;
use crate::cert::{Body, KeysText, Kind, Level, Subject, Usage};
use crate::key::KeyMaterial;
use crate::make;
use crate::wolkey::{self, KeyError};
use serde::{Deserialize, Serialize};

const YEAR: u64 = 365 * 86_400;

/// One generated identity.
pub struct Made {
    /// `root-a`, `root-b`, `authors-ca`, `packages-ca` or `index`.
    pub role: &'static str,
    /// Its serial.
    pub serial: String,
    /// Its `.wolcert`.
    pub cert: Vec<u8>,
    /// Its key, locked with the backup passphrase.
    pub backup_key: Vec<u8>,
}

/// Everything `init` produces.
pub struct Output {
    /// The five identities, roots first.
    pub made: Vec<Made>,
    /// `(file name, .wolkey)` for the hub pipeline, locked with [`Output::ci_passphrase`].
    pub ci_keys: Vec<(String, Vec<u8>)>,
    /// The random passphrase of the CI keys (a GitHub secret, never typed).
    pub ci_passphrase: String,
    /// The backup file (`.wolbackup`).
    pub backup: Vec<u8>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BackupEntry {
    role: String,
    serial: String,
    wolcert: String,
    wolkey: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Backup {
    format: u32,
    kind: String,
    created: u64,
    entries: Vec<BackupEntry>,
}

const BACKUP_KIND: &str = "wolos/trust-backup/1";
/// Largest `.wolbackup`.
pub const MAX_BACKUP: usize = 256 * 1024;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn label(l: &str) -> Subject {
    Subject {
        label: Some(l.to_owned()),
        ..Subject::default()
    }
}

struct Plan {
    role: &'static str,
    label: &'static str,
    root: bool,
    namespaces: &'static [&'static str],
    usage: &'static [Usage],
    ci_file: Option<&'static str>,
}

const PLANS: [Plan; 5] = [
    Plan {
        role: "root-a",
        label: "wolos-root-a",
        root: true,
        namespaces: &["*", "org.wolos."],
        usage: &[Usage::SignCerts, Usage::SignRevocations],
        ci_file: None,
    },
    Plan {
        role: "root-b",
        label: "wolos-root-b",
        root: true,
        namespaces: &["*", "org.wolos."],
        usage: &[Usage::SignCerts, Usage::SignRevocations],
        ci_file: None,
    },
    Plan {
        role: "authors-ca",
        label: "wolos-authors-ca",
        root: false,
        namespaces: &["*"],
        usage: &[Usage::SignCerts],
        ci_file: Some("authors-ca.wolkey"),
    },
    Plan {
        role: "packages-ca",
        label: "wolos-packages-ca",
        root: false,
        namespaces: &["org.wolos."],
        usage: &[Usage::SignCerts],
        ci_file: None,
    },
    Plan {
        role: "index",
        label: "wolos-index",
        root: false,
        namespaces: &[],
        usage: &[Usage::SignIndex, Usage::SignRevocations],
        ci_file: Some("index.wolkey"),
    },
];

/// Makes the authority at `now`. `random` fills a buffer with fresh random bytes (the OS
/// generator; tests pass a fixed one). `pass` is the backup passphrase.
pub fn init(now: u64, pass: &str, random: &mut dyn FnMut(&mut [u8])) -> Result<Output, KeyError> {
    if pass.chars().count() < wolkey::MIN_PASSPHRASE {
        return Err(KeyError::WeakPassphrase);
    }
    let mut ci_raw = [0u8; 32];
    random(&mut ci_raw);
    let ci_passphrase = b64::encode(&ci_raw);
    ci_raw.fill(0);
    let mut out = Output {
        made: Vec::new(),
        ci_keys: Vec::new(),
        ci_passphrase,
        backup: Vec::new(),
    };
    let mut root_a: Option<(KeyMaterial, String)> = None;
    for plan in &PLANS {
        let mut seed = [0u8; 32];
        random(&mut seed);
        let km = KeyMaterial::new(seed, plan.root);
        seed.fill(0);
        let mut ser = [0u8; 8];
        random(&mut ser);
        let serial = hex(&ser);
        let keys: KeysText = make::keys_text(&km.secret());
        let (issuer, not_before, not_after) = match (&root_a, plan.root) {
            (Some((_, a)), false) => (a.clone(), now, now + 3 * YEAR),
            _ => (serial.clone(), now, now + 20 * YEAR),
        };
        let body = Body {
            serial: serial.clone(),
            kind: if plan.root {
                Kind::Root
            } else {
                Kind::Intermediate
            },
            subject: label(plan.label),
            namespaces: plan.namespaces.iter().map(|s| (*s).to_owned()).collect(),
            usage: plan.usage.to_vec(),
            keys,
            issuer,
            not_before,
            not_after,
            level: Level::Wolos,
            verified_at: None,
        };
        let cert = match (&root_a, plan.root) {
            (Some((a, _)), false) => make::issue(&body, &a.secret()),
            _ => make::issue(&body, &km.secret()),
        };
        let mut r28 = [0u8; 28];
        random(&mut r28);
        let backup_key = wolkey::lock(&km, &serial, pass, &r28, wolkey::ROUNDS)?;
        if let Some(f) = plan.ci_file {
            random(&mut r28);
            let k = wolkey::lock(&km, &serial, &out.ci_passphrase, &r28, wolkey::ROUNDS)?;
            out.ci_keys.push((f.to_owned(), k));
        }
        out.made.push(Made {
            role: plan.role,
            serial: serial.clone(),
            cert,
            backup_key,
        });
        if plan.role == "root-a" {
            root_a = Some((km, serial));
        }
    }
    let b = Backup {
        format: 1,
        kind: BACKUP_KIND.into(),
        created: now,
        entries: out
            .made
            .iter()
            .map(|m| BackupEntry {
                role: m.role.into(),
                serial: m.serial.clone(),
                wolcert: b64::encode(&m.cert),
                wolkey: b64::encode(&m.backup_key),
            })
            .collect(),
    };
    out.backup = serde_json::to_vec_pretty(&b).map_err(|_| KeyError::Malformed)?;
    Ok(out)
}

/// `(serial, .wolkey, .wolcert)` of `role` in a backup file (the key stays locked).
pub fn extract(backup: &[u8], role: &str) -> Result<(String, Vec<u8>, Vec<u8>), KeyError> {
    if backup.len() > MAX_BACKUP {
        return Err(KeyError::Malformed);
    }
    let b: Backup = serde_json::from_slice(backup).map_err(|_| KeyError::Malformed)?;
    if b.format != 1 || b.kind != BACKUP_KIND {
        return Err(KeyError::Malformed);
    }
    let e = b
        .entries
        .into_iter()
        .find(|e| e.role == role)
        .ok_or(KeyError::Malformed)?;
    let key = b64::decode(&e.wolkey, wolkey::MAX_FILE).ok_or(KeyError::Malformed)?;
    let cert = b64::decode(&e.wolcert, crate::MAX_CERT_FILE).ok_or(KeyError::Malformed)?;
    Ok((e.serial, key, cert))
}

/// The five lines `woltrust init` prints: what goes where. `dir` is the output folder.
pub fn instructions(out: &Output, dir: &str) -> [String; 5] {
    let repo = "--repo wonderlandos/hub-index";
    [
        format!(
            "1. gh secret set WOLOS_AUTHORS_CA_KEY --env ca {repo} < {dir}/ci/authors-ca.wolkey"
        ),
        format!(
            "2. gh secret set WOLOS_INDEX_SIGNING_KEY --env sign {repo} < {dir}/ci/index.wolkey"
        ),
        format!(
            "3. for e in ca sign; do gh secret set WOLOS_KEY_PASSPHRASE --env $e {repo} < {dir}/ci/passphrase; done"
        ),
        format!(
            "4. Copy {dir}/wolos-trust.wolbackup to your phone; it holds all {} keys and opens only with the passphrase you typed.",
            out.made.len()
        ),
        format!(
            "5. Give {dir}/public/ to the WolOS repository (files/usr/share/wolos/trust/), then delete {dir}."
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn counter() -> impl FnMut(&mut [u8]) {
        let mut n = 0u8;
        move |buf: &mut [u8]| {
            for b in buf.iter_mut() {
                n = n.wrapping_add(37);
                *b = n;
            }
        }
    }

    #[test]
    fn init_makes_two_roots_three_intermediates_and_locked_keys() {
        let mut r = counter();
        let out = init(1_800_000_000, "lead passphrase", &mut r).unwrap();
        assert_eq!(out.made.len(), 5);
        let roots: Vec<&[u8]> = out.made[..2].iter().map(|m| m.cert.as_slice()).collect();
        let inter: Vec<&[u8]> = out.made[2..].iter().map(|m| m.cert.as_slice()).collect();
        let empty: [(&[u8], &[u8]); 0] = [];
        let s = Store::load(&roots, &inter, &empty, 1_800_000_100, false);
        assert_eq!(
            (s.roots.len(), s.intermediates.len()),
            (2, 3),
            "{:?}",
            s.notes
        );
        assert_eq!(out.ci_keys.len(), 2);
        for (name, k) in &out.ci_keys {
            let serial = wolkey::name_of(k).unwrap();
            assert!(
                wolkey::unlock(k, &serial, &out.ci_passphrase).is_ok(),
                "{name}"
            );
            assert!(wolkey::unlock(k, &serial, "lead passphrase").is_err());
        }
        let text = String::from_utf8(out.backup.clone()).unwrap();
        assert!(!text.contains("\"seed\""));
        for role in ["root-a", "root-b", "authors-ca", "packages-ca", "index"] {
            let (serial, key, cert) = extract(&out.backup, role).unwrap();
            assert!(
                wolkey::unlock(&key, &serial, "lead passphrase").is_ok(),
                "{role}"
            );
            assert!(crate::Cert::parse(&cert).is_ok());
        }
        assert_eq!(instructions(&out, "d").len(), 5);
        let mut r = counter();
        assert!(init(1, "short", &mut r).is_err());
    }
}
