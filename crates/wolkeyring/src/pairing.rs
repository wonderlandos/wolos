// SPDX-License-Identifier: BSD-2-Clause
//! `wol keyring move`: two WolOS machines on one LAN, no server (ADR 0054 section 7).
//! Commit-then-reveal with a short authentication string, as Bluetooth numeric comparison:
//!
//! 1. receiver -> sender: `C = derive(FORMAT "/move/commit", receiver's fresh public identity, n_R)`
//! 2. sender -> receiver: `n_S` (32 random bytes), fixed before the receiver reveals
//! 3. receiver -> sender: the public identity and `n_R`; the sender checks `C`
//! 4. both show [`Sas`]: 8 digits from `derive(FORMAT "/move/sas", C, n_S, identity, n_R)`; the
//!    person confirms they match on both screens
//! 5. sender -> receiver: a bundle sealed to that identity, its context the transcript hash
//!
//! A man in the middle must fix his values before he learns the other side's, so he matches the
//! 8 digits with probability 10^-8 per attempt, and there is nothing to compute offline. Every
//! step is a hash or the hybrid KEM: it holds against a quantum adversary.
use crate::bundle::{self, Identity, Opened, Payload, PublicId, To, With};
use crate::rand::Random;
use crate::wire;
use crate::{Error, Result};

/// First bytes of message 1.
pub const MAGIC: &[u8; 4] = b"WKM1";
/// The TCP port the receiver listens on by default.
pub const PORT: u16 = 47_854;
/// Seconds the receiver waits for the whole transfer.
pub const DEADLINE_SECS: u64 = 600;

/// The 8 digits both screens show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sas(pub String);

fn label(what: &str) -> Vec<u8> {
    format!("{}/move/{what}", crate::FORMAT).into_bytes()
}

fn commit(id: &[u8], n_r: &[u8; 32]) -> [u8; 64] {
    *wire::derive(&[&label("commit"), id, n_r])
}

fn sas(c: &[u8; 64], n_s: &[u8; 32], id: &[u8], n_r: &[u8; 32]) -> Sas {
    let h = wire::derive(&[&label("sas"), c, n_s, id, n_r]);
    let mut b = [0u8; 8];
    b.copy_from_slice(&h[..8]);
    let n = u64::from_be_bytes(b) % 100_000_000;
    let s = format!("{n:08}");
    Sas(format!("{} {}", &s[..4], &s[4..]))
}

fn transcript(c: &[u8; 64], n_s: &[u8; 32], id: &[u8], n_r: &[u8; 32]) -> [u8; 64] {
    *wire::derive(&[&label("transcript"), c, n_s, id, n_r])
}

/// The new machine's side.
pub struct Receiver {
    id: Identity,
    n_r: [u8; 32],
    c: [u8; 64],
    transcript: Option<[u8; 64]>,
}

impl Receiver {
    /// A fresh identity and message 1.
    pub fn start(rng: &mut impl Random) -> Result<(Receiver, Vec<u8>)> {
        let seed: [u8; 32] = rng.bytes()?;
        let id = Identity::from_seed(&seed);
        let n_r: [u8; 32] = rng.bytes()?;
        let c = commit(&id.public().to_bytes(), &n_r);
        let msg = [&MAGIC[..], &c].concat();
        Ok((
            Receiver {
                id,
                n_r,
                c,
                transcript: None,
            },
            msg,
        ))
    }

    /// Message 2 in, message 3 out, and the digits to show.
    pub fn reveal(&mut self, msg2: &[u8]) -> Result<(Vec<u8>, Sas)> {
        let n_s: [u8; 32] = msg2
            .try_into()
            .map_err(|_| Error::Malformed("move message 2"))?;
        let idb = self.id.public().to_bytes();
        self.transcript = Some(transcript(&self.c, &n_s, &idb, &self.n_r));
        let s = sas(&self.c, &n_s, &idb, &self.n_r);
        Ok(([idb, self.n_r.to_vec()].concat(), s))
    }

    /// The bundle (message 5), checked against this transfer; call only after the person
    /// confirmed the digits.
    pub fn open(&self, bundle: &[u8]) -> Result<Opened> {
        let t = self
            .transcript
            .ok_or(Error::Malformed("move out of order"))?;
        bundle::open(bundle, With::Identity(&self.id), &t)
    }
}

/// The old machine's side.
pub struct Sender {
    c: [u8; 64],
    n_s: [u8; 32],
    peer: Option<(PublicId, [u8; 64])>,
}

impl Sender {
    /// Message 1 in, message 2 out.
    pub fn start(rng: &mut impl Random, msg1: &[u8]) -> Result<(Sender, Vec<u8>)> {
        let c = msg1
            .strip_prefix(&MAGIC[..])
            .ok_or(Error::Malformed("not a keyring move"))?;
        let c: [u8; 64] = c
            .try_into()
            .map_err(|_| Error::Malformed("move message 1"))?;
        let n_s: [u8; 32] = rng.bytes()?;
        Ok((Sender { c, n_s, peer: None }, n_s.to_vec()))
    }

    /// Message 3 in: the commitment checked, the digits to show.
    pub fn check(&mut self, msg3: &[u8]) -> Result<Sas> {
        let idlen = msg3
            .len()
            .checked_sub(32)
            .ok_or(Error::Malformed("move message 3"))?;
        let (idb, n_r) = msg3.split_at(idlen);
        let n_r: [u8; 32] = n_r
            .try_into()
            .map_err(|_| Error::Malformed("move message 3"))?;
        if !wolcrypt::ct::eq(&commit(idb, &n_r), &self.c) {
            return Err(Error::Damaged(
                "the other machine changed its key after committing",
            ));
        }
        let id = PublicId::from_bytes(idb)?;
        let t = transcript(&self.c, &self.n_s, idb, &n_r);
        let s = sas(&self.c, &self.n_s, idb, &n_r);
        self.peer = Some((id, t));
        Ok(s)
    }

    /// Message 5: the payload, signed by this keyring, sealed to the receiver; call only after the
    /// person confirmed the digits.
    pub fn seal(
        &self,
        rng: &mut impl Random,
        signer: &Identity,
        payload: &Payload,
    ) -> Result<Vec<u8>> {
        let (id, t) = self
            .peer
            .as_ref()
            .ok_or(Error::Malformed("move out of order"))?;
        bundle::seal(rng, signer, payload, To::Recipient(id), t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rand::Counter;

    fn payload() -> Payload {
        Payload {
            created: 1,
            from: "login".into(),
            items: Vec::new(),
        }
    }

    #[test]
    fn both_sides_show_the_same_digits_and_the_bundle_opens() {
        let mut ra = Counter::new(1);
        let mut rb = Counter::new(2);
        let (mut r, m1) = Receiver::start(&mut ra).unwrap();
        let (mut s, m2) = Sender::start(&mut rb, &m1).unwrap();
        let (m3, sas_r) = r.reveal(&m2).unwrap();
        let sas_s = s.check(&m3).unwrap();
        assert_eq!(sas_r, sas_s);
        assert_eq!(sas_r.0.len(), 9);
        let me = Identity::from_seed(&[5; 32]);
        let b = s.seal(&mut rb, &me, &payload()).unwrap();
        let o = r.open(&b).unwrap();
        assert_eq!(o.payload, payload());
        assert_eq!(o.signer, me.public().sign);
    }

    #[test]
    fn a_swapped_key_after_the_commitment_is_refused() {
        let mut ra = Counter::new(1);
        let mut rb = Counter::new(2);
        let (mut r, m1) = Receiver::start(&mut ra).unwrap();
        let (mut s, m2) = Sender::start(&mut rb, &m1).unwrap();
        let (mut m3, _) = r.reveal(&m2).unwrap();
        m3[10] ^= 1;
        assert!(matches!(s.check(&m3), Err(Error::Damaged(_))));
    }

    #[test]
    fn a_man_in_the_middle_gets_other_digits() {
        // The attacker answers the receiver with his own n_S and relays the rest.
        let mut ra = Counter::new(1);
        let mut rb = Counter::new(2);
        let mut rm = Counter::new(3);
        let (mut r, m1) = Receiver::start(&mut ra).unwrap();
        let (mut m_as_receiver, m1x) = Receiver::start(&mut rm).unwrap();
        let (mut s, m2) = Sender::start(&mut rb, &m1x).unwrap();
        let (_mitm_sender, m2x) = Sender::start(&mut rm, &m1).unwrap();
        let (_m3, sas_r) = r.reveal(&m2x).unwrap();
        let (m3x, _) = m_as_receiver.reveal(&m2).unwrap();
        let sas_s = s.check(&m3x).unwrap();
        assert_ne!(sas_r, sas_s);
    }

    #[test]
    fn a_bundle_from_another_transfer_is_refused() {
        let mut ra = Counter::new(1);
        let mut rb = Counter::new(2);
        let (mut r1, m1) = Receiver::start(&mut ra).unwrap();
        let (mut s1, m2) = Sender::start(&mut rb, &m1).unwrap();
        let (m3, _) = r1.reveal(&m2).unwrap();
        s1.check(&m3).unwrap();
        let me = Identity::from_seed(&[5; 32]);
        let b = s1.seal(&mut rb, &me, &payload()).unwrap();
        // Same receiver, a second run with a new n_S: the old bundle no longer fits.
        let (_s2, m2b) = Sender::start(&mut rb, &m1).unwrap();
        r1.reveal(&m2b).unwrap();
        assert!(r1.open(&b).is_err());
    }
}
