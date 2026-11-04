// SPDX-License-Identifier: BSD-2-Clause
//! The keyring's audit log (ADR 0054 section 6): one JSON line per event, append-only, each line
//! carrying the chain value of the line before (`derive(FORMAT "/audit", previous value, previous
//! line)`, first 32 bytes, hex). Checkpoint lines also carry the line count and the store's
//! position (epoch, record count, chain value) and are signed with the keyring's hybrid key
//! (Ed25519 + ML-DSA-65, label `wolos/keyring/1/audit`) over the line without its signature.
//!
//! [`verify`] checks the chain and every checkpoint's signature and reports the last signed
//! line: lines after it are "not yet signed"; a store whose record count is below the last
//! checkpoint's was rolled back. Items are named by their random id, never their label.
use serde::{Deserialize, Serialize};
use wolcrypt::sign::{PublicKeys, Sig};

use crate::bundle::Identity;
use crate::wire;
use crate::{Error, Result};

/// The signature's label.
pub const LABEL: &str = "wolos/keyring/1/audit";
/// A checkpoint after this many lines (and at every unlock and lock).
pub const CHECKPOINT_EVERY: u64 = 64;
/// Longest line read.
pub const MAX_LINE: usize = 16 * 1024;
/// Largest log read (older lines are rotated into `<name>.audit.1` past this).
pub const MAX_LOG: usize = 16 << 20;

/// What happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Event {
    Unlock,
    UnlockRefused,
    Lock,
    Read,
    ReadDenied,
    Write,
    Delete,
    Export,
    Import,
    MoveOut,
    MoveIn,
    Acl,
    Policy,
    Settings,
    Migrate,
    Checkpoint,
}

/// A store position as a checkpoint records it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorePos {
    pub epoch: u64,
    pub records: u64,
    pub chain: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SigJ {
    ed25519: String,
    ml_dsa_65: String,
}

/// One line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub seq: u64,
    pub time: u64,
    pub event: Event,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub prev: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<StorePos>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sig: Option<SigJ>,
}

impl Line {
    /// Whether the line is a signed checkpoint.
    pub fn signed(&self) -> bool {
        self.sig.is_some()
    }
}

fn next(prev: &[u8; 32], line: &[u8]) -> [u8; 32] {
    let h = wire::derive(&[format!("{}/audit", crate::FORMAT).as_bytes(), prev, line]);
    let mut o = [0u8; 32];
    o.copy_from_slice(&h[..32]);
    o
}

fn genesis() -> [u8; 32] {
    next(&[0; 32], b"genesis")
}

/// The writer's state: the next sequence number and chain value.
#[derive(Clone, Debug)]
pub struct Log {
    seq: u64,
    head: [u8; 32],
    since_checkpoint: u64,
    last_checkpoint: Option<u64>,
}

impl Default for Log {
    fn default() -> Self {
        Log {
            seq: 0,
            head: genesis(),
            since_checkpoint: 0,
            last_checkpoint: None,
        }
    }
}

impl Log {
    /// Continues an existing log: the chain is checked (signatures are [`verify`]'s job).
    pub fn resume(text: &str) -> Result<Log> {
        let mut log = Log::default();
        for l in parse(text)? {
            log.step(&l)?;
        }
        Ok(log)
    }

    fn step(&mut self, l: &Line) -> Result<()> {
        if l.seq != self.seq || l.prev != wire::hex(&self.head) {
            return Err(Error::Damaged("audit chain"));
        }
        let bytes = serde_json::to_vec(l).map_err(|e| Error::Io(e.to_string()))?;
        self.head = next(&self.head, &bytes);
        self.seq += 1;
        if l.event == Event::Checkpoint {
            self.last_checkpoint = Some(l.seq);
        }
        self.since_checkpoint = if l.event == Event::Checkpoint {
            0
        } else {
            self.since_checkpoint + 1
        };
        Ok(())
    }

    /// One event line (with its newline).
    pub fn append(
        &mut self,
        time: u64,
        event: Event,
        app: Option<&str>,
        item: Option<&str>,
        detail: Option<&str>,
    ) -> Result<String> {
        let clip = |s: &str| s.chars().take(256).collect::<String>();
        let l = Line {
            seq: self.seq,
            time,
            event,
            app: app.map(clip),
            item: item.map(clip),
            detail: detail.map(clip),
            prev: wire::hex(&self.head),
            store: None,
            sig: None,
        };
        self.emit(l)
    }

    fn emit(&mut self, l: Line) -> Result<String> {
        self.step(&l)?;
        let mut s = serde_json::to_string(&l).map_err(|e| Error::Io(e.to_string()))?;
        s.push('\n');
        Ok(s)
    }

    /// Sequence number of the last checkpoint line in the log (signed or not: [`verify`] checks).
    pub fn last_checkpoint(&self) -> Option<u64> {
        self.last_checkpoint
    }

    /// Whether a checkpoint is due.
    pub fn due(&self) -> bool {
        self.since_checkpoint >= CHECKPOINT_EVERY
    }

    /// A signed checkpoint line binding the store's position.
    pub fn checkpoint(
        &mut self,
        time: u64,
        id: &Identity,
        pos: (u64, u64, [u8; 64]),
    ) -> Result<String> {
        let mut l = Line {
            seq: self.seq,
            time,
            event: Event::Checkpoint,
            app: None,
            item: None,
            detail: None,
            prev: wire::hex(&self.head),
            store: Some(StorePos {
                epoch: pos.0,
                records: pos.1,
                chain: wire::hex(&pos.2),
            }),
            sig: None,
        };
        let msg = signed_message(&l)?;
        let sig = id.sign(&msg)?;
        l.sig = Some(SigJ {
            ed25519: wire::b64(&sig.ed25519),
            ml_dsa_65: wire::b64(&sig.second),
        });
        self.emit(l)
    }
}

fn signed_message(l: &Line) -> Result<Vec<u8>> {
    let mut u = l.clone();
    u.sig = None;
    let b = serde_json::to_vec(&u).map_err(|e| Error::Io(e.to_string()))?;
    Ok(wolcrypt::sign::message(LABEL, &b))
}

/// Every line of a log (bounded).
pub fn parse(text: &str) -> Result<Vec<Line>> {
    if text.len() > MAX_LOG {
        return Err(Error::Limit("audit log size"));
    }
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            if l.len() > MAX_LINE {
                return Err(Error::Limit("audit line"));
            }
            serde_json::from_str(l).map_err(|_| Error::Malformed("audit line"))
        })
        .collect()
}

/// What [`verify`] found.
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub lines: u64,
    /// Sequence number of the last signed checkpoint.
    pub last_signed: Option<u64>,
    /// Lines after it.
    pub unsigned_after: u64,
    /// The store has fewer records (same epoch) or an older epoch than the last checkpoint.
    pub rollback: bool,
}

/// Checks the chain and every checkpoint signature against `signer`; `store` is the store's
/// current position (epoch, records) when known.
pub fn verify(text: &str, signer: &PublicKeys, store: Option<(u64, u64)>) -> Result<Report> {
    let mut log = Log::default();
    let mut last = None;
    let mut last_pos: Option<StorePos> = None;
    let lines = parse(text)?;
    for l in &lines {
        log.step(l)?;
        if let Some(s) = &l.sig {
            let sig = Sig {
                ed25519: wire::unb64_n(&s.ed25519)?,
                second: wire::unb64(&s.ml_dsa_65, wolcrypt::sign::ML_SIG)?,
            };
            wolcrypt::sign::verify_message(signer, &signed_message(l)?, &sig)
                .map_err(|_| Error::Damaged("audit checkpoint signature"))?;
            last = Some(l.seq);
            last_pos = l.store.clone();
        } else if l.event == Event::Checkpoint {
            return Err(Error::Damaged("unsigned checkpoint"));
        }
    }
    let n = lines.len() as u64;
    let rollback = match (store, &last_pos) {
        (Some((e, r)), Some(p)) => e < p.epoch || (e == p.epoch && r < p.records),
        _ => false,
    };
    Ok(Report {
        lines: n,
        last_signed: last,
        unsigned_after: last.map_or(n, |s| n - s - 1),
        rollback,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Identity, String) {
        let id = Identity::from_seed(&[3; 32]);
        let mut log = Log::default();
        let mut t = String::new();
        t += &log
            .append(1, Event::Unlock, Some("wol-lock"), None, None)
            .unwrap();
        t += &log
            .append(2, Event::Read, Some("helium"), Some("ab"), None)
            .unwrap();
        t += &log.checkpoint(3, &id, (1, 5, [0; 64])).unwrap();
        t += &log
            .append(4, Event::Write, Some("helium"), Some("ab"), None)
            .unwrap();
        (id, t)
    }

    #[test]
    fn a_good_log_verifies() {
        let (id, t) = sample();
        let r = verify(&t, &id.public().sign, Some((1, 6))).unwrap();
        assert_eq!(
            r,
            Report {
                lines: 4,
                last_signed: Some(2),
                unsigned_after: 1,
                rollback: false
            }
        );
        let mut log = Log::resume(&t).unwrap();
        assert!(
            log.append(5, Event::Lock, None, None, None)
                .unwrap()
                .contains("\"seq\":4")
        );
    }

    #[test]
    fn an_edited_line_breaks_the_chain() {
        let (id, t) = sample();
        let bad = t.replacen("helium", "helion", 1);
        assert_eq!(
            verify(&bad, &id.public().sign, None),
            Err(Error::Damaged("audit chain"))
        );
        let dropped: String = t
            .lines()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, l)| format!("{l}\n"))
            .collect();
        assert!(verify(&dropped, &id.public().sign, None).is_err());
    }

    #[test]
    fn a_checkpoint_by_another_key_is_refused() {
        let (_, t) = sample();
        let other = Identity::from_seed(&[4; 32]);
        assert_eq!(
            verify(&t, &other.public().sign, None),
            Err(Error::Damaged("audit checkpoint signature"))
        );
    }

    #[test]
    fn a_store_behind_its_checkpoint_is_a_rollback() {
        let (id, t) = sample();
        assert!(
            verify(&t, &id.public().sign, Some((1, 4)))
                .unwrap()
                .rollback
        );
        assert!(
            verify(&t, &id.public().sign, Some((0, 99)))
                .unwrap()
                .rollback
        );
        assert!(
            !verify(&t, &id.public().sign, Some((2, 0)))
                .unwrap()
                .rollback
        );
    }
}
