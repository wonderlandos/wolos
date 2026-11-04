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
    /// When the update service last reached the list's server (its `last-attempt` stamp), Unix
    /// seconds: the "connected" signal of the install gate.
    pub last_attempt: Option<u64>,
    /// When it last installed a list or database (`last-ok`).
    pub last_ok: Option<u64>,
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
        let none: [&[u8]; 0] = [];
        Store::load_with_sections(roots, intermediates, &none, lists, now, allow_test)
    }

    /// [`Store::load`] with root-signed sections (`.rkdr` files, the sections kept from earlier
    /// databases) loaded before any list (audit 2 M5): a section that withdraws the Index (or a
    /// root on its path) is in place before the Index's lists are judged, so they never load.
    pub fn load_with_sections(
        roots: &[impl AsRef<[u8]>],
        intermediates: &[impl AsRef<[u8]>],
        sections: &[impl AsRef<[u8]>],
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
        for (i, raw) in sections.iter().enumerate() {
            if let Err(e) = s.load_root_section(raw.as_ref(), now) {
                s.notes.push(format!("root section {i}: {}", e.as_str()));
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
        if !is_root && !self.list_signer_ok(cert, at) {
            return fail(self, "signer is not a valid, unrevoked intermediate");
        } else if is_root && !cert.valid_at(at) {
            return fail(self, "signing root was not valid when it issued the list");
        }
        Some(RevList {
            list: parsed,
            signer: cert.body.serial.clone(),
            signer_is_root: is_root,
        })
    }

    /// True when the intermediate `cert` may sign a list issued at `at`: its path reaches the
    /// pinned roots, every certificate on it was valid then, and nothing the store holds
    /// withdraws one of them.
    fn list_signer_ok(&self, cert: &Cert, at: u64) -> bool {
        verify::path(cert, &[], self)
            .ok()
            .filter(|p| p.iter().all(|c| c.valid_at(at)))
            .filter(|p| verify::revocation(p, self, None).is_none())
            .is_some()
    }

    /// Drops the lists and databases whose signer a root section kept since withdraws (audit 2
    /// M5: whatever order the files came in, a withdrawn Index's lists stop counting).
    fn prune(&mut self) {
        let lists = std::mem::take(&mut self.lists);
        for l in lists {
            let signer = self
                .roots
                .iter()
                .chain(self.intermediates.iter())
                .find(|c| c.body.serial == l.signer)
                .cloned();
            let ok = signer.is_some_and(|c| {
                if l.signer_is_root {
                    verify::revocation(&[&c], self, None).is_none()
                } else {
                    self.list_signer_ok(&c, l.list.issued)
                }
            });
            if ok {
                self.lists.push(l);
            } else {
                self.notes.push(format!(
                    "list by {}: its signer is withdrawn by a root section",
                    l.signer
                ));
            }
        }
        let dbs = std::mem::take(&mut self.dbs);
        for h in dbs {
            if self
                .signer_ok(&h.db.signer, u64::from(h.db.issued), false, None)
                .is_some()
            {
                self.dbs.push(h);
            } else {
                self.notes.push(format!(
                    "revocation database {}: its signer is withdrawn by a root section",
                    h.db.serial
                ));
            }
        }
    }

    /// Keeps a verified root section (once), then drops what it withdraws.
    fn keep_section(&mut self, r: HeldRoot) {
        if !self
            .root_sections
            .iter()
            .any(|h| h.section.bytes == r.section.bytes)
        {
            self.root_sections.push(r);
            self.prune();
        }
    }

    /// Checks an intermediate certificate before it is kept (`/var/lib/wolos/trust/
    /// intermediates/`, a rotated Index fetched from the CDN): its path reaches the pinned roots
    /// with both roots' signatures, every certificate on it is valid at `now` and none is
    /// withdrawn.
    pub fn check_intermediate(&self, c: &Cert, now: u64) -> Result<(), verify::Code> {
        if c.body.kind != Kind::Intermediate {
            return Err(verify::Code::BadIssuer);
        }
        let p = verify::path(c, &[], self)?;
        if let Some(code) = verify::time_problem(&p, now, None) {
            return Err(code);
        }
        match verify::revocation(&p, self, None) {
            Some(code) => Err(code),
            None => Ok(()),
        }
    }

    /// The hard entries (`key-compromise`, `abuse`) the database `new` adds over `held`, as one
    /// list by its Index: what the memory of seen revocations must keep (audit 2 M4), so a
    /// rolled-back database cannot drop them. Entries `held` has with the same or a stronger
    /// reason are left out.
    pub fn hard_added(new: &HeldDb, held: Option<&HeldDb>) -> RevList {
        let entries = new
            .db
            .entries
            .iter()
            .filter(|e| e.reason.is_hard())
            .filter(|e| {
                held.and_then(|h| h.db.entries.lookup(&e.id))
                    .is_none_or(|o| rkdb::strength(o.reason) < rkdb::strength(e.reason))
            })
            .map(|e| e.to_json())
            .collect();
        RevList {
            list: revoked::ListText {
                format: crate::FORMAT,
                serial: new.db.serial,
                issued: u64::from(new.db.issued),
                expires: u64::from(new.db.expires),
                entries,
            },
            signer: new.signer.clone(),
            signer_is_root: false,
        }
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
    /// Entries the memory has no room for are counted in a note (audit 2 M4: never silently).
    pub fn seen(&mut self, old: &revoked::Seen) -> revoked::Seen {
        let mut s = old.clone();
        let mut refused = 0;
        for l in &self.lists {
            refused += s.add(l).refused;
        }
        if refused > 0 {
            self.notes.push(format!(
                "revocation memory full: {refused} entries not remembered"
            ));
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
    /// does not name itself, not issued after `now` (plus [`verify::CLOCK_SKEW`]; audit 2 L2).
    pub fn check_root_section(
        &self,
        s: &rkdb::RootSection,
        now: u64,
    ) -> Result<HeldRoot, rkdb::Error> {
        if u64::from(s.issued) > now.saturating_add(verify::CLOCK_SKEW) {
            return Err(rkdb::Error::Malformed);
        }
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
            Some(r) => Some(self.check_root_section(r, now)?),
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

    /// The root section a database carries, verified on its own: kept even when the rest of the
    /// file is refused (audit 2 H1: a section that withdraws the Index, or a root on its path,
    /// makes the Index block fail, and it is exactly the withdrawal that must arrive). The
    /// section is found by the layout alone, so a database the Index got wrong in any other way
    /// still delivers it.
    pub fn salvage_root_section(&self, bytes: &[u8], now: u64) -> Option<HeldRoot> {
        let s = rkdb::root_section_in(bytes)?;
        self.check_root_section(&s, now).ok()
    }

    /// Adds a database after [`Store::check_rkdb`]; on failure it is left out with a note, but a
    /// root section it carries that verifies on its own is kept ([`Store::salvage_root_section`]).
    pub fn load_rkdb(&mut self, bytes: &[u8], now: u64) -> Result<(), rkdb::Error> {
        match self.check_rkdb(bytes, now) {
            Ok((db, root)) => {
                self.dbs.push(db);
                if let Some(r) = root {
                    self.keep_section(r);
                }
                Ok(())
            }
            Err(e) => {
                self.refuse_db(bytes, now, e);
                Err(e)
            }
        }
    }

    fn refuse_db(&mut self, bytes: &[u8], now: u64, e: rkdb::Error) {
        self.notes
            .push(format!("revocation database: {}", e.as_str()));
        if let Some(r) = self.salvage_root_section(bytes, now) {
            self.notes.push(format!(
                "revocation database: its root section by {} is kept",
                r.signer
            ));
            self.keep_section(r);
        }
    }

    /// [`Store::load_rkdb`] for a database that must also pass the acceptance rules against what
    /// the store already holds ([`rkdb::accept`]: the floor, no older serial, no dropped or
    /// weakened entry, the chain): the state file `/var/lib/wolos/trust/revoked.rkdb` after the
    /// image's database (audit 2 M4), so a restored or rolled-back file is left out with a note.
    /// Its root section, when it verifies, is kept either way.
    pub fn load_rkdb_accepted(&mut self, bytes: &[u8], now: u64) -> Result<(), rkdb::Error> {
        let (h, root) = match self.check_rkdb(bytes, now) {
            Ok(x) => x,
            Err(e) => {
                self.refuse_db(bytes, now, e);
                return Err(e);
            }
        };
        let held = self.best_db().map(|d| d.db.clone());
        if let Some(r) = root {
            self.keep_section(r);
        }
        if let Err(e) = rkdb::accept(&h.db, held.as_ref(), self.floor) {
            self.notes.push(format!(
                "revocation database {}: {}, left out",
                h.db.serial,
                e.as_str()
            ));
            return Err(e);
        }
        self.dbs.push(h);
        self.prune();
        Ok(())
    }

    /// Adds a root-signed section on its own (`.rkdr`, `woltrust emergency`), checked at `now`.
    pub fn load_root_section(&mut self, bytes: &[u8], now: u64) -> Result<(), rkdb::Error> {
        let (s, n) = rkdb::RootSection::parse(bytes)?;
        if n != bytes.len() {
            return Err(rkdb::Error::Malformed);
        }
        let r = self.check_root_section(&s, now)?;
        self.keep_section(r);
        Ok(())
    }
}
