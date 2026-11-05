// SPDX-License-Identifier: BSD-2-Clause
//! The compiled looks the Crystal apps commit (`src/<app>/style/table.cr` and `classes.cr`, written
//! by the app's `tools/styles.sh` so a package build needs no style compiler) are in step with the
//! compiler and its tokens. The gallery's table shipped stale into the 0.13.0 package build: the
//! brand pass changed the radius scale and the elevation shadows in `src/style/tokens.rs`, which
//! `wolkit-stylec` bakes into the table, and only `make spec` with a style compiler at hand (the
//! package's check()) noticed. This runs the same compile as `tools/styles.sh --check` (the kit
//! sheet first as `00-kit.wolcss`, then `src/styles/*.wolcss` as `10-<name>`, `--deny-warnings`)
//! and needs no Crystal: it compares the table's bytes and the class ids, not the formatted text.
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The apps whose look `tools/styles.sh` compiles: (app folder, its `src/<x>/style` folder).
fn apps() -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    let apps = root().join("wolos-crystal/apps");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&apps)
        .unwrap_or_else(|e| panic!("{}: {e}", apps.display()))
        .map(|e| e.unwrap().path())
        .collect();
    dirs.sort();
    for app in dirs {
        if !app.join("tools/styles.sh").is_file() {
            continue;
        }
        let styles: Vec<PathBuf> = std::fs::read_dir(app.join("src"))
            .unwrap()
            .map(|e| e.unwrap().path().join("style"))
            .filter(|s| s.join("table.cr").is_file())
            .collect();
        assert_eq!(
            styles.len(),
            1,
            "{}: one src/<x>/style/table.cr expected, found {styles:?}",
            app.display()
        );
        out.push((app, styles[0].clone()));
    }
    out
}

/// Every number between `Bytes[` and `]` of a generated `table.cr`.
fn table_bytes(text: &str) -> Vec<u8> {
    let body = text
        .split_once("Bytes[")
        .and_then(|(_, r)| r.split_once(']'))
        .expect("a `Bytes[ ... ]` literal")
        .0;
    body.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .map(|t| t.parse().unwrap_or_else(|e| panic!("`{t}`: {e}")))
        .collect()
}

/// `NAME = 3_u32 # .name` lines of a Crystal styles module, spacing ignored.
fn class_ids(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| l.contains("_u32 # ."))
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

#[test]
fn committed_style_tables_match_the_compiler() {
    let apps = apps();
    assert!(
        apps.iter().any(|(a, _)| a.ends_with("gallery")),
        "the gallery's tools/styles.sh is not found: {apps:?}"
    );
    let kit = root().join("wolos-crystal/shards/wolkit/styles/kit.wolcss");
    for (app, style) in apps {
        let d = std::env::temp_dir().join(format!(
            "wolkit-style-tables-{}-{}",
            std::process::id(),
            app.file_name().unwrap().to_string_lossy()
        ));
        let sheets = d.join("sheets");
        std::fs::create_dir_all(&sheets).unwrap();
        std::fs::copy(&kit, sheets.join("00-kit.wolcss")).unwrap();
        let mut names = vec![sheets.join("00-kit.wolcss")];
        let mut own: Vec<PathBuf> = std::fs::read_dir(app.join("src/styles"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "wolcss"))
            .collect();
        own.sort();
        for f in own {
            let to = sheets.join(format!("10-{}", f.file_name().unwrap().to_string_lossy()));
            std::fs::copy(&f, &to).unwrap();
            names.push(to);
        }
        let (wsb, cr) = (d.join("look.wsb"), d.join("styles.cr"));
        let o = Command::new(env!("CARGO_BIN_EXE_wolkit-stylec"))
            .arg("--deny-warnings")
            .arg("--out")
            .arg(&wsb)
            .arg("--crystal")
            .arg(&cr)
            .args(&names)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{}: {}",
            app.display(),
            String::from_utf8_lossy(&o.stderr)
        );
        let fresh = std::fs::read(&wsb).unwrap();
        let fresh_ids = class_ids(&std::fs::read_to_string(&cr).unwrap());
        std::fs::remove_dir_all(&d).ok();
        let rel = style
            .strip_prefix(root())
            .unwrap_or(&style)
            .display()
            .to_string();
        let app_rel = app
            .strip_prefix(root())
            .unwrap_or(&app)
            .display()
            .to_string();
        let committed = table_bytes(&std::fs::read_to_string(style.join("table.cr")).unwrap());
        assert!(
            committed == fresh,
            "{rel}/table.cr is stale ({} committed bytes, {} compiled; first difference at byte {}): \
             run `make styles` in {} and re-bless its goldens if the look changed",
            committed.len(),
            fresh.len(),
            committed
                .iter()
                .zip(&fresh)
                .position(|(a, b)| a != b)
                .unwrap_or(committed.len().min(fresh.len())),
            app_rel
        );
        let committed_ids = class_ids(&std::fs::read_to_string(style.join("classes.cr")).unwrap());
        assert_eq!(
            committed_ids, fresh_ids,
            "{rel}/classes.cr is stale: run `make styles`"
        );
    }
}

#[test]
fn table_bytes_reads_the_generated_literal() {
    let t = "module G\n  STYLE_TABLE = Bytes[\n    87, 83, 66,\n    49,\n  ]\nend\n";
    assert_eq!(table_bytes(t), vec![87, 83, 66, 49]);
    assert_eq!(
        class_ids("  CARD           =  3_u32 # .card\n  NAMES = {\n"),
        vec!["CARD = 3_u32 # .card"]
    );
}
