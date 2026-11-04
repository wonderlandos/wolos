// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol cert request|add|show|list`, `wol sign`, `wol encrypt FILE --to SERIAL`, `wol decrypt` and
//! `wol trust status` (ADR 0046): an author's keys and certificates under the account's data
//! home, locked with a passphrase (`wol_core::authorkeys`, `woltrust::wolkey`), files sealed with
//! `wolos/seal/1`. Passphrases are read as one line from stdin, never from an argument; a key is
//! never written in the clear.
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

fn passphrase(env: &mut dyn Env) -> Option<zeroize_free::Pass> {
    env.stdin_line().map(zeroize_free::Pass)
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
    env.err("Passphrase of the key: ");
    let pass = passphrase(env).ok_or("No passphrase given.")?;
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
        env.err("Passphrase for the new key (at least 8 characters): ");
        let pass = passphrase(env).ok_or("No passphrase given.")?;
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
        let id = ak::key_id_of(&c);
        let pending = ak::key_path(&dh, &id);
        if pending.exists() {
            let km = unlock(env, &id)?;
            if !ak::is_mine(&c, &km) {
                return Err("This certificate names other keys than yours.".into());
            }
            env.err("Passphrase again, to store the key under its serial: ");
            let pass = passphrase(env).ok_or("No passphrase given.")?;
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

/// `wol sign FILE.wol --as SERIAL [REPO TAG COMMIT]`: adds `CHAIN` and `SHA256.wsig` to the
/// `.wol`; with an origin, the signature also covers where the release comes from.
pub fn sign(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let r = (|| {
        if !inv.flag("as") {
            return Err("Say which certificate signs: wol sign FILE --as SERIAL.".to_owned());
        }
        let path = PathBuf::from(inv.get("file").unwrap_or_default());
        let serial = inv.get("serial").unwrap_or_default().to_owned();
        if path.extension().is_none_or(|x| x != "wol") {
            return Err("wol sign signs .wol packages.".into());
        }
        let c = load_cert(env, &serial)?;
        let ms = wol_core::package::members(&path)?;
        if ms
            .iter()
            .any(|m| [wol_core::package::CHAIN, wol_core::package::WSIG].contains(&m.name.as_str()))
        {
            return Err("This package is signed with a certificate already.".into());
        }
        let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
        for m in &ms {
            parts.push((
                m.name.clone(),
                wol_core::package::read_member(&path, m, shared::package::MAX_SIZE)?,
            ));
        }
        let man = parts
            .iter()
            .find(|(n, _)| n == shared::package::MANIFEST)
            .map(|(_, b)| b.clone())
            .ok_or("This package has no SHA256 member.")?;
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
                make::wsig_with_origin(&serial, &km.secret(), Purpose::Package, &man, &o)
            }
            (None, None, None) => make::wsig(&serial, &km.secret(), Purpose::Package, &man),
            _ => return Err("Give the origin as REPO TAG COMMIT, all three.".into()),
        };
        parts.push((wol_core::package::CHAIN.into(), chain));
        parts.push((wol_core::package::WSIG.into(), w));
        let refs: Vec<(&str, &[u8])> = parts
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let tmp = path.with_extension("wol.signing");
        write_new(&tmp, &wol_core::package::ustar(&refs), 0o644)?;
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
    if s.roots == 0 {
        env.out("No WolOS root is installed yet, so no certificate can be verified.\n");
    }
    0
}

/// `wol trust update LIST SIG` (root; the update-check service): installs a downloaded
/// revocation list into `/var/lib/wolos/trust` only when it verifies and is newer.
pub fn update(env: &mut dyn Env, inv: &Invocation) -> i32 {
    if !env.is_root() {
        return fail(
            env,
            ErrorCode::Denied,
            "wol trust update runs as root (update-check).",
        );
    }
    let r = (|| {
        let list = read_capped(Path::new(inv.get("list").unwrap_or_default()), 1 << 20)?;
        let sig = read_capped(Path::new(inv.get("sig").unwrap_or_default()), 64 * 1024)?;
        let ts = wol_core::trust::TrustStore::load(&wol_core::system::PosixHost);
        let serial = ts.accept_list(&list, &sig, env.now_secs())?;
        let dest = Path::new(wol_core::trust::STATE_REVOKED);
        let dest_sig = PathBuf::from(format!(
            "{}{}",
            wol_core::trust::STATE_REVOKED,
            wol_core::trust::SIG_SUFFIX
        ));
        for (to, bytes) in [(dest_sig.as_path(), &sig), (dest, &list)] {
            let tmp = to.with_extension("new");
            let _ = std::fs::remove_file(&tmp);
            write_new(&tmp, bytes, 0o644)?;
            std::fs::rename(&tmp, to).map_err(|_| format!("Can't replace {}.", to.display()))?;
        }
        env.out(&format!("revocation list {serial} installed\n"));
        Ok(())
    })();
    done(env, r)
}
