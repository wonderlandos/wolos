// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol seal apply|check` and `wol encrypt ...`: the installer and updater call these; the
//! decisions live in `wol_core::seal` and `wol_core::encrypt`. Secrets are files, never arguments.
use crate::cli::Invocation;
use crate::run::{Env, fail};
use shared::errors::ErrorCode;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use wol_core::{encrypt, seal, update};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut v: Vec<_> = rd.flatten().collect();
    v.sort_by_key(|e| e.file_name());
    for e in v {
        match e.file_type() {
            Ok(t) if t.is_dir() => walk(&e.path(), out),
            Ok(t) if t.is_file() => out.push(e.path()),
            _ => {}
        }
    }
}

fn measure(env: &mut dyn Env, path: &Path) -> Option<String> {
    let argv = seal::measure_argv(&path.to_string_lossy());
    let a: Vec<&str> = argv.iter().map(String::as_str).collect();
    seal::parse_measure(&env.capture(&a)?)
}

/// `wol seal apply ROOT DIGESTFILE`: fs-verity on the slot's executables and libraries, the
/// manifest inside the slot, its root digest at DIGESTFILE, then the subvolume read-only.
pub fn apply(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let (Some(root), Some(digest)) = (inv.get("root"), inv.get("digestfile")) else {
        return fail(
            env,
            ErrorCode::Usage,
            "usage: wol seal apply ROOT DIGESTFILE",
        );
    };
    if !env.is_root() {
        return fail(env, ErrorCode::Denied, "seal needs root");
    }
    let root = Path::new(root);
    let mut files = Vec::new();
    for d in seal::DIRS {
        walk(&root.join(d), &mut files);
    }
    let mut man = seal::Manifest::default();
    for f in files {
        let Ok(rel) = f
            .strip_prefix(root)
            .map(|r| r.to_string_lossy().into_owned())
        else {
            continue;
        };
        let mut head = [0u8; 4];
        let n = std::fs::File::open(&f)
            .and_then(|mut h| std::io::Read::read(&mut h, &mut head))
            .unwrap_or(0);
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(&f)
                .map(|m| m.permissions())
                .unwrap_or_else(|_| std::fs::Permissions::from_mode(0o0)),
        );
        if !seal::wants_verity(&rel, mode, &head[..n]) {
            continue;
        }
        if measure(env, &f).is_none()
            && env
                .exec(&seal::enable_argv(&f.to_string_lossy()))
                .unwrap_or(1)
                != 0
        {
            return fail(
                env,
                ErrorCode::Failed,
                &format!("fs-verity could not be enabled on {rel}"),
            );
        }
        let Some(d) = measure(env, &f) else {
            return fail(
                env,
                ErrorCode::Failed,
                &format!("no fs-verity digest for {rel}"),
            );
        };
        man.0.insert(rel, d);
    }
    let mpath = root.join(seal::MANIFEST);
    if env.write_file(&mpath, &man.to_text()).is_err()
        || env
            .write_file(Path::new(digest), &format!("{}\n", man.root_digest()))
            .is_err()
    {
        return fail(env, ErrorCode::Failed, "could not write the seal records");
    }
    if env
        .exec(&seal::seal_argv(&root.to_string_lossy()))
        .unwrap_or(1)
        != 0
    {
        return fail(
            env,
            ErrorCode::Failed,
            "could not make the system read-only",
        );
    }
    env.out(&format!("sealed {} files\n", man.0.len()));
    0
}

/// `wol seal check` (early boot): compare the running slot with what was sealed; on a mismatch
/// boot the other slot and leave a message for the person.
pub fn check(env: &mut dyn Env) -> i32 {
    let cmdline = env.read_file(Path::new("/proc/cmdline"));
    let Some(slot) = cmdline.as_deref().and_then(update::running_slot) else {
        return 0;
    };
    // The signed command line carries the digest; the ESP copy is for systems from before it.
    let signed = cmdline
        .as_deref()
        .and_then(|c| wol_core::boot::cmdline_word(c, "wolos.seal"))
        .map(str::to_owned);
    let Some(expected) = signed.or_else(|| env.read_file(Path::new(&seal::digest_file(slot))))
    else {
        return 0; // never sealed (a system from before the seal): nothing to compare
    };
    let man = env
        .read_file(Path::new(&format!("/{}", seal::MANIFEST)))
        .map(|t| seal::Manifest::parse(&t))
        .unwrap_or_default();
    let mut cache = std::collections::HashMap::new();
    let keys: Vec<&String> = man.0.keys().collect();
    for chunk in keys.chunks(512) {
        let paths: Vec<String> = chunk.iter().map(|p| format!("/{p}")).collect();
        let argv = seal::measure_many_argv(&paths);
        let a: Vec<&str> = argv.iter().map(String::as_str).collect();
        if let Some(out) = env.capture(&a) {
            for (p, d) in seal::parse_measures(&out) {
                cache.insert(p.trim_start_matches('/').to_owned(), Some(d));
            }
        } else {
            // A file with no seal stops the whole run: ask for each one.
            for p in chunk {
                cache.insert((*p).clone(), measure(env, &Path::new("/").join(p)));
            }
        }
    }
    let v = seal::verify(&man, &expected, &|p| cache.get(p).cloned().flatten());
    if v == seal::Verdict::Ok {
        return 0;
    }
    let other = update::other_slot(slot).unwrap_or("");
    let failed_mark = |s: &str| PathBuf::from(format!("/var/lib/wolos/verity-failed-{s}"));
    let entry = |suffix: &str| {
        Path::new(&format!(
            "{}/loader/entries/{other}{suffix}.conf",
            update::ESP
        ))
        .exists()
    };
    let other_ok = !other.is_empty()
        && !failed_mark(other).exists()
        && (entry("") || entry(&format!("+{}", update::TRIES)));
    let fb = seal::fallback(slot, other_ok);
    let _ = env.write_file(&failed_mark(slot), "failed\n");
    let msg = seal::notice(slot, &v, matches!(fb, seal::Fallback::Boot { .. }));
    let _ = env.write_file(Path::new(seal::NOTICE), &msg);
    env.err(&msg);
    if let seal::Fallback::Boot { argv } = fb {
        for a in argv {
            let _ = env.exec(&a);
        }
    }
    1
}

/// `wol encrypt decide MODE`: prints off|tpm2|passphrase (passphrase file from $WOLOS_PASSPHRASE_FILE).
pub fn decide(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let Some(mode) = inv.get("a").and_then(encrypt::Mode::parse) else {
        return fail(
            env,
            ErrorCode::Usage,
            "usage: wol encrypt decide auto|yes|no|passphrase",
        );
    };
    let tpm_dir = env
        .var("WOLOS_TPM_DIR")
        .unwrap_or_else(|| "/sys/class/tpm/tpm0".into());
    let tpm = encrypt::has_tpm2(
        env.read_file(&Path::new(&tpm_dir).join("tpm_version_major"))
            .as_deref(),
    );
    // What enroll adds is what is checked here: the same reading of the file (audit3 R2B-6).
    let pass = match env
        .var("WOLOS_PASSPHRASE_FILE")
        .and_then(|f| crate::install::secret_file(env, Path::new(&f)))
    {
        Some(Ok(p)) => Some(p),
        Some(Err(e)) => {
            return fail(
                env,
                ErrorCode::Usage,
                &format!("the passphrase file is not typed text ({e:?})"),
            );
        }
        None => None,
    };
    match encrypt::decide(mode, tpm, pass.as_deref()) {
        Ok(u) => {
            env.out(match u {
                encrypt::Unlock::Off => "off\n",
                encrypt::Unlock::Tpm2 => "tpm2\n",
                encrypt::Unlock::Passphrase => "passphrase\n",
            });
            0
        }
        Err(encrypt::Refusal::NeedPassphrase) => fail(
            env,
            ErrorCode::Usage,
            "encrypting without a TPM needs a passphrase",
        ),
        Err(encrypt::Refusal::TooShort) => fail(
            env,
            ErrorCode::Usage,
            "the passphrase needs at least 8 characters",
        ),
    }
}

/// Where install-run's results live: a directory it creates, root's only (audit3 R2B-1). The
/// Install app reads them through `install-check --result`.
pub const INSTALL_RESULTS: &str = "/run/wolos/install";

/// `wol encrypt enroll PART KEYFILE tpm2|passphrase [PASSFILE]`: the recovery key goes to
/// $WOLOS_RECOVERY_OUT (default [`INSTALL_RESULTS`]`/recovery-key`, mode 600, shown once by the
/// installer), never to a log. PASSFILE is read as typed ([`encrypt::typed_secret`]); what is
/// added to the disk is that, kept for the moment in a file next to KEYFILE (install-run's key
/// directory, root's, gone with it).
pub fn enroll(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let (Some(part), Some(key), Some(u)) = (inv.get("a"), inv.get("b"), inv.get("c")) else {
        return fail(
            env,
            ErrorCode::Usage,
            "usage: wol encrypt enroll PART KEYFILE tpm2|passphrase [PASSFILE]",
        );
    };
    let unlock = if u == "tpm2" {
        encrypt::Unlock::Tpm2
    } else {
        encrypt::Unlock::Passphrase
    };
    let out = env
        .var("WOLOS_RECOVERY_OUT")
        .unwrap_or_else(|| format!("{INSTALL_RESULTS}/recovery-key"));
    let tpm_pub = env.var("WOLOS_TPM_PUB");
    let typed = Path::new(key).with_file_name("typed");
    let pass = match inv
        .get("d")
        .map(|f| crate::install::secret_file(env, Path::new(f)))
    {
        None => None,
        Some(Some(Ok(p))) => {
            if env.write_file(&typed, &p).is_err() {
                return fail(env, ErrorCode::Failed, "could not read the password file");
            }
            let _ = std::fs::set_permissions(
                &typed,
                std::os::unix::fs::PermissionsExt::from_mode(0o600),
            );
            Some(typed.to_string_lossy().into_owned())
        }
        Some(Some(Err(e))) => {
            return fail(
                env,
                ErrorCode::Usage,
                &format!("the password file is not typed text ({e:?})"),
            );
        }
        Some(None) => return fail(env, ErrorCode::NotFound, "no such password file"),
    };
    let rc = enroll_steps(
        env,
        part,
        key,
        unlock,
        pass.as_deref(),
        tpm_pub.as_deref(),
        &out,
    );
    let _ = std::fs::remove_file(&typed);
    rc
}

fn enroll_steps(
    env: &mut dyn Env,
    part: &str,
    key: &str,
    unlock: encrypt::Unlock,
    pass: Option<&str>,
    tpm_pub: Option<&str>,
    out: &str,
) -> i32 {
    for (i, argv) in encrypt::enroll(part, key, unlock, pass, tpm_pub)
        .iter()
        .enumerate()
    {
        if i == 0 {
            let a: Vec<&str> = argv.iter().map(String::as_str).collect();
            let Some(rk) = env.capture(&a) else {
                return fail(env, ErrorCode::Failed, "could not create the recovery key");
            };
            if env.write_file(Path::new(out), &rk).is_err() {
                return fail(env, ErrorCode::Failed, "could not keep the recovery key");
            }
            let _ =
                std::fs::set_permissions(out, std::os::unix::fs::PermissionsExt::from_mode(0o600));
        } else if env.exec(argv).unwrap_or(1) != 0 {
            return fail(env, ErrorCode::Failed, &format!("{} failed", argv[0]));
        }
    }
    0
}

/// The encrypted root's LUKS UUID from `rd.luks.name=` (None: not encrypted).
fn luks_uuid(env: &dyn Env) -> Option<String> {
    let cmdline = env.read_file(Path::new("/proc/cmdline"))?;
    wol_core::boot::cmdline_word(&cmdline, "rd.luks.name")
        .and_then(|w| w.split_once('=').map(|(u, _)| u.to_owned()))
}

/// The PCRs this machine's chain measures: shim adds PCR 14.
fn pcrs(env: &mut dyn Env) -> &'static str {
    let shim = env
        .capture(&["ls", &format!("{}/EFI/wolos/shimx64.efi", update::ESP)])
        .is_some();
    wol_core::boot::tpm_pcrs(shim)
}

/// `wol encrypt reseal` (every boot, wolos-tpm-reseal.service). When the chip refused the disk
/// (Secure Boot turned off, new firmware: PCR 7 changed), the person typed their password or the
/// recovery key, and systemd-cryptsetup left it in root's keyring ([`encrypt::PASSWORD_CACHE_KEY`]).
/// That secret unlocks the volume once more to seal the key to the state measured now: the next
/// start is automatic again. Nothing typed (the chip opened it): nothing to do.
pub fn reseal(env: &mut dyn Env) -> i32 {
    let Some(uuid) = luks_uuid(env) else {
        return 0;
    };
    let Some(raw) = env.keyring_read(encrypt::PASSWORD_CACHE_KEY) else {
        return 0;
    };
    let tries = encrypt::parse_cache(&raw);
    let public = Path::new(wol_core::boot::STATE_DIR).join("tpm2-pcr-public.pem");
    let public = public.to_string_lossy().into_owned();
    let has_pub = env.read_file(Path::new(&public)).is_some();
    let pcrs = pcrs(env);
    let argv = encrypt::reseal(
        &format!("/dev/disk/by-uuid/{uuid}"),
        has_pub.then_some(public.as_str()),
        pcrs,
    );
    // The last try is the one that opened the disk; earlier ones were wrong.
    let ok = tries
        .iter()
        .rev()
        .any(|t| env.exec_secret(&argv, "PASSWORD", t).unwrap_or(1) == 0);
    env.keyring_forget(encrypt::PASSWORD_CACHE_KEY);
    if !ok {
        return fail(
            env,
            ErrorCode::Failed,
            "the disk key could not be bound to this computer again",
        );
    }
    env.out("the disk opens with the security chip again\n");
    0
}

/// `wol encrypt password USER` (wol-admind's user-password action, after chpasswd, as root): the
/// new password on stdin. Only the account the installer gave the disk's password slot
/// ([`encrypt::UNLOCK_USER_FILE`]); the chip unlocks the volume for the change. Never fails the
/// password change itself: the chip and the recovery key still open the disk. `passwd` in a
/// terminal does not come here, so the slot keeps the previous password then.
pub fn password(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let (Some(user), Some(owner)) = (
        inv.get("a"),
        env.read_file(Path::new(encrypt::UNLOCK_USER_FILE)),
    ) else {
        return 0;
    };
    if !env.is_root() || user != owner.trim() {
        return 0;
    }
    let Some(uuid) = luks_uuid(env) else {
        return 0;
    };
    let Some(pw) = env.stdin_line() else {
        return 0;
    };
    let pw = pw.trim_end_matches(['\0', '\n']);
    if pw.is_empty() {
        return 0;
    }
    let argv = encrypt::password_argv(&format!("/dev/disk/by-uuid/{uuid}"));
    if env.exec_secret(&argv, "NEWPASSWORD", pw).unwrap_or(1) != 0 {
        env.err("wol encrypt: the disk still opens with the previous password\n");
    }
    0
}

/// `wol encrypt rebind` (first boot, wolos-tpm-bind.service): bind the disk key to the PCR values
/// of this firmware state and drop the install key.
pub fn rebind(env: &mut dyn Env) -> i32 {
    let Some(uuid) = luks_uuid(env) else {
        return 0;
    };
    let dir = Path::new(wol_core::boot::STATE_DIR);
    let pcrs = pcrs(env);
    let key = dir.join("rebind.key");
    let public = dir.join("tpm2-pcr-public.pem");
    for argv in encrypt::rebind(
        &format!("/dev/disk/by-uuid/{uuid}"),
        &key.to_string_lossy(),
        &public.to_string_lossy(),
        pcrs,
    ) {
        if env.exec(&argv).unwrap_or(1) != 0 {
            return fail(env, ErrorCode::Failed, &format!("{} failed", argv[0]));
        }
    }
    0
}

/// `wol encrypt words LUKS_UUID tpm2|passphrase` and `wol encrypt hooks FILE`.
pub fn words(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let (Some(uuid), Some(u)) = (inv.get("a"), inv.get("b")) else {
        return fail(
            env,
            ErrorCode::Usage,
            "usage: wol encrypt words LUKS_UUID tpm2|passphrase",
        );
    };
    let unlock = if u == "tpm2" {
        encrypt::Unlock::Tpm2
    } else {
        encrypt::Unlock::Passphrase
    };
    env.out(&format!("{}\n", encrypt::cmdline_words(uuid, unlock)));
    0
}

pub fn hooks(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let Some(f) = inv.get("a") else {
        return fail(env, ErrorCode::Usage, "usage: wol encrypt hooks FILE");
    };
    let Some(t) = env.read_file(Path::new(f)) else {
        return fail(env, ErrorCode::NotFound, f);
    };
    let new: String = t
        .lines()
        .map(|l| format!("{}\n", encrypt::hooks(l)))
        .collect();
    if env.write_file(Path::new(f), &new).is_err() {
        return fail(env, ErrorCode::Failed, "could not write the hooks");
    }
    0
}
