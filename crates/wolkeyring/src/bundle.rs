// SPDX-License-Identifier: BSD-2-Clause
//! The keyring's identity and the export bundle `wolos/keyring-bundle/1` (ADR 0054 section 7).
//!
//! Identity: from the keyring's 32-byte `id_seed`, an X25519 + ML-KEM-768 recipient key and an
//! Ed25519 + ML-DSA-65 signing key, each from its own derived seed. Its public form
//! (`wol keyring id`) is one line: `wolos-keyring-id/1 <base64url of x25519 || ml-kem-768 ||
//! ed25519 || ml-dsa-65>`; its fingerprint is the first 16 bytes of SHA-512 over those bytes.
//!
//! Bundle: `MAGIC || mode (1 recipient, 2 passphrase) || u32 context length || context ||` then
//! for mode 1 `recipient fingerprint (16) || wolcrypt::kem::seal(...)` (X25519 ephemeral, ML-KEM
//! ciphertext, AEAD with the whole header as associated data), for mode 2 `u32 KDF length || KDF
//! JSON || salt (32) || AEAD`. The plaintext is `u32 payload length || payload JSON || signer
//! public keys || Ed25519 signature || ML-DSA-65 signature`; the signature covers
//! `"wolos/keyring/1/bundle" 0x00 || SHA-512(payload) || SHA-512(header)`, so it binds the
//! recipient, the context (a move's transcript) and the mode. Both halves must verify.
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use wolcrypt::kem::{EncPublic, EncSecret, KEM_PK, X_PK};
use wolcrypt::sign::{ED_PK, ED_SIG, ML_PK, ML_SIG, PublicKeys, Scheme, Second, Sig, SigningKey};

use crate::item::Meta;
use crate::kdf::Kdf;
use crate::rand::Random;
use crate::wire::{self, Reader};
use crate::{Error, Result};

/// The first bytes of every bundle.
pub const MAGIC: &[u8] = b"wolos/keyring-bundle/1\0";
/// The format string the KEM schedule binds.
pub const KEM_FORMAT: &str = "wolos/keyring-bundle/1";
/// The signature's label.
pub const SIGN_LABEL: &str = "wolos/keyring/1/bundle";
/// The public identity line's prefix.
pub const ID_PREFIX: &str = "wolos-keyring-id/1 ";
/// Largest bundle (64 MiB, like the store).
pub const MAX_BUNDLE: usize = 64 << 20;
/// Shortest export passphrase, in characters.
pub const MIN_PASSPHRASE: usize = 12;
/// Longest context.
pub const MAX_CONTEXT: usize = 256;
const PUB_LEN: usize = X_PK + KEM_PK + ED_PK + ML_PK;

/// A keyring's secret identity; wiped on drop (wolcrypt's secret types).
pub struct Identity {
    enc: EncSecret,
    sign: SigningKey,
}

/// A keyring's public identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicId {
    pub enc: EncPublic,
    pub sign: PublicKeys,
}

fn seed(id_seed: &[u8; 32], what: &str) -> Zeroizing<[u8; 32]> {
    let d = wire::derive(&[format!("{}/id/{what}", crate::FORMAT).as_bytes(), id_seed]);
    let mut s = Zeroizing::new([0u8; 32]);
    s.copy_from_slice(&d[..32]);
    s
}

impl Identity {
    /// The identity of `id_seed`.
    pub fn from_seed(id_seed: &[u8; 32]) -> Identity {
        Identity {
            enc: EncSecret::from_seed(&seed(id_seed, "enc")),
            sign: SigningKey::from_seed(&seed(id_seed, "sign"), Scheme::Signing),
        }
    }
    /// The public halves.
    pub fn public(&self) -> PublicId {
        PublicId {
            enc: self.enc.public().clone(),
            sign: self.sign.public().clone(),
        }
    }
    /// Signs `msg` (both halves).
    pub fn sign(&self, msg: &[u8]) -> Result<Sig> {
        self.sign
            .sign_message(msg)
            .map_err(|_| Error::Damaged("signature could not be made"))
    }
    /// The recipient secret.
    pub fn enc(&self) -> &EncSecret {
        &self.enc
    }
}

impl PublicId {
    /// x25519 || ml-kem-768 || ed25519 || ml-dsa-65.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(PUB_LEN);
        v.extend_from_slice(&self.enc.x25519);
        v.extend_from_slice(&self.enc.ml_kem_768[..]);
        v.extend_from_slice(&self.sign.ed25519);
        v.extend_from_slice(self.sign.second.as_bytes());
        v
    }
    /// Reads [`PublicId::to_bytes`]; an ML-DSA key is required (no classical-only identity).
    pub fn from_bytes(b: &[u8]) -> Result<PublicId> {
        if b.len() != PUB_LEN {
            return Err(Error::Malformed("identity length"));
        }
        let mut r = Reader::new(b);
        let x25519: [u8; X_PK] = r.arr()?;
        let kem: [u8; KEM_PK] = r.arr()?;
        let ed25519: [u8; ED_PK] = r.arr()?;
        let ml: [u8; ML_PK] = r.arr()?;
        Ok(PublicId {
            enc: EncPublic {
                x25519,
                ml_kem_768: Box::new(kem),
            },
            sign: PublicKeys {
                ed25519,
                second: Second::MlDsa65(Box::new(ml)),
            },
        })
    }
    /// The 16-byte fingerprint.
    pub fn fingerprint(&self) -> [u8; 16] {
        let h = wolcrypt::hash::sha512(&self.to_bytes());
        let mut f = [0u8; 16];
        f.copy_from_slice(&h[..16]);
        f
    }
    /// The fingerprint as people compare it: eight groups of four hex digits.
    pub fn fingerprint_text(&self) -> String {
        group_hex(&self.fingerprint())
    }
    /// The one-line public form.
    pub fn to_line(&self) -> String {
        format!("{ID_PREFIX}{}", wire::b64(&self.to_bytes()))
    }
    /// Reads the one-line public form.
    pub fn from_line(s: &str) -> Result<PublicId> {
        let b = s
            .trim()
            .strip_prefix(ID_PREFIX)
            .ok_or(Error::Malformed("not a keyring identity"))?;
        PublicId::from_bytes(&wire::unb64(b, PUB_LEN)?)
    }
}

/// `abcd 1234 ...`.
pub fn group_hex(b: &[u8]) -> String {
    let h = wire::hex(b);
    h.as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// One exported item: metadata (access list dropped) and the secret in clear inside the bundle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exported {
    pub meta: Meta,
    pub secret: String,
}

/// What a bundle carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    /// Unix seconds.
    pub created: u64,
    /// The exporting keyring's name.
    pub from: String,
    pub items: Vec<Exported>,
}

/// How a bundle is encrypted.
pub enum To<'a> {
    /// To a keyring identity (hybrid KEM).
    Recipient(&'a PublicId),
    /// To a passphrase (KDF, then ChaCha20-Poly1305).
    Passphrase(&'a str, Kdf),
}

/// What opening a bundle gives.
pub struct Opened {
    /// The signer, checked: both signature halves verified.
    pub signer: PublicKeys,
    pub payload: Payload,
}

impl Opened {
    /// The signer's fingerprint (first 16 bytes of SHA-512 of ed25519 || ml-dsa-65), as text.
    pub fn signer_text(&self) -> String {
        let mut v = self.signer.ed25519.to_vec();
        v.extend_from_slice(self.signer.second.as_bytes());
        group_hex(&wolcrypt::hash::sha512(&v)[..16])
    }
}

fn pass_parts<'a>(key: &'a [u8; 32], header: &'a [u8]) -> [&'a [u8]; 3] {
    [b"wolos/keyring-bundle/1/pass", key, header]
}

/// Makes a bundle of `payload`, signed by `signer`, encrypted to `to`, binding `context`.
pub fn seal(
    rng: &mut impl Random,
    signer: &Identity,
    payload: &Payload,
    to: To<'_>,
    context: &[u8],
) -> Result<Vec<u8>> {
    if context.len() > MAX_CONTEXT {
        return Err(Error::Limit("context"));
    }
    let mut header = MAGIC.to_vec();
    let pass_salt: [u8; 32] = rng.bytes()?;
    match &to {
        To::Recipient(r) => {
            header.push(1);
            push_chunk(&mut header, context);
            header.extend_from_slice(&r.fingerprint());
        }
        To::Passphrase(p, kdf) => {
            if p.chars().count() < MIN_PASSPHRASE {
                return Err(Error::Policy(
                    "an export passphrase has at least 12 characters",
                ));
            }
            kdf.check()?;
            header.push(2);
            push_chunk(&mut header, context);
            push_chunk(
                &mut header,
                &serde_json::to_vec(kdf).map_err(|e| Error::Io(e.to_string()))?,
            );
            header.extend_from_slice(&pass_salt);
        }
    }
    let body = Zeroizing::new(serde_json::to_vec(payload).map_err(|e| Error::Io(e.to_string()))?);
    let msg = wolcrypt::sign::message_with(SIGN_LABEL, &body, &[&header]);
    let sig = signer.sign(&msg)?;
    let mut plain = Zeroizing::new(Vec::with_capacity(body.len() + 6000));
    push_chunk(&mut plain, &body);
    plain.extend_from_slice(&signer.sign.public().ed25519);
    plain.extend_from_slice(signer.sign.public().second.as_bytes());
    plain.extend_from_slice(&sig.ed25519);
    if sig.second.len() != ML_SIG {
        return Err(Error::Damaged("signature length"));
    }
    plain.extend_from_slice(&sig.second);
    let out = match to {
        To::Recipient(r) => {
            let rnd: [u8; 64] = rng.bytes()?;
            wolcrypt::kem::seal(KEM_FORMAT, &r.enc, &header, &plain, &rnd)
                .map_err(|_| Error::Malformed("recipient key"))?
        }
        To::Passphrase(p, kdf) => {
            let key = kdf.derive(p.as_bytes(), &pass_salt)?;
            let ct = wire::seal(&pass_parts(&key, &header), &header, &plain);
            [header, ct].concat()
        }
    };
    if out.len() > MAX_BUNDLE {
        return Err(Error::Limit("bundle size"));
    }
    Ok(out)
}

fn push_chunk(v: &mut Vec<u8>, b: &[u8]) {
    v.extend_from_slice(&(b.len() as u32).to_be_bytes());
    v.extend_from_slice(b);
}

/// How to open a bundle.
pub enum With<'a> {
    Identity(&'a Identity),
    Passphrase(&'a str),
}

/// Whether a bundle needs a passphrase (mode 2) or a keyring (mode 1); Malformed if not a bundle.
pub fn needs_passphrase(b: &[u8]) -> Result<bool> {
    match b.strip_prefix(MAGIC).and_then(|r| r.first()) {
        Some(1) => Ok(false),
        Some(2) => Ok(true),
        Some(m) => Err(Error::Unsupported(format!("bundle mode {m}"))),
        None => Err(Error::Malformed("not a keyring bundle")),
    }
}

/// Opens and checks a bundle; `context` must be what the sender bound (empty for a file).
pub fn open(b: &[u8], with: With<'_>, context: &[u8]) -> Result<Opened> {
    if b.len() > MAX_BUNDLE {
        return Err(Error::Limit("bundle size"));
    }
    let rest = b
        .strip_prefix(MAGIC)
        .ok_or(Error::Malformed("not a keyring bundle"))?;
    let mut r = Reader::new(rest);
    let mode = r.take(1)?[0];
    let ctx = r.chunk(MAX_CONTEXT)?;
    if !wolcrypt::ct::eq(ctx, context) {
        return Err(Error::Damaged("the bundle was made for another transfer"));
    }
    let plain = match (mode, with) {
        (1, With::Identity(id)) => {
            let fp: [u8; 16] = r.arr()?;
            if fp != id.public().fingerprint() {
                return Err(Error::WrongFactors);
            }
            let hlen = MAGIC.len() + r.pos();
            wolcrypt::kem::open(KEM_FORMAT, id.enc(), b, hlen)
                .map_err(|_| Error::Damaged("bundle"))?
        }
        (2, With::Passphrase(p)) => {
            let kdf: Kdf = serde_json::from_slice(r.chunk(256)?)
                .map_err(|_| Error::Malformed("bundle kdf"))?;
            let salt: [u8; 32] = r.arr()?;
            let hlen = MAGIC.len() + r.pos();
            let key = kdf.derive(p.as_bytes(), &salt)?;
            let header = &b[..hlen];
            wire::open(&pass_parts(&key, header), header, &b[hlen..]).ok_or(Error::WrongFactors)?
        }
        (1 | 2, _) => return Err(Error::WrongFactors),
        (m, _) => return Err(Error::Unsupported(format!("bundle mode {m}"))),
    };
    let header_len = b.len() - (rest.len() - r.pos());
    let header = &b[..header_len];
    let mut p = Reader::new(&plain);
    let body = p.chunk(MAX_BUNDLE)?;
    let ed25519: [u8; ED_PK] = p.arr()?;
    let ml: [u8; ML_PK] = p.arr()?;
    let sig_ed: [u8; ED_SIG] = p.arr()?;
    let sig_ml = p.take(ML_SIG)?.to_vec();
    if p.rest() != 0 {
        return Err(Error::Malformed("bundle trailing bytes"));
    }
    let signer = PublicKeys {
        ed25519,
        second: Second::MlDsa65(Box::new(ml)),
    };
    let msg = wolcrypt::sign::message_with(SIGN_LABEL, body, &[header]);
    wolcrypt::sign::verify_message(
        &signer,
        &msg,
        &Sig {
            ed25519: sig_ed,
            second: sig_ml,
        },
    )
    .map_err(|_| Error::Damaged("bundle signature"))?;
    let payload: Payload =
        serde_json::from_slice(body).map_err(|_| Error::Malformed("bundle payload"))?;
    for i in &payload.items {
        i.meta.check()?;
    }
    Ok(Opened { signer, payload })
}
