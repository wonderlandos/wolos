// SPDX-License-Identifier: BSD-2-Clause
//! Signature lines (lead decision 2026-11-04 "SIGNATURE LINES", shapes approved 03:12Z): every
//! signed body renders to one canonical, human-readable line, the same everywhere it is shown
//! (the Store sheet, `woltrust verify`, `wol trust show`, the hub). Fields are in angle brackets,
//! separated by one space; ids are always the full 128 hex (a display may shorten, the line never
//! does); dates are UTC `YYYY-MM-DD`; an absent field is `none`.
//!
//! ```text
//! root          <TEXT> <root> <wolos.org/trust/ID> valid-until <DATE> <ed25519+slh-dsa-sha2-128s>
//! intermediate  <LABEL> <issued-by: A | B> <wolos.org/trust/ID> valid-until <DATE> <ed25519+ml-dsa-65>
//! person        <LOGIN> <github:GITHUB_ID> <issued-by: WolOS Authors> <wolos.org/trust/ID>
//!               <namespaces: a.,b.> valid-until <DATE> <origin|trusted>[ <origin-cert: ID>]
//! organisation  <org/LOGIN> ... as a person
//! signer        <LABEL> <issued-by: WolOS Packages> <wolos.org/trust/ID> <namespaces: ...>
//!               valid-until <DATE> <wolos>
//! file sig      <WHO> <github:OWNER/REPO@TAG COMMIT7> <wolos.org/trust/ID>
//!               <wolos.org/terms/trust/vN> <sha512:HASH> <DATE> <origin|trusted|wolos>
//! file cert     <WolOS Index> <wolos.org/hub/APP/VERSION> <author: ID> <sha512:HASH>
//!               <checked DATE> <pass|flag> <rkdb:SERIAL>
//! witness       <WolOS Witness A|B> <sha512:HASH> <DATE> <wolos.org/trust/ID>
//! revocation    <ID> <revoked DATE> <REASON> <by: WolOS Index>
//!               <ID> <withdrawn DATE> <withdrawn-version> <by: WolOS Index>
//! ```
//! WHO is the login, `org/LOGIN` for an organisation, `ORG/LOGIN` for a person publishing from
//! their organisation's repository. A CA's name comes from what it may do (signed usage and
//! namespaces), not from its label: `WolOS Authors` (issues `*`), `WolOS Packages` (issues
//! `org.wolos.`), `WolOS Index` (signs the index); a root by its subject text. The TEST authority's
//! CAs read `TEST WolOS ...`. In a file signature, HASH is SHA-512 of the `SHA256` member (the
//! file id), DATE the origin's build date; in a file certificate, HASH is its id.
use crate::cert::{Cert, Class, Kind, Level, Usage};
use crate::origin::Origin;
use crate::revoked::Reason;

/// `YYYY-MM-DD` (UTC) of Unix seconds ([`wolcrypt::text::date`], so every line shares it).
pub fn date(t: u64) -> String {
    wolcrypt::text::date(t)
}

fn test(c: &Cert) -> &'static str {
    let t = c
        .body
        .subject
        .label
        .as_deref()
        .is_some_and(|l| l.starts_with(crate::TEST_LABEL));
    if t { "TEST " } else { "" }
}

/// The name a CA is shown by: a root by its subject text (else its label), an intermediate by
/// what it may do.
pub fn ca_name(c: &Cert) -> String {
    let b = &c.body;
    if b.kind == Kind::Root {
        return b
            .subject
            .text
            .clone()
            .or_else(|| b.subject.label.clone())
            .unwrap_or_else(|| b.serial.clone());
    }
    let role = if b.usage.contains(&Usage::SignCerts) && b.namespaces.iter().any(|n| n == "*") {
        "WolOS Authors"
    } else if b.usage.contains(&Usage::SignCerts) {
        "WolOS Packages"
    } else if b.usage.contains(&Usage::SignIndex) {
        "WolOS Index"
    } else if b.usage.contains(&Usage::Witness) {
        let l = b.subject.label.as_deref().unwrap_or_default();
        if l.ends_with("-b") {
            "WolOS Witness B"
        } else {
            "WolOS Witness A"
        }
    } else {
        return b.subject.label.clone().unwrap_or_else(|| b.serial.clone());
    };
    format!("{}{role}", test(c))
}

fn class_word(c: &Cert) -> &'static str {
    match (c.body.level, c.body.class()) {
        (Level::Wolos, _) => "wolos",
        (_, Some(Class::Trusted)) => "trusted",
        _ => "origin",
    }
}

fn trust_url(id: &str) -> String {
    format!("wolos.org/trust/{id}")
}

/// The line of a certificate. `issuers` are the certificates that signed it when known: the
/// issuer first, then the other root of an intermediate.
pub fn cert(c: &Cert, issuers: &[&Cert]) -> String {
    let b = &c.body;
    let until = date(b.not_after);
    let by = if issuers.is_empty() {
        b.issuer.clone()
    } else {
        issuers
            .iter()
            .map(|i| ca_name(i))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let id = trust_url(&b.serial);
    let ns = b.namespaces.join(",");
    let label = b.subject.label.clone().unwrap_or_else(|| "none".into());
    match b.kind {
        Kind::Root => format!(
            "<{}> <root> <{id}> valid-until <{until}> <ed25519+slh-dsa-sha2-128s>",
            ca_name(c)
        ),
        Kind::Intermediate => {
            format!("<{label}> <issued-by: {by}> <{id}> valid-until <{until}> <ed25519+ml-dsa-65>")
        }
        Kind::Signer => format!(
            "<{label}> <issued-by: {by}> <{id}> <namespaces: {ns}> valid-until <{until}> <wolos>"
        ),
        Kind::Person | Kind::Org => {
            let login = b.subject.login.clone().unwrap_or_else(|| "none".into());
            let who = if b.kind == Kind::Org {
                format!("org/{login}")
            } else {
                login
            };
            let gh = b
                .subject
                .github_id
                .map_or_else(|| "none".into(), |g| g.to_string());
            let mut line = format!(
                "<{who}> <github:{gh}> <issued-by: {by}> <{id}> <namespaces: {ns}> valid-until <{until}> <{}>",
                class_word(c)
            );
            if let Some(o) = &b.origin_cert {
                line.push_str(&format!(" <origin-cert: {o}>"));
            }
            line
        }
    }
}

/// Who signed a file, as the line shows it.
fn who(signer: &Cert, origin: Option<&Origin>) -> String {
    let s = &signer.body.subject;
    let login = s
        .login
        .clone()
        .or_else(|| s.label.clone())
        .unwrap_or_else(|| "none".into());
    if signer.body.kind == Kind::Org {
        return format!("org/{login}");
    }
    match (&s.org, origin.and_then(Origin::owner)) {
        (Some(org), Some(owner)) if owner.eq_ignore_ascii_case(&org.login) => {
            format!("{}/{login}", org.login)
        }
        _ => login,
    }
}

/// The line of a file signature: `signer` signed `content` (the `SHA256` member) with `origin`.
pub fn signature(signer: &Cert, origin: Option<&Origin>, content: &[u8]) -> String {
    let gh = origin.map_or_else(
        || "github:none".to_owned(),
        |o| {
            let path = o
                .repo
                .strip_prefix("https://github.com/")
                .unwrap_or(&o.repo);
            format!(
                "github:{path}@{} {}",
                o.tag,
                &o.commit[..o.commit.len().min(7)]
            )
        },
    );
    let terms = signer
        .body
        .terms
        .as_ref()
        .map_or_else(|| "none".to_owned(), |t| format!("v{}", t.version));
    let hash = wolcrypt::hash::hex(&wolcrypt::id::of_file(content));
    let when = origin.map_or_else(|| "none".to_owned(), |o| date(o.built));
    format!(
        "<{}> <{gh}> <{}> <wolos.org/terms/trust/{terms}> <sha512:{hash}> <{when}> <{}>",
        who(signer, origin),
        trust_url(&signer.body.serial),
        class_word(signer)
    )
}

/// The line of a witness signature: `<WolOS Witness A|B> <sha512:HASH> <DATE> <wolos.org/trust/ID>`.
pub fn witness(signer: &Cert, file: &[u8], at: u64) -> String {
    format!(
        "<{}> <sha512:{}> <{}> <{}>",
        ca_name(signer),
        wolcrypt::hash::hex(&wolcrypt::hash::sha512(file)),
        date(at),
        trust_url(&signer.body.serial)
    )
}

/// The line of a file certificate signed by `signer` (the Index; None when not known).
pub fn file_certificate(b: &crate::filecert::Body, signer: Option<&Cert>) -> String {
    let by = signer.map_or_else(|| "WolOS Index".to_owned(), ca_name);
    let verdict = match b.verdict {
        crate::report::Verdict::Pass => "pass",
        crate::report::Verdict::Flag => "flag",
        crate::report::Verdict::Block => "block",
        crate::report::Verdict::NotChecked => "not-checked",
    };
    let rk = b
        .rkdb_serial
        .map_or_else(|| "none".to_owned(), |s| s.to_string());
    format!(
        "<{by}> <wolos.org/hub/{}/{}> <author: {}> <sha512:{}> <checked {}> <{verdict}> <rkdb:{rk}>",
        b.id,
        b.version,
        b.cert,
        b.serial,
        date(b.checked_at)
    )
}

/// The line of one revocation entry, withdrawn by `by` (the Index's or a root's [`ca_name`]).
pub fn revocation(id: &str, at: u64, reason: Reason, by: &str) -> String {
    if reason == Reason::WithdrawnVersion {
        format!(
            "<{id}> <withdrawn {}> <withdrawn-version> <by: {by}>",
            date(at)
        )
    } else {
        format!(
            "<{id}> <revoked {}> <{}> <by: {by}>",
            date(at),
            reason.as_str()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_utc_civil_days() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(date(crate::pki::NOW), "2026-02-02");
        assert_eq!(date(4_102_444_800), "2100-01-01");
    }

    #[test]
    fn revocation_entries_render_their_lines() {
        assert_eq!(
            revocation("ab", 0, Reason::KeyCompromise, "WolOS Index"),
            "<ab> <revoked 1970-01-01> <key-compromise> <by: WolOS Index>"
        );
        assert_eq!(
            revocation("ab", 0, Reason::WithdrawnVersion, "WolOS Index"),
            "<ab> <withdrawn 1970-01-01> <withdrawn-version> <by: WolOS Index>"
        );
    }
}
