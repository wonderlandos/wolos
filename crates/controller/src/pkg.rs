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

fn real(env: &mut dyn Env, args: &[String]) -> i32 {
    let mut a = vec![up::PACMAN.to_owned()];
    a.extend(args.iter().cloned());
    env.exec(&a).unwrap_or(127)
}

const UNCHANGED: &str = "Nothing was changed. Try again, or run `wol update` first.";

pub fn run(env: &mut dyn Env, args: &[String]) -> i32 {
    let pkg::Kind::Change(c) = pkg::classify(args) else {
        return real(env, args);
    };
    let cmdline = env.read_file(Path::new("/proc/cmdline"));
    let ro = env.capture(&pkg::RO_ARGV);
    if !env.is_root() || !pkg::is_sealed(cmdline.as_deref(), ro.as_deref()) {
        return real(env, args);
    }
    let names = if c.verb == pkg::Verb::Upgrade {
        Vec::new()
    } else {
        capture(env, &pkg::names_argv(args))
            .map(|t| t.lines().map(|l| l.trim().to_owned()).collect())
            .unwrap_or_else(|| c.targets.clone())
    };
    let route = pkg::route(&c, &names);
    let ok = match route {
        pkg::Route::Layer => in_layer(env, args),
        pkg::Route::Slot => in_slot(env, args, cmdline.as_deref()),
    };
    if !ok {
        return fail(env, ErrorCode::Failed, UNCHANGED);
    }
    env.out(&format!("{}\n", pkg::done(&c, route)));
    0
}

fn in_layer(env: &mut dyn Env, args: &[String]) -> bool {
    let ls = capture(env, &["ls".into(), "-1".into(), layers::DIR.into()]).unwrap_or_default();
    let n = layers::next_index(&ls);
    let q = [up::PACMAN.to_owned(), "-Q".into()];
    let before = capture(env, &q).unwrap_or_default();
    let abort = |env: &mut dyn Env| {
        for a in layers::abort(n) {
            let _ = env.exec(&a);
        }
        false
    };
    for s in layers::begin(n, args) {
        if !run_step(env, &s) {
            return abort(env);
        }
    }
    let after = capture(env, &q).unwrap_or_default();
    for s in layers::finish(n, &layers::delta(&before, &after)) {
        if !run_step(env, &s) {
            return abort(env);
        }
    }
    true
}

fn in_slot(env: &mut dyn Env, args: &[String], cmdline: Option<&str>) -> bool {
    let Some(running) = cmdline.and_then(up::running_slot) else {
        return false;
    };
    let bes =
        up::be::parse(&capture(env, &up::be::LIST_ARGV.map(String::from)).unwrap_or_default());
    let Some((slot, from)) = pkg::base(&bes, running) else {
        return false;
    };
    let Some(steps) = pkg::plan(&slot, from, args, "current", cmdline) else {
        return false;
    };
    for s in &steps {
        if !run_step(env, s) {
            for r in up::rollback_from(&slot, from) {
                let _ = env.exec(&r);
            }
            return false;
        }
    }
    true
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
/// the new slot at ROOT, then clear its layers.
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
    let db = Path::new(inv.get("c").unwrap_or("/var/lib/pacman/local"));
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
    let mut folded: Vec<String> = std::fs::read_to_string(&folded_path)
        .map(|t| t.lines().map(str::to_owned).collect())
        .unwrap_or_default();
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
    let dest = root.join("var/lib/pacman/local");
    let _ = std::fs::create_dir_all(&dest);
    for d in drop {
        for e in std::fs::read_dir(&dest).into_iter().flatten().flatten() {
            if layers::pkg_name(&e.file_name().to_string_lossy()) == d {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    for d in add {
        let _ = std::fs::remove_dir_all(dest.join(&d));
        if copy_tree(&db.join(&d), &dest.join(&d)).is_err() {
            return fail(env, ErrorCode::Failed, "could not fold the package records");
        }
    }
    let _ = std::fs::create_dir_all(folded_path.parent().unwrap_or(root));
    if std::fs::write(&folded_path, folded.join("\n") + "\n").is_err() {
        return fail(env, ErrorCode::Failed, "could not record the folded layers");
    }
    for n in names {
        let _ = std::fs::remove_dir_all(root.join(layers::DIR.trim_start_matches('/')).join(n));
    }
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
