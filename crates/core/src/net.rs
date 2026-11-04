// SPDX-License-Identifier: BSD-2-Clause
//! The system-wide TLS policy (ADR 0055), in one place: what every TLS stack WolOS ships is told
//! to negotiate, and how `wol doctor tls` reads back what it did. The image files hold the same
//! values (`files/usr/share/wolos/tls/openssl.cnf`, `gnutls.priority`; tests in
//! `crates/controller/tests/image_tls.rs` hold them to these constants).
//!
//! The rule, for every stack: TLS 1.3 first and 1.2 at least, AEAD ciphers only, the hybrid
//! post-quantum group X25519MLKEM768 offered first with X25519 right after it (never ML-KEM alone,
//! never classical only when the server has the hybrid group), no renegotiation.

/// The hybrid group's name as OpenSSL, rustls and curl print it (codepoint 0x11EC).
pub const HYBRID: &str = "X25519MLKEM768";
/// GnuTLS's name for the same group.
pub const HYBRID_GNUTLS: &str = "HYBRID-X25519-MLKEM768";

/// OpenSSL `Groups`: `*` sends a key share for both of the first two, so a server without the
/// hybrid group answers with X25519 at no extra round trip.
pub const OPENSSL_GROUPS: &str = "*X25519MLKEM768:*X25519:secp256r1:secp384r1";
/// OpenSSL `MinProtocol`.
pub const OPENSSL_MIN_PROTOCOL: &str = "TLSv1.2";
/// OpenSSL `Ciphersuites` (TLS 1.3): AEAD only, its default set written down.
pub const OPENSSL_TLS13_SUITES: &str =
    "TLS_AES_256_GCM_SHA384:TLS_CHACHA20_POLY1305_SHA256:TLS_AES_128_GCM_SHA256";
/// OpenSSL `CipherString` (TLS 1.2): ephemeral ECDH with AES-GCM or ChaCha20-Poly1305 only.
pub const OPENSSL_TLS12_CIPHERS: &str = "ECDHE+AESGCM:ECDHE+CHACHA20:!aNULL:!eNULL:@SECLEVEL=2";
/// OpenSSL `Options`.
pub const OPENSSL_OPTIONS: &str = "NoRenegotiation";
/// GnuTLS `default-priority-string`: the same rule in GnuTLS words.
pub const GNUTLS_PRIORITY: &str = "NORMAL:-VERS-ALL:+VERS-TLS1.3:+VERS-TLS1.2:-GROUP-ALL:+GROUP-X25519-MLKEM768:+GROUP-X25519:+GROUP-SECP256R1:+GROUP-SECP384R1:-KX-ALL:+ECDHE-ECDSA:+ECDHE-RSA:-CIPHER-ALL:+AES-256-GCM:+CHACHA20-POLY1305:+AES-128-GCM:-MAC-ALL:+AEAD:%SAFE_RENEGOTIATION";

/// Servers `wol doctor tls` connects to: a public post-quantum test server (it always has the
/// hybrid group, so a classical answer there is our side's fault) and the project's update server.
pub const PROBE_HOSTS: [&str; 2] = ["pq.cloudflareresearch.com", "cdn.wolos.org"];

/// One handshake as a stack reported it.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Negotiated {
    /// `TLSv1.3`, `TLSv1.2`.
    pub version: String,
    /// `X25519MLKEM768`, `X25519`, `secp256r1` (OpenSSL / rustls names; GnuTLS's are mapped).
    pub group: String,
    /// `TLS_AES_256_GCM_SHA384`, `AES-256-GCM`.
    pub cipher: String,
}

impl Negotiated {
    /// The key exchange is the hybrid post-quantum group.
    pub fn post_quantum(&self) -> bool {
        self.group == HYBRID
    }
}

/// `curl -sv` stderr: `* SSL connection using TLSv1.3 / TLS_AES_256_GCM_SHA384 / X25519MLKEM768 / id-ecPublicKey`.
pub fn parse_curl(stderr: &str) -> Option<Negotiated> {
    let l = stderr.lines().find_map(|l| {
        l.trim_start_matches("* ")
            .strip_prefix("SSL connection using ")
    })?;
    let mut p = l.split(" / ").map(str::trim);
    let version = p.next()?.to_string();
    let cipher = p.next()?.to_string();
    let group = canonical_group(p.next()?);
    Some(Negotiated {
        version,
        group,
        cipher,
    })
}

/// `openssl s_client -brief` stderr: `Protocol version: TLSv1.3`, `Ciphersuite: ...` and either
/// `Negotiated TLS1.3 group: X25519MLKEM768` or `Peer Temp Key: X25519, 253 bits`.
pub fn parse_openssl(out: &str) -> Option<Negotiated> {
    let field = |k: &str| {
        out.lines()
            .find_map(|l| l.trim().strip_prefix(k))
            .map(|v| v.trim().to_string())
    };
    let version = field("Protocol version:")?;
    let cipher = field("Ciphersuite:").unwrap_or_default();
    let group = field("Negotiated TLS1.3 group:")
        .or_else(|| field("Peer Temp Key:").map(|v| v.split(',').next().unwrap_or("").to_string()))
        .map(|g| canonical_group(&g))
        .unwrap_or_default();
    Some(Negotiated {
        version,
        group,
        cipher,
    })
}

/// `gnutls-cli` stdout: `- Description: (TLS1.3-X.509)-(HYBRID-X25519-MLKEM768)-(ECDSA-SECP256R1-SHA256)-(AES-256-GCM)`.
pub fn parse_gnutls(out: &str) -> Option<Negotiated> {
    let d = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("- Description: "))?;
    let parts: Vec<&str> = d
        .split(")-(")
        .map(|s| s.trim_matches(|c| c == '(' || c == ')'))
        .collect();
    let version = parts.first()?.split('-').next()?.replace("TLS1.", "TLSv1.");
    let kx = parts.get(1)?;
    let group = match kx.strip_prefix("ECDHE-") {
        _ if *kx == HYBRID_GNUTLS => HYBRID.to_string(),
        Some(c) => canonical_group(c),
        None => kx.to_string(),
    };
    let cipher = parts.last()?.to_string();
    Some(Negotiated {
        version,
        group,
        cipher,
    })
}

/// One spelling per group: `x25519` (curl) and `X25519` (OpenSSL) are the same, so are
/// `SECP256R1` (GnuTLS), `prime256v1` and `secp256r1`.
pub fn canonical_group(g: &str) -> String {
    let g = g.trim();
    match g.to_ascii_lowercase().as_str() {
        "x25519mlkem768" => HYBRID.into(),
        "x25519" => "X25519".into(),
        "secp256r1" | "prime256v1" | "p-256" => "secp256r1".into(),
        "secp384r1" | "p-384" => "secp384r1".into(),
        _ => g.into(),
    }
}

// ---------------------------------------------------------------- our Rust stack (feature `tls`)

/// The TLS 1.3 suites our rustls stack may use (AEAD only).
#[cfg(feature = "tls")]
const RUSTLS_SUITES: [rustls::CipherSuite; 9] = {
    use rustls::CipherSuite as S;
    [
        S::TLS13_AES_256_GCM_SHA384,
        S::TLS13_CHACHA20_POLY1305_SHA256,
        S::TLS13_AES_128_GCM_SHA256,
        S::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
        S::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
        S::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
        S::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
        S::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
        S::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
    ]
};

/// wolcrypt's hybrid group, offered for TLS 1.3 only (the codepoint has no TLS 1.2 meaning, and
/// a TLS 1.2 server naming it must not start a hybrid exchange).
#[cfg(feature = "tls")]
#[derive(Debug)]
struct Tls13Only(wolcrypt::tls::X25519MlKem768);

#[cfg(feature = "tls")]
impl rustls::crypto::SupportedKxGroup for Tls13Only {
    fn start(&self) -> Result<Box<dyn rustls::crypto::ActiveKeyExchange>, rustls::Error> {
        self.0.start()
    }
    fn name(&self) -> rustls::NamedGroup {
        self.0.name()
    }
    fn usable_for_version(&self, v: rustls::ProtocolVersion) -> bool {
        v == rustls::ProtocolVersion::TLSv1_3
    }
}

/// The one rustls provider of WolOS's own programs (ADR 0055): ring's, with the hybrid
/// post-quantum group X25519MLKEM768 (wolcrypt's: fips203 ML-KEM-768 and wolcrypt's X25519, the
/// OS's one ML-KEM) first, X25519 second, then P-256 and P-384; AEAD suites only. The server picks
/// inside an authenticated handshake, so a man in the middle cannot push it down to classical.
#[cfg(feature = "tls")]
pub fn tls_provider() -> std::sync::Arc<rustls::crypto::CryptoProvider> {
    use std::sync::{Arc, OnceLock};
    static P: OnceLock<Arc<rustls::crypto::CryptoProvider>> = OnceLock::new();
    static PQ: OnceLock<Tls13Only> = OnceLock::new();
    P.get_or_init(|| {
        let ring = rustls::crypto::ring::default_provider();
        let pq: &'static Tls13Only = PQ.get_or_init(|| {
            Tls13Only(wolcrypt::tls::X25519MlKem768 {
                rng: rustls::crypto::ring::default_provider().secure_random,
            })
        });
        let mut p = ring;
        p.kx_groups = vec![
            pq,
            rustls::crypto::ring::kx_group::X25519,
            rustls::crypto::ring::kx_group::SECP256R1,
            rustls::crypto::ring::kx_group::SECP384R1,
        ];
        p.cipher_suites
            .retain(|s| RUSTLS_SUITES.contains(&s.suite()));
        Arc::new(p)
    })
    .clone()
}

/// ureq's TLS settings for every WolOS fetch: [`tls_provider`] with `roots` (the system's
/// verifier, `RootCerts::PlatformVerifier`, unless a caller pins its authorities). Every crate that
/// builds a ureq agent takes its `TlsConfig` from here (`crates/shared/tests/rules_tls.rs` fails on
/// any other). Callers pinning nothing want [`agent_config`].
#[cfg(feature = "tls")]
pub fn tls_config(roots: ureq::tls::RootCerts) -> ureq::tls::TlsConfig {
    ureq::tls::TlsConfig::builder()
        .root_certs(roots)
        .unversioned_rustls_crypto_provider(tls_provider())
        .build()
}

/// The start of every ureq agent WolOS builds: ureq's config builder with [`tls_config`] and the
/// system's certificate store already set. Callers add their timeouts, redirects and user agent,
/// then `.build().into()`. `crates/shared/tests/rules_tls.rs` fails on an agent made any other way.
#[cfg(feature = "tls")]
pub fn agent_config() -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    ureq::Agent::config_builder().tls_config(tls_config(ureq::tls::RootCerts::PlatformVerifier))
}

/// One handshake with `host:443` through [`tls_provider`] and the system's verifier, for `wol
/// doctor tls`: what our Rust stack negotiated. No request is sent; the connection is closed.
#[cfg(feature = "tls")]
pub fn probe(host: &str, timeout: std::time::Duration) -> Result<Negotiated, String> {
    use std::net::ToSocketAddrs;
    use std::sync::Arc;
    let verifier = rustls_platform_verifier_config()?;
    let config = Arc::new(verifier);
    let name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|_| format!("{host} is not a host name"))?;
    let mut conn = rustls::ClientConnection::new(config, name).map_err(|e| e.to_string())?;
    let addr = (host, 443)
        .to_socket_addrs()
        .map_err(|e| format!("{host}: {e}"))?
        .next()
        .ok_or_else(|| format!("{host} has no address"))?;
    let mut sock =
        std::net::TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("{host}: {e}"))?;
    let _ = sock.set_read_timeout(Some(timeout));
    let _ = sock.set_write_timeout(Some(timeout));
    while conn.is_handshaking() {
        conn.complete_io(&mut sock)
            .map_err(|e| format!("{host}: {e}"))?;
    }
    let version = match conn.protocol_version() {
        Some(rustls::ProtocolVersion::TLSv1_3) => "TLSv1.3".to_string(),
        Some(rustls::ProtocolVersion::TLSv1_2) => "TLSv1.2".to_string(),
        v => format!("{v:?}"),
    };
    let group = conn
        .negotiated_key_exchange_group()
        .map(|g| canonical_group(&format!("{:?}", g.name())))
        .unwrap_or_default();
    let cipher = conn
        .negotiated_cipher_suite()
        .and_then(|s| s.suite().as_str().map(str::to_string))
        .unwrap_or_default();
    conn.send_close_notify();
    let _ = conn.complete_io(&mut sock);
    Ok(Negotiated {
        version,
        group,
        cipher,
    })
}

/// A rustls client config over [`tls_provider`] that verifies with the system's store, as ureq's
/// `RootCerts::PlatformVerifier` does.
#[cfg(feature = "tls")]
fn rustls_platform_verifier_config() -> Result<rustls::ClientConfig, String> {
    let provider = tls_provider();
    let verifier =
        rustls_platform_verifier::Verifier::new(provider.clone()).map_err(|e| e.to_string())?;
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(rustls::ALL_VERSIONS)
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(verifier))
        .with_no_client_auth())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hybrid_group_is_first_and_x25519_second_everywhere() {
        let o: Vec<&str> = OPENSSL_GROUPS.split(':').collect();
        assert_eq!(o[..2], ["*X25519MLKEM768", "*X25519"]);
        assert!(
            !o.iter().any(|g| g.trim_start_matches('*') == "MLKEM768"),
            "never ML-KEM alone"
        );
        let g: Vec<&str> = GNUTLS_PRIORITY
            .split(':')
            .filter(|t| t.starts_with("+GROUP-"))
            .collect();
        assert_eq!(g[..2], ["+GROUP-X25519-MLKEM768", "+GROUP-X25519"]);
        assert!(
            GNUTLS_PRIORITY.contains(":-GROUP-ALL:"),
            "only the listed groups"
        );
    }

    #[test]
    fn only_aead_ciphers_and_tls_1_2_at_least() {
        for s in OPENSSL_TLS13_SUITES.split(':') {
            assert!(s.contains("GCM") || s.contains("CHACHA20_POLY1305"), "{s}");
        }
        for s in OPENSSL_TLS12_CIPHERS
            .split(':')
            .filter(|s| !s.starts_with(['!', '@']))
        {
            assert!(s == "ECDHE+AESGCM" || s == "ECDHE+CHACHA20", "{s}");
        }
        assert_eq!(OPENSSL_MIN_PROTOCOL, "TLSv1.2");
        let ciphers: Vec<&str> = GNUTLS_PRIORITY
            .split(':')
            .skip_while(|t| *t != "-CIPHER-ALL")
            .skip(1)
            .take_while(|t| t.starts_with('+') && !t.starts_with("+AEAD"))
            .filter(|t| !t.starts_with("+ECDHE"))
            .collect();
        assert_eq!(
            ciphers,
            ["+AES-256-GCM", "+CHACHA20-POLY1305", "+AES-128-GCM"]
        );
        assert!(GNUTLS_PRIORITY.contains(":-VERS-ALL:+VERS-TLS1.3:+VERS-TLS1.2:"));
        assert!(GNUTLS_PRIORITY.contains(":-MAC-ALL:+AEAD:"));
    }

    #[cfg(feature = "tls")]
    #[test]
    fn our_provider_offers_the_hybrid_group_first_and_only_aead() {
        use rustls::NamedGroup as G;
        let p = tls_provider();
        let names: Vec<G> = p.kx_groups.iter().map(|g| g.name()).collect();
        assert_eq!(
            names,
            [G::X25519MLKEM768, G::X25519, G::secp256r1, G::secp384r1]
        );
        assert!(!p.kx_groups[0].usable_for_version(rustls::ProtocolVersion::TLSv1_2));
        assert!(p.kx_groups[0].usable_for_version(rustls::ProtocolVersion::TLSv1_3));
        assert!(!p.cipher_suites.is_empty());
        for s in &p.cipher_suites {
            assert!(RUSTLS_SUITES.contains(&s.suite()), "{:?}", s.suite());
        }
        assert_eq!(format!("{:?}", G::X25519MLKEM768), HYBRID);
        assert!(
            std::sync::Arc::ptr_eq(&p, &tls_provider()),
            "one provider per process"
        );
    }

    /// Live: our stack against the post-quantum test server and the update server.
    #[cfg(feature = "tls")]
    #[test]
    #[ignore = "reaches pq.cloudflareresearch.com and cdn.wolos.org"]
    fn our_stack_negotiates_the_hybrid_group_live() {
        for h in PROBE_HOSTS {
            let n = probe(h, std::time::Duration::from_secs(15)).unwrap();
            eprintln!("{h}: {n:?}");
            assert!(n.post_quantum(), "{h}: {n:?}");
            assert_eq!(n.version, "TLSv1.3");
        }
    }

    /// Live: a server without the hybrid group (Arch's mirror, measured 2026-11-04) still answers,
    /// with X25519 (one HelloRetryRequest: only the hybrid share is sent first).
    #[cfg(feature = "tls")]
    #[test]
    #[ignore = "reaches mirror.pkgbuild.com"]
    fn a_classical_server_still_answers_with_x25519() {
        let n = probe("mirror.pkgbuild.com", std::time::Duration::from_secs(15)).unwrap();
        eprintln!("mirror.pkgbuild.com: {n:?}");
        assert_eq!(
            (n.version.as_str(), n.group.as_str()),
            ("TLSv1.3", "X25519")
        );
    }

    /// Lines as curl 8.23, OpenSSL 3.6.5 and GnuTLS 3.8.13 printed them (forge, 2026-11-04).
    #[test]
    fn the_three_reports_parse() {
        let c = parse_curl(
            "* ALPN: curl offers h2\n* SSL connection using TLSv1.3 / TLS_AES_256_GCM_SHA384 / X25519MLKEM768 / id-ecPublicKey\n",
        )
        .unwrap();
        assert_eq!(
            (c.version.as_str(), c.cipher.as_str(), c.post_quantum()),
            ("TLSv1.3", "TLS_AES_256_GCM_SHA384", true)
        );
        let c = parse_curl(
            "* SSL connection using TLSv1.3 / TLS_AES_256_GCM_SHA384 / x25519 / RSASSA-PSS\n",
        )
        .unwrap();
        assert_eq!(c.group, "X25519");
        assert!(!c.post_quantum());
        let o = parse_openssl(
            "Connecting to 1.2.3.4\nCONNECTION ESTABLISHED\nProtocol version: TLSv1.3\nCiphersuite: TLS_AES_128_GCM_SHA256\nNegotiated TLS1.3 group: X25519MLKEM768\n",
        )
        .unwrap();
        assert!(o.post_quantum());
        assert_eq!(o.cipher, "TLS_AES_128_GCM_SHA256");
        let o = parse_openssl("Protocol version: TLSv1.3\nCiphersuite: TLS_AES_256_GCM_SHA384\nPeer Temp Key: X25519, 253 bits\n").unwrap();
        assert_eq!(o.group, "X25519");
        let g = parse_gnutls(
            "- Status: ok\n- Description: (TLS1.3-X.509)-(HYBRID-X25519-MLKEM768)-(ECDSA-SECP256R1-SHA256)-(AES-256-GCM)\n",
        )
        .unwrap();
        assert_eq!(
            (g.version.as_str(), g.group.as_str(), g.cipher.as_str()),
            ("TLSv1.3", HYBRID, "AES-256-GCM")
        );
        let g = parse_gnutls(
            "- Description: (TLS1.3-X.509)-(ECDHE-SECP256R1)-(RSA-PSS-RSAE-SHA256)-(AES-256-GCM)\n",
        )
        .unwrap();
        assert_eq!(g.group, "secp256r1");
        assert!(parse_curl("* Connected\n").is_none());
        assert!(parse_openssl("error\n").is_none());
        assert!(parse_gnutls("*** Fatal error\n").is_none());
    }
}
