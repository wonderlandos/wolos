// SPDX-License-Identifier: GPL-2.0-or-later
//! `pacman` on a sealed system (`/usr/local/bin/pacman`, also `wol pkg -- ARGS`). Queries go
//! straight to the real pacman. A change is made in a layer that is live at once; kernel,
//! initramfs and init packages are made in the pending slot instead. The decisions live in
//! `wol_core::update::{pkg, layers}`; `wol layers fold` runs from the system update.
use crate::cli::Invocation;
use crate::run::{Env, fail};
use shared::errors::ErrorCode;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;
use wol_core::update::{self as up, Step, layers, pkg};

fn run_step(env: &mut dyn Env, s: &Step) -> bool {
    match s {
        Step::Run(a) => env.exec(a).unwrap_or(127) == 0,
        Step::Write { path, text } => env.write_file(Path::new(path), text).is_ok(),
    }
}

fn capture(env: &mut dyn Env, argv: &[String]) -> Option<String> {
    let a: Vec<&str> = argv.iter().map(String::as_str).collect();
    env.capture(&a)
}

/// The real pacman; one that cannot start at all (missing or not executable in a layer chroot or
/// a broken slot) says so instead of a bare 127 (QA-SHADOW-44).
fn real(env: &mut dyn Env, args: &[String]) -> i32 {
    let mut a = vec![up::PACMAN.to_owned()];
    a.extend(args.iter().cloned());
    match env.exec(&a) {
        Ok(code) => code,
        Err(e) => {
            env.err(&format!(
                "wol pkg: the package manager ({}) could not be started: {e}. Nothing was changed.\n",
                up::PACMAN
            ));
            127
        }
    }
}

const UNCHANGED: &str = "Nothing was changed. Try again, or run `wol update` first.";
/// A stacked change that failed past its swap (audit8 stability2 #4): the restart-pending slot
/// went with it.
const PENDING_LOST: &str = "Nothing was changed, and the change that waited for a restart was \
                            removed with it. Run it again, or run `wol update` first.";

pub fn run(env: &mut dyn Env, args: &[String]) -> i32 {
    let pkg::Kind::Change(c) = pkg::classify(args) else {
        return real(env, args);
    };
    let cmdline = env.read_file(Path::new("/proc/cmdline"));
    let ro = env.capture(&pkg::RO_ARGV);
    if !env.is_root() || !pkg::is_sealed(cmdline.as_deref(), ro.as_deref()) {
        return real(env, args);
    }
    let _lock = match crate::run::update_lock(env) {
        Ok(l) => l,
        Err(code) => return code,
    };
    let names = if c.verb == pkg::Verb::Upgrade {
        Vec::new()
    } else {
        capture(env, &pkg::names_argv(args))
            .map(|t| t.lines().map(|l| l.trim().to_owned()).collect())
            .unwrap_or_else(|| c.targets.clone())
    };
    let route = pkg::route(&c, &names);
    let outcome = match route {
        pkg::Route::Layer => in_layer(env, args),
        pkg::Route::Slot => in_slot(env, args, cmdline.as_deref()),
    };
    let shown = match outcome {
        Outcome::Unchanged => return fail(env, ErrorCode::Failed, UNCHANGED),
        Outcome::PendingLost => return fail(env, ErrorCode::Failed, PENDING_LOST),
        Outcome::Done => route,
        // pacman committed into the layer; it merges at the next boot (CORE-5).
        Outcome::AtRestart => pkg::Route::Slot,
    };
    env.out(&format!("{}\n", pkg::done(&c, shown)));
    0
}

/// How a change ended.
enum Outcome {
    /// Nothing was changed.
    Unchanged,
    Done,
    /// pacman committed, the layer could not be merged now: it is kept and merges at boot.
    AtRestart,
    /// Nothing was changed, and the restart-pending slot the change stacked onto is gone.
    PendingLost,
}

/// The live system's package database directory (`DBPath` of `/etc/pacman.conf`).
fn live_db(env: &dyn Env) -> String {
    layers::db_path(
        &env.read_file(Path::new("/etc/pacman.conf"))
            .unwrap_or_default(),
    )
}

/// The booted slot's folded list: its sealed /usr copy, else (a slot from before QA-STORE-44)
/// the one in its /etc.
fn booted_folded(env: &dyn Env) -> String {
    let usr = env.read_file(&Path::new("/").join(layers::FOLDED));
    let etc = env.read_file(&Path::new("/").join(layers::ETC_FOLDED));
    layers::folded_list(usr.as_deref(), etc.as_deref()).to_owned()
}

fn in_layer(env: &mut dyn Env, args: &[String]) -> Outcome {
    // audit8 stability2 #8: a run that stopped at `umount -R` left the chroot mounted (the lock
    // says no other run uses it); never stack another rbind on it.
    let mounted = |env: &dyn Env| {
        env.read_file(Path::new("/proc/self/mountinfo"))
            .is_some_and(|m| layers::chroot_mounted(&m))
    };
    if mounted(env) && (env.exec(&layers::unmount_chroot()).unwrap_or(127) != 0 || mounted(env)) {
        env.err(&format!(
            "wol pkg: {} is still mounted from an earlier change\n",
            layers::CHROOT
        ));
        return Outcome::Unchanged;
    }
    let ls = capture(env, &["ls".into(), "-1".into(), layers::DIR.into()]).unwrap_or_default();
    let counter = env
        .read_file(Path::new(layers::COUNTER))
        .unwrap_or_default();
    let folded = booted_folded(env);
    // R2C-1: a number is never handed out twice, even after `wol layers clear` removed it.
    let n = layers::next_index(&ls, &counter, &folded);
    let before = capture(env, &[up::PACMAN.to_owned(), "-Q".into()]).unwrap_or_default();
    for (i, s) in layers::begin(n, args).iter().enumerate() {
        // The first step creates /var/lib/wolos/layers; record the number right after it.
        let ok = run_step(env, s)
            && (i != 0
                || env
                    .write_file(
                        Path::new(layers::COUNTER),
                        &format!(
                            "{n}
"
                        ),
                    )
                    .is_ok());
        if !ok {
            for a in layers::abort(n) {
                let _ = env.exec(&a);
            }
            return Outcome::Unchanged;
        }
    }
    // Still mounted: the chroot shows the layer's database even under /usr (CORE-2).
    let after_argv = layers::after_query(&live_db(env));
    let after = capture(env, &after_argv).unwrap_or_default();
    // pacman has committed: from here on the layer is the change and is never removed (CORE-5).
    for s in layers::finish(n, &layers::delta(&before, &after)) {
        if !run_step(env, &s) {
            env.err(&format!(
                "wol pkg: {s} did not finish; the layer merges at the next boot\n"
            ));
            return Outcome::AtRestart;
        }
    }
    Outcome::Done
}

/// The same slot change Update Now makes (`pkg::slot_change`, wol-admind); a failed step undoes
/// what it half made (`up::rollback_at`: a stacked change that already swapped the pending slot
/// loses it).
fn in_slot(env: &mut dyn Env, args: &[String], cmdline: Option<&str>) -> Outcome {
    let bootctl = capture(env, &up::be::LIST_ARGV.map(String::from));
    let Some(c) = pkg::slot_change(cmdline, bootctl.as_deref(), args) else {
        return Outcome::Unchanged;
    };
    let (slot, from, steps) = (&c.slot, c.from, &c.steps);
    for (i, s) in steps.iter().enumerate() {
        if !run_step(env, s) {
            let undo = up::rollback_at(slot, from, steps, i);
            for r in &undo.argv {
                let _ = env.exec(r);
            }
            if undo.done {
                env.err(&format!("{}\n", pkg::INSTALLED_NOT_DEFAULT));
                return Outcome::Done;
            }
            return if undo.kept || from != up::Base::Pending {
                Outcome::Unchanged
            } else {
                Outcome::PendingLost
            };
        }
    }
    Outcome::Done
}

/// `wol seal off`: developer mode.
pub fn seal_off(env: &mut dyn Env) -> i32 {
    if !env.is_root() {
        return fail(
            env,
            ErrorCode::Denied,
            "Developer mode needs the administrator.",
        );
    }
    let cmdline = env.read_file(Path::new("/proc/cmdline"));
    let Some(slot) = cmdline.as_deref().and_then(up::running_slot) else {
        return fail(env, ErrorCode::Failed, "This system is not sealed.");
    };
    for a in pkg::unseal_steps(slot) {
        if env.exec(&a).unwrap_or(1) != 0 {
            return fail(env, ErrorCode::Failed, "Could not turn the seal off.");
        }
    }
    env.err(&format!("{}\n", pkg::UNSEAL_WARNING));
    0
}

/// `wol layers fold ROOT [LAYERS [DB]]`: copy the live layers' files and package records into
/// the new slot at ROOT and name them in its folded list. The live layers stay (the running slot
/// still uses them); `wol layers clear` removes them once ROOT boots (CORE-1). DB defaults to the
/// live `DBPath`; the records land in ROOT's own `DBPath`, both read from pacman.conf (CORE-2).
pub fn fold(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let Some(root) = inv.get("a").map(Path::new) else {
        return fail(
            env,
            ErrorCode::Usage,
            "usage: wol layers fold ROOT [LAYERS [DB]]",
        );
    };
    if !env.is_root() {
        return fail(env, ErrorCode::Denied, "fold needs root");
    }
    let src = Path::new(inv.get("b").unwrap_or(layers::DIR));
    let live_local = format!("{}/local", live_db(env));
    let db = Path::new(inv.get("c").unwrap_or(&live_local));
    let names = layers::ordered(
        std::fs::read_dir(src)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default(),
    );
    let folded_path = root.join(layers::FOLDED);
    let mut folded: Vec<String> = Vec::new();
    for f in [layers::FOLDED, layers::ETC_FOLDED, layers::LEGACY_FOLDED] {
        for n in std::fs::read_to_string(root.join(f))
            .unwrap_or_default()
            .lines()
        {
            if layers::is_layer_name(n) && !folded.iter().any(|x| x == n) {
                folded.push(n.to_owned());
            }
        }
    }
    let mut recs = Vec::new();
    let todo: Vec<&String> = names.iter().filter(|n| !folded.contains(n)).collect();
    for n in todo {
        let dir = src.join(n);
        for top in ["usr", "opt"] {
            if copy_tree(&dir.join(top), &root.join(top)).is_err() {
                return fail(
                    env,
                    ErrorCode::Failed,
                    "could not fold the installed packages",
                );
            }
        }
        recs.push(layers::Record::parse(
            &std::fs::read_to_string(dir.join(layers::RECORD)).unwrap_or_default(),
        ));
        folded.push(n.clone());
    }
    let (add, drop) = layers::db_changes(&recs);
    let root_conf = std::fs::read_to_string(root.join("etc/pacman.conf")).unwrap_or_default();
    let dest = root.join(layers::local_db(&root_conf).trim_start_matches('/'));
    let _ = std::fs::create_dir_all(&dest);
    for d in drop {
        for e in std::fs::read_dir(&dest).into_iter().flatten().flatten() {
            if layers::pkg_name(&e.file_name().to_string_lossy()) == d {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    for d in add {
        // One record per package: the slot's older version of it goes.
        for e in std::fs::read_dir(&dest).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n != d && layers::pkg_name(&n) == layers::pkg_name(&d) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
        // A database under /usr came along with the layer's files already.
        if !db.join(&d).is_dir() {
            continue;
        }
        let _ = std::fs::remove_dir_all(dest.join(&d));
        if copy_tree(&db.join(&d), &dest.join(&d)).is_err() {
            return fail(env, ErrorCode::Failed, "could not fold the package records");
        }
    }
    let _ = std::fs::create_dir_all(folded_path.parent().unwrap_or(root));
    // Audit S4-10: a kill mid-write never leaves a truncated list (CORE-1, R2C-1 inputs).
    let list = folded.join("\n") + "\n";
    let mode = crate::run::kept_mode(&folded_path);
    if shared::atomic::atomic_write(&folded_path, mode, list.as_bytes()).is_err() {
        return fail(env, ErrorCode::Failed, "could not record the folded layers");
    }
    0
}

/// The other slot's folded list (the restore point's), read through the btrfs top level; None
/// when there is no other slot or it cannot be read (then nothing kept is removed).
fn other_folded(env: &dyn Env) -> Option<String> {
    let cmdline = env.read_file(Path::new("/proc/cmdline"))?;
    let other = up::running_slot(&cmdline).and_then(up::other_slot)?;
    let root = Path::new(up::TOP).join(format!("@{other}"));
    let usr = env.read_file(&root.join(layers::FOLDED));
    let etc = env.read_file(&root.join(layers::ETC_FOLDED));
    (usr.is_some() || etc.is_some())
        .then(|| layers::folded_list(usr.as_deref(), etc.as_deref()).to_owned())
}

/// `wol layers clear`: on the slot that just booted, remove the live layers it already holds
/// (its folded list), so they stop merging over its newer `/usr` (CORE-1). Nothing to clear: 0.
/// Writes no file: the list is the slot's, sealed in its /usr (QA-STORE-44).
pub fn clear(env: &mut dyn Env) -> i32 {
    if !env.is_root() {
        return fail(env, ErrorCode::Denied, "clear needs root");
    }
    let folded = booted_folded(env);
    let ls = capture(env, &["ls".into(), "-1".into(), layers::DIR.into()]).unwrap_or_default();
    // audit8 stability2 #3: layers pacman never finished in go too, only while no change runs
    // (the one being made has no record yet); held to the end so none starts meanwhile.
    let lock = env.try_lock(Path::new(up::LOCK)).ok().flatten();
    let mut unfinished = Vec::new();
    if lock.is_some() {
        let has = |n: &str| {
            env.read_file(&Path::new(layers::DIR).join(n).join(layers::RECORD))
                .is_some()
        };
        unfinished = layers::unfinished(&ls, has);
    }
    // QA-LIVE3-1: folded layers are kept (out of the merge) while the other slot, the restore
    // point, does not hold them; a slot that does not hold a kept one (a rollback) gets it back.
    // Names, from the folder itself: a missing folder is none kept.
    let kept: String = env
        .list_dir(Path::new(layers::KEPT_DIR))
        .iter()
        .filter_map(|p| {
            Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned() + "\n")
        })
        .collect();
    let other = other_folded(env);
    let tidy = layers::tidy(&folded, other.as_deref(), &ls, &kept, &unfinished);
    let steps = layers::tidy_steps(&tidy);
    let refresh = steps.iter().position(|s| *s == layers::refresh_step());
    for (i, a) in steps.iter().enumerate() {
        if env.exec(a).unwrap_or(127) != 0 {
            // Stopped before the refresh: merge what is in the extensions directory now, so the
            // merge matches it. At or after it: nothing more (the old merge may still use what
            // moved aside).
            if let Some(r) = refresh.filter(|r| i < *r) {
                let _ = env.exec(&steps[r]);
            }
            return fail(env, ErrorCode::Failed, "could not clear the folded layers");
        }
    }
    // The list stays as sealed (QA-STORE-44): a cleared name is gone from the extensions
    // directory and never handed out again (R2C-1, `layers::next_index`), so it matches nothing.
    0
}

/// Copy `from` over `to`; an overlay whiteout deletes the path instead.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    let Ok(rd) = std::fs::read_dir(from) else {
        return Ok(());
    };
    std::fs::create_dir_all(to)?;
    for e in rd.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        let md = std::fs::symlink_metadata(&src)?;
        let ft = md.file_type();
        if layers::is_whiteout(ft.is_char_device(), md.rdev()) {
            if dst.is_dir() && !dst.is_symlink() {
                std::fs::remove_dir_all(&dst)?;
            } else {
                let _ = std::fs::remove_file(&dst);
            }
        } else if ft.is_dir() {
            copy_tree(&src, &dst)?;
        } else if ft.is_symlink() {
            let _ = std::fs::remove_file(&dst);
            std::os::unix::fs::symlink(std::fs::read_link(&src)?, &dst)?;
        } else if ft.is_file() {
            let _ = std::fs::remove_file(&dst);
            std::fs::copy(&src, &dst)?;
            let _ = std::os::unix::fs::lchown(&dst, Some(md.uid()), Some(md.gid()));
        }
    }
    Ok(())
}

pub fn pkg_verb(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let args: Vec<String> = inv.all("args").iter().map(|s| (*s).to_owned()).collect();
    run(env, &args)
}
