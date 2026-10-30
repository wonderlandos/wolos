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
    let Some(expected) = env.read_file(Path::new(&seal::digest_file(slot))) else {
        return 0; // never sealed (a system from before the seal): nothing to compare
    };
    let man = env
        .read_file(Path::new(&format!("/{}", seal::MANIFEST)))
        .map(|t| seal::Manifest::parse(&t))
        .unwrap_or_default();
    let mut cache = std::collections::HashMap::new();
    for p in man.0.keys() {
        cache.insert(p.clone(), measure(env, &Path::new("/").join(p)));
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
            "usage: wol encrypt decide auto|yes|no",
        );
    };
    let tpm_dir = env
        .var("WOLOS_TPM_DIR")
        .unwrap_or_else(|| "/sys/class/tpm/tpm0".into());
    let tpm = encrypt::has_tpm2(
        env.read_file(&Path::new(&tpm_dir).join("tpm_version_major"))
            .as_deref(),
    );
    let pass = env
        .var("WOLOS_PASSPHRASE_FILE")
        .and_then(|f| env.read_file(Path::new(&f)))
        .map(|p| p.trim_end_matches('\n').to_owned());
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

/// `wol encrypt enroll PART KEYFILE tpm2|passphrase [PASSFILE]`: the recovery key goes to
/// $WOLOS_RECOVERY_OUT (mode 600, shown once by the installer), never to a log.
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
        .unwrap_or_else(|| "/run/wolos/recovery-key".into());
    for (i, argv) in encrypt::enroll(part, key, unlock, inv.get("d"))
        .iter()
        .enumerate()
    {
        if i == 0 {
            let a: Vec<&str> = argv.iter().map(String::as_str).collect();
            let Some(rk) = env.capture(&a) else {
                return fail(env, ErrorCode::Failed, "could not create the recovery key");
            };
            if env.write_file(Path::new(&out), &rk).is_err() {
                return fail(env, ErrorCode::Failed, "could not keep the recovery key");
            }
            let _ =
                std::fs::set_permissions(&out, std::os::unix::fs::PermissionsExt::from_mode(0o600));
        } else if env.exec(argv).unwrap_or(1) != 0 {
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
