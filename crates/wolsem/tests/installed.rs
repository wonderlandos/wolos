// SPDX-License-Identifier: BSD-2-Clause
//! What the image ships under `/usr/share/wolos/manual/` is read as `Installed::load` reads it: a
//! missing table is an error (search by words only), a manual that cannot be read costs only the
//! manual, a map is found by language.

use std::path::{Path, PathBuf};

use wolsem::{Installed, Procrustes};

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wolsem-installed-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap_or_else(|e| panic!("{e}"));
    d
}

#[test]
fn a_shipped_directory_loads_and_a_broken_manual_costs_only_the_manual() {
    let d = dir("a");
    std::fs::copy(data("mini.wst"), d.join("static-mrl.wst")).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        Installed::load(Path::new("/nonexistent/wolsem")).is_err(),
        "no table: no meaning search"
    );
    // No manual file: an empty manual.
    let none = Installed::load(&d).unwrap_or_else(|e| panic!("{e}"));
    assert!(none.manual.is_empty() && none.table.len() > 1000);
    // A manual that is not JSON: still an empty manual, the table stays.
    std::fs::write(d.join("sections.json"), "not json").unwrap_or_else(|e| panic!("{e}"));
    assert!(
        Installed::load(&d)
            .unwrap_or_else(|e| panic!("{e}"))
            .manual
            .is_empty()
    );
    // The real sections, with a map for pt-BR.
    std::fs::copy(data("sections.json"), d.join("sections.json")).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(d.join("map.pt-BR.wprc"), Procrustes::identity(8).to_bytes())
        .unwrap_or_else(|e| panic!("{e}"));
    let full = Installed::load(&d).unwrap_or_else(|e| panic!("{e}"));
    assert!(full.manual.len() > 400);
    assert_eq!(full.map_for("pt-BR").map(|m| m.dim()), Some(8));
    assert!(
        full.map_for("de").is_none()
            && full.map_for("../etc").is_none()
            && full.map_for("").is_none()
    );
    let _ = std::fs::remove_dir_all(&d);
}
