// SPDX-License-Identifier: BSD-2-Clause
//! Cryswol, the Crystal shard over libwol (wolos-crystal/shards/cryswol, ADR 0036), run from
//! `cargo test` against the libwol this test just built:
//! - `cryswol_specs_pass`: the shard's whole `crystal spec`, with the scripted fake wold and Kit host
//!   (`examples/wol_fake.rs`) and libwopkg for the opt-in package spec.
//! - `hello_in_crystal_highlevel`: the shard's `examples/hello.cr` against the same fake and checks
//!   as the C, Swift, .NET and raw Crystal `hello` (tests/common/fakes.rs `run_hello`).
//!
//! Skipped when `crystal` is not installed, unless `WOL_BINDINGS_REQUIRED=1`.
use std::path::{Path, PathBuf};
use std::process::Command;
mod common;

use common::fakes::run_hello;
use common::temp;

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The workspace root (the checkout).
fn root() -> PathBuf {
    manifest().join("../..")
}

fn shard() -> PathBuf {
    root().join("wolos-crystal/shards/cryswol")
}

/// `crystal`, or None when it is missing and not required.
fn crystal() -> Option<PathBuf> {
    let found = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|d| d.join("crystal"))
        .find(|p| p.is_file());
    if found.is_none() {
        assert!(
            std::env::var_os("WOL_BINDINGS_REQUIRED").is_none(),
            "crystal is not installed and WOL_BINDINGS_REQUIRED is set"
        );
        eprintln!("skipped: crystal not installed");
    }
    found
}

/// Run `cmd` and check it exited 0 (the test's assertion).
fn run_checked(cmd: &mut Command) {
    let out = cmd.output().unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
    assert!(
        out.status.success(),
        "{cmd:?}\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn link_flags(lib: &Path) -> String {
    format!("-L{0} -Wl,-rpath,{0}", lib.display())
}

/// target/<profile> of this test binary (target/<profile>/deps/<test>-<hash>).
fn profile_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().parent().unwrap().to_path_buf()
}

/// Build with the cargo running this test, into the same target directory and profile, so the
/// artifacts are shared with the test build.
fn cargo_build(args: &[&str]) {
    let profile = profile_dir();
    let target = profile.parent().unwrap();
    let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cmd.current_dir(root())
        .env("CARGO_TARGET_DIR", target)
        .arg("build")
        .args(args);
    match profile.file_name().and_then(|n| n.to_str()) {
        Some("debug") => {}
        Some("release") => {
            cmd.arg("--release");
        }
        Some(other) => {
            cmd.args(["--profile", other]);
        }
        None => panic!("no profile directory"),
    }
    run_checked(&mut cmd);
}

/// `examples/wol_fake`, built for this run.
fn fake() -> PathBuf {
    cargo_build(&["-p", "wol-abi", "--example", "wol_fake"]);
    profile_dir().join("examples/wol_fake")
}

/// libwopkg.so.1 next to libwol.so.1 in `dir`, for the opt-in `cryswol/pkg` spec.
fn link_wopkg(dir: &Path) {
    cargo_build(&["-p", "wopkg"]);
    run_checked(
        Command::new(root().join("crates/wopkg/link.sh"))
            .arg(profile_dir().join("libwopkg.a"))
            .arg(dir),
    );
}

#[test]
fn cryswol_specs_pass() {
    let Some(crystal) = crystal() else { return };
    let dir = temp("cryswol-spec");
    let lib = dir.join("lib");
    for f in ["libwol.so.1", "libwol.so"] {
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::copy(common::lib_dir().join(f), lib.join(f)).unwrap();
    }
    link_wopkg(&lib);
    let out = Command::new(crystal)
        .current_dir(shard())
        .env("CRYSTAL_CACHE_DIR", dir.join("cache"))
        .env("WOL_FAKE", fake())
        .args(["spec", "--error-on-warnings", "--link-flags"])
        .arg(link_flags(&lib))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains(" 0 failures, 0 errors"), "{stdout}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn hello_in_crystal_highlevel() {
    let Some(crystal) = crystal() else { return };
    let dir = temp("cryswol-hello");
    let exe = dir.join("hello");
    run_checked(
        Command::new(crystal)
            .env("CRYSTAL_CACHE_DIR", dir.join("cache"))
            .arg("build")
            .arg(shard().join("examples/hello.cr"))
            .args(["--error-on-warnings", "--link-flags"])
            .arg(link_flags(common::lib_dir()))
            .arg("-o")
            .arg(&exe),
    );
    run_hello(Command::new(exe), &dir);
    let _ = std::fs::remove_dir_all(dir);
}
