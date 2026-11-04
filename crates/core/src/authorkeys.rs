// SPDX-License-Identifier: BSD-2-Clause
//! An author's own keys and the certificates this account holds (ADR 0046, "Key storage"):
//! `<data home>/wolos/trust/keys/<name>.wolkey`, each locked with the author's passphrase and never
//! written in the clear, and `<data home>/wolos/trust/certs/<serial>.wolcert`, the account's own
//! certificates and those of people it seals files to. A key is named by its key id until its
//! certificate arrives, then by the certificate's serial. Pure: the controller does the I/O.
use std::path::{Path, PathBuf};
use woltrust::cert::{Cert, Kind, Subject};
use woltrust::key::KeyMaterial;
use woltrust::request::{self, Request};

/// Below the data home.
pub const TRUST_DIR: &str = "wolos/trust";
/// Extension of a locked key file.
pub const KEY_EXT: &str = "wolkey";
/// Extension of a certificate file.
pub const CERT_EXT: &str = "wolcert";
/// Default validity asked for, in months.
pub const MONTHS: u32 = 12;

/// `<data home>/wolos/trust/keys`.
pub fn keys_dir(data_home: &Path) -> PathBuf {
    data_home.join(TRUST_DIR).join("keys")
}

/// `<data home>/wolos/trust/certs`.
pub fn certs_dir(data_home: &Path) -> PathBuf {
    data_home.join(TRUST_DIR).join("certs")
}

/// The key file of `name` (a key id or a serial).
pub fn key_path(data_home: &Path, name: &str) -> PathBuf {
    keys_dir(data_home).join(format!("{name}.{KEY_EXT}"))
}

/// The certificate file of `serial`.
pub fn cert_path(data_home: &Path, serial: &str) -> PathBuf {
    certs_dir(data_home).join(format!("{serial}.{CERT_EXT}"))
}

/// The request file for new keys: the hub-index request shape with every public key (encryption
/// keys included) and the proof of possession.
pub fn request(
    km: &KeyMaterial,
    kind: Kind,
    subject: Subject,
    namespaces: Vec<String>,
    now: u64,
) -> Result<Request, String> {
    if !matches!(kind, Kind::Person | Kind::Org) {
        return Err("a request is for a person or an organisation".into());
    }
    subject
        .github_id
        .ok_or("a request needs the numeric GitHub id")?;
    if subject.login.is_none() || subject.name.is_none() {
        return Err("a request needs the GitHub login and a name".into());
    }
    if namespaces.is_empty()
        || namespaces.len() > 8
        || !namespaces
            .iter()
            .all(|n| woltrust::cert::is_prefix(n) && woltrust::cert::is_ordinary(n))
    {
        return Err("namespaces are 1 to 8 prefixes like alice. (outside org.wolos.)".into());
    }
    let mut req = Request {
        format: 1,
        action: "issue".into(),
        kind,
        subject,
        namespaces,
        keys: km.keys_text(),
        proof: request::Proof {
            ed25519: String::new(),
            second: String::new(),
        },
        months: MONTHS,
        requested_at: now,
    };
    let claimed = request::Claimed::of_request(&req).ok_or("a request needs a GitHub id")?;
    req.proof = request::prove(&km.secret(), &claimed);
    Ok(req)
}

/// True when `cert` was issued for the keys of `km` (signing and encryption alike).
pub fn is_mine(cert: &Cert, km: &KeyMaterial) -> bool {
    cert.body.keys == km.keys_text()
}

/// The key id a certificate's keys were stored under before it was issued.
pub fn key_id_of(cert: &Cert) -> String {
    request::key_id(&cert.keys.ed25519)
}

/// One line about a certificate for `wol cert list` and `show`.
pub fn describe(c: &Cert, now: u64) -> String {
    let s = &c.body.subject;
    let who = match (&s.name, &s.login, &s.label) {
        (Some(n), Some(l), _) => format!("{n} ({l} on GitHub)"),
        (_, _, Some(l)) => l.clone(),
        _ => "unnamed".into(),
    };
    let state = if now >= c.body.not_after {
        "expired"
    } else if now < c.body.not_before {
        "not yet valid"
    } else {
        "valid"
    };
    let enc = if c.enc.is_some() {
        ", can receive sealed files"
    } else {
        ""
    };
    format!(
        "{} {who}, {}, {state}{enc}",
        c.body.serial,
        c.body.namespaces.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> Subject {
        Subject {
            github_id: Some(1001),
            login: Some("alice".into()),
            name: Some("Alice Doe".into()),
            label: None,
            org: None,
            text: None,
        }
    }

    #[test]
    fn a_request_carries_every_key_and_a_proof_the_ca_can_check() {
        let km = KeyMaterial::new([2; 32], false);
        let r = request(&km, Kind::Person, alice(), vec!["alice.".into()], 5).unwrap();
        assert!(r.keys.x25519.is_some() && r.keys.ml_kem_768.is_some());
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"ml-kem-768\"") && json.contains("\"action\":\"issue\""));
        assert!(!json.contains("seed"));
        let bad_ns = request(&km, Kind::Person, alice(), vec!["org.wolos.x.".into()], 5);
        assert!(bad_ns.is_err());
        assert!(request(&km, Kind::Signer, alice(), vec!["alice.".into()], 5).is_err());
    }

    #[test]
    fn the_store_paths_live_under_the_data_home() {
        let d = Path::new("/h/.local/share");
        assert_eq!(
            key_path(d, "0123456789abcdef"),
            Path::new("/h/.local/share/wolos/trust/keys/0123456789abcdef.wolkey")
        );
        assert!(cert_path(d, "a").ends_with("trust/certs/a.wolcert"));
    }
}
