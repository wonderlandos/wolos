// SPDX-License-Identifier: GPL-2.0-or-later
//! Language bindings for the C ABI (ADR 0020, archicture/modules/abi.md "Other languages"): Swift, Crystal
//! and .NET. Two guards against drift:
//! - The Crystal `lib LibWol` and the .NET P/Invoke class are generated from `include/wol.h`
//!   (`common/bindgen.rs`) and must equal the checked-in files; Swift imports the header itself through
//!   its module map. A header change that is not carried into the bindings fails here.
//! - Each language's `hello` and `parts` programs are built and run against the fake wold and Kit host
//!   (the same ones the C programs use) and must send and print the same things.
//!
//! A new ABI version: add the declarations to `wol.h`, run `WOL_BLESS=1 cargo test -p wol-abi --test
//! bindings` to regenerate, then extend `services` in each language for the new calls (`common/services.rs` checks them).
//! A language whose toolchain is not installed is skipped unless `WOL_BINDINGS_REQUIRED=1`.
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;
use common::bindgen;
use common::fakes::{run_hello, run_parts};
use common::services::run_services;
use common::temp;

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn header() -> bindgen::Header {
    bindgen::parse(&std::fs::read_to_string(manifest().join("include/wol.h")).unwrap())
}

/// Every symbol of every node in `abi/wol.map`.
fn map_symbols() -> Vec<String> {
    std::fs::read_to_string(manifest().join("abi/wol.map"))
        .unwrap()
        .lines()
        .map(|l| l.trim().trim_end_matches(';'))
        .filter(|l| l.starts_with("wol_"))
        .map(String::from)
        .collect()
}

#[test]
fn the_header_model_has_every_exported_function() {
    let mut have: Vec<String> = header().funcs.iter().map(|f| f.name.clone()).collect();
    let mut want = map_symbols();
    have.sort();
    want.sort();
    assert_eq!(
        have, want,
        "wol.h and abi/wol.map disagree (or the header reader missed one)"
    );
}

fn check_generated(rel: &str, want: String) {
    let path = manifest().join(rel);
    if std::env::var_os("WOL_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        have == want,
        "{rel} is out of date with include/wol.h: run `WOL_BLESS=1 cargo test -p wol-abi --test bindings`"
    );
}

#[test]
fn crystal_declarations_match_the_header() {
    check_generated("bindings/crystal/lib_wol.cr", header().crystal());
}

#[test]
fn dotnet_declarations_match_the_header() {
    check_generated("bindings/dotnet/Wol.Native.cs", header().dotnet());
}

#[test]
fn swift_module_map_imports_the_header() {
    let m = std::fs::read_to_string(manifest().join("bindings/swift/module.modulemap")).unwrap();
    assert!(
        m.contains("module Wol") && m.contains("header \"wol.h\""),
        "{m}"
    );
}

/// The program must exist, or the toolchain is missing and the language is skipped.
fn tool(name: &str) -> Option<PathBuf> {
    let found = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .chain(std::env::var_os("WOL_TOOLS").map(PathBuf::from))
        .map(|d| d.join(name))
        .find(|p| p.is_file());
    if found.is_none() {
        assert!(
            std::env::var_os("WOL_BINDINGS_REQUIRED").is_none(),
            "{name} is not installed and WOL_BINDINGS_REQUIRED is set"
        );
        eprintln!("skipped: {name} not installed");
    }
    found
}

fn run(cmd: &mut Command) {
    let out = cmd.output().unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
    assert!(
        out.status.success(),
        "{cmd:?}\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn bindings(lang: &str) -> PathBuf {
    manifest().join("bindings").join(lang)
}

/// Build `<lang>/<program>` into `dir` and return the command that runs it.
type Builder = fn(program: &str, dir: &Path, lib: &Path) -> Command;

fn crystal(program: &str, dir: &Path, lib: &Path) -> Command {
    let exe = dir.join(program);
    run(Command::new(tool("crystal").unwrap())
        .env("CRYSTAL_CACHE_DIR", dir.join("cache"))
        .arg("build")
        .arg(bindings("crystal").join(format!("{program}.cr")))
        .arg("--error-on-warnings")
        .arg("--link-flags")
        .arg(format!("-L{0} -Wl,-rpath,{0}", lib.display()))
        .arg("-o")
        .arg(&exe));
    Command::new(exe)
}

fn swift(program: &str, dir: &Path, lib: &Path) -> Command {
    let inc = dir.join("include");
    std::fs::create_dir_all(&inc).unwrap();
    std::fs::copy(manifest().join("include/wol.h"), inc.join("wol.h")).unwrap();
    std::fs::copy(
        bindings("swift").join("module.modulemap"),
        inc.join("module.modulemap"),
    )
    .unwrap();
    let exe = dir.join(program);
    run(Command::new(tool("swiftc").unwrap())
        .args(["-swift-version", "5", "-static-stdlib", "-I"])
        .arg(&inc)
        .arg(bindings("swift").join(format!("{program}.swift")))
        .arg(format!("-L{}", lib.display()))
        .args(["-lwol", "-Xlinker", "-rpath", "-Xlinker"])
        .arg(lib)
        .arg("-o")
        .arg(&exe));
    Command::new(exe)
}

fn dotnet(program: &str, dir: &Path, lib: &Path) -> Command {
    let src = dir.join("src");
    let status = Command::new("cp")
        .arg("-r")
        .arg(bindings("dotnet"))
        .arg(&src)
        .status()
        .unwrap();
    assert!(status.success());
    let out = dir.join("out");
    run(Command::new(tool("dotnet").unwrap())
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_HOME", dir.join("home"))
        .arg("build")
        .arg(src.join(program).join(format!("{program}.csproj")))
        .args(["-c", "Release", "-warnaserror", "-o"])
        .arg(&out));
    let mut c = Command::new(tool("dotnet").unwrap());
    c.arg(out.join(format!("{program}.dll")))
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("LD_LIBRARY_PATH", lib);
    c
}

fn each(lang: &str, program: &str, build: Builder, check: fn(Command, &Path)) {
    let dir = temp(&format!("{lang}-{program}"));
    // The runtime tools run only when the toolchain exists.
    let want = match lang {
        "swift" => "swiftc",
        "dotnet" => "dotnet",
        _ => "crystal",
    };
    if tool(want).is_none() {
        return;
    }
    let cmd = build(program, &dir, common::lib_dir());
    check(cmd, &dir);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn hello_in_crystal() {
    each("crystal", "hello", crystal, run_hello);
}

#[test]
fn parts_in_crystal() {
    each("crystal", "parts", crystal, run_parts);
}

#[test]
fn hello_in_swift() {
    each("swift", "hello", swift, run_hello);
}

#[test]
fn parts_in_swift() {
    each("swift", "parts", swift, run_parts);
}

#[test]
fn hello_in_dotnet() {
    each("dotnet", "hello", dotnet, run_hello);
}

#[test]
fn parts_in_dotnet() {
    each("dotnet", "parts", dotnet, run_parts);
}

#[test]
fn services_in_crystal() {
    each("crystal", "services", crystal, run_services);
}

#[test]
fn services_in_swift() {
    each("swift", "services", swift, run_services);
}

#[test]
fn services_in_dotnet() {
    each("dotnet", "services", dotnet, run_services);
}
