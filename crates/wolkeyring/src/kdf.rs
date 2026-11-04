// SPDX-License-Identifier: BSD-2-Clause
//! Password KDFs of the keyring (ADR 0054 section 3). The header names the KDF and its
//! parameters; a reader refuses parameters below the floor, so a file cannot be downgraded to a
//! cheaper KDF, and above the ceiling, so a hostile file cannot cost unbounded time.
//!
//! Argon2id (RFC 9106, RustCrypto `argon2` 0.5.3, approved in chat #plan 9168) is what new typed
//! factors and passphrase bundles use: 64 MiB, 3 passes, 1 lane. PBKDF2-HMAC-SHA512 (SP 800-132,
//! wolcrypt) is still read and may be chosen where a FIPS-approved KDF is required.
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// PBKDF2 rounds for new files (the `.wolkey` value, ADR 0046).
pub const PBKDF2_ROUNDS: u32 = wolcrypt::kdf::ROUNDS;
/// Argon2id memory for new files, KiB (64 MiB, RFC 9106 section 4 second option).
pub const ARGON2_M_KIB: u32 = 64 * 1024;
/// Argon2id passes for new files.
pub const ARGON2_T: u32 = 3;
/// Argon2id lanes for new files (one: the crate computes lanes on one thread).
pub const ARGON2_P: u32 = 1;
/// Floor: OWASP's minimum (19 MiB, 2 passes); a file naming less is refused.
pub const ARGON2_MIN_M_KIB: u32 = 19 * 1024;
pub const ARGON2_MIN_T: u32 = 2;
/// Ceiling: 1 GiB, 10 passes, 8 lanes (bounds what a hostile file can cost).
pub const ARGON2_MAX_M_KIB: u32 = 1024 * 1024;
pub const ARGON2_MAX_T: u32 = 10;
pub const ARGON2_MAX_P: u32 = 8;

/// A KDF and its parameters, as the header stores them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "name", deny_unknown_fields)]
pub enum Kdf {
    /// PBKDF2-HMAC-SHA512.
    #[serde(rename = "pbkdf2-hmac-sha512")]
    Pbkdf2 { rounds: u32 },
    /// Argon2id: memory in KiB, passes, lanes.
    #[serde(rename = "argon2id")]
    Argon2id { m_kib: u32, t: u32, p: u32 },
}

impl Kdf {
    /// What new files use.
    pub fn default_new() -> Kdf {
        Kdf::Argon2id {
            m_kib: ARGON2_M_KIB,
            t: ARGON2_T,
            p: ARGON2_P,
        }
    }

    /// Checks the parameters against the floor and ceiling.
    pub fn check(&self) -> crate::Result<()> {
        match self {
            Kdf::Pbkdf2 { rounds } => {
                if *rounds < wolcrypt::kdf::MIN_ROUNDS || *rounds > wolcrypt::kdf::MAX_ROUNDS {
                    return Err(crate::Error::Unsupported(format!(
                        "pbkdf2 with {rounds} rounds (allowed {}..={})",
                        wolcrypt::kdf::MIN_ROUNDS,
                        wolcrypt::kdf::MAX_ROUNDS
                    )));
                }
                Ok(())
            }
            Kdf::Argon2id { m_kib, t, p } => {
                let ok = (ARGON2_MIN_M_KIB..=ARGON2_MAX_M_KIB).contains(m_kib)
                    && (ARGON2_MIN_T..=ARGON2_MAX_T).contains(t)
                    && (1..=ARGON2_MAX_P).contains(p)
                    && *m_kib >= 8 * *p;
                if !ok {
                    return Err(crate::Error::Unsupported(format!(
                        "argon2id m={m_kib} t={t} p={p}"
                    )));
                }
                Ok(())
            }
        }
    }

    /// The 32-byte key of `secret` with `salt` (16..=64 bytes).
    pub fn derive(&self, secret: &[u8], salt: &[u8]) -> crate::Result<Zeroizing<[u8; 32]>> {
        self.check()?;
        if !(16..=64).contains(&salt.len()) {
            return Err(crate::Error::Malformed("kdf salt"));
        }
        match self {
            Kdf::Pbkdf2 { rounds } => wolcrypt::kdf::pbkdf2_sha512(secret, salt, *rounds)
                .ok_or(crate::Error::Unsupported("pbkdf2 parameters".into())),
            Kdf::Argon2id { m_kib, t, p } => {
                let mut out = Zeroizing::new([0u8; 32]);
                argon2_raw(secret, salt, None, *m_kib, *t, *p, &mut out[..])
                    .ok_or(crate::Error::Unsupported("argon2id parameters".into()))?;
                Ok(out)
            }
        }
    }
}

/// Argon2id version 0x13 with optional associated data, into `out`; None on parameters the
/// crate refuses.
pub fn argon2_raw(
    pass: &[u8],
    salt: &[u8],
    secret_ad: Option<(&[u8], &[u8])>,
    m_kib: u32,
    t: u32,
    p: u32,
    out: &mut [u8],
) -> Option<()> {
    use argon2::{Algorithm, Argon2, AssociatedData, ParamsBuilder, Version};
    let mut b = ParamsBuilder::new();
    b.m_cost(m_kib).t_cost(t).p_cost(p).output_len(out.len());
    let a = match secret_ad {
        Some((secret, ad)) => {
            b.data(AssociatedData::new(ad).ok()?);
            Argon2::new_with_secret(secret, Algorithm::Argon2id, Version::V0x13, b.build().ok()?)
                .ok()?
        }
        None => Argon2::new(Algorithm::Argon2id, Version::V0x13, b.build().ok()?),
    };
    a.hash_password_into(pass, salt, out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_and_ceiling_are_refused() {
        assert!(Kdf::Pbkdf2 { rounds: 1000 }.check().is_err());
        assert!(Kdf::Pbkdf2 { rounds: 20_000_000 }.check().is_err());
        assert!(Kdf::default_new().check().is_ok());
        assert!(
            Kdf::Argon2id {
                m_kib: 65536,
                t: 3,
                p: 1
            }
            .check()
            .is_ok()
        );
        assert!(
            Kdf::Argon2id {
                m_kib: 1024,
                t: 3,
                p: 1
            }
            .check()
            .is_err()
        );
        assert!(
            Kdf::Argon2id {
                m_kib: 65536,
                t: 1,
                p: 1
            }
            .check()
            .is_err()
        );
        assert!(
            Kdf::Argon2id {
                m_kib: 4 << 20,
                t: 3,
                p: 1
            }
            .check()
            .is_err()
        );
    }

    #[test]
    fn serde_names_the_kdf() {
        let j = serde_json::to_string(&Kdf::default_new()).unwrap();
        assert_eq!(j, r#"{"name":"argon2id","m_kib":65536,"t":3,"p":1}"#);
        let j = serde_json::to_string(&Kdf::Pbkdf2 {
            rounds: PBKDF2_ROUNDS,
        })
        .unwrap();
        assert_eq!(j, r#"{"name":"pbkdf2-hmac-sha512","rounds":600000}"#);
        let a: Kdf =
            serde_json::from_str(r#"{"name":"argon2id","m_kib":65536,"t":3,"p":1}"#).unwrap();
        assert!(matches!(a, Kdf::Argon2id { .. }));
        assert!(serde_json::from_str::<Kdf>(r#"{"name":"md5"}"#).is_err());
    }
}
