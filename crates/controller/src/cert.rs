// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol cert request|add|show|list`, `wol sign`, `wol encrypt FILE --to SERIAL`, `wol decrypt` and
//! `wol trust status` (ADR 0046): an author's keys and certificates under the account's data
//! home, locked with a passphrase (`wol_core::authorkeys`, `woltrust::wolkey`), files sealed with
//! `wolos/seal/1`. Passphrases are read as one line from stdin, never from an argument, and not
//! echoed on a terminal (`crate::secret`); a key is never written in the clear.
use crate::cli::Invocation;
use crate::run::{Env, emit_json, fail};
use shared::errors::ErrorCode;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use wol_core::authorkeys as ak;
use wol_core::woltrust::cert::{Cert, Kind, Subject};
use wol_core::woltrust::key::KeyMaterial;
use wol_core::woltrust::{Purpose, make, seal, wolkey};

/// Extension of a sealed file.
pub const SEALED_EXT: &str = "wolsealed";
/// Largest file `wol sign` or `wol encrypt` reads.
const MAX_INPUT: u64 = 64 << 20;

fn random(buf: &mut [u8]) -> Result<(), String> {
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(buf))
        .map_err(|_| "This computer gave no random bytes.".to_owned())
}

fn read_capped(p: &Path, cap: u64) -> Result<Vec<u8>, String> {
    let f = std::fs::File::open(p).map_err(|_| format!("Can't read {}.", p.display()))?;
    let mut buf = Vec::new();
    f.take(cap + 1)
        .read_to_end(&mut buf)
        .map_err(|_| format!("Can't read {}.", p.display()))?;
    if u64::try_from(buf.len()).unwrap_or(u64::MAX) > cap {
        return Err(format!("{} is larger than {} MB.", p.display(), cap >> 20));
    }
    Ok(buf)
}

/// Writes a new file, owner-only, never over an existing one.
fn write_new(p: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|_| format!("Can't create {}.", d.display()))?;
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(p)
        .and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()))
        .map_err(|_| format!("Can't write {} (does it exist already?).", p.display()))
}

/// The passphrase after `prompt`, never echoed on a terminal (`crate::secret`).
fn passphrase(env: &mut dyn Env, prompt: &str) -> Option<zeroize_free::Pass> {
    env.read_secret(prompt).map(zeroize_free::Pass)
}

/// A passphrase wrapper that clears its text when dropped.
mod zeroize_free {
    pub struct Pass(pub String);
    impl Drop for Pass {
        // Releases nothing but memory: overwrites the passphrase text.
        fn drop(&mut self) {
            let n = self.0.len();
            self.0.clear();
            self.0.extend(std::iter::repeat_n('\0', n));
        }
    }
}

fn unlock(env: &mut dyn Env, name: &str) -> Result<KeyMaterial, String> {
    let path = ak::key_path(&env.data_home(), name);
    let file = read_capped(&path, 4096)
        .map_err(|_| format!("There is no key {name} on this account (wol cert list)."))?;
    let pass = passphrase(env, "Passphrase of the key: ").ok_or("No passphrase given.")?;
    wolkey::unlock(&file, name, &pass.0).map_err(|e| e.sentence().to_owned())
}

fn load_cert(env: &dyn Env, what: &str) -> Result<Cert, String> {
    let p = if wol_core::woltrust::cert::is_serial(what) {
        ak::cert_path(&env.data_home(), what)
    } else {
        PathBuf::from(what)
    };
    let bytes = read_capped(&p, 64 * 1024)
        .map_err(|_| format!("No certificate {what} here; add it with wol cert add FILE."))?;
    Cert::parse(&bytes).map_err(|e| format!("{what}: {e}"))
}

fn done(env: &mut dyn Env, r: Result<(), String>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => fail(env, ErrorCode::Failed, &e),
    }
}

/// `wol cert request GITHUB_ID LOGIN NAMESPACE NAME... [--org]`: new keys, locked with the
/// passphrase on stdin, and the request file (printed) for the hub.
pub fn request(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let r = (|| {
        let id: u64 = inv
            .get("github_id")
            .and_then(|v| v.parse().ok())
            .ok_or("The GitHub id is the account's number (api.github.com/users/LOGIN).")?;
        let name = inv.all("name").join(" ");
        let subject = Subject {
            github_id: Some(id),
            login: inv.get("login").map(str::to_owned),
            name: Some(name),
            label: None,
            org: None,
            text: None,
        };
        let kind = if inv.flag("org") {
            Kind::Org
        } else {
            Kind::Person
        };
        let ns = vec![inv.get("namespace").unwrap_or_default().to_owned()];
        let mut seed = [0u8; 32];
        random(&mut seed)?;
        let km = KeyMaterial::new(seed, false);
        seed.fill(0);
        let req = ak::request(&km, kind, subject, ns, env.now_secs())?;
        let pass = passphrase(env, "Passphrase for the new key (at least 8 characters): ")
            .ok_or("No passphrase given.")?;
        let mut rnd = [0u8; 28];
        random(&mut rnd)?;
        let id = km.key_id();
        let locked = wolkey::lock(&km, &id, &pass.0, &rnd, wolkey::ROUNDS)
            .map_err(|e| e.sentence().to_owned())?;
        let dh = env.data_home();
        write_new(&ak::key_path(&dh, &id), &locked, 0o600)?;
        let text = serde_json::to_string_pretty(&req).map_err(|e| e.to_string())?;
        let req_path = dh.join(ak::TRUST_DIR).join(format!("{id}.request.json"));
        write_new(&req_path, text.as_bytes(), 0o644)?;
        env.out(&format!("{text}\n"));
        env.err(&format!(
            "Key {id} saved, locked with your passphrase. Send {} to WolOS Hub; when the certificate comes, run wol cert add FILE.\n",
            req_path.display()
        ));
        Ok(())
    })();
    done(env, r)
}

/// `wol cert add FILE`: keeps a certificate. When it is for one of this account's keys, the key
/// is checked (passphrase on stdin) and renamed to the certificate's serial.
pub fn add(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let r = (|| {
        let file = inv.get("file").unwrap_or_default();
        let bytes = read_capped(Path::new(file), 64 * 1024)?;
        let c = Cert::parse(&bytes).map_err(|e| format!("{file}: {e}"))?;
        let dh = env.data_home();
        // Rule "IDS": a key made now is already stored under the certificate's serial; one made
        // before waits under its 16-hex key id and is renamed.
        let id = ak::legacy_key_id_of(&c);
        let pending = ak::key_path(&dh, &id);
        let named = ak::key_path(&dh, &c.body.serial);
        if ak::key_id_of(&c) == c.body.serial && named.exists() {
            let km = unlock(env, &c.body.serial)?;
            if !ak::is_mine(&c, &km) {
                return Err("This certificate names other keys than yours.".into());
            }
        } else if pending.exists() {
            let km = unlock(env, &id)?;
            if !ak::is_mine(&c, &km) {
                return Err("This certificate names other keys than yours.".into());
            }
            let pass = passphrase(env, "Passphrase again, to store the key under its serial: ")
                .ok_or("No passphrase given.")?;
            let mut rnd = [0u8; 28];
            random(&mut rnd)?;
            let old = read_capped(&pending, 4096)?;
            let moved = wolkey::rename(&old, &id, &c.body.serial, &pass.0, &rnd)
                .map_err(|e| e.sentence().to_owned())?;
            write_new(&ak::key_path(&dh, &c.body.serial), &moved, 0o600)?;
            std::fs::remove_file(&pending).map_err(|_| "Can't remove the old key file.")?;
        }
        let dest = ak::cert_path(&dh, &c.body.serial);
        if !dest.exists() {
            write_new(&dest, &bytes, 0o644)?;
        }
        env.out(&format!("{}\n", ak::describe(&c, env.now_secs())));
        Ok(())
    })();
    done(env, r)
}

/// `wol cert show SERIAL|FILE`.
pub fn show(env: &mut dyn Env, inv: &Invocation) -> i32 {
    match load_cert(env, inv.get("cert").unwrap_or_default()) {
        Ok(c) if inv.flag("json") => {
            let v = serde_json::to_value(&c.body).unwrap_or_default();
            emit_json(env, &v);
            0
        }
        Ok(c) => {
            let line = ak::describe(&c, env.now_secs());
            env.out(&format!("{line}\n"));
            0
        }
        Err(e) => fail(env, ErrorCode::NotFound, &e),
    }
}

fn names(dir: &Path, ext: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.strip_suffix(&format!(".{ext}")).map(str::to_owned)
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// `wol cert list`: certificates kept here, and keys still waiting for theirs.
pub fn list(env: &mut dyn Env, _inv: &Invocation) -> i32 {
    let dh = env.data_home();
    let certs = names(&ak::certs_dir(&dh), ak::CERT_EXT);
    let keys = names(&ak::keys_dir(&dh), ak::KEY_EXT);
    let now = env.now_secs();
    for s in &certs {
        if let Ok(c) = load_cert(env, s) {
            let mine = if keys.contains(s) { " (your key)" } else { "" };
            env.out(&format!("{}{mine}\n", ak::describe(&c, now)));
        }
    }
    for k in keys.iter().filter(|k| !certs.contains(k)) {
        env.out(&format!("{k} key waiting for its certificate\n"));
    }
    if certs.is_empty() && keys.is_empty() {
        env.out("No certificates yet. Make keys with wol cert request.\n");
    }
    0
}

/// The members of the file to sign, in order, and the `SHA256` bytes both signatures cover.
struct ToSign {
    parts: Vec<(String, Vec<u8>)>,
    manifest: Vec<u8>,
    purpose: Purpose,
}

fn already_signed() -> String {
    "This file is signed with a certificate already.".into()
}

/// A `.wol`: every member read with its cap.
fn wol_parts(path: &Path) -> Result<ToSign, String> {
    let ms = wol_core::package::members(path)?;
    if ms
        .iter()
        .any(|m| [wol_core::package::CHAIN, wol_core::package::WSIG].contains(&m.name.as_str()))
    {
        return Err(already_signed());
    }
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    for m in &ms {
        parts.push((
            m.name.clone(),
            wol_core::package::read_member(path, m, shared::package::MAX_SIZE)?,
        ));
    }
    let manifest = parts
        .iter()
        .find(|(n, _)| n == shared::package::MANIFEST)
        .map(|(_, b)| b.clone())
        .ok_or("This package has no SHA256 member.")?;
    Ok(ToSign {
        parts,
        manifest,
        purpose: Purpose::Package,
    })
}

/// A `.wolplug`: unpacked through zstd with its bound, every check of `wol plugin add` (a legacy
/// index signature must still verify), and the plugin id inside the certificate's namespaces.
fn wolplug_parts(path: &Path, c: &Cert) -> Result<ToSign, String> {
    let tar = crate::plugin::wolplug_tar(path)?;
    let keys = wol_core::package::read_keys(Path::new(shared::package::KEYS_DIR));
    let (v, extra) = wol_core::package::verify_wolplug_extra(&tar, &keys)?;
    if extra.wsig.is_some() {
        return Err(already_signed());
    }
    if !wol_core::woltrust::cert::covers(&c.body.namespaces, &v.manifest.id) {
        return Err(format!(
            "{} is outside this certificate's names ({}).",
            v.manifest.id,
            c.body.namespaces.join(", ")
        ));
    }
    let parts = wol_core::package::members_in(&tar)?
        .iter()
        .map(|m| {
            let at = usize::try_from(m.offset).unwrap_or(usize::MAX);
            let n = usize::try_from(m.size).unwrap_or(usize::MAX);
            (m.name.clone(), tar[at..at + n].to_vec())
        })
        .collect();
    Ok(ToSign {
        parts,
        manifest: extra.manifest,
        purpose: Purpose::Plugin,
    })
}

/// The signed `.wolplug` tar back through zstd (level 19, as `wol plugin pack`), within
/// `WOLPLUG_MAX_FILE`.
fn compress_wolplug(path: &Path, tar: &[u8]) -> Result<Vec<u8>, String> {
    let tmp = path.with_extension("wolplug.signing.tar");
    let _ = std::fs::remove_file(&tmp);
    write_new(&tmp, tar, 0o600)?;
    let argv: Vec<String> = ["zstd", "-q", "-19", "-c", "--"]
        .iter()
        .map(|s| (*s).to_owned())
        .chain([tmp.to_string_lossy().into_owned()])
        .collect();
    let out = wol_core::widgets::run_limited(
        &argv,
        Path::new("/"),
        std::time::Duration::from_secs(60),
        usize::try_from(wol_core::package::WOLPLUG_MAX_FILE).unwrap_or(usize::MAX),
    );
    let _ = std::fs::remove_file(&tmp);
    out.map_err(|e| match e {
        wol_core::widgets::SourceError::TooLarge => "The signed plugin file is too large.".into(),
        wol_core::widgets::SourceError::NotFound(_) => {
            "zstd is missing, so the plugin file cannot be written.".into()
        }
        _ => "zstd could not compress the signed plugin.".into(),
    })
}

/// `wol sign FILE --as SERIAL [REPO TAG COMMIT]`: adds `CHAIN` and `SHA256.wsig` to a `.wol`
/// (label `wolos/package/1`) or a `.wolplug` (label `wolos/plugin/1`, unpacked and packed again
/// with zstd); with an origin, the signature also covers where the release comes from.
pub fn sign(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let r = (|| {
        if !inv.flag("as") {
            return Err("Say which certificate signs: wol sign FILE --as SERIAL.".to_owned());
        }
        let path = PathBuf::from(inv.get("file").unwrap_or_default());
        let serial = inv.get("serial").unwrap_or_default().to_owned();
        let plugin = match path.extension().and_then(|x| x.to_str()) {
            Some("wol") => false,
            Some("wolplug") => true,
            _ => return Err("wol sign signs .wol packages and .wolplug plugins.".into()),
        };
        let c = load_cert(env, &serial)?;
        let ToSign {
            mut parts,
            manifest: man,
            purpose,
        } = if plugin {
            wolplug_parts(&path, &c)?
        } else {
            wol_parts(&path)?
        };
        let km = unlock(env, &serial)?;
        if !ak::is_mine(&c, &km) {
            return Err("This key does not belong to that certificate.".into());
        }
        let chain = wol_core::woltrust::pki::chain_of(&[&c.to_bytes()]);
        let w = match (inv.get("repo"), inv.get("tag"), inv.get("commit")) {
            (Some(repo), Some(tag), Some(commit)) => {
                let o = wol_core::woltrust::Origin {
                    repo: repo.to_owned(),
                    tag: tag.to_owned(),
                    commit: commit.to_owned(),
                    built: env.now_secs(),
                    rkdb_serial: None,
                }
                .to_bytes();
                wol_core::woltrust::Origin::parse(&o).map_err(|e| {
                    format!("The origin is not usable ({e}): REPO is https://github.com/OWNER/NAME, COMMIT the full 40-character id.")
                })?;
                if !wol_core::woltrust::Origin::parse(&o)
                    .is_ok_and(|x| x.belongs_to(&c.body.subject))
                {
                    return Err("That repository does not belong to this certificate's GitHub account or organisation.".into());
                }
                make::wsig_with_origin(&serial, &km.secret(), purpose, &man, &o)
            }
            (None, None, None) => make::wsig(&serial, &km.secret(), purpose, &man),
            _ => return Err("Give the origin as REPO TAG COMMIT, all three.".into()),
        };
        parts.push((wol_core::package::CHAIN.into(), chain));
        parts.push((wol_core::package::WSIG.into(), w));
        let refs: Vec<(&str, &[u8])> = parts
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let tar = wol_core::package::ustar(&refs);
        let bytes = if plugin {
            compress_wolplug(&path, &tar)?
        } else {
            tar
        };
        let tmp = path.with_extension(if plugin {
            "wolplug.signing"
        } else {
            "wol.signing"
        });
        let _ = std::fs::remove_file(&tmp);
        write_new(&tmp, &bytes, 0o644)?;
        std::fs::rename(&tmp, &path).map_err(|_| format!("Can't replace {}.", path.display()))?;
        env.out(&format!("Signed {} as {serial}.\n", path.display()));
        Ok(())
    })();
    done(env, r)
}

/// `wol encrypt FILE --to SERIAL`: writes `FILE.wolsealed`, readable only with that
/// certificate's keys.
pub fn encrypt(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let r = (|| {
        if !inv.flag("to") {
            return Err("Say who may read it: wol encrypt FILE --to SERIAL.".to_owned());
        }
        let path = PathBuf::from(inv.get("file").unwrap_or_default());
        let c = load_cert(env, inv.get("serial").unwrap_or_default())?;
        let data = read_capped(&path, MAX_INPUT)?;
        let mut rnd = [0u8; 64];
        random(&mut rnd)?;
        let sealed =
            seal::seal(&c, seal::FILE_LABEL, &data, &rnd).map_err(|e| e.sentence().to_owned())?;
        rnd.fill(0);
        let out = PathBuf::from(format!("{}.{SEALED_EXT}", path.display()));
        write_new(&out, &sealed, 0o644)?;
        env.out(&format!(
            "Sealed to {} as {}.\n",
            ak::describe(&c, env.now_secs()),
            out.display()
        ));
        Ok(())
    })();
    done(env, r)
}

/// `wol decrypt FILE.wolsealed`: opens it with this account's key (passphrase on stdin) and
/// writes FILE next to it.
pub fn decrypt(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let r = (|| {
        let path = PathBuf::from(inv.get("file").unwrap_or_default());
        let out = path
            .to_string_lossy()
            .strip_suffix(&format!(".{SEALED_EXT}"))
            .map(PathBuf::from)
            .ok_or("wol decrypt opens .wolsealed files.")?;
        let sealed = read_capped(&path, MAX_INPUT + 8192)?;
        let h = seal::peek(&sealed).map_err(|e| e.sentence().to_owned())?;
        let km = unlock(env, &h.recipient)?;
        let plain = seal::open(&km.enc(), &h.recipient, seal::FILE_LABEL, &sealed)
            .map_err(|e| e.sentence().to_owned())?;
        write_new(&out, &plain, 0o600)?;
        env.out(&format!("Opened {}.\n", out.display()));
        Ok(())
    })();
    done(env, r)
}

/// `wol trust status`: what the certificate store holds and how old its revocation list is.
pub fn status(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let ts = wol_core::trust::TrustStore::load(&wol_core::system::PosixHost);
    let s = ts.status(env.now_secs());
    if inv.flag("json") {
        emit_json(env, &serde_json::to_value(&s).unwrap_or_default());
        return 0;
    }
    env.out(&format!(
        "roots: {}\nintermediates: {}\n",
        s.roots, s.intermediates
    ));
    match (s.list_serial, s.list_age) {
        (Some(n), Some(age)) => env.out(&format!(
            "revocation list: number {n}, {} withdrawn, issued {} hours ago, {}\n",
            s.revoked,
            age / 3600,
            s.freshness
        )),
        _ => env.out("revocation list: none yet\n"),
    }
    match s.rkdb_serial {
        Some(n) => env.out(&format!(
            "revocation database: number {n}, {} withdrawn, floor {}\n",
            s.rkdb_entries, s.floor
        )),
        None => env.out(&format!(
            "revocation database: none yet, floor {}\n",
            s.floor
        )),
    }
    if s.behind {
        env.out("could not update the list: new apps wait until it is current; installed apps keep working\n");
    }
    for t in &s.root_texts {
        env.out(&format!("who vouches: {t}\n"));
    }
    if s.remembered > 0 {
        env.out(&format!(
            "remembered withdrawals: {} (kept forever)\n",
            s.remembered
        ));
    }
    let keys = wol_core::package::read_keys(Path::new(shared::package::KEYS_DIR));
    for k in &keys {
        // Audit H1: the previous flat keys can be withdrawn in revoked.json by this id.
        env.out(&format!(
            "previous key: {} ({})\n",
            wol_core::woltrust::request::key_id(k),
            if s.roots == 0 {
                "accepted until WolOS roots are installed"
            } else {
                "refused: WolOS roots are installed"
            }
        ));
    }
    if s.roots == 0 {
        env.out("No WolOS root is installed yet, so no certificate can be verified.\n");
    }
    0
}

/// GET `url` over HTTPS only (no redirect to another scheme), at most `max` bytes, 60 s.
fn fetch(url: &str, max: usize) -> Option<Vec<u8>> {
    let argv: Vec<String> = [
        "curl",
        "-fsSL",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--max-filesize",
        &max.to_string(),
        "--max-time",
        "60",
        "--",
        url,
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    wol_core::widgets::run_limited(
        &argv,
        Path::new("/"),
        std::time::Duration::from_secs(70),
        max,
    )
    .ok()
    .filter(|b| !b.is_empty())
}

/// The list and its signature from the first of [`wol_core::trust::revoked_urls`] that gives
/// both (the CDN, then its mirror).
fn fetch_list() -> Result<(Vec<u8>, Vec<u8>), String> {
    wol_core::trust::revoked_urls()
        .iter()
        .find_map(|u| {
            // The caps the store reads them with.
            let list = fetch(u, wol_core::woltrust::MAX_REVOKED_BYTES)?;
            let sig = fetch(
                &format!("{u}{}", wol_core::trust::SIG_SUFFIX),
                wol_core::woltrust::MAX_WSIG,
            )?;
            Some((list, sig))
        })
        .ok_or_else(|| {
            format!(
                "The revocation list could not be downloaded ({}).",
                wol_core::trust::revoked_urls().join(", ")
            )
        })
}

/// Replaces `to` with `bytes` through a `.new` file and a rename.
fn replace(to: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = to.with_extension("new");
    let _ = std::fs::remove_file(&tmp);
    write_new(&tmp, bytes, 0o644)?;
    std::fs::rename(&tmp, to).map_err(|_| format!("Can't replace {}.", to.display()))
}

fn read_or_empty(p: &Path) -> Result<Vec<u8>, String> {
    match std::fs::read(p) {
        Ok(b) => Ok(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err(format!("Can't read {}.", p.display())),
    }
}

/// Installs a verified revocation database (lead decision 2026-11-04 "RKDB"): the memory of
/// root-signed withdrawals grows first, then the database, then the floor.
fn install_rkdb(
    env: &mut dyn Env,
    db: &[u8],
    cdn: Option<&[u8]>,
    log: Option<&[u8]>,
) -> Result<(), String> {
    let ts = wol_core::trust::TrustStore::load(&wol_core::system::PosixHost);
    let now = env.now_secs();
    let up = ts.accept_rkdb(db, cdn, log, now)?;
    let seen = Path::new(wol_core::trust::SEEN_FILE);
    let memory = ts.memory_after_rkdb(&read_or_empty(seen)?, db, now)?;
    replace(seen, &memory)?;
    replace(Path::new(wol_core::trust::STATE_RKDB), db)?;
    let floor_path = Path::new(wol_core::trust::FLOOR_FILE);
    let old = wol_core::trust::parse_floor(&read_or_empty(floor_path)?);
    replace(floor_path, &wol_core::trust::floor_text(old.max(up.floor)))?;
    if let Some(w) = &up.warning {
        env.err(&format!("wol trust update: {w}\n"));
    }
    env.out(&format!("revocation database {} installed\n", up.serial));
    Ok(())
}

/// Installs a verified JSON list and its signature (the fallback, and `wol trust update LIST SIG`).
fn install_list(env: &mut dyn Env, list: &[u8], sig: &[u8]) -> Result<(), String> {
    let ts = wol_core::trust::TrustStore::load(&wol_core::system::PosixHost);
    let serial = ts.accept_list(list, sig, env.now_secs())?;
    // Audit M1: the memory of every withdrawal seen grows first, so a crash between the two
    // writes never loses one.
    let seen_path = Path::new(wol_core::trust::SEEN_FILE);
    let memory = ts.memory_after(&read_or_empty(seen_path)?, list, sig, env.now_secs())?;
    replace(seen_path, &memory)?;
    let dest_sig = PathBuf::from(format!(
        "{}{}",
        wol_core::trust::STATE_REVOKED,
        wol_core::trust::SIG_SUFFIX
    ));
    replace(&dest_sig, sig)?;
    replace(Path::new(wol_core::trust::STATE_REVOKED), list)?;
    env.out(&format!("revocation list {serial} installed\n"));
    Ok(())
}

/// `wol trust update [FILE [SIG]]` (root; the update-check service, hourly): installs the
/// revocation database (`FILE` a `revoked.rkdb`) or a JSON list with its signature into
/// `/var/lib/wolos/trust`, only when it verifies, is not below the floor and keeps every
/// withdrawal held. Without files it fetches `revoked.rkdb` from [`wol_core::trust::rkdb_urls`]
/// and the two checkpoints ([`wol_core::cdn::RKDB_CHECKPOINTS`]), else the JSON list from
/// [`wol_core::trust::revoked_urls`]; when neither comes it says "could not update the list".
pub fn update(env: &mut dyn Env, inv: &Invocation) -> i32 {
    if !env.is_root() {
        return fail(
            env,
            ErrorCode::Denied,
            "wol trust update runs as root (update-check).",
        );
    }
    let r = (|| match (inv.get("list"), inv.get("sig")) {
        (Some(f), None) => {
            let db = read_capped(Path::new(f), wol_core::trust::MAX_RKDB_BYTES as u64)?;
            install_rkdb(env, &db, None, None)
        }
        (Some(l), Some(s)) => install_list(
            env,
            &read_capped(Path::new(l), 1 << 20)?,
            &read_capped(Path::new(s), 64 * 1024)?,
        ),
        (None, None) => {
            let db = wol_core::trust::rkdb_urls()
                .iter()
                .find_map(|u| fetch(u, wol_core::trust::MAX_RKDB_BYTES));
            if let Some(db) = db {
                let cps: Vec<Option<Vec<u8>>> = wol_core::cdn::RKDB_CHECKPOINTS
                    .iter()
                    .map(|u| fetch(u, 256))
                    .collect();
                return install_rkdb(env, &db, cps[0].as_deref(), cps[1].as_deref());
            }
            match fetch_list() {
                Ok((list, sig)) => install_list(env, &list, &sig),
                Err(_) => Err(
                    "WolOS could not update the list of withdrawn certificates. New apps wait until it can; apps already installed keep working.".to_owned(),
                ),
            }
        }
        _ => Err("Give a revoked.rkdb, or a revoked.json and its .wsig, or nothing.".to_owned()),
    })();
    done(env, r)
}

/// `wol trust show FILE`: the signature line of a certificate, a file certificate, a revocation
/// database (each entry), a root-signed section, or a `.wol` (its signature), the same line the
/// Store sheet and `woltrust verify` show.
pub fn show_line(env: &mut dyn Env, inv: &Invocation) -> i32 {
    use wol_core::woltrust::{self as wt, line, rkdb};
    let r =
        (|| {
            let f = inv.get("file").unwrap_or_default();
            let bytes = read_capped(Path::new(f), wol_core::trust::MAX_RKDB_BYTES as u64)?;
            let ts = wol_core::trust::TrustStore::load(&wol_core::system::PosixHost);
            let st = &ts.store;
            let name_of = |serial: &str| {
                st.roots
                    .iter()
                    .chain(st.intermediates.iter())
                    .find(|c| c.body.serial == serial)
                    .map_or_else(|| serial.to_owned(), line::ca_name)
            };
            let mut out = Vec::new();
            if bytes.starts_with(rkdb::MAGIC) {
                let db = rkdb::Db::parse(&bytes).map_err(|e| format!("{f}: {}", e.as_str()))?;
                let checked = st.check_rkdb(&bytes, env.now_secs()).is_ok();
                if !checked {
                    out.push(
                        "(this database does not verify against this computer's roots)".to_owned(),
                    );
                }
                let by = name_of(&rkdb::id_text(&db.signer));
                if let Some(r) = &db.root {
                    let rb = name_of(&rkdb::id_text(&r.signer));
                    for e in r.entries.iter() {
                        out.push(line::revocation(
                            &rkdb::id_text(&e.id),
                            u64::from(e.at),
                            e.reason,
                            &rb,
                        ));
                    }
                }
                for e in db.entries.iter() {
                    out.push(line::revocation(
                        &rkdb::id_text(&e.id),
                        u64::from(e.at),
                        e.reason,
                        &by,
                    ));
                }
            } else if bytes.starts_with(rkdb::ROOT_MAGIC) {
                let (r, _) =
                    rkdb::RootSection::parse(&bytes).map_err(|e| format!("{f}: {}", e.as_str()))?;
                let by = name_of(&rkdb::id_text(&r.signer));
                for e in r.entries.iter() {
                    out.push(line::revocation(
                        &rkdb::id_text(&e.id),
                        u64::from(e.at),
                        e.reason,
                        &by,
                    ));
                }
            } else if let Ok(c) = Cert::parse(&bytes) {
                let mut issuers: Vec<&Cert> = st
                    .roots
                    .iter()
                    .chain(st.intermediates.iter())
                    .filter(|i| i.body.serial == c.body.issuer && c.body.kind != Kind::Root)
                    .collect();
                issuers.extend(
                    st.roots
                        .iter()
                        .filter(|r| c.cosigs.iter().any(|x| x.issuer == r.body.serial)),
                );
                out.push(line::cert(&c, &issuers));
            } else if let Some(body) = filecert_body(&bytes) {
                let ok = wt::filecert::verify(st, &bytes, &body.file_sha256).is_ok();
                out.push(line::file_certificate(&body, None));
                if !ok {
                    out.push(
                        "(this file certificate does not verify against this computer's roots)"
                            .to_owned(),
                    );
                }
            } else {
                let keys = wol_core::package::read_keys(Path::new(shared::package::KEYS_DIR));
                let c = ts.verify_wol_at(Path::new(f), &keys, None, env.now_secs())?;
                out.push(c.outcome.line.unwrap_or_else(|| {
                    format!("(no signature line: {})", c.outcome.code.as_str())
                }));
            }
            for l in out {
                env.out(&format!("{l}\n"));
            }
            Ok(())
        })();
    done(env, r)
}

fn filecert_body(bytes: &[u8]) -> Option<wol_core::woltrust::filecert::Body> {
    let f: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    f.get("cert")?;
    let raw = wol_core::woltrust::b64::decode(
        f.get("body")?.as_str()?,
        wol_core::woltrust::filecert::MAX_BODY,
    )?;
    serde_json::from_slice(&raw).ok()
}
