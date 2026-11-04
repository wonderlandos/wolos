// SPDX-License-Identifier: BSD-2-Clause
//! The `.wolkey` file: an author's key material encrypted at rest with a passphrase (ADR 0046,
//! "Key storage"). PBKDF2-HMAC-SHA512 (600 000 rounds, 16-byte random salt) turns the passphrase
//! into a ChaCha20-Poly1305 key; the associated data binds the format, the name the key is stored
//! under and the KDF parameters, so a file renamed to another serial or downgraded to fewer rounds
//! does not open. Argon2id would be stronger against GPUs but is not in the dependency tree; both
//! primitives come from [`wolcrypt`] (`kdf`, `aead`), pure Rust: files written with ring before
//! open byte for byte the same (`vectors/wolkey/ring-made.wolkey`).
//!
//! `{"format":1,"name":"<certificate id, 128 hex; 16 hex before rule IDS>","kdf":"pbkdf2-hmac-sha512","rounds":600000,"salt":"<b64url>",
//! "aead":"chacha20-poly1305","nonce":"<b64url>","sealed":"<b64url>"}`
use crate::b64;
use crate::cert::is_serial;
use crate::key::KeyMaterial;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// PBKDF2 rounds for new files.
pub const ROUNDS: u32 = wolcrypt::kdf::ROUNDS;
/// Fewest rounds a file may name (lower is refused, so a file cannot be downgraded).
pub const MIN_ROUNDS: u32 = wolcrypt::kdf::MIN_ROUNDS;
/// Most rounds a file may name (bounds the time a hostile file can cost).
pub const MAX_ROUNDS: u32 = wolcrypt::kdf::MAX_ROUNDS;
/// Largest `.wolkey` file.
pub const MAX_FILE: usize = 2048;
const NONCE: usize = wolcrypt::aead::NONCE;
/// Shortest passphrase accepted for a new file, in characters.
pub const MIN_PASSPHRASE: usize = 8;
const KDF: &str = "pbkdf2-hmac-sha512";
const AEAD: &str = "chacha20-poly1305";
const LABEL: &[u8] = b"wolos/wolkey/1\0";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct File {
    format: u32,
    name: String,
    kdf: String,
    rounds: u32,
    salt: String,
    aead: String,
    nonce: String,
    sealed: String,
}

/// Why a `.wolkey` did not open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError {
    /// Not a `.wolkey`, or parameters out of range.
    Malformed,
    /// Stored under another name than the one asked for.
    WrongName,
    /// Wrong passphrase, or the file was changed.
    WrongPassphrase,
    /// The passphrase is shorter than [`MIN_PASSPHRASE`].
    WeakPassphrase,
}

impl KeyError {
    /// What a person reads.
    pub fn sentence(self) -> &'static str {
        match self {
            KeyError::Malformed => "This key file is damaged.",
            KeyError::WrongName => "This key file belongs to another certificate.",
            KeyError::WrongPassphrase => "The passphrase does not open this key.",
            KeyError::WeakPassphrase => "Choose a passphrase of at least 8 characters.",
        }
    }
}

fn aad(name: &str, rounds: u32, salt: &[u8]) -> Vec<u8> {
    let mut a = LABEL.to_vec();
    a.extend_from_slice(name.as_bytes());
    a.push(0);
    a.extend_from_slice(rounds.to_string().as_bytes());
    a.push(0);
    a.extend_from_slice(salt);
    a
}

fn key(pass: &[u8], salt: &[u8], rounds: u32) -> Option<Zeroizing<[u8; 32]>> {
    wolcrypt::kdf::pbkdf2_sha512(pass, salt, rounds)
}

/// Encrypts `km` under `pass` for storage as `<name>.wolkey`; `rnd` is 28 fresh random bytes (salt
/// and nonce). The output never holds the seed in the clear.
pub fn lock(
    km: &KeyMaterial,
    name: &str,
    pass: &str,
    rnd: &[u8; 28],
    rounds: u32,
) -> Result<Vec<u8>, KeyError> {
    if pass.chars().count() < MIN_PASSPHRASE {
        return Err(KeyError::WeakPassphrase);
    }
    if !is_serial(name) || !(MIN_ROUNDS..=MAX_ROUNDS).contains(&rounds) {
        return Err(KeyError::Malformed);
    }
    let (salt, nonce) = rnd.split_at(16);
    let k = key(pass.as_bytes(), salt, rounds).ok_or(KeyError::Malformed)?;
    let n: [u8; NONCE] = nonce.try_into().map_err(|_| KeyError::Malformed)?;
    let mut body = km.to_text();
    wolcrypt::aead::seal(&k, &n, &aad(name, rounds, salt), &mut body);
    let f = File {
        format: 1,
        name: name.to_owned(),
        kdf: KDF.into(),
        rounds,
        salt: b64::encode(salt),
        aead: AEAD.into(),
        nonce: b64::encode(nonce),
        sealed: b64::encode(&body),
    };
    serde_json::to_vec(&f).map_err(|_| KeyError::Malformed)
}

/// The name a `.wolkey` says it is stored under (no passphrase needed).
pub fn name_of(file: &[u8]) -> Result<String, KeyError> {
    parse(file).map(|f| f.name)
}

fn parse(file: &[u8]) -> Result<File, KeyError> {
    if file.len() > MAX_FILE {
        return Err(KeyError::Malformed);
    }
    let f: File = serde_json::from_slice(file).map_err(|_| KeyError::Malformed)?;
    let ok = f.format == 1
        && f.kdf == KDF
        && f.aead == AEAD
        && is_serial(&f.name)
        && (MIN_ROUNDS..=MAX_ROUNDS).contains(&f.rounds);
    if ok { Ok(f) } else { Err(KeyError::Malformed) }
}

/// Decrypts a `.wolkey` stored as `name` with `pass`.
pub fn unlock(file: &[u8], name: &str, pass: &str) -> Result<KeyMaterial, KeyError> {
    let f = parse(file)?;
    if f.name != name {
        return Err(KeyError::WrongName);
    }
    let salt = b64::decode(&f.salt, 64).ok_or(KeyError::Malformed)?;
    let nonce = b64::decode(&f.nonce, 12).ok_or(KeyError::Malformed)?;
    let mut body = Zeroizing::new(b64::decode(&f.sealed, MAX_FILE).ok_or(KeyError::Malformed)?);
    let k = key(pass.as_bytes(), &salt, f.rounds).ok_or(KeyError::Malformed)?;
    let n: [u8; NONCE] = nonce
        .as_slice()
        .try_into()
        .map_err(|_| KeyError::Malformed)?;
    wolcrypt::aead::open(&k, &n, &aad(name, f.rounds, &salt), &mut body)
        .ok_or(KeyError::WrongPassphrase)?;
    KeyMaterial::parse(&body).map_err(|_| KeyError::Malformed)
}

/// Re-encrypts under a new name (a key made before its certificate gets the serial on issue).
pub fn rename(
    file: &[u8],
    from: &str,
    to: &str,
    pass: &str,
    rnd: &[u8; 28],
) -> Result<Vec<u8>, KeyError> {
    let km = unlock(file, from, pass)?;
    let rounds = parse(file)?.rounds;
    lock(&km, to, pass, rnd, rounds)
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: &str = "0123456789abcdef";

    #[test]
    fn a_locked_key_opens_only_with_its_passphrase_and_name() {
        let km = KeyMaterial::new([3; 32], false);
        let f = lock(&km, N, "correct horse", &[1; 28], MIN_ROUNDS).unwrap();
        let text = String::from_utf8(f.clone()).unwrap();
        assert!(!text.contains(&b64::encode(&[3; 32])), "seed in the clear");
        assert!(!text.contains("seed"), "key material text in the clear");
        let back = unlock(&f, N, "correct horse").unwrap();
        assert_eq!(back.secret().public(), km.secret().public());
        assert_eq!(
            unlock(&f, N, "wrong horse!").err(),
            Some(KeyError::WrongPassphrase)
        );
        assert_eq!(
            unlock(&f, "fedcba9876543210", "correct horse").err(),
            Some(KeyError::WrongName)
        );
        assert_eq!(name_of(&f), Ok(N.to_owned()));
        let moved = rename(&f, N, "fedcba9876543210", "correct horse", &[2; 28]).unwrap();
        assert!(unlock(&moved, "fedcba9876543210", "correct horse").is_ok());
    }

    #[test]
    fn a_changed_or_downgraded_file_does_not_open() {
        let km = KeyMaterial::new([3; 32], false);
        let f = lock(&km, N, "correct horse", &[1; 28], MIN_ROUNDS).unwrap();
        let text = String::from_utf8(f).unwrap();
        let fewer = text.replace("\"rounds\":210000", "\"rounds\":1000");
        assert_eq!(
            unlock(fewer.as_bytes(), N, "correct horse").err(),
            Some(KeyError::Malformed)
        );
        let other = text.replace("\"rounds\":210000", "\"rounds\":210001");
        assert_eq!(
            unlock(other.as_bytes(), N, "correct horse").err(),
            Some(KeyError::WrongPassphrase)
        );
        assert_eq!(
            lock(&km, N, "short", &[1; 28], ROUNDS).err(),
            Some(KeyError::WeakPassphrase)
        );
    }
}
