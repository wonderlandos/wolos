// SPDX-License-Identifier: BSD-2-Clause
//! Revocation database vectors (`vectors/rkdb/`): what a machine holding `held` (and a floor) makes
//! of a newly arrived database, delta or pair of checkpoints. Every byte derives from the TEST
//! authority, so another implementation reproduces the files. Blobs are base64url; the roots and
//! intermediates are the TEST root A and B and the derived Index (`crate::vectors_ids`).
use crate::b64;
use crate::filecert;
use crate::pki::{DAY, NOW, Pki};
use crate::revoked::Reason;
use crate::rkdb::{self, Checkpoint};
use crate::store::Store;
use crate::vectors_ids::{Ids, db, entry};
use serde::{Deserialize, Serialize};

/// One vector.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RkdbVector {
    /// Always 1.
    pub format: u32,
    /// File name without `.json`.
    pub name: String,
    /// What is tested.
    pub about: String,
    /// The clock.
    pub now: u64,
    /// Root certificates.
    pub roots: Vec<String>,
    /// Intermediate certificates.
    pub intermediates: Vec<String>,
    /// The database the machine holds, if any.
    pub held: Option<String>,
    /// The floor before.
    pub floor: u64,
    /// A file certificate verified first (its `rkdb_serial` raises the floor), if any.
    pub filecert: Option<String>,
    /// The new full database, if any.
    pub new: Option<String>,
    /// A delta onto `held`, if any (applied, then treated as `new`).
    pub delta: Option<String>,
    /// The CDN checkpoint and the public log's, if any.
    pub checkpoints: Vec<Option<String>>,
    /// The result.
    pub expect: RkdbExpect,
}

/// The expected result.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RkdbExpect {
    /// `accepted` or the error's wire name.
    pub result: String,
    /// The floor after.
    pub floor: u64,
    /// `agree`, `disagree`, `one-source` or `none`.
    pub checkpoints: String,
    /// The signature line of every entry of the accepted database (root section first).
    pub lines: Vec<String>,
    /// The SHA-512/256 of the accepted database (hex), empty when refused.
    pub hash: String,
}

fn blob(s: &str) -> Result<Vec<u8>, String> {
    b64::decode(s, 1 << 26).ok_or_else(|| "blob is not base64url".to_owned())
}

/// Runs a vector.
pub fn run(v: &RkdbVector) -> Result<RkdbExpect, String> {
    let all = |xs: &[String]| xs.iter().map(|s| blob(s)).collect::<Result<Vec<_>, _>>();
    let none: [(&[u8], &[u8]); 0] = [];
    let mut store = Store::load(&all(&v.roots)?, &all(&v.intermediates)?, &none, v.now, true);
    store.floor = v.floor;
    let held = v.held.as_deref().map(blob).transpose()?;
    if let Some(h) = &held {
        store
            .load_rkdb(h, v.now)
            .map_err(|e| format!("held: {}", e.as_str()))?;
    }
    if let Some(fc) = &v.filecert {
        let raw = blob(fc)?;
        if let Ok(c) = filecert::verify(&store, &raw, &c_sha(&raw)?)
            && let Some(s) = c.body().rkdb_serial
        {
            store.raise_floor(s);
        }
    }
    let cps: Vec<Option<Checkpoint>> = v
        .checkpoints
        .iter()
        .map(|c| c.as_deref().map(|t| Checkpoint::parse(t.as_bytes())))
        .map(Option::flatten)
        .collect();
    let (floor, agreement) = rkdb::checkpoints(
        cps.first().copied().flatten(),
        cps.get(1).copied().flatten(),
        store.floor,
    );
    store.floor = floor;
    let agreed = match agreement {
        rkdb::Agreement::Agree(c) => Some(c),
        _ => None,
    };
    let word = match agreement {
        rkdb::Agreement::Agree(_) => "agree",
        rkdb::Agreement::Disagree => "disagree",
        rkdb::Agreement::OneSource => "one-source",
        rkdb::Agreement::None => "none",
    };
    let new = match (&v.new, &v.delta, &held) {
        (Some(n), _, _) => Some(blob(n)?),
        (None, Some(d), Some(h)) => match rkdb::apply(h, &blob(d)?) {
            Ok(b) => Some(b),
            Err(e) => return Ok(refused(e, store.floor, word)),
        },
        _ => None,
    };
    let Some(new) = new else {
        return Ok(RkdbExpect {
            result: "none".into(),
            floor: store.floor,
            checkpoints: word.into(),
            lines: Vec::new(),
            hash: String::new(),
        });
    };
    let checked = store.check_rkdb(&new, v.now);
    let (h, root) = match checked {
        Ok(x) => x,
        Err(e) => return Ok(refused(e, store.floor, word)),
    };
    let prev = store.best_db().map(|d| d.db.clone());
    if let Err(e) = rkdb::accept(&h.db, prev.as_ref(), store.floor)
        .and_then(|()| rkdb::check_checkpoint(&h.db, agreed))
    {
        return Ok(refused(e, store.floor, word));
    }
    let index = store
        .intermediates
        .iter()
        .find(|c| c.body.serial == h.signer)
        .map_or_else(|| "WolOS Index".into(), crate::line::ca_name);
    let mut lines = Vec::new();
    if let Some(r) = &root {
        let by = store
            .roots
            .iter()
            .find(|c| c.body.serial == r.signer)
            .map_or_else(|| r.signer.clone(), crate::line::ca_name);
        for e in r.section.entries.iter() {
            lines.push(crate::line::revocation(
                &rkdb::id_text(&e.id),
                u64::from(e.at),
                e.reason,
                &by,
            ));
        }
    }
    for e in h.db.entries.iter() {
        lines.push(crate::line::revocation(
            &rkdb::id_text(&e.id),
            u64::from(e.at),
            e.reason,
            &index,
        ));
    }
    Ok(RkdbExpect {
        result: "accepted".into(),
        floor: store.floor.max(h.db.serial),
        checkpoints: word.into(),
        lines,
        hash: wolcrypt::hash::hex(&h.db.hash),
    })
}

fn c_sha(raw: &[u8]) -> Result<String, String> {
    let f: serde_json::Value = serde_json::from_slice(raw).map_err(|e| e.to_string())?;
    let body = f["body"].as_str().ok_or("no body")?;
    let b: filecert::Body = serde_json::from_slice(&blob(body)?).map_err(|e| e.to_string())?;
    Ok(b.file_sha256)
}

fn refused(e: rkdb::Error, floor: u64, word: &str) -> RkdbExpect {
    RkdbExpect {
        result: e.as_str().into(),
        floor,
        checkpoints: word.into(),
        lines: Vec::new(),
        hash: String::new(),
    }
}

/// Runs a vector and compares.
pub fn check(v: &RkdbVector) -> Result<(), String> {
    let got = run(v)?;
    if got == v.expect {
        Ok(())
    } else {
        Err(format!("{}: expected {:?}, got {got:?}", v.name, v.expect))
    }
}

fn e(b: &[u8]) -> String {
    b64::encode(b)
}

/// Generates every vector (deterministic).
pub fn generate() -> Result<Vec<RkdbVector>, String> {
    let p = Pki::new();
    let ids = Ids::new(&p);
    let roots = vec![e(&p.root_a.file), e(&p.root_b.file)];
    let inter = vec![e(&ids.index.file)];
    let three = [
        entry(&ids.bob.body.serial, NOW - 3 * DAY, Reason::KeyCompromise),
        entry(&p.alice.body.serial, NOW - 2 * DAY, Reason::Superseded),
        entry(
            &wolcrypt::id::to_hex(&wolcrypt::id::of_file(b"a withdrawn version\n")),
            NOW - DAY,
            Reason::WithdrawnVersion,
        ),
    ];
    let base = db(&ids, 1, NOW - 2 * DAY, &three[..2], None, [0; 32]);
    let base_hash = rkdb::Db::parse(&base).map_err(|e| e.as_str())?.hash;
    let db3 = db(&ids, 2, NOW - 3600, &three, None, base_hash);
    let db3_hash = rkdb::Db::parse(&db3).map_err(|e| e.as_str())?.hash;
    let delta = rkdb::delta(&base, &db3).map_err(|e| e.as_str())?;
    let mut tampered = db3.clone();
    tampered[rkdb::HEADER + 70] ^= 1;
    let shrunk = db(&ids, 3, NOW - 1800, &three[1..], None, db3_hash);
    let older = db(&ids, 1, NOW - 1800, &three, None, [0; 32]);
    let fc_body = filecert::Body {
        format: 1,
        serial: wolcrypt::id::to_hex(&wolcrypt::id::of_file(crate::vectors::CONTENT)),
        file_sha256: wolcrypt::hash::hex(&wolcrypt::hash::sha256(b"the bob.notes file")),
        manifest_sha256: wolcrypt::hash::hex(&wolcrypt::hash::sha256(crate::vectors::CONTENT)),
        id: "bob.notes".into(),
        version: "2.0".into(),
        kind: filecert::FileKind::Package,
        origin: None,
        cert: ids.bob.body.serial.clone(),
        class: Some(crate::cert::Class::Origin),
        verdict: crate::report::Verdict::Pass,
        findings: 0,
        checked_at: NOW - 600,
        rkdb_serial: Some(9),
        terms: Some(crate::vectors_ids::terms()),
    };
    let fc = filecert::make(&fc_body, &ids.index.body.serial, &ids.index.secret());
    let cp = |db: &[u8]| -> Result<String, String> {
        Ok(Checkpoint::of(&rkdb::Db::parse(db).map_err(|e| e.as_str())?).to_json())
    };
    let other_cp = Checkpoint {
        serial: 3,
        hash: [7; 32],
    }
    .to_json();
    let v = |name: &str, about: &str| RkdbVector {
        format: 1,
        name: name.into(),
        about: about.into(),
        now: NOW,
        roots: roots.clone(),
        intermediates: inter.clone(),
        held: None,
        floor: 0,
        filecert: None,
        new: None,
        delta: None,
        checkpoints: Vec::new(),
        expect: RkdbExpect {
            result: String::new(),
            floor: 0,
            checkpoints: String::new(),
            lines: Vec::new(),
            hash: String::new(),
        },
    };
    let mut out = cases(
        Blobs {
            db3: &db3,
            base: &base,
            delta: &delta,
            tampered: &tampered,
            shrunk: &shrunk,
            older: &older,
            fc: &fc,
            cp3: cp(&db3)?,
            other_cp,
        },
        &v,
    );
    for x in &mut out {
        x.expect = run(x)?;
    }
    Ok(out)
}

struct Blobs<'a> {
    db3: &'a [u8],
    base: &'a [u8],
    delta: &'a [u8],
    tampered: &'a [u8],
    shrunk: &'a [u8],
    older: &'a [u8],
    fc: &'a [u8],
    cp3: String,
    other_cp: String,
}

fn cases(b: Blobs, v: &dyn Fn(&str, &str) -> RkdbVector) -> Vec<RkdbVector> {
    let (db3, base, delta, tampered, shrunk, older, fc) =
        (b.db3, b.base, b.delta, b.tampered, b.shrunk, b.older, b.fc);
    let other_cp = b.other_cp.clone();
    vec![
        RkdbVector {
            new: Some(e(db3)),
            checkpoints: vec![Some(b.cp3.clone()), Some(b.cp3.clone())],
            ..v(
                "rkdb-three-entries",
                "a fresh database of three entries (a leaf, a legacy serial, a file certificate), both checkpoints agree",
            )
        },
        RkdbVector {
            held: Some(e(base)),
            delta: Some(e(delta)),
            ..v(
                "rkdb-delta",
                "a delta from serial 1 to 2 carrying one entry and the target's signature: applied, it is the full file byte for byte",
            )
        },
        RkdbVector {
            new: Some(e(tampered)),
            ..v(
                "rkdb-tampered",
                "one bit flipped in an entry: the Index signature fails",
            )
        },
        RkdbVector {
            held: Some(e(db3)),
            new: Some(e(shrunk)),
            ..v(
                "rkdb-shrunk-refused",
                "serial 3 leaves out an entry serial 2 had: refused",
            )
        },
        RkdbVector {
            held: Some(e(db3)),
            new: Some(e(older)),
            ..v(
                "rkdb-older-serial-refused",
                "a validly signed serial 1 shown to a machine holding serial 2: refused",
            )
        },
        RkdbVector {
            filecert: Some(e(fc)),
            new: Some(e(db3)),
            ..v(
                "rkdb-floor-raised-by-file-certificate",
                "a file certificate says the hub held serial 9: the machine's floor rises, and serial 2 is now too old",
            )
        },
        RkdbVector {
            held: Some(e(base)),
            checkpoints: vec![Some(b.cp3.clone()), Some(other_cp)],
            ..v(
                "rkdb-checkpoints-disagree",
                "the CDN says serial 2, the public log serial 3 with another hash: warn and keep the higher floor",
            )
        },
        RkdbVector {
            new: Some(e(db3)),
            checkpoints: vec![
                Some(
                    Checkpoint {
                        serial: 2,
                        hash: [9; 32],
                    }
                    .to_json(),
                ),
                Some(
                    Checkpoint {
                        serial: 2,
                        hash: [9; 32],
                    }
                    .to_json(),
                ),
            ],
            ..v(
                "rkdb-checkpoint-hash-mismatch",
                "both checkpoints name serial 2 with another hash: the database shown is not the published one",
            )
        },
    ]
}
