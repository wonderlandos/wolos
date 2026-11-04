// SPDX-License-Identifier: BSD-2-Clause
//! The revocation database `revoked.rkdb` (lead decision 2026-11-04 "RKDB"): every withdrawn id
//! in one sorted, signed binary file that a machine can search in microseconds at a million
//! entries, plus the rules that keep a machine from being shown an older one. Pure: bytes in,
//! answers out; `wol_core::trust` reads and writes the files.
//!
//! Layout (integers big-endian):
//! ```text
//! header   59 B  "WRKDB" | format u8 = 1 | serial u64 | issued u32 | expires u32 | count u32
//!                | prev_hash [32] (SHA-512/256 of the previous full file; zeros for the first)
//!                | flags u8 (bit 0: a root section follows the entries; other bits 0)
//! entries  count x 69 B, strictly ascending by id: id [64] | at u32 | reason u8
//! root     only with flag bit 0, the root-signed section (also a file of its own, `.rkdr`):
//!                "WRKDR" | format u8 = 1 | issued u32 | count u32 | count x 69 B sorted
//!                | signer id [64] | Ed25519 [64] | SLH-DSA-SHA2-128s [7856]
//!                the root signs `wolos/revocation/1` over every byte before its signer id
//! sig      signer id [64] | Ed25519 [64] | ML-DSA-65 [3309]: the Index intermediate signs
//!                `wolos/revocation/1` over every byte before its signer id
//! ```
//! An id is the 64-byte certificate or file id (rule "IDS"); a legacy-length serial (16 hex, the
//! TEST authority) is its 8 bytes followed by 56 zero bytes, which no SHA-512 output is in
//! practice. Reasons: 1 key-compromise, 2 abuse, 3 superseded, 4 ceased, 5 withdrawn-version.
//!
//! The Index withdraws leaves and file certificates; only the root section withdraws an
//! intermediate or a root, never the root that signed it (an emergency entry, `woltrust
//! emergency`). A delta (`.rkdd`) carries only the new entries and the full signature of the
//! file it produces, so applying it to a matching base gives the same bytes as the full file.
use crate::revoked::Reason;
use wolcrypt::sign::{ED_SIG, ML_SIG, SLH_SIG, Sig};

/// File magic.
pub const MAGIC: &[u8; 5] = b"WRKDB";
/// Root section magic.
pub const ROOT_MAGIC: &[u8; 5] = b"WRKDR";
/// Delta magic.
pub const DELTA_MAGIC: &[u8; 5] = b"WRKDD";
/// Format number.
pub const FORMAT: u8 = 1;
/// Header length.
pub const HEADER: usize = 59;
/// One entry.
pub const ENTRY: usize = 69;
/// Root section header length (magic, format, issued, count).
pub const ROOT_HEADER: usize = 14;
/// Index signature block: signer id, Ed25519, ML-DSA-65.
pub const INDEX_SIG: usize = 64 + ED_SIG + ML_SIG;
/// Root signature block: signer id, Ed25519, SLH-DSA-SHA2-128s.
pub const ROOT_SIG: usize = 64 + ED_SIG + SLH_SIG;
/// Most entries in one file: 4 million (276 MB); a bound for hostile input, far above need.
pub const MAX_ENTRIES: usize = 4_000_000;
/// Most entries in a root section.
pub const MAX_ROOT_ENTRIES: usize = 64;
/// How long a database is fresh: 24 hours from `issued` (republished hourly).
pub const LIFETIME: u64 = 86_400;
/// The signature label (the same as `revoked.json`; the content starts with `WRKDB`, so the two
/// can never be confused).
pub const LABEL: &str = "wolos/revocation/1";
/// Flag bit: a root section follows the entries.
pub const FLAG_ROOT: u8 = 1;

/// Why a database, a section or a delta is refused (stable names, used by the vectors).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not the layout above: magic, format, lengths, flags, unsorted or repeated ids, a reason
    /// outside 1..=5, `expires` not after `issued`.
    Malformed,
    /// The signer is not a valid Index intermediate (or, for the root section, a pinned root),
    /// or a half does not verify.
    BadSignature,
    /// A root section withdraws the root that signed it, or the Index withdraws an intermediate
    /// or a root.
    NotAllowed,
    /// Older than the floor this machine already holds.
    Older,
    /// Leaves out an entry the machine holds, or weakens one.
    Shrunk,
    /// `prev_hash` does not chain onto the database held one serial below.
    Chain,
    /// The delta does not start from the database held.
    BaseMismatch,
    /// Disagrees with the checkpoint both sources published for its serial.
    Checkpoint,
}

impl Error {
    /// Stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Error::Malformed => "rkdb-malformed",
            Error::BadSignature => "rkdb-bad-signature",
            Error::NotAllowed => "rkdb-not-allowed",
            Error::Older => "rkdb-older",
            Error::Shrunk => "rkdb-shrunk",
            Error::Chain => "rkdb-chain",
            Error::BaseMismatch => "rkdb-base-mismatch",
            Error::Checkpoint => "rkdb-checkpoint",
        }
    }
}

/// The reason byte.
pub fn reason_code(r: Reason) -> u8 {
    match r {
        Reason::KeyCompromise => 1,
        Reason::Abuse => 2,
        Reason::Superseded => 3,
        Reason::Ceased => 4,
        Reason::WithdrawnVersion => 5,
    }
}

/// Inverse of [`reason_code`].
pub fn reason_of(b: u8) -> Option<Reason> {
    Some(match b {
        1 => Reason::KeyCompromise,
        2 => Reason::Abuse,
        3 => Reason::Superseded,
        4 => Reason::Ceased,
        5 => Reason::WithdrawnVersion,
        _ => return None,
    })
}

/// How strong a reason is when two entries for one id meet in a merge: key-compromise, abuse,
/// ceased, superseded, withdrawn-version (hard before soft; ceased keeps fewer files valid than a
/// superseded, which on a trusted certificate only downgrades).
pub fn strength(r: Reason) -> u8 {
    match r {
        Reason::KeyCompromise => 5,
        Reason::Abuse => 4,
        Reason::Ceased => 3,
        Reason::Superseded => 2,
        Reason::WithdrawnVersion => 1,
    }
}

/// The 64-byte id of a serial: a 128-hex id, or a 16-hex legacy serial padded with zeros.
pub fn id_bytes(serial: &str) -> Option<[u8; 64]> {
    if let Some(b) = wolcrypt::id::from_hex(serial) {
        return Some(b);
    }
    if !crate::cert::is_legacy_serial(serial) {
        return None;
    }
    let raw = wolcrypt::hash::unhex(serial)?;
    let mut out = [0u8; 64];
    out[..8].copy_from_slice(&raw);
    Some(out)
}

/// The text of an id: 16 hex when it is a padded legacy serial, else 128 hex.
pub fn id_text(id: &[u8; 64]) -> String {
    if id[8..].iter().all(|b| *b == 0) {
        wolcrypt::hash::hex(&id[..8])
    } else {
        wolcrypt::hash::hex(id)
    }
}

/// One entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The withdrawn id.
    pub id: [u8; 64],
    /// When, Unix seconds.
    pub at: u32,
    /// Why.
    pub reason: Reason,
}

impl Entry {
    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.id);
        out.extend_from_slice(&self.at.to_be_bytes());
        out.push(reason_code(self.reason));
    }

    fn read(b: &[u8]) -> Option<Entry> {
        let id: [u8; 64] = b.get(..64)?.try_into().ok()?;
        let at = u32::from_be_bytes(b.get(64..68)?.try_into().ok()?);
        Some(Entry {
            id,
            at,
            reason: reason_of(*b.get(68)?)?,
        })
    }

    /// The entry as the JSON list writes it.
    pub fn to_json(&self) -> crate::revoked::Entry {
        crate::revoked::Entry {
            serial: id_text(&self.id),
            at: u64::from(self.at),
            reason: self.reason,
        }
    }
}

/// Sorted entries as raw 69-byte records: searched in place, never copied into a tree, so a
/// million entries cost their 69 MB and one binary search 20 comparisons.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entries {
    raw: Vec<u8>,
}

impl Entries {
    /// From records already checked to be sorted and valid.
    fn from_raw(raw: Vec<u8>) -> Entries {
        Entries { raw }
    }

    /// Sorts and joins `list` (repeated ids merged as [`merge`] does).
    pub fn from_list(list: &[Entry]) -> Entries {
        let mut v = list.to_vec();
        v.sort_by_key(|a| a.id);
        let mut out: Vec<Entry> = Vec::with_capacity(v.len());
        for e in v {
            match out.last_mut() {
                Some(l) if l.id == e.id => *l = stronger(*l, e),
                _ => out.push(e),
            }
        }
        let mut raw = Vec::with_capacity(out.len() * ENTRY);
        for e in &out {
            e.write(&mut raw);
        }
        Entries { raw }
    }

    /// How many.
    pub fn len(&self) -> usize {
        self.raw.len() / ENTRY
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// The `i`-th entry.
    pub fn get(&self, i: usize) -> Option<Entry> {
        Entry::read(self.raw.get(i * ENTRY..(i + 1) * ENTRY)?)
    }

    fn id_at(&self, i: usize) -> &[u8] {
        &self.raw[i * ENTRY..i * ENTRY + 64]
    }

    /// Binary search for `id`.
    pub fn lookup(&self, id: &[u8; 64]) -> Option<Entry> {
        let (mut lo, mut hi) = (0, self.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.id_at(mid).cmp(&id[..]) {
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
                core::cmp::Ordering::Equal => return self.get(mid),
            }
        }
        None
    }

    /// Every entry in id order.
    pub fn iter(&self) -> impl Iterator<Item = Entry> + '_ {
        self.raw
            .as_chunks::<ENTRY>()
            .0
            .iter()
            .filter_map(|r| Entry::read(r))
    }

    /// The raw records.
    pub fn as_bytes(&self) -> &[u8] {
        &self.raw
    }
}

fn stronger(a: Entry, b: Entry) -> Entry {
    let reason = if strength(b.reason) > strength(a.reason) {
        b.reason
    } else {
        a.reason
    };
    Entry {
        id: a.id,
        at: a.at.min(b.at),
        reason,
    }
}

/// Checks one run of records: valid reasons, strictly ascending ids.
fn check_run(raw: &[u8]) -> bool {
    let mut prev: Option<&[u8]> = None;
    for r in raw.as_chunks::<ENTRY>().0 {
        if reason_of(r[68]).is_none() || prev.is_some_and(|p| p >= &r[..64]) {
            return false;
        }
        prev = Some(&r[..64]);
    }
    true
}

/// Records per thread before the check is split (std threads only; no new crate).
const PARALLEL_FROM: usize = 1 << 16;

/// Checks `raw` holds `n` valid records in strictly ascending id order. Large files are checked
/// in parallel runs (the seams between runs too), with `std::thread::scope`.
fn check_records(raw: &[u8], n: usize) -> Result<(), Error> {
    if raw.len() != n * ENTRY {
        return Err(Error::Malformed);
    }
    let threads = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(16);
    if n < PARALLEL_FROM || threads < 2 {
        return if check_run(raw) {
            Ok(())
        } else {
            Err(Error::Malformed)
        };
    }
    let per = n.div_ceil(threads) * ENTRY;
    let ok = std::thread::scope(|s| {
        let runs: Vec<_> = raw
            .chunks(per)
            .map(|c| s.spawn(move || check_run(c)))
            .collect();
        runs.into_iter().all(|h| h.join().unwrap_or(false))
    });
    let seams = (per..raw.len())
        .step_by(per)
        .all(|at| raw[at - ENTRY..at - ENTRY + 64] < raw[at..at + 64]);
    if ok && seams {
        Ok(())
    } else {
        Err(Error::Malformed)
    }
}

/// Union of two sorted sets, never dropping an id; where both name one id, the earlier time and
/// the stronger reason ([`strength`]) win. Linear in the two sizes.
pub fn merge(a: &Entries, b: &Entries) -> Entries {
    let (mut i, mut j) = (0, 0);
    let mut raw = Vec::with_capacity(a.raw.len() + b.raw.len());
    while i < a.len() || j < b.len() {
        let next = match (a.get(i), b.get(j)) {
            (Some(x), Some(y)) if x.id == y.id => {
                i += 1;
                j += 1;
                stronger(x, y)
            }
            (Some(x), Some(y)) if x.id < y.id => {
                i += 1;
                x
            }
            (Some(_), Some(y)) | (None, Some(y)) => {
                j += 1;
                y
            }
            (Some(x), None) => {
                i += 1;
                x
            }
            (None, None) => break,
        };
        next.write(&mut raw);
    }
    Entries { raw }
}

/// True when every entry of `old` is in `new` with a time no later and a reason no weaker.
pub fn covers(new: &Entries, old: &Entries) -> bool {
    old.iter().all(|o| {
        new.lookup(&o.id)
            .is_some_and(|n| n.at <= o.at && strength(n.reason) >= strength(o.reason))
    })
}

/// A root-signed section: emergency withdrawals of intermediates and roots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootSection {
    /// When the root signed it.
    pub issued: u32,
    /// The entries.
    pub entries: Entries,
    /// The signing root's id.
    pub signer: [u8; 64],
    /// The root's signature.
    pub sig: Sig,
    /// The exact bytes.
    pub bytes: Vec<u8>,
}

impl RootSection {
    /// The bytes the root signs (everything before the signer id).
    pub fn signed_part(&self) -> &[u8] {
        &self.bytes[..self.bytes.len() - ROOT_SIG]
    }

    /// Parses one section from the start of `b`; returns it and its length.
    pub fn parse(b: &[u8]) -> Result<(RootSection, usize), Error> {
        if b.get(..5) != Some(&ROOT_MAGIC[..]) || b.get(5) != Some(&FORMAT) {
            return Err(Error::Malformed);
        }
        let issued = u32::from_be_bytes(
            b.get(6..10)
                .ok_or(Error::Malformed)?
                .try_into()
                .map_err(|_| Error::Malformed)?,
        );
        let count = len_of(u32::from_be_bytes(
            b.get(10..14)
                .ok_or(Error::Malformed)?
                .try_into()
                .map_err(|_| Error::Malformed)?,
        ));
        if count > MAX_ROOT_ENTRIES {
            return Err(Error::Malformed);
        }
        let end = ROOT_HEADER + count * ENTRY;
        let raw = b.get(ROOT_HEADER..end).ok_or(Error::Malformed)?;
        check_records(raw, count)?;
        let total = end + ROOT_SIG;
        let sigb = b.get(end..total).ok_or(Error::Malformed)?;
        let signer: [u8; 64] = sigb[..64].try_into().map_err(|_| Error::Malformed)?;
        let sig = Sig {
            ed25519: sigb[64..128].try_into().map_err(|_| Error::Malformed)?,
            second: sigb[128..].to_vec(),
        };
        Ok((
            RootSection {
                issued,
                entries: Entries::from_raw(raw.to_vec()),
                signer,
                sig,
                bytes: b[..total].to_vec(),
            },
            total,
        ))
    }

    /// A section of `entries` signed by the root `signer` (its id) with `secret`.
    pub fn make(
        issued: u32,
        entries: &[Entry],
        signer: &str,
        secret: &crate::make::Secret,
    ) -> Result<Vec<u8>, Error> {
        let e = Entries::from_list(entries);
        if e.len() > MAX_ROOT_ENTRIES {
            return Err(Error::Malformed);
        }
        let mut out = Vec::with_capacity(ROOT_HEADER + e.raw.len() + ROOT_SIG);
        out.extend_from_slice(ROOT_MAGIC);
        out.push(FORMAT);
        out.extend_from_slice(&issued.to_be_bytes());
        out.extend_from_slice(&len32(e.len()).to_be_bytes());
        out.extend_from_slice(&e.raw);
        let s = secret.sign_message(&wolcrypt::sign::message(LABEL, &out));
        if s.second.len() != SLH_SIG {
            return Err(Error::BadSignature);
        }
        out.extend_from_slice(&id_bytes(signer).ok_or(Error::Malformed)?);
        out.extend_from_slice(&s.ed25519);
        out.extend_from_slice(&s.second);
        Ok(out)
    }
}

/// A parsed database (signatures not yet checked: [`crate::Store::load_rkdb`] does that).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Db {
    /// Grows with every publication.
    pub serial: u64,
    /// Issued, Unix seconds.
    pub issued: u32,
    /// Fresh until, Unix seconds (issued + 24 h).
    pub expires: u32,
    /// SHA-512/256 of the previous full file.
    pub prev_hash: [u8; 32],
    /// Index-signed entries.
    pub entries: Entries,
    /// The root-signed section, when present.
    pub root: Option<RootSection>,
    /// The Index signer's id.
    pub signer: [u8; 64],
    /// The Index signature.
    pub sig: Sig,
    /// SHA-512/256 of the whole file (the next file's `prev_hash`, the checkpoint's hash).
    pub hash: [u8; 32],
    /// Length of the signed part (everything before the signer id).
    signed_len: usize,
}

/// A count as the u32 the layout stores (every count is capped far below `u32::MAX` first).
fn len32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A stored u32 as a length (saturating on a 16-bit target, where every check then fails).
fn len_of(x: u32) -> usize {
    usize::try_from(x).unwrap_or(usize::MAX)
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl Db {
    /// Parses a whole file. Linear in its size; nothing is copied but the entry records.
    pub fn parse(b: &[u8]) -> Result<Db, Error> {
        if b.len() < HEADER + INDEX_SIG || &b[..5] != MAGIC || b[5] != FORMAT {
            return Err(Error::Malformed);
        }
        let serial = u64::from_be_bytes(b[6..14].try_into().map_err(|_| Error::Malformed)?);
        let issued = u32_at(b, 14);
        let expires = u32_at(b, 18);
        let count = len_of(u32_at(b, 22));
        let prev_hash: [u8; 32] = b[26..58].try_into().map_err(|_| Error::Malformed)?;
        let flags = b[58];
        if flags & !FLAG_ROOT != 0 || expires <= issued || count > MAX_ENTRIES {
            return Err(Error::Malformed);
        }
        let end = HEADER + count * ENTRY;
        let raw = b.get(HEADER..end).ok_or(Error::Malformed)?;
        check_records(raw, count)?;
        let (root, at) = if flags & FLAG_ROOT != 0 {
            let (r, n) = RootSection::parse(&b[end..])?;
            (Some(r), end + n)
        } else {
            (None, end)
        };
        if b.len() != at + INDEX_SIG {
            return Err(Error::Malformed);
        }
        let sigb = &b[at..];
        Ok(Db {
            serial,
            issued,
            expires,
            prev_hash,
            entries: Entries::from_raw(raw.to_vec()),
            root,
            signer: sigb[..64].try_into().map_err(|_| Error::Malformed)?,
            sig: Sig {
                ed25519: sigb[64..128].try_into().map_err(|_| Error::Malformed)?,
                second: sigb[128..].to_vec(),
            },
            hash: wolcrypt::hash::sha512_256(b),
            signed_len: at,
        })
    }

    /// The Index-signed message of a file `b` this was parsed from.
    pub fn message(&self, b: &[u8]) -> Vec<u8> {
        wolcrypt::sign::message(LABEL, &b[..self.signed_len.min(b.len())])
    }

    /// The entry for `serial` (128-hex id or 16-hex legacy serial), Index or root.
    pub fn lookup(&self, serial: &str) -> Option<(Entry, bool)> {
        let id = id_bytes(serial)?;
        if let Some(e) = self.root.as_ref().and_then(|r| r.entries.lookup(&id)) {
            return Some((e, true));
        }
        self.entries.lookup(&id).map(|e| (e, false))
    }

    /// The database as the JSON list (`revoked.json`, the human-readable form): Index entries,
    /// in id order. Root-signed entries stay in their own signed section.
    pub fn to_list(&self) -> crate::revoked::ListText {
        crate::revoked::ListText {
            format: 1,
            serial: self.serial,
            issued: u64::from(self.issued),
            expires: u64::from(self.expires),
            entries: self.entries.iter().map(|e| e.to_json()).collect(),
        }
    }
}

/// The fields of a new database.
#[derive(Clone, Debug)]
pub struct Make<'a> {
    /// Its serial.
    pub serial: u64,
    /// Issued.
    pub issued: u32,
    /// Expires (issued + [`LIFETIME`]).
    pub expires: u32,
    /// SHA-512/256 of the previous full file (zeros for the first).
    pub prev_hash: [u8; 32],
    /// The Index entries.
    pub entries: &'a Entries,
    /// The root section bytes, carried as they are.
    pub root: Option<&'a [u8]>,
}

fn header(m: &Make) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER);
    out.extend_from_slice(MAGIC);
    out.push(FORMAT);
    out.extend_from_slice(&m.serial.to_be_bytes());
    out.extend_from_slice(&m.issued.to_be_bytes());
    out.extend_from_slice(&m.expires.to_be_bytes());
    out.extend_from_slice(&len32(m.entries.len()).to_be_bytes());
    out.extend_from_slice(&m.prev_hash);
    out.push(if m.root.is_some() { FLAG_ROOT } else { 0 });
    out
}

fn unsigned(m: &Make) -> Vec<u8> {
    let mut out = header(m);
    out.reserve(m.entries.raw.len() + m.root.map_or(0, <[u8]>::len) + INDEX_SIG);
    out.extend_from_slice(&m.entries.raw);
    if let Some(r) = m.root {
        out.extend_from_slice(r);
    }
    out
}

/// Builds and signs a database as the Index intermediate `signer` (its id) with `secret`.
pub fn make(m: &Make, signer: &str, secret: &crate::make::Secret) -> Result<Vec<u8>, Error> {
    let mut out = unsigned(m);
    let s = secret.sign_message(&wolcrypt::sign::message(LABEL, &out));
    if s.second.len() != ML_SIG {
        return Err(Error::BadSignature);
    }
    out.extend_from_slice(&id_bytes(signer).ok_or(Error::Malformed)?);
    out.extend_from_slice(&s.ed25519);
    out.extend_from_slice(&s.second);
    Db::parse(&out)?;
    Ok(out)
}

/// A delta from the database `base` to `target` (both full files, `target` newer and a superset):
/// `"WRKDD" | format u8 | from_serial u64 | to_serial u64 | base_hash [32] | target header [59] |
/// added count u32 | added x 69 B (sorted) | root section length u32 | root section | target
/// Index signature block`. Nothing but the new entries travels; the signature is the target's.
pub fn delta(base: &[u8], target: &[u8]) -> Result<Vec<u8>, Error> {
    let b = Db::parse(base)?;
    let t = Db::parse(target)?;
    if t.serial <= b.serial || !covers(&t.entries, &b.entries) {
        return Err(Error::Shrunk);
    }
    let added: Vec<Entry> = t
        .entries
        .iter()
        .filter(|e| b.entries.lookup(&e.id) != Some(*e))
        .collect();
    if added.iter().any(|e| b.entries.lookup(&e.id).is_some()) {
        // A changed entry (earlier time, stronger reason) cannot travel as an addition.
        return Err(Error::Shrunk);
    }
    let added = Entries::from_list(&added);
    let mut out = Vec::new();
    out.extend_from_slice(DELTA_MAGIC);
    out.push(FORMAT);
    out.extend_from_slice(&b.serial.to_be_bytes());
    out.extend_from_slice(&t.serial.to_be_bytes());
    out.extend_from_slice(&b.hash);
    out.extend_from_slice(&target[..HEADER]);
    out.extend_from_slice(&len32(added.len()).to_be_bytes());
    out.extend_from_slice(&added.raw);
    let root = t.root.as_ref().map_or(&[][..], |r| &r.bytes[..]);
    out.extend_from_slice(&len32(root.len()).to_be_bytes());
    out.extend_from_slice(root);
    out.extend_from_slice(&target[t.signed_len..]);
    Ok(out)
}

/// The full file a delta produces from `base`, or why not. The result still has to verify
/// ([`crate::Store::load_rkdb`]): the signature in the delta is the target file's.
pub fn apply(base: &[u8], d: &[u8]) -> Result<Vec<u8>, Error> {
    let b = Db::parse(base)?;
    if d.len() < 5 + 1 + 8 + 8 + 32 + HEADER + 4 || &d[..5] != DELTA_MAGIC || d[5] != FORMAT {
        return Err(Error::Malformed);
    }
    let from = u64::from_be_bytes(d[6..14].try_into().map_err(|_| Error::Malformed)?);
    let to = u64::from_be_bytes(d[14..22].try_into().map_err(|_| Error::Malformed)?);
    if from != b.serial || d[22..54] != b.hash {
        return Err(Error::BaseMismatch);
    }
    let th = &d[54..54 + HEADER];
    let mut at = 54 + HEADER;
    let n = len_of(u32_at(d, at));
    at += 4;
    if n > MAX_ENTRIES {
        return Err(Error::Malformed);
    }
    let raw = d.get(at..at + n * ENTRY).ok_or(Error::Malformed)?;
    check_records(raw, n)?;
    at += n * ENTRY;
    let added = Entries::from_raw(raw.to_vec());
    if added.iter().any(|e| b.entries.lookup(&e.id).is_some()) {
        return Err(Error::Malformed);
    }
    let rlen = len_of(u32_at(d, at.min(d.len().saturating_sub(4))));
    if d.len() < at + 4 {
        return Err(Error::Malformed);
    }
    at += 4;
    let root = d.get(at..at + rlen).ok_or(Error::Malformed)?;
    at += rlen;
    let sig = d.get(at..).ok_or(Error::Malformed)?;
    if sig.len() != INDEX_SIG
        || &th[..5] != MAGIC
        || u64::from_be_bytes(th[6..14].try_into().map_err(|_| Error::Malformed)?) != to
    {
        return Err(Error::Malformed);
    }
    let merged = merge(&b.entries, &added);
    if len_of(u32_at(th, 22)) != merged.len() || (th[58] & FLAG_ROOT != 0) != !root.is_empty() {
        return Err(Error::Malformed);
    }
    let mut out = Vec::with_capacity(HEADER + merged.raw.len() + rlen + INDEX_SIG);
    out.extend_from_slice(th);
    out.extend_from_slice(&merged.raw);
    out.extend_from_slice(root);
    out.extend_from_slice(sig);
    Db::parse(&out)?;
    Ok(out)
}

/// The rules for taking a newly verified database `new` (lead decision "RKDB"): its serial is not
/// below `floor` (the image's database, every serial seen in a release manifest or a file
/// certificate, the checkpoints); it keeps every entry of `held` (no shrinking, no weakening);
/// one serial above `held`, its `prev_hash` is `held`'s hash.
pub fn accept(new: &Db, held: Option<&Db>, floor: u64) -> Result<(), Error> {
    if new.serial < floor {
        return Err(Error::Older);
    }
    if let Some(h) = held {
        if new.serial < h.serial {
            return Err(Error::Older);
        }
        if !covers(&new.entries, &h.entries) {
            return Err(Error::Shrunk);
        }
        if let (Some(hr), nr) = (&h.root, &new.root)
            && !nr.as_ref().is_some_and(|r| covers(&r.entries, &hr.entries))
        {
            return Err(Error::Shrunk);
        }
        if new.serial == h.serial + 1 && new.prev_hash != h.hash {
            return Err(Error::Chain);
        }
        if new.serial == h.serial && new.hash != h.hash {
            return Err(Error::Chain);
        }
    }
    Ok(())
}

/// The latest serial and hash, published twice (the CDN and the GitHub public log).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// The latest serial.
    pub serial: u64,
    /// SHA-512/256 of that full file.
    pub hash: [u8; 32],
}

impl Checkpoint {
    /// Parses `{"format":1,"serial":N,"sha512_256":"<64 hex>"}` (at most 256 bytes).
    pub fn parse(b: &[u8]) -> Option<Checkpoint> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct T {
            format: u32,
            serial: u64,
            sha512_256: String,
        }
        if b.len() > 256 {
            return None;
        }
        let t: T = serde_json::from_slice(b).ok()?;
        let h = wolcrypt::hash::unhex(&t.sha512_256)?;
        (t.format == 1).then_some(())?;
        Some(Checkpoint {
            serial: t.serial,
            hash: h.try_into().ok()?,
        })
    }

    /// The JSON text.
    pub fn to_json(&self) -> String {
        format!(
            "{{\"format\":1,\"serial\":{},\"sha512_256\":\"{}\"}}",
            self.serial,
            wolcrypt::hash::hex(&self.hash)
        )
    }

    /// The checkpoint of a full file.
    pub fn of(db: &Db) -> Checkpoint {
        Checkpoint {
            serial: db.serial,
            hash: db.hash,
        }
    }
}

/// What the two checkpoints say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agreement {
    /// Both answered and agree.
    Agree(Checkpoint),
    /// Both answered and differ: warn, keep the higher serial as the floor.
    Disagree,
    /// Only one answered: warn, keep it as the floor.
    OneSource,
    /// Neither answered.
    None,
}

/// Combines the CDN checkpoint `a` and the public log's `b` with the `floor` held. Returns the new
/// floor (never lower) and the agreement. A machine never drops below the floor; a checkpoint
/// only raises it.
pub fn checkpoints(a: Option<Checkpoint>, b: Option<Checkpoint>, floor: u64) -> (u64, Agreement) {
    match (a, b) {
        (Some(x), Some(y)) if x == y => (floor.max(x.serial), Agreement::Agree(x)),
        (Some(x), Some(y)) => (floor.max(x.serial).max(y.serial), Agreement::Disagree),
        (Some(x), None) | (None, Some(x)) => (floor.max(x.serial), Agreement::OneSource),
        (None, None) => (floor, Agreement::None),
    }
}

/// A database whose serial both checkpoints name must have their hash.
pub fn check_checkpoint(db: &Db, agreed: Option<Checkpoint>) -> Result<(), Error> {
    match agreed {
        Some(c) if c.serial == db.serial && c.hash != db.hash => Err(Error::Checkpoint),
        _ => Ok(()),
    }
}

/// Whether a machine may install a new app now (existing apps keep running whatever this says).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Yes.
    Open,
    /// No: connected, but the list is past its 24 hours or below the floor ("could not update
    /// the list"); installs of new apps wait.
    HoldNew,
}

/// The install gate: a connected machine whose newest database expired or sits below the floor
/// holds installs of new apps; offline, the stale rules of ADR 0046 apply instead.
pub fn gate(
    expires: Option<u64>,
    held_serial: Option<u64>,
    floor: u64,
    now: u64,
    connected: bool,
) -> Gate {
    let stale = expires.is_none_or(|e| now >= e);
    let behind = held_serial.is_none_or(|s| s < floor);
    if connected && (stale || behind) {
        Gate::HoldNew
    } else {
        Gate::Open
    }
}

/// The sentence for [`Gate::HoldNew`].
pub const HOLD_SENTENCE: &str = "WolOS could not update the list of withdrawn certificates, \
    so new apps wait until it can. Apps already installed keep working.";

#[cfg(test)]
mod tests {
    use super::*;

    fn e(n: u8, at: u32, r: Reason) -> Entry {
        let mut id = [0u8; 64];
        id[0] = n;
        id[63] = 1;
        Entry { id, at, reason: r }
    }

    #[test]
    fn merge_is_a_union_with_earliest_time_and_strongest_reason() {
        let a = Entries::from_list(&[e(1, 10, Reason::Superseded), e(3, 5, Reason::Ceased)]);
        let b = Entries::from_list(&[e(1, 20, Reason::KeyCompromise), e(2, 7, Reason::Abuse)]);
        let m = merge(&a, &b);
        assert_eq!(m.len(), 3);
        assert_eq!(
            m.lookup(&e(1, 0, Reason::Abuse).id),
            Some(e(1, 10, Reason::KeyCompromise))
        );
        assert!(covers(&m, &a) && covers(&m, &b));
        assert_eq!(merge(&b, &a), m, "commutative");
        assert!(!covers(&a, &m));
    }

    #[test]
    fn ids_round_trip_both_lengths() {
        let legacy = "0123456789abcdef";
        assert_eq!(id_text(&id_bytes(legacy).unwrap()), legacy);
        let full = "ab".repeat(64);
        assert_eq!(id_text(&id_bytes(&full).unwrap()), full);
        assert!(id_bytes("xyz").is_none());
    }

    #[test]
    fn checkpoints_never_lower_the_floor() {
        let c = |s| Checkpoint {
            serial: s,
            hash: [s as u8; 32],
        };
        assert_eq!(
            checkpoints(Some(c(5)), Some(c(5)), 3),
            (5, Agreement::Agree(c(5)))
        );
        assert_eq!(
            checkpoints(Some(c(5)), Some(c(7)), 3),
            (7, Agreement::Disagree)
        );
        assert_eq!(checkpoints(Some(c(2)), None, 3), (3, Agreement::OneSource));
        assert_eq!(checkpoints(None, None, 3), (3, Agreement::None));
        assert_eq!(Checkpoint::parse(c(9).to_json().as_bytes()), Some(c(9)));
        assert!(Checkpoint::parse(b"{}").is_none());
    }

    #[test]
    fn the_gate_holds_new_installs_only_when_connected_and_behind() {
        assert_eq!(gate(Some(100), Some(5), 5, 50, true), Gate::Open);
        assert_eq!(gate(Some(100), Some(5), 5, 150, true), Gate::HoldNew);
        assert_eq!(gate(Some(100), Some(4), 5, 50, true), Gate::HoldNew);
        assert_eq!(gate(Some(100), Some(4), 5, 150, false), Gate::Open);
        assert_eq!(gate(None, None, 0, 1, true), Gate::HoldNew);
    }

    #[test]
    fn garbage_never_panics() {
        for b in [&b""[..], b"WRKDB", &[0u8; 4000][..], b"WRKDD\x01"] {
            assert!(Db::parse(b).is_err());
            assert!(RootSection::parse(b).is_err());
        }
    }
}
