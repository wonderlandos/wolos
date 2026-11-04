// SPDX-License-Identifier: BSD-2-Clause
//! Author certificates of ADR 0046: the `.wolcert` format, hybrid signatures, certificate chains,
//! revocation lists and the trust verdict. Pure: bytes and a clock value go in, a verdict comes out;
//! there is no file, network or clock access, so the system, the hub Worker and the browser share
//! this one implementation (docs/modules/trust.md). Every input is size-capped before it is parsed.
#![forbid(unsafe_code)]

pub mod b64;
pub mod cert;
pub mod crypto;
pub mod filecert;
pub mod filecert_vectors;
#[cfg(feature = "seal")]
pub mod init;
pub mod key;
pub mod make;
pub mod origin;
pub mod pki;
pub mod report;
pub mod request;
#[cfg(feature = "seal")]
pub mod request_vectors;
pub mod revoked;
#[cfg(feature = "seal")]
pub mod seal;
#[cfg(feature = "seal")]
pub mod seal_vectors;
pub mod store;
pub mod vectors;
pub mod verify;
#[cfg(feature = "seal")]
pub mod wolkey;

pub use cert::{Body, Cert, Class, Kind, Level, OrgRef, Usage};
pub use crypto::{Purpose, Wsig};
pub use origin::Origin;
pub use report::{CertInfo, Finding, Report, Trust, Verdict};
pub use revoked::{Reason, RevList};
pub use store::Store;
pub use verify::{Code, Input, Outcome, verify};

/// Certificate format number (`"format":1` in a `.wolcert`).
pub const FORMAT: u32 = 1;
/// Largest certificate body (the exact signed bytes): 8 KiB, room for the 1.6 KB of encryption keys
/// next to the 2.6 KB ML-DSA-65 key.
pub const MAX_BODY: usize = 8192;
/// Largest `.wolcert` file: 8 KiB of body, a 7.8 KB root signature, base64 and braces.
pub const MAX_CERT_FILE: usize = 24 * 1024;
/// Most certificates in a `CHAIN` member (the leaf first, never a root).
pub const MAX_CHAIN_CERTS: usize = 4;
/// Largest `CHAIN` member.
pub const MAX_CHAIN_BYTES: usize = 64 * 1024;
/// Largest `SHA256.wsig` member or revocation-list signature file.
pub const MAX_WSIG: usize = 16 * 1024;
/// Largest `revoked.json`.
pub const MAX_REVOKED_BYTES: usize = 1 << 20;
/// Most entries in a revocation list.
pub const MAX_REVOKED_ENTRIES: usize = 10_000;
/// How long a revocation list is fresh: 7 days from `issued`.
pub const LIST_LIFETIME: u64 = 7 * 86_400;
/// How long past `expires` a list still lets verified files show as verified: 30 days.
pub const STALE_WINDOW: u64 = 30 * 86_400;
/// Subject label that marks a root as the shared test root; the system never loads one.
pub const TEST_LABEL: &str = "test";
