// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol doctor tls` (ADR 0055): what each TLS stack WolOS ships negotiates right now, with a
//! post-quantum test server (it always has the hybrid group, so a classical answer there is our
//! side's fault) and with the update server. One check per stack: OpenSSL through curl (what
//! pacman, the update check and DNS over TLS use), GnuTLS through gnutls-cli, WolOS's own Rust
//! stack (`wol_core::net::probe`). Asked for by name only: it connects to servers outside the
//! computer, so plain `wol doctor` (and the Doctor app) never runs it. Whether the system policy is
//! in place and what the browser is told only reads files, so those two checks ([`file_checks`])
//! run in the `security` area every time.
use super::{Check, Status, check};
use crate::run::Env;
use std::path::Path;
use wol_core::net::{self, Negotiated};

/// The area's name.
pub const AREA: &str = "tls";

const OPENSSL_CNF: &str = "/etc/ssl/openssl.cnf";
const GNUTLS_CONFIG: &str = "/etc/gnutls/config";
const BROWSER_POLICY: &str = "/etc/chromium/policies/managed/wolos.json";

/// The area the file checks join: they reach nothing outside the computer, so plain `wol doctor`
/// and the Doctor app show a system whose TLS went classical (`tls-policy` left a file alone).
pub const FILE_AREA: &str = "security";

/// The checks that only read files, for the always-run [`FILE_AREA`]: the system policy is in
/// place, and the browser's policy has its TLS rows.
pub fn file_checks(env: &mut dyn Env) -> Vec<Check> {
    vec![policy(env), browser(env)]
}

/// Every check of the on-ask area, in order: the network probes.
pub fn checks(env: &mut dyn Env) -> Vec<Check> {
    vec![
        stack(env, "openssl", "Updates and system tools", &|env, h| {
            curl(env, h)
        }),
        stack(env, "gnutls", "GTK apps and wget", &|env, h| gnutls(env, h)),
        stack(env, "wolos", "App Store and plugins", &|env, h| {
            env.tls_probe(h)
        }),
    ]
}

/// The system policy is what OpenSSL and GnuTLS read (`files/usr/lib/wolos/tls-policy`).
fn policy(env: &mut dyn Env) -> Check {
    let title = "System TLS policy";
    let ossl = env.read_file(Path::new(OPENSSL_CNF)).unwrap_or_default();
    let gtls = env.read_file(Path::new(GNUTLS_CONFIG)).unwrap_or_default();
    let o = ossl
        .lines()
        .any(|l| l.trim() == "ssl_conf = wolos_ssl_conf")
        && ossl
            .lines()
            .any(|l| l.trim() == ".include /usr/share/wolos/tls/openssl.cnf");
    let g = gtls
        .lines()
        .any(|l| l.trim() == format!("default-priority-string = {}", net::GNUTLS_PRIORITY));
    let (status, detail) = match (o, g) {
        (true, true) => (
            Status::Ok,
            "OpenSSL and GnuTLS offer the post-quantum group first, TLS 1.2 or newer, modern ciphers only.".to_string(),
        ),
        (o, g) => {
            let missing: Vec<&str> = [(!o).then_some("OpenSSL"), (!g).then_some("GnuTLS")]
                .into_iter()
                .flatten()
                .collect();
            (
                Status::Fail,
                format!(
                    "The WolOS TLS policy is not applied to {}: its settings file has settings of its own, or the policy was never applied. Reinstalling wolos-system applies it again.",
                    missing.join(" and ")
                ),
            )
        }
    };
    check(FILE_AREA, "tls-policy", title, status, &detail)
}

/// curl over the system OpenSSL, as the update check calls it (no curlrc, https only).
fn curl(env: &mut dyn Env, host: &str) -> Result<Negotiated, String> {
    let url = format!("https://{host}/");
    let argv = [
        "curl",
        "--disable",
        "-sv",
        "--head",
        "-o",
        "/dev/null",
        "--max-time",
        "15",
        "--proto",
        "=https",
        &url,
    ];
    match env.capture_status(&argv) {
        None => Err("curl did not run".into()),
        Some((_, _, err)) => net::parse_curl(&err).ok_or_else(|| reason(&err)),
    }
}

/// gnutls-cli with the system's default priority (the policy's `default-priority-string`).
fn gnutls(env: &mut dyn Env, host: &str) -> Result<Negotiated, String> {
    match env.capture_status(&["gnutls-cli", host]) {
        None => Err("gnutls-cli did not run".into()),
        Some((_, out, err)) => {
            net::parse_gnutls(&out).ok_or_else(|| reason(&format!("{out}\n{err}")))
        }
    }
}

/// The last line a tool printed about why, for the detail.
fn reason(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(|l| l.trim_start_matches("* ").chars().take(120).collect())
        .unwrap_or_else(|| "no answer".into())
}

fn words(n: &Negotiated) -> String {
    let v = n.version.replace("TLSv", "TLS ");
    if n.post_quantum() {
        format!("{v}, post-quantum ({}), {}", n.group, n.cipher)
    } else {
        format!("{v}, classical ({}), {}", n.group, n.cipher)
    }
}

/// One stack against [`net::PROBE_HOSTS`]: fail when the post-quantum test server got a classical
/// exchange or an old version, warn when the update server did, skipped when neither answered.
fn stack(
    env: &mut dyn Env,
    name: &str,
    title: &str,
    probe: &dyn Fn(&mut dyn Env, &str) -> Result<Negotiated, String>,
) -> Check {
    let mut parts = Vec::new();
    let mut status = Status::Ok;
    let mut reached = 0;
    for (i, host) in net::PROBE_HOSTS.iter().enumerate() {
        match probe(env, host) {
            Ok(n) => {
                reached += 1;
                let old = !matches!(n.version.as_str(), "TLSv1.3" | "TLSv1.2");
                if old || (i == 0 && !n.post_quantum()) {
                    status = Status::Fail;
                } else if !n.post_quantum() && status == Status::Ok {
                    status = Status::Warn;
                }
                parts.push(format!("{host}: {}", words(&n)));
            }
            Err(e) => parts.push(format!("{host}: not reached ({e})")),
        }
    }
    if reached == 0 {
        status = Status::Skipped;
    }
    check(AREA, name, title, status, &(parts.join(". ") + "."))
}

/// The browser negotiates on its own (Chromium offers X25519MLKEM768 to every site); this says
/// whether the policy file that pins ECH and its DNS records is in place.
fn browser(env: &mut dyn Env) -> Check {
    let title = "Atlas browser";
    let text = env.read_file(Path::new(BROWSER_POLICY)).unwrap_or_default();
    let p: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let rows = wol_core::theme::helium::tls_policies();
    let missing: Vec<&String> = rows
        .iter()
        .filter(|(k, v)| p.get(k.as_str()) != Some(v))
        .map(|(k, _)| k)
        .collect();
    if missing.is_empty() {
        check(
            FILE_AREA,
            "browser-tls",
            title,
            Status::Ok,
            "Offers the post-quantum group to every site; Encrypted Client Hello and http-to-https upgrades are on (set by policy, measured in the browser, not here).",
        )
    } else {
        check(
            FILE_AREA,
            "browser-tls",
            title,
            Status::Warn,
            &format!(
                "The browser policy lacks {}; the browser still offers the post-quantum group.",
                missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::FakeEnv;

    const PQ_CURL: &str = "* SSL connection using TLSv1.3 / TLS_AES_256_GCM_SHA384 / X25519MLKEM768 / id-ecPublicKey\n";
    const CLASSIC_CURL: &str =
        "* SSL connection using TLSv1.3 / TLS_AES_256_GCM_SHA384 / x25519 / id-ecPublicKey\n";

    fn curl_line(host: &str) -> String {
        format!(
            "curl --disable -sv --head -o /dev/null --max-time 15 --proto =https https://{host}/"
        )
    }

    fn env_with(curl_err: [&str; 2]) -> FakeEnv {
        let mut env = FakeEnv::default();
        for (h, e) in net::PROBE_HOSTS.iter().zip(curl_err) {
            env.statuses.push((curl_line(h), 0, e.to_string()));
            env.captures.push((curl_line(h), String::new()));
        }
        env
    }

    fn by_id<'a>(c: &'a [Check], id: &str) -> &'a Check {
        c.iter().find(|c| c.id == id).unwrap()
    }

    #[test]
    fn post_quantum_everywhere_is_ok_and_says_what_was_negotiated() {
        let mut env = env_with([PQ_CURL, PQ_CURL]);
        let c = checks(&mut env);
        let o = by_id(&c, "tls.openssl");
        assert_eq!(o.status, Status::Ok, "{o:?}");
        assert!(
            o.detail.starts_with("pq.cloudflareresearch.com: TLS 1.3, post-quantum (X25519MLKEM768), TLS_AES_256_GCM_SHA384."),
            "{}",
            o.detail
        );
        // The fake's own Rust probe answers post-quantum for every host.
        assert_eq!(by_id(&c, "tls.wolos").status, Status::Ok);
        let ids: Vec<&str> = c.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["tls.openssl", "tls.gnutls", "tls.wolos"]);
        let f: Vec<String> = file_checks(&mut env).into_iter().map(|c| c.id).collect();
        assert_eq!(f, ["security.tls-policy", "security.browser-tls"]);
    }

    #[test]
    fn a_classical_answer_from_the_pq_server_fails_and_from_the_update_server_warns() {
        let mut env = env_with([CLASSIC_CURL, PQ_CURL]);
        assert_eq!(by_id(&checks(&mut env), "tls.openssl").status, Status::Fail);
        let mut env = env_with([PQ_CURL, CLASSIC_CURL]);
        let c = checks(&mut env);
        let o = by_id(&c, "tls.openssl");
        assert_eq!(o.status, Status::Warn);
        assert!(
            o.detail
                .contains("cdn.wolos.org: TLS 1.3, classical (X25519)"),
            "{}",
            o.detail
        );
    }

    #[test]
    fn no_connection_is_skipped_with_the_reason() {
        let mut env = env_with([
            "* Could not resolve host: pq.cloudflareresearch.com\n",
            "* Could not resolve host: cdn.wolos.org\n",
        ]);
        let o = by_id(&checks(&mut env), "tls.openssl").clone();
        assert_eq!(o.status, Status::Skipped);
        assert!(
            o.detail
                .contains("not reached (Could not resolve host: pq.cloudflareresearch.com)"),
            "{}",
            o.detail
        );
    }

    #[test]
    fn gnutls_and_the_policy_files_are_read() {
        let mut env = env_with([PQ_CURL, PQ_CURL]);
        for h in net::PROBE_HOSTS {
            env.captures.push((
                format!("gnutls-cli {h}"),
                "- Description: (TLS1.3-X.509)-(HYBRID-X25519-MLKEM768)-(ECDSA-SECP256R1-SHA256)-(AES-256-GCM)\n".into(),
            ));
        }
        env.files.push((
            OPENSSL_CNF.into(),
            "[openssl_init]\n# wolos-tls\nssl_conf = wolos_ssl_conf\n\n# wolos-tls\n.include /usr/share/wolos/tls/openssl.cnf\n".into(),
        ));
        env.files.push((
            GNUTLS_CONFIG.into(),
            format!(
                "[overrides]\ndefault-priority-string = {}\n",
                net::GNUTLS_PRIORITY
            ),
        ));
        env.files
            .push((BROWSER_POLICY.into(), wol_core::theme::helium::policies()));
        let c = checks(&mut env);
        assert_eq!(
            by_id(&c, "tls.gnutls").status,
            Status::Ok,
            "{:?}",
            by_id(&c, "tls.gnutls")
        );
        let f = file_checks(&mut env);
        assert_eq!(by_id(&f, "security.tls-policy").status, Status::Ok);
        assert_eq!(by_id(&f, "security.browser-tls").status, Status::Ok);
        // Without the files: the policy fails naming both libraries, the browser warns.
        let mut bare = env_with([PQ_CURL, PQ_CURL]);
        let f = file_checks(&mut bare);
        let p = by_id(&f, "security.tls-policy");
        assert_eq!(p.status, Status::Fail);
        assert!(p.detail.contains("OpenSSL and GnuTLS"), "{}", p.detail);
        assert_eq!(by_id(&f, "security.browser-tls").status, Status::Warn);
    }
}
