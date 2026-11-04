// SPDX-License-Identifier: BSD-2-Clause
//! Key material: the one 32-byte seed from which a certificate's signing keys ([`Secret`]) and,
//! with `seal`, its encryption keys derive. Its text form, `{"format":1,"seed":"<b64url>",
//! "second":"ml-dsa-65"|"slh-dsa-sha2-128s"}`, is what `woltrust sign|issue --key-env` reads from a
//! CI secret. An author's seed is never stored in this form: it lives inside a passphrase-encrypted
//! `.wolkey` (`crate::wolkey`).
use crate::b64;
use crate::make::Secret;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

/// Largest key material text.
pub const MAX_TEXT: usize = 512;

/// One seed and which post-quantum signature it signs with.
pub struct KeyMaterial {
    seed: Zeroizing<[u8; 32]>,
    /// True for a root (SLH-DSA-SHA2-128s), false for everything else (ML-DSA-65).
    pub root: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Text {
    format: u32,
    seed: String,
    second: String,
}

impl Drop for Text {
    // Releases nothing but memory: wipes the base64 seed.
    fn drop(&mut self) {
        self.seed.zeroize();
    }
}

const ML: &str = "ml-dsa-65";
const SLH: &str = "slh-dsa-sha2-128s";

impl KeyMaterial {
    /// Key material for `seed`.
    pub fn new(seed: [u8; 32], root: bool) -> KeyMaterial {
        KeyMaterial {
            seed: Zeroizing::new(seed),
            root,
        }
    }

    /// Parses the text form (surrounding whitespace allowed).
    pub fn parse(text: &[u8]) -> Result<KeyMaterial, &'static str> {
        if text.len() > MAX_TEXT {
            return Err("key material too large");
        }
        let t: Text =
            serde_json::from_slice(text.trim_ascii()).map_err(|_| "key material damaged")?;
        if t.format != 1 {
            return Err("unknown key material format");
        }
        let root = match t.second.as_str() {
            ML => false,
            SLH => true,
            _ => return Err("unknown signature scheme in key material"),
        };
        let raw = Zeroizing::new(b64::decode(&t.seed, 32).ok_or("key material seed malformed")?);
        let seed: [u8; 32] = raw
            .as_slice()
            .try_into()
            .map_err(|_| "key material seed length")?;
        Ok(KeyMaterial::new(seed, root))
    }

    /// The text form. Holds the secret: the caller keeps it out of logs and plain files.
    pub fn to_text(&self) -> Zeroizing<Vec<u8>> {
        let t = Text {
            format: 1,
            seed: b64::encode(&self.seed[..]),
            second: if self.root { SLH } else { ML }.to_owned(),
        };
        Zeroizing::new(serde_json::to_vec(&t).unwrap_or_default())
    }

    /// The signing keys.
    pub fn secret(&self) -> Secret {
        Secret::from_seed(&self.seed, self.root)
    }

    /// The encryption keys.
    #[cfg(feature = "seal")]
    pub fn enc(&self) -> crate::seal::EncSecret {
        crate::seal::EncSecret::from_seed(&self.seed)
    }

    /// The `keys` object of a certificate body for this material, with the encryption keys when
    /// `seal` is built (roots never carry them).
    pub fn keys_text(&self) -> crate::cert::KeysText {
        let t = crate::make::keys_text(&self.secret());
        #[cfg(feature = "seal")]
        if !self.root {
            let e = self.enc();
            return crate::cert::KeysText {
                x25519: Some(b64::encode(&e.public().x25519)),
                ml_kem_768: Some(b64::encode(&e.public().ml_kem_768[..])),
                ..t
            };
        }
        t
    }

    /// A 16-hex id of the public signing key, naming a key before its certificate exists.
    pub fn key_id(&self) -> String {
        crate::request::key_id(&self.secret().public().ed25519)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_material_round_trips_and_refuses_garbage() {
        for root in [false, true] {
            let k = KeyMaterial::new([4; 32], root);
            let back = KeyMaterial::parse(&k.to_text()).unwrap();
            assert_eq!(back.root, root);
            assert_eq!(back.secret().public(), k.secret().public());
        }
        for bad in [
            &b""[..],
            b"{}",
            br#"{"format":2,"seed":"AAAA","second":"ml-dsa-65"}"#,
            br#"{"format":1,"seed":"AAAA","second":"ml-dsa-65"}"#,
            br#"{"format":1,"seed":"BAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQ","second":"rsa"}"#,
        ] {
            assert!(KeyMaterial::parse(bad).is_err());
        }
    }
}
