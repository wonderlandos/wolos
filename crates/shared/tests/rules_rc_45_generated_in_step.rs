// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-45 (docs/STANDARDS.md "Origins of the 0.13 bugs"): a file a tool writes
//! is the tool's, and a test says when it has fallen behind its input. RH-451 reads every
//! committed `Cargo.lock` (the workspace's and `fuzz/`'s) against the `Cargo.toml` of each of its
//! own crates: a dependency the manifest names (normal and build; a workspace member's dev
//! dependencies too) must be in the lock's list for that crate. `fuzz/Cargo.lock` was found one
//! change behind on 2026-11-04 (core gained `toml`; `cargo build --locked` of `fuzz/` failed before
//! anyone touched it), and a stale lock is only found when something builds it with `--locked`.
//! The generated design files have the same shape and their own check (the wallpaper step of
//! wolos-system fails when the generator is missing: RP-056). Counts are frozen per lock in
//! [`BASELINE`].
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, assert_ratchet, hit};
use std::collections::{BTreeMap, BTreeSet};

/// Locks behind their manifests when the rule came (2026-11-04, merged tree up to wd-store).
/// `fuzz/Cargo.lock` lacks core's `toml`; ATLAS-BRIDGE regenerates it in its change. Only shrinks.
const BASELINE: &[Row] = &[("RH-451", "fuzz/Cargo.lock", 1, "STD-7")];

const IDS: &[&str] = &["RH-451"];

/// Crate name -> dependency names of the `[[package]]` entries of a lock that come from this
/// repository (no `source` line).
fn lock_packages(text: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut out = BTreeMap::new();
    for block in text.split("[[package]]").skip(1) {
        let mut name = None;
        let mut local = true;
        let mut deps = BTreeSet::new();
        let mut in_deps = false;
        for l in block.lines() {
            let l = l.trim();
            if let Some(v) = l.strip_prefix("name = ") {
                name = Some(v.trim_matches('"').to_owned());
            } else if l.starts_with("source = ") {
                local = false;
            } else if l.starts_with("dependencies = [") {
                in_deps = true;
            } else if in_deps && l.starts_with(']') {
                in_deps = false;
            } else if in_deps {
                let d = l.trim_matches([',', '"']);
                deps.insert(d.split_whitespace().next().unwrap_or("").to_owned());
            }
        }
        if let (Some(n), true) = (name, local) {
            out.insert(n, deps);
        }
    }
    out
}

/// (package name, normal and build dependency names, dev and optional dependency names) of a manifest.
fn manifest(
    text: &str,
    renames: &BTreeMap<String, String>,
) -> Option<(String, BTreeSet<String>, BTreeSet<String>)> {
    let (mut pkg, mut section) = (None, String::new());
    let (mut normal, mut dev) = (BTreeSet::new(), BTreeSet::new());
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            section = t.trim_matches(['[', ']']).to_owned();
            if let Some((_, rest)) = section.split_once("dependencies.") {
                let set = if section.contains("dev-dependencies") {
                    &mut dev
                } else {
                    &mut normal
                };
                set.insert(rest.to_owned());
            }
            continue;
        }
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if section == "package"
            && let Some(v) = t.strip_prefix("name = ")
        {
            pkg = Some(v.trim_matches('"').to_owned());
        }
        let deps = section == "dependencies"
            || section == "build-dependencies"
            || section.ends_with(".dependencies")
            || section.ends_with(".build-dependencies");
        let devs = section == "dev-dependencies" || section.ends_with(".dev-dependencies");
        if deps || devs {
            let key = t.split(['=', '.', ' ']).next().unwrap_or("");
            // `x = { path = "..", package = "real" }`: the lock names the real package.
            let real = t
                .split("package = \"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .or_else(|| renames.get(key).map(String::as_str))
                .unwrap_or(key);
            if !real.is_empty() {
                // A dev dependency and an optional one reach a lock only through a workspace member.
                if devs || t.contains("optional = true") {
                    dev.insert(real.to_owned())
                } else {
                    normal.insert(real.to_owned())
                };
            }
        }
    }
    Some((pkg?, normal, dev))
}

/// Dependencies a manifest names that the lock's entry for it lacks.
fn behind(
    lock: &BTreeMap<String, BTreeSet<String>>,
    manifest_text: &str,
    member: bool,
    renames: &BTreeMap<String, String>,
) -> Vec<String> {
    let Some((name, normal, dev)) = manifest(manifest_text, renames) else {
        return Vec::new();
    };
    let Some(have) = lock.get(&name) else {
        return Vec::new();
    };
    let dev: Vec<&String> = if member {
        dev.iter().collect()
    } else {
        Vec::new()
    };
    normal
        .iter()
        .chain(dev)
        .filter(|d| !have.contains(*d))
        .map(|d| format!("{name} names `{d}`"))
        .collect()
}

/// `[workspace.dependencies]` entries that rename their package (`wol_core = { package = "core" }`).
fn workspace_renames(root_toml: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut on = false;
    for l in root_toml.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            on = t == "[workspace.dependencies]";
        } else if on
            && let Some((key, rest)) = t.split_once('=')
            && let Some(real) = rest
                .split("package = \"")
                .nth(1)
                .and_then(|r| r.split('"').next())
        {
            out.insert(key.trim().to_owned(), real.to_owned());
        }
    }
    out
}

fn manifests() -> Vec<String> {
    let mut v: Vec<String> = rc::walk("crates", &["toml"])
        .into_iter()
        .filter(|p| p.ends_with("/Cargo.toml") && p.matches('/').count() == 2)
        .collect();
    v.push("fuzz/Cargo.toml".to_owned());
    v
}

#[test]
fn rh_451_a_lock_is_not_behind_its_manifests() {
    let mut h = Hits::new();
    for (lock_rel, member) in [("Cargo.lock", true), ("fuzz/Cargo.lock", false)] {
        let lock = lock_packages(&rc::read(lock_rel));
        let renames = workspace_renames(&rc::read("Cargo.toml"));
        assert!(
            lock.len() >= 5,
            "{lock_rel}: only {} local packages read",
            lock.len()
        );
        for m in manifests() {
            // A lock's own crate (its workspace member) is held to dev dependencies too; a crate
            // it only reaches by path is not. The root lock does not carry the fuzz crate.
            if m == "fuzz/Cargo.toml" && lock_rel == "Cargo.lock" {
                continue;
            }
            let own = member || m == "fuzz/Cargo.toml";
            for what in behind(&lock, &rc::read(&m), own, &renames) {
                hit(
                    &mut h,
                    lock_rel,
                    format!("{lock_rel}: {m}: {what} and the lock does not"),
                );
            }
        }
    }
    assert_ratchet(
        "RH-451",
        "a committed lock lists what its manifests name",
        BASELINE,
        &h,
    );
}

#[test]
fn rc_45_reader_reads_what_it_says() {
    let lock = lock_packages(concat!(
        "[[package]]\nname = \"core\"\nversion = \"1\"\ndependencies = [\n \"serde\",\n \"toml 1.0.0\",\n]\n\n",
        "[[package]]\nname = \"serde\"\nversion = \"1\"\nsource = \"registry+x\"\n",
    ));
    assert!(
        lock.contains_key("core") && !lock.contains_key("serde"),
        "a registry package is not ours"
    );
    let none = BTreeMap::new();
    let ok = "[package]\nname = \"core\"\n[dependencies]\nserde.workspace = true\ntoml = \"1\"\n";
    assert!(behind(&lock, ok, true, &none).is_empty());
    let stale = "[package]\nname = \"core\"\n[dependencies]\nserde.workspace = true\ntoml = \"1\"\nlibc = \"0.2\"\n";
    assert_eq!(behind(&lock, stale, true, &none), ["core names `libc`"]);
    let opt =
        "[package]\nname = \"core\"\n[dependencies]\nx = { version = \"1\", optional = true }\n";
    assert_eq!(
        behind(&lock, opt, true, &none),
        ["core names `x`"],
        "a member's optional dependency counts"
    );
    assert!(
        behind(&lock, opt, false, &none).is_empty(),
        "a path dependency's optional one is off"
    );
    let dev = "[package]\nname = \"core\"\n[dev-dependencies]\ntempfile = \"3\"\n";
    assert_eq!(
        behind(&lock, dev, true, &none),
        ["core names `tempfile`"],
        "a member's dev dependency counts"
    );
    assert!(
        behind(&lock, dev, false, &none).is_empty(),
        "a path dependency's dev dependency does not"
    );
    let renamed =
        "[package]\nname = \"core\"\n[dependencies]\nw = { path = \"..\", package = \"serde\" }\n";
    assert!(
        behind(&lock, renamed, true, &none).is_empty(),
        "a rename reads the real package"
    );
    let ws = workspace_renames(
        "[workspace.dependencies]\nw = { path = \"..\", package = \"serde\" }\n[other]\nx = { package = \"y\" }\n",
    );
    assert_eq!(ws.get("w").map(String::as_str), Some("serde"));
    assert_eq!(ws.len(), 1);
    let via = "[package]\nname = \"core\"\n[dependencies]\nw.workspace = true\n";
    assert!(
        behind(&lock, via, true, &ws).is_empty(),
        "a workspace rename reads the real package"
    );
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_45_generated_in_step.rs");
    rc::assert_doc_agrees("rules_rc_45_generated_in_step.rs", me, IDS, BASELINE);
}
