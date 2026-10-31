// SPDX-License-Identifier: GPL-2.0-or-later
//! The analyser wopkg's AurHook runs: `wopkg-aurlaws --dir D [--artifact F] [--trace T] --json`.
//! Prints `pass` or `flag` with evidence and exits 0. Any other exit means "not checked". There is
//! no `block`: a broken law asks for review, and the person decides.
//! A separate process on purpose: it reads hostile text, and a crash here cannot pass a package.
use aurlaws::graph::Stage;
use aurlaws::{Evidence, Verdict, observed};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

const MAX_FILE: u64 = 1_000_000;

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
        if e.metadata().is_ok_and(|m| m.len() < MAX_FILE) {
            if let Ok(t) = std::fs::read_to_string(e.path()) {
                out.insert(name, t);
            }
        }
    }
    out
}

/// The built package's paths and install script, read with bsdtar and never unpacked.
fn artifact(path: &str) -> Option<(Vec<String>, Option<String>)> {
    let o = Command::new("bsdtar")
        .args(["-tf", path])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let list = String::from_utf8_lossy(&o.stdout);
    let files = list
        .lines()
        .filter(|l| !l.starts_with('.'))
        .map(|l| format!("/{}", l.trim_end_matches('/')))
        .collect();
    let script = Command::new("bsdtar")
        .args(["-xOf", path, ".INSTALL"])
        .output()
        .ok()
        .filter(|o| o.status.success() && !o.stdout.is_empty())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    Some((files, script))
}

fn usage() -> ! {
    eprintln!("usage: wopkg-aurlaws --dir DIR [--artifact FILE] [--trace FILE] --json");
    std::process::exit(2);
}

fn main() {
    let (mut dir, mut art, mut trace) = (None, None, None);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dir" => dir = args.next(),
            "--artifact" => art = args.next(),
            "--trace" => trace = args.next(),
            "--json" => {}
            _ => usage(),
        }
    }
    let Some(dir) = dir else { usage() };
    let dir = Path::new(&dir);
    let Ok(raw) = std::fs::read(dir.join("PKGBUILD")) else {
        eprintln!("no PKGBUILD in {}", dir.display());
        std::process::exit(3);
    };
    let pkgbuild = String::from_utf8_lossy(&raw).into_owned();
    let mut files = text_files(dir);
    let mut ev = Evidence::default();
    let mut said = Vec::new();
    match art.as_deref().map(|a| (a, artifact(a))) {
        Some((_, Some((list, script)))) => {
            ev.package_files = list;
            if let Some(s) = script {
                files.insert("built package's install script".to_owned() + ".install", s);
            }
        }
        Some((a, None)) => said.push(format!(
            "The built package {a} could not be read, so its file list was not used."
        )),
        None => said.push("No built package was given, so its file list was not used.".to_owned()),
    }
    match trace {
        Some(t) => match std::fs::read(&t) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let g = observed::parse(&text, Stage::Build, "build trace");
                ev.gaps.extend(observed::damage(&text, &g));
                ev.observed.push(g);
            }
            Err(_) => ev.gaps.push(format!(
                "The build trace {t} is missing, so what the build did was not seen."
            )),
        },
        None => said.push(
            "The build was not traced, so only what the package's files say was judged.".to_owned(),
        ),
    }
    let r = aurlaws::judge_with(&pkgbuild, &files, &ev);
    let mut evidence: Vec<String> = r.findings.iter().map(|f| f.sentence.clone()).collect();
    if r.verdict == Verdict::NoLawBroken {
        evidence.push("No law broken.".to_owned());
    }
    evidence.append(&mut said);
    evidence.extend(r.notes.iter().map(|n| match &n.loc {
        Some(l) => format!("{} ({l}).", n.text),
        None => format!("{}.", n.text),
    }));
    evidence.push(r.disclaimer.clone());
    let verdict = if r.verdict == Verdict::NeedsReview {
        "flag"
    } else {
        "pass"
    };
    let out = serde_json::json!({"verdict": verdict, "evidence": evidence, "findings": r.findings, "notes": r.notes, "meta": r.meta});
    println!("{out}");
}
