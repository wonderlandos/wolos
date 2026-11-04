// SPDX-License-Identifier: BSD-2-Clause
//! The check of ADR 0046, in the order the ADR gives: bounded parse of the signature and `CHAIN`;
//! the signer's usage and namespace; the hybrid signature; each certificate against its issuer
//! (the issuer may issue that kind and those namespaces) up to a pinned root; revocation in every
//! list held; validity at the clock; freshness of the newest list. Pure: the caller passes `now`.
use crate::cert::{self, Cert, ChainError, Kind, Level};
use crate::crypto::{self, Purpose, Wsig};
use crate::report::{CertInfo, Finding, Report, Trust, Verdict};
use crate::revoked::{Freshness, Reason, RevList};
use crate::store::Store;

/// Most links walked from a signer to a root (leaf, intermediate, root, with room to spare).
const MAX_PATH: usize = 5;

/// The machine-readable reason behind a trust value; the string is stable and replayed by the
/// vectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    /// Verified (or made by WolOS), the revocation list is fresh.
    Ok,
    /// As [`Code::Ok`], but the revocation list ran out up to 30 days ago.
    ListStale,
    /// No signature member.
    Unsigned,
    /// A certificate in the chain has expired.
    Expired,
    /// A certificate in the chain starts after the clock.
    NotYetValid,
    /// The revocation list is more than 30 days past its end, or missing.
    ListTooOld,
    /// The signature file cannot be read.
    SignatureMalformed,
    /// The `CHAIN` member cannot be read.
    ChainMalformed,
    /// More than four certificates in `CHAIN`.
    ChainTooLong,
    /// `CHAIN` is larger than 64 KiB.
    ChainTooLarge,
    /// The certificate named by the signature is not in the chain or the store.
    SignerUnknown,
    /// A certificate's issuer is neither in the chain, nor the store, nor a pinned root.
    IssuerUnknown,
    /// A signature half does not verify.
    BadSignature,
    /// An issuer may not issue that kind or those namespaces.
    BadIssuer,
    /// The signer's usage does not cover this kind of file.
    WrongUsage,
    /// The file's id is outside the signer's namespaces.
    WrongNamespace,
    /// Withdrawn: key exposed.
    RevokedKeyCompromise,
    /// Withdrawn: abuse.
    RevokedAbuse,
    /// Withdrawn: replaced, and the file is not known to predate it.
    RevokedSuperseded,
    /// Withdrawn: owner stopped, and the file is not known to predate it.
    RevokedCeased,
}

impl Code {
    /// Stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Code::Ok => "ok",
            Code::ListStale => "list-stale",
            Code::Unsigned => "unsigned",
            Code::Expired => "expired",
            Code::NotYetValid => "not-yet-valid",
            Code::ListTooOld => "list-too-old",
            Code::SignatureMalformed => "signature-malformed",
            Code::ChainMalformed => "chain-malformed",
            Code::ChainTooLong => "chain-too-long",
            Code::ChainTooLarge => "chain-too-large",
            Code::SignerUnknown => "signer-unknown",
            Code::IssuerUnknown => "issuer-unknown",
            Code::BadSignature => "bad-signature",
            Code::BadIssuer => "bad-issuer",
            Code::WrongUsage => "wrong-usage",
            Code::WrongNamespace => "wrong-namespace",
            Code::RevokedKeyCompromise => "revoked-key-compromise",
            Code::RevokedAbuse => "revoked-abuse",
            Code::RevokedSuperseded => "revoked-superseded",
            Code::RevokedCeased => "revoked-ceased",
        }
    }

    /// The sentence a person reads for this code (None when nothing needs saying).
    pub fn sentence(self) -> Option<&'static str> {
        Some(match self {
            Code::Ok => return None,
            Code::ListStale => {
                "This computer has not checked for withdrawn certificates for a while."
            }
            Code::Unsigned => {
                "Nobody vouches for this file. WolOS can't tell who made it or whether someone changed it on the way."
            }
            Code::Expired => {
                "The certificate this file was signed with has expired, so nobody vouches for it any more."
            }
            Code::NotYetValid => {
                "This computer's clock is earlier than the start of the certificate this file was signed with."
            }
            Code::ListTooOld => {
                "This computer has not checked for withdrawn certificates for more than 30 days, so the signature can't be counted."
            }
            Code::SignatureMalformed
            | Code::ChainMalformed
            | Code::ChainTooLong
            | Code::ChainTooLarge => "The signature on this file is damaged.",
            Code::SignerUnknown | Code::IssuerUnknown => {
                "The signature on this file comes from a certificate WolOS doesn't know."
            }
            Code::BadSignature => {
                "The signature on this file does not check out. It may have been changed on the way."
            }
            Code::BadIssuer => {
                "The certificate this file was signed with was not issued by someone allowed to issue it."
            }
            Code::WrongUsage => {
                "The certificate this file was signed with is not meant for this kind of file."
            }
            Code::WrongNamespace => {
                "This file's id is outside the names its certificate is allowed to sign."
            }
            Code::RevokedKeyCompromise => {
                "WolOS no longer trusts the certificate this file was signed with, because its key was exposed."
            }
            Code::RevokedAbuse => {
                "WolOS no longer trusts the certificate this file was signed with, because it was misused."
            }
            Code::RevokedSuperseded => {
                "WolOS no longer trusts the certificate this file was signed with, because it was replaced."
            }
            Code::RevokedCeased => {
                "WolOS no longer trusts the certificate this file was signed with, because its owner stopped publishing."
            }
        })
    }

    fn revoked(r: Reason) -> Code {
        match r {
            Reason::KeyCompromise => Code::RevokedKeyCompromise,
            Reason::Abuse => Code::RevokedAbuse,
            Reason::Superseded => Code::RevokedSuperseded,
            Reason::Ceased => Code::RevokedCeased,
        }
    }
}

/// Everything one check needs. `wsig` and `chain` are the bytes of the `SHA256.wsig` and `CHAIN`
/// members (or the signature next to an index or list).
#[derive(Clone, Copy, Debug)]
pub struct Input<'a> {
    /// What the signature is for.
    pub purpose: Purpose,
    /// The signed bytes (the `SHA256` member, the index, the list).
    pub content: &'a [u8],
    /// The app id from `Info.json`; needed for packages, plugins and extensions.
    pub app_id: Option<&'a str>,
    /// The signature file, or None for an unsigned file.
    pub wsig: Option<&'a [u8]>,
    /// The `CHAIN` member, or None.
    pub chain: Option<&'a [u8]>,
    /// When the signed index first saw this exact file (already verified by the caller). It lets
    /// an expired certificate still count, and a `superseded` or `ceased` one too, for files that
    /// predate it.
    pub first_seen: Option<u64>,
    /// The clock, Unix seconds.
    pub now: u64,
}

/// The result of a check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// What the signature is worth.
    pub trust: Trust,
    /// Why.
    pub code: Code,
    /// The signing certificate, when one was found.
    pub cert: Option<CertInfo>,
    /// True when the verdict stands on a revocation list past its end (still inside 30 days).
    pub stale: bool,
    /// `expires` of the newest list held, for "not checked since" lines.
    pub list_expires: Option<u64>,
}

impl Outcome {
    fn bare(trust: Trust, code: Code) -> Outcome {
        Outcome {
            trust,
            code,
            cert: None,
            stale: false,
            list_expires: None,
        }
    }

    /// The I7 report for this outcome, with the hub's `verdict` and `findings` (the trust sentence
    /// is appended as a finding unless the file is plainly trusted).
    pub fn report(&self, verdict: Verdict, mut findings: Vec<Finding>) -> Report {
        if let Some(sentence) = self.code.sentence() {
            findings.push(Finding {
                rule: format!("trust/{}", self.code.as_str()),
                sentence: sentence.to_owned(),
                evidence: self
                    .cert
                    .as_ref()
                    .map(|c| c.serial.clone())
                    .unwrap_or_default(),
            });
        }
        Report {
            verdict,
            findings,
            trust: self.trust,
            cert: self.cert.clone(),
        }
    }
}

fn info(c: &Cert) -> CertInfo {
    CertInfo {
        serial: c.body.serial.clone(),
        subject: c.body.subject.clone(),
        level: c.body.level,
    }
}

fn may_issue(issuer: &Cert, subject: &Cert) -> bool {
    let kind_ok = match issuer.body.kind {
        Kind::Root => subject.body.kind == Kind::Intermediate,
        Kind::Intermediate => matches!(subject.body.kind, Kind::Person | Kind::Org | Kind::Signer),
        Kind::Person | Kind::Org | Kind::Signer => false,
    };
    kind_ok
        && issuer.body.usage.contains(&cert::Usage::SignCerts)
        && subject
            .body
            .namespaces
            .iter()
            .all(|n| cert::covers(&issuer.body.namespaces, n))
}

fn lookup<'a>(serial: &str, chain: &'a [Cert], store: &'a Store) -> Option<&'a Cert> {
    chain
        .iter()
        .chain(store.intermediates.iter())
        .chain(store.roots.iter())
        .find(|c| c.body.serial == serial)
}

/// Walks `signer` up to a pinned root, checking each certificate's signature and its issuer's
/// right to issue it. Returns the path, signer first, root last. No time or revocation check.
pub(crate) fn path<'a>(
    signer: &'a Cert,
    rest: &'a [Cert],
    store: &'a Store,
) -> Result<Vec<&'a Cert>, Code> {
    let mut out = vec![signer];
    let mut cur = signer;
    while cur.body.kind != Kind::Root {
        if out.len() >= MAX_PATH {
            return Err(Code::BadIssuer);
        }
        let issuer = lookup(&cur.body.issuer, rest, store).ok_or(Code::IssuerUnknown)?;
        if out.iter().any(|c| c.body.serial == issuer.body.serial) || !may_issue(issuer, cur) {
            return Err(Code::BadIssuer);
        }
        let msg = crypto::message(Purpose::Cert, &cur.body_bytes);
        crypto::verify_hybrid(&issuer.keys, &msg, &cur.sig).map_err(|_| Code::BadSignature)?;
        out.push(issuer);
        cur = issuer;
    }
    // A root counts only if the store pins it by its exact keys.
    if !store.roots.iter().any(|r| r.keys == cur.keys) {
        return Err(Code::IssuerUnknown);
    }
    Ok(out)
}

/// The revocation that applies to `path` for a file first seen at `first_seen`, in path order.
pub(crate) fn revocation(
    path: &[&Cert],
    lists: &[RevList],
    first_seen: Option<u64>,
) -> Option<Code> {
    let mut soft = None;
    for c in path {
        for l in lists {
            for e in l.list.entries.iter().filter(|e| e.serial == c.body.serial) {
                let leaf_kind = matches!(c.body.kind, Kind::Person | Kind::Org | Kind::Signer);
                let allowed = leaf_kind || (l.signer_is_root && l.signer != c.body.serial);
                if !allowed {
                    continue;
                }
                if e.reason.is_hard() {
                    return Some(Code::revoked(e.reason));
                }
                if first_seen.is_none_or(|t| t >= e.at) && soft.is_none() {
                    soft = Some(Code::revoked(e.reason));
                }
            }
        }
    }
    soft
}

/// The first time problem on `path`: expired or not yet valid at `now` (a file the index saw while
/// the certificate was valid is not expired).
fn time_problem(path: &[&Cert], now: u64, first_seen: Option<u64>) -> Option<Code> {
    let mut late = false;
    for c in path {
        if c.valid_at(now) {
            continue;
        }
        if now < c.body.not_before {
            return Some(Code::NotYetValid);
        }
        if !first_seen.is_some_and(|t| c.valid_at(t)) {
            late = true;
        }
    }
    late.then_some(Code::Expired)
}

fn split_chain_error(e: ChainError) -> Code {
    match e {
        ChainError::TooLarge => Code::ChainTooLarge,
        ChainError::TooLong => Code::ChainTooLong,
        ChainError::Malformed(_) => Code::ChainMalformed,
    }
}

/// Checks one signed file against the store at `input.now`. Never panics on any input.
pub fn verify(store: &Store, input: &Input) -> Outcome {
    let list_expires = store.best_list().map(|l| l.list.expires);
    let mut out = verify_inner(store, input);
    out.list_expires = list_expires;
    out
}

fn verify_inner(store: &Store, input: &Input) -> Outcome {
    let invalid = |code| Outcome::bare(Trust::Invalid, code);
    let Some(wsig_bytes) = input.wsig else {
        return Outcome::bare(Trust::Community, Code::Unsigned);
    };
    let Ok(wsig) = Wsig::parse(wsig_bytes) else {
        return invalid(Code::SignatureMalformed);
    };
    let Some(sig) = wsig.sig() else {
        return invalid(Code::SignatureMalformed);
    };
    let chain = match input.chain.map(cert::parse_chain).transpose() {
        Ok(c) => c.unwrap_or_default(),
        Err(e) => return invalid(split_chain_error(e)),
    };
    let signer = match chain.first() {
        Some(leaf) if leaf.body.serial == wsig.cert => Some(leaf),
        Some(_) => None,
        None => store
            .intermediates
            .iter()
            .chain(store.roots.iter())
            .find(|c| c.body.serial == wsig.cert),
    };
    let Some(signer) = signer else {
        return invalid(Code::SignerUnknown);
    };
    let who = Some(info(signer));
    let bad = |code| Outcome {
        cert: who.clone(),
        ..invalid(code)
    };
    if !signer.body.usage.contains(&input.purpose.usage()) {
        return bad(Code::WrongUsage);
    }
    if matches!(
        input.purpose,
        Purpose::Package | Purpose::Plugin | Purpose::Extension
    ) {
        let ok = input.app_id.is_some_and(|id| {
            signer
                .body
                .namespaces
                .iter()
                .any(|p| id.starts_with(p.as_str()))
        });
        if !ok {
            return bad(Code::WrongNamespace);
        }
    }
    let msg = crypto::message(input.purpose, input.content);
    if crypto::verify_hybrid(&signer.keys, &msg, &sig).is_err() {
        return bad(Code::BadSignature);
    }
    let rest = chain.get(1..).unwrap_or_default();
    let path = match path(signer, rest, store) {
        Ok(p) => p,
        Err(code) => return bad(code),
    };
    let make = |trust, code, stale| Outcome {
        trust,
        code,
        cert: who.clone(),
        stale,
        list_expires: None,
    };
    if let Some(code) = revocation(&path, &store.lists, input.first_seen) {
        return make(Trust::Withdrawn, code, false);
    }
    if let Some(code) = time_problem(&path, input.now, input.first_seen) {
        return make(Trust::Community, code, false);
    }
    let level = match signer.body.level {
        Level::Wolos => Trust::Wolos,
        Level::Verified => Trust::Verified,
    };
    match store.freshness(input.now) {
        Freshness::Fresh => make(level, Code::Ok, false),
        Freshness::Stale => make(level, Code::ListStale, true),
        Freshness::TooOld | Freshness::Missing => make(Trust::Community, Code::ListTooOld, false),
    }
}
