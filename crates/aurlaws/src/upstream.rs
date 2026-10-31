// SPDX-License-Identifier: GPL-2.0-or-later
//! The upstream tree's build scripts as one graph. Only files that run at build or install time are
//! read, each once per content hash, in parallel. Files read by nothing here are not judged.
use crate::graph::{Graph, Stage};
use crate::langs::{self, Lang};
use crate::shell::{Scope, Walker};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

const MAX_FILE: u64 = 2_000_000;
/// Generated `configure` scripts run long, and they are exactly where a build hides things.
const MAX_CONFIGURE: u64 = 8_000_000;
const MAX_FILES: usize = 20_000;
/// Bump when analysis changes, so old cached graphs are not reused.
const CACHE_VERSION: u32 = 1;
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "__pycache__",
];

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub cache: Option<PathBuf>,
    /// Worker threads; 0 means up to 8, as the machine allows.
    pub jobs: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Scan {
    pub graph: Graph,
    pub parsed: usize,
    pub cached: usize,
    /// Build scripts too big to read.
    pub skipped: usize,
    /// Build scripts that hit a limit while being read.
    pub truncated: usize,
}

impl Scan {
    /// What was not read, in plain sentences.
    pub fn gaps(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.skipped > 0 {
            out.push(format!(
                "{} upstream build scripts were too big to read, so they were not judged.",
                self.skipped
            ));
        }
        if self.truncated > 0 {
            out.push(format!("{} upstream build scripts went past the analyser's limits, so part of them was not read.", self.truncated));
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Shell,
    /// Macro files: line by line, since m4 quoting is not shell.
    Lines,
    Code(Lang),
    Make,
    Cmake,
    Npm,
}

/// Folders a package build does not run: CI, tests, documents. A build can still reach into them,
/// and the trace is what would show it.
fn idle(d: &str) -> bool {
    let d = d.to_ascii_lowercase();
    d.starts_with("test")
        || d.starts_with(".git")
        || d.starts_with(".circleci")
        || matches!(
            d.as_str(),
            "ci" | "t"
                | "examples"
                | "example"
                | "docs"
                | "doc"
                | "demo"
                | "demos"
                | "bench"
                | "benchmarks"
                | "docker"
                | "packaging"
        )
}

fn role(rel: &str) -> Option<Role> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let dir = rel.strip_suffix(name).unwrap_or("").trim_end_matches('/');
    if dir.split('/').any(idle) {
        return None;
    }
    let tooling = dir.split('/').any(|d| {
        matches!(
            d,
            "tools" | "scripts" | "build" | "buildtools" | "build-aux"
        )
    });
    let generator = ["gen", "mk", "build"].iter().any(|p| name.starts_with(p));
    let ext = name.rsplit_once('.').map(|x| x.1).unwrap_or("");
    Some(match (name, ext) {
        ("package.json", _) => Role::Npm,
        ("CMakeLists.txt", _) | (_, "cmake") => Role::Cmake,
        ("Makefile" | "makefile" | "GNUmakefile" | "Makefile.am" | "Makefile.in", _)
        | (_, "mk") => Role::Make,
        ("configure" | "autogen.sh" | "bootstrap", _) => Role::Shell,
        (_, "sh" | "bash") if tooling || dir.is_empty() => Role::Shell,
        ("configure.ac" | "configure.in", _) | (_, "m4") => Role::Lines,
        ("setup.py" | "build.py" | "install.py" | "wscript" | "noxfile.py", _) => {
            Role::Code(Lang::Python)
        }
        ("build.rs", _) => Role::Code(Lang::Rust),
        (_, "py") if tooling => Role::Code(Lang::Python),
        ("install.js" | "postinstall.js" | "preinstall.js" | "build.js", _) => Role::Code(Lang::Js),
        (_, "js" | "mjs" | "cjs") if tooling => Role::Code(Lang::Js),
        (_, "go") if tooling || generator => Role::Code(Lang::Go),
        (_, "c") if tooling || generator => Role::Code(Lang::C),
        _ => return None,
    })
}

fn collect(root: &Path) -> (Vec<(String, Role)>, usize) {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut too_many = 0;
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(t) = e.file_type() else { continue };
            let name = e.file_name();
            if t.is_dir() {
                if !SKIP_DIRS.iter().any(|s| name == *s) {
                    stack.push(e.path());
                }
            } else if t.is_file() {
                let p = e.path();
                let rel = p
                    .strip_prefix(root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                if let Some(r) = role(&rel) {
                    if out.len() >= MAX_FILES {
                        too_many += 1;
                    } else {
                        out.push((rel, r));
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    (out, too_many)
}

/// Make's `$$` and `$(VAR)` as the shell would see them.
fn make_to_shell(line: &str) -> String {
    let s = line.trim_start().trim_start_matches(['@', '-', '+']);
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        match it.peek() {
            Some('$') => {
                it.next();
                out.push('$');
            }
            Some('(') => {
                it.next();
                let (mut depth, mut name) = (1, String::new());
                for x in it.by_ref() {
                    match x {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                    if depth == 0 {
                        break;
                    }
                    name.push(x);
                }
                out.push_str(&format!(
                    "${{{}}}",
                    name.replace(|c: char| !(c.is_alphanumeric() || c == '_'), "_")
                ));
            }
            _ => out.push('$'),
        }
    }
    out
}

fn make(w: &mut Walker, text: &str, file: &str) -> bool {
    let lang = tree_sitter_make::LANGUAGE.into();
    let Some(tree) = langs::parse(&lang, text) else {
        return false;
    };
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        let line = n.start_position().row as u32 + 1;
        let body = n.utf8_text(text.as_bytes()).unwrap_or("");
        match n.kind() {
            "recipe_line" => w.snippet(&make_to_shell(body), file, line, Stage::Upstream),
            "shell_function" => {
                let inner = body
                    .trim_start_matches("$(shell")
                    .trim_start_matches("${shell")
                    .trim_end_matches([')', '}']);
                w.snippet(&make_to_shell(inner), file, line, Stage::Upstream);
            }
            _ => {
                let mut c = n.walk();
                let kids: Vec<_> = n.named_children(&mut c).collect();
                stack.extend(kids.into_iter().rev());
            }
        }
    }
    true
}

const CMAKE_STOP: &[&str] = &[
    "WORKING_DIRECTORY",
    "OUTPUT_VARIABLE",
    "ERROR_VARIABLE",
    "RESULT_VARIABLE",
    "OUTPUT_QUIET",
    "ERROR_QUIET",
    "OUTPUT_FILE",
    "ERROR_FILE",
    "TIMEOUT",
    "DEPENDS",
    "COMMENT",
    "BYPRODUCTS",
    "OUTPUT",
    "VERBATIM",
    "USES_TERMINAL",
    "COMMAND_ERROR_IS_FATAL",
    "OUTPUT_STRIP_TRAILING_WHITESPACE",
    "ERROR_STRIP_TRAILING_WHITESPACE",
    "MAIN_DEPENDENCY",
    "ALL",
];

fn cmake(w: &mut Walker, text: &str, file: &str) -> bool {
    let lang = tree_sitter_cmake::LANGUAGE.into();
    let Some(tree) = langs::parse(&lang, text) else {
        return false;
    };
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        if n.kind() == "normal_command" {
            let line = n.start_position().row as u32 + 1;
            let mut c = n.walk();
            let kids: Vec<_> = n.named_children(&mut c).collect();
            let name = kids
                .first()
                .and_then(|k| k.utf8_text(text.as_bytes()).ok())
                .unwrap_or("")
                .to_ascii_lowercase();
            let mut args: Vec<String> = Vec::new();
            for k in &kids[1.min(kids.len())..] {
                let mut c2 = k.walk();
                for a in k.named_children(&mut c2) {
                    args.push(
                        a.utf8_text(text.as_bytes())
                            .unwrap_or("")
                            .trim_matches('"')
                            .to_owned(),
                    );
                }
            }
            match name.as_str() {
                "file" if args.first().is_some_and(|a| a == "DOWNLOAD") => {
                    let hashed = args
                        .iter()
                        .any(|a| a == "EXPECTED_HASH" || a == "EXPECTED_MD5");
                    if let (Some(url), Some(dest)) = (args.get(1), args.get(2)) {
                        w.download(url, dest, hashed, file, line);
                    }
                }
                "execute_process" | "add_custom_command" | "add_custom_target" => {
                    let mut it = args.iter().peekable();
                    while let Some(a) = it.next() {
                        if a != "COMMAND" {
                            continue;
                        }
                        let mut argv = Vec::new();
                        while let Some(x) = it.peek() {
                            if *x == "COMMAND" || CMAKE_STOP.contains(&x.as_str()) {
                                break;
                            }
                            argv.push((*x).clone());
                            it.next();
                        }
                        w.argv_text(argv, file, line);
                    }
                }
                _ => {}
            }
        }
        // In file order: a check only counts if it comes before the run.
        let mut c = n.walk();
        let kids: Vec<_> = n.named_children(&mut c).collect();
        stack.extend(kids.into_iter().rev());
    }
    true
}

const LIFECYCLE: &[&str] = &[
    "preinstall",
    "install",
    "postinstall",
    "prepare",
    "prepublish",
];

fn npm(w: &mut Walker, text: &str, file: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let Some(s) = v.get("scripts").and_then(|s| s.as_object()) else {
        return;
    };
    for k in LIFECYCLE {
        if let Some(cmd) = s.get(*k).and_then(|c| c.as_str()) {
            let line = text
                .lines()
                .position(|l| l.contains(&format!("\"{k}\"")))
                .map_or(1, |i| i as u32 + 1);
            w.snippet(cmd, file, line, Stage::Upstream);
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Entry {
    graph: Graph,
    truncated: bool,
}

fn analyse(rel: &str, r: Role, text: &str) -> Entry {
    let mut graph = Graph::new();
    let mut w = Walker::new(&mut graph, Scope::default());
    w.enter_file(rel, Stage::Upstream);
    // Build systems pass commands through variables all the way down, so here only what can be
    // traced flowing into them is judged. A PKGBUILD gets no such allowance.
    w.lenient = !matches!(r, Role::Code(_));
    let mut ok = true;
    match r {
        Role::Shell => w.walk_file(text, rel, Stage::Upstream),
        Role::Lines => {
            for (i, l) in text.lines().enumerate() {
                let t = l.trim();
                if !t.is_empty() && !t.starts_with("dnl") && !t.starts_with('#') {
                    w.snippet(t, rel, i as u32 + 1, Stage::Upstream);
                }
            }
        }
        Role::Code(lang) => ok = langs::walk(lang, text, rel, Stage::Upstream, &mut w, None),
        Role::Make => ok = make(&mut w, text, rel),
        Role::Cmake => ok = cmake(&mut w, text, rel),
        Role::Npm => npm(&mut w, text, rel),
    }
    let truncated = w.truncated || !ok;
    drop(w);
    Entry { graph, truncated }
}

fn cache_path(dir: &Path, rel: &str, bytes: &[u8]) -> PathBuf {
    let mut h = Sha256::new();
    h.update(CACHE_VERSION.to_le_bytes());
    h.update(rel.as_bytes());
    h.update([0]);
    h.update(bytes);
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    dir.join(format!("{hex}.json"))
}

fn one(root: &Path, rel: &str, r: Role, o: &Options) -> Option<(Entry, bool)> {
    let p = root.join(rel);
    let cap = if rel.ends_with("configure") {
        MAX_CONFIGURE
    } else {
        MAX_FILE
    };
    if std::fs::metadata(&p).ok()?.len() > cap {
        return None;
    }
    let bytes = std::fs::read(&p).ok()?;
    let at = o.cache.as_ref().map(|d| cache_path(d, rel, &bytes));
    if let Some(mut e) = at
        .as_ref()
        .and_then(|c| std::fs::read(c).ok())
        .and_then(|b| serde_json::from_slice::<Entry>(&b).ok())
    {
        e.graph.reindex();
        return Some((e, true));
    }
    let e = analyse(rel, r, &String::from_utf8_lossy(&bytes));
    if let (Some(c), Ok(json)) = (at, serde_json::to_vec(&e)) {
        if let Some(d) = c.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        // A cache that cannot be written only costs time.
        let tmp = c.with_extension(format!("tmp{}", std::process::id()));
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &c);
        }
    }
    Some((e, false))
}

/// Read every build script under `root`.
pub fn scan(root: &Path, o: &Options) -> Scan {
    let (files, too_many) = collect(root);
    let jobs = if o.jobs > 0 {
        o.jobs
    } else {
        std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8)
    }
    .min(files.len().max(1));
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<Option<(Entry, bool)>>> = Mutex::new(files.iter().map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((rel, r)) = files.get(i) else { break };
                    let e = one(root, rel, *r, o);
                    if let Ok(mut d) = done.lock() {
                        d[i] = e;
                    }
                }
            });
        }
    });
    let mut out = Scan {
        skipped: too_many,
        ..Scan::default()
    };
    for e in done.into_inner().unwrap_or_default() {
        match e {
            Some((e, cached)) => {
                out.parsed += usize::from(!cached);
                out.cached += usize::from(cached);
                out.truncated += usize::from(e.truncated);
                out.graph.merge(&e.graph);
            }
            None => out.skipped += 1,
        }
    }
    out
}
