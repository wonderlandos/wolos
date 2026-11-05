// SPDX-License-Identifier: BSD-2-Clause
//! Against the live network (needs the internet; `cargo test -p federate --test live -- --ignored`).
#[test]
#[ignore = "reaches https://federate.network"]
fn the_live_home_page_opens_verified() {
    let r = federate::serve("fed://home.fed");
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert!(r.mime.starts_with("text/html"));
    let css = federate::serve("fed://home.fed/style.css");
    assert_eq!(
        (css.status, css.mime.as_str()),
        (200, "text/css; charset=utf-8")
    );
    assert_eq!(federate::serve("fed://nobody-here.fed").status, 404);
    assert_eq!(
        federate::map_http("http://home.fed/").as_deref(),
        Some("fed://home.fed")
    );
}

/// The door trusts Let's Encrypt's roots only: the network is reached, a site under another
/// authority is refused (with the system store it is reached).
#[test]
#[ignore = "reaches https://federate.network and https://www.digicert.com"]
fn the_door_trusts_only_the_pinned_authorities() {
    use federate::Source;
    let pinned = federate::Https::new("https://federate.network");
    assert!(pinned.get("/v1/bootstrap", 1 << 20).is_ok());
    let other = federate::Https::new("https://www.digicert.com");
    let e = other.get("/", 1 << 20).unwrap_err();
    assert!(e.contains("could not be reached"), "{e}");
    let system = federate::Https::with_roots(
        "https://www.digicert.com",
        ureq::tls::RootCerts::PlatformVerifier,
    );
    assert!(
        system.get("/", 4 << 20).is_ok(),
        "the same site through the system store"
    );
}

/// The live server takes the hybrid group: offered alone, the handshake completes and the
/// negotiated group is X25519MLKEM768 (prints it).
#[test]
#[ignore = "reaches https://federate.network"]
fn the_live_handshake_is_post_quantum_hybrid() {
    use std::io::{Read, Write};
    let mut provider = federate::source::tls_provider();
    provider.kx_groups.truncate(1);
    assert_eq!(
        provider.kx_groups[0].name(),
        rustls::NamedGroup::X25519MLKEM768
    );
    let mut roots = rustls::RootCertStore::empty();
    for d in federate::source::ANCHORS {
        roots
            .add(rustls::pki_types::CertificateDer::from(d.to_vec()))
            .unwrap();
    }
    let config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(provider))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = "federate.network".try_into().unwrap();
    let conn = rustls::ClientConnection::new(std::sync::Arc::new(config), name).unwrap();
    let sock = std::net::TcpStream::connect("federate.network:443").unwrap();
    let mut tls = rustls::StreamOwned::new(conn, sock);
    tls.write_all(
        b"GET /v1/bootstrap HTTP/1.1\r\nHost: federate.network\r\nConnection: close\r\n\r\n",
    )
    .unwrap();
    let mut out = Vec::new();
    let _ = tls.read_to_end(&mut out);
    assert!(
        out.starts_with(b"HTTP/1.1 200"),
        "{}",
        String::from_utf8_lossy(&out[..out.len().min(200)])
    );
    let group = tls.conn.negotiated_key_exchange_group().unwrap().name();
    eprintln!("negotiated group: {group:?}");
    assert_eq!(group, rustls::NamedGroup::X25519MLKEM768);
}
