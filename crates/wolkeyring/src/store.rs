// SPDX-License-Identifier: BSD-2-Clause
//! The keyring store `wolos/keyring/1` (ADR 0054 section 2): one file, a header and an
//! append-only list of encrypted records.
//!
//! File: `u32 header length || header JSON || records`, each record `u32 length || seq (u64) ||
//! salt (32) || ChaCha20-Poly1305 ciphertext and tag`. Keys: the master key MK (wrapped by the
//! rules, [`crate::factor`]) seals the key block `dek_wu || dek_afu || id_seed`. A record is
//! sealed under `derive(FORMAT "/rec", store, epoch, seq, salt, dek_afu)` with associated data
//! `FORMAT, store, epoch, seq, salt, chain value of the record before`, so a record moved,
//! replayed, reordered or removed from the middle does not open. An item's secret is sealed again
//! inside its record under its class key (`dek_wu` or `dek_afu`) and a fresh salt, so locking the
//! screen (dropping `dek_wu`) keeps the index and `after-first-unlock` secrets and nothing else.
//! The header carries a tag made with MK over its own bytes: a changed label, setting or rule list
//! is found at unlock. Every key is used for one message.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use zeroize::Zeroizing;

use crate::factor::{self, Factor, Presented, Rule};
use crate::item::{AppId, Class, MAX_SECRET, Meta};
use crate::rand::Random;
use crate::wire::{self, Reader};
use crate::{Error, FORMAT, Result};

/// Most items.
pub const MAX_ITEMS: usize = 65_536;
/// Largest file.
pub const MAX_FILE: usize = 64 << 20;
/// Largest header.
pub const MAX_HEADER: usize = 256 * 1024;
/// Largest record.
pub const MAX_RECORD: usize = MAX_SECRET + 16 * 1024;

/// Settings that travel with the keyring (Settings > Privacy & Security > Keyring).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Lock after this many seconds without a read; 0 = never.
    pub lock_after_secs: u64,
    /// Lock when the machine goes to sleep or the lid closes.
    pub lock_on_sleep: bool,
    /// Lock with the screen.
    pub lock_with_screen: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            lock_after_secs: 900,
            lock_on_sleep: true,
            lock_with_screen: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sealed {
    salt: String,
    sealed: String,
}

/// The header, as stored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    /// Always [`FORMAT`]; anything else is refused (no downgrade, no guess).
    pub format: String,
    /// 16 random bytes, hex: binds every key and record to this keyring.
    pub store: String,
    /// The keyring's name ("login").
    pub name: String,
    /// Raised by every compaction: records of one epoch never open in another.
    pub epoch: u64,
    pub factors: Vec<Factor>,
    pub rules: Vec<Rule>,
    keys: Sealed,
    pub settings: Settings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mac: Option<Sealed>,
}

impl Header {
    fn store_id(&self) -> Result<[u8; 16]> {
        let v = wolcrypt::hash::unhex(&self.store).ok_or(Error::Malformed("store id"))?;
        v.try_into().map_err(|_| Error::Malformed("store id"))
    }
    fn unsigned_bytes(&self) -> Vec<u8> {
        let mut h = self.clone();
        h.mac = None;
        serde_json::to_vec(&h).unwrap_or_default()
    }
    /// The check every reader runs before anything is decrypted.
    fn check(&self) -> Result<()> {
        if self.format != FORMAT {
            return Err(Error::Unsupported(self.format.chars().take(40).collect()));
        }
        self.store_id()?;
        if self.name.is_empty() || self.name.len() > 64 {
            return Err(Error::Malformed("keyring name"));
        }
        factor::check_policy(&self.factors, &self.rules)
    }
}

struct Keys {
    mk: Zeroizing<[u8; 32]>,
    wu: Zeroizing<[u8; 32]>,
    id_seed: Zeroizing<[u8; 32]>,
}

struct Entry {
    meta: Meta,
    /// salt (32) || sealed secret.
    secret: Vec<u8>,
}

/// What a record does.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Body {
    Put { meta: Meta, secret: String },
    Del { id: String },
}

/// A keyring in memory: the header always, the index once unlocked, the keys while unlocked.
pub struct Keyring {
    header: Header,
    store: [u8; 16],
    /// Records not decrypted yet (the file was read while sealed).
    pending: Vec<u8>,
    items: BTreeMap<String, Entry>,
    by_attr: HashMap<(String, String), BTreeSet<String>>,
    chain: [u8; 64],
    seq: u64,
    afu: Option<Zeroizing<[u8; 32]>>,
    keys: Option<Keys>,
    /// The rule of the last unlock.
    rule: Option<u8>,
}

fn chain_start(store: &[u8; 16], epoch: u64) -> [u8; 64] {
    *wire::derive(&[
        format!("{FORMAT}/chain").as_bytes(),
        store,
        &epoch.to_be_bytes(),
    ])
}

fn chain_next(prev: &[u8; 64], record: &[u8]) -> [u8; 64] {
    *wire::derive(&[format!("{FORMAT}/chain").as_bytes(), prev, record])
}

fn class_byte(c: Class) -> u8 {
    match c {
        Class::WhenUnlocked => 1,
        Class::AfterFirstUnlock => 2,
    }
}

fn new_id(rng: &mut impl Random) -> Result<String> {
    Ok(wire::hex(&rng.bytes::<16>()?))
}

/// Factor secrets in rule order, from what was presented; None when a factor of the rule was
/// not presented.
fn secrets_for(
    factors: &[Factor],
    rule: &Rule,
    presented: &[(u8, Presented)],
) -> Option<Result<Vec<Zeroizing<[u8; 32]>>>> {
    let mut out = Vec::new();
    for id in &rule.factors {
        let (_, p) = presented.iter().find(|(i, _)| i == id)?;
        let f = factors.iter().find(|f| f.id == *id)?;
        out.push(f.secret(p));
    }
    Some(out.into_iter().collect())
}

impl Keyring {
    /// A new keyring named `name` with `factors` (each with its secret) and `rules` (lists of
    /// factor ids). Returns the keyring, unlocked, and the file's bytes.
    pub fn create(
        rng: &mut impl Random,
        name: &str,
        factors: Vec<(Factor, Zeroizing<[u8; 32]>)>,
        rules: &[Vec<u8>],
    ) -> Result<(Keyring, Vec<u8>)> {
        let store: [u8; 16] = rng.bytes()?;
        let mk = Zeroizing::new(rng.bytes::<32>()?);
        let block = Zeroizing::new(rng.bytes::<96>()?);
        let descs: Vec<Factor> = factors.iter().map(|(f, _)| f.clone()).collect();
        let mut rs = Vec::new();
        for (i, ids) in rules.iter().enumerate() {
            let rid = u8::try_from(i + 1).map_err(|_| Error::Policy("between 1 and 80 rules"))?;
            let secrets: Vec<&[u8; 32]> = ids
                .iter()
                .map(|id| factors.iter().find(|(f, _)| f.id == *id).map(|(_, s)| &**s))
                .collect::<Option<_>>()
                .ok_or(Error::Malformed("rule names a factor that is not enrolled"))?;
            rs.push(factor::wrap(rng, &store, rid, &descs, ids, &secrets, &mk)?);
        }
        let keys = seal_block(rng, &store, &mk, &block)?;
        let mut header = Header {
            format: FORMAT.into(),
            store: wire::hex(&store),
            name: name.into(),
            epoch: 1,
            factors: descs,
            rules: rs,
            keys,
            settings: Settings::default(),
            mac: None,
        };
        header.check()?;
        header.mac = Some(header_mac(rng, &store, &mk, &header)?);
        let (wu, afu, seed) = split_block(&block[..]);
        let k = Keyring {
            store,
            pending: Vec::new(),
            items: BTreeMap::new(),
            by_attr: HashMap::new(),
            chain: chain_start(&store, header.epoch),
            seq: 0,
            afu: Some(afu),
            keys: Some(Keys {
                mk,
                wu,
                id_seed: seed,
            }),
            rule: Some(1),
            header,
        };
        let bytes = k.header_file()?;
        Ok((k, bytes))
    }

    /// Reads a file without decrypting anything: the header is checked, the records kept.
    pub fn parse(bytes: &[u8]) -> Result<Keyring> {
        if bytes.len() > MAX_FILE {
            return Err(Error::Limit("keyring file size"));
        }
        let mut r = Reader::new(bytes);
        let hb = r.chunk(MAX_HEADER)?;
        let header: Header =
            serde_json::from_slice(hb).map_err(|_| Error::Malformed("keyring header"))?;
        header.check()?;
        let store = header.store_id()?;
        let at = r.pos();
        Ok(Keyring {
            chain: chain_start(&store, header.epoch),
            store,
            pending: bytes[at..].to_vec(),
            items: BTreeMap::new(),
            by_attr: HashMap::new(),
            seq: 0,
            afu: None,
            keys: None,
            rule: None,
            header,
        })
    }

    /// Takes in a newer copy of the same keyring's file (another process wrote it): the keys in
    /// memory stay, the header's tag is checked with them, the records are read again. Refused
    /// for another keyring's file.
    pub fn refresh(&mut self, bytes: &[u8]) -> Result<()> {
        let mut new = Keyring::parse(bytes)?;
        if new.store != self.store {
            return Err(Error::Damaged("the file now holds another keyring"));
        }
        if let Some(k) = &self.keys {
            check_mac(&new.store, &k.mk, &new.header)?;
        }
        new.afu = self.afu.take();
        new.keys = self.keys.take();
        new.rule = self.rule;
        if new.afu.is_some() {
            new.replay()?;
        }
        *self = new;
        Ok(())
    }

    /// Reads records another process appended (`tail` = the file's bytes after the length this
    /// keyring has read).
    pub fn take_tail(&mut self, tail: &[u8]) -> Result<()> {
        self.pending.extend_from_slice(tail);
        if self.afu.is_some() {
            self.replay()?;
        }
        Ok(())
    }

    /// The header.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Whether the full keys are in memory.
    pub fn unlocked(&self) -> bool {
        self.keys.is_some()
    }

    /// Whether the index and `after-first-unlock` secrets are readable.
    pub fn first_unlocked(&self) -> bool {
        self.afu.is_some()
    }

    /// Whether the last unlock used a rule that may export, move and change the policy.
    pub fn strong(&self) -> bool {
        self.rule.is_some_and(|r| {
            self.header
                .rules
                .iter()
                .find(|x| x.id == r)
                .is_some_and(|x| factor::rule_is_strong(&self.header.factors, &x.factors))
        })
    }

    /// The factors a rule of the policy needs, for the unlock prompt.
    pub fn rules(&self) -> Vec<Vec<&Factor>> {
        self.header
            .rules
            .iter()
            .map(|r| {
                r.factors
                    .iter()
                    .filter_map(|id| self.header.factors.iter().find(|f| f.id == *id))
                    .collect()
            })
            .collect()
    }

    /// Unlocks with what was presented: the first rule whose factors were all presented and
    /// whose wrap opens. The header's tag is checked and the records are read.
    pub fn unlock(&mut self, presented: &[(u8, Presented)]) -> Result<u8> {
        let mut found = None;
        for rule in &self.header.rules {
            let Some(secrets) = secrets_for(&self.header.factors, rule, presented) else {
                continue;
            };
            let Ok(secrets) = secrets else { continue };
            let refs: Vec<&[u8; 32]> = secrets.iter().map(|s| &**s).collect();
            if let Ok(mk) = factor::unwrap(&self.store, rule, &self.header.factors, &refs) {
                found = Some((rule.id, mk));
                break;
            }
        }
        let (rid, mk) = found.ok_or(Error::WrongFactors)?;
        check_mac(&self.store, &mk, &self.header)?;
        let block = open_block(&self.store, &mk, &self.header.keys)?;
        let (wu, afu, id_seed) = split_block(&block);
        if self.afu.is_none() {
            self.afu = Some(afu);
            self.replay()?;
        }
        self.keys = Some(Keys { mk, wu, id_seed });
        self.rule = Some(rid);
        Ok(rid)
    }

    /// Drops the master key, `dek_wu` and the identity (screen lock, sleep, timeout). The index
    /// and `after-first-unlock` secrets stay until [`Keyring::seal`].
    pub fn lock(&mut self) {
        self.keys = None;
        self.rule = None;
    }

    /// Drops every key and the index (sign-out).
    pub fn seal(&mut self) {
        self.lock();
        self.afu = None;
        self.items.clear();
        self.by_attr.clear();
    }

    fn replay(&mut self) -> Result<()> {
        let afu = self.afu.as_ref().ok_or(Error::Locked)?;
        let afu = **afu;
        let raw = std::mem::take(&mut self.pending);
        let mut r = Reader::new(&raw);
        while r.rest() > 0 {
            let rec = r.chunk(MAX_RECORD)?;
            let body = open_record(
                &self.store,
                self.header.epoch,
                self.seq,
                &self.chain,
                &afu,
                rec,
            )?;
            self.chain = chain_next(&self.chain, rec);
            self.seq += 1;
            self.apply(body)?;
        }
        Ok(())
    }

    fn apply(&mut self, body: Body) -> Result<()> {
        match body {
            Body::Put { meta, secret } => {
                meta.check()?;
                let secret = wire::unb64(&secret, MAX_SECRET + 64)?;
                if !self.items.contains_key(&meta.id) && self.items.len() >= MAX_ITEMS {
                    return Err(Error::Limit("item count"));
                }
                self.unindex(&meta.id);
                for (k, v) in &meta.attrs {
                    self.by_attr
                        .entry((k.clone(), v.clone()))
                        .or_default()
                        .insert(meta.id.clone());
                }
                self.items.insert(meta.id.clone(), Entry { meta, secret });
            }
            Body::Del { id } => {
                self.unindex(&id);
                self.items.remove(&id);
            }
        }
        Ok(())
    }

    fn unindex(&mut self, id: &str) {
        if let Some(old) = self.items.get(id) {
            for (k, v) in &old.meta.attrs {
                let key = (k.clone(), v.clone());
                if let Some(set) = self.by_attr.get_mut(&key) {
                    set.remove(id);
                    if set.is_empty() {
                        self.by_attr.remove(&key);
                    }
                }
            }
        }
    }

    fn record(&mut self, rng: &mut impl Random, body: &Body) -> Result<Vec<u8>> {
        let afu = **self.afu.as_ref().ok_or(Error::Locked)?;
        let salt: [u8; 32] = rng.bytes()?;
        let plain = Zeroizing::new(serde_json::to_vec(body).map_err(|e| Error::Io(e.to_string()))?);
        let ad = record_ad(&self.store, self.header.epoch, self.seq, &salt, &self.chain);
        let ct = wire::seal(
            &record_parts(&self.store, self.header.epoch, self.seq, &salt, &afu).as_refs(),
            &ad,
            &plain,
        );
        let mut rec = Vec::with_capacity(40 + ct.len());
        rec.extend_from_slice(&self.seq.to_be_bytes());
        rec.extend_from_slice(&salt);
        rec.extend_from_slice(&ct);
        if rec.len() > MAX_RECORD {
            return Err(Error::Limit("item size"));
        }
        self.chain = chain_next(&self.chain, &rec);
        self.seq += 1;
        let mut out = Vec::with_capacity(4 + rec.len());
        out.extend_from_slice(&(rec.len() as u32).to_be_bytes());
        out.extend_from_slice(&rec);
        Ok(out)
    }

    /// Adds or replaces an item. `meta.id` empty = a new item (an id is made). Returns the id and
    /// the bytes to append to the file. Needs the full unlock.
    pub fn put(
        &mut self,
        rng: &mut impl Random,
        mut meta: Meta,
        secret: &[u8],
    ) -> Result<(String, Vec<u8>)> {
        let keys = self.keys.as_ref().ok_or(Error::Locked)?;
        if secret.len() > MAX_SECRET {
            return Err(Error::Limit("secret size"));
        }
        if meta.id.is_empty() {
            meta.id = new_id(rng)?;
        }
        meta.check()?;
        if !self.items.contains_key(&meta.id) && self.items.len() >= MAX_ITEMS {
            return Err(Error::Limit("item count"));
        }
        let class_key = match meta.class {
            Class::WhenUnlocked => *keys.wu,
            Class::AfterFirstUnlock => **self.afu.as_ref().ok_or(Error::Locked)?,
        };
        let salt: [u8; 32] = rng.bytes()?;
        let mut blob = salt.to_vec();
        let id = wolcrypt::hash::unhex(&meta.id).ok_or(Error::Malformed("item id"))?;
        blob.extend(wire::seal(
            &secret_parts(&self.store, &id, meta.class, &salt, &class_key).as_refs(),
            &secret_ad(&self.store, &id, meta.class),
            secret,
        ));
        let body = Body::Put {
            meta: meta.clone(),
            secret: wire::b64(&blob),
        };
        let rec = self.record(rng, &body)?;
        let id = meta.id.clone();
        self.apply(body)?;
        Ok((id, rec))
    }

    /// Deletes an item; the bytes to append.
    pub fn delete(&mut self, rng: &mut impl Random, id: &str) -> Result<Vec<u8>> {
        if self.keys.is_none() {
            return Err(Error::Locked);
        }
        if !self.items.contains_key(id) {
            return Err(Error::NotFound);
        }
        let body = Body::Del { id: id.into() };
        let rec = self.record(rng, &body)?;
        self.apply(body)?;
        Ok(rec)
    }

    /// An item's metadata.
    pub fn meta(&self, id: &str) -> Option<&Meta> {
        self.items.get(id).map(|e| &e.meta)
    }

    /// Every item's metadata, in id order.
    pub fn all(&self) -> impl Iterator<Item = &Meta> {
        self.items.values().map(|e| &e.meta)
    }

    /// Item count.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are no items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Items whose attributes include every pair of `query` (in id order): the shortest posting
    /// list of the index, filtered. An empty query lists all.
    pub fn search(&self, query: &BTreeMap<String, String>) -> Vec<&Meta> {
        let mut best: Option<&BTreeSet<String>> = None;
        for (k, v) in query {
            match self.by_attr.get(&(k.clone(), v.clone())) {
                None => return Vec::new(),
                Some(s) if best.is_none_or(|b| s.len() < b.len()) => best = Some(s),
                Some(_) => {}
            }
        }
        match best {
            None => self.all().collect(),
            Some(ids) => ids
                .iter()
                .filter_map(|id| self.items.get(id))
                .map(|e| &e.meta)
                .filter(|m| m.matches(query))
                .collect(),
        }
    }

    /// The secret of item `id` for `caller` (None = the owner at the command line, who presented
    /// the factors). The class decides which unlock it needs; the access list decides whether a
    /// prompt is needed ([`Error::Denied`]).
    pub fn secret(&self, id: &str, caller: Option<&AppId>) -> Result<Zeroizing<Vec<u8>>> {
        let e = self.items.get(id).ok_or(if self.afu.is_none() {
            Error::Locked
        } else {
            Error::NotFound
        })?;
        let class_key = match e.meta.class {
            Class::WhenUnlocked => *self.keys.as_ref().ok_or(Error::Locked)?.wu,
            Class::AfterFirstUnlock => **self.afu.as_ref().ok_or(Error::Locked)?,
        };
        if let Some(c) = caller
            && !e.meta.allows(c)
        {
            return Err(Error::Denied);
        }
        let salt: [u8; 32] = e
            .secret
            .get(..32)
            .and_then(|s| s.try_into().ok())
            .ok_or(Error::Damaged("secret"))?;
        let id_b = wolcrypt::hash::unhex(id).ok_or(Error::Malformed("item id"))?;
        wire::open(
            &secret_parts(&self.store, &id_b, e.meta.class, &salt, &class_key).as_refs(),
            &secret_ad(&self.store, &id_b, e.meta.class),
            &e.secret[32..],
        )
        .ok_or(Error::Damaged("item secret"))
    }

    /// Adds `app` to item `id`'s access list ("Always allow"); the bytes to append.
    pub fn allow(&mut self, rng: &mut impl Random, id: &str, app: AppId) -> Result<Vec<u8>> {
        let mut meta = self.meta(id).ok_or(Error::NotFound)?.clone();
        if meta.acl.contains(&app) {
            return Ok(Vec::new());
        }
        meta.acl.push(app);
        let secret = self.secret(id, None)?;
        Ok(self.put(rng, meta, &secret)?.1)
    }

    /// Replaces item `id`'s access list (Settings, `wol keyring acl`); the bytes to append.
    pub fn set_acl(&mut self, rng: &mut impl Random, id: &str, acl: Vec<AppId>) -> Result<Vec<u8>> {
        let mut meta = self.meta(id).ok_or(Error::NotFound)?.clone();
        meta.acl = acl;
        let secret = self.secret(id, None)?;
        Ok(self.put(rng, meta, &secret)?.1)
    }

    /// The keyring's hybrid identity (signing key and recipient key); needs the full unlock.
    pub fn identity(&self) -> Result<crate::bundle::Identity> {
        let k = self.keys.as_ref().ok_or(Error::Locked)?;
        Ok(crate::bundle::Identity::from_seed(&k.id_seed))
    }

    /// Replaces the policy (enrol or remove factors). `factors` carry their secrets; every
    /// factor of the new policy must be presented with its secret. Needs a strong unlock.
    /// Returns the whole new file (the header changed, so the file is rewritten).
    pub fn set_policy(
        &mut self,
        rng: &mut impl Random,
        factors: Vec<(Factor, Zeroizing<[u8; 32]>)>,
        rules: &[Vec<u8>],
    ) -> Result<Vec<u8>> {
        if !self.strong() {
            return Err(Error::Policy(
                "changing how the keyring unlocks needs the password, a security key or the recovery code",
            ));
        }
        let keys = self.keys.as_ref().ok_or(Error::Locked)?;
        let descs: Vec<Factor> = factors.iter().map(|(f, _)| f.clone()).collect();
        let mut rs = Vec::new();
        for (i, ids) in rules.iter().enumerate() {
            let rid = u8::try_from(i + 1).map_err(|_| Error::Policy("between 1 and 80 rules"))?;
            let secrets: Vec<&[u8; 32]> = ids
                .iter()
                .map(|id| factors.iter().find(|(f, _)| f.id == *id).map(|(_, s)| &**s))
                .collect::<Option<_>>()
                .ok_or(Error::Malformed("rule names a factor that is not enrolled"))?;
            rs.push(factor::wrap(
                rng,
                &self.store,
                rid,
                &descs,
                ids,
                &secrets,
                &keys.mk,
            )?);
        }
        factor::check_policy(&descs, &rs)?;
        if !rs
            .iter()
            .any(|r| factor::rule_is_strong(&descs, &r.factors))
        {
            return Err(Error::Policy(
                "at least one way to unlock must use the password, a security key or the recovery code",
            ));
        }
        self.header.factors = descs;
        self.header.rules = rs;
        self.rule = None;
        self.compact(rng)
    }

    /// Changes the settings; the whole new file.
    pub fn set_settings(&mut self, rng: &mut impl Random, s: Settings) -> Result<Vec<u8>> {
        if self.keys.is_none() {
            return Err(Error::Locked);
        }
        self.header.settings = s;
        self.compact(rng)
    }

    /// Rewrites the whole file under the next epoch: header (tag remade) and every item as one
    /// record. Needs the full unlock.
    pub fn compact(&mut self, rng: &mut impl Random) -> Result<Vec<u8>> {
        let keys = self.keys.as_ref().ok_or(Error::Locked)?;
        let mk = keys.mk.clone();
        self.header.epoch += 1;
        self.header.mac = None;
        self.header.mac = Some(header_mac(rng, &self.store, &mk, &self.header)?);
        self.chain = chain_start(&self.store, self.header.epoch);
        self.seq = 0;
        let mut out = self.header_file()?;
        let entries: Vec<(Meta, String)> = self
            .items
            .values()
            .map(|e| (e.meta.clone(), wire::b64(&e.secret)))
            .collect();
        for (meta, secret) in entries {
            out.extend(self.record(rng, &Body::Put { meta, secret })?);
        }
        Ok(out)
    }

    fn header_file(&self) -> Result<Vec<u8>> {
        let h = serde_json::to_vec(&self.header).map_err(|e| Error::Io(e.to_string()))?;
        if h.len() > MAX_HEADER {
            return Err(Error::Limit("header size"));
        }
        let mut out = (h.len() as u32).to_be_bytes().to_vec();
        out.extend(h);
        Ok(out)
    }

    /// What `wol keyring export` and `move` carry: every item but `this-device-only` ones (and,
    /// when `collection` is given, only that collection), access lists dropped (another machine's
    /// apps ask once). Needs a strong unlock: a fingerprint or the chip alone never exports.
    pub fn export_payload(
        &self,
        now: u64,
        collection: Option<&str>,
    ) -> Result<crate::bundle::Payload> {
        if !self.strong() {
            return Err(Error::Policy(
                "exporting needs the password, a security key or the recovery code",
            ));
        }
        let mut items = Vec::new();
        for e in self.items.values() {
            if e.meta.device_only || collection.is_some_and(|c| c != e.meta.collection) {
                continue;
            }
            let mut meta = e.meta.clone();
            meta.acl.clear();
            let secret = self.secret(&meta.id, None)?;
            items.push(crate::bundle::Exported {
                meta,
                secret: wire::b64(&secret),
            });
        }
        Ok(crate::bundle::Payload {
            created: now,
            from: self.header.name.clone(),
            items,
        })
    }

    /// Merges imported items: an item with the same collection and attributes is replaced only
    /// when the imported one is newer; others are added under new ids. Returns (added, replaced,
    /// kept) and the bytes to append.
    pub fn import(
        &mut self,
        rng: &mut impl Random,
        payload: &crate::bundle::Payload,
    ) -> Result<((usize, usize, usize), Vec<u8>)> {
        if self.keys.is_none() {
            return Err(Error::Locked);
        }
        let (mut added, mut replaced, mut kept) = (0, 0, 0);
        let mut out = Vec::new();
        for x in &payload.items {
            let secret = Zeroizing::new(wire::unb64(&x.secret, MAX_SECRET)?);
            let same = self
                .search(&x.meta.attrs)
                .into_iter()
                .find(|m| m.collection == x.meta.collection && m.attrs == x.meta.attrs)
                .map(|m| (m.id.clone(), m.modified));
            let mut meta = x.meta.clone();
            meta.acl.clear();
            meta.device_only = false;
            match same {
                Some((_, modified)) if modified >= meta.modified => {
                    kept += 1;
                    continue;
                }
                Some((id, _)) => {
                    meta.id = id;
                    replaced += 1;
                }
                None => {
                    meta.id = String::new();
                    added += 1;
                }
            }
            out.extend(self.put(rng, meta, &secret)?.1);
        }
        Ok(((added, replaced, kept), out))
    }

    /// Record count and chain value: what an audit checkpoint signs (rollback detection).
    pub fn position(&self) -> (u64, u64, [u8; 64]) {
        (self.header.epoch, self.seq, self.chain)
    }
}

fn seal_block(
    rng: &mut impl Random,
    store: &[u8; 16],
    mk: &[u8; 32],
    block: &[u8; 96],
) -> Result<Sealed> {
    let salt: [u8; 32] = rng.bytes()?;
    let ct = wire::seal(
        &[format!("{FORMAT}/keys").as_bytes(), store, &salt, mk],
        &block_ad(store),
        block,
    );
    Ok(Sealed {
        salt: wire::b64(&salt),
        sealed: wire::b64(&ct),
    })
}

fn open_block(store: &[u8; 16], mk: &[u8; 32], s: &Sealed) -> Result<Zeroizing<Vec<u8>>> {
    let salt: [u8; 32] = wire::unb64_n(&s.salt)?;
    let ct = wire::unb64(&s.sealed, 96 + 16)?;
    let b = wire::open(
        &[format!("{FORMAT}/keys").as_bytes(), store, &salt, mk],
        &block_ad(store),
        &ct,
    )
    .ok_or(Error::Damaged("key block"))?;
    if b.len() != 96 {
        return Err(Error::Damaged("key block length"));
    }
    Ok(b)
}

fn block_ad(store: &[u8; 16]) -> Vec<u8> {
    [FORMAT.as_bytes(), b"/keys\0", store].concat()
}

/// The key block's three 32-byte keys (`dek_wu`, `dek_afu`, `id_seed`).
type Keys3 = (
    Zeroizing<[u8; 32]>,
    Zeroizing<[u8; 32]>,
    Zeroizing<[u8; 32]>,
);

fn split_block(b: &[u8]) -> Keys3 {
    let mut wu = Zeroizing::new([0u8; 32]);
    let mut afu = Zeroizing::new([0u8; 32]);
    let mut seed = Zeroizing::new([0u8; 32]);
    wu.copy_from_slice(&b[..32]);
    afu.copy_from_slice(&b[32..64]);
    seed.copy_from_slice(&b[64..96]);
    (wu, afu, seed)
}

fn header_mac(
    rng: &mut impl Random,
    store: &[u8; 16],
    mk: &[u8; 32],
    h: &Header,
) -> Result<Sealed> {
    let salt: [u8; 32] = rng.bytes()?;
    let tag = wire::seal(
        &[format!("{FORMAT}/header").as_bytes(), store, &salt, mk],
        &h.unsigned_bytes(),
        &[],
    );
    Ok(Sealed {
        salt: wire::b64(&salt),
        sealed: wire::b64(&tag),
    })
}

fn check_mac(store: &[u8; 16], mk: &[u8; 32], h: &Header) -> Result<()> {
    let m = h.mac.as_ref().ok_or(Error::Damaged("header tag missing"))?;
    let salt: [u8; 32] = wire::unb64_n(&m.salt)?;
    let tag = wire::unb64(&m.sealed, 16)?;
    wire::open(
        &[format!("{FORMAT}/header").as_bytes(), store, &salt, mk],
        &h.unsigned_bytes(),
        &tag,
    )
    .map(|_| ())
    .ok_or(Error::Damaged("keyring header"))
}

fn record_parts(
    store: &[u8; 16],
    epoch: u64,
    seq: u64,
    salt: &[u8; 32],
    afu: &[u8; 32],
) -> Vec<Vec<u8>> {
    vec![
        format!("{FORMAT}/rec").into_bytes(),
        store.to_vec(),
        epoch.to_be_bytes().to_vec(),
        seq.to_be_bytes().to_vec(),
        salt.to_vec(),
        afu.to_vec(),
    ]
}

fn record_ad(store: &[u8; 16], epoch: u64, seq: u64, salt: &[u8; 32], prev: &[u8; 64]) -> Vec<u8> {
    [
        FORMAT.as_bytes(),
        b"/rec\0",
        store,
        &epoch.to_be_bytes(),
        &seq.to_be_bytes(),
        salt,
        prev,
    ]
    .concat()
}

fn open_record(
    store: &[u8; 16],
    epoch: u64,
    seq: u64,
    prev: &[u8; 64],
    afu: &[u8; 32],
    rec: &[u8],
) -> Result<Body> {
    let mut r = Reader::new(rec);
    let got = r.u64()?;
    if got != seq {
        return Err(Error::Damaged("record order"));
    }
    let salt: [u8; 32] = r.arr()?;
    let ct = &rec[r.pos()..];
    let parts = record_parts(store, epoch, seq, &salt, afu);
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    let plain = wire::open(&refs, &record_ad(store, epoch, seq, &salt, prev), ct)
        .ok_or(Error::Damaged("record"))?;
    serde_json::from_slice(&plain).map_err(|_| Error::Damaged("record body"))
}

fn secret_parts(
    store: &[u8; 16],
    id: &[u8],
    class: Class,
    salt: &[u8; 32],
    key: &[u8; 32],
) -> Vec<Vec<u8>> {
    vec![
        format!("{FORMAT}/secret").into_bytes(),
        store.to_vec(),
        id.to_vec(),
        vec![class_byte(class)],
        salt.to_vec(),
        key.to_vec(),
    ]
}

fn secret_ad(store: &[u8; 16], id: &[u8], class: Class) -> Vec<u8> {
    [
        FORMAT.as_bytes(),
        b"/secret\0",
        store,
        id,
        &[class_byte(class)],
    ]
    .concat()
}

trait AsRefs {
    fn as_refs(&self) -> Vec<&[u8]>;
}

impl AsRefs for Vec<Vec<u8>> {
    fn as_refs(&self) -> Vec<&[u8]> {
        self.iter().map(Vec::as_slice).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_opens_only_for_its_item_class_and_store() {
        let (store, key, salt) = ([1u8; 16], [2u8; 32], [3u8; 32]);
        let (a, b) = ([4u8; 16], [5u8; 16]);
        let ct = wire::seal(
            &secret_parts(&store, &a, Class::WhenUnlocked, &salt, &key).as_refs(),
            &secret_ad(&store, &a, Class::WhenUnlocked),
            b"s",
        );
        let open = |s: &[u8; 16], id: &[u8; 16], c: Class| {
            wire::open(
                &secret_parts(s, id, c, &salt, &key).as_refs(),
                &secret_ad(s, id, c),
                &ct,
            )
            .is_some()
        };
        assert!(open(&store, &a, Class::WhenUnlocked));
        assert!(!open(&store, &b, Class::WhenUnlocked));
        assert!(!open(&store, &a, Class::AfterFirstUnlock));
        assert!(!open(&[9; 16], &a, Class::WhenUnlocked));
    }
}
