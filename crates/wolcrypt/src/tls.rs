// SPDX-License-Identifier: BSD-2-Clause
//! The hybrid TLS 1.3 key exchange X25519MLKEM768 (codepoint 0x11EC, draft-ietf-tls-ecdhe-mlkem),
//! client side, as a key-exchange group a TLS stack can be given (feature `tls`; ADR 0052
//! "Transport"). Built from this crate's own X25519 ([`crate::kem::x25519`]) and fips203's
//! ML-KEM-768, so the OS has one ML-KEM. Layout (ML-KEM first): the client's share is the ML-KEM-768
//! encapsulation key (1184 bytes) followed by its X25519 public key (32); the server's is the
//! ML-KEM-768 ciphertext (1088) followed by its X25519 public key (32); the shared secret is the
//! ML-KEM shared secret (32) followed by the X25519 one (32). A break of either half alone does not
//! reveal the secret. Random bytes come from the caller's generator (this crate never draws any).
use crate::kem::{x25519, x25519_public};
use crate::take;
use fips203::ml_kem_768;
use fips203::traits::{Decaps as _, KeyGen as _, SerDes as _};
use rustls::crypto::{ActiveKeyExchange, SecureRandom, SharedSecret, SupportedKxGroup};
use rustls::{Error, NamedGroup, PeerMisbehaved};
use std::boxed::Box;
use std::vec::Vec;
use zeroize::Zeroizing;

const EK: usize = ml_kem_768::EK_LEN;
const CT: usize = ml_kem_768::CT_LEN;
const X: usize = 32;

/// The group, drawing its ephemeral secrets from `rng` (the TLS provider's generator).
#[derive(Debug)]
pub struct X25519MlKem768 {
    /// Where the 96 random bytes of each exchange come from.
    pub rng: &'static dyn SecureRandom,
}

/// One exchange from fixed secrets: `x` the X25519 secret, `d` and `z` the ML-KEM-768 seeds
/// (FIPS 203 `ML-KEM.KeyGen_internal`). For known-answer tests; [`X25519MlKem768`] draws them.
pub fn start_from_seed(x: [u8; 32], d: [u8; 32], z: [u8; 32]) -> Box<dyn ActiveKeyExchange> {
    let (ek, dk) = ml_kem_768::KG::keygen_from_seed(d, z);
    let mut share = Vec::with_capacity(EK + X);
    share.extend_from_slice(&ek.into_bytes());
    share.extend_from_slice(&x25519_public(&x));
    Box::new(Active {
        x: Zeroizing::new(x),
        dk: Box::new(dk),
        share,
    })
}

impl SupportedKxGroup for X25519MlKem768 {
    fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
        let mut r = Zeroizing::new([0u8; 96]);
        self.rng.fill(&mut r[..])?;
        Ok(start_from_seed(
            take::<32>(&r[..], 0),
            take::<32>(&r[..], 32),
            take::<32>(&r[..], 64),
        ))
    }

    fn name(&self) -> NamedGroup {
        NamedGroup::X25519MLKEM768
    }
}

struct Active {
    x: Zeroizing<[u8; 32]>,
    dk: Box<ml_kem_768::DecapsKey>,
    share: Vec<u8>,
}

impl ActiveKeyExchange for Active {
    fn complete(self: Box<Self>, peer: &[u8]) -> Result<SharedSecret, Error> {
        let bad = || Error::PeerMisbehaved(PeerMisbehaved::InvalidKeyShare);
        if peer.len() != CT + X {
            return Err(bad());
        }
        let ct = ml_kem_768::CipherText::try_from_bytes(take::<CT>(peer, 0)).map_err(|_| bad())?;
        let ss_m = Zeroizing::new(self.dk.try_decaps(&ct).map_err(|_| bad())?.into_bytes());
        // A low-order X25519 point (an all-zero secret) is refused.
        let ss_x = x25519(&self.x, &take::<X>(peer, CT)).ok_or_else(bad)?;
        let mut secret = Vec::with_capacity(64);
        secret.extend_from_slice(&ss_m[..]);
        secret.extend_from_slice(&ss_x[..]);
        Ok(SharedSecret::from(secret))
    }

    fn pub_key(&self) -> &[u8] {
        &self.share
    }

    fn group(&self) -> NamedGroup {
        NamedGroup::X25519MLKEM768
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fips203::traits::Encaps as _;

    /// The server's side done independently (FIPS 203 encapsulation, X25519) agrees with the
    /// client's secret, and the shares have the draft's sizes and order.
    #[test]
    fn the_client_and_an_independent_server_agree() {
        let (x, d, z) = ([7u8; 32], [1u8; 32], [2u8; 32]);
        let active = start_from_seed(x, d, z);
        let share = active.pub_key().to_vec();
        assert_eq!(share.len(), EK + X);
        assert_eq!(active.group(), NamedGroup::X25519MLKEM768);
        let (ek, _) = ml_kem_768::KG::keygen_from_seed(d, z);
        assert_eq!(&share[..EK], &ek.clone().into_bytes()[..], "ML-KEM first");
        assert_eq!(&share[EK..], &x25519_public(&x)[..], "X25519 second");
        // The server: encapsulate to the client's key, its own X25519 pair.
        let server_ek = ml_kem_768::EncapsKey::try_from_bytes(take::<EK>(&share, 0)).unwrap();
        let (ss_m, ct) = server_ek.encaps_from_seed(&[3u8; 32]);
        let sx = [9u8; 32];
        let ss_x = x25519(&sx, &x25519_public(&x)).unwrap();
        let mut reply = ct.into_bytes().to_vec();
        reply.extend_from_slice(&x25519_public(&sx));
        let mut want = ss_m.into_bytes().to_vec();
        want.extend_from_slice(&ss_x[..]);
        let got = active.complete(&reply).unwrap();
        assert_eq!(got.secret_bytes(), &want[..]);
    }

    fn hex(b: &[u8]) -> String {
        crate::hash::sha256(b)
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect()
    }

    /// Known answer for fixed secrets: SHA-256 of the client's share and of the shared secret
    /// (pinned from the first run, after the independent-server test above agreed with it), so a
    /// change of the layout, the order of the halves or either half's derivation is caught.
    #[test]
    fn known_answer_for_fixed_secrets() {
        let (x, d, z) = ([7u8; 32], [1u8; 32], [2u8; 32]);
        let active = start_from_seed(x, d, z);
        assert_eq!(
            hex(active.pub_key()),
            "f2fd3dbe0a9ac9cdf907655f24c36fc0aa27b679d53c42c500dff1f3cc0d0c08"
        );
        let server_ek =
            ml_kem_768::EncapsKey::try_from_bytes(take::<EK>(active.pub_key(), 0)).unwrap();
        let (_, ct) = server_ek.encaps_from_seed(&[3u8; 32]);
        let mut reply = ct.into_bytes().to_vec();
        reply.extend_from_slice(&x25519_public(&[9u8; 32]));
        assert_eq!(
            hex(&reply),
            "f6dafec1e8c598899083bd0c9067c1cd8e01cf819e7c2287db813282032b9aea"
        );
        let got = active.complete(&reply).unwrap();
        assert_eq!(
            hex(got.secret_bytes()),
            "33fa8ec4da4608cff4cb8b8843a6476749906a63a5b96ad5a293ed6a810f6e6e"
        );
    }

    #[test]
    fn a_malformed_or_low_order_share_is_refused() {
        let short = start_from_seed([7; 32], [1; 32], [2; 32]);
        assert!(short.complete(&[0u8; CT + X - 1]).is_err());
        // A valid-length reply whose X25519 half is the all-zero (low-order) point.
        let a = start_from_seed([7; 32], [1; 32], [2; 32]);
        let ek = ml_kem_768::EncapsKey::try_from_bytes(take::<EK>(a.pub_key(), 0)).unwrap();
        let (_, ct) = ek.encaps_from_seed(&[3u8; 32]);
        let mut reply = ct.into_bytes().to_vec();
        reply.extend_from_slice(&[0u8; 32]);
        assert!(a.complete(&reply).is_err());
    }
}
