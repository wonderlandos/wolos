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
/// Extension of a certificate file.
pub const CERT_EXT: &str = ".wolcert";
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
        TrustStore {
            store: Store::load(&roots, &inter, &lists, unix(host), false),
        }
    }

    /// Judges a `.wol`: the structural checks of [`package::verify_extra`], then the hybrid
    /// signature if the file has one, else the legacy Ed25519 signature against `legacy_keys`
    /// (WolOS's own flat keys, which count as made by WolOS), else unsigned. `first_seen` is the
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
        } else if matches!(verified.trust, package::Trust::Signed(_)) {
            legacy_outcome(woltrust::Trust::Wolos, woltrust::Code::Ok)
        } else {
            legacy_outcome(woltrust::Trust::Community, woltrust::Code::Unsigned)
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
            if matches!(
                o.trust,
                woltrust::Trust::Invalid | woltrust::Trust::Withdrawn
            ) {
                return Err(o
                    .code
                    .sentence()
                    .unwrap_or("The plugin's signature does not check out.")
                    .to_owned());
            }
            o
        } else if matches!(verified.trust, package::Trust::Signed(_)) {
            legacy_outcome(woltrust::Trust::Wolos, woltrust::Code::Ok)
        } else {
            legacy_outcome(woltrust::Trust::Community, woltrust::Code::Unsigned)
        };
        Ok(CheckedPlugin { verified, outcome })
    }

    /// Checks a freshly downloaded revocation list before it may replace [`STATE_REVOKED`]: its
    /// signature must verify against this store (the Index intermediate or a root, test roots
    /// never), and its serial must be higher than every list held. Returns the new serial.
    pub fn accept_list(&self, list: &[u8], sig: &[u8], now: u64) -> Result<u64, String> {
        self.accept(list, sig, now, false)
    }

    fn accept(&self, list: &[u8], sig: &[u8], now: u64, allow_test: bool) -> Result<u64, String> {
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
        let probe = Store::load(&roots, &inter, &[(list, sig)], now, allow_test);
        let new = probe
            .lists
            .first()
            .ok_or_else(|| {
                format!(
                    "the new revocation list does not verify ({})",
                    probe.notes.join("; ")
                )
            })?
            .list
            .serial;
        match self.store.best_list() {
            Some(old) if old.list.serial >= new => Err(format!(
                "the new revocation list ({new}) is not newer than the one held ({})",
                old.list.serial
            )),
            _ => Ok(new),
        }
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
        assert_eq!(
            (c.outcome.trust, c.outcome.code),
            (Trust::Verified, Code::Ok)
        );
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

    #[test]
    fn a_legacy_wolos_signature_still_counts_as_wolos() {
        let seed = [3u8; 32];
        let f = wol("org.wolos.notes", "d", |man| {
            vec![(SIGNATURE, package::sign(&seed, man).to_vec())]
        });
        let c = TrustStore::default()
            .verify_wol(&host(), &f.path, &[package::public_of(&seed)], None)
            .unwrap();
        assert_eq!(c.outcome.trust, Trust::Wolos);
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
            (Trust::Verified, Code::ListStale, true)
        );
        h.advance(40.0 * DAY as f64);
        let c = ts.verify_wol(&h, &f.path, &[], None).unwrap();
        assert_eq!(
            (c.outcome.trust, c.outcome.code),
            (Trust::Community, Code::ListTooOld)
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
        assert_eq!(
            (c.outcome.trust, c.outcome.code),
            (Trust::Verified, Code::Ok)
        );
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
