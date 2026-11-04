// SPDX-License-Identifier: BSD-2-Clause
//! Vectors for the lead decisions of 2026-11-04 night: derived ids (rule "IDS"), both roots on
//! every intermediate, emergency root-signed withdrawals, the witness, terms and signature lines.
//! The TEST authority's derived half lives here: an Authors CA, an Index and a witness with
//! derived ids, each signed by TEST root A and co-signed by TEST root B, and a person `bob` with
//! encryption keys and terms. The older vectors keep their 16-hex serials ("legacy-length",
//! accepted only under a TEST root).
use crate::cert::{Body, Kind, Level, Subject, Terms, Usage};
use crate::crypto::Purpose;
use crate::key::KeyMaterial;
use crate::make::{self, Secret};
use crate::pki::{self, DAY, NOW, Pki};
use crate::report::Trust;
use crate::revoked::Reason;
use crate::rkdb;
use crate::vectors::{CONTENT, Gen, Vector, inline};
use crate::verify::Code;

/// One derived identity.
pub struct Derived {
    /// Its seed name.
    pub name: &'static str,
    /// Its key material.
    pub km: KeyMaterial,
    /// Its body.
    pub body: Body,
    /// Its `.wolcert` (both roots' signatures for an intermediate).
    pub file: Vec<u8>,
}

impl Derived {
    /// The signing keys.
    pub fn secret(&self) -> Secret {
        self.km.secret()
    }
}

/// The derived half of the TEST authority.
pub struct Ids {
    /// Authors CA, derived id, by root A and root B.
    pub authors: Derived,
    /// Index, derived id, by both roots.
    pub index: Derived,
    /// Witness A, derived id, by both roots.
    pub witness: Derived,
    /// A person with encryption keys and terms, issued by [`Ids::authors`].
    pub bob: Derived,
}

/// The test terms text and its record.
pub fn terms() -> Terms {
    Terms {
        version: 1,
        sha256: wolcrypt::hash::hex(&wolcrypt::hash::sha256(b"WolOS trust terms v1 (TEST)\n")),
    }
}

fn label(l: &str) -> Subject {
    Subject {
        label: Some(l.to_owned()),
        ..Subject::default()
    }
}

/// An intermediate body with a derived id under TEST root A.
fn inter_body(p: &Pki, name: &'static str, ns: &[&str], usage: &[Usage]) -> (KeyMaterial, Body) {
    let km = KeyMaterial::new(pki::seed(name), false);
    let s = km.secret();
    let body = Body {
        serial: s.id(None),
        kind: Kind::Intermediate,
        subject: label(name),
        namespaces: ns.iter().map(|n| (*n).to_owned()).collect(),
        usage: usage.to_vec(),
        keys: make::keys_text(&s),
        issuer: p.root_a.body.serial.clone(),
        not_before: NOW - 30 * DAY,
        not_after: NOW + 3 * 365 * DAY,
        level: Level::Wolos,
        verified_at: None,
        class: None,
        origin_cert: None,
        terms: None,
    };
    (km, body)
}

fn cosigned(p: &Pki, name: &'static str, ns: &[&str], usage: &[Usage]) -> Derived {
    let (km, body) = inter_body(p, name, ns, usage);
    let file = make::issue_cosigned(
        &body,
        &p.root_a.secret,
        &[(p.root_b.body.serial.as_str(), &p.root_b.secret)],
    );
    Derived {
        name,
        km,
        body,
        file,
    }
}

impl Ids {
    /// Builds them (deterministic).
    pub fn new(p: &Pki) -> Ids {
        let authors = cosigned(p, "test-authors-2", &["*"], &[Usage::SignCerts]);
        let index = cosigned(
            p,
            "test-index-2",
            &[],
            &[Usage::SignIndex, Usage::SignRevocations],
        );
        let witness = cosigned(p, "test-witness-a", &[], &[Usage::Witness]);
        let km = KeyMaterial::new(pki::seed("test-bob"), false);
        let body = Body {
            serial: km.key_id(),
            kind: Kind::Person,
            subject: Subject {
                github_id: Some(3003),
                login: Some("bob".into()),
                name: Some("Bob Lima".into()),
                ..Subject::default()
            },
            namespaces: vec!["bob.".into()],
            usage: vec![Usage::SignPackage, Usage::SignPlugin, Usage::SignExtension],
            keys: km.keys_text(),
            issuer: authors.body.serial.clone(),
            not_before: NOW - 30 * DAY,
            not_after: NOW + 335 * DAY,
            level: Level::Verified,
            verified_at: Some(NOW - 31 * DAY),
            class: None,
            origin_cert: None,
            terms: Some(terms()),
        };
        let file = make::issue(&body, &authors.secret());
        let bob = Derived {
            name: "test-bob",
            km,
            body,
            file,
        };
        Ids {
            authors,
            index,
            witness,
            bob,
        }
    }
}

/// Bob's origin record.
pub fn bob_origin() -> Vec<u8> {
    crate::origin::Origin {
        repo: "https://github.com/bob/notes".into(),
        tag: "v2.0".into(),
        commit: "89abcdef0123456789abcdef0123456789abcdef".into(),
        built: NOW - DAY,
        rkdb_serial: Some(7),
    }
    .to_bytes()
}

fn chain(files: &[&[u8]]) -> Vec<u8> {
    pki::chain_of(files)
}

/// A store with roots A and B and the derived Index (its fresh list), for bob's package.
fn bob_vector(v: &mut Vector) {
    v.app_id = Some("bob.notes".into());
    v.wsig = Some("@wsig-bob".into());
    v.chain = Some("@chain-bob".into());
    v.store.roots = vec!["@root-a".into(), "@root-b".into()];
}

/// A database of `entries` by the derived Index, optionally carrying a root section.
pub fn db(
    ids: &Ids,
    serial: u64,
    issued: u64,
    entries: &[rkdb::Entry],
    root: Option<&[u8]>,
    prev: [u8; 32],
) -> Vec<u8> {
    let set = rkdb::Entries::from_list(entries);
    rkdb::make(
        &rkdb::Make {
            serial,
            issued: u32::try_from(issued).unwrap_or(0),
            expires: u32::try_from(issued + rkdb::LIFETIME).unwrap_or(0),
            prev_hash: prev,
            entries: &set,
            root,
        },
        &ids.index.body.serial,
        &ids.index.secret(),
    )
    .unwrap_or_default()
}

/// An entry for `serial`.
pub fn entry(serial: &str, at: u64, reason: Reason) -> rkdb::Entry {
    rkdb::Entry {
        id: rkdb::id_bytes(serial).unwrap_or([0; 64]),
        at: u32::try_from(at).unwrap_or(0),
        reason,
    }
}

/// The cases.
pub(crate) fn cases(g: &mut Gen) -> Result<(), String> {
    use Code as C;
    use Trust as T;
    let ids = Ids::new(&g.p);
    let origin = bob_origin();
    let wsig_bob = make::wsig_with_origin(
        &ids.bob.body.serial,
        &ids.bob.secret(),
        Purpose::Package,
        CONTENT,
        &origin,
    );
    g.put("authors-2", &ids.authors.file);
    g.put("index-2", &ids.index.file);
    g.put("witness-a", &ids.witness.file);
    g.put("bob", &ids.bob.file);
    g.put("chain-bob", &chain(&[&ids.bob.file, &ids.authors.file]));
    g.put("wsig-bob", &wsig_bob);
    g.add(
        "valid-derived-id",
        "rule IDS: every serial on the path but the TEST roots is SHA-512 of the keys; the \
         Authors CA carries both roots' signatures; bob's body names the terms",
        (T::Origin, C::Ok),
        |_, v| bob_vector(v),
    )?;
    // Both roots on every intermediate.
    let (_, ab) = inter_body(&g.p, "test-authors-2", &["*"], &[Usage::SignCerts]);
    let lone = make::issue(&ab, &g.p.root_a.secret);
    let lone_chain = chain(&[&ids.bob.file, &lone]);
    g.add(
        "cosign-missing",
        "an intermediate with a derived id signed by root A alone is refused",
        (T::Invalid, C::RootSignatures),
        |_, v| {
            bob_vector(v);
            v.chain = Some(inline(&lone_chain));
        },
    )?;
    let bad = {
        let mut c = crate::Cert::parse(&ids.authors.file).map_err(|e| e.to_owned())?;
        c.cosigs[0].sig.second[11] ^= 1;
        c.to_bytes()
    };
    let bad_chain = chain(&[&ids.bob.file, &bad]);
    g.add(
        "cosign-bad",
        "root B's signature on the Authors CA has one bit flipped: refused",
        (T::Invalid, C::RootSignatures),
        |_, v| {
            bob_vector(v);
            v.chain = Some(inline(&bad_chain));
        },
    )?;
    g.add(
        "cosign-root-b-not-pinned",
        "only root A is pinned: the second signature cannot be checked, refused",
        (T::Invalid, C::RootSignatures),
        |_, v| {
            bob_vector(v);
            v.store.roots = vec!["@root-a".into()];
        },
    )?;
    // Rule IDS.
    let mut wrong = ids.bob.body.clone();
    wrong.serial = wolcrypt::id::to_hex(&[0x5a; 64]);
    let wrong_file = make::issue(&wrong, &ids.authors.secret());
    let wrong_chain = chain(&[&wrong_file, &ids.authors.file]);
    let wrong_wsig = make::wsig(&wrong.serial, &ids.bob.secret(), Purpose::Package, CONTENT);
    g.add(
        "serial-mismatch",
        "a certificate whose 128-hex serial is not SHA-512 of its keys never parses",
        (T::Invalid, C::ChainMalformed),
        |_, v| {
            bob_vector(v);
            v.chain = Some(inline(&wrong_chain));
            v.wsig = Some(inline(&wrong_wsig));
        },
    )?;
    let (legacy_root, legacy_chain, legacy_wsig) = legacy_under_production_root();
    g.add(
        "legacy-serial-refused",
        "a 16-hex serial under a root that is not the TEST authority's is refused",
        (T::Invalid, C::LegacySerial),
        |_, v| {
            v.app_id = Some("carol.notes".into());
            v.store.roots = vec![inline(&legacy_root)];
            v.chain = Some(inline(&legacy_chain));
            v.wsig = Some(inline(&legacy_wsig));
        },
    )?;
    cases_emergency(g, &ids)?;
    cases_witness(g, &ids)
}

/// Root-signed (emergency) withdrawals and the Index's database.
fn cases_emergency(g: &mut Gen, ids: &Ids) -> Result<(), String> {
    use Code as C;
    use Trust as T;
    let by_b = |entries: &[rkdb::Entry]| {
        rkdb::RootSection::make(
            u32::try_from(NOW - DAY).unwrap_or(0),
            entries,
            &g.p.root_b.body.serial,
            &g.p.root_b.secret,
        )
        .unwrap_or_default()
    };
    let root_a_gone = by_b(&[entry(
        &g.p.root_a.body.serial,
        NOW - DAY,
        Reason::KeyCompromise,
    )]);
    let authors_gone = by_b(&[entry(
        &ids.authors.body.serial,
        NOW - DAY,
        Reason::KeyCompromise,
    )]);
    let forged = rkdb::RootSection::make(
        u32::try_from(NOW - DAY).unwrap_or(0),
        &[entry(
            &ids.authors.body.serial,
            NOW - DAY,
            Reason::KeyCompromise,
        )],
        &ids.index.body.serial,
        &ids.index.secret(),
    )
    .unwrap_or_default();
    g.put("emergency-root-a", &root_a_gone);
    g.put("emergency-authors-2", &authors_gone);
    g.add(
        "emergency-root-a-by-b",
        "root B withdraws root A: every intermediate carries root A's signature, so bob's chain is withdrawn",
        (T::Withdrawn, C::RevokedKeyCompromise),
        |_, v| {
            bob_vector(v);
            v.store.root_sections = vec!["@emergency-root-a".into()];
        },
    )?;
    g.add(
        "emergency-authors-ca-alone",
        "root B withdraws the derived Authors CA alone: bob's chain is withdrawn",
        (T::Withdrawn, C::RevokedKeyCompromise),
        |_, v| {
            bob_vector(v);
            v.store.root_sections = vec!["@emergency-authors-2".into()];
        },
    )?;
    g.add(
        "emergency-other-chain-unaffected",
        "the Authors CA withdrawal does not touch a chain through another CA (WolOS's signer)",
        (T::Wolos, C::Ok),
        |_, v| {
            v.app_id = Some("org.wolos.notes".into());
            v.wsig = Some("@wsig-signer".into());
            v.chain = Some("@chain-signer".into());
            v.store.root_sections = vec!["@emergency-authors-2".into()];
        },
    )?;
    g.add(
        "emergency-forged-by-intermediate",
        "an emergency section signed by the Index, not a root, is refused: bob stays valid",
        (T::Origin, C::Ok),
        |_, v| {
            bob_vector(v);
            v.store.intermediates = vec!["@index".into(), "@index-2".into()];
            v.store.root_sections = vec![inline(&forged)];
        },
    )?;
    let old_db = db(ids, 4, NOW - DAY, &[], None, [0; 32]);
    g.add(
        "emergency-replayed-older-list",
        "the machine kept root B's withdrawal; an older database without it, replayed, cannot undo it",
        (T::Withdrawn, C::RevokedKeyCompromise),
        |_, v| {
            bob_vector(v);
            v.store.intermediates = vec!["@index".into(), "@index-2".into()];
            v.store.rkdb = vec![inline(&old_db)];
            v.store.root_sections = vec!["@emergency-authors-2".into()];
        },
    )?;
    let in_db = db(
        ids,
        5,
        NOW - DAY,
        &[entry(
            &ids.bob.body.serial,
            NOW - 2 * DAY,
            Reason::KeyCompromise,
        )],
        None,
        [0; 32],
    );
    g.add(
        "rkdb-revokes-leaf",
        "bob's id in the derived Index's revocation database: found by binary search, withdrawn",
        (T::Withdrawn, C::RevokedKeyCompromise),
        |_, v| {
            bob_vector(v);
            v.store.intermediates = vec!["@index".into(), "@index-2".into()];
            v.store.rkdb = vec![inline(&in_db)];
        },
    )?;
    let idx_wrong = db(
        ids,
        6,
        NOW - DAY,
        &[entry(
            &ids.authors.body.serial,
            NOW - 2 * DAY,
            Reason::KeyCompromise,
        )],
        None,
        [0; 32],
    );
    g.add(
        "rkdb-index-cannot-revoke-a-ca",
        "the Index's database names the Authors CA: ignored, only a root withdraws a CA",
        (T::Origin, C::Ok),
        |_, v| {
            bob_vector(v);
            v.store.intermediates = vec!["@index".into(), "@index-2".into()];
            v.store.rkdb = vec![inline(&idx_wrong)];
        },
    )?;
    cases_audit2(g, ids)
}

/// Audit 2: the emergency section that withdraws the Index travels in the Index's own database
/// (H1), lists and databases with an eternal lifetime (M1), a root section from the future (L2).
fn cases_audit2(g: &mut Gen, ids: &Ids) -> Result<(), String> {
    use Code as C;
    use Trust as T;
    let index_gone = rkdb::RootSection::make(
        u32::try_from(NOW - DAY).unwrap_or(0),
        &[entry(
            &ids.index.body.serial,
            NOW - DAY,
            Reason::KeyCompromise,
        )],
        &g.p.root_b.body.serial,
        &g.p.root_b.secret,
    )
    .unwrap_or_default();
    let carried = db(ids, 7, NOW - 3600, &[], Some(&index_gone[..]), [0; 32]);
    let wsig_index = make::wsig(
        &ids.index.body.serial,
        &ids.index.secret(),
        Purpose::Index,
        CONTENT,
    );
    let index_vector = |v: &mut Vector| {
        v.purpose = Purpose::Index.label().into();
        v.app_id = None;
        v.chain = None;
        v.wsig = Some(inline(&wsig_index));
        v.store.roots = vec!["@root-a".into(), "@root-b".into()];
        v.store.intermediates = vec!["@index".into(), "@index-2".into()];
    };
    g.add(
        "emergency-index-in-db",
        "root B withdraws the derived Index in a section carried by a database that Index signed: \
         the database is refused, the section is kept, and what the Index signed is withdrawn",
        (T::Withdrawn, C::RevokedKeyCompromise),
        |_, v| {
            index_vector(v);
            v.store.rkdb = vec![inline(&carried)];
        },
    )?;
    let future = rkdb::RootSection::make(
        u32::try_from(NOW + 300 * DAY).unwrap_or(0),
        &[entry(
            &ids.index.body.serial,
            NOW + 300 * DAY,
            Reason::Ceased,
        )],
        &g.p.root_b.body.serial,
        &g.p.root_b.secret,
    )
    .unwrap_or_default();
    g.add(
        "emergency-section-future-dated",
        "a root section dated 300 days ahead of the clock is refused (beyond the 24-hour skew): \
         it would delay a ceased or superseded withdrawal",
        (T::Wolos, C::Ok),
        |_, v| {
            index_vector(v);
            v.store.root_sections = vec![inline(&future)];
        },
    )?;
    let eternal_list = crate::revoked::ListText {
        format: 1,
        serial: 11,
        issued: NOW - DAY,
        expires: NOW + 100 * 365 * DAY,
        entries: Vec::new(),
    };
    let spec = g.list(&eternal_list, 0);
    g.add(
        "list-eternal",
        "a validly signed list fresh for a hundred years is refused: lifetime at most 7 days plus \
         the 24-hour skew, so the signature keeps its value with \"not checked since\"",
        (T::Origin, C::ListTooOld),
        |_, v| v.store.lists = vec![spec.clone()],
    )?;
    let wsig_no_at = make::wsig(
        &ids.witness.body.serial,
        &ids.witness.secret(),
        Purpose::Witness,
        &g.p.root_a.file,
    );
    g.add(
        "witness-without-at-refused",
        "a witness signature without its date is not a witness signature: refused, never wolos",
        (T::Invalid, C::SignatureMalformed),
        |g, v| {
            v.purpose = Purpose::Witness.label().into();
            v.content = inline(&g.p.root_a.file);
            v.app_id = None;
            v.chain = None;
            v.wsig = Some(inline(&wsig_no_at));
            v.store.roots = vec!["@root-a".into(), "@root-b".into()];
            v.store.intermediates = vec!["@index".into(), "@witness-a".into()];
        },
    )?;
    Ok(())
}

/// The witness: valid on a published file, refused as an app's signer.
fn cases_witness(g: &mut Gen, ids: &Ids) -> Result<(), String> {
    use Code as C;
    use Trust as T;
    let witness_sig = crate::witness::sign(
        &ids.witness.body.serial,
        &ids.witness.secret(),
        &g.p.root_a.file,
        NOW - DAY,
    );
    g.add(
        "witness-valid",
        "witness A's detached signature on root A's certificate: both roots signed the witness",
        (T::Wolos, C::Ok),
        |g, v| {
            v.purpose = Purpose::Witness.label().into();
            v.content = inline(&g.p.root_a.file);
            v.app_id = None;
            v.chain = None;
            v.wsig = Some(inline(&witness_sig));
            v.store.roots = vec!["@root-a".into(), "@root-b".into()];
            v.store.intermediates = vec!["@index".into(), "@witness-a".into()];
        },
    )?;
    let witness_pkg = make::wsig(
        &ids.witness.body.serial,
        &ids.witness.secret(),
        Purpose::Package,
        CONTENT,
    );
    g.add(
        "witness-as-app-signer-refused",
        "a witness never signs an app",
        (T::Invalid, C::WrongUsage),
        |_, v| {
            v.app_id = Some("bob.notes".into());
            v.chain = None;
            v.wsig = Some(inline(&witness_pkg));
            v.store.roots = vec!["@root-a".into(), "@root-b".into()];
            v.store.intermediates = vec!["@index".into(), "@witness-a".into()];
        },
    )?;
    Ok(())
}

/// A non-TEST root (derived id, label `vectors-root-c`), a legacy-serial intermediate under it, a
/// legacy-serial person, and the person's signature.
fn legacy_under_production_root() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let rk = KeyMaterial::new(pki::seed("vectors-root-c"), true);
    let rs = rk.secret();
    let root = Body {
        serial: rs.id(None),
        kind: Kind::Root,
        subject: Subject {
            label: Some("vectors-root-c".into()),
            text: Some("Vectors root C".into()),
            ..Subject::default()
        },
        namespaces: vec!["*".into()],
        usage: vec![Usage::SignCerts, Usage::SignRevocations],
        keys: make::keys_text(&rs),
        issuer: rs.id(None),
        not_before: NOW - 365 * DAY,
        not_after: NOW + 3650 * DAY,
        level: Level::Wolos,
        verified_at: None,
        class: None,
        origin_cert: None,
        terms: None,
    };
    let root_file = make::issue(&root, &rs);
    let ik = Secret::from_seed(&pki::seed("vectors-ca-c"), false);
    let inter = Body {
        serial: pki::serial("vectors-ca-c"),
        kind: Kind::Intermediate,
        subject: label("vectors-ca-c"),
        namespaces: vec!["*".into()],
        usage: vec![Usage::SignCerts],
        keys: make::keys_text(&ik),
        issuer: root.serial.clone(),
        not_before: NOW - 30 * DAY,
        not_after: NOW + 300 * DAY,
        level: Level::Wolos,
        verified_at: None,
        class: None,
        origin_cert: None,
        terms: None,
    };
    let inter_file = make::issue(&inter, &rs);
    let lk = Secret::from_seed(&pki::seed("vectors-carol"), false);
    let leaf = Body {
        serial: pki::serial("vectors-carol"),
        kind: Kind::Person,
        subject: Subject {
            github_id: Some(6006),
            login: Some("carol".into()),
            name: Some("Carol".into()),
            ..Subject::default()
        },
        namespaces: vec!["carol.".into()],
        usage: vec![Usage::SignPackage],
        keys: make::keys_text(&lk),
        issuer: inter.serial.clone(),
        not_before: NOW - 30 * DAY,
        not_after: NOW + 300 * DAY,
        level: Level::Verified,
        verified_at: None,
        class: None,
        origin_cert: None,
        terms: None,
    };
    let leaf_file = make::issue(&leaf, &ik);
    let wsig = make::wsig(&leaf.serial, &lk, Purpose::Package, CONTENT);
    (root_file, chain(&[&leaf_file, &inter_file]), wsig)
}

/// The signature line of every TEST certificate (`vectors/lines.json`): roots, CAs (the derived
/// ones with both roots in `issued-by`), witnesses, people, organisations and WolOS's signer.
pub fn cert_lines() -> std::collections::BTreeMap<String, String> {
    use crate::line;
    let p = Pki::new();
    let ids = Ids::new(&p);
    let c = |f: &[u8]| crate::Cert::parse(f).ok();
    let mut out = std::collections::BTreeMap::new();
    let (Some(ra), Some(rb)) = (c(&p.root_a.file), c(&p.root_b.file)) else {
        return out;
    };
    let mut put = |name: &str, file: &[u8], issuers: &[&crate::Cert]| {
        if let Some(x) = c(file) {
            out.insert(name.to_owned(), line::cert(&x, issuers));
        }
    };
    put("root-a", &p.root_a.file, &[]);
    put("root-b", &p.root_b.file, &[]);
    put("authors", &p.authors.file, &[&ra]);
    put("packages", &p.packages.file, &[&ra]);
    put("index", &p.index.file, &[&ra]);
    put("authors-2", &ids.authors.file, &[&ra, &rb]);
    put("index-2", &ids.index.file, &[&ra, &rb]);
    put("witness-a", &ids.witness.file, &[&ra, &rb]);
    let (Some(au), Some(au2), Some(pk)) = (
        c(&p.authors.file),
        c(&ids.authors.file),
        c(&p.packages.file),
    ) else {
        return out;
    };
    put("alice", &p.alice.file, &[&au]);
    put("alice-trusted", &p.alice_trusted.file, &[&au]);
    put("org", &p.org.file, &[&au]);
    put("ana", &p.ana.file, &[&au]);
    put("signer", &p.signer.file, &[&pk]);
    put("bob", &ids.bob.file, &[&au2]);
    // The production shape with the two subject texts of the lead's runbook example (03:19Z,
    // 03:20Z), made from public seeds: each root shows its own subject; an intermediate both.
    let root = |seed: &str, label: &str, text: &str| {
        let k = KeyMaterial::new(pki::seed(seed), true);
        let s = k.secret();
        let b = Body {
            serial: s.id(None),
            kind: Kind::Root,
            subject: Subject {
                label: Some(label.into()),
                text: Some(text.into()),
                ..Subject::default()
            },
            namespaces: vec!["*".into(), "org.wolos.".into()],
            usage: vec![Usage::SignCerts, Usage::SignRevocations],
            keys: make::keys_text(&s),
            issuer: s.id(None),
            not_before: NOW,
            not_after: NOW + 20 * 365 * DAY,
            level: Level::Wolos,
            verified_at: None,
            class: None,
            origin_cert: None,
            terms: None,
        };
        (make::issue(&b, &s), s, b.serial)
    };
    let (fa, sa, ia) = root(
        "example-root-a",
        "wolos-root-a",
        "Júlia Klee - Kleedolinux - WolOS.org - 10/04/2009 - 9 de Julho 1932 - São Paulo",
    );
    let (fb, sb, ib) = root(
        "example-root-b",
        "wolos-root-b",
        "WolOS Project - hello@wolos.org",
    );
    let ik = KeyMaterial::new(pki::seed("example-authors-ca"), false);
    let is = ik.secret();
    let ib_body = Body {
        serial: is.id(None),
        kind: Kind::Intermediate,
        subject: label("wolos-authors-ca"),
        namespaces: vec!["*".into()],
        usage: vec![Usage::SignCerts],
        keys: make::keys_text(&is),
        issuer: ia,
        not_before: NOW,
        not_after: NOW + 3 * 365 * DAY,
        level: Level::Wolos,
        verified_at: None,
        class: None,
        origin_cert: None,
        terms: None,
    };
    let fi = make::issue_cosigned(&ib_body, &sa, &[(ib.as_str(), &sb)]);
    if let (Some(a), Some(b)) = (c(&fa), c(&fb)) {
        out.insert("example-root-a".into(), line::cert(&a, &[]));
        out.insert("example-root-b".into(), line::cert(&b, &[]));
        if let Some(i) = c(&fi) {
            out.insert("example-authors-ca".into(), line::cert(&i, &[&a, &b]));
        }
    }
    out
}
