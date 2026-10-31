// SPDX-License-Identifier: GPL-2.0-or-later
//! The upstream tree's build scripts as one graph. Only files that run at build or install time are
//! read, each once per content hash, in parallel. Files read by nothing here are not judged.
use crate::graph::{Graph, Stage};
use crate::langs::{self, Lang};
use crate::shell::{Scope, Walker};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use tree_sitter::Node;

const MAX_FILE: u64 = 2_000_000;
/// Generated `configure` scripts run long, and they are exactly where a build hides things.
const MAX_CONFIGURE: u64 = 8_000_000;
const MAX_FILES: usize = 20_000;
/// Bump when analysis changes, so old cached graphs are not reused.
const CACHE_VERSION: u32 = 3;
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
    /// The package's own text: what it names, it may run.
    pub mention: String,
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
    /// `run_command` lines of a `meson.build`.
    Meson,
    /// A build backend the tree itself provides (PEP 517): Python, judged in full.
    Backend,
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
    let stem = name.split('.').next().unwrap_or(name);
    let generator = matches!(stem, "gen" | "generate" | "build" | "mkall")
        || ["gen_", "gen-", "mk_", "mk-", "build_", "build-"]
            .iter()
            .any(|p| stem.starts_with(p));
    let ext = name.rsplit_once('.').map(|x| x.1).unwrap_or("");
    Some(match (name, ext) {
        ("package.json", _) => Role::Npm,
        ("meson.build", _) => Role::Meson,
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

/// Files a build runs on its way in. A helper they call is judged by what flows, as a build
/// system is; these are judged in full.
fn entry(rel: &str) -> bool {
    matches!(
        rel.rsplit('/').next().unwrap_or(rel),
        "setup.py"
            | "build.py"
            | "install.py"
            | "wscript"
            | "noxfile.py"
            | "build.rs"
            | "install.js"
            | "postinstall.js"
            | "preinstall.js"
            | "build.js"
    )
}

/// C that runs before `main` whether or not anyone calls it, the way the xz backdoor hooked in.
fn runs_before_main(rel: &str, path: &Path) -> bool {
    let ext = rel.rsplit_once('.').map_or("", |x| x.1);
    if !matches!(ext, "c" | "cc" | "cpp" | "cxx")
        || rel
            .rsplit_once('/')
            .is_some_and(|x| x.0.split('/').any(idle))
    {
        return false;
    }
    let small = std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_FILE);
    small
        && std::fs::read(path).is_ok_and(|b| {
            let t = String::from_utf8_lossy(&b);
            t.contains("__attribute__") && (t.contains("constructor") || t.contains("ifunc"))
        })
}

/// Modules `setup.py` imports from the source tree run with it, however quiet their names.
fn local_imports(root: &Path, files: &mut Vec<(String, Role)>) {
    let mut queue: Vec<String> = files
        .iter()
        .filter(|(r, _)| r.rsplit('/').next() == Some("setup.py"))
        .map(|(r, _)| r.clone())
        .collect();
    let mut left = 50;
    while let Some(rel) = queue.pop() {
        let Ok(text) = std::fs::read_to_string(root.join(&rel)) else {
            continue;
        };
        let dir = rel.rsplit_once('/').map_or("", |x| x.0).to_owned();
        for l in text.lines() {
            let l = l.trim();
            let mods: Vec<&str> = if let Some(r) = l.strip_prefix("from ") {
                r.split_whitespace().next().into_iter().collect()
            } else if let Some(r) = l.strip_prefix("import ") {
                r.split(',')
                    .filter_map(|m| m.split_whitespace().next())
                    .collect()
            } else {
                continue;
            };
            for m in mods {
                let stem = m.trim_start_matches('.').replace('.', "/");
                let at = |p: String| {
                    if dir.is_empty() {
                        p
                    } else {
                        format!("{dir}/{p}")
                    }
                };
                for c in [at(format!("{stem}.py")), at(format!("{stem}/__init__.py"))] {
                    if left > 0
                        && !stem.is_empty()
                        && root.join(&c).is_file()
                        && !files.iter().any(|(r, _)| *r == c)
                    {
                        left -= 1;
                        files.push((c.clone(), Role::Code(Lang::Python)));
                        queue.push(c);
                    }
                }
            }
        }
    }
}

/// The command in a `run_command('sh', '-c', '...')` line.
fn meson_command(line: &str) -> Option<String> {
    let rest = line.split_once("run_command(")?.1;
    let q: Vec<&str> = rest.split('\'').skip(1).step_by(2).collect();
    match q.as_slice() {
        [_, "-c", c, ..] => Some((*c).to_owned()),
        [] => None,
        all => Some(all.join(" ")),
    }
}

/// The module file a `pyproject.toml` names as its in-tree build backend.
fn backend_file(text: &str, dir: &str) -> Option<String> {
    let value = |key: &str| {
        text.lines()
            .find(|l| l.trim_start().starts_with(key))
            .and_then(|l| l.split('"').nth(1))
    };
    let module = value("build-backend")?.split(':').next()?.replace('.', "/");
    let path = value("backend-path")?;
    let p = if dir.is_empty() {
        format!("{path}/{module}.py")
    } else {
        format!("{dir}/{path}/{module}.py")
    };
    Some(p.replace("/./", "/").trim_start_matches("./").to_owned())
}

/// Scripts deep in a tree count only if something the build uses mentions them: the PKGBUILD, a
/// build file, a top-level script, or a script already reached. A glob over the folder counts.
fn keep_reached(root: &Path, files: &mut Vec<(String, Role)>, mention: &str) {
    let gated = |rel: &str, r: Role| {
        let inner = rel.strip_prefix("src/").unwrap_or(rel);
        let name = rel.rsplit('/').next().unwrap_or(rel);
        inner.matches('/').count() >= 2
            && !matches!(name, "configure" | "autogen.sh" | "bootstrap")
            && matches!(
                r,
                Role::Shell | Role::Code(Lang::Python | Lang::Js | Lang::Go)
            )
    };
    let read = |rel: &str| std::fs::read(root.join(rel)).unwrap_or_default();
    let mut text = mention.as_bytes().to_vec();
    let mut waiting = Vec::new();
    let mut kept = Vec::new();
    for (rel, r) in files.drain(..) {
        if gated(&rel, r) {
            waiting.push((rel, r));
        } else {
            text.extend(read(&rel));
            text.push(b'\n');
            kept.push((rel, r));
        }
    }
    let seen = |hay: &[u8], s: &str| hay.windows(s.len().max(1)).any(|w| w == s.as_bytes());
    loop {
        let (hit, rest): (Vec<_>, Vec<_>) =
            waiting.into_iter().partition(|(rel, r): &(String, Role)| {
                let mut parts = rel.rsplit('/');
                let name = parts.next().unwrap_or(rel);
                let dir = parts.next().unwrap_or("");
                let word = match r {
                    Role::Code(_) if !entry(rel) => name.rsplit_once('.').map_or(name, |x| x.0),
                    _ => name,
                };
                seen(&text, word) || seen(&text, &format!("{dir}/*"))
            });
        waiting = rest;
        if hit.is_empty() {
            break;
        }
        for (rel, r) in hit {
            text.extend(read(&rel));
            text.push(b'\n');
            kept.push((rel, r));
        }
    }
    *files = kept;
}

/// What upstream's `.gitmodules` files list, near the top of a tree.
pub fn submodules(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if depth < 2 && e.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push((p, depth + 1));
            } else if e.file_name() == ".gitmodules"
                && let Ok(t) = std::fs::read_to_string(&p)
            {
                out.extend(t.lines().filter_map(|l| {
                    let (k, v) = l.split_once('=')?;
                    (k.trim() == "url").then(|| v.trim().to_owned())
                }));
            }
        }
    }
    out
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
                let found = role(&rel)
                    .or_else(|| runs_before_main(&rel, &p).then_some(Role::Code(Lang::C)));
                if e.file_name() == "pyproject.toml" {
                    let dir = rel.strip_suffix("pyproject.toml").unwrap_or("");
                    if let Some(f) = std::fs::read_to_string(&p)
                        .ok()
                        .and_then(|t| backend_file(&t, dir.trim_end_matches('/')))
                        && root.join(&f).is_file()
                    {
                        out.push((f, Role::Backend));
                    }
                }
                if let Some(r) = found {
                    if out.len() >= MAX_FILES {
                        too_many += 1;
                    } else {
                        out.push((rel, r));
                    }
                }
            }
        }
    }
    local_imports(root, &mut out);
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

/// What the build asks `make` for. Recipes of other targets (lint, release, docs) never run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct MakeCalls {
    /// Unknown, so every recipe counts: no package text, or a target only known at run time.
    all: bool,
    /// `make` with no target: the file's first rule.
    default: bool,
    targets: BTreeSet<String>,
}

impl MakeCalls {
    fn key(&self) -> String {
        let t: Vec<&str> = self.targets.iter().map(String::as_str).collect();
        format!("{} {} {}", self.all, self.default, t.join(" "))
    }

    /// Reads every `make` command in `text`.
    fn read(&mut self, text: &str) {
        for line in text.lines() {
            let line = line.trim_start();
            if line.starts_with('#') {
                continue;
            }
            let spaced = line.replace(['"', '\'', ',', '[', ']'], " ");
            let mut it = spaced.split_whitespace().peekable();
            while let Some(t) = it.next() {
                if !matches!(
                    t.trim_start_matches(['(', '@', '-', '+']),
                    "make" | "gmake" | "$(MAKE)" | "${MAKE}" | "$MAKE"
                ) {
                    continue;
                }
                let mut named = false;
                while let Some(a) = it.next() {
                    if matches!(a, "&&" | "||" | ";" | "|" | ")") {
                        break;
                    }
                    if matches!(a, "-C" | "-f" | "-j" | "-l" | "--directory" | "--file") {
                        it.next();
                    } else if a.starts_with('-') || a.contains('=') {
                    } else if a.contains(['$', '`', '*']) {
                        self.all = true;
                    } else {
                        named = true;
                        self.targets
                            .insert(a.trim_end_matches([';', ')']).to_owned());
                    }
                    if a.ends_with(';') {
                        break;
                    }
                }
                self.default |= !named;
            }
        }
    }
}

/// The value of `$(X)` and `${X}` in `word`, as far as the file's plain assignments tell.
fn make_expand(word: &str, vars: &HashMap<String, String>, depth: u8) -> String {
    let mut out = word.to_owned();
    for (open, close) in [("$(", ')'), ("${", '}')] {
        while let Some(at) = out.find(open) {
            let Some(end) = out[at..].find(close) else {
                return out;
            };
            let name = &out[at + 2..at + end];
            let Some(v) = vars.get(name).filter(|_| depth < 6) else {
                return out;
            };
            let v = make_expand(v, vars, depth + 1);
            out.replace_range(at..at + end + 1, &v);
        }
    }
    out
}

/// One rule's text: targets, prerequisites, recipe.
fn make_rule(body: &str) -> (Vec<String>, String, String) {
    let joined = body.replace("\\\n", " ");
    let (head, recipe) = joined.split_once('\n').unwrap_or((&joined, ""));
    let (targets, rest) = head.split_once(':').unwrap_or((head, ""));
    let rest = rest.split(';').next().unwrap_or("").replace('|', " ");
    (
        targets.split_whitespace().map(str::to_owned).collect(),
        rest.trim_start_matches(':').to_owned(),
        recipe.to_owned(),
    )
}

/// Start bytes of the rules a build can run; None when that cannot be told.
fn reached_rules(root: Node, text: &str, calls: &MakeCalls) -> Option<HashSet<usize>> {
    if calls.all {
        return None;
    }
    let mut vars: HashMap<String, String> = HashMap::new();
    for l in text.lines().filter(|l| !l.starts_with('\t')) {
        let l = l.trim_start_matches("export ");
        if let Some((k, v)) = l.split_once('=') {
            let k = k.trim_end_matches([':', '+', '?', ' ']).trim();
            if !k.is_empty()
                && k.chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
            {
                let e = vars.entry(k.to_owned()).or_default();
                e.push(' ');
                e.push_str(v.trim());
            }
        }
    }
    let mut rules = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind() == "rule" {
            let (t, pre, rec) = make_rule(n.utf8_text(text.as_bytes()).unwrap_or(""));
            rules.push((n.start_byte(), t, pre, rec));
        } else {
            let mut c = n.walk();
            stack.extend(n.named_children(&mut c));
        }
    }
    rules.sort_by_key(|r| r.0);
    let mut want = calls.targets.clone();
    if calls.default
        && let Some(t) = rules
            .iter()
            .flat_map(|r| &r.1)
            .find(|t| !t.starts_with('.') && !t.contains('%'))
    {
        want.insert(t.clone());
    }
    let mut seen = HashSet::new();
    loop {
        let before = seen.len();
        for (at, t, pre, rec) in &rules {
            let hit = t.iter().any(|x| {
                let e = make_expand(x, &vars, 0);
                x.contains('%') || e.contains('$') || e.split_whitespace().any(|w| want.contains(w))
            });
            if !hit || !seen.insert(*at) {
                continue;
            }
            for w in pre.split_whitespace() {
                let e = make_expand(w, &vars, 0);
                if e.contains('$') {
                    return None;
                }
                want.extend(e.split_whitespace().map(str::to_owned));
            }
            let mut more = MakeCalls::default();
            more.read(rec);
            if more.all {
                return None;
            }
            want.extend(more.targets);
        }
        if seen.len() == before {
            return Some(seen);
        }
    }
}

fn make(w: &mut Walker, text: &str, file: &str, calls: &MakeCalls) -> bool {
    let lang = tree_sitter_make::LANGUAGE.into();
    let Some(tree) = langs::parse(&lang, text) else {
        return false;
    };
    let reached = reached_rules(tree.root_node(), text, calls);
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        let line = n.start_position().row as u32 + 1;
        let body = n.utf8_text(text.as_bytes()).unwrap_or("");
        if n.kind() == "rule"
            && reached
                .as_ref()
                .is_some_and(|r| !r.contains(&n.start_byte()))
        {
            continue;
        }
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

fn analyse(rel: &str, r: Role, text: &str, calls: &MakeCalls) -> Entry {
    let mut graph = Graph::new();
    let mut w = Walker::new(&mut graph, Scope::default());
    w.enter_file(rel, Stage::Upstream);
    // Build systems pass commands through variables all the way down, so here only what can be
    // traced flowing into them is judged. A PKGBUILD gets no such allowance.
    w.lenient = !(matches!(r, Role::Code(_)) && entry(rel) || r == Role::Backend);
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
        Role::Make => ok = make(&mut w, text, rel, calls),
        Role::Cmake => ok = cmake(&mut w, text, rel),
        Role::Npm => npm(&mut w, text, rel),
        Role::Backend => ok = langs::walk(Lang::Python, text, rel, Stage::Upstream, &mut w, None),
        Role::Meson => {
            for (i, l) in text.lines().enumerate() {
                if let Some(c) = meson_command(l) {
                    w.snippet(&c, rel, i as u32 + 1, Stage::Upstream);
                }
            }
        }
    }
    let truncated = w.truncated || !ok;
    drop(w);
    Entry { graph, truncated }
}

fn cache_path(dir: &Path, rel: &str, bytes: &[u8], calls: &str) -> PathBuf {
    let mut h = Sha256::new();
    h.update(CACHE_VERSION.to_le_bytes());
    h.update(rel.as_bytes());
    h.update([0]);
    h.update(calls.as_bytes());
    h.update([0]);
    h.update(bytes);
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    dir.join(format!("{hex}.json"))
}

fn one(root: &Path, rel: &str, r: Role, o: &Options, calls: &MakeCalls) -> Option<(Entry, bool)> {
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
    let key = if r == Role::Make {
        calls.key()
    } else {
        String::new()
    };
    let at = o.cache.as_ref().map(|d| cache_path(d, rel, &bytes, &key));
    if let Some(mut e) = at
        .as_ref()
        .and_then(|c| std::fs::read(c).ok())
        .and_then(|b| serde_json::from_slice::<Entry>(&b).ok())
    {
        e.graph.reindex();
        return Some((e, true));
    }
    let e = analyse(rel, r, &String::from_utf8_lossy(&bytes), calls);
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
    let (mut files, too_many) = collect(root);
    keep_reached(root, &mut files, &o.mention);
    let mut calls = MakeCalls {
        all: o.mention.is_empty(),
        ..Default::default()
    };
    calls.read(&o.mention);
    for (rel, r) in &files {
        // `configure` probes make with its own throwaway files.
        let probes = matches!(r, Role::Lines) || rel.ends_with("configure");
        if !matches!(r, Role::Make) && !probes {
            calls.read(&String::from_utf8_lossy(
                &std::fs::read(root.join(rel)).unwrap_or_default(),
            ));
        }
    }
    let calls = &calls;
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
                    let e = one(root, rel, *r, o, calls);
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

#[cfg(test)]
mod make_reach {
    use super::*;

    fn calls(text: &str) -> MakeCalls {
        let mut c = MakeCalls::default();
        c.read(text);
        c
    }

    fn reached(mk: &str, c: &MakeCalls) -> Option<usize> {
        let lang = tree_sitter_make::LANGUAGE.into();
        let tree = langs::parse(&lang, mk).unwrap();
        reached_rules(tree.root_node(), mk, c).map(|s| s.len())
    }

    const MK: &str = "BIN = app\nall: $(BIN)\n$(BIN): main.c\n\tcc main.c\ninstall:\n\tcp app /usr\nlint:\n\tgo run x@latest\n";

    #[test]
    fn a_recipe_nothing_asks_for_does_not_run() {
        assert_eq!(
            reached(MK, &calls("  make\n  make DESTDIR=x install\n")),
            Some(3)
        );
        assert_eq!(reached(MK, &calls("cmake --build .\n")), Some(0));
    }

    #[test]
    fn a_target_known_only_at_run_time_keeps_them_all() {
        assert!(calls("make $goal\n").all);
        assert_eq!(reached(MK, &calls("make $goal\n")), None);
    }

    #[test]
    fn the_command_ends_where_the_shell_says() {
        let c = calls("make -C sub all && rm x\nsubprocess.run([\"make\", \"dist\"])\n");
        assert!(c.targets.contains("all") && c.targets.contains("dist"));
        assert!(!c.targets.contains("rm") && !c.targets.contains("sub"));
    }
}
