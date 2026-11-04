// SPDX-License-Identifier: BSD-2-Clause
//! The trust store held in memory (ADR 0046, interface I2): pinned roots, known intermediates and
//! the revocation lists whose signature checked out. Loading takes bytes, not paths: the system
//! reads `/usr/share/wolos/trust/` and `/var/lib/wolos/trust/`, the hub Worker reads its own
//! storage, and both hand the same bytes here. Anything that does not check out is left out and
//! named in `notes`, never trusted.
use crate::cert::{Cert, Kind, Usage};
use crate::crypto::{self, Purpose, Wsig};
use crate::revoked::{self, Freshness, RevList};
use crate::verify;

/// Roots, intermediates and verified revocation lists.
#[derive(Clone, Debug, Default)]
pub struct Store {
    /// Pinned roots (A and B), each self-signed.
    pub roots: Vec<Cert>,
    /// Intermediates (and re-issued leaves) known from the image or the signed index.
    pub intermediates: Vec<Cert>,
    /// Revocation lists with a verified signature.
    pub lists: Vec<RevList>,
    /// Entries remembered from lists verified before ([`Store::remember`]): consulted by every
    /// check like the lists, never for freshness.
    pub remembered: Vec<RevList>,
    /// Why something was left out (for a log, never shown).
    pub notes: Vec<String>,
}

impl Store {
    /// Builds a store at `now`. `lists` are `(revoked.json, revoked.json.wsig)` pairs. A root whose
    /// subject label starts with `test` is refused unless `allow_test` (tests and the vectors).
    pub fn load(
        roots: &[impl AsRef<[u8]>],
        intermediates: &[impl AsRef<[u8]>],
        lists: &[(impl AsRef<[u8]>, impl AsRef<[u8]>)],
        now: u64,
        allow_test: bool,
    ) -> Store {
        let mut s = Store::default();
        for (i, raw) in roots.iter().enumerate() {
            match Cert::parse(raw.as_ref()) {
                Ok(c) if c.body.kind == Kind::Root => {
                    let msg = crypto::message(Purpose::Cert, &c.body_bytes);
                    let is_test = c
                        .body
                        .subject
                        .label
                        .as_deref()
                        .is_some_and(|l| l.starts_with(crate::TEST_LABEL));
                    if crypto::verify_hybrid(&c.keys, &msg, &c.sig).is_err() {
                        s.notes
                            .push(format!("root {i}: self-signature does not verify"));
                    } else if is_test && !allow_test {
                        s.notes
                            .push(format!("root {i}: a test root is never loaded"));
                    } else if !s.roots.iter().any(|r| r.body.serial == c.body.serial) {
                        s.roots.push(c);
                    }
                }
                Ok(_) => s.notes.push(format!("root {i}: not a root")),
                Err(e) => s.notes.push(format!("root {i}: {e}")),
            }
        }
        for (i, raw) in intermediates.iter().enumerate() {
            match Cert::parse(raw.as_ref()) {
                Ok(c) if c.body.kind != Kind::Root => {
                    if !s
                        .intermediates
                        .iter()
                        .any(|r| r.body.serial == c.body.serial)
                    {
                        s.intermediates.push(c);
                    }
                }
                Ok(_) => s
                    .notes
                    .push(format!("intermediate {i}: a root does not belong here")),
                Err(e) => s.notes.push(format!("intermediate {i}: {e}")),
            }
        }
        // Lists signed by a root first: they can revoke the Index intermediate that signs the rest.
        for want_root in [true, false] {
            for (i, (list, sig)) in lists.iter().enumerate() {
                if let Some(l) = s.check_list(list.as_ref(), sig.as_ref(), now, want_root, i) {
                    s.lists.push(l);
                }
            }
        }
        s
    }

    fn check_list(
        &mut self,
        list: &[u8],
        sig: &[u8],
        now: u64,
        want_root: bool,
        i: usize,
    ) -> Option<RevList> {
        let parsed = match revoked::parse(list) {
            Ok(l) => l,
            Err(e) => {
                if want_root {
                    self.notes.push(format!("list {i}: {e}"));
                }
                return None;
            }
        };
        let signer = match Wsig::parse(sig).ok().and_then(|w| Some((w.sig()?, w.cert))) {
            Some((s, serial)) => (s, serial),
            None => {
                if want_root {
                    self.notes.push(format!("list {i}: signature file damaged"));
                }
                return None;
            }
        };
        let cert = self
            .roots
            .iter()
            .chain(self.intermediates.iter())
            .find(|c| c.body.serial == signer.1)?
            .clone();
        let cert = &cert;
        let is_root = cert.body.kind == Kind::Root;
        if is_root != want_root {
            return None;
        }
        let fail = |this: &mut Store, why: &str| {
            this.notes.push(format!("list {i}: {why}"));
            None
        };
        if !cert.body.usage.contains(&Usage::SignRevocations) {
            return fail(self, "signer may not sign revocations");
        }
        let msg = crypto::message(Purpose::Revocation, list);
        if crypto::verify_hybrid(&cert.keys, &msg, &signer.0).is_err() {
            return fail(self, "signature does not verify");
        }
        // The signer must have been valid when it issued the list (audit M2: an Index that
        // expired since does not drop what it withdrew), and the list cannot come from the future.
        if parsed.issued > now.saturating_add(verify::CLOCK_SKEW) {
            return fail(self, "list issued in the future");
        }
        let at = parsed.issued;
        if !is_root {
            let ok = verify::path(cert, &[], self)
                .ok()
                .filter(|p| p.iter().all(|c| c.valid_at(at)))
                .filter(|p| verify::revocation(p, &self.all_lists(), None).is_none());
            if ok.is_none() {
                return fail(self, "signer is not a valid, unrevoked intermediate");
            }
        } else if !cert.valid_at(at) {
            return fail(self, "signing root was not valid when it issued the list");
        }
        Some(RevList {
            list: parsed,
            signer: cert.body.serial.clone(),
            signer_is_root: is_root,
        })
    }

    /// Adds the memory of seen revocations (audit M1). Entries whose signer is no root or
    /// intermediate this store knows are left out, with a note.
    pub fn remember(&mut self, seen: &revoked::Seen) {
        for l in seen.as_lists() {
            let known = self.roots.iter().chain(self.intermediates.iter()).any(|c| {
                c.body.serial == l.signer && (c.body.kind == Kind::Root) == l.signer_is_root
            });
            if known {
                self.remembered.push(l);
            } else {
                self.notes
                    .push(format!("remembered entries by unknown signer {}", l.signer));
            }
        }
    }

    /// The memory after this store's lists: what a machine must keep (`seen-revocations.json`).
    pub fn seen(&self, old: &revoked::Seen) -> revoked::Seen {
        let mut s = old.clone();
        for l in &self.lists {
            s.add(l);
        }
        s
    }

    /// Every list the checks consult: the verified lists, then the remembered entries.
    pub fn all_lists(&self) -> Vec<&RevList> {
        self.lists.iter().chain(self.remembered.iter()).collect()
    }

    /// The list with the highest serial (a higher serial replaces a lower one, never the reverse).
    pub fn best_list(&self) -> Option<&RevList> {
        self.lists.iter().max_by_key(|l| l.list.serial)
    }

    /// Freshness of the best list at `now`.
    pub fn freshness(&self, now: u64) -> Freshness {
        self.best_list().map_or(Freshness::Missing, |l| {
            revoked::freshness(l.list.expires, now)
        })
    }
}
