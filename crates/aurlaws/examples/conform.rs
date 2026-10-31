// SPDX-License-Identifier: GPL-2.0-or-later
//! Writes one Bend program over many packages, plus Rust's own answers, so the two checkers can be
//! compared and timed: `conform OUT_DIR REPEAT PACKAGE_DIR...`. Packages are read, never run.
//! Also prints how long Rust's checker takes over the same graphs, on 1 and 4 threads.
use aurlaws::{Report, facts, judge_files, laws};
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write;
use std::path::Path;
use std::time::Instant;

const TAIL: &str = "def show.law(l: R.Law) -> String:
  match l:
    case R.FetchExecuteUnverified{}: \"FetchExecuteUnverified\"
    case R.DecodeExecute{}: \"DecodeExecute\"
    case R.UnresolvedDynamic{}: \"UnresolvedDynamic\"
    case R.WriteOutsidePackage{}: \"WriteOutsidePackage\"
    case R.ReadCredentials{}: \"ReadCredentials\"
    case R.CredentialToNetwork{}: \"CredentialToNetwork\"
    case R.ScriptletUndeclaredFile{}: \"ScriptletUndeclaredFile\"
    case R.SourceOffUpstream{}: \"SourceOffUpstream\"
    case R.UndeclaredPersistence{}: \"UndeclaredPersistence\"
    case R.PostinstallNetwork{}: \"PostinstallNetwork\"
    case R.BinChecksumUpstream{}: \"BinChecksumUpstream\"
    case R.UnexplainedNewEdge{}: \"UnexplainedNewEdge\"
    case R.ObservedMismatch{}: \"ObservedMismatch\"
    case R.IncompleteEvidence{}: \"IncompleteEvidence\"

def show.rest(ls: +List<R.Law>) -> String:
  match ls:
    case Nil{}:
      \"\\n\"
    case Con{h, t}:
      \",\" ++ show.law(h) ++ show.rest(t)

def show(ls: +List<R.Law>) -> String:
  match ls:
    case Nil{}:
      \"\\n\"
    case Con{h, t}:
      show.law(h) ++ show.rest(t)
";

fn load(dir: &Path) -> Option<Report> {
    let raw = std::fs::read(dir.join("PKGBUILD")).ok()?;
    let mut files = HashMap::new();
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.ends_with(".install") {
            if let Ok(t) = std::fs::read_to_string(e.path()) {
                files.insert(n, t);
            }
        }
    }
    Some(judge_files(&String::from_utf8_lossy(&raw), &files))
}

fn line(r: &Report) -> String {
    let set: BTreeSet<usize> = r.findings.iter().map(|f| f.law as usize).collect();
    set.iter()
        .map(|i| facts::LAW_NAMES[*i])
        .collect::<Vec<_>>()
        .join(",")
}

/// A balanced fork-join tree over `lo..hi`, callees first, as Bend's scheduler wants it.
fn tree(b: &mut String, lo: usize, hi: usize, n: usize) {
    if hi - lo == 1 {
        let _ = writeln!(
            b,
            "def t{lo}_{hi}() -> String:\n  show(R.check(g{}()))\n",
            lo % n
        );
        return;
    }
    let mid = lo + (hi - lo) / 2;
    tree(b, lo, mid, n);
    tree(b, mid, hi, n);
    let _ = writeln!(
        b,
        "def t{lo}_{hi}() -> String:\n  x y = t{lo}_{mid}() t{mid}_{hi}()\n  x ++ y\n"
    );
}

fn time(reports: &[Report], repeat: usize, threads: usize) -> f64 {
    let t = Instant::now();
    let work: Vec<&Report> = (0..repeat).flat_map(|_| reports.iter()).collect();
    let chunk = work.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        for c in work.chunks(chunk) {
            s.spawn(move || {
                let mut found = 0;
                for r in c {
                    found += laws::recheck(r).len();
                }
                std::hint::black_box(found);
            });
        }
    });
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (out, repeat) = (Path::new(&args[0]), args[1].parse::<usize>().unwrap());
    let reports: Vec<Report> = args[2..]
        .iter()
        .filter_map(|d| load(Path::new(d)))
        .collect();
    let mut b = String::from("import Base\nimport ./rules.bend as R\n\n");
    b.push_str(TAIL);
    b.push('\n');
    for (i, r) in reports.iter().enumerate() {
        let _ = writeln!(b, "def g{i}() -> R.Graph:\n  {}\n", facts::term(r, &[]));
    }
    let total = reports.len() * repeat;
    tree(&mut b, 0, total, reports.len());
    let _ = write!(b, "def main() -> IO(Unit):\n  IO.write(t0_{total}())\n");
    std::fs::write(out.join("corpus.bend"), b).unwrap();
    let mut rust = String::new();
    for _ in 0..repeat {
        for r in &reports {
            rust.push_str(&line(r));
            rust.push('\n');
        }
    }
    std::fs::write(out.join("rust.txt"), rust).unwrap();
    println!("{} packages, {total} graphs", reports.len());
    for t in [1, 4] {
        let ms = (0..5)
            .map(|_| time(&reports, repeat, t))
            .fold(f64::MAX, f64::min);
        println!("rust check, {t} threads: {ms:.2} ms");
    }
}
