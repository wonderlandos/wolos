// SPDX-License-Identifier: BSD-2-Clause
//! The resolver's one piece of local state, the newest root zone seen (version and digest), sealed
//! with wolcrypt (ADR 0052, "Local state"): the record is sealed to the hybrid KEM key X25519 +
//! ML-KEM-768 and the sealed bytes are signed with the hybrid signature Ed25519 + ML-DSA-65, both
//! derived from a 32-byte key kept beside it (`state.key`, mode 0600, made on first write). A record
//! that does not verify and open with that key is ignored, as a missing one is: the floor falls back
//! to the built-in minimum, never below it.
//!
//! What this stops: another user, an edit of the disk while the system is off, a backup or a copy
//! from another machine put back. What it cannot stop: a program running as the same user, which
//! can read `state.key` as the client does; and putting back an older sealed record of this machine
//! (that needs a monotonic counter, a TPM NV index, a later task).
use std::io::Write;
use std::path::{Path, PathBuf};
use wolcrypt::kem::{self, EncSecret};
use wolcrypt::sign::{self, Scheme, SigningKey};

/// The format and signature label.
pub const FORMAT: &str = "wolos/federate-state/1";
const PREFIX: &[u8] = b"wolos/federate-state/1\n";

/// Fills a buffer with fresh random bytes; false when it could not.
pub type Fill<'a> = &'a dyn Fn(&mut [u8]) -> bool;

/// The key file beside `state`.
pub fn key_path(state: &Path) -> PathBuf {
    state.with_file_name("state.key")
}

fn read_key(path: &Path) -> Option<[u8; 32]> {
    std::fs::read(path).ok()?.try_into().ok()
}

/// The key, made (0600, never replaced) when there is none.
fn key(path: &Path, fill: Fill) -> Option<[u8; 32]> {
    if let Some(k) = read_key(path) {
        return Some(k);
    }
    let mut k = [0u8; 32];
    if !fill(&mut k) {
        return None;
    }
    std::fs::create_dir_all(path.parent()?).ok()?;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    match o.open(path) {
        Ok(mut f) => f.write_all(&k).ok().map(|()| k),
        // Another process made it first: use that one.
        Err(_) => read_key(path),
    }
}

/// The sealed and signed bytes of `record` under `seed`.
pub fn seal(seed: &[u8; 32], record: &[u8], rnd: &[u8; 64]) -> Option<Vec<u8>> {
    let enc = EncSecret::from_seed(seed);
    let mut out = kem::seal(FORMAT, enc.public(), PREFIX, record, rnd).ok()?;
    let sig = SigningKey::from_seed(seed, Scheme::Signing)
        .sign(FORMAT, &out)
        .ok()?;
    out.extend_from_slice(&sig.ed25519);
    out.extend_from_slice(&sig.second);
    Some(out)
}

/// The record inside `bytes`, when both signature halves verify and it opens under `seed`.
pub fn open(seed: &[u8; 32], bytes: &[u8]) -> Option<Vec<u8>> {
    let n = sign::ED_SIG + sign::ML_SIG;
    let (sealed, sig) = bytes.split_at(bytes.len().checked_sub(n)?);
    if !sealed.starts_with(PREFIX) {
        return None;
    }
    let sig = sign::Sig {
        ed25519: sig[..sign::ED_SIG].try_into().ok()?,
        second: sig[sign::ED_SIG..].to_vec(),
    };
    let keys = SigningKey::from_seed(seed, Scheme::Signing);
    sign::verify(keys.public(), FORMAT, sealed, &sig).ok()?;
    let plain = kem::open(FORMAT, &EncSecret::from_seed(seed), sealed, PREFIX.len()).ok()?;
    Some(plain.to_vec())
}

/// The record at `path` (`<version> <digest>`), or `None` when it is missing, does not verify or
/// does not open with the key beside it.
pub fn load(path: &Path) -> Option<String> {
    let seed = read_key(&key_path(path))?;
    let bytes = std::fs::read(path).ok()?;
    String::from_utf8(open(&seed, &bytes)?).ok()
}

/// Writes `record` sealed to `path` (through a temporary file and a rename); false when it could
/// not (the record is then only in memory).
pub fn store(path: &Path, record: &str, fill: Fill) -> bool {
    let Some(seed) = key(&key_path(path), fill) else {
        return false;
    };
    let mut rnd = [0u8; 64];
    if !fill(&mut rnd) {
        return false;
    }
    let Some(bytes) = seal(&seed, record.as_bytes(), &rnd) else {
        return false;
    };
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, path).is_ok()
}

/// Fresh random bytes from the TLS provider's generator (ring's).
pub fn system_fill(buf: &mut [u8]) -> bool {
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(buf)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_record_opens_only_whole_and_only_with_its_key() {
        let seed = [5u8; 32];
        let b = seal(&seed, b"1783402200 abc", &[1u8; 64]).unwrap();
        assert_eq!(open(&seed, &b).as_deref(), Some(&b"1783402200 abc"[..]));
        assert!(
            !b.windows(10).any(|w| w == b"1783402200"),
            "the record is not in clear"
        );
        for i in [0, PREFIX.len() + 5, b.len() / 2, b.len() - 1] {
            let mut t = b.clone();
            t[i] ^= 1;
            assert_eq!(open(&seed, &t), None, "byte {i}");
        }
        assert_eq!(open(&[6u8; 32], &b), None, "another key");
        assert_eq!(open(&seed, b"1783402200 abc"), None, "a plain record");
    }
}
