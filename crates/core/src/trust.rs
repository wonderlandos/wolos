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
        let mut store = Store::load(&roots, &inter, &lists, unix(host), false);
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
        let serial = woltrust::request::key_id(key);
        if let Some(code) = woltrust::verify::withdrawn(&self.store, &serial) {
            return legacy_outcome(woltrust::Trust::Withdrawn, code);
        }
        if self.store.roots.is_empty() {
            legacy_outcome(woltrust::Trust::Legacy, woltrust::Code::LegacyKey)
        } else {
            legacy_outcome(woltrust::Trust::Invalid, woltrust::Code::LegacyRefused)
        }
    }

    fn legacy_of(&self, trust: package::Trust, keys: &[[u8; 32]]) -> Outcome {
        match trust {
            package::Trust::Signed(i) => keys.get(i).map_or_else(
                || legacy_outcome(woltrust::Trust::Invalid, woltrust::Code::SignerUnknown),
                |k| self.legacy(k),
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
            self.legacy_of(verified.trust, legacy_keys)
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
                    self.legacy_of(verified.trust, legacy_keys)
                }
                _ => o,
            }
        } else {
            self.legacy_of(verified.trust, legacy_keys)
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

    /// The list comes from the CDN under the project's domain first, then its mirror; the
    /// update-check script's defaults are the same two URLs (it cannot import this).
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
        let urls = revoked_urls();
        for (var, url) in [("REVOKED_URL:-", &urls[0]), ("REVOKED_MIRROR-", &urls[1])] {
            assert!(
                script.contains(&format!("{var}{url}}}")),
                "update-check {var} is {url}"
            );
        }
        assert!(script.contains(r#"revoked "$REVOKED_URL" || revoked "$REVOKED_MIRROR""#));
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
