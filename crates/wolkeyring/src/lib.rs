// SPDX-License-Identifier: BSD-2-Clause
//! wolkeyring: the WolOS keyring of ADR 0054. Every secret a person's apps keep (browser
//! passwords, VPN keys, tokens) lives in one store whose keys are 256-bit symmetric keys wrapped
//! by the unlock factors (password, PIN with the TPM, TPM device key, fingerprint through the TPM,
//! FIDO2 hmac-secret, recovery code), whose exports are encrypted to the hybrid KEM X25519 +
//! ML-KEM-768 and signed with Ed25519 + ML-DSA-65 (crate wolcrypt). There is no classical-only
//! path: no format of this crate can be written or read with one half missing.
//!
//! The core modules are pure (bytes and fresh random bytes in, bytes out): [`store`], [`factor`],
//! [`item`], [`audit`], [`bundle`], [`pairing`], [`proto`]. [`io`] holds every file, process and
//! socket access, so the core is tested without a machine.
#![forbid(unsafe_code)]

pub mod audit;
pub mod bundle;
pub mod daemon;
pub mod factor;
pub mod io;
pub mod item;
pub mod kdf;
pub mod migrate;
pub mod pairing;
pub mod proto;
pub mod rand;
pub mod secret_service;
pub mod service;
pub mod store;
pub mod wire;

/// The format string of the store, the first bytes of every derivation label.
pub const FORMAT: &str = "wolos/keyring/1";

/// Why an operation was refused. Every variant is a reason a person or a caller can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A file or message is cut short, too large, or not in the format.
    Malformed(&'static str),
    /// A format this reader does not read (a later version, or a classical-only form).
    Unsupported(String),
    /// The factors given open no rule of the policy.
    WrongFactors,
    /// A byte changed: the authentication tag or a signature did not verify.
    Damaged(&'static str),
    /// The keyring is locked (or the item's class needs the full unlock).
    Locked,
    /// No such item.
    NotFound,
    /// The caller is not on the item's access list (a prompt may add it).
    Denied,
    /// The request breaks a rule of the policy (a PIN alone, a fingerprint for export...).
    Policy(&'static str),
    /// A limit (item count, secret size) would be passed.
    Limit(&'static str),
    /// A file or process step failed.
    Io(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Malformed(w) => write!(f, "not a keyring file or message ({w})"),
            Error::Unsupported(w) => write!(f, "format not supported: {w}"),
            Error::WrongFactors => write!(f, "those unlock factors do not open this keyring"),
            Error::Damaged(w) => write!(f, "damaged or changed: {w}"),
            Error::Locked => write!(f, "the keyring is locked"),
            Error::NotFound => write!(f, "no such item"),
            Error::Denied => write!(f, "this app is not allowed to read that item"),
            Error::Policy(w) => write!(f, "not allowed by the unlock policy: {w}"),
            Error::Limit(w) => write!(f, "limit reached: {w}"),
            Error::Io(w) => write!(f, "{w}"),
        }
    }
}

impl std::error::Error for Error {}

/// Shorthand.
pub type Result<T> = std::result::Result<T, Error>;
