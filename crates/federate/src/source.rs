// SPDX-License-Identifier: BSD-2-Clause
//! Where the bytes come from. Every source is an untrusted distributor: what it returns counts
//! only after the checks in [`crate::verify`]. The network's native transport (framed JSON over
//! TCP port 4077) has no public node listed today (`native_nodes` is empty in the bootstrap
//! answer), so the HTTPS compatibility door is the source that works.
use std::io::Read;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Largest root zone read (the reference client's cap is the same order).
pub const MAX_ROOT: u64 = 8 << 20;
/// Largest manifest read.
pub const MAX_MANIFEST: u64 = 4 << 20;
/// Largest content block of a document (the protocol caps blocks at 64 MiB).
pub const MAX_BLOCK: u64 = 64 << 20;
/// Largest block a document loads (scripts, styles, pictures, fetches): a page of many large
/// subresources cannot hold gigabytes in memory.
pub const MAX_SUBRESOURCE: u64 = 16 << 20;

pub trait Source: Send + Sync {
    /// The body of `path` (`/v1/root`, `/v1/manifest/<address>`, `/v1/block/<address>`), at
    /// most `max` bytes, or a sentence saying why not.
    fn get(&self, path: &str, max: u64) -> Result<Vec<u8>, String>;
}

/// The certificate authorities `federate.network` is reached through (ADR 0052, "Transport"):
/// ISRG Root X1 (SHA-256 96bcec06...08c6, until 2035-06-04) and ISRG Root X2 (69729b8e...1470,
/// until 2040-09-17), Let's Encrypt's roots, the issuers of the server's chain on 2026-11-04
/// (leaf <- YR2 <- Root YR <- ISRG Root X1). The usual Web PKI checks run against these two only,
/// so no other authority in the system store (or one added to it) can stand in for the network's
/// door. Rotation: a WolOS update adds the next root before the network moves to it.
pub const ANCHORS: &[&[u8]] = &[
    include_bytes!("anchors/isrg-root-x1.der"),
    include_bytes!("anchors/isrg-root-x2.der"),
];

/// The TLS provider of the door: rustls's ring provider with the hybrid post-quantum group
/// X25519MLKEM768 (wolcrypt's) offered first and X25519 after it, so a server that has the hybrid
/// group (`federate.network` does) gets it and one that has not still answers. The choice is the
/// server's inside an authenticated handshake: a man in the middle cannot push it down.
pub fn tls_provider() -> rustls::crypto::CryptoProvider {
    static PQ: OnceLock<wolcrypt::tls::X25519MlKem768> = OnceLock::new();
    let mut p = rustls::crypto::ring::default_provider();
    let pq = PQ.get_or_init(|| wolcrypt::tls::X25519MlKem768 {
        rng: rustls::crypto::ring::default_provider().secure_random,
    });
    p.kx_groups.insert(0, pq);
    p
}

/// The network's HTTPS compatibility door, with its certificate authorities pinned ([`ANCHORS`])
/// and the hybrid key exchange preferred ([`tls_provider`]).
pub struct Https {
    base: String,
    agent: ureq::Agent,
}

impl Https {
    pub fn new(base: &str) -> Https {
        Https::with_roots(base, Https::pinned())
    }

    /// The pinned roots, as ureq takes them.
    pub fn pinned() -> ureq::tls::RootCerts {
        let certs = ANCHORS
            .iter()
            .map(|d| ureq::tls::Certificate::from_der(d))
            .collect();
        ureq::tls::RootCerts::Specific(std::sync::Arc::new(certs))
    }

    /// A door to `base` trusting `roots` (tests reach other hosts with the system's store).
    pub fn with_roots(base: &str, roots: ureq::tls::RootCerts) -> Https {
        use ureq::tls::TlsConfig;
        let agent = ureq::Agent::config_builder()
            .tls_config(
                TlsConfig::builder()
                    .root_certs(roots)
                    .unversioned_rustls_crypto_provider(Arc::new(tls_provider()))
                    .build(),
            )
            .https_only(true)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_per_read(Some(Duration::from_secs(30)))
            .max_redirects(4)
            .user_agent(concat!("wolos-federate/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Https {
            base: base.trim_end_matches('/').to_owned(),
            agent,
        }
    }
}

impl Source for Https {
    fn get(&self, path: &str, max: u64) -> Result<Vec<u8>, String> {
        let url = format!("{}{path}", self.base);
        let host = self.base.trim_start_matches("https://");
        let resp = self
            .agent
            .get(&url)
            .call()
            .map_err(|e| format!("{host} could not be reached ({e})"))?;
        match resp.status().as_u16() {
            200 => {}
            404 => return Err(format!("{host} does not have {path}")),
            s => return Err(format!("{host} answered {s} for {path}")),
        }
        let mut bytes = Vec::new();
        resp.into_body()
            .into_reader()
            .take(max + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("the download from {host} stopped ({e})"))?;
        if bytes.len() as u64 > max {
            return Err(format!("{path} from {host} is larger than {max} bytes"));
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn the_hybrid_group_is_offered_first_and_x25519_after_it() {
        let names: Vec<_> = tls_provider().kx_groups.iter().map(|g| g.name()).collect();
        assert_eq!(names[0], rustls::NamedGroup::X25519MLKEM768);
        assert_eq!(names[1], rustls::NamedGroup::X25519);
    }

    #[test]
    fn the_anchors_are_the_two_isrg_roots() {
        let hex = |d: &[u8]| -> String {
            Sha256::digest(d)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        };
        let got: Vec<String> = ANCHORS.iter().map(|d| hex(d)).collect();
        assert_eq!(
            got,
            [
                "96bcec06264976f37460779acf28c5a7cfe8a3c0aae11a8ffcee05c0bddf08c6",
                "69729b8e15a86efc177a57afb7171dfc64add28c2fca8cf1507e34453ccb1470",
            ]
        );
    }
}
