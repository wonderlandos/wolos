// SPDX-License-Identifier: BSD-2-Clause
//! Test vectors (interface I1): the format, a runner, and the generator behind
//! `woltrust-vectors`. A vector is one JSON file holding the inputs of a check (clock, purpose,
//! signed bytes, signature, chain, store, lists) and the expected result, including the exact
//! report JSON. Bytes are base64url strings; a string starting with `@` names a blob in
//! `pki.json`, which holds the TEST authority once so the vectors stay small. A native build and the
//! wasm build of the hub must give the same `expect` for every file.
use crate::cert::{Cert, Kind};
use crate::crypto::{Purpose, Wsig};
use crate::make;
use crate::pki::{DAY, NOW, Pki, chain_of};
use crate::report::{Trust, Verdict};
use crate::revoked::{ListText, Reason};
use crate::store::Store;
use crate::verify::{Code, Input, Outcome, verify};
use crate::{LIST_LIFETIME, STALE_WINDOW, b64};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Largest blob read from a vector file.
const MAX_BLOB: usize = 1 << 20;

/// `pki.json`: the TEST authority's named blobs.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PkiFile {
    /// Always 1.
    pub format: u32,
    /// Always `test`.
    pub name: String,
    /// Name to base64url bytes.
    pub blobs: BTreeMap<String, String>,
}

/// A revocation list and its signature file.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListSpec {
    /// `revoked.json`.
    pub list: String,
    /// `revoked.json.wsig`.
    pub wsig: String,
}

/// What the machine holds.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoreSpec {
    /// Root certificates.
    pub roots: Vec<String>,
    /// Intermediate certificates.
    pub intermediates: Vec<String>,
    /// Revocation lists.
    pub lists: Vec<ListSpec>,
    /// Whether a root labelled `test...` may load (true for every vector; the flag is part of the
    /// replay so `test-root-refused` can say false).
    pub allow_test: bool,
    /// Revocation databases (`revoked.rkdb`), loaded after the lists.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rkdb: Vec<String>,
    /// Root-signed sections kept on their own (`.rkdr`, the memory of emergency withdrawals).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub root_sections: Vec<String>,
}

/// The expected result.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    /// Trust value.
    pub trust: Trust,
    /// Code string.
    pub code: String,
    /// True when it stands on a stale list.
    pub stale: bool,
    /// Serial of the signing certificate, if one was found.
    pub cert: Option<String>,
    /// Roots that loaded.
    pub roots: usize,
    /// Intermediates that loaded.
    pub intermediates: usize,
    /// Revocation lists that loaded (a list with a bad signature does not).
    pub lists: usize,
    /// Revocation databases and root sections that verified.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dbs: usize,
    /// `outcome.report(Verdict::Pass, [])` as JSON text, byte for byte.
    pub report: String,
    /// The signature line (`crate::line::signature`) of a valid file (made by WolOS, trusted or
    /// origin); absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
}

/// One vector file.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Vector {
    /// Always 1.
    pub format: u32,
    /// File name without `.json`.
    pub name: String,
    /// One sentence on what is being tested.
    pub about: String,
    /// The clock.
    pub now: u64,
    /// The signature label (`wolos/package/1`, ...).
    pub purpose: String,
    /// The signed bytes.
    pub content: String,
    /// The app id from `Info.json`.
    pub app_id: Option<String>,
    /// The index's `first_seen` for this file.
    pub first_seen: Option<u64>,
    /// `SHA256.wsig`, or null for an unsigned file.
    pub wsig: Option<String>,
    /// The `CHAIN` member, or null.
    pub chain: Option<String>,
    /// The store.
    pub store: StoreSpec,
    /// The result.
    pub expect: Expect,
}

fn blob(pki: &PkiFile, s: &str) -> Result<Vec<u8>, String> {
    match s.strip_prefix('@') {
        Some(name) => {
            let t = pki
                .blobs
                .get(name)
                .ok_or_else(|| format!("no blob {name}"))?;
            b64::decode(t, MAX_BLOB).ok_or_else(|| format!("blob {name} damaged"))
        }
        None => b64::decode(s, MAX_BLOB).ok_or_else(|| "value is not base64url".to_owned()),
    }
}

/// Runs the vector and returns what the code makes of it (no comparison).
pub fn run(v: &Vector, pki: &PkiFile) -> Result<(Outcome, Store), String> {
    let all = |xs: &[String]| {
        xs.iter()
            .map(|s| blob(pki, s))
            .collect::<Result<Vec<_>, _>>()
    };
    let lists = v
        .store
        .lists
        .iter()
        .map(|l| Ok((blob(pki, &l.list)?, blob(pki, &l.wsig)?)))
        .collect::<Result<Vec<_>, String>>()?;
    // Root sections first (audit 2 M5), as a machine loads them.
    let mut store = Store::load_with_sections(
        &all(&v.store.roots)?,
        &all(&v.store.intermediates)?,
        &all(&v.store.root_sections)?,
        &lists,
        v.now,
        v.store.allow_test,
    );
    for d in &v.store.rkdb {
        let _ = store.load_rkdb(&blob(pki, d)?, v.now);
    }
    let content = blob(pki, &v.content)?;
    let wsig = v.wsig.as_deref().map(|s| blob(pki, s)).transpose()?;
    let chain = v.chain.as_deref().map(|s| blob(pki, s)).transpose()?;
    let purpose = Purpose::from_label(&v.purpose).ok_or("unknown purpose label")?;
    let out = verify(
        &store,
        &Input {
            purpose,
            content: &content,
            app_id: v.app_id.as_deref(),
            wsig: wsig.as_deref(),
            chain: chain.as_deref(),
            first_seen: v.first_seen,
            now: v.now,
        },
    );
    Ok((out, store))
}

/// The expectation that matches an actual run.
pub fn observed(out: &Outcome, store: &Store) -> Expect {
    Expect {
        trust: out.trust,
        code: out.code.as_str().to_owned(),
        stale: out.stale,
        cert: out.cert.as_ref().map(|c| c.serial.clone()),
        roots: store.roots.len(),
        intermediates: store.intermediates.len(),
        lists: store.lists.len(),
        dbs: store.dbs.len() + store.root_sections.len(),
        report: out.report(Verdict::Pass, Vec::new()).to_json(),
        line: out
            .line
            .clone()
            .filter(|_| matches!(out.trust, Trust::Wolos | Trust::Trusted | Trust::Origin)),
    }
}

/// Runs the vector and compares everything with `expect`.
pub fn check(v: &Vector, pki: &PkiFile) -> Result<(), String> {
    let (out, store) = run(v, pki)?;
    let got = observed(&out, &store);
    if got == v.expect {
        Ok(())
    } else {
        Err(format!(
            "{}: expected {:?}, got {:?}",
            v.name, v.expect, got
        ))
    }
}

// ---- generator ----

pub(crate) const CONTENT: &[u8] = b"test package manifest 1\n";

pub(crate) struct Gen {
    pub(crate) p: Pki,
    pub(crate) blobs: BTreeMap<String, String>,
    pub(crate) out: Vec<Vector>,
}

fn flip(mut b: Vec<u8>, at: usize) -> Vec<u8> {
    if let Some(x) = b.get_mut(at) {
        *x ^= 0x01;
    }
    b
}

/// `wsig` with one bit flipped in the Ed25519 half (`second == false`) or the other half.
fn flip_half(wsig: &[u8], second: bool) -> Vec<u8> {
    let (Ok(w), true) = (Wsig::parse(wsig), true) else {
        return Vec::new();
    };
    let Some(s) = w.sig() else {
        return Vec::new();
    };
    let (ed, sec) = if second {
        (s.ed25519.to_vec(), flip(s.second, 7))
    } else {
        (flip(s.ed25519.to_vec(), 7), s.second)
    };
    Wsig {
        cert: w.cert,
        ed25519: b64::encode(&ed),
        second: b64::encode(&sec),
        origin: w.origin,
        at: None,
    }
    .to_bytes()
}

/// A certificate file with one bit flipped in one half of its issuer signature.
fn flip_cert(file: &[u8], second: bool) -> Vec<u8> {
    let Ok(c) = Cert::parse(file) else {
        return Vec::new();
    };
    let mut sig = c.sig.clone();
    if second {
        sig.second = flip(sig.second, 9);
    } else {
        sig.ed25519[9] ^= 1;
    }
    crate::cert::encode(&c.body_bytes, &sig)
}

pub(crate) fn inline(bytes: &[u8]) -> String {
    b64::encode(bytes)
}

impl Gen {
    pub(crate) fn put(&mut self, name: &str, bytes: &[u8]) -> String {
        self.blobs.insert(name.to_owned(), b64::encode(bytes));
        format!("@{name}")
    }

    pub(crate) fn list_text(serial: u64, issued: u64, entries: &[(&str, u64, Reason)]) -> ListText {
        ListText {
            format: 1,
            serial,
            issued,
            expires: issued + LIST_LIFETIME,
            entries: entries
                .iter()
                .map(|(s, at, r)| crate::revoked::Entry {
                    serial: (*s).to_owned(),
                    at: *at,
                    reason: *r,
                })
                .collect(),
        }
    }

    /// A list signed by the Index (`by = 0`), root A (1) or root B (2).
    pub(crate) fn list(&self, l: &ListText, by: u8) -> ListSpec {
        let id = match by {
            0 => &self.p.index,
            1 => &self.p.root_a,
            _ => &self.p.root_b,
        };
        let (a, b) = make::revocation_list(l, &id.body.serial, &id.secret);
        ListSpec {
            list: inline(&a),
            wsig: inline(&b),
        }
    }

    fn base(&self, name: &str, about: &str) -> Vector {
        Vector {
            format: 1,
            name: name.to_owned(),
            about: about.to_owned(),
            now: NOW,
            purpose: Purpose::Package.label().to_owned(),
            content: "@content".into(),
            app_id: Some("alice.notes".into()),
            first_seen: None,
            wsig: Some("@wsig-alice".into()),
            chain: Some("@chain-alice".into()),
            store: StoreSpec {
                roots: vec!["@root-a".into()],
                intermediates: vec!["@index".into()],
                lists: vec![self.list(&Self::list_text(10, NOW - DAY, &[]), 0)],
                allow_test: true,
                rkdb: Vec::new(),
                root_sections: Vec::new(),
            },
            expect: Expect {
                trust: Trust::Invalid,
                code: String::new(),
                stale: false,
                cert: None,
                roots: 0,
                intermediates: 0,
                lists: 0,
                dbs: 0,
                report: String::new(),
                line: None,
            },
        }
    }

    /// Adds a vector; fails when the code does not give the trust and code the author meant.
    pub(crate) fn add(
        &mut self,
        name: &str,
        about: &str,
        want: (Trust, Code),
        f: impl FnOnce(&mut Gen, &mut Vector),
    ) -> Result<(), String> {
        let mut v = self.base(name, about);
        f(self, &mut v);
        let pki = self.pki_file();
        let (out, store) = run(&v, &pki)?;
        if (out.trust, out.code) != want {
            return Err(format!(
                "{name}: wanted {want:?}, got {:?} {:?}",
                out.trust, out.code
            ));
        }
        v.expect = observed(&out, &store);
        self.out.push(v);
        Ok(())
    }

    fn pki_file(&self) -> PkiFile {
        PkiFile {
            format: 1,
            name: "test".into(),
            blobs: self.blobs.clone(),
        }
    }
}

/// Builds `pki.json` and every vector. Deterministic: two runs give the same bytes.
pub fn generate() -> Result<(PkiFile, Vec<Vector>), String> {
    let mut g = Gen {
        p: Pki::new(),
        blobs: BTreeMap::new(),
        out: Vec::new(),
    };
    g.put("content", CONTENT);
    let files: Vec<(&str, Vec<u8>)> = vec![
        ("root-a", g.p.root_a.file.clone()),
        ("root-b", g.p.root_b.file.clone()),
        ("authors", g.p.authors.file.clone()),
        ("packages", g.p.packages.file.clone()),
        ("index", g.p.index.file.clone()),
        ("alice", g.p.alice.file.clone()),
        ("org", g.p.org.file.clone()),
        ("signer", g.p.signer.file.clone()),
        ("ana", g.p.ana.file.clone()),
        ("chain-ana", Pki::chain(&[&g.p.ana, &g.p.authors])),
        ("chain-alice", Pki::chain(&[&g.p.alice, &g.p.authors])),
        ("chain-org", Pki::chain(&[&g.p.org, &g.p.authors])),
        ("chain-signer", Pki::chain(&[&g.p.signer, &g.p.packages])),
        ("chain-alice-only", Pki::chain(&[&g.p.alice])),
        ("alice-trusted", g.p.alice_trusted.file.clone()),
        (
            "chain-alice-trusted",
            Pki::chain(&[&g.p.alice_trusted, &g.p.authors]),
        ),
        (
            "wsig-alice-trusted",
            make::wsig(
                &g.p.alice_trusted.body.serial,
                &g.p.alice_trusted.secret,
                Purpose::Package,
                CONTENT,
            ),
        ),
        (
            "wsig-alice",
            make::wsig(
                &g.p.alice.body.serial,
                &g.p.alice.secret,
                Purpose::Package,
                CONTENT,
            ),
        ),
        (
            "wsig-org",
            make::wsig(
                &g.p.org.body.serial,
                &g.p.org.secret,
                Purpose::Package,
                CONTENT,
            ),
        ),
        (
            "wsig-signer",
            make::wsig(
                &g.p.signer.body.serial,
                &g.p.signer.secret,
                Purpose::Package,
                CONTENT,
            ),
        ),
    ];
    for (n, b) in &files {
        g.put(n, b);
    }
    cases(&mut g)?;
    let pki = g.pki_file();
    Ok((pki, g.out))
}

fn revoked_case(
    g: &mut Gen,
    name: &str,
    reason: Reason,
    first_seen: Option<u64>,
    want: (Trust, Code),
) -> Result<(), String> {
    let serial = g.p.alice.body.serial.clone();
    let l = Gen::list_text(11, NOW - DAY, &[(&serial, NOW - 2 * DAY, reason)]);
    g.add(
        name,
        "a leaf certificate in the revocation list",
        want,
        |g, v| {
            v.store.lists = vec![g.list(&l, 0)];
            v.first_seen = first_seen;
        },
    )
}

fn cases(g: &mut Gen) -> Result<(), String> {
    cases_valid(g)?;
    cases_signature(g)?;
    cases_time(g)?;
    cases_revoked(g)?;
    cases_lists(g)?;
    cases_shape(g)?;
    cases_origin(g)?;
    cases_class(g)?;
    crate::vectors_ids::cases(g)
}

/// The two certificates of one person (lead decisions 2026-11-04): trusted after an administrator
/// approved her, origin from sign-in. Key-compromise and abuse on either kill everything; the
/// trusted one superseded leaves origin; ceased keeps older files; withdrawing the origin
/// withdraws what the trusted one signed; a version entry never touches a certificate.
fn cases_class(g: &mut Gen) -> Result<(), String> {
    use Code as C;
    use Trust as T;
    let trusted = |v: &mut Vector| {
        v.wsig = Some("@wsig-alice-trusted".into());
        v.chain = Some("@chain-alice-trusted".into());
    };
    let t_serial = g.p.alice_trusted.body.serial.clone();
    let o_serial = g.p.alice.body.serial.clone();
    g.add(
        "valid-trusted",
        "a package signed with a trusted certificate: trusted developer",
        (T::Trusted, C::Ok),
        |_, v| trusted(v),
    )?;
    let l = Gen::list_text(
        13,
        NOW - DAY,
        &[(&t_serial, NOW - 2 * DAY, Reason::Superseded)],
    );
    g.add(
        "trusted-superseded-downgrade",
        "the trusted certificate withdrawn as superseded: the file stays valid, shown as origin",
        (T::Origin, C::TrustWithdrawn),
        |g, v| {
            trusted(v);
            v.store.lists = vec![g.list(&l, 0)];
        },
    )?;
    for (name, about, reason, first_seen, want) in [
        (
            "trusted-abuse",
            "the trusted certificate withdrawn for abuse: everything its key signed is withdrawn",
            Reason::Abuse,
            None,
            (T::Withdrawn, C::RevokedAbuse),
        ),
        (
            "trusted-key-compromise",
            "the trusted certificate's key exposed: withdrawn, even for a file seen long before",
            Reason::KeyCompromise,
            Some(NOW - 300 * DAY),
            (T::Withdrawn, C::RevokedKeyCompromise),
        ),
        (
            "trusted-ceased",
            "the trusted developer stopped publishing: a file not known to predate it is withdrawn",
            Reason::Ceased,
            None,
            (T::Withdrawn, C::RevokedCeased),
        ),
        (
            "trusted-ceased-older-file",
            "the trusted developer stopped publishing: a file the index saw before stays trusted",
            Reason::Ceased,
            Some(NOW - 10 * DAY),
            (T::Trusted, C::Ok),
        ),
    ] {
        let l = Gen::list_text(13, NOW - DAY, &[(&t_serial, NOW - 2 * DAY, reason)]);
        g.add(name, about, want, |g, v| {
            trusted(v);
            v.store.lists = vec![g.list(&l, 0)];
            v.first_seen = first_seen;
        })?;
    }
    let l = Gen::list_text(
        16,
        NOW - DAY,
        &[(&o_serial, NOW - 2 * DAY, Reason::Superseded)],
    );
    g.add(
        "trusted-origin-superseded",
        "the origin certificate superseded: what its trusted certificate signed afterwards is withdrawn",
        (T::Withdrawn, C::RevokedSuperseded),
        |g, v| {
            trusted(v);
            v.store.lists = vec![g.list(&l, 0)];
        },
    )?;
    let l = Gen::list_text(
        14,
        NOW - DAY,
        &[(&o_serial, NOW - 2 * DAY, Reason::KeyCompromise)],
    );
    g.add(
        "trusted-origin-revoked",
        "the origin certificate withdrawn (supply-chain emergency): what its trusted certificate signed is withdrawn too",
        (T::Withdrawn, C::RevokedKeyCompromise),
        |g, v| {
            trusted(v);
            v.store.lists = vec![g.list(&l, 0)];
        },
    )?;
    let l = Gen::list_text(
        15,
        NOW - DAY,
        &[(&t_serial, NOW - 2 * DAY, Reason::WithdrawnVersion)],
    );
    g.add(
        "version-entry-ignored-for-certificates",
        "a withdrawn-version entry with a certificate's serial: certificate checks ignore it",
        (T::Trusted, C::Ok),
        |g, v| {
            trusted(v);
            v.store.lists = vec![g.list(&l, 0)];
        },
    )?;
    Ok(())
}

/// The origin record of the `ana` vectors, from `owner`'s repository.
fn origin_of(owner: &str) -> Vec<u8> {
    crate::origin::Origin {
        repo: format!("https://github.com/{owner}/notes"),
        tag: "v1.2".into(),
        commit: "0123456789abcdef0123456789abcdef01234567".into(),
        built: NOW - DAY,
        rkdb_serial: None,
    }
    .to_bytes()
}

fn cases_origin(g: &mut Gen) -> Result<(), String> {
    use Code as C;
    use Trust as T;
    let ana = |g: &Gen, owner: &str| {
        make::wsig_with_origin(
            &g.p.ana.body.serial,
            &g.p.ana.secret,
            Purpose::Package,
            CONTENT,
            &origin_of(owner),
        )
    };
    g.add(
        "valid-origin",
        "a release from the signer's own repository: verified, with its origin and organisation",
        (T::Origin, C::Ok),
        |g, v| {
            v.app_id = Some("ana.notes".into());
            v.wsig = Some(inline(&ana(g, "ana")));
            v.chain = Some("@chain-ana".into());
        },
    )?;
    g.add(
        "valid-origin-organisation",
        "a release from the repository of the organisation the person publishes for",
        (T::Origin, C::Ok),
        |g, v| {
            v.app_id = Some("ana.notes".into());
            v.wsig = Some(inline(&ana(g, "anaco")));
            v.chain = Some("@chain-ana".into());
        },
    )?;
    g.add(
        "origin-mismatch",
        "the signed origin names someone else's repository",
        (T::Invalid, C::OriginMismatch),
        |g, v| {
            v.app_id = Some("ana.notes".into());
            v.wsig = Some(inline(&ana(g, "bob")));
            v.chain = Some("@chain-ana".into());
        },
    )?;
    g.add(
        "origin-tampered",
        "the origin changed after signing (another owner's repository put in)",
        (T::Invalid, C::BadSignature),
        |g, v| {
            let mut w = Wsig::parse(&ana(g, "ana")).unwrap_or_else(|_| Wsig {
                cert: String::new(),
                ed25519: String::new(),
                second: String::new(),
                origin: None,
                at: None,
            });
            w.origin = Some(b64::encode(&origin_of("anaco")));
            v.app_id = Some("ana.notes".into());
            v.wsig = Some(inline(&w.to_bytes()));
            v.chain = Some("@chain-ana".into());
        },
    )?;
    Ok(())
}

fn cases_valid(g: &mut Gen) -> Result<(), String> {
    use Code as C;
    use Trust as T;
    g.add(
        "valid-person",
        "a person's package: verified, fresh list",
        (T::Origin, C::Ok),
        |_, _| {},
    )?;
    g.add(
        "valid-org",
        "an organisation's package",
        (T::Origin, C::Ok),
        |_, v| {
            v.app_id = Some("examplelabs.tool".into());
            v.wsig = Some("@wsig-org".into());
            v.chain = Some("@chain-org".into());
        },
    )?;
    g.add(
        "valid-wolos",
        "a package of WolOS's own: Packages CA chain",
        (T::Wolos, C::Ok),
        |_, v| {
            v.app_id = Some("org.wolos.notes".into());
            v.wsig = Some("@wsig-signer".into());
            v.chain = Some("@chain-signer".into());
        },
    )?;
    g.add(
        "valid-chain-from-store",
        "leaf only in CHAIN, issuer from the store (repair path)",
        (T::Origin, C::Ok),
        |_, v| {
            v.chain = Some("@chain-alice-only".into());
            v.store.intermediates = vec!["@authors".into(), "@index".into()];
        },
    )?;
    g.add(
        "unsigned",
        "no signature member",
        (T::Community, C::Unsigned),
        |_, v| {
            v.wsig = None;
            v.chain = None;
        },
    )?;
    Ok(())
}

fn cases_signature(g: &mut Gen) -> Result<(), String> {
    use Code as C;
    use Trust as T;
    g.add(
        "bad-half-ed25519",
        "one bit flipped in the Ed25519 half",
        (T::Invalid, C::BadSignature),
        |g, v| {
            let w = flip_half(&g.blobs_bytes("wsig-alice"), false);
            v.wsig = Some(inline(&w));
        },
    )?;
    g.add(
        "bad-half-second",
        "one bit flipped in the ML-DSA-65 half: both must verify",
        (T::Invalid, C::BadSignature),
        |g, v| {
            let w = flip_half(&g.blobs_bytes("wsig-alice"), true);
            v.wsig = Some(inline(&w));
        },
    )?;
    g.add(
        "bad-half-cert-second",
        "the issuer's ML-DSA-65 half of the leaf certificate is wrong",
        (T::Invalid, C::BadSignature),
        |g, v| {
            let bad = flip_cert(&g.p.alice.file, true);
            v.chain = Some(inline(&chain_of(&[&bad, &g.p.authors.file])));
        },
    )?;
    g.add(
        "bad-half-cert-ed25519",
        "the issuer's Ed25519 half of the leaf certificate is wrong",
        (T::Invalid, C::BadSignature),
        |g, v| {
            let bad = flip_cert(&g.p.alice.file, false);
            v.chain = Some(inline(&chain_of(&[&bad, &g.p.authors.file])));
        },
    )?;
    g.add(
        "bad-content",
        "the signed bytes changed after signing",
        (T::Invalid, C::BadSignature),
        |_, v| {
            v.content = inline(b"test package manifest 2\n");
        },
    )?;
    g.add(
        "replayed-label",
        "a package signature offered as a plugin signature",
        (T::Invalid, C::BadSignature),
        |_, v| {
            v.purpose = Purpose::Plugin.label().to_owned();
        },
    )?;
    Ok(())
}

fn cases_time(g: &mut Gen) -> Result<(), String> {
    use Code as C;
    use Trust as T;
    let two_years = 2 * 365 * DAY;
    g.add(
        "expired",
        "the leaf has expired and the index has no timestamp for the file",
        (T::Community, C::Expired),
        |g, v| {
            v.now = NOW + two_years;
            v.store.lists = vec![g.list(&Gen::list_text(12, NOW + two_years - DAY, &[]), 0)];
        },
    )?;
    g.add(
        "expired-but-timestamped",
        "expired, but the signed index saw this file inside the validity",
        (T::Origin, C::Ok),
        |g, v| {
            v.now = NOW + two_years;
            v.first_seen = Some(NOW);
            v.store.lists = vec![g.list(&Gen::list_text(12, NOW + two_years - DAY, &[]), 0)];
        },
    )?;
    g.add(
        "not-yet-valid",
        "the clock is before the certificate starts",
        (T::Community, C::NotYetValid),
        |_, v| {
            v.now = NOW - 60 * DAY;
        },
    )?;
    Ok(())
}

fn cases_revoked(g: &mut Gen) -> Result<(), String> {
    use Code as C;
    use Trust as T;
    revoked_case(
        g,
        "revoked-key-compromise",
        Reason::KeyCompromise,
        None,
        (T::Withdrawn, C::RevokedKeyCompromise),
    )?;
    revoked_case(
        g,
        "revoked-key-compromise-old-file",
        Reason::KeyCompromise,
        Some(NOW - 10 * DAY),
        (T::Withdrawn, C::RevokedKeyCompromise),
    )?;
    revoked_case(
        g,
        "revoked-abuse",
        Reason::Abuse,
        None,
        (T::Withdrawn, C::RevokedAbuse),
    )?;
    revoked_case(
        g,
        "revoked-superseded",
        Reason::Superseded,
        None,
        (T::Withdrawn, C::RevokedSuperseded),
    )?;
    revoked_case(
        g,
        "revoked-ceased",
        Reason::Ceased,
        None,
        (T::Withdrawn, C::RevokedCeased),
    )?;
    revoked_case(
        g,
        "revoked-superseded-older-file",
        Reason::Superseded,
        Some(NOW - 10 * DAY),
        (T::Origin, C::Ok),
    )?;
    revoked_case(
        g,
        "revoked-ceased-older-file",
        Reason::Ceased,
        Some(NOW - 10 * DAY),
        (T::Origin, C::Ok),
    )?;
    let authors = g.p.authors.body.serial.clone();
    let l = Gen::list_text(
        11,
        NOW - DAY,
        &[(&authors, NOW - DAY, Reason::KeyCompromise)],
    );
    g.add(
        "revoked-intermediate-by-root",
        "the Authors CA withdrawn by a root: every author under it fails",
        (T::Withdrawn, C::RevokedKeyCompromise),
        |g, v| {
            v.store.lists = vec![g.list(&l, 1)];
        },
    )?;
    g.add(
        "revoked-intermediate-by-index-ignored",
        "the Index may not withdraw an intermediate: the entry does not apply",
        (T::Origin, C::Ok),
        |g, v| {
            v.store.lists = vec![g.list(&l, 0)];
        },
    )?;
    let ra = g.p.root_a.body.serial.clone();
    let l = Gen::list_text(11, NOW - DAY, &[(&ra, NOW - DAY, Reason::KeyCompromise)]);
    g.add(
        "root-a-revoked-by-b",
        "root B withdraws root A; both are pinned",
        (T::Withdrawn, C::RevokedKeyCompromise),
        |g, v| {
            v.store.roots = vec!["@root-a".into(), "@root-b".into()];
            v.store.lists = vec![g.list(&l, 2)];
        },
    )?;
    g.add(
        "root-a-revoked-by-itself-ignored",
        "a root cannot withdraw itself",
        (T::Origin, C::Ok),
        |g, v| {
            v.store.lists = vec![g.list(&l, 1)];
        },
    )?;
    Ok(())
}

fn cases_lists(g: &mut Gen) -> Result<(), String> {
    use Code as C;
    use Trust as T;
    g.add(
        "stale-list",
        "the list ran out 5 days ago: still verified, with the stale line",
        (T::Origin, C::ListStale),
        |g, v| {
            v.store.lists =
                vec![g.list(&Gen::list_text(10, NOW - LIST_LIFETIME - 5 * DAY, &[]), 0)];
        },
    )?;
    g.add(
        "list-too-old",
        "the list ran out more than 30 days ago: the signature keeps its value, not checked for withdrawn certificates",
        (T::Origin, C::ListTooOld),
        |g, v| {
            v.store.lists = vec![g.list(
                &Gen::list_text(10, NOW - LIST_LIFETIME - STALE_WINDOW - DAY, &[]),
                0,
            )];
        },
    )?;
    g.add(
        "no-list",
        "no revocation list at all: verified offline, not checked for withdrawn certificates",
        (T::Origin, C::ListTooOld),
        |_, v| {
            v.store.lists = Vec::new();
        },
    )?;
    g.add(
        "list-bad-signature",
        "a list with a damaged signature is not loaded (as if there were none)",
        (T::Origin, C::ListTooOld),
        |g, v| {
            let l = g.list(&Gen::list_text(10, NOW - DAY, &[]), 0);
            let w = b64::decode(&l.wsig, 1 << 16).unwrap_or_default();
            v.store.lists = vec![ListSpec {
                list: l.list,
                wsig: inline(&flip_half(&w, true)),
            }];
        },
    )?;
    Ok(())
}

fn cases_shape(g: &mut Gen) -> Result<(), String> {
    use Code as C;
    use Trust as T;
    g.add(
        "namespace-mismatch",
        "the app id is outside the certificate's namespaces",
        (T::Invalid, C::WrongNamespace),
        |_, v| {
            v.app_id = Some("bob.notes".into());
        },
    )?;
    g.add(
        "namespace-reserved",
        "an author signing inside org.wolos.",
        (T::Invalid, C::WrongNamespace),
        |_, v| {
            v.app_id = Some("org.wolos.notes".into());
        },
    )?;
    g.add(
        "wrong-usage",
        "an author certificate offered for the index",
        (T::Invalid, C::WrongUsage),
        |_, v| {
            v.purpose = Purpose::Index.label().to_owned();
        },
    )?;
    g.add(
        "bad-issuer",
        "a signer for org.wolos. issued by the Authors CA",
        (T::Invalid, C::BadIssuer),
        |g, v| {
            let sec = crate::make::Secret::from_seed(&crate::pki::seed("test-rogue"), false);
            let spec = crate::pki::Spec {
                name: "test-rogue",
                kind: Kind::Signer,
                subject: crate::cert::Subject {
                    label: Some("test-rogue".into()),
                    ..Default::default()
                },
                namespaces: &["org.wolos."],
                usage: &[crate::cert::Usage::SignPackage],
                issuer: g.p.authors.body.serial.clone(),
                not_before: NOW - DAY,
                not_after: NOW + 100 * DAY,
                level: crate::cert::Level::Wolos,
            };
            let body = crate::pki::body(&spec, &sec);
            let file = make::issue(&body, &g.p.authors.secret);
            let w = make::wsig(&body.serial, &sec, Purpose::Package, CONTENT);
            v.app_id = Some("org.wolos.notes".into());
            v.wsig = Some(inline(&w));
            v.chain = Some(inline(&chain_of(&[&file, &g.p.authors.file])));
        },
    )?;
    g.add(
        "unknown-root",
        "only root B is pinned; the chain ends at root A",
        (T::Invalid, C::IssuerUnknown),
        |_, v| {
            v.store.roots = vec!["@root-b".into()];
        },
    )?;
    g.add(
        "test-root-refused",
        "a system store refuses a root labelled test",
        (T::Invalid, C::IssuerUnknown),
        |_, v| {
            v.store.allow_test = false;
        },
    )?;
    g.add(
        "signer-unknown",
        "the signature names a certificate nobody holds",
        (T::Invalid, C::SignerUnknown),
        |_, v| {
            v.chain = None;
        },
    )?;
    g.add(
        "signature-malformed",
        "the signature file is not a signature",
        (T::Invalid, C::SignatureMalformed),
        |_, v| {
            v.wsig = Some(inline(b"{\"cert\":1}"));
        },
    )?;
    g.add(
        "chain-malformed",
        "CHAIN is not an array of certificates",
        (T::Invalid, C::ChainMalformed),
        |_, v| {
            v.chain = Some(inline(b"{\"not\":\"a chain\"}"));
        },
    )?;
    g.add(
        "chain-too-long",
        "five certificates in CHAIN",
        (T::Invalid, C::ChainTooLong),
        |g, v| {
            let a = &g.p.alice.file;
            let files: Vec<&[u8]> = vec![a, a, a, a, a];
            v.chain = Some(inline(&chain_of(&files)));
        },
    )?;
    g.add(
        "chain-too-large",
        "CHAIN larger than 64 KiB",
        (T::Invalid, C::ChainTooLarge),
        |g, v| {
            let mut c = Pki::chain(&[&g.p.alice, &g.p.authors]);
            c.resize(crate::MAX_CHAIN_BYTES + 1, b' ');
            v.chain = Some(inline(&c));
        },
    )?;
    Ok(())
}

impl Gen {
    fn blobs_bytes(&self, name: &str) -> Vec<u8> {
        self.blobs
            .get(name)
            .and_then(|t| b64::decode(t, MAX_BLOB))
            .unwrap_or_default()
    }
}

/// The pretty JSON text of a vector or `pki.json` (what the files hold, plus a final newline).
pub fn to_text<T: Serialize>(v: &T) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap_or_default();
    s.push('\n');
    s
}
