// SPDX-License-Identifier: GPL-2.0-or-later
//! Every local file a [wolos] recipe lists in `source=` matches its `sha256sums=` entry, so
//! makepkg's validity check passes on the build hosts (a header edit without new sums broke
//! helium-browser).
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn sha256(p: &Path) -> String {
    let o = Command::new("sha256sum").arg(p).output().expect("sha256sum runs");
    String::from_utf8_lossy(&o.stdout).split_whitespace().next().unwrap_or("").to_owned()
}

#[test]
fn local_sources_match_their_sums() {
    let mut bad = Vec::new();
    let mut seen = 0;
    for dir in ["image/repo", "kernel"] {
        for e in std::fs::read_dir(root().join(dir)).unwrap() {
            let d = e.unwrap().path();
            if !d.join("PKGBUILD").is_file() {
                continue;
            }
            let o = Command::new("bash")
                .current_dir(&d)
                .env("startdir", &d)
                .arg("-c")
                .arg(r#"source ./PKGBUILD >/dev/null 2>&1; for i in "${!source[@]}"; do printf '%s\t%s\n' "${source[$i]}" "${sha256sums[$i]:-}"; done"#)
                .output()
                .unwrap();
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                let (src, sum) = line.split_once('\t').unwrap_or((line, ""));
                if src.contains("::") || src.contains("://") || sum.is_empty() || sum == "SKIP" {
                    continue;
                }
                seen += 1;
                let got = sha256(&d.join(src));
                if got != sum {
                    bad.push(format!("{}/{src}: {got}", d.display()));
                }
            }
        }
    }
    assert!(seen > 0, "no local sources with sums found");
    assert!(bad.is_empty(), "stale sha256sums: {bad:?}");
}
