// SPDX-License-Identifier: BSD-2-Clause
//! The stability guarantee of libWolkit (ADR 0039). A shipped symbol, declaration, `#define` or
//! property id never goes away or changes; new ones are only added, under a new version node.
//! - every symbol in `abi/wolkit-*.symbols` is exported by `libwolkit.so` with that version
//! - every exported symbol is in a baseline, the linker map and `include/wolkit.h`; nothing leaks
//! - every declaration of each `abi/wolkit-1.x.h` snapshot is still in `include/wolkit.h`
//! - `include/wolkit.h` prototypes and `#define`s match the Rust exports and constants exactly
//! - `abi/wolkit-1.0.props` still lists every property id with its name and type
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

mod common;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read(p: impl AsRef<Path>) -> String {
    std::fs::read_to_string(root().join(p.as_ref())).unwrap()
}

fn lib() -> PathBuf {
    common::lib_dir().join("libwolkit.so.1")
}

fn readelf(args: &[&str]) -> String {
    let out = std::process::Command::new("readelf").args(args).arg(lib()).env("LC_ALL", "C").output().expect("readelf");
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `name@VERSION` of every defined dynamic symbol, plus anything defined that is not `wolkit_`.
fn exported() -> (BTreeSet<String>, Vec<String>) {
    let (mut syms, mut stray) = (BTreeSet::new(), Vec::new());
    for line in readelf(&["--dyn-syms", "--wide"]).lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 8 || f[0] == "Num:" || f[6] == "UND" || !f[0].ends_with(':') {
            continue;
        }
        let name = f[7];
        if f[3] == "OBJECT" && f[6] == "ABS" {
            continue; // version node definitions (WOLKIT_1.0)
        }
        match name.split_once("@@").or_else(|| name.split_once('@')) {
            Some((n, v)) if n.starts_with("wolkit_") => {
                syms.insert(format!("{n}@{v}"));
            }
            _ => stray.push(name.to_owned()),
        }
    }
    (syms, stray)
}

fn baselines() -> BTreeSet<String> {
    let mut all = BTreeSet::new();
    for e in std::fs::read_dir(root().join("abi")).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "symbols") {
            for l in std::fs::read_to_string(&p).unwrap().lines() {
                let l = l.trim();
                if !l.is_empty() && !l.starts_with('#') {
                    all.insert(l.to_owned());
                }
            }
        }
    }
    all
}

fn names(syms: &BTreeSet<String>) -> BTreeSet<String> {
    syms.iter().map(|s| s.split('@').next().unwrap().to_owned()).collect()
}

#[test]
fn shipped_symbols_never_go_away() {
    let (exp, stray) = exported();
    let base = baselines();
    assert!(!base.is_empty());
    let missing: Vec<_> = base.difference(&exp).collect();
    assert!(missing.is_empty(), "removed or re-versioned: {missing:?}");
    let unrecorded: Vec<_> = exp.difference(&base).collect();
    assert!(
        unrecorded.is_empty(),
        "new symbols need a line in abi/wolkit-<next>.symbols and a WOLKIT_<next> node: {unrecorded:?}"
    );
    assert!(stray.is_empty(), "only wolkit_ symbols are exported: {stray:?}");
}

#[test]
fn library_is_named_for_its_major_version() {
    assert!(readelf(&["-d"]).contains("Library soname: [libwolkit.so.1]"));
}

#[test]
fn linker_map_lists_every_baseline_symbol() {
    let map = read("abi/wolkit.map");
    let listed: BTreeSet<String> = map
        .lines()
        .map(str::trim)
        .filter_map(|l| l.strip_suffix(';'))
        .filter(|l| l.starts_with("wolkit_"))
        .map(str::to_owned)
        .collect();
    assert_eq!(listed, names(&baselines()));
}

// ---- header <-> Rust ----

fn strip_comments(src: &str) -> String {
    let mut out = String::new();
    let mut rest = src;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        rest = rest[i..].split_once("*/").map_or("", |x| x.1);
    }
    out.push_str(rest);
    out.lines()
        .map(|l| l.split("//").next().unwrap())
        .filter(|l| {
            let t = l.trim();
            !t.starts_with('#') && t != "extern \"C\" {" && t != "}"
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn squash(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s.replace(" *", "*").replace("( ", "(").replace(" )", ")")
}

/// `const char *utf8` -> `const char*`; `float *xywh` -> `float*`; `wolkit_error **` -> `wolkit_error**`.
fn c_param(p: &str) -> String {
    let p = squash(p);
    if p == "void" {
        return String::new();
    }
    // A parameter name is an identifier after the type; types ending in `*` or one word have none.
    let words: Vec<&str> = p.split(' ').collect();
    let last = words.last().copied().unwrap_or("");
    let is_name = !last.ends_with('*') && words.len() > 1 && !matches!(last, "const" | "void") && {
        // `unsigned x` style types do not occur; a final identifier after a type is a name unless
        // the whole parameter is a bare typedef name like `wolkit_error*`.
        let before = words[..words.len() - 1].join(" ");
        before.ends_with('*') || matches!(before.as_str(), "uint8_t" | "uint32_t" | "uint64_t" | "int32_t" | "size_t" | "float" | "int" | "const char" | "void")
            || before.starts_with("const ")
    };
    if is_name {
        // strip a trailing name glued to `*` like `uint8_t *buf`
        return squash(&words[..words.len() - 1].join(" "));
    }
    // `const char *utf8` squashed to `const char*utf8`: cut a trailing identifier after `*`.
    if let Some(star) = p.rfind('*') {
        let tail = &p[star + 1..];
        if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return squash(&p[..=star]);
        }
    }
    p
}

/// name -> `ret(params)` from wolkit.h prototypes.
fn header_prototypes(h: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for stmt in strip_comments(h).split(';') {
        let s = squash(stmt);
        if s.starts_with("typedef") || s.starts_with("extern") || !s.ends_with(')') {
            continue;
        }
        let Some(open) = s.find('(') else { continue };
        let head = &s[..open];
        let name_at = head.rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map_or(0, |i| i + 1);
        let (ret, name) = (squash(&head[..name_at]), head[name_at..].to_owned());
        if !name.starts_with("wolkit_") {
            continue;
        }
        let params: Vec<String> = s[open + 1..s.len() - 1].split(',').map(c_param).filter(|p| !p.is_empty()).collect();
        m.insert(name, format!("{ret}({})", params.join(",")));
    }
    m
}

fn c_type(rust: &str) -> String {
    let t = rust.trim().trim_start_matches("std::ffi::");
    if let Some(inner) = t.strip_prefix("*const ") {
        let i = c_type(inner);
        return if i.ends_with('*') { format!("{i}const*") } else { format!("const {i}*") };
    }
    if let Some(inner) = t.strip_prefix("*mut ") {
        return format!("{}*", c_type(inner));
    }
    match t {
        "c_char" => "char",
        "c_void" => "void",
        "c_int" => "int",
        "usize" => "size_t",
        "u8" => "uint8_t",
        "u32" => "uint32_t",
        "i32" => "int32_t",
        "u64" => "uint64_t",
        "f32" => "float",
        other => other,
    }
    .to_owned()
}

/// name -> `ret(params)` from `extern "C" fn wolkit_*` in src/ (all files, recursively).
fn rust_exports() -> BTreeMap<String, String> {
    fn files(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                files(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut all = Vec::new();
    files(&root().join("src/ffi"), &mut all);
    let mut m = BTreeMap::new();
    for f in all {
        let src = std::fs::read_to_string(f).unwrap();
        let mut rest = src.as_str();
        while let Some(i) = rest.find("extern \"C\" fn wolkit_") {
            let sig = &rest[i + "extern \"C\" fn ".len()..];
            let end = sig.find('{').unwrap();
            let sig = sig[..end].split_whitespace().collect::<Vec<_>>().join(" ");
            rest = &rest[i + 1..];
            let open = sig.find('(').unwrap();
            let close = sig.rfind(')').unwrap();
            let name = sig[..open].to_owned();
            let params: Vec<String> = split_top(&sig[open + 1..close])
                .into_iter()
                .filter(|p| !p.trim().is_empty())
                .map(|p| c_type(p.split_once(':').unwrap().1))
                .collect();
            let ret = sig[close + 1..].trim().strip_prefix("->").map_or_else(|| "void".to_owned(), c_type);
            m.insert(
                name,
                format!("{}({})", squash(&ret), params.iter().map(|p| squash(p)).collect::<Vec<_>>().join(",")),
            );
        }
    }
    m
}

fn split_top(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth) = (Vec::new(), String::new(), 0i32);
    for c in s.chars() {
        match c {
            '(' | '<' => depth += 1,
            ')' | '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out
}

#[test]
fn header_declares_exactly_the_rust_exports() {
    let h = header_prototypes(&read("include/wolkit.h"));
    let r = rust_exports();
    assert!(r.len() > 40, "parser found {} exports", r.len());
    assert_eq!(h, r);
    assert_eq!(h.keys().cloned().collect::<BTreeSet<_>>(), names(&baselines()));
}

fn header_defines(h: &str) -> BTreeMap<String, i64> {
    h.lines()
        .filter_map(|l| l.trim().strip_prefix("#define "))
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let (n, v) = (f.next()?, f.next()?);
            Some((n.to_owned(), v.trim_end_matches('u').parse().ok()?))
        })
        .collect()
}

#[test]
fn header_constants_match_the_library() {
    let defines = header_defines(&read("include/wolkit.h"));
    let consts = wolkit::abi::constants();
    for (name, value) in &consts {
        assert_eq!(defines.get(name), Some(value), "{name}");
    }
    assert_eq!(defines.len(), consts.len(), "a define without a constant: {:?}", {
        let known: BTreeSet<_> = consts.iter().map(|c| &c.0).collect();
        defines.keys().filter(|k| !known.contains(k)).collect::<Vec<_>>()
    });
}

#[test]
fn property_ids_never_change() {
    let want = read("abi/wolkit-1.0.props");
    let now = wolkit::scene::prop::snapshot_lines();
    let now: BTreeSet<&str> = now.iter().map(String::as_str).collect();
    for line in want.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
        assert!(now.contains(line), "a shipped property changed or went away: {line}");
    }
}

#[test]
fn shipped_declarations_never_change() {
    let now: BTreeSet<String> = strip_comments(&read("include/wolkit.h")).split(';').map(squash).collect();
    let current = read("include/wolkit.h");
    for e in std::fs::read_dir(root().join("abi")).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "h") {
            let snap = std::fs::read_to_string(&p).unwrap();
            for decl in strip_comments(&snap).split(';').map(squash) {
                assert!(now.contains(&decl), "{}: changed or removed: {decl}", p.display());
            }
            // Every value but the version (which says which header this is) is forever.
            for def in snap.lines().filter(|l| l.starts_with("#define WOLKIT_") && !l.starts_with("#define WOLKIT_VERSION_")) {
                assert!(current.lines().any(|l| l == def), "{def}");
            }
        }
    }
}

#[test]
fn detectors_catch_changes() {
    let h = "wolkit_status wolkit_x(const char *s, size_t n, wolkit_error **error);\nvoid wolkit_y(void);\nconst char *const *wolkit_z(uint32_t *count);\nuint64_t wolkit_w(const wolkit_engine *, uint32_t key);";
    let p = header_prototypes(h);
    assert_eq!(p["wolkit_x"], "wolkit_status(const char*,size_t,wolkit_error**)");
    assert_eq!(p["wolkit_y"], "void()");
    assert_eq!(p["wolkit_z"], "const char*const*(uint32_t*)");
    assert_eq!(p["wolkit_w"], "uint64_t(const wolkit_engine*,uint32_t)");
    assert_eq!(c_type("*mut *mut wolkit_error"), "wolkit_error**");
    assert_eq!(c_type("*const *const c_char"), "const char*const*");
    assert_eq!(c_type("*const c_char"), "const char*");
    assert_eq!(c_type("f32"), "float");
    assert_eq!(c_type("*mut u8"), "uint8_t*");
}
