// SPDX-License-Identifier: GPL-2.0-or-later
//! Ruby rules (docs/RULES-ruby.md, ids RB-nnn; ADR 0041): every file under files/, desktop/ and
//! tools/ with a ruby shebang or a `.rb` name is read and checked against a catalogue of safety and
//! consistency rules, the same way rules_system.rs checks shell scripts. The kind of a file comes
//! from `controller::script`, the code `wol script` runs, so the rules and the runner agree on what
//! a Ruby script is. Nothing runs except `ruby -wc` and RuboCop (each test says so and is skipped
//! with a message when the tool is missing; WOLOS_REQUIRE_RUBOCOP=1 turns a missing tool into a
//! failure, CI sets it). A rule the files do not meet yet carries a BASELINE below: the list only
//! shrinks, a new violation fails, a fixed one is reported on stderr so the entry is removed.
//! `rb_090_fixtures_trip_exactly_their_rule` runs every scanner over tests/fixtures/ruby: a bad
//! fixture trips only the rule on its `# trips:` line, good.rb trips nothing.
//! `rb_100_doc_and_tests_agree` ties the tests to the document.
#[path = "support/ruby_scan.rs"]
mod ruby_scan;

use ruby_scan::*;

use controller::script::{RUBOCOP_COPS, shebang_interpreter};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------- baselines (growth-only)

/// RB-001..RB-018, RB-022: (rule, file) pairs that break a text rule today. Empty: there is no
/// Ruby in the tree yet, so every new script meets the rules from its first commit.
const B_RB: &[(&str, &str)] = &[];
/// RB-032: RuboCop offences per file (the number may only fall).
const B_RB_RUBOCOP: &[(&str, usize)] = &[];

// ---------------------------------------------------------------- policy lists

/// RB-030: Ruby files allowed to run as root (under files/usr/lib/wolos, files/usr/lib/systemd,
/// files/etc). Empty by decision (ADR 0041): root helpers stay POSIX sh. An entry needs an ADR
/// and `ruby` in wolos-system's depends.
const ROOT_RUBY: &[&str] = &[];
/// Where a root helper lives.
const ROOT_TREES: &[&str] = &[
    "files/usr/lib/wolos/",
    "files/usr/lib/systemd/",
    "files/etc/",
];
/// RB-031: interpreter kinds each top folder may hold (by shebang). A new pair fails: a new
/// language in the tree is a review question.
const CENSUS: &[(&str, &str)] = &[
    ("desktop", "other"),
    ("desktop", "ruby"),
    ("desktop", "sh"),
    ("files", "sh"),
    ("tools", "bash"),
    ("tools", "python"),
    ("tools", "ruby"),
    ("tools", "sh"),
];
/// RB-031: Python scripts per top folder may only fall (new tooling is sh or Ruby).
/// tools 2 -> 11 (INT-2, the 0.13 integration; none of them is shipped): the Memory Saver and the
/// JavaScript-runtime benchmark harnesses drive Electron, WebKit and /proc from the forge
/// (`tools/bench/discord/*.py`, six; `tools/bench/js-runtime-x/*.py`, two: DevTools websocket and
/// smaps readers), and `tools/theme/atlas-gtk3-sample.py` draws a GTK 3 sample through python-gi
/// (ATLAS-THEME), a binding Ruby does not have here.
// tools 13 = 11 + the Atlas Drive benchmark (ADR 0063): tools/bench/atlas-drive/bench.py runs both
// tools interleaved and chart.py draws docs/bench/atlas-drive.png with matplotlib (no Rust plotting).
const B_PYTHON: &[(&str, usize)] = &[("tools", 13)];
/// Scanner fixtures (wrong on purpose; never part of the scope).
const FIXTURES: &str = "crates/controller/tests/fixtures/ruby/";
/// The folders whose Ruby is checked.
const TOPS: &[&str] = &["files", "desktop", "tools"];
/// Trees that ship: RB-001 asks for the fixed interpreter path there.
const SHIPPED: &[&str] = &["files/", "desktop/"];

// ---------------------------------------------------------------- repo access

fn repo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let n = e.file_name().to_string_lossy().into_owned();
        if matches!(
            n.as_str(),
            "target" | ".git" | "node_modules" | ".work" | "__pycache__"
        ) {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Repo-relative paths of the files under `top`, sorted.
fn files_under(top: &str) -> Vec<String> {
    let base = repo_dir();
    let mut v = Vec::new();
    walk(&base.join(top), &mut v);
    let mut s: Vec<String> = v
        .iter()
        .filter_map(|p| p.strip_prefix(&base).ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    s.sort();
    s
}

fn read_repo(rel: &str) -> String {
    std::fs::read_to_string(repo_dir().join(rel)).unwrap_or_default()
}

/// The interpreter kind of a file by its shebang, as RB-031 counts it.
fn census_kind(first: &str) -> Option<&'static str> {
    let i = shebang_interpreter(first)?;
    Some(match i.as_str() {
        "sh" | "dash" => "sh",
        "bash" => "bash",
        "ruby" => "ruby",
        "python" => "python",
        _ => "other",
    })
}

fn is_ruby(rel: &str, text: &str) -> bool {
    rel.ends_with(".rb") || census_kind(text.lines().next().unwrap_or("")) == Some("ruby")
}

/// The Ruby files in scope: (path, text).
fn ruby_files() -> &'static Vec<(String, String)> {
    static CELL: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        TOPS.iter()
            .flat_map(|t| files_under(t))
            .filter_map(|f| {
                let t = std::fs::read(repo_dir().join(&f)).ok()?;
                let t = String::from_utf8(t).ok()?;
                is_ruby(&f, &t).then_some((f, t))
            })
            .collect()
    })
}

fn fixtures() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = std::fs::read_dir(repo_dir().join(FIXTURES))
        .expect("tests/fixtures/ruby")
        .flatten()
        .map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            (n, std::fs::read_to_string(e.path()).unwrap())
        })
        .collect();
    v.sort();
    v
}

// ---------------------------------------------------------------- ratchets

fn assert_ratchet(rule: &str, found: BTreeMap<String, Vec<Finding>>) {
    let base: BTreeSet<&str> = B_RB
        .iter()
        .filter(|(r, _)| *r == rule)
        .map(|(_, f)| *f)
        .collect();
    let mut new = Vec::new();
    for (f, v) in &found {
        if !base.contains(f.as_str()) {
            for (n, why) in v {
                new.push(format!("{f}:{n}: {why}"));
            }
        }
    }
    assert!(
        new.is_empty(),
        "{rule}: new violations (fix them; baselines only shrink):\n{}",
        new.join("\n")
    );
    for b in base {
        if !found.contains_key(b) {
            eprintln!("{rule}: baseline entry {b} is compliant now; remove it");
        }
    }
}

fn assert_ratchet_count(rule: &str, found: &BTreeMap<String, usize>, baseline: &[(&str, usize)]) {
    let mut bad = Vec::new();
    for (f, n) in found {
        let allowed = baseline.iter().find(|(b, _)| b == f).map_or(0, |(_, n)| *n);
        if *n > allowed {
            bad.push(format!("(\"{f}\", {n}),  // allowed {allowed}"));
        } else if *n < allowed {
            eprintln!("{rule}: {f} is down to {n}; lower the baseline");
        }
    }
    assert!(bad.is_empty(), "{rule}: counts grew:\n{}", bad.join("\n"));
}

/// Runs scanner `id` over the scope (`shipped_only`: files/ and desktop/ only).
fn check(id: &str, shipped_only: bool) {
    let f = scanner(id);
    let mut found = BTreeMap::new();
    for (rel, text) in ruby_files() {
        if shipped_only && !SHIPPED.iter().any(|s| rel.starts_with(s)) {
            continue;
        }
        let r = f(text);
        if !r.is_empty() {
            found.insert(rel.clone(), r);
        }
    }
    assert_ratchet(id, found);
}

// ---------------------------------------------------------------- tools

fn require_tools() -> bool {
    std::env::var("WOLOS_REQUIRE_RUBOCOP").is_ok_and(|v| v == "1")
}

fn skip_or_fail(rule: &str, why: &str) {
    assert!(!require_tools(), "{rule}: {why} (WOLOS_REQUIRE_RUBOCOP=1)");
    eprintln!("{rule}: skipped, {why}");
}

fn runs(prog: &str) -> bool {
    Command::new(prog)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// `ruby -wc`: parses, and the parser printed no warning.
fn ruby_parses(path: &Path) -> Result<(), String> {
    let o = Command::new("ruby").arg("-wc").arg(path).output().unwrap();
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    if o.status.success() && !err.contains("warning:") {
        Ok(())
    } else {
        Err(format!("{}{err}", String::from_utf8_lossy(&o.stdout)))
    }
}

/// RuboCop offences per file with tools/ruby/rubocop.yml (JSON format).
fn rubocop_offences(paths: &[PathBuf]) -> BTreeMap<String, usize> {
    let o = Command::new("rubocop")
        .current_dir(repo_dir())
        .arg("-c")
        .arg(repo_dir().join("tools/ruby/rubocop.yml"))
        .args(["--cache", "false", "--format", "json", "--"])
        .args(paths)
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("rubocop json: {e}: {}", String::from_utf8_lossy(&o.stderr)));
    let base = repo_dir();
    v["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| {
            let n = f["offenses"].as_array()?.len();
            let p = PathBuf::from(f["path"].as_str()?);
            let rel = p.strip_prefix(&base).unwrap_or(&p).to_string_lossy();
            (n > 0).then(|| (rel.replace('\\', "/"), n))
        })
        .collect()
}

/// `Cop/Name:` keys of tools/ruby/rubocop.yml with `Enabled: true` (AllCops left out).
fn rubocop_yml() -> (Vec<String>, String) {
    let t = read_repo("tools/ruby/rubocop.yml");
    let mut cops = Vec::new();
    let mut cur: Option<String> = None;
    for l in t.lines() {
        if !l.starts_with(' ') && l.ends_with(':') && l.contains('/') {
            cur = Some(l.trim_end_matches(':').to_string());
        } else if l.trim() == "Enabled: true"
            && let Some(c) = cur.take()
        {
            cops.push(c);
        }
    }
    (cops, t)
}

// ---------------------------------------------------------------- text rules

#[test]
fn rb_001_shebang_is_usr_bin_ruby() {
    check("RB-001", true);
}

#[test]
fn rb_002_spdx_on_line_two() {
    check("RB-002", false);
}

#[test]
fn rb_003_header_says_what_why_how() {
    check("RB-003", false);
}

#[test]
fn rb_004_frozen_string_literal_first() {
    check("RB-004", false);
}

#[test]
fn rb_005_no_code_built_at_run_time() {
    check("RB-005", false);
}

#[test]
fn rb_006_commands_go_out_as_argv() {
    check("RB-006", false);
}

#[test]
fn rb_007_no_pipe_open_or_open_uri() {
    check("RB-007", false);
}

#[test]
fn rb_008_no_object_building_decoders() {
    check("RB-008", false);
}

#[test]
fn rb_009_requires_are_default_gems() {
    check("RB-009", false);
}

#[test]
fn rb_010_rescues_name_what_they_handle() {
    check("RB-010", false);
}

#[test]
fn rb_011_no_fixed_tmp_paths() {
    check("RB-011", false);
}

#[test]
fn rb_012_nothing_world_writable() {
    check("RB-012", false);
}

#[test]
fn rb_013_recursive_deletes_are_guarded() {
    check("RB-013", false);
}

#[test]
fn rb_014_no_home_literals() {
    check("RB-014", false);
}

#[test]
fn rb_015_no_debugger_or_trace() {
    check("RB-015", false);
}

#[test]
fn rb_016_http_sets_timeouts() {
    check("RB-016", false);
}

#[test]
fn rb_017_no_credentials() {
    check("RB-017", false);
}

#[test]
fn rb_018_text_hygiene() {
    check("RB-018", false);
}

// ---------------------------------------------------------------- tool rules

#[test]
fn rb_019_ruby_parses_without_warnings() {
    if !runs("ruby") {
        skip_or_fail("RB-019", "ruby not installed");
        return;
    }
    let base = repo_dir();
    let mut bad = BTreeMap::new();
    for (rel, _) in ruby_files() {
        if let Err(e) = ruby_parses(&base.join(rel)) {
            bad.insert(rel.clone(), vec![(1, e)]);
        }
    }
    assert_ratchet("RB-019", bad);
}

#[test]
fn rb_020_rubocop_runs_the_same_explicit_list_everywhere() {
    let (cops, text) = rubocop_yml();
    assert_eq!(
        cops, RUBOCOP_COPS,
        "RB-020: tools/ruby/rubocop.yml and controller::script::RUBOCOP_COPS differ"
    );
    for want in ["DisabledByDefault: true", "NewCops: disable"] {
        assert!(text.contains(want), "RB-020: rubocop.yml needs {want}");
    }
    if !runs("rubocop") {
        skip_or_fail("RB-020", "rubocop not installed");
        return;
    }
    let o = Command::new("rubocop")
        .args(["--force-default-config", "--show-cops"])
        .output()
        .unwrap();
    let all = String::from_utf8_lossy(&o.stdout);
    for c in RUBOCOP_COPS {
        assert!(
            all.lines().any(|l| l == format!("{c}:")),
            "RB-020: the installed RuboCop has no cop {c}"
        );
    }
}

#[test]
fn rb_021_every_root_tree_ruby_file_is_referenced() {
    let mut bad = BTreeMap::new();
    let others: Vec<String> = ["files", "desktop", "crates", "docs"]
        .iter()
        .flat_map(|t| files_under(t))
        .collect();
    for (rel, _) in ruby_files() {
        if !rel.starts_with("files/usr/lib/wolos/") {
            continue;
        }
        let name = rel.rsplit('/').next().unwrap();
        let used = others
            .iter()
            .filter(|o| *o != rel)
            .any(|o| read_repo(o).contains(name));
        if !used {
            bad.insert(rel.clone(), vec![(1, "nothing names it".into())]);
        }
    }
    assert_ratchet("RB-021", bad);
}

#[test]
fn rb_022_temporary_folders_are_removed() {
    check("RB-022", false);
}

#[test]
fn rb_030_root_helpers_are_not_ruby() {
    let bad: Vec<&String> = ruby_files()
        .iter()
        .map(|(r, _)| r)
        .filter(|r| ROOT_TREES.iter().any(|t| r.starts_with(t)))
        .filter(|r| !ROOT_RUBY.contains(&r.as_str()))
        .collect();
    assert!(
        bad.is_empty(),
        "RB-030: Ruby where root runs it (root helpers stay POSIX sh, ADR 0041): {bad:?}"
    );
}

#[test]
fn rb_031_interpreter_census_only_grows_by_review() {
    let mut seen: BTreeMap<(String, &str), usize> = BTreeMap::new();
    for top in ["files", "desktop", "tools"] {
        for f in files_under(top) {
            let Ok(bytes) = std::fs::read(repo_dir().join(&f)) else {
                continue;
            };
            let head = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).into_owned();
            if let Some(k) = census_kind(head.lines().next().unwrap_or("")) {
                *seen.entry((top.to_string(), k)).or_default() += 1;
            }
        }
    }
    for ((top, kind), n) in &seen {
        assert!(
            CENSUS.contains(&(top.as_str(), kind)),
            "RB-031: {n} {kind} script(s) under {top}/ is a new interpreter there; add ({top:?}, \
             {kind:?}) to CENSUS with its reason"
        );
    }
    let python: BTreeMap<String, usize> = seen
        .iter()
        .filter(|((_, k), _)| *k == "python")
        .map(|((t, _), n)| (t.clone(), *n))
        .collect();
    assert_ratchet_count("RB-031", &python, B_PYTHON);
}

#[test]
fn rb_032_rubocop_offences_only_fall() {
    if !runs("rubocop") {
        skip_or_fail("RB-032", "rubocop not installed");
        return;
    }
    let base = repo_dir();
    let paths: Vec<PathBuf> = ruby_files().iter().map(|(r, _)| base.join(r)).collect();
    if paths.is_empty() {
        return;
    }
    assert_ratchet_count("RB-032", &rubocop_offences(&paths), B_RB_RUBOCOP);
}

// ---------------------------------------------------------------- self-tests

fn trips(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix("# trips: "))
        .map(|s| s.trim().to_string())
}

#[test]
fn rb_090_fixtures_trip_exactly_their_rule() {
    let all = fixtures();
    assert!(all.iter().any(|(n, _)| n == "good.rb"), "RB-090: good.rb");
    let mut covered = BTreeSet::new();
    for (name, text) in &all {
        let want = trips(text);
        assert!(
            name == "good.rb" || want.is_some(),
            "RB-090: {name} needs a `# trips: RB-nnn` line"
        );
        let got: BTreeSet<String> = FILE_SCANNERS
            .iter()
            .filter(|(_, f)| !f(text).is_empty())
            .map(|(id, _)| id.to_string())
            .collect();
        match want.as_deref() {
            None => assert!(got.is_empty(), "RB-090: good.rb trips {got:?}"),
            Some(id @ ("RB-019" | "RB-032")) => {
                assert!(got.is_empty(), "RB-090: {name} ({id}) trips {got:?}");
                covered.insert(id.to_string());
            }
            Some(id) => {
                assert_eq!(got, BTreeSet::from([id.to_string()]), "RB-090: {name}");
                covered.insert(id.to_string());
            }
        }
    }
    let scanners: BTreeSet<String> = FILE_SCANNERS.iter().map(|(i, _)| i.to_string()).collect();
    let missing: Vec<&String> = scanners.difference(&covered).collect();
    assert!(missing.is_empty(), "RB-090: no bad fixture for {missing:?}");

    // The tool rules, when the tools are here: only bad-syntax.rb fails to parse, only
    // bad-rubocop.rb of good/bad-rubocop has offences.
    let dir = repo_dir().join(FIXTURES);
    if runs("ruby") {
        for (name, _) in &all {
            let r = ruby_parses(&dir.join(name));
            assert_eq!(
                r.is_err(),
                name == "bad-syntax.rb",
                "RB-090 RB-019 {name}: {r:?}"
            );
        }
    } else {
        skip_or_fail("RB-090", "ruby not installed; RB-019 fixture not run");
    }
    if runs("rubocop") {
        let got = rubocop_offences(&[dir.join("good.rb"), dir.join("bad-rubocop.rb")]);
        assert_eq!(
            got.keys().map(String::as_str).collect::<Vec<_>>(),
            [format!("{FIXTURES}bad-rubocop.rb").as_str()],
            "RB-090 RB-032"
        );
    } else {
        skip_or_fail("RB-090", "rubocop not installed; RB-032 fixture not run");
    }
}

#[test]
fn rb_100_doc_and_tests_agree() {
    let doc = read_repo("docs/RULES-ruby.md");
    assert!(!doc.is_empty(), "docs/RULES-ruby.md");
    let src =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/rules_ruby.rs"))
            .unwrap();
    let mut tests: BTreeMap<String, String> = BTreeMap::new();
    for l in src.lines() {
        if let Some(r) = l.strip_prefix("fn rb_") {
            let name = r.split('(').next().unwrap();
            tests.insert(format!("RB-{}", &name[..3]), format!("rb_{name}"));
        }
    }
    let mut documented = BTreeMap::new();
    for l in doc.lines().filter(|l| l.starts_with("| RB-")) {
        let c: Vec<&str> = l.split('|').map(str::trim).collect();
        // | id | area | rule | test | status |
        if c.len() >= 6 {
            documented.insert(c[1].to_string(), c[4].trim_matches('`').to_string());
        }
    }
    assert_eq!(
        documented, tests,
        "docs/RULES-ruby.md rows (id, test) and the rb_* tests differ"
    );
    for (id, _) in FILE_SCANNERS {
        assert!(tests.contains_key(*id), "{id}: a scanner without a test");
    }
    for b in [
        "B_RB",
        "B_RB_RUBOCOP",
        "ROOT_RUBY",
        "CENSUS",
        "B_PYTHON",
        "STDLIB",
    ] {
        assert!(
            doc.contains(&format!("`{b}`")),
            "docs/RULES-ruby.md names {b}"
        );
    }
    // baselines are sorted and unique
    let mut pairs = B_RB.to_vec();
    pairs.sort_unstable();
    pairs.dedup();
    assert_eq!(pairs, B_RB, "B_RB sorted and unique");
    let mut s = STDLIB.to_vec();
    s.sort_unstable();
    s.dedup();
    assert_eq!(s, STDLIB, "STDLIB sorted and unique");
    let mut c = CENSUS.to_vec();
    c.sort_unstable();
    c.dedup();
    assert_eq!(c, CENSUS, "CENSUS sorted and unique");
}
