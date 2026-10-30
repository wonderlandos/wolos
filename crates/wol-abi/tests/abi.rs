//! The stability guarantee of libwol (ADR 0020). A shipped symbol or declaration never goes away or
//! changes; new ones are only added, under a new version node (WOL_1.1, ...).
//! - every symbol in `abi/wol-*.symbols` is exported by `libwol.so` with that version
//! - every exported symbol is in a baseline, the linker map and `include/wol.h`; nothing else leaks
//! - every declaration of `abi/wol-1.0.h` (the 1.0 header snapshot) is still in `include/wol.h`
//! - `include/wol.h` prototypes and `#define`s match the Rust exports and constants exactly
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
    common::lib_dir().join("libwol.so.1")
}

fn readelf(args: &[&str]) -> String {
    let out = std::process::Command::new("readelf")
        .args(args)
        .arg(lib())
        .env("LC_ALL", "C")
        .output()
        .expect("readelf");
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `name@VERSION` of every defined dynamic symbol, plus anything defined that is not `wol_`.
fn exported() -> (BTreeSet<String>, Vec<String>) {
    let (mut syms, mut stray) = (BTreeSet::new(), Vec::new());
    for line in readelf(&["--dyn-syms", "--wide"]).lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 8 || f[0] == "Num:" || f[6] == "UND" || !f[0].ends_with(':') {
            continue;
        }
        let name = f[7];
        if f[3] == "OBJECT" && f[6] == "ABS" {
            continue; // version node definitions (WOL_1.0)
        }
        match name.split_once("@@").or_else(|| name.split_once('@')) {
            Some((n, v)) if n.starts_with("wol_") => {
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
    syms.iter()
        .map(|s| s.split('@').next().unwrap().to_owned())
        .collect()
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
        "new symbols need a line in abi/wol-<next>.symbols and a WOL_<next> node: {unrecorded:?}"
    );
    assert!(
        stray.is_empty(),
        "only wol_ symbols are exported: {stray:?}"
    );
}

#[test]
fn library_is_named_for_its_major_version() {
    assert!(readelf(&["-d"]).contains("Library soname: [libwol.so.1]"));
}

#[test]
fn linker_map_lists_every_baseline_symbol() {
    let map = read("abi/wol.map");
    let listed: BTreeSet<String> = map
        .lines()
        .map(str::trim)
        .filter_map(|l| l.strip_suffix(';'))
        .filter(|l| l.starts_with("wol_"))
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

/// `const char *utf8` -> `const char*`.
fn c_param(p: &str) -> String {
    let p = squash(p);
    if p == "void" {
        return String::new();
    }
    let cut = p
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .map_or(0, |i| i + 1);
    squash(&p[..cut])
}

/// name -> `ret(params)` from wol.h prototypes.
fn header_prototypes(h: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for stmt in strip_comments(h).split(';') {
        let s = squash(stmt);
        if s.starts_with("typedef") || s.starts_with("extern") || !s.ends_with(')') {
            continue;
        }
        let Some(open) = s.find('(') else { continue };
        let head = &s[..open];
        let name_at = head
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .map_or(0, |i| i + 1);
        let (ret, name) = (squash(&head[..name_at]), head[name_at..].to_owned());
        if !name.starts_with("wol_") {
            continue;
        }
        let params: Vec<String> = s[open + 1..s.len() - 1]
            .split(',')
            .map(c_param)
            .filter(|p| !p.is_empty())
            .collect();
        m.insert(name, format!("{ret}({})", params.join(",")));
    }
    m
}

fn c_type(rust: &str) -> String {
    let t = rust.trim().trim_start_matches("std::ffi::");
    if let Some(inner) = t.strip_prefix("*const ") {
        let i = c_type(inner);
        return if i.ends_with('*') {
            format!("{i}const*")
        } else {
            format!("const {i}*")
        };
    }
    if let Some(inner) = t.strip_prefix("*mut ") {
        return format!("{}*", c_type(inner));
    }
    match t {
        "c_char" => "char",
        "c_void" => "void",
        "c_int" => "int",
        "usize" => "size_t",
        "u32" => "uint32_t",
        "i32" => "int32_t",
        "u64" => "uint64_t",
        other => other,
    }
    .to_owned()
}

/// name -> `ret(params)` from `extern "C" fn wol_*` in src/.
fn rust_exports() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for e in std::fs::read_dir(root().join("src")).unwrap() {
        let src = std::fs::read_to_string(e.unwrap().path()).unwrap();
        let mut rest = src.as_str();
        while let Some(i) = rest.find("extern \"C\" fn wol_") {
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
            let ret = sig[close + 1..]
                .trim()
                .strip_prefix("->")
                .map_or_else(|| "void".to_owned(), c_type);
            m.insert(
                name,
                format!(
                    "{}({})",
                    squash(&ret),
                    params
                        .iter()
                        .map(|p| squash(p))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
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
    let h = header_prototypes(&read("include/wol.h"));
    let r = rust_exports();
    assert!(r.len() > 30, "parser found {} exports", r.len());
    assert_eq!(h, r);
    assert_eq!(
        h.keys().cloned().collect::<BTreeSet<_>>(),
        names(&baselines())
    );
}

#[test]
fn header_constants_match_the_library() {
    let h = read("include/wol.h");
    let defines: BTreeMap<String, i64> = h
        .lines()
        .filter_map(|l| l.trim().strip_prefix("#define "))
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let (n, v) = (f.next()?, f.next()?);
            Some((n.to_owned(), v.trim_end_matches('u').parse().ok()?))
        })
        .collect();
    for (name, value) in wol::abi::CONSTANTS {
        assert_eq!(defines.get(*name), Some(value), "{name}");
    }
    assert_eq!(defines.len(), wol::abi::CONSTANTS.len(), "{defines:?}");
}

#[test]
fn shipped_declarations_never_change() {
    let now: BTreeSet<String> = strip_comments(&read("include/wol.h"))
        .split(';')
        .map(squash)
        .collect();
    for e in std::fs::read_dir(root().join("abi")).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "h") {
            let snap = std::fs::read_to_string(&p).unwrap();
            for decl in strip_comments(&snap).split(';').map(squash) {
                assert!(
                    now.contains(&decl),
                    "{}: changed or removed: {decl}",
                    p.display()
                );
            }
            for def in snap.lines().filter(|l| l.starts_with("#define WOL_")) {
                assert!(read("include/wol.h").lines().any(|l| l == def), "{def}");
            }
        }
    }
}

#[test]
fn detectors_catch_changes() {
    let h = "wol_status wol_x(const char *s, size_t n);\nvoid wol_y(void);";
    let p = header_prototypes(h);
    assert_eq!(p["wol_x"], "wol_status(const char*,size_t)");
    assert_eq!(p["wol_y"], "void()");
    assert_eq!(c_type("*mut *mut wol_error"), "wol_error**");
    assert_eq!(c_type("*const c_char"), "const char*");
    assert_eq!(c_type("*const wol_string"), "const wol_string*");
}
