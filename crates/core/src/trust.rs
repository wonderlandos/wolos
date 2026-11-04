// SPDX-License-Identifier: BSD-2-Clause
//! The trust store on disk and the verdict for a `.wol` (ADR 0046, docs/modules/trust.md). Files
//! are read through [`Host`] with a size cap each, handed to the pure `woltrust` crate, and the
//! system clock is read once per check. Roots and intermediates come from the image
//! ([`ROOTS_DIR`], [`INTERMEDIATES_DIR`], [`IMAGE_REVOKED`]); newer intermediates and lists that
//! the update-check service verified are kept in [`STATE_DIR`]. Anything unreadable, oversize or
//! not verifying is left out, never trusted; a test root is never loaded.
use crate::package;
use crate::system::Host;
use std::path::Path;
use woltrust::{Input, Outcome, Purpose, Store};

/// Pinned roots A and B (`*.wolcert`), fixed at image build.
pub const ROOTS_DIR: &str = "/usr/share/wolos/trust/roots";
/// Intermediates known to the image (`*.wolcert`).
pub const INTERMEDIATES_DIR: &str = "/usr/share/wolos/trust/intermediates";
/// The revocation list of the image; its signature is the same path plus [`SIG_SUFFIX`].
pub const IMAGE_REVOKED: &str = "/usr/share/wolos/trust/revoked.json";
/// Written only by the update-check service, after it verified what it keeps.
pub const STATE_DIR: &str = "/var/lib/wolos/trust";
/// Intermediates and re-issued leaves from the signed index, below [`STATE_DIR`].
pub const STATE_INTERMEDIATES: &str = "/var/lib/wolos/trust/intermediates";
/// The newer revocation list below [`STATE_DIR`].
pub const STATE_REVOKED: &str = "/var/lib/wolos/trust/revoked.json";
/// Appended to a list path for its signature file.
pub const SIG_SUFFIX: &str = ".wsig";
/// Every revocation this machine has seen, kept forever (audit M1): `wol trust update` appends to
/// it, never shortens it, and every check consults it, so a later list that lacks an entry, a
/// rolled-back list file or an expired Index never un-revokes a certificate.
pub const SEEN_FILE: &str = "/var/lib/wolos/trust/seen-revocations.json";
/// Extension of a certificate file.
pub const CERT_EXT: &str = ".wolcert";
/// Where the revocation list lives on the CDN server, below each of [`crate::cdn::HOSTS`]; its
/// signature is the same URL plus [`SIG_SUFFIX`].
pub const REVOKED_PATH: &str = "/plugins/revoked.json";
/// The revocation database of the image (lead decision 2026-11-04 "RKDB"): current at build
/// time; its serial is a floor the machine never goes below.
pub const IMAGE_RKDB: &str = "/usr/share/wolos/trust/revoked.rkdb";
/// The newer revocation database below [`STATE_DIR`], replaced only by a verified superset.
pub const STATE_RKDB: &str = "/var/lib/wolos/trust/revoked.rkdb";
/// The floor: the highest database serial this machine was told of (a signed release manifest, a
/// file certificate, the checkpoints), one decimal number.
pub const FLOOR_FILE: &str = "/var/lib/wolos/trust/rkdb-floor";
/// Written by update-check each run in which the list's server answered (Unix seconds).
pub const LAST_ATTEMPT_FILE: &str = "/var/lib/wolos/trust/last-attempt";
/// Written by update-check after `wol trust update` installed a list or database.
pub const LAST_OK_FILE: &str = "/var/lib/wolos/trust/last-ok";
/// How long a `last-attempt` stamp counts as "connected": two hourly runs.
pub const CONNECTED_WINDOW: u64 = 2 * 3600;

/// The floor file's new text when `serial` raises the floor held in `old` (its bytes, empty when
/// there is none), or None when it does not: the floor is never lowered. The callers that may write
/// [`FLOOR_FILE`] (root: `wol trust update --floor N`, wopkg as root) write it with
/// `shared::atomic::atomic_write`.
pub fn floor_after(old: &[u8], serial: u64) -> Option<Vec<u8>> {
    (serial > parse_floor(old)).then(|| floor_text(serial))
}

/// Where the database lives on the CDN server, below each of [`crate::cdn::HOSTS`].
pub const RKDB_PATH: &str = "/plugins/revoked.rkdb";
/// Largest database read: 128 MiB (about 1.9 million entries); a bound for hostile input.
pub const MAX_RKDB_BYTES: usize = 128 << 20;

/// The database's URLs in the order tried (`wol trust update`, `update-check`).
pub fn rkdb_urls() -> Vec<String> {
    crate::cdn::HOSTS
        .iter()
        .map(|h| format!("{h}{RKDB_PATH}"))
        .collect()
}

/// The end of the previous flat Ed25519 key (audit 2 M6): from 2027-02-01T00:00:00Z a `.wol` or
/// `.wolplug` signed only with it is refused, roots or not. The lead may move this date.
pub const LEGACY_UNTIL: u64 = 1_801_440_000;

/// Root-signed sections kept on this machine (`<sha512/256 hex>.rkdr`), loaded before every
/// list and database (audit 2 M5), so a withdrawn Index's lists never count.
pub const STATE_SECTIONS: &str = "/var/lib/wolos/trust/sections";
/// Extension of a kept root section.
pub const SECTION_EXT: &str = ".rkdr";

/// The file name a verified section is kept under in [`STATE_SECTIONS`] (SHA-256 of its bytes).
pub fn section_file_name(r: &woltrust::store::HeldRoot) -> String {
    format!(
        "{}{SECTION_EXT}",
        crate::package::sha256_hex(&r.section.bytes)
    )
}

/// Most kept sections read.
const MAX_SECTIONS: usize = 64;

/// Where the hub publishes the current root-signed emergency section, below each of
/// [`crate::cdn::HOSTS`] (fetched on every `wol trust update`, independently of the database).
pub const EMERGENCY_PATH: &str = "/plugins/emergency.rkdr";
/// Where the hub publishes the current intermediates (a JSON array of `.wolcert` objects).
pub const INTERMEDIATES_PATH: &str = "/plugins/intermediates.json";
/// Largest intermediates file read.
pub const MAX_INTERMEDIATES_BYTES: usize = 512 * 1024;

/// `path` below each of [`crate::cdn::HOSTS`], in the order tried.
pub fn cdn_urls(path: &str) -> Vec<String> {
    crate::cdn::HOSTS
        .iter()
        .map(|h| format!("{h}{path}"))
        .collect()
}

/// Whether the state database may join a store that holds the image's (audit 2 M4): it keeps
/// every entry of the image's, chains onto it, and is not below the floor.
pub fn state_db_counts(bytes: &[u8], store: &Store) -> Result<(), String> {
    let db = woltrust::rkdb::Db::parse(bytes).map_err(|_| "damaged, left out".to_owned())?;
    let image = store.best_db().map(|d| d.db.clone());
    woltrust::rkdb::accept(&db, image.as_ref(), store.floor)
        .map_err(|_| "older than the image's database or the floor, left out".to_owned())
}

fn seen_of(old: &[u8]) -> Result<woltrust::revoked::Seen, String> {
    if old.is_empty() {
        Ok(woltrust::revoked::Seen::default())
    } else {
        woltrust::revoked::Seen::parse(old).map_err(|e| format!("{SEEN_FILE}: {e}"))
    }
}

fn root_list(r: &woltrust::store::HeldRoot) -> woltrust::RevList {
    woltrust::RevList {
        list: woltrust::revoked::ListText {
            format: 1,
            serial: 0,
            issued: u64::from(r.section.issued),
            expires: u64::from(r.section.issued) + 1,
            entries: r.section.entries.iter().map(|e| e.to_json()).collect(),
        },
        signer: r.signer.clone(),
        signer_is_root: true,
    }
}

/// The floor file's text for `floor`.
pub fn floor_text(floor: u64) -> Vec<u8> {
    format!("{floor}\n").into_bytes()
}

/// The floor in a floor file's text (0 when unreadable).
pub fn parse_floor(text: &[u8]) -> u64 {
    std::str::from_utf8(text)
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0)
}

/// What a newly downloaded database came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RkdbUpdate {
    /// Its serial.
    pub serial: u64,
    /// The floor after it and the checkpoints.
    pub floor: u64,
    /// A warning for the log and `wol trust status` (the checkpoints disagree, or one is
    /// missing).
    pub warning: Option<String>,
}

/// The revocation list's URLs in the order tried (`wol trust update`, `update-check`): the CDN
/// under the project's domain, then its mirror name.
pub fn revoked_urls() -> Vec<String> {
    crate::cdn::HOSTS
        .iter()
        .map(|h| format!("{h}{REVOKED_PATH}"))
        .collect()
}

/// Most certificate files read from one folder.
const MAX_FILES: usize = 64;

/// What this machine trusts: the pure store plus a note of what was left out.
#[derive(Clone, Debug, Default)]
pub struct TrustStore {
    /// Roots, intermediates and verified lists.
    pub store: Store,
}

/// A `.wol` that passed the structural checks, with what its signature is worth.
#[derive(Clone, Debug)]
pub struct Checked {
    /// The package.
    pub verified: package::Verified,
    /// Trust value, reason, signer and staleness.
    pub outcome: Outcome,
}

/// Reads one file of at most `cap` bytes; None when missing, unreadable or larger.
fn read_capped(host: &dyn Host, path: &str, cap: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; cap + 1];
    let n = host.read_head(path, &mut buf)?;
    (n <= cap).then(|| {
        buf.truncate(n);
        buf
    })
}

fn read_certs(host: &dyn Host, dir: &str) -> Vec<Vec<u8>> {
    host.list_dir(dir)
        .iter()
        .filter(|p| p.ends_with(CERT_EXT))
        .take(MAX_FILES)
        .filter_map(|p| read_capped(host, p, woltrust::MAX_CERT_FILE))
        .collect()
}

fn read_list(host: &dyn Host, path: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let list = read_capped(host, path, woltrust::MAX_REVOKED_BYTES)?;
    let sig = read_capped(host, &format!("{path}{SIG_SUFFIX}"), woltrust::MAX_WSIG)?;
    Some((list, sig))
}

impl TrustStore {
    /// Loads the store from the image and [`STATE_DIR`] at the host clock. Never fails: a missing
    /// folder is an empty store, which makes every signature "unknown".
    pub fn load(host: &dyn Host) -> TrustStore {
        let roots = read_certs(host, ROOTS_DIR);
        let mut inter = read_certs(host, INTERMEDIATES_DIR);
        inter.extend(read_certs(host, STATE_INTERMEDIATES));
        let lists: Vec<_> = [IMAGE_REVOKED, STATE_REVOKED]
            .iter()
            .filter_map(|p| read_list(host, p))
            .collect();
        let now = unix(host);
        // Audit 2 M5: the kept root-signed sections go in first, so every list and database is
        // judged with them (a withdrawn Index's lists never count).
        let sections: Vec<Vec<u8>> = host
            .list_dir(STATE_SECTIONS)
            .iter()
            .filter(|p| p.ends_with(SECTION_EXT))
            .take(MAX_SECTIONS)
            .filter_map(|p| read_capped(host, p, 1 << 20))
            .collect();
        let mut store = Store::load_with_sections(&roots, &inter, &sections, &lists, now, false);
        // The image's database sets the floor; the state's is newer or equal (checked when kept).
        if let Some(b) = read_capped(host, IMAGE_RKDB, MAX_RKDB_BYTES) {
            let _ = store.load_rkdb(&b, now);
            store.floor = store.db_serial().unwrap_or(0);
        }
        if let Some(t) = read_capped(host, FLOOR_FILE, 64) {
            store.raise_floor(parse_floor(&t));
        }
        // Audit 2 M4: the state database counts only when it keeps everything the image's had
        // and is not below the floor (a restored or rolled-back file is left out, with a note).
        if let Some(b) = read_capped(host, STATE_RKDB, MAX_RKDB_BYTES) {
            match state_db_counts(&b, &store) {
                Ok(()) => {
                    let _ = store.load_rkdb(&b, now);
                }
                Err(e) => store.notes.push(format!("{STATE_RKDB}: {e}")),
            }
        }
        let stamp = |p| {
            read_capped(host, p, 64)
                .map(|t| parse_floor(&t))
                .filter(|t| *t > 0)
        };
        store.last_attempt = stamp(LAST_ATTEMPT_FILE);
        store.last_ok = stamp(LAST_OK_FILE);
        match read_capped(host, SEEN_FILE, woltrust::revoked::MAX_SEEN_BYTES)
            .map(|b| woltrust::revoked::Seen::parse(&b))
        {
            Some(Ok(seen)) => store.remember(&seen),
            Some(Err(e)) => store.notes.push(format!("{SEEN_FILE}: {e}")),
            None => {}
        }
        TrustStore { store }
    }

    /// What a raw Ed25519 `SHA256.sig` by WolOS's previous flat key `key` is worth (audit H1): its
    /// key id (16 hex, [`woltrust::request::key_id`]) can be withdrawn in `revoked.json` like a
    /// certificate; once this machine holds WolOS roots it is refused; before that it is
    /// [`woltrust::Trust::Legacy`], never WolOS.
    pub fn legacy(&self, key: &[u8; 32]) -> Outcome {
        // Audit 2 M3: a list may name the flat key by its 16-hex key id or by its id under rule
        // IDS (SHA-512, 128 hex); either withdraws it.
        let short = woltrust::request::key_id(key);
        let full = woltrust::request::legacy_key_id(key);
        if let Some(code) = woltrust::verify::withdrawn(&self.store, &short)
            .or_else(|| woltrust::verify::withdrawn(&self.store, &full))
        {
            return legacy_outcome(woltrust::Trust::Withdrawn, code);
        }
        if self.store.roots.is_empty() {
            legacy_outcome(woltrust::Trust::Legacy, woltrust::Code::LegacyKey)
        } else {
            legacy_outcome(woltrust::Trust::Invalid, woltrust::Code::LegacyRefused)
        }
    }

    /// [`TrustStore::legacy`] at `now`: from [`LEGACY_UNTIL`] on, the previous flat key vouches
    /// for nothing (audit 2 M6), whether or not the machine holds roots.
    pub fn legacy_at(&self, key: &[u8; 32], now: u64) -> Outcome {
        let o = self.legacy(key);
        if o.trust == woltrust::Trust::Legacy && now >= LEGACY_UNTIL {
            return legacy_outcome(woltrust::Trust::Invalid, woltrust::Code::LegacyRefused);
        }
        o
    }

    fn legacy_of(&self, trust: package::Trust, keys: &[[u8; 32]], now: u64) -> Outcome {
        match trust {
            package::Trust::Signed(i) => keys.get(i).map_or_else(
                || legacy_outcome(woltrust::Trust::Invalid, woltrust::Code::SignerUnknown),
                |k| self.legacy_at(k, now),
            ),
            package::Trust::Unsigned => {
                legacy_outcome(woltrust::Trust::Community, woltrust::Code::Unsigned)
            }
        }
    }

    /// Judges a `.wol`: the structural checks of [`package::verify_extra`], then the hybrid
    /// signature if the file has one, else the legacy Ed25519 signature against `legacy_keys`
    /// (WolOS's own flat keys: [`TrustStore::legacy`]), else unsigned. `first_seen` is the
    /// signed index's first sighting of this exact file, if the caller has it.
    pub fn verify_wol(
        &self,
        host: &dyn Host,
        path: &Path,
        legacy_keys: &[[u8; 32]],
        first_seen: Option<u64>,
    ) -> Result<Checked, String> {
        self.verify_wol_at(path, legacy_keys, first_seen, unix(host))
    }

    /// [`TrustStore::verify_wol`] at a given clock (wol-admind passes its own).
    pub fn verify_wol_at(
        &self,
        path: &Path,
        legacy_keys: &[[u8; 32]],
        first_seen: Option<u64>,
        now: u64,
    ) -> Result<Checked, String> {
        let (verified, extra) = package::verify_extra(path, legacy_keys)?;
        let outcome = if extra.wsig.is_some() {
            woltrust::verify(
                &self.store,
                &Input {
                    purpose: Purpose::Package,
                    content: &extra.manifest,
                    app_id: Some(&verified.info.id),
                    wsig: extra.wsig.as_deref(),
                    chain: extra.chain.as_deref(),
                    first_seen,
                    now,
                },
            )
        } else {
            self.legacy_of(verified.trust, legacy_keys, now)
        };
        Ok(Checked { verified, outcome })
    }
}

/// A `.wolplug` that passed the structural checks, with what its signature is worth.
#[derive(Clone, Debug)]
pub struct CheckedPlugin {
    /// The plugin.
    pub verified: package::VerifiedPlugin,
    /// Trust value, reason, signer and staleness.
    pub outcome: Outcome,
}

impl CheckedPlugin {
    /// True when someone vouches for the plugin: its certificate chain says made by WolOS, by a
    /// trusted developer or by a proven origin, or the legacy index key signed it (0.12 indexes,
    /// only while this machine holds no WolOS root and the key is not withdrawn). An expired
    /// certificate counts as unsigned, as for a `.wol`.
    pub fn vouched(&self) -> bool {
        matches!(
            self.outcome.trust,
            woltrust::Trust::Wolos
                | woltrust::Trust::Trusted
                | woltrust::Trust::Origin
                | woltrust::Trust::Legacy
        )
    }

    /// The plugin when [`CheckedPlugin::vouched`], else why not: the certificate's sentence when
    /// it has one (expired, list too old), else "not signed".
    pub fn require_vouched(self) -> Result<package::VerifiedPlugin, String> {
        if self.vouched() {
            return Ok(self.verified);
        }
        Err(match self.outcome.code {
            woltrust::Code::Unsigned => {
                "The plugin is not signed by the WolOS plugin index.".into()
            }
            c => c
                .sentence()
                .unwrap_or("The plugin is not signed by the WolOS plugin index.")
                .to_owned(),
        })
    }
}

/// What `wol trust status` shows: what the store holds and how old its revocation list is.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    /// Pinned roots loaded.
    pub roots: usize,
    /// Intermediates loaded.
    pub intermediates: usize,
    /// Serial of the newest verified revocation list.
    pub list_serial: Option<u64>,
    /// When it was issued.
    pub list_issued: Option<u64>,
    /// When it runs out.
    pub list_expires: Option<u64>,
    /// Seconds since it was issued.
    pub list_age: Option<u64>,
    /// `fresh`, `stale`, `too-old` or `missing`.
    pub freshness: &'static str,
    /// Certificates withdrawn in it.
    pub revoked: usize,
    /// What was left out and why.
    pub notes: Vec<String>,
    /// The subject text of each pinned root (who vouches), in load order.
    pub root_texts: Vec<String>,
    /// Revocations remembered from earlier lists ([`SEEN_FILE`]).
    pub remembered: usize,
    /// Serial of the newest verified revocation database ([`STATE_RKDB`], [`IMAGE_RKDB`]).
    pub rkdb_serial: Option<u64>,
    /// Entries in it.
    pub rkdb_entries: usize,
    /// The floor ([`FLOOR_FILE`]).
    pub floor: u64,
    /// True when the newest database is below the floor ("could not update the list").
    pub behind: bool,
    /// The update service's `last-attempt` and `last-ok` stamps.
    pub last_attempt: Option<u64>,
    /// See `last_attempt`.
    pub last_ok: Option<u64>,
}

impl TrustStore {
    /// Judges a decompressed `.wolplug`: [`package::verify_wolplug_extra`], then the hybrid
    /// signature (label `wolos/plugin/1`) when `CHAIN` and `SHA256.wsig` are there. A present
    /// hybrid signature that is wrong or withdrawn refuses the plugin; none leaves the legacy
    /// verdict (index key, or unsigned).
    pub fn verify_wolplug(
        &self,
        tar: &[u8],
        legacy_keys: &[[u8; 32]],
        now: u64,
    ) -> Result<CheckedPlugin, String> {
        let (verified, extra) = package::verify_wolplug_extra(tar, legacy_keys)?;
        let refused = |o: &Outcome| {
            matches!(
                o.trust,
                woltrust::Trust::Invalid | woltrust::Trust::Withdrawn
            )
        };
        let say = |o: &Outcome| {
            o.code
                .sentence()
                .unwrap_or("The plugin's signature does not check out.")
                .to_owned()
        };
        let outcome = if extra.wsig.is_some() {
            let o = woltrust::verify(
                &self.store,
                &Input {
                    purpose: Purpose::Plugin,
                    content: &extra.manifest,
                    app_id: Some(&verified.manifest.id),
                    wsig: extra.wsig.as_deref(),
                    chain: extra.chain.as_deref(),
                    first_seen: None,
                    now,
                },
            );
            if refused(&o) {
                return Err(say(&o));
            }
            // An expired certificate next to the 0.12 index key: the index key still counts.
            match (o.trust, verified.trust) {
                (woltrust::Trust::Community, package::Trust::Signed(_)) => {
                    self.legacy_of(verified.trust, legacy_keys, now)
                }
                _ => o,
            }
        } else {
            self.legacy_of(verified.trust, legacy_keys, now)
        };
        if refused(&outcome) {
            return Err(say(&outcome));
        }
        Ok(CheckedPlugin { verified, outcome })
    }

    /// Checks a freshly downloaded revocation list before it may replace [`STATE_REVOKED`]: its
    /// signature must verify against this store (the Index intermediate or a root, test roots
    /// never), and its serial must be higher than every list held. Returns the new serial.
    pub fn accept_list(&self, list: &[u8], sig: &[u8], now: u64) -> Result<u64, String> {
        self.accept(list, sig, now, false)
    }

    fn accept(&self, list: &[u8], sig: &[u8], now: u64, allow_test: bool) -> Result<u64, String> {
        let probe = self.probe(list, sig, now, allow_test);
        let new = probe.lists.first().ok_or_else(|| {
            format!(
                "the new revocation list does not verify ({})",
                probe.notes.join("; ")
            )
        })?;
        // Audit M1: a list that leaves out a withdrawal this machine has seen from a signer of
        // the same rank (Index or root) is refused, so the pipeline cannot forget one.
        let held = self.store.all_lists();
        let dropped = held
            .iter()
            .filter(|l| l.signer_is_root == new.signer_is_root)
            .flat_map(|l| l.list.entries.iter())
            .find(|e| {
                e.reason.is_hard()
                    && !new
                        .list
                        .entries
                        .iter()
                        .any(|n| n.serial == e.serial && n.reason == e.reason)
            });
        if let Some(e) = dropped {
            return Err(format!(
                "the new revocation list leaves out {} ({}), which was withdrawn before",
                e.serial,
                e.reason.as_str()
            ));
        }
        let new = new.list.serial;
        match self.store.best_list() {
            Some(old) if old.list.serial >= new => Err(format!(
                "the new revocation list ({new}) is not newer than the one held ({})",
                old.list.serial
            )),
            _ => Ok(new),
        }
    }

    /// The memory file ([`SEEN_FILE`]) after the lists held and the new list: `old` (its current
    /// bytes, empty when there is none) with every entry not yet in it appended. A damaged `old`
    /// is an error, so a damaged memory is never replaced by a shorter one.
    pub fn memory_after(
        &self,
        old: &[u8],
        list: &[u8],
        sig: &[u8],
        now: u64,
    ) -> Result<Vec<u8>, String> {
        self.memory(old, list, sig, now, false)
    }

    fn memory(
        &self,
        old: &[u8],
        list: &[u8],
        sig: &[u8],
        now: u64,
        allow_test: bool,
    ) -> Result<Vec<u8>, String> {
        let mut seen = if old.is_empty() {
            woltrust::revoked::Seen::default()
        } else {
            woltrust::revoked::Seen::parse(old).map_err(|e| format!("{SEEN_FILE}: {e}"))?
        };
        let probe = self.probe(list, sig, now, allow_test);
        for l in self.store.lists.iter().chain(probe.lists.iter()) {
            seen.add(l);
        }
        Ok(seen.to_bytes())
    }

    /// A store of this one's roots and intermediates with only the new list.
    fn probe(&self, list: &[u8], sig: &[u8], now: u64, allow_test: bool) -> Store {
        let roots: Vec<Vec<u8>> = self
            .store
            .roots
            .iter()
            .map(woltrust::Cert::to_bytes)
            .collect();
        let inter: Vec<Vec<u8>> = self
            .store
            .intermediates
            .iter()
            .map(woltrust::Cert::to_bytes)
            .collect();
        Store::load(&roots, &inter, &[(list, sig)], now, allow_test)
    }

    /// Checks a downloaded revocation database before it may replace [`STATE_RKDB`]: it verifies
    /// against this store (the Index, its root section by a root), its serial is not below the
    /// floor, it keeps every entry of the database held and chains onto it, and when both
    /// checkpoints (`cdn`, `log`: the bytes of each, if they answered) agree on its serial, it
    /// has their hash. Disagreeing or missing checkpoints only warn; the higher serial becomes the
    /// floor.
    pub fn accept_rkdb(
        &self,
        bytes: &[u8],
        cdn: Option<&[u8]>,
        log: Option<&[u8]>,
        now: u64,
    ) -> Result<RkdbUpdate, String> {
        use woltrust::rkdb;
        let say = |e: rkdb::Error| match e {
            rkdb::Error::Older => {
                "the revocation database is older than one this computer already knows".to_owned()
            }
            rkdb::Error::Shrunk => {
                "the revocation database leaves out a withdrawal this computer holds".to_owned()
            }
            rkdb::Error::Chain => "the revocation database does not follow the one held".to_owned(),
            rkdb::Error::Checkpoint => {
                "the revocation database is not the one both checkpoints name".to_owned()
            }
            rkdb::Error::Malformed => "the revocation database is damaged".to_owned(),
            rkdb::Error::NotAllowed => {
                "the revocation database withdraws what its signer may not".to_owned()
            }
            _ => "the revocation database does not verify against this computer's roots".to_owned(),
        };
        let (held, _) = self.store.check_rkdb(bytes, now).map_err(say)?;
        let cp = |b: Option<&[u8]>| b.and_then(rkdb::Checkpoint::parse);
        let (_, agreement) = rkdb::checkpoints(cp(cdn), cp(log), self.store.floor);
        // Audit 2 M2: an unsigned checkpoint from one source, or two that disagree, never raise
        // the floor (one URL could freeze every machine); only two that agree do, for this check.
        let floor = match agreement {
            rkdb::Agreement::Agree(c) => self.store.floor.max(c.serial),
            _ => self.store.floor,
        };
        let prev = self.store.best_db().map(|d| d.db.clone());
        rkdb::accept(&held.db, prev.as_ref(), floor).map_err(say)?;
        let agreed = match agreement {
            rkdb::Agreement::Agree(c) => Some(c),
            _ => None,
        };
        rkdb::check_checkpoint(&held.db, agreed).map_err(say)?;
        let warning = match agreement {
            rkdb::Agreement::Disagree => Some(
                "the two checkpoints (CDN and public log) disagree; the floor stays where it was"
                    .to_owned(),
            ),
            rkdb::Agreement::OneSource => Some("only one checkpoint answered".to_owned()),
            rkdb::Agreement::None => Some("no checkpoint answered".to_owned()),
            rkdb::Agreement::Agree(_) => None,
        };
        // The floor kept on disk rises only to the serial of a database that verified.
        Ok(RkdbUpdate {
            serial: held.db.serial,
            floor: self.store.floor.max(held.db.serial),
            warning,
        })
    }

    /// The root-signed section a database carries, verified on its own (audit 2 H1): kept even
    /// when the database's Index block then fails, because the section may be what withdraws
    /// that Index.
    pub fn salvage_root_section(
        &self,
        bytes: &[u8],
        now: u64,
    ) -> Option<woltrust::store::HeldRoot> {
        let db = woltrust::rkdb::Db::parse(bytes).ok()?;
        let r = db.root?;
        self.check_section(&r, now).ok()
    }

    fn check_section(
        &self,
        r: &woltrust::rkdb::RootSection,
        now: u64,
    ) -> Result<woltrust::store::HeldRoot, String> {
        // Audit 2 L2: a section from the future is refused here too.
        if u64::from(r.issued) > now.saturating_add(woltrust::verify::CLOCK_SKEW) {
            return Err("the emergency section is dated in the future".into());
        }
        self.store.check_root_section(r, now).map_err(|_| {
            "the emergency section is not signed by a WolOS root that may sign it".into()
        })
    }

    /// Checks a bare root-signed section (`emergency.rkdr`, `woltrust emergency`): a pinned root
    /// signed it, not naming itself, not dated in the future.
    pub fn accept_root_section(
        &self,
        bytes: &[u8],
        now: u64,
    ) -> Result<woltrust::store::HeldRoot, String> {
        let (r, n) = woltrust::rkdb::RootSection::parse(bytes)
            .map_err(|_| "the emergency section is damaged".to_owned())?;
        if n != bytes.len() {
            return Err("the emergency section is damaged".into());
        }
        self.check_section(&r, now)
    }

    /// The memory file ([`SEEN_FILE`]) with a verified root section's entries added (kept for
    /// ever, so a replayed older list or database cannot undo them).
    pub fn memory_after_root(
        &self,
        old: &[u8],
        r: &woltrust::store::HeldRoot,
    ) -> Result<Vec<u8>, String> {
        let mut seen = seen_of(old)?;
        seen.add(&root_list(r));
        Ok(seen.to_bytes())
    }

    /// Intermediates published by the hub (`intermediates.json`, a JSON array of `.wolcert`
    /// objects): each kept only when it is an intermediate with a derived id, signed by one pinned
    /// root and co-signed by the other, both valid now and not withdrawn, the certificate valid
    /// now (audit 2 H1c: a rotated Index or Authors CA reaches machines without a new image).
    /// Returns `(serial, .wolcert bytes)` of each that checks out; the rest are named in the error
    /// list.
    pub fn accept_intermediates(
        &self,
        bytes: &[u8],
        now: u64,
    ) -> (Vec<(String, Vec<u8>)>, Vec<String>) {
        let mut ok = Vec::new();
        let mut bad = Vec::new();
        let items: Vec<serde_json::Value> = if bytes.len() > MAX_INTERMEDIATES_BYTES {
            bad.push("the intermediates file is too large".into());
            Vec::new()
        } else {
            serde_json::from_slice(bytes).unwrap_or_else(|_| {
                bad.push("the intermediates file is damaged".into());
                Vec::new()
            })
        };
        for (i, item) in items.iter().take(MAX_FILES).enumerate() {
            let raw = serde_json::to_vec(item).unwrap_or_default();
            match woltrust::Cert::parse(&raw)
                .map_err(str::to_owned)
                .and_then(|c| self.check_intermediate(&c, now).map(|()| c))
            {
                Ok(c) => ok.push((c.body.serial.clone(), c.to_bytes())),
                Err(e) => bad.push(format!("intermediate {i}: {e}")),
            }
        }
        (ok, bad)
    }

    /// The two-root rule for one intermediate (see [`TrustStore::accept_intermediates`]):
    /// woltrust's `Store::check_intermediate`, and a derived id (a legacy-length TEST serial is
    /// never published).
    fn check_intermediate(&self, c: &woltrust::Cert, now: u64) -> Result<(), String> {
        if woltrust::cert::is_legacy_serial(&c.body.serial) {
            return Err("not an intermediate with a derived id".into());
        }
        self.store.check_intermediate(c, now).map_err(|code| {
            code.sentence()
                .unwrap_or("it does not check out")
                .to_owned()
        })
    }

    /// The memory file ([`SEEN_FILE`]) after a database: its root-signed entries (emergency
    /// withdrawals of intermediates and roots) are kept forever, like every root list's.
    pub fn memory_after_rkdb(&self, old: &[u8], bytes: &[u8], now: u64) -> Result<Vec<u8>, String> {
        let mut seen = seen_of(old)?;
        for l in &self.store.lists {
            seen.add(l);
        }
        if let Some(r) = self.salvage_root_section(bytes, now) {
            seen.add(&root_list(&r));
        }
        // Audit 2 M4: the hard withdrawals (key-compromise, abuse) a new database adds over the
        // one held are remembered too, so a rolled-back database file cannot drop them.
        if let Ok((h, _)) = self.store.check_rkdb(bytes, now) {
            let held = self.store.best_db();
            let entries: Vec<woltrust::revoked::Entry> =
                h.db.entries
                    .iter()
                    .filter(|e| e.reason.is_hard())
                    .filter(|e| held.is_none_or(|d| d.db.entries.lookup(&e.id) != Some(*e)))
                    .map(|e| e.to_json())
                    .collect();
            if !entries.is_empty() {
                seen.add(&woltrust::RevList {
                    list: woltrust::revoked::ListText {
                        format: 1,
                        serial: 0,
                        issued: u64::from(h.db.issued),
                        expires: u64::from(h.db.issued) + 1,
                        entries,
                    },
                    signer: h.signer,
                    signer_is_root: false,
                });
            }
        }
        Ok(seen.to_bytes())
    }

    /// A signed release manifest or a file certificate said its signer held database `serial`:
    /// this machine never accepts an older one again ([`FLOOR_FILE`] keeps it).
    pub fn observe_rkdb_serial(&mut self, serial: u64) {
        self.store.raise_floor(serial);
    }

    /// Whether a new app may be installed now (lead decision "RKDB"): a connected machine whose
    /// newest list expired (24 hours) or that is below its floor says "could not update the list"
    /// and holds installs of new apps; apps already installed keep running. Offline, the stale
    /// rules apply instead.
    pub fn gate_new_install(&self, now: u64, connected: bool) -> Result<(), &'static str> {
        match woltrust::rkdb::gate(
            self.store.newest_expires(),
            self.store
                .db_serial()
                .or(self.store.best_list().map(|l| l.list.serial)),
            self.store.floor,
            now,
            connected,
        ) {
            woltrust::rkdb::Gate::Open => Ok(()),
            woltrust::rkdb::Gate::HoldNew => Err(woltrust::rkdb::HOLD_SENTENCE),
        }
    }

    /// True when the update service reached the list's server within [`CONNECTED_WINDOW`] (its
    /// `last-attempt` stamp): a machine that is online but still holds a stale or behind list.
    pub fn connected(&self, now: u64) -> bool {
        self.store.last_attempt.is_some_and(|t| {
            t <= now.saturating_add(woltrust::verify::CLOCK_SKEW)
                && now.saturating_sub(t) <= CONNECTED_WINDOW
        })
    }

    /// [`TrustStore::gate_new_install`] with [`TrustStore::connected`]: what wol-admind asks
    /// before it installs an app that is not installed yet.
    pub fn gate_new_install_now(&self, now: u64) -> Result<(), &'static str> {
        self.gate_new_install(now, self.connected(now))
    }

    /// The store's state at `now`.
    pub fn status(&self, now: u64) -> Status {
        let best = self.store.best_list();
        let freshness = match self.store.freshness(now) {
            woltrust::revoked::Freshness::Fresh => "fresh",
            woltrust::revoked::Freshness::Stale => "stale",
            woltrust::revoked::Freshness::TooOld => "too-old",
            woltrust::revoked::Freshness::Missing => "missing",
        };
        Status {
            roots: self.store.roots.len(),
            intermediates: self.store.intermediates.len(),
            list_serial: best.map(|l| l.list.serial),
            list_issued: best.map(|l| l.list.issued),
            list_expires: best.map(|l| l.list.expires),
            list_age: best.map(|l| now.saturating_sub(l.list.issued)),
            freshness,
            revoked: best.map_or(0, |l| l.list.entries.len()),
            notes: self.store.notes.clone(),
            root_texts: self
                .store
                .roots
                .iter()
                .filter_map(|r| r.body.subject.text.clone())
                .collect(),
            remembered: self
                .store
                .remembered
                .iter()
                .map(|l| l.list.entries.len())
                .sum(),
            rkdb_serial: self.store.db_serial(),
            rkdb_entries: self.store.best_db().map_or(0, |d| d.db.entries.len()),
            floor: self.store.floor,
            behind: self.store.behind(),
            last_attempt: self.store.last_attempt,
            last_ok: self.store.last_ok,
        }
    }
}

fn legacy_outcome(trust: woltrust::Trust, code: woltrust::Code) -> Outcome {
    Outcome {
        trust,
        code,
        cert: None,
        stale: false,
        list_expires: None,
        origin: None,
        root_text: None,
        line: None,
    }
}

/// The host clock in whole seconds (0 before 1970).
fn unix(host: &dyn Host) -> u64 {
    host.now().max(0.0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;
    use shared::package::{ICON, MANIFEST, PAYLOAD, SIGNATURE};
    use woltrust::make::wsig;
    use woltrust::pki::{DAY, NOW, Pki};
    use woltrust::{Code, Trust};

    fn info(id: &str) -> String {
        format!(
            r#"{{"id":"{id}","version":"1.0","name":"Fixture","package":"fixture","targets":[{{"abi":"freebsd-native","arch":"amd64","entry":"bin/f"}}]}}"#
        )
    }

    struct Wol {
        path: std::path::PathBuf,
    }

    /// Writes a `.wol` for `id`; `sign` gets the `SHA256` bytes and returns the extra members.
    fn wol(id: &str, tag: &str, sign: impl FnOnce(&[u8]) -> Vec<(&'static str, Vec<u8>)>) -> Wol {
        let body: Vec<(&str, Vec<u8>)> = vec![
            ("Info.json", info(id).into_bytes()),
            (ICON, b"\x89PNG".to_vec()),
            (PAYLOAD, b"payload".to_vec()),
        ];
        let refs: Vec<(&str, &[u8])> = body.iter().map(|(n, b)| (*n, b.as_slice())).collect();
        let man = package::manifest(&refs).into_bytes();
        let mut all = body.clone();
        all.push((MANIFEST, man.clone()));
        all.extend(sign(&man));
        let refs: Vec<(&str, &[u8])> = all.iter().map(|(n, b)| (*n, b.as_slice())).collect();
        let dir = std::env::temp_dir().join(format!("wol-trust-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.wol");
        std::fs::write(&path, package::ustar(&refs)).unwrap();
        Wol { path }
    }

    fn store(p: &Pki, list_age_days: u64) -> TrustStore {
        let l = woltrust::revoked::ListText {
            format: 1,
            serial: 5,
            issued: NOW - list_age_days * DAY,
            expires: NOW - list_age_days * DAY + woltrust::LIST_LIFETIME,
            entries: vec![],
        };
        let lists = [woltrust::make::revocation_list(
            &l,
            &p.index.body.serial,
            &p.index.secret,
        )];
        TrustStore {
            store: Store::load(&[&p.root_a.file], &[&p.index.file], &lists, NOW, true),
        }
    }

    #[test]
    fn a_revocation_database_is_kept_only_as_a_newer_superset_above_the_floor() {
        use woltrust::revoked::Reason;
        use woltrust::rkdb::{Checkpoint, Db};
        use woltrust::vectors_ids::{Ids, db, entry};
        let p = Pki::new();
        let ids = Ids::new(&p);
        let a = entry(&ids.bob.body.serial, NOW - DAY, Reason::KeyCompromise);
        let b = entry(&p.alice.body.serial, NOW - DAY, Reason::Superseded);
        let db1 = db(&ids, 1, NOW - 3600, &[a], None, [0; 32]);
        let none: [(&[u8], &[u8]); 0] = [];
        let mut s = Store::load(
            &[&p.root_a.file, &p.root_b.file],
            &[&ids.index.file],
            &none,
            NOW,
            true,
        );
        s.load_rkdb(&db1, NOW).unwrap();
        s.floor = 1;
        let ts = TrustStore { store: s };
        let h1 = Db::parse(&db1).unwrap().hash;
        let db2 = db(&ids, 2, NOW - 60, &[a, b], None, h1);
        let ok = ts.accept_rkdb(&db2, None, None, NOW).unwrap();
        assert_eq!((ok.serial, ok.floor), (2, 2));
        assert!(ok.warning.is_some(), "no checkpoint answered");
        let cp = Checkpoint::of(&Db::parse(&db2).unwrap()).to_json();
        let agreed = ts
            .accept_rkdb(&db2, Some(cp.as_bytes()), Some(cp.as_bytes()), NOW)
            .unwrap();
        assert_eq!(agreed.warning, None);
        let shrunk = db(&ids, 2, NOW - 60, &[b], None, h1);
        assert!(
            ts.accept_rkdb(&shrunk, None, None, NOW)
                .unwrap_err()
                .contains("leaves out")
        );
        let unchained = db(&ids, 2, NOW - 60, &[a, b], None, [1; 32]);
        assert!(
            ts.accept_rkdb(&unchained, None, None, NOW)
                .unwrap_err()
                .contains("follow")
        );
        let other = Checkpoint {
            serial: 2,
            hash: [3; 32],
        }
        .to_json();
        assert!(
            ts.accept_rkdb(&db2, Some(other.as_bytes()), Some(other.as_bytes()), NOW)
                .unwrap_err()
                .contains("checkpoints")
        );
        // Audit 2 M2: one unsigned checkpoint (or two that disagree) never raises the floor.
        let high = Checkpoint {
            serial: 9,
            hash: [3; 32],
        }
        .to_json();
        let one = ts
            .accept_rkdb(&db2, Some(high.as_bytes()), None, NOW)
            .unwrap();
        assert_eq!(
            one.floor, 2,
            "the floor kept is the verified database's serial"
        );
        let low = Checkpoint {
            serial: 2,
            hash: [3; 32],
        }
        .to_json();
        assert!(
            ts.accept_rkdb(&db2, Some(high.as_bytes()), Some(low.as_bytes()), NOW)
                .is_ok()
        );
        assert!(
            ts.accept_rkdb(&db2, Some(high.as_bytes()), Some(high.as_bytes()), NOW)
                .unwrap_err()
                .contains("older"),
            "two that agree on a higher serial do refuse it"
        );
        // The install gate: fresh and at the floor, open; behind the floor, new installs wait.
        assert_eq!(ts.gate_new_install(NOW, true), Ok(()));
        let mut behind = ts.clone();
        behind.observe_rkdb_serial(5);
        assert_eq!(
            behind.gate_new_install(NOW, true),
            Err(woltrust::rkdb::HOLD_SENTENCE)
        );
        assert_eq!(
            behind.gate_new_install(NOW, false),
            Ok(()),
            "offline: the stale rules apply"
        );
        assert!(behind.status(NOW).behind);
        assert_eq!(parse_floor(&floor_text(42)), 42);
        // Connected: the update service reached the server within two hours.
        let mut online = behind.clone();
        assert!(!online.connected(NOW));
        online.store.last_attempt = Some(NOW - 600);
        assert!(online.connected(NOW));
        assert_eq!(
            online.gate_new_install_now(NOW),
            Err(woltrust::rkdb::HOLD_SENTENCE)
        );
        online.store.last_attempt = Some(NOW - 3 * 3600);
        assert_eq!(
            online.gate_new_install_now(NOW),
            Ok(()),
            "offline: stale rules instead"
        );
        assert_eq!(floor_after(b"", 7), Some(b"7\n".to_vec()));
        assert_eq!(floor_after(b"7\n", 3), None, "never lowered");
        assert_eq!(floor_after(b"7\n", 7), None);
        assert_eq!(floor_after(b"7\n", 9), Some(b"9\n".to_vec()));
        assert_eq!(parse_floor(b"junk"), 0);
    }

    /// Audit 2 H1: a database by the derived Index carrying root B's section that withdraws
    /// that Index is refused, but the section is salvaged and kept in the memory; a bare
    /// section is accepted alone; one dated in the future is refused (L2).
    #[test]
    fn the_emergency_section_survives_a_database_its_index_cannot_sign() {
        use woltrust::revoked::Reason;
        use woltrust::rkdb::RootSection;
        use woltrust::vectors_ids::{Ids, db, entry};
        let p = Pki::new();
        let ids = Ids::new(&p);
        let none: [(&[u8], &[u8]); 0] = [];
        let s = Store::load(
            &[&p.root_a.file, &p.root_b.file],
            &[&ids.index.file],
            &none,
            NOW,
            true,
        );
        let ts = TrustStore { store: s };
        let at = u32::try_from(NOW - 600).unwrap();
        let section = RootSection::make(
            at,
            &[entry(
                &ids.index.body.serial,
                NOW - 600,
                Reason::KeyCompromise,
            )],
            &p.root_b.body.serial,
            &p.root_b.secret,
        )
        .unwrap();
        let carried = db(&ids, 3, NOW - 600, &[], Some(&section), [0; 32]);
        assert!(
            ts.accept_rkdb(&carried, None, None, NOW).is_err(),
            "the Index is withdrawn"
        );
        let kept = ts
            .salvage_root_section(&carried, NOW)
            .expect("the section is kept");
        let mem = ts.memory_after_root(b"", &kept).unwrap();
        let mut again = ts.clone();
        again
            .store
            .remember(&woltrust::revoked::Seen::parse(&mem).unwrap());
        assert!(woltrust::verify::withdrawn(&again.store, &ids.index.body.serial).is_some());
        assert!(ts.accept_root_section(&section, NOW).is_ok());
        let future = RootSection::make(
            u32::try_from(NOW + 3 * 86_400).unwrap(),
            &[entry(&ids.index.body.serial, NOW, Reason::KeyCompromise)],
            &p.root_b.body.serial,
            &p.root_b.secret,
        )
        .unwrap();
        assert!(
            ts.accept_root_section(&future, NOW)
                .unwrap_err()
                .contains("future")
        );
        let mut forged = section.clone();
        let last = forged.len() - 1;
        forged[last] ^= 1;
        assert!(
            ts.accept_root_section(&forged, NOW).is_err(),
            "the root's signature is checked"
        );
    }

    /// Audit 2 H1c: published intermediates are kept only with both roots' signatures.
    #[test]
    fn published_intermediates_need_both_pinned_roots() {
        use woltrust::vectors_ids::Ids;
        let p = Pki::new();
        let ids = Ids::new(&p);
        let none: [(&[u8], &[u8]); 0] = [];
        let both = TrustStore {
            store: Store::load(
                &[&p.root_a.file, &p.root_b.file],
                &[] as &[&[u8]],
                &none,
                NOW,
                true,
            ),
        };
        let mut lone = woltrust::Cert::parse(&ids.authors.file).unwrap();
        lone.cosigs.clear();
        let file = |b: &[u8]| serde_json::from_slice::<serde_json::Value>(b).unwrap();
        let list = serde_json::to_vec(&serde_json::json!([
            file(&ids.authors.file),
            file(&ids.index.file),
            file(&lone.to_bytes()),
            file(&p.authors.file),
            file(&ids.bob.file)
        ]))
        .unwrap();
        let (ok, bad) = both.accept_intermediates(&list, NOW);
        let kept: Vec<&str> = ok.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(
            kept,
            [
                ids.authors.body.serial.as_str(),
                ids.index.body.serial.as_str()
            ]
        );
        assert_eq!(bad.len(), 3, "{bad:?}");
        let only_a = TrustStore {
            store: Store::load(&[&p.root_a.file], &[] as &[&[u8]], &none, NOW, true),
        };
        assert!(only_a.accept_intermediates(&list, NOW).0.is_empty());
        assert!(!both.accept_intermediates(b"not json", NOW).1.is_empty());
    }

    /// Audit 2 M6: the previous flat key vouches until LEGACY_UNTIL and never after.
    #[test]
    fn the_previous_key_stops_vouching_on_its_end_date() {
        let ts = TrustStore::default();
        let key = package::public_of(&[4; 32]);
        assert_eq!(ts.legacy_at(&key, LEGACY_UNTIL - 1).trust, Trust::Legacy);
        let after = ts.legacy_at(&key, LEGACY_UNTIL);
        assert_eq!(
            (after.trust, after.code),
            (Trust::Invalid, Code::LegacyRefused)
        );
        assert_eq!(woltrust::line::date(LEGACY_UNTIL), "2027-02-01");
    }

    /// Audit 2 M3: the flat key withdrawn under its id by rule IDS is refused too.
    #[test]
    fn a_flat_key_withdrawn_by_its_full_id_is_withdrawn() {
        let p = Pki::new();
        let key = [7u8; 32];
        let l = woltrust::revoked::ListText {
            format: 1,
            serial: 5,
            issued: NOW - DAY,
            expires: NOW - DAY + woltrust::LIST_LIFETIME,
            entries: vec![woltrust::revoked::Entry {
                serial: woltrust::request::legacy_key_id(&key),
                at: NOW - DAY,
                reason: woltrust::Reason::KeyCompromise,
            }],
        };
        let lists = [woltrust::make::revocation_list(
            &l,
            &p.index.body.serial,
            &p.index.secret,
        )];
        let ts = TrustStore {
            store: Store::load(&[&p.root_a.file], &[&p.index.file], &lists, NOW, true),
        };
        assert_eq!(ts.legacy(&key).trust, Trust::Withdrawn);
    }

    /// Audit 2 M4: a state database below the image's (a rolled-back file) is left out.
    #[test]
    fn a_rolled_back_state_database_is_left_out() {
        use woltrust::revoked::Reason;
        use woltrust::rkdb::Db;
        use woltrust::vectors_ids::{Ids, db, entry};
        let p = Pki::new();
        let ids = Ids::new(&p);
        let a = entry(&ids.bob.body.serial, NOW - DAY, Reason::KeyCompromise);
        let old = db(&ids, 1, NOW - 7200, &[], None, [0; 32]);
        let image = db(
            &ids,
            2,
            NOW - 3600,
            &[a],
            None,
            Db::parse(&old).unwrap().hash,
        );
        let none: [(&[u8], &[u8]); 0] = [];
        let mut s = Store::load(
            &[&p.root_a.file, &p.root_b.file],
            &[&ids.index.file],
            &none,
            NOW,
            true,
        );
        s.load_rkdb(&image, NOW).unwrap();
        s.floor = 2;
        assert!(state_db_counts(&old, &s).is_err());
        let next = db(
            &ids,
            3,
            NOW - 60,
            &[a],
            None,
            Db::parse(&image).unwrap().hash,
        );
        assert_eq!(state_db_counts(&next, &s), Ok(()));
        let dropped = db(
            &ids,
            3,
            NOW - 60,
            &[],
            None,
            Db::parse(&image).unwrap().hash,
        );
        assert!(
            state_db_counts(&dropped, &s).is_err(),
            "it lost an entry the image has"
        );
    }

    fn host() -> FakeHost {
        let h = FakeHost::linux();
        h.advance(NOW as f64 - 1_000_000.0);
        h
    }

    fn author(p: &Pki, id: &str, tag: &str) -> Wol {
        wol(id, tag, |man| {
            vec![
                (package::CHAIN, Pki::chain(&[&p.alice, &p.authors])),
                (
                    package::WSIG,
                    wsig(&p.alice.body.serial, &p.alice.secret, Purpose::Package, man),
                ),
            ]
        })
    }

    #[test]
    fn author_wolos_and_unsigned_files_get_their_values() {
        let p = Pki::new();
        let (ts, h) = (store(&p, 1), host());
        let mine = author(&p, "alice.notes", "a");
        let c = ts.verify_wol(&h, &mine.path, &[], None).unwrap();
        assert_eq!((c.outcome.trust, c.outcome.code), (Trust::Origin, Code::Ok));
        assert_eq!(
            c.outcome.cert.unwrap().subject.login.as_deref(),
            Some("alice")
        );
        let theirs = wol("org.wolos.notes", "b", |man| {
            vec![
                (package::CHAIN, Pki::chain(&[&p.signer, &p.packages])),
                (
                    package::WSIG,
                    wsig(
                        &p.signer.body.serial,
                        &p.signer.secret,
                        Purpose::Package,
                        man,
                    ),
                ),
            ]
        });
        let c = ts.verify_wol(&h, &theirs.path, &[], None).unwrap();
        assert_eq!(c.outcome.trust, Trust::Wolos);
        let none = wol("alice.notes", "c", |_| vec![]);
        let c = ts.verify_wol(&h, &none.path, &[], None).unwrap();
        assert_eq!(
            (c.outcome.trust, c.outcome.code),
            (Trust::Community, Code::Unsigned)
        );
    }

    /// Audit H1: a raw Ed25519 `SHA256.sig` by the previous flat key is "legacy", never WolOS;
    /// once the machine holds roots it is refused; its key id can be withdrawn in revoked.json.
    #[test]
    fn a_legacy_signature_is_legacy_refused_with_roots_and_revocable() {
        let seed = [3u8; 32];
        let key = package::public_of(&seed);
        let f = wol("org.wolos.notes", "d", |man| {
            vec![(SIGNATURE, package::sign(&seed, man).to_vec())]
        });
        let c = TrustStore::default()
            .verify_wol(&host(), &f.path, &[key], None)
            .unwrap();
        assert_eq!(
            (c.outcome.trust, c.outcome.code),
            (Trust::Legacy, Code::LegacyKey)
        );
        let p = Pki::new();
        let with_roots = store(&p, 1);
        let c = with_roots
            .verify_wol(&host(), &f.path, &[key], None)
            .unwrap();
        assert_eq!(
            (c.outcome.trust, c.outcome.code),
            (Trust::Invalid, Code::LegacyRefused)
        );
        // Withdrawn by its key id in a list the Index signed, held in memory only.
        let serial = woltrust::request::key_id(&key);
        let mut seen = woltrust::revoked::Seen::default();
        seen.add(&woltrust::RevList {
            list: woltrust::revoked::ListText {
                format: 1,
                serial: 3,
                issued: NOW - DAY,
                expires: NOW + DAY,
                entries: vec![woltrust::revoked::Entry {
                    serial,
                    at: NOW - DAY,
                    reason: woltrust::Reason::KeyCompromise,
                }],
            },
            signer: p.index.body.serial.clone(),
            signer_is_root: false,
        });
        let mut ts = TrustStore {
            store: Store::load(
                &[] as &[&[u8]],
                &[&p.index.file],
                &[(&b""[..], &b""[..])],
                NOW,
                true,
            ),
        };
        ts.store.remember(&seen);
        assert_eq!(ts.legacy(&key).trust, Trust::Withdrawn);
        assert_eq!(
            ts.legacy(&package::public_of(&[4; 32])).trust,
            Trust::Legacy
        );
    }

    fn hard_list(
        p: &Pki,
        serial: u64,
        entries: Vec<woltrust::revoked::Entry>,
    ) -> (Vec<u8>, Vec<u8>) {
        let l = woltrust::revoked::ListText {
            format: 1,
            serial,
            issued: NOW - DAY,
            expires: NOW - DAY + woltrust::LIST_LIFETIME,
            entries,
        };
        woltrust::make::revocation_list(&l, &p.index.body.serial, &p.index.secret)
    }

    /// Audit M1 (A4/A5): the memory keeps a key-compromise after the list that carried it is
    /// gone; a list that leaves it out is refused; the memory only grows.
    #[test]
    fn a_seen_withdrawal_is_never_forgotten() {
        let p = Pki::new();
        let entry = woltrust::revoked::Entry {
            serial: p.alice.body.serial.clone(),
            at: NOW - 2 * DAY,
            reason: woltrust::Reason::KeyCompromise,
        };
        let (l2, s2) = hard_list(&p, 2, vec![entry.clone()]);
        let holding = TrustStore {
            store: Store::load(
                &[&p.root_a.file],
                &[&p.authors.file, &p.index.file],
                &[(&l2, &s2)],
                NOW,
                true,
            ),
        };
        let mem = holding.memory(b"", &l2, &s2, NOW, true).unwrap();
        // A higher list without the entry: refused while the machine holds the old one.
        let (l3, s3) = hard_list(&p, 3, vec![]);
        let err = holding.accept(&l3, &s3, NOW, true).unwrap_err();
        assert!(err.contains("leaves out"), "{err}");
        // The state file rolled back to an empty list: the memory still withdraws Alice.
        let mut rolled = TrustStore {
            store: Store::load(
                &[&p.root_a.file],
                &[&p.authors.file, &p.index.file],
                &[(&l3, &s3)],
                NOW,
                true,
            ),
        };
        rolled
            .store
            .remember(&woltrust::revoked::Seen::parse(&mem).unwrap());
        let f = author(&p, "alice.notes", "m1");
        let c = rolled.verify_wol(&host(), &f.path, &[], None).unwrap();
        assert_eq!(
            (c.outcome.trust, c.outcome.code),
            (Trust::Withdrawn, Code::RevokedKeyCompromise)
        );
        // Appending: the old bytes are a prefix of the new memory's entries.
        let again = rolled.memory(&mem, &l3, &s3, NOW, true).unwrap();
        let (a, b) = (
            woltrust::revoked::Seen::parse(&mem).unwrap(),
            woltrust::revoked::Seen::parse(&again).unwrap(),
        );
        assert!(b.entries.starts_with(&a.entries) && !a.entries.is_empty());
        assert!(rolled.memory(b"{damaged", &l3, &s3, NOW, true).is_err());
    }

    #[test]
    fn load_reads_the_memory_file() {
        let p = Pki::new();
        let h = host();
        let mut seen = woltrust::revoked::Seen::default();
        seen.add(&woltrust::RevList {
            list: woltrust::revoked::ListText {
                format: 1,
                serial: 1,
                issued: NOW,
                expires: NOW + 1,
                entries: vec![woltrust::revoked::Entry {
                    serial: p.alice.body.serial.clone(),
                    at: NOW,
                    reason: woltrust::Reason::Abuse,
                }],
            },
            signer: p.index.body.serial.clone(),
            signer_is_root: false,
        });
        put(
            &h,
            "/usr/share/wolos/trust/intermediates/index.wolcert",
            &p.index.file,
        );
        put(&h, SEEN_FILE, &seen.to_bytes());
        let ts = TrustStore::load(&h);
        assert_eq!(ts.status(NOW).remembered, 1);
        h.file(SEEN_FILE, "{not json");
        assert!(
            TrustStore::load(&h)
                .store
                .notes
                .iter()
                .any(|n| n.contains("seen-revocations"))
        );
    }

    #[test]
    fn a_changed_member_or_a_wrong_name_is_invalid_not_unsigned() {
        let p = Pki::new();
        let (ts, h) = (store(&p, 1), host());
        let other = author(&p, "bob.notes", "e");
        let c = ts.verify_wol(&h, &other.path, &[], None).unwrap();
        assert_eq!(
            (c.outcome.trust, c.outcome.code),
            (Trust::Invalid, Code::WrongNamespace)
        );
        let swapped = wol("alice.notes", "f", |man| {
            let mut other = man.to_vec();
            other.push(b'\n');
            vec![(
                package::WSIG,
                wsig(
                    &p.alice.body.serial,
                    &p.alice.secret,
                    Purpose::Package,
                    &other,
                ),
            )]
        });
        let store = TrustStore {
            store: Store::load(
                &[&p.root_a.file],
                &[&p.authors.file, &p.index.file],
                &[(&b""[..], &b""[..])],
                NOW,
                true,
            ),
        };
        let c = store.verify_wol(&h, &swapped.path, &[], None).unwrap();
        assert_eq!(c.outcome.trust, Trust::Invalid);
    }

    #[test]
    fn the_host_clock_decides_freshness() {
        let p = Pki::new();
        let ts = store(&p, 1);
        let f = author(&p, "alice.notes", "g");
        let h = host();
        assert_eq!(
            ts.verify_wol(&h, &f.path, &[], None).unwrap().outcome.code,
            Code::Ok
        );
        h.advance(8.0 * DAY as f64);
        let c = ts.verify_wol(&h, &f.path, &[], None).unwrap();
        assert_eq!(
            (c.outcome.trust, c.outcome.code, c.outcome.stale),
            (Trust::Origin, Code::ListStale, true)
        );
        // Audit M3: past the stale window the signature keeps its value, flagged.
        h.advance(40.0 * DAY as f64);
        let c = ts.verify_wol(&h, &f.path, &[], None).unwrap();
        assert_eq!(
            (c.outcome.trust, c.outcome.code, c.outcome.stale),
            (Trust::Origin, Code::ListTooOld, true)
        );
    }

    fn put(h: &FakeHost, path: &str, bytes: &[u8]) {
        h.file(path, std::str::from_utf8(bytes).unwrap());
    }

    #[test]
    fn load_reads_the_store_paths_and_never_a_test_root() {
        let p = Pki::new();
        let h = host();
        put(&h, "/usr/share/wolos/trust/roots/a.wolcert", &p.root_a.file);
        put(
            &h,
            "/usr/share/wolos/trust/intermediates/index.wolcert",
            &p.index.file,
        );
        put(
            &h,
            "/var/lib/wolos/trust/intermediates/authors.wolcert",
            &p.authors.file,
        );
        let ts = TrustStore::load(&h);
        assert!(ts.store.roots.is_empty(), "the TEST root must not load");
        assert!(
            ts.store.notes.iter().any(|n| n.contains("test root")),
            "{:?}",
            ts.store.notes
        );
        assert_eq!(ts.store.intermediates.len(), 2);
        // The same store with test roots allowed, as a vector run does.
        let allowed = Store::load(
            &[&p.root_a.file],
            &[&p.index.file],
            &[(&b""[..], &b""[..])],
            NOW,
            true,
        );
        assert_eq!(allowed.roots.len(), 1);
    }

    #[test]
    fn load_drops_unverifiable_lists_and_oversize_files() {
        let p = Pki::new();
        let h = host();
        let l = woltrust::revoked::ListText {
            format: 1,
            serial: 9,
            issued: NOW - DAY,
            expires: NOW - DAY + woltrust::LIST_LIFETIME,
            entries: vec![],
        };
        // A root that signs the list is needed, so allow-test cannot apply to `load`: the list
        // is dropped, with a note, and so is the oversize certificate.
        let (list, sig) =
            woltrust::make::revocation_list(&l, &p.root_a.body.serial, &p.root_a.secret);
        put(&h, "/usr/share/wolos/trust/revoked.json", &list);
        put(&h, "/usr/share/wolos/trust/revoked.json.wsig", &sig);
        h.file(
            "/usr/share/wolos/trust/intermediates/huge.wolcert",
            &"x".repeat(woltrust::MAX_CERT_FILE + 1),
        );
        let ts = TrustStore::load(&h);
        assert!(ts.store.lists.is_empty() && ts.store.intermediates.is_empty());
    }

    fn plug(p: &Pki, id: &str, signer: &woltrust::make::Secret, serial: &str) -> Vec<u8> {
        let files: Vec<(String, Vec<u8>)> =
            crate::plugins::scaffold(id, "Demo", &[shared::plugins::Kind::Theme])
                .unwrap()
                .into_iter()
                .map(|(rel, text)| (format!("{id}/{rel}"), text.into_bytes()))
                .collect();
        let mut refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let sha = package::manifest(&refs);
        let chain = Pki::chain(&[&p.alice, &p.authors]);
        let w = wsig(serial, signer, Purpose::Plugin, sha.as_bytes());
        refs.push(("SHA256", sha.as_bytes()));
        refs.push((package::CHAIN, &chain));
        refs.push((package::WSIG, &w));
        package::ustar(&refs)
    }

    #[test]
    fn a_wolplug_with_chain_and_wsig_is_judged_by_its_certificate() {
        let p = Pki::new();
        let ts = store(&p, 1);
        let good = plug(&p, "alice.demo", &p.alice.secret, &p.alice.body.serial);
        let c = ts.verify_wolplug(&good, &[], NOW).unwrap();
        assert_eq!((c.outcome.trust, c.outcome.code), (Trust::Origin, Code::Ok));
        assert_eq!(c.verified.manifest.id, "alice.demo");
        let other = plug(&p, "bob.demo", &p.alice.secret, &p.alice.body.serial);
        assert!(
            ts.verify_wolplug(&other, &[], NOW).is_err(),
            "outside alice."
        );
        let forged = plug(&p, "alice.demo", &p.org.secret, &p.alice.body.serial);
        assert!(ts.verify_wolplug(&forged, &[], NOW).is_err(), "wrong key");
        // The legacy reader takes the same file and ignores the pair.
        assert!(package::verify_wolplug(&good, &[]).is_ok());
    }

    #[test]
    fn status_reports_the_list_age_and_freshness() {
        let p = Pki::new();
        let s = store(&p, 2).status(NOW);
        assert_eq!(
            (s.roots, s.freshness, s.list_age),
            (1, "fresh", Some(2 * DAY))
        );
        assert_eq!(TrustStore::default().status(NOW).freshness, "missing");
    }

    #[test]
    fn a_new_list_replaces_only_when_it_verifies_and_is_newer() {
        let p = Pki::new();
        let mk = |serial: u64| {
            let l = woltrust::revoked::ListText {
                format: 1,
                serial,
                issued: NOW - DAY,
                expires: NOW - DAY + woltrust::LIST_LIFETIME,
                entries: vec![],
            };
            woltrust::make::revocation_list(&l, &p.index.body.serial, &p.index.secret)
        };
        // A store as the system holds it, but with the TEST root pinned for the test.
        let ts = store(&p, 1);
        let held = ts.store.best_list().map(|l| l.list.serial).unwrap();
        let (newer, sig) = mk(held + 1);
        assert_eq!(ts.accept(&newer, &sig, NOW, true), Ok(held + 1));
        // The system path never allows a test root, so the TEST authority cannot install one.
        assert!(ts.accept_list(&newer, &sig, NOW).is_err());
        let (older, osig) = mk(held);
        assert!(ts.accept(&older, &osig, NOW, true).is_err(), "not newer");
        let mut bad = sig.clone();
        bad[20] ^= 1;
        assert!(ts.accept(&newer, &bad, NOW, true).is_err(), "bad signature");
    }

    /// The list comes from the CDN under the project's domain first, then its mirror; so does the
    /// database; the update-check script leaves the fetching to `wol trust update`, every hour,
    /// before its once-a-day gate (lists live 24 hours).
    #[test]
    fn the_revoked_list_is_read_from_the_cdn_then_its_mirror() {
        assert_eq!(
            revoked_urls(),
            [
                "https://cdn.wolos.org/plugins/revoked.json",
                "https://wolos.squareweb.app/plugins/revoked.json"
            ]
        );
        let script = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../files/usr/lib/wolos/update-check"),
        )
        .unwrap();
        assert_eq!(
            rkdb_urls(),
            [
                "https://cdn.wolos.org/plugins/revoked.rkdb",
                "https://wolos.squareweb.app/plugins/revoked.rkdb"
            ]
        );
        let update = script
            .find("wol trust update")
            .expect("update-check calls it");
        let gate = script.find("-ge $EVERY").expect("the daily gate");
        assert!(update < gate, "the list is refreshed on every hourly run");
        assert!(
            !script.lines().any(|l| l.contains("curl")
                && (l.contains("revoked.json") || l.contains("revoked.rkdb"))),
            "wol trust update fetches the files itself"
        );
    }

    #[test]
    fn the_image_trust_folder_holds_no_test_root() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../files/usr/share/wolos/trust");
        for sub in ["roots", "intermediates"] {
            for e in std::fs::read_dir(dir.join(sub))
                .into_iter()
                .flatten()
                .flatten()
            {
                let bytes = std::fs::read(e.path()).unwrap();
                let c = woltrust::Cert::parse(&bytes).unwrap();
                let label = c.body.subject.label.unwrap_or_default();
                assert!(
                    !label.starts_with(woltrust::TEST_LABEL),
                    "{} is a test certificate",
                    e.path().display()
                );
            }
        }
    }
}
