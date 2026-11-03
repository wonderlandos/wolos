// SPDX-License-Identifier: GPL-2.0-or-later
//! wol-xguard: the command behind `wol security scan`, the first-use guard and rule builds.
//!   wol-xguard scan [--json] [--stream] [--rules DIR] PATH...   exit 0 clean, 1 found, 2 incomplete, 3 error
//!   wol-xguard guard [--state D] [--rules D] [--scope M]... [--downloads D]...  the first-use guard
//!   wol-xguard check|allow PATH   ask the guard about a file; Open Anyway
//!   wol-xguard sweep [--rules D] [STATE [ROOT...]]  the background pass (roots given: no idle gate)
//!   wol-xguard rules build OUT VERSION SOURCE-DIR...  compile signature lists to a database
//!   wol-xguard rules yara OUT SOURCE-DIR...           compile rule sources
//!   wol-xguard rules bundle DIR VERSION               the manifest of a signature update
//!   wol-xguard rules install SRC [DEST]               install a signed update
use std::io::Write;
use std::path::{Path, PathBuf};
use xguard::{Engine, Limits, Report, Verdict};

fn die(m: &str) -> ! {
    eprintln!("wol-xguard: {m}");
    std::process::exit(3)
}

/// The rule sets in `dir`. Err: a rules file is there but does not load (a sandbox that forbids
/// making code, audit S4, or a damaged file): the scan must not then say "nothing found".
fn deep(dir: &Path) -> Result<Option<Box<dyn xguard::Deep>>, ()> {
    #[cfg(feature = "yara")]
    {
        let file = dir.join(xguard::yara::FILE);
        if let Some(y) = xguard::yara::Rules::open(&file) {
            return Ok(Some(Box::new(Box::leak(Box::new(y)).session())));
        }
        if file.exists() {
            eprintln!(
                "wol-xguard: {}: the rule sets do not load; results are incomplete",
                file.display()
            );
            return Err(());
        }
    }
    let _ = dir;
    Ok(None)
}

/// No rules file at all in `dir` (as opposed to one that does not load).
fn deep_missing(dir: &Path) -> bool {
    #[cfg(feature = "yara")]
    return !dir.join(xguard::yara::FILE).exists();
    #[cfg(not(feature = "yara"))]
    {
        let _ = dir;
        true
    }
}

/// The value of `--rules`: present, not another flag, an existing directory. A missing value
/// must not swallow the next path, and a typo must not silently mean "no signatures".
fn rules_arg(v: Option<&String>) -> PathBuf {
    match v {
        Some(d) if !d.starts_with('-') && Path::new(d).is_dir() => PathBuf::from(d),
        Some(d) => die(&format!("--rules {d}: not a directory")),
        None => die("--rules needs a directory"),
    }
}

fn scan(args: &[String]) -> i32 {
    let (mut json, mut need_clam, mut dir, mut paths) = (false, false, None, Vec::new());
    let mut stream = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => json = true,
            // ClamAV must answer: without it the result is Incomplete, not clean.
            "--clamav" => need_clam = true,
            // The guard's deep check of a file above its memory cap: pieces, never whole (R2C-4).
            "--stream" => stream = true,
            "--rules" => dir = Some(rules_arg(it.next())),
            // Everything after `--` is a path, even one that starts with `-` (S4-16).
            "--" => {
                paths.extend(it.by_ref().cloned());
                break;
            }
            p => paths.push(p.to_owned()),
        }
    }
    let dir = dir.unwrap_or_else(xguard::rules_dir);
    let e = Engine::open(&dir).unwrap_or_else(|m| die(&m));
    let (extra, rules_ok) = match deep(&dir) {
        Ok(d) => (d, true),
        Err(()) => (None, false),
    };
    let clam = xguard::clam::socket(
        std::env::var_os(xguard::clam::SOCKET_ENV).map(PathBuf::from),
        Path::new(xguard::clam::DB_DIR),
    );
    let (mut worst, mut found) = (0, false);
    for p in paths {
        // `-`: the file is on standard input (the guard passes it this way, with no path to open).
        let opened = if p == "-" {
            use std::os::fd::AsFd;
            std::io::stdin()
                .as_fd()
                .try_clone_to_owned()
                .map(std::fs::File::from)
        } else {
            std::fs::File::open(&p)
        };
        // One unreadable file must not cost the caller the answers for the rest.
        let mut f = match opened {
            Ok(f) => f,
            Err(m) => {
                eprintln!("wol-xguard: {p}: {m}");
                worst = worst.max(3);
                continue;
            }
        };
        let scanned = if stream {
            xguard::walk::scan_stream(&e, &mut f, &p, extra.as_deref())
        } else {
            xguard::scan_file(&e, &mut f, &p, extra.as_deref(), Limits::default())
        };
        let mut r: Report = scanned.unwrap_or_else(|m| die(&format!("{p}: {m}")));
        if !rules_ok {
            r.skipped.push(xguard::Skipped {
                path: p.clone(),
                why: xguard::guard::RULES_DID_NOT_LOAD.into(),
            });
            if r.verdict == Verdict::Clean {
                r.verdict = Verdict::Incomplete;
            }
        }
        if let Some(s) = &clam {
            use std::io::Seek;
            let _ = f.rewind();
            let a = xguard::clam::ask(s, &f, xguard::clam::WAIT);
            xguard::clam::merge(&mut r, a, &p);
        } else if need_clam {
            let a = xguard::clam::Answer::NotRun("no signatures on this computer".into());
            xguard::clam::merge(&mut r, a, &p);
        }
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
        found |= r.verdict == Verdict::Found;
        worst = worst.max(match r.verdict {
            Verdict::Incomplete => 2,
            _ => 0,
        });
    }
    let _ = std::io::stdout().flush();
    if found { 1 } else { worst }
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
    shared::atomic::atomic_write(std::path::Path::new(out), 0o644, &bytes)
        .unwrap_or_else(|m| die(&format!("{out}: {m}")));
    println!("{}", serde_json::to_string(&st).unwrap_or_default());
    println!("{out}: {} bytes", bytes.len());
    0
}

fn guard(args: &[String]) -> i32 {
    let mut o = xguard::guard::Options {
        state: xguard::guard::STATE_DIR.into(),
        rules: xguard::rules_dir(),
        scope: Vec::new(),
        downloads: Vec::new(),
        worker: std::env::current_exe().ok(),
        socket: xguard::ctl::SOCKET.into(),
    };
    let mut it = args.iter();
    while let (Some(k), Some(v)) = (it.next(), it.next()) {
        match k.as_str() {
            "--state" => o.state = v.into(),
            "--rules" => o.rules = rules_arg(Some(v)),
            "--scope" => o.scope.push(v.into()),
            "--downloads" => o.downloads.push(v.into()),
            "--socket" => o.socket = v.into(),
            _ => die("usage: guard [--state D] [--rules D] [--scope M]... [--downloads D]..."),
        }
    }
    match xguard::guard::run(o) {
        Ok(()) => 0,
        Err(m) => die(&m),
    }
}

/// `check PATH` and `allow PATH`: through the guard; a check still works without it.
fn ask(verb: &str, args: &[String]) -> i32 {
    let (sock, path) = match args {
        [p] => (xguard::ctl::SOCKET.to_owned(), p),
        [flag, s, p] if flag == "--socket" => (s.clone(), p),
        _ => die("usage: check|allow [--socket S] PATH"),
    };
    let f = std::fs::File::open(path).unwrap_or_else(|m| die(&format!("{path}: {m}")));
    match xguard::ctl::ask(Path::new(&sock), verb, &f) {
        Ok(r) if r.state == "found" => {
            println!("{}", r.name);
            1
        }
        // The guard could not finish the check (see its log): like an incomplete scan.
        Ok(r) if r.state == "unchecked" => 2,
        Ok(_) => 0,
        Err(_) if verb == "check" => scan(std::slice::from_ref(path)),
        Err(m) => die(&m.to_string()),
    }
}

/// The background pass (a timer starts it).
fn sweep(args: &[String]) -> i32 {
    use xguard::sweep::{gate, lowest_qos, pass, targets};
    let (dir, args) = match args {
        [flag, rest @ ..] if flag == "--rules" => {
            (rules_arg(rest.first()), rest.get(1..).unwrap_or(&[]))
        }
        _ => (xguard::rules_dir(), args),
    };
    let state = args
        .first()
        .map_or(xguard::guard::STATE_DIR, String::as_str)
        .to_owned();
    let roots: Vec<PathBuf> = args.iter().skip(1).map(PathBuf::from).collect();
    let open = || {
        args.len() > 1
            || gate(
                Path::new("/sys/class/power_supply"),
                Path::new("/run/user"),
                Path::new("/proc"),
            )
    };
    if !open() {
        return 0;
    }
    lowest_qos();
    let e = Engine::open(&dir).unwrap_or_else(|m| die(&m));
    let rules = e.versions().iter().copied().max().unwrap_or(0);
    let store = std::sync::Mutex::new(
        xguard::store::Store::open(&Path::new(&state).join("verdicts"))
            .unwrap_or_else(|m| die(&m.to_string())),
    );
    let mut files = Vec::new();
    let tagged = |p: &Path| {
        std::fs::File::open(p).is_ok_and(|f| {
            use std::os::fd::AsRawFd;
            xguard::guard::is_tagged(f.as_raw_fd())
        })
    };
    let home = [PathBuf::from("/home")];
    for r in if roots.is_empty() {
        &home[..]
    } else {
        &roots[..]
    } {
        targets(r, &tagged, &mut files);
    }
    let found = std::sync::Mutex::new(Vec::new());
    let finished = pass(&files, &open, &|p| {
        thread_local!(static DEEP: std::cell::OnceCell<Option<Box<dyn xguard::Deep>>> =
            const { std::cell::OnceCell::new() });
        let Ok(mut f) = std::fs::File::open(p) else {
            return;
        };
        let Ok(id) = xguard::store::FileId::of(&f) else {
            return;
        };
        let lock = || store.lock().unwrap_or_else(|p| p.into_inner());
        if lock()
            .by_id(&id)
            .is_some_and(|k| k.deep && (k.rules >= rules || k.state != xguard::store::State::Clean))
        {
            return;
        }
        let name = p.to_string_lossy();
        // Rule sets that do not load: the pass still hashes and matches, but the file is not
        // remembered as deeply checked, so a later pass looks again (audit S4).
        let mut rules_ok = true;
        let Ok(r) = DEEP.with(|c| {
            let extra = c.get_or_init(|| deep(&dir).ok().flatten());
            rules_ok = extra.is_some() || !cfg!(feature = "yara") || deep_missing(&dir);
            xguard::scan_file(&e, &mut f, &name, extra.as_deref(), Limits::default())
        }) else {
            return;
        };
        let mut sha = [0u8; 16];
        for (i, b) in sha.iter_mut().enumerate() {
            *b =
                u8::from_str_radix(r.sha256.get(i * 2..i * 2 + 2).unwrap_or("00"), 16).unwrap_or(0);
        }
        let state = if r.findings.is_empty() {
            xguard::store::State::Clean
        } else {
            xguard::store::State::Found
        };
        lock().put(
            &id,
            xguard::store::Entry {
                state,
                rules,
                deep: rules_ok,
                sha,
            },
        );
        if let Some(n) = r.findings.first() {
            let mut l = found.lock().unwrap_or_else(|p| p.into_inner());
            l.push(wol_core::malware::Blocked {
                path: name.into_owned(),
                name: n.name.clone(),
                sha256: r.sha256.clone(),
                when: xguard::scan::now(),
                uid: {
                    use std::os::unix::fs::MetadataExt;
                    f.metadata().map_or(0, |m| m.uid())
                },
            });
        }
    });
    let l: Vec<wol_core::malware::Blocked> = found.into_inner().unwrap_or_default();
    let mut owners: Vec<u32> = l.iter().map(|b| b.uid).collect();
    owners.sort_unstable();
    owners.dedup();
    for uid in owners {
        let mine: Vec<_> = l.iter().filter(|b| b.uid == uid).collect();
        let t = serde_json::to_string(&mine).unwrap_or_default();
        let p = Path::new(&state).join(wol_core::malware::sweep_file(uid));
        // S4-10: owner and 0600 before the list appears; atomic, so a reader never sees half.
        // The owner change needs CAP_CHOWN in wolos-xguard-sweep.service (audit R7-1).
        if let Err(e) = shared::atomic::write_as(&p, 0o600, Some(uid), t.as_bytes()) {
            eprintln!("wol-xguard: could not write {}: {e}", p.display());
        }
    }
    println!(
        "{} files, {}",
        files.len(),
        if finished.is_ok() {
            "finished"
        } else {
            "stopped"
        }
    );
    0
}

/// `rules yara OUT SOURCE-DIR...`: compile rule sources.
#[cfg(feature = "yara")]
fn yara_build(args: &[String]) -> i32 {
    let [out, dirs @ ..] = args else {
        die("usage: rules yara OUT SOURCE-DIR...")
    };
    fn walk(d: &Path, s: &mut Vec<(String, String)>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, s);
            } else if p.extension().is_some_and(|x| x == "yara" || x == "yar")
                && let Ok(t) = std::fs::read_to_string(&p)
            {
                s.push((p.display().to_string(), t));
            }
        }
    }
    let mut sources = Vec::new();
    dirs.iter().for_each(|d| walk(Path::new(d), &mut sources));
    sources.sort();
    let (b, refused) = xguard::yara::compile(&sources).unwrap_or_else(|m| die(&m));
    std::fs::write(out, &b).unwrap_or_else(|m| die(&m.to_string()));
    println!(
        "{} sources, {refused} refused, {} bytes",
        sources.len(),
        b.len()
    );
    0
}

/// `rules bundle DIR VERSION`: the manifest to sign; `rules install SRC [DEST]`: a signed update.
fn bundle(args: &[String]) -> i32 {
    let [dir, version] = args else {
        die("usage: rules bundle DIR VERSION")
    };
    let v = version.parse().unwrap_or_else(|_| die("bad version"));
    let b = xguard::bundle::describe(Path::new(dir), v, xguard::scan::now())
        .unwrap_or_else(|m| die(&m));
    let t = serde_json::to_vec(&b).unwrap_or_default();
    std::fs::write(Path::new(dir).join(xguard::bundle::MANIFEST), t)
        .unwrap_or_else(|m| die(&m.to_string()));
    0
}

fn install(args: &[String]) -> i32 {
    let (src, dest) = match args {
        [s] => (s.as_str(), xguard::RULES_DIR),
        [s, d] => (s.as_str(), d.as_str()),
        _ => die("usage: rules install SRC [DEST]"),
    };
    let keys = wol_core::package::read_keys(Path::new(shared::package::KEYS_DIR));
    match xguard::bundle::install(Path::new(src), Path::new(dest), &keys) {
        Ok(v) => {
            println!("signatures {v} installed");
            0
        }
        Err(m) => die(&m),
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let code = match a.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["scan", ..] => scan(&a[1..]),
        ["guard", ..] => guard(&a[1..]),
        ["check", ..] => ask("check", &a[1..]),
        ["allow", ..] => ask("allow", &a[1..]),
        ["sweep", ..] => sweep(&a[1..]),
        ["rules", "bundle", ..] => bundle(&a[2..]),
        ["rules", "install", ..] => install(&a[2..]),
        #[cfg(feature = "yara")]
        ["rules", "yara", ..] => yara_build(&a[2..]),
        ["rules", "build", ..] => build(&a[2..]),
        _ => die("usage: wol-xguard scan|check|allow|guard|sweep|rules build|bundle|install"),
    };
    std::process::exit(code)
}
