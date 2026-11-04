// SPDX-License-Identifier: BSD-2-Clause
//! The trust store held in memory (ADR 0046, interface I2): pinned roots, known intermediates and
//! the revocation lists whose signature checked out. Loading takes bytes, not paths: the system
//! reads `/usr/share/wolos/trust/` and `/var/lib/wolos/trust/`, the hub Worker reads its own
//! storage, and both hand the same bytes here. Anything that does not check out is left out and
//! named in `notes`, never trusted.
use crate::cert::{Cert, Kind, Usage};
use crate::crypto::{self, Purpose, Wsig};
use crate::revoked::{self, Freshness, RevList};
use crate::rkdb;
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
    /// Revocation databases (`revoked.rkdb`) whose Index signature verified.
    pub dbs: Vec<HeldDb>,
    /// Root-signed sections (emergency withdrawals of intermediates and roots) that verified,
    /// from a database or on their own (`.rkdr`).
    pub root_sections: Vec<HeldRoot>,
    /// The lowest database serial this machine accepts (the image's, and every `rkdb_serial` it
    /// saw in a signed release manifest, a file certificate or the checkpoints). A store whose
    /// newest database is below it is behind: [`Store::freshness`] says stale at best.
    pub floor: u64,
    /// Why something was left out (for a log, never shown).
    pub notes: Vec<String>,
}

/// A revocation database whose signature verified.
#[derive(Clone, Debug)]
pub struct HeldDb {
    /// The parsed file.
    pub db: rkdb::Db,
    /// The Index intermediate that signed it (id text).
    pub signer: String,
}

/// A root-signed section that verified.
#[derive(Clone, Debug)]
pub struct HeldRoot {
    /// The section.
    pub section: rkdb::RootSection,
    /// The root that signed it (id text).
    pub signer: String,
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
                    } else if !is_test && crate::cert::is_legacy_serial(&c.body.serial) {
                        s.notes
                            .push(format!("root {i}: a 16-hex serial is the test authority's"));
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
                .filter(|p| verify::revocation(p, self, None).is_none());
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

    /// The latest `expires` of every list and database held.
    pub fn newest_expires(&self) -> Option<u64> {
        let lists = self.lists.iter().map(|l| l.list.expires);
        let dbs = self.dbs.iter().map(|h| u64::from(h.db.expires));
        lists.chain(dbs).max()
    }

    /// The highest database serial held.
    pub fn db_serial(&self) -> Option<u64> {
        self.dbs.iter().map(|h| h.db.serial).max()
    }

    /// The newest database held.
    pub fn best_db(&self) -> Option<&HeldDb> {
        self.dbs.iter().max_by_key(|h| h.db.serial)
    }

    /// True when the newest database is below the floor (or there is none while a floor is set).
    pub fn behind(&self) -> bool {
        self.floor > 0 && self.db_serial().is_none_or(|s| s < self.floor)
    }

    /// Raises the floor to `serial` (a signed release manifest or file certificate said the
    /// signer saw that database); never lowers it.
    pub fn raise_floor(&mut self, serial: u64) {
        self.floor = self.floor.max(serial);
    }

    /// Freshness of the newest list or database at `now`; a store behind its floor is stale at
    /// best.
    pub fn freshness(&self, now: u64) -> Freshness {
        let f = self
            .newest_expires()
            .map_or(Freshness::Missing, |e| revoked::freshness(e, now));
        if f == Freshness::Fresh && self.behind() {
            Freshness::Stale
        } else {
            f
        }
    }

    fn signer_ok(
        &self,
        id: &[u8; 64],
        issued: u64,
        root: bool,
        also: Option<&rkdb::RootSection>,
    ) -> Option<Cert> {
        let text = rkdb::id_text(id);
        let pool: Vec<&Cert> = if root {
            self.roots.iter().collect()
        } else {
            self.intermediates.iter().collect()
        };
        let c = pool.into_iter().find(|c| c.body.serial == text)?.clone();
        if !c.body.usage.contains(&Usage::SignRevocations) || !c.valid_at(issued) {
            return None;
        }
        if root {
            return (verify::revocation(&[&c], self, None).is_none()).then_some(c);
        }
        let ok = verify::path(&c, &[], self)
            .ok()
            .filter(|p| p.iter().all(|x| x.valid_at(issued)))
            .filter(|p| verify::revocation(p, self, None).is_none())
            // The root section that travels with the database may withdraw its own signer.
            .filter(|p| {
                also.is_none_or(|r| {
                    !p.iter().any(|x| {
                        rkdb::id_bytes(&x.body.serial)
                            .is_some_and(|i| r.entries.lookup(&i).is_some())
                    })
                })
            })
            .is_some();
        ok.then_some(c)
    }

    /// Verifies a root-signed section: a pinned root, valid when it signed, not withdrawn, that
    /// does not name itself.
    pub fn check_root_section(&self, s: &rkdb::RootSection) -> Result<HeldRoot, rkdb::Error> {
        let c = self
            .signer_ok(&s.signer, u64::from(s.issued), true, None)
            .ok_or(rkdb::Error::BadSignature)?;
        let msg = wolcrypt::sign::message(rkdb::LABEL, s.signed_part());
        crypto::verify_hybrid(&c.keys, &msg, &s.sig).map_err(|_| rkdb::Error::BadSignature)?;
        if s.entries.lookup(&s.signer).is_some() {
            return Err(rkdb::Error::NotAllowed);
        }
        Ok(HeldRoot {
            section: s.clone(),
            signer: c.body.serial.clone(),
        })
    }

    /// Verifies a whole database: the Index intermediate signed it (valid when it issued it, up to
    /// the pinned roots, not withdrawn), and its root section, when present, verifies too.
    pub fn check_rkdb(
        &self,
        bytes: &[u8],
        now: u64,
    ) -> Result<(HeldDb, Option<HeldRoot>), rkdb::Error> {
        let db = rkdb::Db::parse(bytes)?;
        if u64::from(db.issued) > now.saturating_add(verify::CLOCK_SKEW) {
            return Err(rkdb::Error::Malformed);
        }
        // The root section first: it may withdraw the Index itself.
        let root = match &db.root {
            Some(r) => Some(self.check_root_section(r)?),
            None => None,
        };
        let c = self
            .signer_ok(
                &db.signer,
                u64::from(db.issued),
                false,
                root.as_ref().map(|r| &r.section),
            )
            .ok_or(rkdb::Error::BadSignature)?;
        if c.body.kind != Kind::Intermediate {
            return Err(rkdb::Error::BadSignature);
        }
        crypto::verify_hybrid(&c.keys, &db.message(bytes), &db.sig)
            .map_err(|_| rkdb::Error::BadSignature)?;
        let signer = c.body.serial.clone();
        Ok((HeldDb { db, signer }, root))
    }

    /// Adds a database after [`Store::check_rkdb`]; on failure it is left out with a note.
    pub fn load_rkdb(&mut self, bytes: &[u8], now: u64) -> Result<(), rkdb::Error> {
        match self.check_rkdb(bytes, now) {
            Ok((db, root)) => {
                if let Some(r) = root {
                    self.root_sections.push(r);
                }
                self.dbs.push(db);
                Ok(())
            }
            Err(e) => {
                self.notes
                    .push(format!("revocation database: {}", e.as_str()));
                Err(e)
            }
        }
    }

    /// Adds a root-signed section on its own (`.rkdr`, `woltrust emergency`).
    pub fn load_root_section(&mut self, bytes: &[u8]) -> Result<(), rkdb::Error> {
        let (s, n) = rkdb::RootSection::parse(bytes)?;
        if n != bytes.len() {
            return Err(rkdb::Error::Malformed);
        }
        let r = self.check_root_section(&s)?;
        self.root_sections.push(r);
        Ok(())
    }
}
