// SPDX-License-Identifier: BSD-2-Clause
//! STANDARDS 7.1 Fuzzing: every target in `fuzz/fuzz_targets/` is a `[[bin]]` of `fuzz/Cargo.toml`,
//! has a seeded `fuzz/regressions/<target>/` (replayed by `tools/ci/fuzz-replay.sh`), runs in the
//! ci.yml `fuzz` matrix and is named in the STANDARDS list; and nothing is listed that does not exist.
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn targets() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root().join("fuzz/fuzz_targets"))
        .unwrap()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.strip_suffix(".rs").map(str::to_owned)
        })
        .collect();
    v.sort();
    v
}

/// Names inside the first `[...]` after `key` on one line.
fn bracket_list(text: &str, key: &str) -> Vec<String> {
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with(key))
        .unwrap_or_else(|| panic!("no {key} line"));
    let inner = line
        .split('[')
        .nth(1)
        .and_then(|r| r.split(']').next())
        .unwrap_or("");
    let mut v: Vec<String> = inner
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    v.sort();
    v
}

#[test]
fn every_fuzz_target_is_built_seeded_run_and_documented() {
    let t = targets();
    assert!(t.len() >= 12, "{t:?}");
    let cargo = read("fuzz/Cargo.toml");
    let ci = read(".github/workflows/ci.yml");
    let standards = read("docs/STANDARDS.md");
    let fuzz_line = standards
        .lines()
        .find(|l| l.starts_with("- **Fuzzing**"))
        .expect("STANDARDS Fuzzing rule");
    // The rule's list runs to the first full stop after the opening parenthesis.
    let listed: String = standards[standards.find(fuzz_line).unwrap()..]
        .split(").")
        .next()
        .unwrap()
        .to_owned();
    for n in &t {
        assert!(
            cargo.contains(&format!("name = \"{n}\"\npath = \"fuzz_targets/{n}.rs\"")),
            "{n}: no [[bin]] in fuzz/Cargo.toml"
        );
        let seeds = std::fs::read_dir(root().join("fuzz/regressions").join(n))
            .map(|d| d.flatten().count())
            .unwrap_or(0);
        assert!(seeds > 0, "{n}: fuzz/regressions/{n}/ has no inputs");
        assert!(
            listed.contains(&format!("`{n}`")),
            "{n}: not in the STANDARDS fuzzing list"
        );
    }
    assert_eq!(bracket_list(&ci, "target: ["), t, "ci.yml fuzz matrix");
}
