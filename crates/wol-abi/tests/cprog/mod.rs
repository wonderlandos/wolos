// SPDX-License-Identifier: GPL-2.0-or-later
//! Compile a C test program against `wol.h` and the built `libwol.so.1`, and run it with the
//! sockets of a fake wold and Kit host.
#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh directory for one test.
pub fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-cprog-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// `tests/c/<name>.c` -> `dir/<name>`, strict warnings, linked by soname.
pub fn build(name: &str, dir: &Path, lib: &Path) -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exe = dir.join(name);
    let cc = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()))
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-pedantic"])
        .arg("-I")
        .arg(manifest.join("include"))
        .arg(manifest.join(format!("tests/c/{name}.c")))
        .arg(format!("-L{}", lib.display()))
        .arg("-lwol")
        .arg(format!("-Wl,-rpath,{}", lib.display()))
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("cc");
    assert!(
        cc.status.success(),
        "cc: {}",
        String::from_utf8_lossy(&cc.stderr)
    );
    exe
}

/// Run with `dir/wold.sock` and `dir/kit.sock`; stdout lines, after checking the exit status.
pub fn run(exe: &Path, dir: &Path) -> Vec<String> {
    let out: Output = Command::new(exe)
        .env("WOL_SOCKET", dir.join("wold.sock"))
        .env("WOL_KIT_SOCKET", dir.join("kit.sock"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout.lines().map(str::to_owned).collect()
}
