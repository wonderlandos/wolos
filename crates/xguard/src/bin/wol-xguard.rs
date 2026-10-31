// SPDX-License-Identifier: GPL-2.0-or-later
//! wol-xguard: the command behind `wol security scan`, the first-use guard and rule builds.
//!   wol-xguard scan [--json] [--rules DIR] PATH...   exit 0 clean, 1 found, 2 incomplete, 3 error
//!   wol-xguard rules build OUT VERSION SOURCE-DIR...  compile signature lists to a database
use std::io::Write;
use std::path::{Path, PathBuf};
use xguard::{Engine, Limits, Report, Verdict};

fn die(m: &str) -> ! {
    eprintln!("wol-xguard: {m}");
    std::process::exit(3)
}

fn deep(dir: &Path) -> Option<Box<dyn xguard::Deep>> {
    #[cfg(feature = "yara")]
    if let Some(y) = xguard::yara::Rules::open(&dir.join(xguard::yara::FILE)) {
        return Some(Box::new(y));
    }
    let _ = dir;
    None
}

fn scan(args: &[String]) -> i32 {
    let (mut json, mut dir, mut paths) = (false, None, Vec::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => json = true,
            "--rules" => dir = it.next().map(PathBuf::from),
            p => paths.push(p.to_owned()),
        }
    }
    let dir = dir.unwrap_or_else(xguard::rules_dir);
    let e = Engine::open(&dir).unwrap_or_else(|m| die(&m));
    let extra = deep(&dir);
    let mut worst = 0;
    for p in paths {
        let mut f = std::fs::File::open(&p).unwrap_or_else(|m| die(&format!("{p}: {m}")));
        let r: Report = xguard::scan_file(&e, &mut f, &p, extra.as_deref(), Limits::default())
            .unwrap_or_else(|m| die(&format!("{p}: {m}")));
        if json {
            println!("{}", serde_json::to_string(&r).unwrap_or_default());
        } else {
            match r.verdict {
                Verdict::Clean => println!("{p}: nothing found"),
                Verdict::Found => r
                    .findings
                    .iter()
                    .for_each(|f| println!("{}: {} ({})", f.path, f.name, f.how)),
                Verdict::Incomplete => r
                    .skipped
                    .iter()
                    .for_each(|s| println!("{}: not read: {}", s.path, s.why)),
            }
        }
        worst = worst.max(match r.verdict {
            Verdict::Clean => 0,
            Verdict::Incomplete => 2,
            Verdict::Found => 1,
        });
    }
    let _ = std::io::stdout().flush();
    worst
}

fn build(args: &[String]) -> i32 {
    let [out, version, sources @ ..] = args else {
        die("usage: rules build OUT VERSION SOURCE-DIR...")
    };
    let version = version.parse().unwrap_or_else(|_| die("bad version"));
    let mut lists: Vec<(String, String)> = Vec::new();
    for s in sources {
        let mut files: Vec<_> = std::fs::read_dir(s)
            .unwrap_or_else(|m| die(&format!("{s}: {m}")))
            .flatten()
            .map(|e| e.path())
            .collect();
        files.sort();
        for f in files {
            let Some(ext) = f.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if xguard::sigs::EXTENSIONS.contains(&ext) {
                let bytes = std::fs::read(&f).unwrap_or_else(|m| die(&format!("{f:?}: {m}")));
                lists.push((ext.to_owned(), String::from_utf8_lossy(&bytes).into_owned()));
            }
        }
    }
    let refs: Vec<(&str, &str)> = lists
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let mut b = xguard::db::Builder::default();
    let st = xguard::sigs::import(&mut b, &refs);
    let bytes = b.build(version, xguard::scan::now());
    let tmp = format!("{out}.tmp");
    std::fs::write(&tmp, &bytes)
        .and_then(|()| std::fs::rename(&tmp, out))
        .unwrap_or_else(|m| die(&format!("{out}: {m}")));
    println!("{}", serde_json::to_string(&st).unwrap_or_default());
    println!("{out}: {} bytes", bytes.len());
    0
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let code = match a.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["scan", ..] => scan(&a[1..]),
        ["rules", "build", ..] => build(&a[2..]),
        _ => die("usage: wol-xguard scan|rules build"),
    };
    std::process::exit(code)
}
