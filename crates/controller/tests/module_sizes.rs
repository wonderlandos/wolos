// SPDX-License-Identifier: GPL-2.0-or-later
//! The registry's `size_mb` is the measured download, not a guess (`wol modules status` prints it
//! as "download: N MB", the installer and Settings > Modules show it). Unit: MiB, as pacman's
//! "Total Download Size", rounded up to the next 10. A module downloads its meta package's
//! dependencies that a fresh slot lacks, its recipes (user side, `RECIPE_BYTES`) and, for a record
//! with a `backend`, the largest backend variant (the worst case the label promises).
//!
//! Always: every record has a positive size that is a multiple of 10, and every recipe a record
//! names has a byte count here for its current version (a recipe bump fails until it is measured).
//! With `WOLOS_REPO_DB` (a pacman DBPath whose `local` is a fresh installed slot's database and
//! whose `sync` holds the refreshed lists of that slot's repos; `WOLOS_PACMAN_CONF` names the
//! pacman.conf listing them, default the host's) the sizes are recomputed with `pacman -Sp` and
//! must equal the records.

use std::path::PathBuf;
use std::process::Command;
use wol_core::modules::{IDS, Module};

/// Recipe download sizes (bytes of the release file, Content-Length), by name and version.
const RECIPE_BYTES: &[(&str, &str, u64)] = &[
    ("claude-code", "2.1.309", 293_469_552),
    ("powershell", "7.6.9", 73_399_520),
];

/// The GPU variants of a record's `backend` stem (`Module::backend_package`).
const BACKENDS: &[&str] = &["cuda", "rocm", "vulkan"];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn record(id: &str) -> Module {
    let path = root().join(format!("files/usr/share/wolos/modules/{id}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Module::parse(&text).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// The `version = "..."` of wopkg/recipes/<name>/recipe.toml.
fn recipe_version(name: &str) -> String {
    let path = root().join(format!("wopkg/recipes/{name}/recipe.toml"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .find_map(|l| {
            let v = l
                .strip_prefix("version")?
                .trim_start()
                .strip_prefix('=')?
                .trim();
            Some(v.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| panic!("{}: no version", path.display()))
}

fn recipe_bytes(name: &str) -> u64 {
    let v = recipe_version(name);
    RECIPE_BYTES
        .iter()
        .find(|(n, ver, _)| *n == name && *ver == v)
        .map(|(_, _, b)| *b)
        .unwrap_or_else(|| {
            panic!("recipe {name} {v}: measure its download and add it to RECIPE_BYTES")
        })
}

/// Bytes pacman would download for TARGETS into the slot of DB.
fn pacman_bytes(db: &str, targets: &[String]) -> u64 {
    let mut c = Command::new("pacman");
    if let Ok(conf) = std::env::var("WOLOS_PACMAN_CONF") {
        c.args(["--config", &conf]);
    }
    // An empty cache: pacman prints 0 for a package it already holds, and a fresh slot holds none.
    let cache = std::env::temp_dir().join(format!("wolos-module-sizes-{}", std::process::id()));
    std::fs::create_dir_all(&cache).expect("cache dir");
    let out = c
        .args(["--dbpath", db, "--cachedir"])
        .arg(&cache)
        .args(["-Sp", "--print-format", "%s"])
        .args(targets)
        .output()
        .expect("pacman");
    let _ = std::fs::remove_dir(&cache);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "pacman -Sp {targets:?}: {}{text}",
        String::from_utf8_lossy(&out.stderr)
    );
    text.lines()
        .map(|l| {
            l.trim()
                .parse::<u64>()
                .unwrap_or_else(|_| panic!("pacman -Sp: {l:?}"))
        })
        .sum()
}

/// MiB rounded up to the next 10.
fn rounded_mib(bytes: u64) -> u32 {
    let tens = bytes.div_ceil(10 * 1024 * 1024);
    u32::try_from(tens * 10).expect("size fits")
}

#[test]
fn rounded_mib_rounds_up_to_ten() {
    assert_eq!(rounded_mib(1), 10);
    assert_eq!(rounded_mib(10 * 1024 * 1024), 10);
    assert_eq!(rounded_mib(10 * 1024 * 1024 + 1), 20);
    assert_eq!(rounded_mib(358_517_837), 350);
}

#[test]
fn module_sizes_are_positive_and_rounded_to_ten() {
    for id in IDS {
        let m = record(id);
        assert!(
            m.size_mb > 0 && m.size_mb.is_multiple_of(10),
            "{id}: size_mb {} must be a positive multiple of 10",
            m.size_mb
        );
        let recipes: u64 = m.recipes.iter().map(|r| recipe_bytes(r)).sum();
        assert!(
            rounded_mib(recipes) <= m.size_mb,
            "{id}: size_mb {} is less than its recipes alone",
            m.size_mb
        );
    }
}

#[test]
fn module_sizes_match_the_repo() {
    let Ok(db) = std::env::var("WOLOS_REPO_DB") else {
        eprintln!("WOLOS_REPO_DB unset: sizes not recomputed");
        return;
    };
    let mut wrong = Vec::new();
    for id in IDS {
        let m = record(id);
        let meta = m.package();
        let mut variants = vec![vec![meta.clone()]];
        if let Some(stem) = &m.backend {
            variants = BACKENDS
                .iter()
                .map(|b| vec![meta.clone(), format!("{stem}-{b}")])
                .collect();
        }
        let packages = variants
            .iter()
            .map(|t| pacman_bytes(&db, t))
            .max()
            .unwrap_or(0);
        let recipes: u64 = m.recipes.iter().map(|r| recipe_bytes(r)).sum();
        let want = rounded_mib(packages + recipes);
        eprintln!(
            "{id}: packages {packages} B, recipes {recipes} B, size_mb {want} (record {})",
            m.size_mb
        );
        if want != m.size_mb {
            wrong.push(format!("{id}: record {} measured {want}", m.size_mb));
        }
    }
    assert!(wrong.is_empty(), "size_mb drifted: {wrong:?}");
}
