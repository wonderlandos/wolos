// SPDX-License-Identifier: BSD-2-Clause
//! Language bindings for the C ABIs (ADR 0020, ADR 0036, docs/modules/abi.md "Other languages"): Swift,
//! Crystal and .NET. Guards against drift:
//! - The Crystal `lib`s for `wol.h`, `wopkg.h`, `wol_realtime.h` and `wolkit.h` and the .NET P/Invoke
//!   class are generated from the headers (`common/bindgen.rs`) and must equal the checked-in files,
//!   in every place they are kept (the conformance programs, the wopkg CLI, the cryswol and wolkit
//!   shards). Swift imports
//!   the header itself through its module map. A header change not carried into the bindings fails here.
//! - Every header function has a wrapper in a high layer (the shard, the wopkg CLI) or is listed in
//!   `EXEMPT` with a reason: a new libwol call costs a wrapper in the same change (ADR 0036).
//! - Each language's `hello` and `parts` programs are built and run against the fake wold and Kit host
//!   (the same ones the C programs use) and must send and print the same things; with Crystal present,
//!   the generated files pass `crystal tool format --check` and the links spec links every function.
//!
//! A new ABI version: add the declarations to `wol.h`, run `WOL_BLESS=1 cargo test -p wol-abi --test
//! bindings` to regenerate, then extend `services` in each language for the new calls (`common/services.rs` checks them).
//! A language whose toolchain is not installed is skipped unless `WOL_BINDINGS_REQUIRED=1` (every
//! toolchain) or lists it (`WOL_BINDINGS_REQUIRED=crystal` on a box with only Crystal).
use std::path::{Path, PathBuf};
use std::process::Command;
mod common;

use common::bindgen::{self, Target};
use common::fakes::{run_hello, run_parts};
use common::services::run_services;
use common::temp;
fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The `wolos` repository root (this crate is `crates/wol-abi`).
fn root() -> PathBuf {
    manifest().join("../..")
}

/// The Crystal shard (ADR 0036): `require "wol"` loads `src/wol/lib_wol.cr` and the high layer.
const SHARD: &str = "wolos-crystal/shards/cryswol";

const WOL: Target = Target {
    header: "crates/wol-abi/include/wol.h",
    prefix: "wol_",
    lib: "LibWol",
    link: Some("wol"),
    license: "BSD-2-Clause",
};
const WOPKG: Target = Target {
    header: "crates/wopkg/include/wopkg.h",
    prefix: "wopkg_",
    lib: "LibWopkg",
    link: Some("wopkg"),
    license: "GPL-2.0-or-later",
};
/// The UI engine (ADR 0039, plan-wolkit 5.5): its high layer is the wolkit shard.
const WOLKIT: Target = Target {
    header: "crates/wolkit/include/wolkit.h",
    prefix: "wolkit_",
    lib: "LibWolkit",
    link: Some("wolkit"),
    license: "BSD-2-Clause",
};
/// The wolkit shard, which wraps `LibWolkit`.
const WOLKIT_SHARD: &str = "wolos-crystal/shards/wolkit";
/// Compositor-internal and only a static archive, so no `@[Link]`; opt-in (`require "cryswol/realtime"`).
const REALTIME: Target = Target {
    header: "crates/realtime/include/wol_realtime.h",
    prefix: "wol_",
    lib: "LibRealtime",
    link: None,
    license: "GPL-2.0-or-later",
};

/// Each header, its generated Crystal `lib`, and every place that file is kept (from the repo root).
/// Copies, not links: the tree also goes through checkouts without symlinks.
const CRYSTAL_LIBS: &[(Target, &[&str])] = &[
    (
        WOL,
        &[
            "crates/wol-abi/bindings/crystal/lib_wol.cr",
            "wolos-crystal/shards/cryswol/src/wol/lib_wol.cr",
        ],
    ),
    (
        WOPKG,
        &[
            "wopkg/src/wopkg/lib_wopkg.cr",
            "wolos-crystal/shards/cryswol/src/wol/pkg/lib_wopkg.cr",
        ],
    ),
    (
        REALTIME,
        &["wolos-crystal/shards/cryswol/src/wol/realtime/lib_realtime.cr"],
    ),
    (
        WOLKIT,
        &["wolos-crystal/shards/wolkit/src/wolkit/lib_wolkit.cr"],
    ),
];

/// The links spec, from the shard root; it requires the shard's `lib_wol.cr`.
const LINKS_SPEC: &str = "spec/generated/links_spec.cr";

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn header_of(t: &Target) -> bindgen::Header {
    bindgen::parse(&read(t.header))
}

fn header() -> bindgen::Header {
    header_of(&WOL)
}

/// Every `prefix*` symbol of every node in a linker version script.
fn map_symbols(rel: &str, prefix: &str) -> Vec<String> {
    read(rel)
        .lines()
        .map(|l| l.trim().trim_end_matches(';'))
        .filter(|l| l.starts_with(prefix))
        .map(String::from)
        .collect()
}

fn names(h: &bindgen::Header) -> Vec<String> {
    let mut n: Vec<String> = h.funcs.iter().map(|f| f.name.clone()).collect();
    n.sort();
    n
}

#[test]
fn the_header_model_has_every_exported_function() {
    let mut want = map_symbols("crates/wol-abi/abi/wol.map", "wol_");
    want.sort();
    assert_eq!(
        names(&header()),
        want,
        "wol.h and abi/wol.map disagree (or the header reader missed one)"
    );
}

#[test]
fn the_wopkg_header_model_has_every_exported_function() {
    let mut want = map_symbols("crates/wopkg/abi/wopkg.map", "wopkg_");
    want.sort();
    assert_eq!(
        names(&header_of(&WOPKG)),
        want,
        "wopkg.h and crates/wopkg/abi/wopkg.map disagree (or the header reader missed one)"
    );
}

#[test]
fn the_wolkit_header_model_has_every_exported_function() {
    let mut want = map_symbols("crates/wolkit/abi/wolkit.map", "wolkit_");
    want.sort();
    assert_eq!(
        names(&header_of(&WOLKIT)),
        want,
        "wolkit.h and crates/wolkit/abi/wolkit.map disagree (or the header reader missed one)"
    );
}

/// realtime has no version script: its exports are the `extern "C" fn`s of `src/ffi.rs`.
#[test]
fn the_realtime_header_model_has_every_exported_function() {
    let mut want: Vec<String> = read("crates/realtime/src/ffi.rs")
        .lines()
        .filter_map(|l| l.split_once("extern \"C\" fn ").map(|(_, r)| r))
        .map(|r| r.split('(').next().unwrap().trim().to_string())
        .filter(|n| n.starts_with("wol_"))
        .collect();
    want.sort();
    let h = header_of(&REALTIME);
    assert_eq!(names(&h), want, "wol_realtime.h and src/ffi.rs disagree");
    assert_eq!(h.structs.len(), 1, "{:?}", h.structs);
}

fn check_generated(rel: &str, want: &str) {
    let path = root().join(rel);
    if std::env::var_os("WOL_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        have == want,
        "{rel} is out of date with its header: run `WOL_BLESS=1 cargo test -p wol-abi --test bindings`"
    );
}

fn links_spec() -> String {
    header().crystal_links_spec(&WOL, "../../src/wol/lib_wol")
}

#[test]
fn crystal_declarations_match_the_header() {
    for (t, places) in CRYSTAL_LIBS {
        let want = header_of(t).crystal(t);
        for rel in *places {
            check_generated(rel, &want);
        }
    }
    check_generated(&format!("{SHARD}/{LINKS_SPEC}"), &links_spec());
}

#[test]
fn dotnet_declarations_match_the_header() {
    check_generated(
        "crates/wol-abi/bindings/dotnet/Wol.Native.cs",
        &header().dotnet(),
    );
}

/// Header functions that need no wrapper of their own, and why. The test fails if one gains a
/// wrapper (drop it here) so the list stays exact.
const EXEMPT: &[(&str, &str)] = &[
    // One Crystal object owns each handle and releases it once (from finalize or close); a second
    // reference is a second Crystal reference to the same object, not a C retain.
    ("wol_string_retain", "owned once, released by the wrapper"),
    ("wol_context_retain", "owned once, released by the wrapper"),
    (
        "wol_system_info_retain",
        "owned once, released by the wrapper",
    ),
    ("wol_window_retain", "owned once, released by the wrapper"),
    ("wol_widget_retain", "owned once, released by the wrapper"),
    (
        "wol_notification_retain",
        "owned once, released by the wrapper",
    ),
    ("wol_menu_retain", "owned once, released by the wrapper"),
    ("wol_toolbar_retain", "owned once, released by the wrapper"),
    ("wol_info_retain", "owned once, released by the wrapper"),
    ("wol_file_retain", "owned once, released by the wrapper"),
    (
        "wol_assertion_retain",
        "owned once, released by the wrapper",
    ),
    (
        "wol_search_provider_retain",
        "owned once, released by the wrapper",
    ),
    ("wol_endpoint_retain", "owned once, released by the wrapper"),
    (
        "wolkit_engine_retain",
        "owned once, released by the wrapper",
    ),
    (
        "wolkit_window_retain",
        "owned once, released by the wrapper",
    ),
    (
        "wolkit_style_table_retain",
        "owned once, released by the wrapper",
    ),
    ("wolkit_image_retain", "owned once, released by the wrapper"),
    (
        "wolkit_surface_retain",
        "owned once, released by the wrapper",
    ),
];

/// Whole headers with no high layer of their own: the raw `lib` is the API. Drift and parser tests
/// still cover them.
const RAW_ONLY: &[(&str, &str)] = &[(
    "LibRealtime",
    "compositor-internal, opt-in raw layer; its consumer is the compositor plugin",
)];

/// Functions the high layer does not wrap yet. CR1 is done, so libwol has none; a new header
/// function is either wrapped in the same change or listed in EXEMPT with a reason. It may only
/// shrink. The wolkit 1.0 calls below are the shard's next layer (version queries, images, canvas
/// streams and GPU surfaces), not wrapped in its first release.
const PENDING: &[&str] = &[
    "wolkit_version",
    "wolkit_version_string",
    "wolkit_engine_get_gpu_uuid",
    "wolkit_image_create",
    "wolkit_image_create_rgba",
    "wolkit_image_get_id",
    "wolkit_image_release",
    "wolkit_window_canvas_submit",
    "wolkit_surface_create",
    "wolkit_surface_release",
    "wolkit_surface_get_size",
    "wolkit_surface_get_vk_instance_extensions",
    "wolkit_surface_create_vk",
];

/// The high-layer Crystal sources that call `lib` (generated `lib_*.cr` files are not wrappers).
fn high_layer(lib: &str) -> Vec<(PathBuf, String)> {
    let mut dirs = vec![root().join(SHARD).join("src")];
    if lib == "LibWopkg" {
        dirs.push(root().join("wopkg/src"));
    }
    if lib == "LibWolkit" {
        dirs = vec![root().join(WOLKIT_SHARD).join("src")];
    }
    let mut out = Vec::new();
    while let Some(d) = dirs.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            if p.is_dir() {
                dirs.push(p);
            } else if n.ends_with(".cr") && !n.starts_with("lib_") {
                out.push((p.clone(), std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    out
}

/// `LibWol.wol_info_get` as a whole name (not the start of `LibWol.wol_info_get_count`).
fn calls(src: &str, lib: &str, name: &str) -> bool {
    let needle = format!("{lib}.{name}");
    src.match_indices(&needle).any(|(i, _)| {
        !src[i + needle.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

#[test]
fn every_header_function_has_a_crystal_wrapper_or_an_exemption() {
    let mut missing = Vec::new();
    let mut stale = Vec::new();
    for (t, _) in CRYSTAL_LIBS {
        if RAW_ONLY.iter().any(|(l, _)| *l == t.lib) {
            continue;
        }
        let srcs = high_layer(t.lib);
        for f in &header_of(t).funcs {
            let n = f.name.as_str();
            let wrapped = srcs.iter().any(|(_, s)| calls(s, t.lib, n));
            let listed = EXEMPT.iter().any(|(e, _)| *e == n) || PENDING.contains(&n);
            match (wrapped, listed) {
                (false, false) => missing.push(format!("{}.{n}", t.lib)),
                (true, true) => stale.push(n.to_string()),
                _ => {}
            }
        }
    }
    let known: Vec<String> = CRYSTAL_LIBS
        .iter()
        .flat_map(|(t, _)| names(&header_of(t)))
        .collect();
    for n in EXEMPT
        .iter()
        .map(|(n, _)| *n)
        .chain(PENDING.iter().copied())
    {
        assert!(
            known.iter().any(|k| k == n),
            "{n} is listed but in no header"
        );
    }
    assert!(
        missing.is_empty(),
        "header functions with no Crystal wrapper (wrap each in the shard's high layer in this \
         change, or list it in EXEMPT in tests/bindings.rs with a reason): {missing:?}"
    );
    assert!(
        stale.is_empty(),
        "these now have a wrapper: drop them from EXEMPT/PENDING in tests/bindings.rs: {stale:?}"
    );
}

#[test]
fn the_reader_takes_structs_suffixed_constants_and_wider_scalars() {
    let h = bindgen::parse(
        "#define A 1u\n#define B 0x100u\n#define C 3\n#define GUARD_H\n\
         typedef struct V {\n    float scale;\n    float opacity;\n} V;\n\
         /* split\n   comment */\n\
         int32_t f(uint32_t kind, uint64_t start,\n          V *out);\n\
         int32_t g(uint8_t *buf, size_t len);\nconst char *h(int32_t code);\n",
    );
    assert_eq!(
        h.consts,
        [("A", "1u"), ("B", "0x100u"), ("C", "3")].map(|(a, b)| (a.to_string(), b.to_string()))
    );
    assert_eq!(h.structs.len(), 1);
    assert_eq!(h.structs[0].0, "V");
    assert_eq!(h.structs[0].1.len(), 2);
    assert_eq!(names(&h), ["f", "g", "h"]);
    let t = Target {
        header: "test.h",
        prefix: "x_",
        lib: "LibX",
        link: None,
        license: "BSD-2-Clause",
    };
    let cr = h.crystal(&t);
    for want in [
        "  A =     1_u32\n  B = 0x100_u32\n  C =         3\n",
        "  struct V\n    scale : Float32\n    opacity : Float32\n  end\n",
        "  fun f(kind : UInt32, start : UInt64, out_ : V*) : Int32\n",
        "  fun g(buf : UInt8*, len : LibC::SizeT) : Int32\n",
        "  fun h(code : Int32) : UInt8*\n",
    ] {
        assert!(cr.contains(want), "{want:?} not in\n{cr}");
    }
    assert!(!cr.contains("@[Link(\""), "{cr}");
}

#[test]
fn constants_keep_their_c_type() {
    for (c, cr) in [
        ("1", "1"),
        ("-1", "-1"),
        ("1u", "1_u32"),
        ("0x100u", "0x100_u32"),
        ("0X1F", "0X1F"),
        ("8ull", "8_u64"),
        ("2L", "2_i64"),
    ] {
        assert_eq!(bindgen::crystal_literal(c), cr);
    }
    for bad in ["(1 << 2)", "1.5f", "FOO"] {
        assert!(
            std::panic::catch_unwind(|| bindgen::crystal_literal(bad)).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn an_unknown_type_names_the_table_to_extend() {
    let h = bindgen::parse("int32_t f(SomeLib x);");
    let t = Target {
        header: "other.h",
        prefix: "x_",
        lib: "LibX",
        link: Some("x"),
        license: "BSD-2-Clause",
    };
    let err = std::panic::catch_unwind(|| h.crystal(&t)).unwrap_err();
    let msg = err.downcast_ref::<String>().unwrap();
    assert!(
        msg.contains("SomeLib") && msg.contains("EXTERN_TYPES") && msg.contains("bindgen.rs"),
        "{msg}"
    );
    let v = bindgen::parse("int32_t f(VkInstance i, VkSurfaceKHR *s);").crystal(&t);
    assert!(v.contains("fun f(i : Void*, s : UInt64*) : Int32"), "{v}");
}

/// The pinned Crystal release, (major, minor), from `tools/crystal/toolchain.json` (RCR-046, the
/// one place the version lives).
fn crystal_pin() -> (u32, u32) {
    let path = root().join("tools/crystal/toolchain.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let pin = v["crystal"]
        .as_str()
        .expect("toolchain.json has a \"crystal\" string");
    let mut p = pin.split('.').map(|n| n.parse::<u32>().unwrap_or(0));
    (p.next().unwrap_or(0), p.next().unwrap_or(0))
}

/// The shard's minimum Crystal, the pin (S0 in plan-cr1.md: older compilers cannot take the
/// address of a `lib` fun with parameters, which the links spec does). `None` when Crystal is not
/// installed or older (skipped, or a failure under `WOL_BINDINGS_REQUIRED`).
fn crystal_for_shard() -> Option<PathBuf> {
    let exe = tool("crystal")?;
    let out = Command::new(&exe).arg("--version").output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let v: Vec<u32> = text
        .split_whitespace()
        .nth(1)
        .unwrap_or("0")
        .split('.')
        .map(|n| n.parse().unwrap_or(0))
        .collect();
    let (maj, min) = crystal_pin();
    if (v[0], v.get(1).copied().unwrap_or(0)) >= (maj, min) {
        return Some(exe);
    }
    assert!(
        !required("crystal"),
        "the cryswol shard needs Crystal >= {maj}.{min} (tools/crystal/toolchain.json), found {}",
        text.trim()
    );
    eprintln!("skipped: Crystal older than {maj}.{min}");
    None
}

#[test]
fn generated_crystal_is_formatted() {
    let Some(crystal) = tool("crystal") else {
        return;
    };
    let mut cmd = Command::new(crystal);
    cmd.args(["tool", "format", "--check"]);
    for (_, places) in CRYSTAL_LIBS {
        for rel in *places {
            cmd.arg(root().join(rel));
        }
    }
    run_checked(cmd.arg(root().join(SHARD).join(LINKS_SPEC)));
}

#[test]
fn the_links_spec_links_every_wol_function() {
    let Some(crystal) = crystal_for_shard() else {
        return;
    };
    let dir = temp("crystal-links");
    let lib = common::lib_dir();
    run_checked(
        Command::new(crystal)
            .env("CRYSTAL_CACHE_DIR", dir.join("cache"))
            .arg("spec")
            .arg(root().join(SHARD).join(LINKS_SPEC))
            .arg("--error-on-warnings")
            .arg("--link-flags")
            .arg(format!("-L{0} -Wl,-rpath,{0}", lib.display())),
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// libwopkg and librealtime are not built by this crate, so their `lib`s are type-checked (every
/// fun's address taken) without linking; the wopkg CLI specs link libwopkg for real.
#[test]
fn the_wopkg_and_realtime_libs_compile() {
    let Some(crystal) = crystal_for_shard() else {
        return;
    };
    let dir = temp("crystal-libs");
    for (t, places) in CRYSTAL_LIBS.iter().filter(|(t, _)| t.lib != "LibWol") {
        // A relative require: `require` does not take absolute paths.
        std::fs::copy(root().join(places[0]), dir.join("lib.cr")).unwrap();
        let spec = dir.join(format!("{}_spec.cr", t.lib));
        std::fs::write(&spec, header_of(t).crystal_links_spec(t, "./lib")).unwrap();
        run_checked(
            Command::new(&crystal)
                .env("CRYSTAL_CACHE_DIR", dir.join("cache"))
                .args(["build", "--no-codegen", "--error-on-warnings"])
                .arg(&spec),
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn swift_module_map_imports_the_header() {
    let m = std::fs::read_to_string(manifest().join("bindings/swift/module.modulemap")).unwrap();
    assert!(
        m.contains("module Wol") && m.contains("header \"wol.h\""),
        "{m}"
    );
}

/// `WOL_BINDINGS_REQUIRED=1` requires every toolchain; a list (`crystal,swiftc`) only those tools.
fn required(name: &str) -> bool {
    std::env::var("WOL_BINDINGS_REQUIRED")
        .is_ok_and(|v| v == "1" || v.split(',').any(|t| t.trim() == name))
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
            !required(name),
            "{name} is not installed and WOL_BINDINGS_REQUIRED asks for it"
        );
        eprintln!("skipped: {name} not installed");
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

fn bindings(lang: &str) -> PathBuf {
    manifest().join("bindings").join(lang)
}

/// Build `<lang>/<program>` into `dir` and return the command that runs it.
type Builder = fn(program: &str, dir: &Path, lib: &Path) -> Command;

fn crystal(program: &str, dir: &Path, lib: &Path) -> Command {
    let exe = dir.join(program);
    run_checked(
        Command::new(tool("crystal").unwrap())
            .env("CRYSTAL_CACHE_DIR", dir.join("cache"))
            .arg("build")
            .arg(bindings("crystal").join(format!("{program}.cr")))
            .arg("--error-on-warnings")
            .arg("--link-flags")
            .arg(format!("-L{0} -Wl,-rpath,{0}", lib.display()))
            .arg("-o")
            .arg(&exe),
    );
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
    run_checked(
        Command::new(tool("swiftc").unwrap())
            .args(["-swift-version", "5", "-static-stdlib", "-I"])
            .arg(&inc)
            .arg(bindings("swift").join(format!("{program}.swift")))
            .arg(format!("-L{}", lib.display()))
            .args(["-lwol", "-Xlinker", "-rpath", "-Xlinker"])
            .arg(lib)
            .arg("-o")
            .arg(&exe),
    );
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
    run_checked(
        Command::new(tool("dotnet").unwrap())
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
            .env("DOTNET_NOLOGO", "1")
            .env("DOTNET_CLI_HOME", dir.join("home"))
            .arg("build")
            .arg(src.join(program).join(format!("{program}.csproj")))
            .args(["-c", "Release", "-warnaserror", "-o"])
            .arg(&out),
    );
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
