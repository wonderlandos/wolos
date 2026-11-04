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

const PLANS: [Plan; 7] = [
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
    // The post-quantum witnesses (lead decision 2026-11-04 03:2xZ): one per root, signed by both
    // roots like every intermediate; they sign published files (`woltrust witness sign`), never an
    // app. Kept in the backup only (`woltrust extract --role witness-a`).
    Plan {
        role: "witness-a",
        label: "wolos-witness-a",
        root: false,
        namespaces: &[],
        usage: &[Usage::Witness],
        ci_file: None,
    },
    Plan {
        role: "witness-b",
        label: "wolos-witness-b",
        root: false,
        namespaces: &[],
        usage: &[Usage::Witness],
        ci_file: None,
    },
];

/// Checks a root's subject text (`woltrust init --subject`): 1 to [`crate::cert::MAX_TEXT`]
/// characters, no control characters. It is public: it is signed in both roots and shown as who
/// vouches.
pub fn check_subject(text: &str) -> Result<(), &'static str> {
    if text.trim().is_empty()
        || text.chars().count() > crate::cert::MAX_TEXT
        || text
            .chars()
            .any(|c| c.is_control() || crate::cert::is_invisible(c))
    {
        return Err("the subject text is 1 to 200 characters on one line");
    }
    Ok(())
}

/// Makes the authority at `now`. `subject_a` and `subject_b` are the human-readable texts of root
/// A and root B (signed, public, see [`check_subject`]; `woltrust init --subject-a --subject-b`,
/// both required). `random` fills a buffer with fresh random bytes (the OS
/// generator; tests pass a fixed one). `pass` is the backup passphrase.
pub fn init(
    now: u64,
    subject_a: &str,
    subject_b: &str,
    pass: &str,
    random: &mut dyn FnMut(&mut [u8]),
) -> Result<Output, KeyError> {
    if pass.chars().count() < wolkey::MIN_PASSPHRASE {
        return Err(KeyError::WeakPassphrase);
    }
    check_subject(subject_a).map_err(|_| KeyError::Malformed)?;
    check_subject(subject_b).map_err(|_| KeyError::Malformed)?;
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
    let mut root_b: Option<(KeyMaterial, String)> = None;
    for plan in &PLANS {
        let mut seed = [0u8; 32];
        random(&mut seed);
        let km = KeyMaterial::new(seed, plan.root);
        seed.fill(0);
        // Rule "IDS": the serial is the id of the keys, never a random number.
        let serial = km.secret().id(None);
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
            subject: Subject {
                text: match plan.role {
                    "root-a" => Some(subject_a.to_owned()),
                    "root-b" => Some(subject_b.to_owned()),
                    _ => None,
                },
                ..label(plan.label)
            },
            namespaces: plan.namespaces.iter().map(|s| (*s).to_owned()).collect(),
            usage: plan.usage.to_vec(),
            keys,
            issuer,
            not_before,
            not_after,
            level: Level::Wolos,
            verified_at: None,
            class: None,
            origin_cert: None,
            terms: None,
        };
        // Every intermediate carries both roots' signatures (lead decision 2026-11-04 03:17Z).
        let cert = match (&root_a, &root_b, plan.root) {
            (Some((a, _)), Some((b, bs)), false) => {
                make::issue_cosigned(&body, &a.secret(), &[(bs.as_str(), &b.secret())])
            }
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
        } else if plan.role == "root-b" {
            root_b = Some((km, serial));
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

/// `woltrust init --rotate-intermediates --root SURVIVOR` (the emergency runbook, after
/// `woltrust emergency` withdrew the other root): a new root takes the withdrawn one's place (fresh
/// keys, so a fresh id, the subject text `new_subject`), and the three intermediates are made again
/// with fresh keys, issued by the surviving root and co-signed by the new one (every intermediate
/// carries two root signatures). The survivor's key is unlocked from `backup` with `pass`. Returns
/// the new root and intermediates (`made`; the new root must reach machines in a new image, since
/// roots are pinned), the new CI keys under a new CI passphrase, and a new backup holding the
/// survivor, the new root and the new intermediates. Leaves are then re-issued by the new Authors
/// CA with their own keys (same leaf ids).
pub fn rotate(
    backup: &[u8],
    root: &str,
    new_subject: &str,
    pass: &str,
    now: u64,
    random: &mut dyn FnMut(&mut [u8]),
) -> Result<Output, KeyError> {
    let dead = match root {
        "root-a" => "root-b",
        "root-b" => "root-a",
        _ => return Err(KeyError::Malformed),
    };
    let (root_serial, root_key, root_cert) = extract(backup, root)?;
    let root_km = wolkey::unlock(&root_key, &root_serial, pass)?;
    check_subject(new_subject).map_err(|_| KeyError::Malformed)?;
    let rc = crate::cert::Cert::parse(&root_cert).map_err(|_| KeyError::Malformed)?;
    if !root_km.root || rc.body.serial != root_serial || rc.keys != *root_km.secret().public() {
        return Err(KeyError::Malformed);
    }
    let old: Backup = serde_json::from_slice(backup).map_err(|_| KeyError::Malformed)?;
    let mut ci_raw = [0u8; 32];
    random(&mut ci_raw);
    let mut out = Output {
        made: Vec::new(),
        ci_keys: Vec::new(),
        ci_passphrase: b64::encode(&ci_raw),
        backup: Vec::new(),
    };
    ci_raw.fill(0);
    let mut entries: Vec<BackupEntry> =
        old.entries.into_iter().filter(|e| e.role == root).collect();
    let mut seed = [0u8; 32];
    random(&mut seed);
    let new_km = KeyMaterial::new(seed, true);
    seed.fill(0);
    let new_serial = new_km.secret().id(None);
    let dead_plan = PLANS
        .iter()
        .find(|p| p.role == dead)
        .ok_or(KeyError::Malformed)?;
    let new_body = Body {
        serial: new_serial.clone(),
        kind: Kind::Root,
        subject: Subject {
            text: Some(new_subject.to_owned()),
            ..label(dead_plan.label)
        },
        namespaces: dead_plan
            .namespaces
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        usage: dead_plan.usage.to_vec(),
        keys: make::keys_text(&new_km.secret()),
        issuer: new_serial.clone(),
        not_before: now,
        not_after: now + 20 * YEAR,
        level: Level::Wolos,
        verified_at: None,
        class: None,
        origin_cert: None,
        terms: None,
    };
    let new_cert = make::issue(&new_body, &new_km.secret());
    let mut r28 = [0u8; 28];
    random(&mut r28);
    let new_key = wolkey::lock(&new_km, &new_serial, pass, &r28, wolkey::ROUNDS)?;
    entries.push(BackupEntry {
        role: dead.into(),
        serial: new_serial.clone(),
        wolcert: b64::encode(&new_cert),
        wolkey: b64::encode(&new_key),
    });
    out.made.push(Made {
        role: dead_plan.role,
        serial: new_serial.clone(),
        cert: new_cert,
        backup_key: new_key,
    });
    for plan in PLANS.iter().filter(|p| !p.root) {
        let mut seed = [0u8; 32];
        random(&mut seed);
        let km = KeyMaterial::new(seed, false);
        seed.fill(0);
        let serial = km.secret().id(None);
        let body = Body {
            serial: serial.clone(),
            kind: Kind::Intermediate,
            subject: label(plan.label),
            namespaces: plan.namespaces.iter().map(|s| (*s).to_owned()).collect(),
            usage: plan.usage.to_vec(),
            keys: make::keys_text(&km.secret()),
            issuer: root_serial.clone(),
            not_before: now,
            not_after: now + 3 * YEAR,
            level: Level::Wolos,
            verified_at: None,
            class: None,
            origin_cert: None,
            terms: None,
        };
        let cert = make::issue_cosigned(
            &body,
            &root_km.secret(),
            &[(new_serial.as_str(), &new_km.secret())],
        );
        random(&mut r28);
        let backup_key = wolkey::lock(&km, &serial, pass, &r28, wolkey::ROUNDS)?;
        if let Some(f) = plan.ci_file {
            random(&mut r28);
            let k = wolkey::lock(&km, &serial, &out.ci_passphrase, &r28, wolkey::ROUNDS)?;
            out.ci_keys.push((f.to_owned(), k));
        }
        entries.push(BackupEntry {
            role: plan.role.into(),
            serial: serial.clone(),
            wolcert: b64::encode(&cert),
            wolkey: b64::encode(&backup_key),
        });
        out.made.push(Made {
            role: plan.role,
            serial,
            cert,
            backup_key,
        });
    }
    let b = Backup {
        format: 1,
        kind: BACKUP_KIND.into(),
        created: now,
        entries,
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

    const SUBJECT: &str = "WolOS test authority A";
    const SUBJECT_B: &str = "WolOS test authority B";
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
        let out = init(1_800_000_000, SUBJECT, SUBJECT_B, "lead passphrase", &mut r).unwrap();
        assert_eq!(out.made.len(), 7);
        let roots: Vec<&[u8]> = out.made[..2].iter().map(|m| m.cert.as_slice()).collect();
        let inter: Vec<&[u8]> = out.made[2..].iter().map(|m| m.cert.as_slice()).collect();
        let empty: [(&[u8], &[u8]); 0] = [];
        let s = Store::load(&roots, &inter, &empty, 1_800_000_100, false);
        assert_eq!(
            (s.roots.len(), s.intermediates.len()),
            (2, 5),
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
        for role in [
            "root-a",
            "root-b",
            "authors-ca",
            "packages-ca",
            "index",
            "witness-a",
            "witness-b",
        ] {
            let (serial, key, cert) = extract(&out.backup, role).unwrap();
            assert!(
                wolkey::unlock(&key, &serial, "lead passphrase").is_ok(),
                "{role}"
            );
            assert!(crate::Cert::parse(&cert).is_ok());
        }
        assert_eq!(instructions(&out, "d").len(), 5);

        let mut r = counter();
        assert!(init(1, SUBJECT, SUBJECT_B, "short", &mut r).is_err());
        assert!(
            init(1, "", SUBJECT_B, "lead passphrase", &mut r).is_err(),
            "a subject is required"
        );
        assert!(init(1, SUBJECT, "two\nlines", "lead passphrase", &mut r).is_err());
        // Rule "IDS": every serial is the id of the keys; every intermediate has both roots.
        for m in &out.made {
            let c = crate::Cert::parse(&m.cert).unwrap();
            assert_eq!(
                c.body.serial,
                crate::cert::id_of(&c.keys, None),
                "{}",
                m.role
            );
            assert_eq!(
                c.cosigs.len(),
                usize::from(c.body.kind == Kind::Intermediate)
            );
        }
        for c in &s.intermediates {
            let p = crate::verify::path(c, &[], &s).expect("both root signatures verify");
            assert_eq!(p.len(), 3, "intermediate, root B, root A");
        }
        // Root A alone does not make an intermediate: drop root B's signature.
        let mut lone = s.intermediates[0].clone();
        lone.cosigs.clear();
        assert_eq!(
            crate::verify::path(&lone, &[], &s).err(),
            Some(crate::verify::Code::RootSignatures)
        );
    }

    #[test]
    fn rotation_replaces_the_withdrawn_root_and_re_signs_the_intermediates() {
        let mut r = counter();
        let out = init(1_800_000_000, SUBJECT, SUBJECT_B, "lead passphrase", &mut r).unwrap();
        let rot = rotate(
            &out.backup,
            "root-b",
            "WolOS test authority A2",
            "lead passphrase",
            1_800_100_000,
            &mut r,
        )
        .unwrap();
        assert_eq!(
            rot.made.len(),
            6,
            "new root A, three intermediates, two witnesses"
        );
        assert_eq!(rot.made[0].role, "root-a");
        assert_ne!(
            rot.made[0].serial, out.made[0].serial,
            "fresh keys, fresh id"
        );
        let roots = [rot.made[0].cert.clone(), out.made[1].cert.clone()];
        let inter: Vec<Vec<u8>> = rot.made[1..].iter().map(|m| m.cert.clone()).collect();
        let empty: [(&[u8], &[u8]); 0] = [];
        let s = Store::load(&roots, &inter, &empty, 1_800_100_100, false);
        assert_eq!(s.intermediates.len(), 5);
        for c in &s.intermediates {
            assert!(crate::verify::path(c, &[], &s).is_ok());
        }
        // The old intermediates no longer reach a pinned pair.
        let old: Vec<Vec<u8>> = out.made[2..].iter().map(|m| m.cert.clone()).collect();
        let s2 = Store::load(&roots, &old, &empty, 1_800_100_100, false);
        assert!(
            s2.intermediates
                .iter()
                .all(|c| crate::verify::path(c, &[], &s2).is_err())
        );
        assert!(extract(&rot.backup, "root-a").is_ok() && extract(&rot.backup, "index").is_ok());
        assert!(rotate(&out.backup, "root-c", SUBJECT, "lead passphrase", 1, &mut r).is_err());
        assert!(
            rotate(
                &out.backup,
                "root-a",
                SUBJECT,
                "wrong passphrase",
                1,
                &mut r
            )
            .is_err()
        );
    }
}
