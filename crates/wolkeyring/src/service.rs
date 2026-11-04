// SPDX-License-Identifier: BSD-2-Clause
//! One open keyring with its files and audit log: what the daemon and the command line share
//! (ADR 0054). Every change is appended to the store and audited in the same call; the audit
//! gets a signed checkpoint at every unlock and lock and every [`audit::CHECKPOINT_EVERY`] lines.
use std::collections::BTreeMap;
use std::path::PathBuf;
use zeroize::Zeroizing;

use crate::audit::{self, Event, Log};
use crate::factor::{Factor, Kind, Presented};
use crate::item::{AppId, Class, Meta};
use crate::kdf::Kdf;
use crate::rand::{OsRandom, Random};
use crate::store::{Keyring, MAX_FILE};
use crate::{Error, Result, io, wire};

/// The collection every item lives in (the Secret Service's `login`).
pub const COLLECTION: &str = "login";

/// An open keyring.
pub struct Session {
    pub name: String,
    pub keyring: Keyring,
    log: Log,
    store_path: PathBuf,
    audit_path: PathBuf,
    rng: OsRandom,
    /// Unix seconds of the last read or write (the idle lock).
    pub last_use: u64,
    /// (inode, length) of the store and the audit log as this session last read them.
    store_seen: Option<(u64, u64)>,
    audit_seen: Option<(u64, u64)>,
    /// What the rollback record says is wrong (see [`Session::findings`]); empty when nothing is.
    /// Cleared only by [`Session::accept_rollback`].
    pub warnings: Vec<String>,
}

/// The rollback record: where the last signed checkpoint stood, kept in a file outside the data
/// folder (ADR 0054 section 6). Restoring the store and the audit log to an earlier consistent
/// pair leaves each file valid on its own; this record is what notices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Baseline {
    /// Sequence number of the audit checkpoint.
    seq: u64,
    epoch: u64,
    records: u64,
}

/// The credential name a TPM-sealed factor secret is bound to.
pub fn tpm_name(store: &str, factor: u8) -> String {
    format!("wolos-keyring-{store}-f{factor}")
}

impl Session {
    /// Opens keyring `name` (sealed: nothing decrypted until [`Session::unlock`]).
    pub fn open(name: &str) -> Result<Session> {
        let store_path = io::store_path(name)?;
        let audit_path = io::audit_path(name)?;
        let bytes = io::read(&store_path, MAX_FILE)?.ok_or(Error::NotFound)?;
        let keyring = Keyring::parse(&bytes)?;
        let text = io::read(&audit_path, audit::MAX_LOG)?.unwrap_or_default();
        let log = Log::resume(&String::from_utf8_lossy(&text))?;
        let store_seen = io::stat(&store_path);
        let audit_seen = io::stat(&audit_path);
        let mut s = Session {
            name: name.into(),
            keyring,
            log,
            store_seen,
            audit_seen,
            store_path,
            audit_path,
            rng: OsRandom,
            last_use: io::now(),
            warnings: Vec::new(),
        };
        s.check_baseline();
        Ok(s)
    }

    fn baseline(&self) -> Option<Baseline> {
        let b = io::read(&io::state_path(&self.name).ok()?, 4096).ok()??;
        serde_json::from_slice(&b).ok()
    }

    /// Compares the files with the rollback record and notes what is behind it. A finding stays
    /// until the owner accepts it (`wol keyring audit accept`).
    fn check_baseline(&mut self) {
        let Some(b) = self.baseline() else { return };
        let (epoch, records, _) = self.keyring.position();
        let mut found = Vec::new();
        if self.log.last_checkpoint().is_none_or(|c| c < b.seq) {
            found.push("the audit log is shorter than its last signed checkpoint recorded outside it: the log was truncated or rolled back".to_string());
        }
        // A sealed keyring has not read its records yet: its position counts from the unlock.
        if self.keyring.unlocked() && (epoch, records) < (b.epoch, b.records) {
            found.push("the store is older than its last signed checkpoint recorded outside it: the keyring file was rolled back".to_string());
        }
        for f in found {
            if !self.warnings.contains(&f) {
                self.warnings.push(f);
            }
        }
    }

    /// Records the last checkpoint as the new baseline: never lower than before, and not at all
    /// while a finding is outstanding (a rolled-back file must not teach the record its own age),
    /// unless `force` (the owner accepted).
    fn set_baseline(&self, force: bool) -> Result<()> {
        if !force && !self.warnings.is_empty() {
            return Ok(());
        }
        let Some(seq) = self.log.last_checkpoint() else {
            return Ok(());
        };
        let (epoch, records, _) = self.keyring.position();
        let mut new = Baseline {
            seq,
            epoch,
            records,
        };
        if !force && let Some(old) = self.baseline() {
            new = Baseline {
                seq: new.seq.max(old.seq),
                epoch: new.epoch.max(old.epoch),
                records: if new.epoch > old.epoch {
                    new.records
                } else if new.epoch < old.epoch {
                    old.records
                } else {
                    new.records.max(old.records)
                },
            };
        }
        let bytes = serde_json::to_vec(&new).map_err(|e| Error::Io(e.to_string()))?;
        io::replace(&io::state_path(&self.name)?, &bytes)
    }

    /// The owner accepts the files as they are (after a restore from a backup, say): the rollback
    /// record is set to them. Needs the full unlock; audited.
    pub fn accept_rollback(&mut self) -> Result<()> {
        let _l = io::lock(&self.name)?;
        self.sync_audit()?;
        let line = self.log.append(
            io::now(),
            Event::Settings,
            Some("wol-keyring"),
            None,
            Some("rollback record accepted"),
        )?;
        io::append(&self.audit_path, line.as_bytes())?;
        self.audit_seen = io::stat(&self.audit_path);
        self.checkpoint_locked()?;
        self.warnings.clear();
        self.set_baseline(true)
    }

    /// Whether keyring `name` exists.
    pub fn exists(name: &str) -> bool {
        io::store_path(name).is_ok_and(|p| p.exists())
    }

    /// Makes keyring `name` with the login password, the TPM when the machine has one, and a
    /// recovery code; returns the session (unlocked) and the recovery code to show once.
    pub fn create(name: &str, password: &str) -> Result<(Session, Zeroizing<String>)> {
        if password.chars().count() < crate::factor::MIN_PASSWORD {
            return Err(Error::Policy("the password has at least 8 characters"));
        }
        let store_path = io::store_path(name)?;
        if store_path.exists() {
            return Err(Error::Io("this keyring exists already".into()));
        }
        let mut rng = OsRandom;
        let mut factors = Vec::new();
        let mut rules = Vec::new();
        let pw = typed_factor(&mut rng, 1, Kind::Password, "Login password")?;
        let s = pw.secret(&Presented::Typed(Zeroizing::new(password.into())))?;
        factors.push((pw, s));
        rules.push(vec![1]);
        let raw: [u8; 16] = rng.bytes()?;
        let code = crate::factor::recovery_code(&raw);
        let rc = typed_factor(&mut rng, 2, Kind::Recovery, "Recovery code")?;
        let s = rc.secret(&Presented::Typed(code.clone()))?;
        factors.push((rc, s));
        rules.push(vec![2]);
        if io::has_tpm2() {
            // The store id is not known before create; the credential is bound to the keyring
            // name and factor id instead.
            let secret = Zeroizing::new(rng.bytes::<32>()?);
            if let Ok(blob) = io::tpm_seal(&tpm_name(name, 3), &secret) {
                let f = Factor {
                    id: 3,
                    kind: Kind::Tpm,
                    label: "This computer's security chip".into(),
                    kdf: None,
                    salt: None,
                    sealed: Some(wire::b64(&blob)),
                    credential: None,
                    hmac_salt: None,
                };
                let s = f.secret(&Presented::Bytes(secret))?;
                factors.push((f, s));
                rules.push(vec![3]);
            }
        }
        let (keyring, bytes) = Keyring::create(&mut rng, name, factors, &rules)?;
        io::replace(&store_path, &bytes)?;
        let mut s = Session {
            name: name.into(),
            keyring,
            log: Log::default(),
            store_path,
            audit_path: io::audit_path(name)?,
            rng: OsRandom,
            last_use: io::now(),
            store_seen: None,
            audit_seen: None,
            warnings: Vec::new(),
        };
        // A new keyring starts a new record (an old one of the same name belongs to a keyring
        // that no longer exists).
        let _ = std::fs::remove_file(io::state_path(name)?);
        s.store_seen = io::stat(&s.store_path);
        s.audit(Event::Policy, Some("wol-keyring"), None, Some("created"))?;
        s.checkpoint()?;
        Ok((s, code))
    }

    /// Appends one audit line (and a checkpoint when due and possible).
    pub fn audit(
        &mut self,
        e: Event,
        app: Option<&str>,
        item: Option<&str>,
        detail: Option<&str>,
    ) -> Result<()> {
        let _l = io::lock(&self.name)?;
        self.sync_audit()?;
        let line = self.log.append(io::now(), e, app, item, detail)?;
        // A read is logged at once but synced with the next durable line or checkpoint (at most
        // 64 lines later); everything that changes state is synced here.
        if e == Event::Read {
            io::append_lazy(&self.audit_path, line.as_bytes())?;
        } else {
            io::append(&self.audit_path, line.as_bytes())?;
        }
        self.audit_seen = io::stat(&self.audit_path);
        // A read never waits for a signature: the daemon's timer signs the checkpoint that falls
        // due (`checkpoint_if_due`), within seconds.
        if e != Event::Read && self.log.due() && self.keyring.unlocked() {
            self.checkpoint_locked()?;
        }
        Ok(())
    }

    /// A signed checkpoint when 64 lines have gathered since the last (the daemon's timer).
    pub fn checkpoint_if_due(&mut self) -> Result<()> {
        if self.keyring.unlocked() && self.log.due() {
            self.checkpoint()?;
        }
        Ok(())
    }

    fn sync_audit(&mut self) -> Result<()> {
        let now = io::stat(&self.audit_path);
        if now != self.audit_seen {
            let text = io::read(&self.audit_path, audit::MAX_LOG)?.unwrap_or_default();
            self.log = Log::resume(&String::from_utf8_lossy(&text))?;
            self.audit_seen = now;
        }
        Ok(())
    }

    /// Reads what another process wrote to the store since this session last looked: the
    /// appended records only when the file grew in place, the whole file after a rewrite.
    pub fn sync(&mut self) -> Result<()> {
        let now = io::stat(&self.store_path);
        match (self.store_seen, now) {
            (a, b) if a == b => {}
            (Some((ino, len)), Some((ino2, len2))) if ino == ino2 && len2 > len => {
                let tail = io::read_from(&self.store_path, len, MAX_FILE)?;
                self.keyring.take_tail(&tail)?;
            }
            (_, Some(_)) => {
                let bytes = io::read(&self.store_path, MAX_FILE)?.ok_or(Error::NotFound)?;
                self.keyring.refresh(&bytes)?;
            }
            (_, None) => return Err(Error::NotFound),
        }
        self.store_seen = now;
        Ok(())
    }

    /// Appends records to the store under the lock, after taking in what others appended.
    fn write(
        &mut self,
        f: impl FnOnce(&mut Keyring, &mut OsRandom) -> Result<Vec<u8>>,
    ) -> Result<()> {
        let _l = io::lock(&self.name)?;
        self.sync()?;
        let rec = f(&mut self.keyring, &mut self.rng)?;
        io::append(&self.store_path, &rec)?;
        self.store_seen = io::stat(&self.store_path);
        Ok(())
    }

    /// Rewrites the store under the lock (policy and settings changes, compaction).
    fn rewrite(
        &mut self,
        f: impl FnOnce(&mut Keyring, &mut OsRandom) -> Result<Vec<u8>>,
    ) -> Result<()> {
        let _l = io::lock(&self.name)?;
        self.sync()?;
        let bytes = f(&mut self.keyring, &mut self.rng)?;
        io::replace(&self.store_path, &bytes)?;
        self.store_seen = io::stat(&self.store_path);
        Ok(())
    }

    /// A signed checkpoint now (needs the full unlock).
    pub fn checkpoint(&mut self) -> Result<()> {
        let _l = io::lock(&self.name)?;
        self.sync_audit()?;
        self.checkpoint_locked()
    }

    fn checkpoint_locked(&mut self) -> Result<()> {
        let id = self.keyring.identity()?;
        let line = self
            .log
            .checkpoint(io::now(), &id, self.keyring.position())?;
        io::append(&self.audit_path, line.as_bytes())?;
        self.audit_seen = io::stat(&self.audit_path);
        self.set_baseline(false)
    }

    /// Presents the machine factors (TPM) the policy names, plus `typed` for typed factors of
    /// the kinds given, and unlocks.
    pub fn unlock(&mut self, typed: &[(Kind, &str)], use_machine: bool, app: &str) -> Result<u8> {
        let mut presented: Vec<(u8, Presented)> = Vec::new();
        for f in &self.keyring.header().factors {
            if let Some((_, t)) = typed.iter().find(|(k, _)| *k == f.kind) {
                presented.push((f.id, Presented::Typed(Zeroizing::new((*t).to_string()))));
            } else if use_machine && f.kind == Kind::Tpm {
                let blob = wire::unb64(f.sealed.as_deref().unwrap_or(""), 64 * 1024)?;
                if let Ok(s) = io::tpm_open(&tpm_name(&self.name, f.id), &blob) {
                    presented.push((f.id, Presented::Bytes(s)));
                }
            }
        }
        match self.keyring.unlock(&presented) {
            Ok(rule) => {
                self.last_use = io::now();
                self.audit(
                    Event::Unlock,
                    Some(app),
                    None,
                    Some(&format!("rule {rule}")),
                )?;
                self.checkpoint()?;
                Ok(rule)
            }
            Err(e) => {
                self.audit(Event::UnlockRefused, Some(app), None, None)?;
                Err(e)
            }
        }
    }

    /// Locks (screen lock, sleep, idle): checkpoint first, while the key is still there.
    pub fn lock(&mut self, why: &str) -> Result<()> {
        if self.keyring.unlocked() {
            let _ = self.checkpoint();
            self.keyring.lock();
            self.audit(Event::Lock, None, None, Some(why))?;
        }
        Ok(())
    }

    /// Stores a secret for `app`: replaces the item with the same attributes, else makes one;
    /// the caller goes on the access list.
    pub fn set(
        &mut self,
        app: Option<&AppId>,
        label: &str,
        attrs: BTreeMap<String, String>,
        secret: &[u8],
        class: Class,
    ) -> Result<String> {
        let now = io::now();
        let existing = self
            .keyring
            .search(&attrs)
            .into_iter()
            .find(|m| m.attrs == attrs && m.collection == COLLECTION)
            .cloned();
        let meta = match existing {
            Some(mut m) => {
                if let Some(a) = app
                    && !m.allows(a)
                {
                    self.audit(
                        Event::ReadDenied,
                        Some(a.display()),
                        Some(&m.id),
                        Some("write"),
                    )?;
                    return Err(Error::Denied);
                }
                m.label = label.into();
                m.modified = now;
                m.class = class;
                m
            }
            None => Meta {
                id: String::new(),
                collection: COLLECTION.into(),
                label: label.into(),
                attrs,
                content_type: "text/plain".into(),
                created: now,
                modified: now,
                class,
                device_only: false,
                acl: app.cloned().into_iter().collect(),
            },
        };
        let mut id = String::new();
        self.write(|k, r| {
            let (i, rec) = k.put(r, meta, secret)?;
            id = i;
            Ok(rec)
        })?;
        self.last_use = now;
        self.audit(
            Event::Write,
            app.map(AppId::display).or(Some("wol-keyring")),
            Some(&id),
            None,
        )?;
        Ok(id)
    }

    /// Reads a secret for `app` (None = the owner at the command line). An item no app is on
    /// the list of (imported, migrated or made at the terminal) is the owner's alone: an app asks
    /// for nothing it can get until the owner puts it on the list (`wol keyring acl allow`); the
    /// shell's "Always allow" sheet replaces this once it exists (ADR 0054 section 4).
    pub fn get(&mut self, app: Option<&AppId>, id: &str) -> Result<Zeroizing<Vec<u8>>> {
        let who = app.map(AppId::display).unwrap_or("wol-keyring").to_string();
        match self.keyring.secret(id, app) {
            Ok(s) => {
                self.last_use = io::now();
                self.audit(Event::Read, Some(&who), Some(id), None)?;
                Ok(s)
            }
            Err(e) => {
                if e == Error::Denied {
                    self.audit(Event::ReadDenied, Some(&who), Some(id), None)?;
                }
                Err(e)
            }
        }
    }

    /// Deletes an item (the caller must be on its access list).
    pub fn delete(&mut self, app: Option<&AppId>, id: &str) -> Result<()> {
        if let Some(a) = app
            && self.keyring.meta(id).is_some_and(|m| !m.allows(a))
        {
            return Err(Error::Denied);
        }
        self.write(|k, r| k.delete(r, id))?;
        self.audit(
            Event::Delete,
            app.map(AppId::display).or(Some("wol-keyring")),
            Some(id),
            None,
        )
    }

    /// Replaces an item's access list (the owner at the command line or Settings).
    pub fn set_acl(&mut self, id: &str, acl: Vec<AppId>) -> Result<()> {
        let n = acl.len();
        self.write(|k, r| k.set_acl(r, id, acl))?;
        self.audit(
            Event::Acl,
            Some("wol-keyring"),
            Some(id),
            Some(&format!("{n} apps on the list")),
        )
    }

    /// Imports a payload; returns (added, replaced, kept).
    pub fn import(
        &mut self,
        payload: &crate::bundle::Payload,
        signer: &str,
        how: Event,
    ) -> Result<(usize, usize, usize)> {
        let mut counts = (0, 0, 0);
        self.write(|k, r| {
            let (c, recs) = k.import(r, payload)?;
            counts = c;
            Ok(recs)
        })?;
        self.audit(
            how,
            Some("wol-keyring"),
            None,
            Some(&format!(
                "signer {signer}, {} added {} replaced {} kept",
                counts.0, counts.1, counts.2
            )),
        )?;
        Ok(counts)
    }

    /// The randomness source.
    pub fn rng(&mut self) -> &mut OsRandom {
        &mut self.rng
    }

    /// Rewrites the whole store under the next epoch (drops replaced records).
    pub fn compact(&mut self) -> Result<()> {
        self.rewrite(|k, r| k.compact(r))
    }

    /// The audit log's text.
    pub fn audit_text(&self) -> Result<String> {
        Ok(String::from_utf8_lossy(
            &io::read(&self.audit_path, audit::MAX_LOG)?.unwrap_or_default(),
        )
        .into_owned())
    }

    /// Adds a recovery code or PIN (with the chip) factor; returns the new file written. Needs a
    /// strong unlock; `typed` is the PIN, or None to make a new recovery code (returned).
    pub fn enrol(
        &mut self,
        kind: Kind,
        typed: Option<&str>,
        current: &[(Kind, &str)],
    ) -> Result<Option<Zeroizing<String>>> {
        let mut factors = Vec::new();
        let header = self.keyring.header().clone();
        // Existing factors with their secrets: typed ones from `current`, the chip from the TPM.
        for f in &header.factors {
            if f.kind == kind && kind == Kind::Recovery {
                continue; // a new recovery code replaces the old
            }
            let p = if let Some((_, t)) = current.iter().find(|(k, _)| *k == f.kind) {
                Presented::Typed(Zeroizing::new((*t).to_string()))
            } else if f.kind == Kind::Tpm {
                let blob = wire::unb64(f.sealed.as_deref().unwrap_or(""), 64 * 1024)?;
                Presented::Bytes(io::tpm_open(&tpm_name(&self.name, f.id), &blob)?)
            } else {
                return Err(Error::Policy(
                    "every current factor must be presented to change the policy",
                ));
            };
            factors.push((f.clone(), f.secret(&p)?));
        }
        let id = (1..=255u8)
            .find(|i| !factors.iter().any(|(f, _)| f.id == *i))
            .ok_or(Error::Limit("factors"))?;
        let mut shown = None;
        let new = match kind {
            Kind::Recovery => {
                let raw: [u8; 16] = self.rng.bytes()?;
                let code = crate::factor::recovery_code(&raw);
                let f = typed_factor(&mut self.rng, id, Kind::Recovery, "Recovery code")?;
                let s = f.secret(&Presented::Typed(code.clone()))?;
                shown = Some(code);
                (f, s)
            }
            Kind::Pin => {
                let pin = typed.ok_or(Error::Policy("a PIN is needed"))?;
                if pin.len() < crate::factor::MIN_PIN || !pin.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(Error::Policy("a PIN is at least 6 digits"));
                }
                let f = typed_factor(&mut self.rng, id, Kind::Pin, "PIN")?;
                let s = f.secret(&Presented::Typed(Zeroizing::new(pin.into())))?;
                (f, s)
            }
            _ => return Err(Error::Policy("enrol this factor in Settings")),
        };
        factors.push(new);
        let tpm = factors
            .iter()
            .find(|(f, _)| f.kind == Kind::Tpm)
            .map(|(f, _)| f.id);
        let mut rules = Vec::new();
        for (f, _) in &factors {
            match f.kind {
                Kind::Pin => {
                    if let Some(t) = tpm {
                        let mut r = vec![t, f.id];
                        r.sort_unstable();
                        rules.push(r);
                    }
                }
                _ => rules.push(vec![f.id]),
            }
        }
        if kind == Kind::Pin && tpm.is_none() {
            return Err(Error::Policy(
                "a PIN unlocks only together with the security chip",
            ));
        }
        self.rewrite(|k, r| k.set_policy(r, factors, &rules))?;
        self.audit(
            Event::Policy,
            Some("wol-keyring"),
            None,
            Some(&format!("enrolled {kind:?}")),
        )?;
        // set_policy dropped the unlock; the caller unlocks again.
        Ok(shown)
    }
}

/// A typed factor's descriptor with a fresh salt and the default KDF.
pub fn typed_factor(rng: &mut impl Random, id: u8, kind: Kind, label: &str) -> Result<Factor> {
    let salt: [u8; 16] = rng.bytes()?;
    Ok(Factor {
        id,
        kind,
        label: label.into(),
        kdf: Some(Kdf::default_new()),
        salt: Some(wire::b64(&salt)),
        sealed: None,
        credential: None,
        hmac_salt: None,
    })
}
