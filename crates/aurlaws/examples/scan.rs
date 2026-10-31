// SPDX-License-Identifier: GPL-2.0-or-later
//! Static scan of AUR checkouts, one JSON line per package. Reads files; runs nothing from them.
//! Usage: scan <dir-holding-PKGBUILD>...
use std::collections::HashMap;
use std::path::Path;

fn text_files(dir: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "PKGBUILD" || !e.path().is_file() {
            continue;
        }
        if e.metadata().is_ok_and(|m| m.len() < 1_000_000) {
            if let Ok(t) = std::fs::read_to_string(e.path()) {
                out.insert(name, t);
            }
        }
    }
    out
}

fn main() {
    for d in std::env::args().skip(1) {
        let dir = Path::new(&d);
        let Ok(p) = std::fs::read_to_string(dir.join("PKGBUILD")) else {
            continue;
        };
        let t = std::time::Instant::now();
        let r = aurlaws::judge_files(&p, &text_files(dir));
        let line = serde_json::json!({
            "pkg": dir.file_name().map(|x| x.to_string_lossy().into_owned()),
            "verdict": r.verdict,
            "us": t.elapsed().as_micros() as u64,
            "truncated": r.truncated,
            "nodes": r.graph.nodes.len(),
            "edges": r.graph.edges.len(),
            "notes": r.notes.len(),
            "findings": r.findings.iter().map(|f| serde_json::json!({"law": f.law, "sentence": f.sentence})).collect::<Vec<_>>(),
        });
        println!("{line}");
    }
}
