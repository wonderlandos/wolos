// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol app new|run|check` (docs/RULES-crystal.md RCR-050 to RCR-053, the Crystal app skill in
//! wolos-crystal/skill/SKILL.md): one way to start, run and check a WolOS app in Crystal.
//!
//! - `new <id> [--template app|cli|tool|ai]` copies a template folder (`$WOLKIT_TEMPLATES`, else
//!   `/usr/share/wolos/templates`) into a new folder named after the app. A `.tmpl` file loses the
//!   suffix and has its placeholders filled; I7 names them `{{id}}` (the reverse-DNS app id),
//!   `{{name}}` (the folder, target and program name) and `{{module}}` (the Crystal module). The
//!   app template also uses the wolkit shard's own names, `{{class_name}}` and `{{app_id}}`, filled
//!   with the same values. Placeholders in file names are filled too.
//! - `run [dir] [--watch]` runs the project's `make run`; `--watch` adds `WOL_APP_WATCH=1` to its
//!   environment and nothing else (the template's Makefile decides what watching means).
//! - `check [dir]` reads every `.cr` file of the project against the per-file RCR rules (the same
//!   scanners `crates/controller/tests/rules_crystal.rs` runs over the repo), the shard pins, the
//!   build files, the public-method docs and the `-Dgc_none` rule, then type-checks with `make
//!   check`. Nothing here needs root; `new` writes only the new folder.
use crate::cli::Invocation;
use crate::run::{Env, emit_json, fail};
use serde_json::json;
use shared::errors::ErrorCode;
use std::path::{Path, PathBuf};

// The lexer and the per-file scanners are the test's own (one copy: CR-8 and later rules change
// them there, and `wol app check` follows).
// TODO(ADR 0036, CR-5): move crystal_scan.rs into src/ once CR-8 has merged (CTO, 2026-11-03).
#[path = "../tests/support/crystal_scan.rs"]
mod crystal_scan;

/// Where an installed system keeps the templates (the wolkit and wolos-crystal packages).
pub const TEMPLATES_DIR: &str = "/usr/share/wolos/templates";
/// The templates `wol app new` offers (I7), the first is the default.
pub const TEMPLATES: &[&str] = &["app", "cli", "tool", "ai"];
/// The shards an installed system carries under `/usr/lib/wolos/crystal` (the CRYSTAL_PATH entry
/// the wolos-crystal package sets), so a project may require them without a shard.yml entry.
pub const SYSTEM_SHARDS: &[&str] = &["cryswol", "wolkit"];
/// Projects are small: the walk stops after this many files and this depth.
const MAX_FILES: usize = 2000;
const MAX_DEPTH: usize = 8;

/// The three I7 values for one new project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Names {
    /// Reverse-DNS app id (`org.wolos.notes`, or the id given).
    pub id: String,
    /// Folder, shard, target and program name (`notes`, `my-notes`).
    pub name: String,
    /// Crystal module or class name (`Notes`, `MyNotes`).
    pub module: String,
}

fn is_name(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=32).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_')
}

/// `notes` or `org.example.notes` -> the three values. The name is a lower-case word (letters,
/// digits, `-`, `_`, at most 32, a letter first: safe as a folder, a target and a class); its module
/// must not be a standard-library or WolOS shard name (`file` would reopen `File`).
pub fn names(arg: &str) -> Result<Names, String> {
    let (id, name) = match arg.rsplit_once('.') {
        Some((_, last)) => {
            let ok = arg.split('.').all(is_name) && arg.len() <= 128;
            if !ok {
                return Err(format!(
                    "{arg} is not an app id: lower-case words joined by dots, like org.example.notes."
                ));
            }
            (arg.to_owned(), last.to_owned())
        }
        None => {
            if !is_name(arg) {
                return Err(format!(
                    "{arg} is not a project name: a lower-case letter, then letters, digits, '-' or '_' (at most 32)."
                ));
            }
            (
                format!("org.wolos.{}", arg.replace(['-', '_'], "")),
                arg.to_owned(),
            )
        }
    };
    let module: String = name
        .split(['-', '_'])
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            c.next()
                .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                .unwrap_or_default()
        })
        .collect();
    let plain = module.to_ascii_lowercase();
    if crystal_scan::STDLIB.contains(&plain.as_str())
        || SYSTEM_SHARDS.contains(&plain.as_str())
        || ["wol", "spec", "object", "string", "process"].contains(&plain.as_str())
    {
        return Err(format!(
            "{name} would be the Crystal module {module}, which the standard library or WolOS already has; choose another name."
        ));
    }
    Ok(Names { id, name, module })
}

/// Fill the placeholders of `text`. An unknown `{{word}}` stays as it is.
pub fn fill(text: &str, n: &Names) -> String {
    text.replace("{{id}}", &n.id)
        .replace("{{app_id}}", &n.id)
        .replace("{{name}}", &n.name)
        .replace("{{module}}", &n.module)
        .replace("{{class_name}}", &n.module)
}

/// Every file under `dir` (relative, `/`-separated, sorted), read through `env`. A path that
/// reads as text is a file; anything else is listed as a folder.
fn walk(env: &dyn Env, dir: &Path, skip: &dyn Fn(&str) -> bool) -> Vec<(String, String)> {
    fn go(
        env: &dyn Env,
        root: &Path,
        dir: &Path,
        depth: usize,
        skip: &dyn Fn(&str) -> bool,
        out: &mut Vec<(String, String)>,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        for p in env.list_dir(dir) {
            if out.len() >= MAX_FILES {
                return;
            }
            let p = PathBuf::from(p);
            let rel = p
                .strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            if skip(&rel) {
                continue;
            }
            match env.read_file(&p) {
                Some(text) => out.push((rel, text)),
                None => go(env, root, &p, depth + 1, skip, out),
            }
        }
    }
    let mut out = Vec::new();
    go(env, dir, dir, 0, skip, &mut out);
    out.sort();
    out
}

/// Write the project for `names` from the template folder `source` into `target`; the written
/// paths (relative, sorted). `target` must not hold anything yet.
pub fn render(
    env: &mut dyn Env,
    source: &Path,
    target: &Path,
    names: &Names,
) -> Result<Vec<String>, String> {
    if env.read_file(target).is_some() || !env.list_dir(target).is_empty() {
        return Err(format!(
            "{} exists already; choose another name or folder.",
            target.display()
        ));
    }
    let files = walk(env, source, &|_| false);
    if files.is_empty() {
        return Err(format!("There is no template in {}.", source.display()));
    }
    let mut written = Vec::new();
    for (rel, text) in files {
        let (rel, text) = match rel.strip_suffix(".tmpl") {
            Some(r) => (fill(r, names), fill(&text, names)),
            None => (rel, text),
        };
        let path = target.join(&rel);
        if let Some(parent) = path.parent() {
            env.make_dir(parent)
                .map_err(|e| format!("Cannot make {}: {e}", parent.display()))?;
        }
        env.write_file(&path, &text)
            .map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
        written.push(rel);
    }
    written.sort();
    Ok(written)
}

// ---------------------------------------------------------------- RCR-051, RCR-052 (scanners)

/// One finding: (line, why).
pub type Finding = (usize, String);

/// RCR-052: every public method (`def`, `def self.`, `abstract def`; not `private` or `protected`)
/// has a comment on the line right above it (annotations may sit between). Specs are not asked.
pub fn scan_public_docs(text: &str) -> Vec<Finding> {
    let lx = crystal_scan::lex(text);
    let orig: Vec<&str> = text.lines().collect();
    let mut v = Vec::new();
    for (n, l) in lx.bare_lines() {
        let t = l.trim_start();
        let t = t.strip_prefix("abstract ").unwrap_or(t);
        let Some(rest) = t.strip_prefix("def ") else {
            continue;
        };
        let mut i = n - 1; // index of this line in `orig`
        let documented = loop {
            if i == 0 {
                break false;
            }
            i -= 1;
            let above = orig.get(i).map_or("", |s| s.trim());
            if above.starts_with("@[") {
                continue;
            }
            break above.starts_with('#');
        };
        if !documented {
            let name: String = rest
                .chars()
                .take_while(|c| !matches!(c, '(' | ' ' | ':'))
                .collect();
            v.push((
                n,
                format!("public method `{name}` has no doc comment above it"),
            ));
        }
    }
    v
}

/// RCR-051: the lines of a build file (Makefile, shard.yml, script) that build with `-Dgc_none`
/// (comments left out).
pub fn scan_gc_none(text: &str) -> Vec<Finding> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| {
            let code = l.split('#').next().unwrap_or("");
            code.contains("-Dgc_none")
        })
        .map(|(i, _)| {
            (
                i + 1,
                "`-Dgc_none` builds without the garbage collector; only a short command may (the cli template)".to_string(),
            )
        })
        .collect()
}

// ---------------------------------------------------------------- wol app check

/// One finding of `wol app check`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub file: String,
    pub line: usize,
    pub rule: &'static str,
    pub why: String,
}

/// The rules that do not ask specs (docs/RULES-crystal.md, "Specs ... are held to every rule
/// except"), plus RCR-052.
const NOT_FOR_SPECS: &[&str] = &[
    "RCR-002", "RCR-022", "RCR-024", "RCR-026", "RCR-030", "RCR-035",
];

fn is_spec(rel: &str) -> bool {
    rel.starts_with("spec/") || rel.contains("/spec/") || rel.ends_with("_spec.cr")
}

/// What a project leaves out of its checks: what tools write (`bin/`, `build/`, `lib/` from
/// `shards install`, `src/gen/` from the style compiler) and hidden folders.
fn generated(rel: &str) -> bool {
    let first = rel.split('/').next().unwrap_or("");
    matches!(first, "bin" | "build" | "lib")
        || first.starts_with('.')
        || rel.starts_with("src/gen/")
}

/// Every finding for the project in `dir` (files read through `env`), sorted by file and line.
pub fn check(env: &dyn Env, dir: &Path) -> Vec<Problem> {
    let files = walk(env, dir, &generated);
    let mut out = Vec::new();
    let mut push = |file: &str, list: Vec<(usize, String)>, rule: &'static str| {
        for (line, why) in list {
            out.push(Problem {
                file: file.to_owned(),
                line,
                rule,
                why,
            });
        }
    };
    let yml = files
        .iter()
        .find(|f| f.0 == "shard.yml")
        .map(|f| f.1.as_str());
    let declared: Vec<String> = yml
        .map(|y| {
            crystal_scan::parse_shard_yml(y)
                .1
                .into_iter()
                .map(|d| d.name)
                .collect()
        })
        .unwrap_or_default();
    let mut needs_gc = false;
    for (rel, text) in files.iter().filter(|f| f.0.ends_with(".cr")) {
        let s = crystal_scan::Src::new(rel, text);
        let spec = is_spec(rel);
        for (id, f) in crystal_scan::FILE_SCANNERS {
            if spec && NOT_FOR_SPECS.contains(id) {
                continue;
            }
            let mut found = f(&s);
            if *id == "RCR-043" {
                // A project may require a system shard or one its shard.yml pins (RCR-040 checks
                // the pin); anything else is unknown.
                found.retain(|(_, why)| {
                    let name = why.split('"').nth(1).unwrap_or("");
                    let top = name.split('/').next().unwrap_or("");
                    !SYSTEM_SHARDS.contains(&top) && !declared.iter().any(|d| d == top)
                });
            }
            push(rel, found, id);
        }
        if !spec {
            push(rel, scan_public_docs(text), "RCR-052");
        }
        needs_gc |= s.lx.code.lines().any(|l| {
            let t = l.trim_start();
            SYSTEM_SHARDS
                .iter()
                .any(|sh| t.starts_with(&format!("require \"{sh}")))
        });
    }
    for (rel, text) in &files {
        let base = rel.rsplit('/').next().unwrap_or(rel);
        let build = base == "Makefile" || base == "shard.yml" || base.ends_with(".sh");
        if !build {
            continue;
        }
        push(rel, crystal_scan::scan_shards_network(text), "RCR-044");
        if needs_gc {
            push(rel, scan_gc_none(text), "RCR-051");
        }
    }
    match yml {
        Some(y) => {
            push("shard.yml", crystal_scan::scan_shard_yml(y), "RCR-040");
            let lock = files
                .iter()
                .find(|f| f.0 == "shard.lock")
                .map(|f| f.1.as_str());
            push(
                "shard.yml",
                crystal_scan::scan_shard_lock(y, lock),
                "RCR-041",
            );
        }
        None => push(
            "shard.yml",
            vec![(
                0,
                "there is no shard.yml: this is not a Crystal project folder".into(),
            )],
            "RCR-040",
        ),
    }
    out.sort_by(|a, b| (&a.file, a.line, a.rule).cmp(&(&b.file, b.line, b.rule)));
    out
}

// ---------------------------------------------------------------- the verbs

fn dir_of(inv: &Invocation) -> PathBuf {
    PathBuf::from(inv.get("dir").filter(|d| !d.is_empty()).unwrap_or("."))
}

/// The argv `wol app run` starts (pure; tests read it).
pub fn run_argv(dir: &Path, watch: bool) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    if watch {
        a.extend(["env".into(), "WOL_APP_WATCH=1".into()]);
    }
    a.extend([
        "make".into(),
        "-C".into(),
        dir.to_string_lossy().into_owned(),
        "run".into(),
    ]);
    a
}

fn new(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let id = inv.get("id").unwrap_or_default();
    let template = inv
        .get("template")
        .filter(|t| !t.is_empty())
        .unwrap_or(TEMPLATES[0]);
    if inv.flag("template") && inv.get("template").is_none() {
        return fail(
            env,
            ErrorCode::Usage,
            "Name the template after --template: app, cli, tool or ai.",
        );
    }
    if !TEMPLATES.contains(&template) {
        return fail(
            env,
            ErrorCode::Usage,
            &format!("{template} is not a template; the templates are app, cli, tool and ai."),
        );
    }
    let n = match names(id) {
        Ok(n) => n,
        Err(m) => return fail(env, ErrorCode::Usage, &m),
    };
    let root = env
        .var("WOLKIT_TEMPLATES")
        .map_or_else(|| PathBuf::from(TEMPLATES_DIR), PathBuf::from);
    let source = root.join(template);
    if env.list_dir(&source).is_empty() {
        return fail(
            env,
            ErrorCode::Unavailable,
            &format!(
                "The {template} template is not installed ({}); install the Crystal tools with `wol install wolos-crystal`.",
                source.display()
            ),
        );
    }
    let target = PathBuf::from(&n.name);
    match render(env, &source, &target, &n) {
        Ok(files) => {
            if inv.flag("json") {
                emit_json(
                    env,
                    &json!({"id": n.id, "name": n.name, "module": n.module, "template": template,
                        "dir": n.name, "files": files}),
                );
            } else {
                for f in &files {
                    env.out(&format!("{}/{f}\n", n.name));
                }
                env.out(&format!(
                    "Made {} ({}) from the {template} template. Next: cd {} && make spec\n",
                    n.name, n.id, n.name
                ));
            }
            0
        }
        Err(m) => fail(env, ErrorCode::Invalid, &m),
    }
}

fn run(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let dir = dir_of(inv);
    if env.read_file(&dir.join("Makefile")).is_none() {
        return fail(
            env,
            ErrorCode::NotFound,
            &format!(
                "There is no Makefile in {}: wol app run runs its `run` target (wol app new writes one).",
                dir.display()
            ),
        );
    }
    match env.exec(&run_argv(&dir, inv.flag("watch"))) {
        Ok(code) => code,
        Err(_) => fail(
            env,
            ErrorCode::Unavailable,
            "make is not installed, so the app cannot be built here.",
        ),
    }
}

fn check_verb(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let dir = dir_of(inv);
    let found = check(env, &dir);
    let crystal_files = walk(env, &dir, &generated)
        .iter()
        .filter(|f| f.0.ends_with(".cr"))
        .count();
    if inv.flag("json") {
        let list: Vec<_> = found
            .iter()
            .map(|p| json!({"file": p.file, "line": p.line, "rule": p.rule, "why": p.why}))
            .collect();
        emit_json(env, &json!({"files": crystal_files, "findings": list}));
    } else {
        for p in &found {
            env.out(&format!("{}:{}: {} {}\n", p.file, p.line, p.rule, p.why));
        }
    }
    if !found.is_empty() {
        if !inv.flag("json") {
            env.out(&format!(
                "{} findings; the rules are in docs/RULES-crystal.md.\n",
                found.len()
            ));
        }
        return 1;
    }
    // The rules read files; the compiler then says whether the program type-checks.
    if inv.flag("rules") || env.read_file(&dir.join("Makefile")).is_none() {
        if !inv.flag("json") {
            env.out(&format!(
                "{crystal_files} Crystal files follow the WolOS rules.\n"
            ));
        }
        return 0;
    }
    let argv: Vec<String> = ["make", "-s", "-C", &dir.to_string_lossy(), "check"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let code = if inv.flag("json") || !env.stdout_tty() {
        env.exec_quiet(&argv)
    } else {
        env.exec(&argv)
    };
    match code {
        Ok(0) => {
            if !inv.flag("json") {
                env.out(&format!(
                    "{crystal_files} Crystal files follow the WolOS rules, and the program type-checks.\n"
                ));
            }
            0
        }
        Ok(c) => c,
        Err(_) => fail(
            env,
            ErrorCode::Unavailable,
            "make is not installed, so the program cannot be type-checked here (--rules checks the files only).",
        ),
    }
}

/// `wol app new|run|check`.
pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    match inv.action {
        "app.new" => new(inv, env),
        "app.run" => run(inv, env),
        "app.check" => check_verb(inv, env),
        other => fail(env, ErrorCode::Usage, &format!("no handler for {other}")),
    }
}
