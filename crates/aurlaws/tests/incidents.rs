// SPDX-License-Identifier: GPL-2.0-or-later
//! Defanged copies of published techniques, read by the same binary wopkg runs. Each fixture names
//! the law it must break. Nothing in `tests/fixtures` is run, and every address is reserved or fake.
use std::path::Path;
use std::process::Command;

#[test]
fn every_development_fixture_breaks_the_law_it_names() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dev");
    let mut seen = 0;
    let mut missed = Vec::new();
    for e in std::fs::read_dir(root).unwrap().flatten() {
        let want = std::fs::read_to_string(e.path().join("EXPECT")).unwrap();
        let o = Command::new(env!("CARGO_BIN_EXE_wopkg-aurlaws"))
            .arg("--dir")
            .arg(e.path())
            .arg("--json")
            .output()
            .unwrap();
        assert!(o.status.success());
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        let laws: Vec<&str> = v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["law"].as_str().unwrap())
            .collect();
        seen += 1;
        if v["verdict"] != "flag" || !laws.contains(&want.trim()) {
            missed.push(format!(
                "{:?}: wanted {} got {laws:?}",
                e.file_name(),
                want.trim()
            ));
        }
    }
    assert!(seen >= 20, "{seen}");
    assert!(missed.is_empty(), "{missed:#?}");
}

/// Written after the rules were frozen, and known to be missed in part. This guards the count
/// from falling. It is not a target: the misses stay in the set.
#[test]
fn the_held_out_fixtures_never_lose_ground() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/holdout");
    let (mut seen, mut flagged) = (0, 0);
    for e in std::fs::read_dir(root).unwrap().flatten() {
        let o = Command::new(env!("CARGO_BIN_EXE_wopkg-aurlaws"))
            .arg("--dir")
            .arg(e.path())
            .arg("--json")
            .output()
            .unwrap();
        assert!(o.status.success());
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        seen += 1;
        flagged += i32::from(v["verdict"] == "flag");
    }
    assert_eq!(seen, 28);
    assert!(flagged >= 19, "{flagged}");
}

#[test]
fn no_fixture_points_at_a_real_address() {
    fn walk(d: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            if e.path().is_dir() {
                walk(&e.path(), out);
            } else {
                out.push(std::fs::read_to_string(e.path()).unwrap());
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        &mut files,
    );
    for t in files {
        for w in
            t.split(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '(' || c == ')')
        {
            let Some(i) = w.find("://") else { continue };
            let host = w[i + 3..].split(['/', ':']).next().unwrap();
            let fine = host.starts_with("203.0.113.")
                || host.ends_with(".invalid")
                || host == "example.org"
                || host == "github.com" && w.contains("github.com/example/")
                || host.starts_with('$');
            assert!(fine, "{w}");
        }
    }
}
