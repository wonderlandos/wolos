// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol seal apply|check` and `wol encrypt ...`: the installer and updater call these; the
//! decisions live in `wol_core::seal` and `wol_core::encrypt`. Secrets are files, never arguments.
use crate::cli::Invocation;
use crate::run::{Env, fail};
use shared::errors::ErrorCode;
use std::path::{Path, PathBuf};
use wol_core::{encrypt, seal, update};

/// Regular files and symlinks under `dir`, sorted; `true` marks a symlink (never followed).
fn walk(dir: &Path, out: &mut Vec<(PathBuf, bool)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut v: Vec<_> = rd.flatten().collect();
    v.sort_by_key(|e| e.file_name());
    for e in v {
        match e.file_type() {
            Ok(t) if t.is_symlink() => out.push((e.path(), true)),
            Ok(t) if t.is_dir() => walk(&e.path(), out),
            Ok(t) if t.is_file() => out.push((e.path(), false)),
            _ => {}
        }
    }
}

/// [`walk`] of [`seal::sealed_dirs`] under `root`, as paths relative to it. A directory that is
/// itself a symlink (usr/sbin on Arch) is one entry, never followed; a missing one is skipped.
fn slot_files(root: &Path) -> Vec<(String, PathBuf, bool)> {
    let mut files = Vec::new();
    for d in seal::sealed_dirs() {
        let p = root.join(d);
        match std::fs::symlink_metadata(&p) {
            Ok(m) if m.file_type().is_symlink() => files.push((p, true)),
            Ok(m) if m.is_dir() => walk(&p, &mut files),
            _ => {}
        }
    }
    files
        .into_iter()
        .filter_map(|(f, link)| {
            let rel = f.strip_prefix(root).ok()?.to_string_lossy().into_owned();
            Some((rel, f, link))
        })
        .collect()
}

/// [`seal::hash_field`] of a regular file (never through a symlink).
fn hash_of(path: &Path) -> Option<String> {
    let m = std::fs::symlink_metadata(path).ok()?;
    m.is_file()
        .then(|| std::fs::read(path).ok())
        .flatten()
        .map(|b| seal::hash_field(&b))
}

fn link_of(path: &Path) -> Option<String> {
    use std::os::unix::ffi::OsStrExt;
    std::fs::read_link(path)
        .ok()
        .map(|t| seal::link_field(t.as_os_str().as_bytes()))
}

/// (device, inode) of a regular file, never through a symlink: the identity fs-verity is kept on.
fn inode_of(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(path).ok()?;
    m.is_file().then(|| (m.dev(), m.ino()))
}

fn measure(env: &mut dyn Env, path: &Path) -> Option<String> {
    let argv = seal::measure_argv(&path.to_string_lossy());
    let a: Vec<&str> = argv.iter().map(String::as_str).collect();
    seal::parse_measure(&env.capture(&a)?)
}

/// fs-verity digests of `paths` (absolute), 512 to a process; a chunk whose run fails (a file
/// with no verity stops it) is asked one file at a time.
fn measure_all(env: &mut dyn Env, paths: &[PathBuf]) -> std::collections::HashMap<PathBuf, String> {
    let mut found = std::collections::HashMap::new();
    for chunk in paths.chunks(512) {
        let names: Vec<String> = chunk
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let argv = seal::measure_many_argv(&names);
        let a: Vec<&str> = argv.iter().map(String::as_str).collect();
        if let Some(out) = env.capture(&a) {
            for (p, d) in seal::parse_measures(&out) {
                found.insert(PathBuf::from(p), d);
            }
        } else {
            for p in chunk {
                if let Some(d) = measure(env, p) {
                    found.insert(p.clone(), d);
                }
            }
        }
    }
    found
}

/// `wol seal apply ROOT DIGESTFILE`: fs-verity on every file under the slot's
/// [`seal::VERITY_DIRS`], a content hash of every file under [`seal::HASHED_DIRS`], symlinks with
/// their targets, the complete manifest
/// inside the slot (QA-F11), its root digest at DIGESTFILE, then the subvolume read-only.
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
    let mut man = seal::Manifest::default();
    let mut sealed: Vec<(String, PathBuf)> = Vec::new();
    for (rel, f, link) in slot_files(root) {
        match (seal::covers(&rel), link) {
            (seal::Cover::Outside, _) => {}
            (_, true) => {
                let Some(l) = link_of(&f) else {
                    return fail(
                        env,
                        ErrorCode::Failed,
                        &format!("could not read the link {rel}"),
                    );
                };
                man.0.insert(rel, l);
            }
            (seal::Cover::Hashed, false) => {
                let Some(h) = hash_of(&f) else {
                    return fail(env, ErrorCode::Failed, &format!("could not read {rel}"));
                };
                man.0.insert(rel, h);
            }
            (seal::Cover::Verity, false) => sealed.push((rel, f)),
        }
    }
    let paths: Vec<PathBuf> = sealed.iter().map(|(_, f)| f.clone()).collect();
    let mut digests = measure_all(env, &paths);
    // fs-verity belongs to the inode, not the path: hard links (perl's perlbug and perlthanks)
    // are one file, enabled once; enabling it again fails with EEXIST (IMG013-SEAL-HARDLINK).
    let mut first_link: std::collections::HashMap<(u64, u64), PathBuf> =
        std::collections::HashMap::new();
    for (rel, f) in &sealed {
        // The first path seen of this inode; a later link takes its digest.
        let first = inode_of(f).map_or_else(
            || f.clone(),
            |i| first_link.entry(i).or_insert_with(|| f.clone()).clone(),
        );
        if let Some(d) = digests.get(&first).cloned() {
            digests.insert(f.clone(), d);
            continue;
        }
        let enabled = env
            .exec(&seal::enable_argv(&f.to_string_lossy()))
            .unwrap_or(1)
            == 0;
        // A failed enable on an inode that has verity already (EEXIST: sealed since it was
        // measured, through a link not told apart above) is sealed: its digest stands.
        let Some(d) = measure(env, f) else {
            let why = if enabled {
                format!("no fs-verity digest for {rel}")
            } else {
                format!("fs-verity could not be enabled on {rel}")
            };
            return fail(env, ErrorCode::Failed, &why);
        };
        digests.insert(f.clone(), d);
    }
    // Complete: every sealed path carries its digest, or nothing is sealed.
    for (rel, f) in sealed {
        let Some(d) = digests.get(&f) else {
            return fail(
                env,
                ErrorCode::Failed,
                &format!("no fs-verity digest for {rel}"),
            );
        };
        man.0.insert(rel, d.clone());
    }
    let mpath = root.join(seal::MANIFEST);
    if env.write_file(&mpath, &man.to_complete_text()).is_err()
        || env
            .write_file(
                Path::new(digest),
                &format!("{}\n", man.complete_root_digest()),
            )
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

/// The mark `wol seal check` leaves on a slot whose files did not match their seal.
pub(crate) const VERITY_MARK: &str = "verity-failed";
/// The mark `wol boot check` leaves on a slot older than the floor.
pub(crate) const EPOCH_MARK: &str = "epoch-failed";

/// `/var/lib/wolos/<kind>-<slot>`: that slot failed an early-boot check of that kind.
pub(crate) fn failed_mark(kind: &str, slot: &str) -> PathBuf {
    PathBuf::from(format!("/var/lib/wolos/{kind}-{slot}"))
}

/// The safety notice (`seal::NOTICE`) goes once its cause is gone (QA-LIVE3-16: it outlived the
/// failure for good). Two causes write it: a slot that failed an early-boot check (it carries a
/// failed mark until it boots and passes) and `wol boot bless` finding the other slot's image
/// damaged (no mark; the cause is that image, until it verifies again or is re-sealed). While
/// either holds, the notice stays.
pub(crate) fn clear_notice_if_healthy(env: &mut dyn Env) {
    let marked = update::SLOTS.into_iter().any(|slot| {
        [VERITY_MARK, EPOCH_MARK]
            .iter()
            .any(|k| env.read_file(&failed_mark(k, slot)).is_some())
    });
    if marked {
        return;
    }
    // Only a notice that is there needs the (costly) image check.
    if env.read_file(Path::new(seal::NOTICE)).is_some()
        && update::SLOTS.into_iter().any(|s| image_damaged(env, s))
    {
        return;
    }
    let _ = env.remove_file(Path::new(seal::NOTICE));
}

/// The other slot's menu entry the early-boot checks (`wol seal check`, `wol boot check`) may
/// restart into: found under any counter state with tries left (QA-SYS-12), its image on the ESP
/// and not empty (a ghost entry restarts into this slot again: BOOT2-2, QA-STORE-51), and that
/// slot failed neither check itself (two failing slots would restart into each other for ever,
/// audit core-install #4). None: stay.
pub(crate) fn fallback_entry(env: &mut dyn Env, other: &str) -> Option<String> {
    if other.is_empty()
        || [VERITY_MARK, EPOCH_MARK]
            .iter()
            .any(|k| env.read_file(&failed_mark(k, other)).is_some())
    {
        return None;
    }
    let dir = format!("{}/loader/entries", update::ESP);
    let listed: Vec<String> = env
        .list_dir(Path::new(&dir))
        .iter()
        .filter_map(|p| {
            Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .collect();
    let names: Vec<&str> = listed.iter().map(String::as_str).collect();
    let entry = seal::entry_of(other, &names)?.to_owned();
    let text = env.read_file(&Path::new(&dir).join(&entry))?;
    let image = format!("{}{}", update::ESP, seal::entry_image(&text)?);
    let parent = Path::new(&image).parent()?.to_path_buf();
    if !env.list_dir(&parent).contains(&image) {
        return None;
    }
    let size = env.capture(&["stat", "-c", "%s", &image]);
    if size.as_deref().map(str::trim) == Some("0") || image_damaged(env, other) {
        return None;
    }
    Some(entry)
}

/// The other slot's image is not the one esp-sync wrote: its digest (`<slot>/image.sha256` on
/// the ESP) does not match, or the image cannot be read. No digest kept (an image from before):
/// not known to be damaged.
pub(crate) fn image_damaged(env: &mut dyn Env, slot: &str) -> bool {
    let Some(want) = env
        .read_file(Path::new(&format!("{}/{slot}/image.sha256", update::ESP)))
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
    else {
        return false;
    };
    let image = format!("{}/EFI/wolos/{slot}.efi", update::ESP);
    let got = env
        .capture(&["sha256sum", &image])
        .and_then(|o| o.split_whitespace().next().map(str::to_owned));
    got.as_deref() != Some(want.as_str())
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
    // Tests point this at a slot on disk; at boot it is `/`, before any layer merges over /usr
    // (wolos-verity-check.service runs before systemd-sysext). Run later, with the layers merged,
    // the slot is looked at through a plain bind of `/`, which has no merge over its /usr:
    // through the merge every sealed file read as changed (QA-LIVE3-2).
    // The override is for tests only: a release build always checks the real slot (D8).
    let test_root = if cfg!(debug_assertions) {
        env.var("WOLOS_SEAL_ROOT")
    } else {
        None
    };
    let mut bound = false;
    let root = match test_root {
        Some(r) => PathBuf::from(r),
        None if layers_merged(env) => {
            if !bind_slot(env) {
                // Through the merge every sealed file reads as changed: no verdict, no mark, no
                // restart into the other slot (D7).
                env.err(
                    "wol seal check: the layers are merged over /usr and the slot could not be \
                     looked at under them; not checked\n",
                );
                return 1;
            }
            bound = true;
            PathBuf::from(SLOT_VIEW)
        }
        None => PathBuf::from("/"),
    };
    let code = check_at(env, slot, &expected, &root);
    if bound {
        let _ = env.exec(&["umount".to_owned(), SLOT_VIEW.to_owned()]);
    }
    code
}

/// Where `wol seal check` binds `/` (without the layers merged over /usr) when run late.
const SLOT_VIEW: &str = "/run/wolos/seal-root";

/// systemd-sysext has merged layers over /usr (an overlay mounted there).
fn layers_merged(env: &mut dyn Env) -> bool {
    // findmnt exits 1 when nothing is mounted on /usr (no layers); any other failure is said.
    match env.capture_status(&["findmnt", "-n", "-o", "FSTYPE", "/usr"]) {
        Some((0, t, _)) => t.trim() == "overlay",
        Some((1, _, _)) => false,
        other => {
            let why = other.map_or_else(
                || "it did not run".to_owned(),
                |(c, _, e)| format!("exit {c}: {}", e.trim()),
            );
            env.err(&format!(
                "wol seal: findmnt /usr failed ({why}): checked as if no layers were merged\n"
            ));
            false
        }
    }
}

/// A non-recursive bind of `/` at [`SLOT_VIEW`]: the slot's own /usr, nothing mounted over it.
fn bind_slot(env: &mut dyn Env) -> bool {
    let s = |a: &[&str]| a.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    env.exec(&s(&["mkdir", "-p", SLOT_VIEW])).unwrap_or(1) == 0
        && env
            .exec(&s(&["mount", "--bind", "/", SLOT_VIEW]))
            .unwrap_or(1)
            == 0
}

/// The seal check of the slot seen at `root` (see [`check`]).
fn check_at(env: &mut dyn Env, slot: &str, expected: &str, root: &Path) -> i32 {
    let root = root.to_path_buf();
    let text = env
        .read_file(&root.join(seal::MANIFEST))
        .unwrap_or_default();
    let (man, complete) = seal::Manifest::parse_text(&text);
    let paths: Vec<PathBuf> = man.verity_paths().iter().map(|p| root.join(p)).collect();
    let digests = measure_all(env, &paths);
    let measured = |p: &str| digests.get(&root.join(p)).cloned();
    let link = |p: &str| link_of(&root.join(p));
    let hashed = |p: &str| hash_of(&root.join(p));
    let present: Vec<String> = if complete {
        slot_files(&root)
            .into_iter()
            .map(|(rel, _, _)| rel)
            .collect()
    } else {
        Vec::new()
    };
    let seen = seal::Seen {
        measured: &measured,
        link: &link,
        hashed: &hashed,
        present: &present,
    };
    let v = seal::verify_text(&text, expected, &seen);
    if v == seal::Verdict::Ok {
        // A slot that passes has not failed: an old mark (a check that could not see every file)
        // would keep the other slot from ever falling back to this one.
        let _ = env.remove_file(&failed_mark(VERITY_MARK, slot));
        clear_notice_if_healthy(env);
        return 0;
    }
    // This slot failed before and is running again: the fallback did not take (the other one
    // did not start, or the person picked this one). Restarting again would loop (QA-STORE-51).
    let again = env.read_file(&failed_mark(VERITY_MARK, slot)).is_some();
    let other = update::other_slot(slot).unwrap_or("");
    let entry = if again {
        None
    } else {
        fallback_entry(env, other)
    };
    let fb = seal::fallback(slot, entry.as_deref());
    let _ = env.write_file(&failed_mark(VERITY_MARK, slot), "failed\n");
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
            // Audit S4-10: 0600 from the first byte (never umask, then chmod).
            if env.write_private(&typed, &p).is_err() {
                return fail(env, ErrorCode::Failed, "could not read the password file");
            }
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
            if env.write_private(Path::new(out), &rk).is_err() {
                return fail(env, ErrorCode::Failed, "could not keep the recovery key");
            }
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

/// Does this start's `rd.luks.options=<uuid>=...` ask the initramfs for the security chip
/// (`tpm2-device`, [`encrypt::cmdline_words`])? A passphrase-only install does not: the chip
/// must never be bound (or its absence reported as a failure) behind the person's choice.
fn unlocks_with_chip(env: &dyn Env, uuid: &str) -> bool {
    let Some(cmdline) = env.read_file(Path::new("/proc/cmdline")) else {
        return false;
    };
    let prefix = format!("rd.luks.options={uuid}=");
    cmdline
        .split_whitespace()
        .filter_map(|w| w.strip_prefix(prefix.as_str()))
        .any(|opts| opts.split(',').any(|o| o.starts_with("tpm2-device")))
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
/// start is automatic again. Nothing typed (the chip opened it) or an install that never used the
/// chip (no `tpm2-device` in `rd.luks.options`): nothing to do, exit 0.
pub fn reseal(env: &mut dyn Env) -> i32 {
    // Root's keyring and the disk are not a person's to touch: said, not a silent success
    // (QA-LIVE3-32: a user got rc 0 and no output).
    if !env.is_root() {
        return fail(
            env,
            ErrorCode::Denied,
            "only an administrator can bind the disk key",
        );
    }
    let Some(uuid) = luks_uuid(env) else {
        return 0;
    };
    // Passphrase-only install (audit I6-1): nothing to seal; the typed secret stays untouched.
    if !unlocks_with_chip(env, &uuid) {
        return 0;
    }
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
