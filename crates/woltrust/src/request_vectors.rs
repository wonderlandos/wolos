// SPDX-License-Identifier: BSD-2-Clause
//! Test vectors for certificate requests (`vectors/request/`): each holds the exact text of a
//! request file and whether [`crate::request::parse`] plus [`Request::check_proof`] accept it.
//! They prove two things another reader (the hub's CA script, the Worker) relies on: the hub's own
//! copy, with `id`, `approved_by`, `approved_at` and `verified_at` added, reads the
//! same as the file `wol cert request` wrote; and the proof binds every field the CA copies (GitHub
//! id, kind, login, name, organisation, namespaces, keys; audit H2). Keys come from public TEST
//! seeds.
use crate::cert::{Kind, OrgRef, Subject};
use crate::key::KeyMaterial;
use crate::pki::{self, NOW};
use crate::request::{self, Request, RequestError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// What reading the request must give.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestExpect {
    /// True when it parses and its proof verifies.
    pub accepted: bool,
    /// The [`RequestError`] name when refused, else null.
    pub error: Option<String>,
}

/// One request vector.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestVector {
    /// Always 1.
    pub format: u32,
    /// File name without `.json`.
    pub name: String,
    /// What is being tested.
    pub about: String,
    /// The exact bytes of the request file (UTF-8 JSON).
    pub request: String,
    /// The result.
    pub expect: RequestExpect,
}

/// Reads the vector's request as the CA does and returns what came out.
pub fn observe(v: &RequestVector) -> RequestExpect {
    match request::parse(v.request.as_bytes()).and_then(|r| r.check_proof()) {
        Ok(()) => RequestExpect {
            accepted: true,
            error: None,
        },
        Err(e) => RequestExpect {
            accepted: false,
            error: Some(e.as_str().to_owned()),
        },
    }
}

/// Replays the vector.
pub fn check(v: &RequestVector) -> Result<(), String> {
    let got = observe(v);
    if got == v.expect {
        Ok(())
    } else {
        Err(format!("{}: expected {:?}, got {got:?}", v.name, v.expect))
    }
}

/// A request as `wol cert request` writes it, for the TEST key `seed_name`.
fn made(seed_name: &str, kind: Kind, login: &str, name: &str, id: u64, enc: bool) -> Request {
    let km = KeyMaterial::new(pki::seed(seed_name), false);
    let mut keys = km.keys_text();
    if !enc {
        keys.x25519 = None;
        keys.ml_kem_768 = None;
    }
    let mut r = Request {
        format: 1,
        action: "issue".into(),
        kind,
        subject: Subject {
            github_id: Some(id),
            login: Some(login.into()),
            name: Some(name.into()),
            label: None,
            org: None,
            text: None,
        },
        namespaces: vec![format!("{login}.")],
        keys,
        proof: request::Proof {
            ed25519: String::new(),
            second: String::new(),
        },
        months: 12,
        requested_at: NOW - 3600,
        terms: None,
    };
    if let Some(c) = request::Claimed::of_request(&r) {
        r.proof = request::prove(&km.secret(), &c);
    }
    r
}

/// The same request with the hub's additions (`requests/<id>.json`).
fn hub_copy(r: &Request) -> Value {
    let mut v = serde_json::to_value(r).unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.insert("id".into(), json!("r-1001-0001"));
        o.insert("approved_by".into(), json!(["test-admin"]));
        o.insert("approved_at".into(), json!(NOW - 600));
        o.insert("verified_at".into(), json!(NOW - 900));
    }
    v
}

fn text(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap_or_default();
    s.push('\n');
    s
}

fn edit(mut v: Value, f: impl FnOnce(&mut Value)) -> Value {
    f(&mut v);
    v
}

/// One vector's name, description, request text and expected result.
type Case = (&'static str, &'static str, Value, Result<(), RequestError>);

/// The cases over the TEST requests: as written, the hub's copies, and every field changed after
/// the proof.
fn cases(
    alice: &Request,
    org: &Request,
    plain: &Request,
    member: &Request,
    added: &Request,
) -> Result<Vec<Case>, String> {
    let a = serde_json::to_value(alice).map_err(|e| e.to_string())?;
    let other_x = serde_json::to_value(&org.keys.x25519).map_err(|e| e.to_string())?;
    let ok = Ok(());
    Ok(vec![
        (
            "request-person",
            "a person's request as wol cert request writes it, encryption keys included",
            a.clone(),
            ok,
        ),
        (
            "request-org",
            "an organisation's request",
            serde_json::to_value(org).map_err(|e| e.to_string())?,
            ok,
        ),
        (
            "request-signing-keys-only",
            "a request without encryption keys (both halves absent) is accepted",
            serde_json::to_value(plain).map_err(|e| e.to_string())?,
            ok,
        ),
        (
            "request-hub-copy",
            "the hub's copy adds id, approved_by, approved_at and verified_at: ignored, the request reads the same",
            hub_copy(alice),
            ok,
        ),
        (
            "request-hub-copy-member",
            "a person who names the organisation they publish for in the request: the hub's copy is accepted",
            hub_copy(member),
            ok,
        ),
        (
            "request-org-added-later",
            "subject.org added after the proof was made (by the hub or anyone): refused",
            hub_copy(added),
            Err(RequestError::BadProof),
        ),
        (
            "request-extra-namespace",
            "a namespace added after the proof was made (audit H2: mozilla. for alice)",
            edit(a.clone(), |v| {
                v["namespaces"] = json!(["alice.", "mozilla."])
            }),
            Err(RequestError::BadProof),
        ),
        (
            "request-other-login",
            "the login changed after the proof was made",
            edit(a.clone(), |v| v["subject"]["login"] = json!("bob")),
            Err(RequestError::BadProof),
        ),
        (
            "request-other-name",
            "the display name changed after the proof was made",
            edit(a.clone(), |v| v["subject"]["name"] = json!("Bob Roe")),
            Err(RequestError::BadProof),
        ),
        (
            "request-other-github-id",
            "the GitHub id changed after the proof was made",
            edit(hub_copy(alice), |v| v["subject"]["github_id"] = json!(1002)),
            Err(RequestError::BadProof),
        ),
        (
            "request-other-kind",
            "a person's request relabelled as an organisation",
            edit(a.clone(), |v| v["kind"] = json!("org")),
            Err(RequestError::BadProof),
        ),
        (
            "request-other-encryption-key",
            "the X25519 key swapped for someone else's: the proof covers the encryption keys too",
            edit(a.clone(), |v| v["keys"]["x25519"] = other_x),
            Err(RequestError::BadProof),
        ),
        (
            "request-proof-damaged",
            "one character of the Ed25519 half of the proof changed",
            edit(a.clone(), |v| {
                let s = v["proof"]["ed25519"].as_str().unwrap_or_default();
                let flip = if s.starts_with('A') { "B" } else { "A" };
                v["proof"]["ed25519"] = json!(format!("{flip}{}", s.get(1..).unwrap_or_default()));
            }),
            Err(RequestError::BadProof),
        ),
        (
            "request-no-proof",
            "a request without its proof member",
            edit(a.clone(), |v| {
                if let Some(o) = v.as_object_mut() {
                    o.remove("proof");
                }
            }),
            Err(RequestError::Damaged),
        ),
        (
            "request-unknown-in-proof",
            "only the top level and subject.org may grow: an unknown field inside the proof is refused",
            edit(a.clone(), |v| v["proof"]["note"] = json!("x")),
            Err(RequestError::Damaged),
        ),
        (
            "request-not-issue",
            "an action other than issue",
            edit(a, |v| v["action"] = json!("revoke")),
            Err(RequestError::Unsupported),
        ),
    ])
}

/// Builds every request vector. Deterministic (the proofs are deterministic signatures).
pub fn generate() -> Result<Vec<RequestVector>, String> {
    let alice = made(
        "test-alice-request",
        Kind::Person,
        "alice",
        "Alice Doe",
        1001,
        true,
    );
    let org = made(
        "test-examplelabs-request",
        Kind::Org,
        "examplelabs",
        "Example Labs",
        2002,
        true,
    );
    let plain = made(
        "test-bob-request",
        Kind::Person,
        "bob",
        "Bob Roe",
        1003,
        false,
    );
    // The organisation is part of what the proof covers (audit H2): the requester names it,
    // the hub only checks the membership. Added afterwards, it breaks the proof.
    let mut added = alice.clone();
    added.subject.org = Some(OrgRef {
        github_id: 2002,
        login: "examplelabs".into(),
    });
    let mut member = made(
        "test-carol-request",
        Kind::Person,
        "carol",
        "Carol Poe",
        1004,
        true,
    );
    member.subject.org = added.subject.org.clone();
    let km = KeyMaterial::new(pki::seed("test-carol-request"), false);
    if let Some(c) = request::Claimed::of_request(&member) {
        member.proof = request::prove(&km.secret(), &c);
    }
    let mut cases = cases(&alice, &org, &plain, &member, &added)?;
    // Terms (lead decision "SIGNATURE LINES"): named in the request, covered by the proof.
    let mut termed = made(
        "test-dana-request",
        Kind::Person,
        "dana",
        "Dana Roe",
        1005,
        true,
    );
    termed.terms = Some(crate::vectors_ids::terms());
    let dk = KeyMaterial::new(pki::seed("test-dana-request"), false);
    if let Some(c) = request::Claimed::of_request(&termed) {
        termed.proof = request::prove(&dk.secret(), &c);
    }
    let tv = serde_json::to_value(&termed).map_err(|e| e.to_string())?;
    cases.push((
        "request-terms",
        "a request that names the trust terms (version and SHA-256 of the text): the proof covers them",
        tv.clone(),
        Ok(()),
    ));
    cases.push((
        "request-terms-changed",
        "the terms version changed after the proof was made: refused",
        edit(tv.clone(), |v| v["terms"]["version"] = json!(2)),
        Err(RequestError::BadProof),
    ));
    cases.push((
        "request-terms-removed",
        "the terms removed after the proof was made: refused",
        edit(tv, |v| {
            if let Some(o) = v.as_object_mut() {
                o.remove("terms");
            }
        }),
        Err(RequestError::BadProof),
    ));
    let mut out = Vec::new();
    for (name, about, value, want) in cases {
        let v = RequestVector {
            format: 1,
            name: name.into(),
            about: about.into(),
            request: text(&value),
            expect: RequestExpect {
                accepted: want.is_ok(),
                error: want.err().map(|e| e.as_str().to_owned()),
            },
        };
        let got = observe(&v);
        if got != v.expect {
            return Err(format!("{name}: wanted {:?}, got {got:?}", v.expect));
        }
        out.push(v);
    }
    // The hub's copy reads as exactly the request it was made from.
    let hub = request::parse(text(&hub_copy(&alice)).as_bytes()).map_err(|e| e.as_str())?;
    if hub != alice {
        return Err("the hub's copy does not read as the original request".into());
    }
    Ok(out)
}
