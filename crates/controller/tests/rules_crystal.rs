// SPDX-License-Identifier: GPL-2.0-or-later
//! Crystal rules (docs/RULES-crystal.md, ids RCR-nnn): every `.cr` file in the repo (wopkg/, hue/,
//! crates/wol-abi/bindings/crystal/ and any tree added later; the walk is the whole checkout),
//! every `shard.yml`/`shard.lock`, and the build files that could fetch shards are read and checked
//! against a catalogue of safety and consistency rules. Nothing is compiled or run except
//! `crystal tool format --check` and `ameba` (each test says so and is skipped with a message when
//! the tool is missing; WOLOS_REQUIRE_CRYSTAL_LINT=1 turns a missing tool into a failure, CI sets
//! it). A rule the code does not meet yet carries a BASELINE below: the list only shrinks, a new
//! violation fails the test, a fixed one is reported on stderr so the entry is removed.
//! `rcr_090_fixtures_trip_exactly_their_rule` runs every scanner over
//! tests/fixtures/crystal/{good,bad-*}: a bad fixture trips only the rule named on its `# trips:`
//! line, a good one trips nothing. `rcr_091_doc_and_tests_agree` ties the tests to the document.
#[path = "support/crystal_scan.rs"]
mod crystal_scan;
#[path = "support/siblings.rs"]
mod siblings;

use crystal_scan::*;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------- baselines (growth-only)

/// RCR-002: non-spec files without a purpose comment (40+ characters in the first 30 lines).
const B_PURPOSE: &[&str] = &[];
/// RCR-010: files outside the FFI allowlist that use pointers or other unsafe constructs.
const B_UNSAFE: &[&str] = &[];
/// RCR-020: files that start a process through a shell or with a command line in one string.
const B_PROCESS_SHELL: &[&str] = &[];
/// RCR-022: files that use HTTP::Client class methods (no way to set a timeout) or a client
/// without connect and read timeouts.
const B_HTTP_TIMEOUT: &[&str] = &[];
/// RCR-024: untyped JSON/YAML parses per non-spec file (the number may only fall; new code uses
/// `Type.from_json` / `from_yaml`).
const B_UNTYPED_PARSE: &[(&str, usize)] = &[("wopkg/src/wopkg/core.cr", 2)];
/// RCR-026: catch-all rescues (bare `rescue`, `rescue e`, `rescue nil` modifier) per non-spec
/// file (the number may only fall; new code names the exception types it handles).
const B_CATCH_ALL: &[(&str, usize)] = &[];
/// RCR-026 allowlist (not a baseline): (file, most catch-all rescues it may hold, reason). More
/// rescues than the count fail the test; a new entry needs an ADR.
const ALLOW_CATCH_ALL: &[(&str, usize, &str)] = &[(
    "wolos-crystal/shards/cryswol/src/wol/callbacks.cr",
    3,
    "a callback must not unwind into C; exceptions are routed to Wol::Loop.on_error",
)];
/// RCR-030: files with a fixed /tmp path in a string (not a spec).
const B_TMP: &[&str] = &["wopkg/src/wopkg/sandbox.cr"];
/// RCR-032: files using MD5 or SHA-1.
const B_WEAK_HASH: &[&str] = &[];
/// RCR-033: files using a non-secure random generator.
const B_RANDOM: &[&str] = &[];
/// RCR-035: files naming /home/ in a string (not a spec).
const B_HOME: &[&str] = &[];
/// RCR-007: `Rule file` pairs in .ameba.yml `Excluded:` (the legacy files that still break the
/// rule). Only shrinks; a pair here that no longer breaks the rule is reported for removal.
const B_AMEBA_EXCLUDED: &[&str] = &[];

// ---------------------------------------------------------------- policy lists

/// RCR-010, RCR-011: the only files that may declare C bindings or touch raw pointers. A new entry
/// needs an ADR. A directory entry ends in `/`, a prefix entry in `*`.
/// Reasons: the generated bindings and `lib_*` files declare the C ABI; hue color/image and wopkg
/// core call C libraries; the cryswol files `callbacks`, `context`, `error`, `handle`, `info`,
/// `string` and `pkg/ffi` are the libwol handle/callback boundary.
const FFI_ALLOW: &[&str] = &[
    "crates/wol-abi/bindings/crystal/",
    "hue/src/hue/color.cr",
    "hue/src/hue/image.cr",
    "wolos-crystal/shards/cryswol/spec/generated/",
    "wolos-crystal/shards/cryswol/src/wol/callbacks.cr",
    "wolos-crystal/shards/cryswol/src/wol/context.cr",
    "wolos-crystal/shards/cryswol/src/wol/error.cr",
    "wolos-crystal/shards/cryswol/src/wol/handle.cr",
    "wolos-crystal/shards/cryswol/src/wol/info.cr",
    "wolos-crystal/shards/cryswol/src/wol/lib_*",
    "wolos-crystal/shards/cryswol/src/wol/pkg/ffi.cr",
    "wolos-crystal/shards/cryswol/src/wol/pkg/lib_*",
    "wolos-crystal/shards/cryswol/src/wol/realtime/lib_*",
    "wolos-crystal/shards/cryswol/src/wol/string.cr",
    "wopkg/src/wopkg/core.cr",
    "wopkg/src/wopkg/lib_wopkg.cr",
];
/// RCR-042: the shards the repo may depend on (direct and transitive), by name. Empty today:
/// the Crystal code uses the standard library only. A shard is added with an ADR, here, and in
/// docs/THIRD-PARTY.md.
const ALLOWED_SHARDS: &[&str] = &[];
/// RCR-042: ceiling of the number of shard.yml dependency entries plus vendored shards.
const SHARD_CENSUS: usize = ALLOWED_SHARDS.len();
/// Directories whose `.cr` files are scanner fixtures (wrong on purpose, never linted).
const FIXTURES: &str = "crates/controller/tests/fixtures/crystal/";
/// The ameba release whose rule list .ameba.yml carries (tools/crystal/lint.sh passes the same).
const AMEBA_VERSION: &str = "1.7.0";
/// The Crystal release `shard.yml` asks for; the formatter output differs between releases, so a
/// toolchain older than this is not a reference for RCR-004.
const CRYSTAL_FLOOR: (u32, u32) = (1, 12);

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
        if n == "target" || n == ".git" || n == "node_modules" || n == ".work" {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Repo-relative '/'-separated paths of every file under `base` (the checkout, or the overlaid
/// three-repo tree), sorted.
fn all_files(base: &Path) -> Vec<String> {
    let mut v = Vec::new();
    walk(base, &mut v);
    let mut s: Vec<String> = v
        .iter()
        .filter_map(|p| p.strip_prefix(base).ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    s.sort();
    s
}

fn is_fixture(rel: &str) -> bool {
    rel.starts_with(FIXTURES)
}

fn is_spec(rel: &str) -> bool {
    rel.contains("/spec/") || rel.starts_with("spec/") || rel.ends_with("_spec.cr")
}

fn ffi_allowed(rel: &str) -> bool {
    FFI_ALLOW.iter().any(|a| {
        if let Some(prefix) = a.strip_suffix('*') {
            rel.starts_with(prefix)
        } else if a.ends_with('/') {
            rel.starts_with(a)
        } else {
            rel == *a
        }
    })
}

/// The Crystal files of this checkout (fixtures left out): (relative path, text).
fn crystal_files() -> &'static Vec<(String, String)> {
    static CELL: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        let base = repo_dir();
        all_files(&base)
            .into_iter()
            .filter(|f| f.ends_with(".cr") && !is_fixture(f))
            .map(|f| {
                let t = std::fs::read_to_string(base.join(&f)).unwrap_or_default();
                (f, t)
            })
            .collect()
    })
}

fn read_repo(rel: &str) -> Option<String> {
    std::fs::read_to_string(repo_dir().join(rel)).ok()
}

// ---------------------------------------------------------------- ratchets

fn assert_ratchet(rule: &str, found: BTreeSet<String>, baseline: &[&str]) {
    let base: BTreeSet<&str> = baseline.iter().copied().collect();
    let new: Vec<&String> = found
        .iter()
        .filter(|f| !base.contains(f.as_str()))
        .collect();
    assert!(
        new.is_empty(),
        "{rule}: new violations (fix them; baselines only shrink):\n{}",
        new.iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    for b in base {
        if !found.contains(b) {
            eprintln!("{rule}: baseline entry {b} is compliant now; remove it");
        }
    }
}

fn assert_ratchet_count(rule: &str, found: BTreeMap<String, usize>, baseline: &[(&str, usize)]) {
    let mut bad = Vec::new();
    for (f, n) in &found {
        let allowed = baseline.iter().find(|(b, _)| b == f).map_or(0, |(_, n)| *n);
        if *n > allowed {
            bad.push(format!("(\"{f}\", {n}),  // allowed {allowed}"));
        } else if *n < allowed {
            eprintln!("{rule}: {f} is down to {n}; lower the baseline");
        }
    }
    for (f, _) in baseline.iter().filter(|(f, _)| !found.contains_key(*f)) {
        eprintln!("{rule}: {f} is down to 0; drop it from the baseline");
    }
    assert!(bad.is_empty(), "{rule}: counts grew:\n{}", bad.join("\n"));
}

/// Files (not specs when `skip_specs`, not the FFI allowlist when `skip_ffi`) with their findings.
fn findings(id: &str, skip_specs: bool, skip_ffi: bool) -> BTreeMap<String, Vec<Finding>> {
    let f = scanner(id);
    let mut out = BTreeMap::new();
    for (rel, text) in crystal_files() {
        if (skip_specs && is_spec(rel)) || (skip_ffi && ffi_allowed(rel)) {
            continue;
        }
        let r = f(&Src::new(rel, text));
        if !r.is_empty() {
            out.insert(rel.clone(), r);
        }
    }
    out
}

fn show(m: &BTreeMap<String, Vec<Finding>>) -> BTreeSet<String> {
    m.keys().cloned().collect()
}

fn explain(rule: &str, m: &BTreeMap<String, Vec<Finding>>, baseline: &[&str]) -> BTreeSet<String> {
    for (f, v) in m {
        if !baseline.contains(&f.as_str()) {
            for (n, why) in v {
                eprintln!("{rule}: {f}:{n}: {why}");
            }
        }
    }
    show(m)
}

fn counts(m: &BTreeMap<String, Vec<Finding>>) -> BTreeMap<String, usize> {
    m.iter().map(|(k, v)| (k.clone(), v.len())).collect()
}

fn names(m: &BTreeMap<String, Vec<Finding>>) -> Vec<String> {
    m.iter()
        .map(|(k, v)| {
            format!(
                "{k}: {} finding(s), first line {}: {}",
                v.len(),
                v[0].0,
                v[0].1
            )
        })
        .collect()
}

fn assert_no_findings(rule: &str, m: BTreeMap<String, Vec<Finding>>) {
    assert!(m.is_empty(), "{rule}:\n{}", names(&m).join("\n"));
}

// ---------------------------------------------------------------- tools (format, ameba)

fn require_tools() -> bool {
    std::env::var("WOLOS_REQUIRE_CRYSTAL_LINT").is_ok_and(|v| v == "1")
}

fn skip_or_fail(rule: &str, why: &str) {
    assert!(
        !require_tools(),
        "{rule}: {why} (WOLOS_REQUIRE_CRYSTAL_LINT=1)"
    );
    eprintln!("{rule}: skipped, {why}");
}

fn crystal_version() -> Option<(u32, u32)> {
    let o = Command::new("crystal").arg("--version").output().ok()?;
    let t = String::from_utf8_lossy(&o.stdout);
    let v = t.split_whitespace().nth(1)?;
    let mut p = v.split('.');
    Some((p.next()?.parse().ok()?, p.next()?.parse().ok()?))
}

fn ameba_runs() -> bool {
    Command::new("ameba")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Tracked-style list: the checkout's `.cr` files minus the fixtures.
fn lint_files() -> Vec<String> {
    crystal_files().iter().map(|(f, _)| f.clone()).collect()
}

/// The rule names `ameba --rules` prints (colour codes stripped).
fn ameba_rule_names() -> Vec<String> {
    let o = Command::new("ameba")
        .arg("--rules")
        .arg("--no-color")
        .current_dir(repo_dir())
        .output()
        .expect("ameba --rules");
    let t = String::from_utf8_lossy(&o.stdout);
    let mut v = Vec::new();
    for l in t.lines() {
        for w in l.split_whitespace() {
            if w.contains('/')
                && w.chars().next().is_some_and(char::is_uppercase)
                && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '/')
            {
                v.push(w.to_string());
            }
        }
    }
    v.sort();
    v.dedup();
    v
}

/// Rules in .ameba.yml: name -> (enabled, excluded files).
fn ameba_yml() -> BTreeMap<String, (Option<bool>, Vec<String>)> {
    let text = read_repo(".ameba.yml").unwrap_or_default();
    let mut m: BTreeMap<String, (Option<bool>, Vec<String>)> = BTreeMap::new();
    let mut cur: Option<String> = None;
    let mut in_ex = false;
    for l in text.lines() {
        if l.trim().is_empty() || l.trim_start().starts_with('#') {
            continue;
        }
        let indent = l.len() - l.trim_start().len();
        let t = l.trim();
        if indent == 0 {
            in_ex = false;
            cur = t
                .strip_suffix(':')
                .filter(|n| n.contains('/'))
                .map(str::to_string);
            if let Some(c) = &cur {
                m.insert(c.clone(), (None, vec![]));
            }
        } else if let Some(c) = cur.clone() {
            if let Some(v) = t.strip_prefix("Enabled:") {
                m.get_mut(&c).unwrap().0 = match v.trim() {
                    "true" => Some(true),
                    "false" => Some(false),
                    _ => None,
                };
                in_ex = false;
            } else if t == "Excluded:" {
                in_ex = true;
            } else if in_ex && let Some(f) = t.strip_prefix("- ") {
                m.get_mut(&c)
                    .unwrap()
                    .1
                    .push(f.trim().trim_matches('"').to_string());
            }
        }
    }
    m
}

// ================================================================ the rules

#[test]
fn rcr_001_spdx_on_line_one() {
    assert_no_findings("RCR-001", findings("RCR-001", false, false));
}

#[test]
fn rcr_002_header_says_what_the_file_is_for() {
    let m = findings("RCR-002", true, false);
    assert_ratchet("RCR-002", explain("RCR-002", &m, B_PURPOSE), B_PURPOSE);
}

#[test]
fn rcr_003_text_hygiene() {
    assert_no_findings("RCR-003", findings("RCR-003", false, false));
}

#[test]
fn rcr_004_crystal_format_is_clean() {
    let Some((maj, min)) = crystal_version() else {
        return skip_or_fail("RCR-004", "crystal is not installed");
    };
    if (maj, min) < CRYSTAL_FLOOR {
        return skip_or_fail(
            "RCR-004",
            &format!(
                "crystal {maj}.{min} is older than {}.{} (the formatter output differs between releases)",
                CRYSTAL_FLOOR.0, CRYSTAL_FLOOR.1
            ),
        );
    }
    let o = Command::new("crystal")
        .args(["tool", "format", "--check"])
        .args(lint_files())
        .current_dir(repo_dir())
        .output()
        .expect("crystal tool format");
    assert!(
        o.status.success(),
        "RCR-004: crystal {maj}.{min} tool format --check fails (run `make -C wopkg fmt` with the CI toolchain):\n{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn rcr_005_ameba_is_clean() {
    if !ameba_runs() {
        return skip_or_fail("RCR-005", "ameba is not installed (or does not start)");
    }
    let o = Command::new("ameba")
        .args(["--up-to-version", AMEBA_VERSION, "--no-color"])
        .args(lint_files())
        .current_dir(repo_dir())
        .output()
        .expect("ameba");
    assert!(
        o.status.success(),
        "RCR-005: ameba finds problems (config: .ameba.yml; tools/crystal/lint.sh):\n{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn rcr_006_ameba_config_lists_every_rule_explicitly() {
    let yml = ameba_yml();
    assert!(
        !yml.is_empty(),
        "RCR-006: .ameba.yml is missing or lists no rule"
    );
    let undecided: Vec<&String> = yml
        .iter()
        .filter(|(_, v)| v.0.is_none())
        .map(|(k, _)| k)
        .collect();
    assert!(
        undecided.is_empty(),
        "RCR-006: rules without `Enabled: true|false`: {undecided:?}"
    );
    let text = read_repo(".ameba.yml").unwrap();
    assert!(
        text.lines()
            .next()
            .is_some_and(|l| l.starts_with("# SPDX-License-Identifier:")),
        "RCR-006: .ameba.yml line 1 is the SPDX line"
    );
    assert!(
        text.contains(FIXTURES.trim_end_matches('/')),
        "RCR-006: .ameba.yml must keep the scanner fixtures out of Globs"
    );
    // the ten rules that guard memory, injection and error handling stay on
    for must in [
        "Lint/Syntax",
        "Lint/UnreachableCode",
        "Lint/ShadowedException",
        "Lint/UselessAssign",
        "Lint/UnusedArgument",
        "Lint/DebugCalls",
        "Lint/DebuggerStatement",
        "Lint/HashDuplicatedKey",
        "Lint/NotNil",
        "Lint/SharedVarInFiber",
    ] {
        assert_eq!(
            yml.get(must).and_then(|v| v.0),
            Some(true),
            "RCR-006: {must} must stay enabled"
        );
    }
    if !ameba_runs() {
        return skip_or_fail(
            "RCR-006",
            "ameba is not installed: the rule list was not compared with `ameba --rules`",
        );
    }
    let have = ameba_rule_names();
    let missing: Vec<&String> = have.iter().filter(|r| !yml.contains_key(*r)).collect();
    let stale: Vec<&String> = yml.keys().filter(|r| !have.contains(r)).collect();
    assert!(
        missing.is_empty(),
        "RCR-006: this ameba has rules .ameba.yml does not decide (add each with Enabled:): {missing:?}"
    );
    assert!(
        stale.is_empty(),
        "RCR-006: .ameba.yml names rules this ameba does not have: {stale:?}"
    );
}

#[test]
fn rcr_007_ameba_exclusions_only_shrink() {
    let yml = ameba_yml();
    let mut found = BTreeSet::new();
    for (rule, (_, ex)) in &yml {
        for f in ex {
            found.insert(format!("{rule} {f}"));
        }
    }
    assert_ratchet("RCR-007", found, B_AMEBA_EXCLUDED);
    // an excluded file must exist
    let files: BTreeSet<&str> = crystal_files().iter().map(|(f, _)| f.as_str()).collect();
    for (rule, (_, ex)) in &yml {
        for f in ex {
            assert!(
                files.contains(f.as_str()),
                "RCR-007: {rule} excludes {f}, which is not a Crystal file of this repo"
            );
        }
    }
}

#[test]
fn rcr_010_unsafe_constructs_stay_in_the_ffi_modules() {
    let m = findings("RCR-010", false, true);
    assert_ratchet("RCR-010", explain("RCR-010", &m, B_UNSAFE), B_UNSAFE);
}

#[test]
fn rcr_011_c_bindings_stay_in_the_ffi_modules() {
    assert_no_findings("RCR-011", findings("RCR-011", false, true));
    for a in FFI_ALLOW {
        let used = crystal_files().iter().any(|(f, _)| {
            if let Some(prefix) = a.strip_suffix('*') {
                f.starts_with(prefix)
            } else {
                f == a || (a.ends_with('/') && f.starts_with(a))
            }
        });
        assert!(
            used,
            "RCR-011: FFI_ALLOW lists {a}, which matches no Crystal file (drop it)"
        );
    }
}

#[test]
fn rcr_020_commands_go_out_as_argv() {
    let m = findings("RCR-020", false, false);
    assert_ratchet(
        "RCR-020",
        explain("RCR-020", &m, B_PROCESS_SHELL),
        B_PROCESS_SHELL,
    );
}

#[test]
fn rcr_021_no_code_built_or_loaded_at_run_time() {
    assert_no_findings("RCR-021", findings("RCR-021", false, false));
}

#[test]
fn rcr_022_network_clients_set_timeouts() {
    let m = findings("RCR-022", true, false);
    assert_ratchet(
        "RCR-022",
        explain("RCR-022", &m, B_HTTP_TIMEOUT),
        B_HTTP_TIMEOUT,
    );
}

#[test]
fn rcr_023_tls_verification_stays_on() {
    assert_no_findings("RCR-023", findings("RCR-023", false, false));
}

#[test]
fn rcr_024_json_and_yaml_decode_into_types() {
    let m = findings("RCR-024", true, false);
    assert_ratchet_count("RCR-024", counts(&m), B_UNTYPED_PARSE);
}

#[test]
fn rcr_025_nothing_rescues_exception() {
    assert_no_findings("RCR-025", findings("RCR-025", false, false));
}

#[test]
fn rcr_026_rescues_name_their_exceptions() {
    let m = findings("RCR-026", true, false);
    let mut found = counts(&m);
    for (f, max, reason) in ALLOW_CATCH_ALL {
        let n = found.remove(*f).unwrap_or(0);
        assert!(
            n <= *max,
            "RCR-026: {f} has {n} catch-all rescues, the allowlist permits {max} ({reason})"
        );
        if n < *max {
            eprintln!("RCR-026: {f} is down to {n}; lower ALLOW_CATCH_ALL");
        }
    }
    assert_ratchet_count("RCR-026", found, B_CATCH_ALL);
}

#[test]
fn rcr_030_no_fixed_tmp_paths() {
    let m = findings("RCR-030", true, false);
    assert_ratchet("RCR-030", explain("RCR-030", &m, B_TMP), B_TMP);
}

#[test]
fn rcr_031_nothing_is_made_world_writable() {
    assert_no_findings("RCR-031", findings("RCR-031", false, false));
}

#[test]
fn rcr_032_no_md5_or_sha1() {
    let m = findings("RCR-032", false, false);
    assert_ratchet("RCR-032", explain("RCR-032", &m, B_WEAK_HASH), B_WEAK_HASH);
}

#[test]
fn rcr_033_secrets_use_the_secure_random() {
    let m = findings("RCR-033", false, false);
    assert_ratchet("RCR-033", explain("RCR-033", &m, B_RANDOM), B_RANDOM);
}

#[test]
fn rcr_034_no_literal_credentials() {
    assert_no_findings("RCR-034", findings("RCR-034", false, false));
}

#[test]
fn rcr_035_no_home_directory_literals() {
    let m = findings("RCR-035", true, false);
    assert_ratchet("RCR-035", explain("RCR-035", &m, B_HOME), B_HOME);
}

/// Every shard.yml of the checkout (the repo's own, and any vendored under a `lib/` directory).
fn shard_ymls() -> Vec<(String, String)> {
    let base = repo_dir();
    all_files(&base)
        .into_iter()
        .filter(|f| (f == "shard.yml" || f.ends_with("/shard.yml")) && !is_fixture(f))
        .map(|f| {
            let t = std::fs::read_to_string(base.join(&f)).unwrap_or_default();
            (f, t)
        })
        .collect()
}

#[test]
fn rcr_040_shard_dependencies_are_pinned_by_commit() {
    let all = shard_ymls();
    assert!(
        all.len() >= 2,
        "RCR-040: expected the wopkg and hue shard.yml, found {}",
        all.len()
    );
    let mut bad = Vec::new();
    for (f, t) in &all {
        for (n, why) in scan_shard_yml(t) {
            bad.push(format!("{f}:{n}: {why}"));
        }
    }
    assert!(bad.is_empty(), "RCR-040:\n{}", bad.join("\n"));
}

#[test]
fn rcr_041_shard_lock_is_committed_and_matches() {
    let base = repo_dir();
    let mut bad = Vec::new();
    for (f, t) in shard_ymls() {
        let lock_path = f.replace("shard.yml", "shard.lock");
        let lock = std::fs::read_to_string(base.join(&lock_path)).ok();
        for (n, why) in scan_shard_lock(&t, lock.as_deref()) {
            bad.push(format!("{f}:{n}: {why}"));
        }
    }
    assert!(bad.is_empty(), "RCR-041:\n{}", bad.join("\n"));
}

#[test]
fn rcr_042_shard_census_only_falls() {
    let mut found: BTreeSet<String> = BTreeSet::new();
    let base = repo_dir();
    for (f, t) in shard_ymls() {
        for d in parse_shard_yml(&t).1 {
            found.insert(d.name);
        }
        // a vendored shard has its own shard.yml below a `lib/` directory
        if f.contains("/lib/") || f.starts_with("lib/") {
            let dir = f.trim_end_matches("shard.yml").trim_end_matches('/');
            found.insert(dir.rsplit('/').next().unwrap_or(dir).to_string());
        }
        let lock = std::fs::read_to_string(base.join(f.replace("shard.yml", "shard.lock")))
            .unwrap_or_default();
        found.extend(parse_shard_lock(&lock).into_keys());
    }
    // a require of a non-stdlib name is a shard too
    let unknown: Vec<&String> = found
        .iter()
        .filter(|n| !ALLOWED_SHARDS.contains(&n.as_str()))
        .collect();
    assert!(
        unknown.is_empty(),
        "RCR-042: shards not in ALLOWED_SHARDS (needs an ADR, THIRD-PARTY.md and the list): {unknown:?}"
    );
    let census = std::hint::black_box(SHARD_CENSUS); // 0 today; the compiler need not know
    assert!(
        found.len() <= census,
        "RCR-042: {} shards, census allows {census}",
        found.len()
    );
    if found.len() < census {
        eprintln!(
            "RCR-042: {} shards found, census says {census}; lower it",
            found.len()
        );
    }
}

#[test]
fn rcr_043_requires_are_stdlib_or_in_repo() {
    let m = findings("RCR-043", false, false);
    // a required shard has to be in the census (RCR-042); here only the undeclared ones fail
    assert_no_findings("RCR-043", m);
}

#[test]
fn rcr_044_build_files_do_not_fetch_shards() {
    let base = siblings::root();
    let mut bad = Vec::new();
    for f in all_files(&base) {
        let name = f.rsplit('/').next().unwrap_or(&f);
        let build_file = name == "PKGBUILD"
            || name == "Makefile"
            || name.ends_with(".sh")
            || name.ends_with(".yml")
            || name.ends_with(".yaml")
            || name.ends_with(".install");
        if !build_file || is_fixture(&f) || f.contains("/golden") {
            continue;
        }
        let Ok(t) = std::fs::read_to_string(base.join(&f)) else {
            continue;
        };
        for (n, why) in scan_shards_network(&t) {
            bad.push(format!("{f}:{n}: {why}"));
        }
    }
    assert!(bad.is_empty(), "RCR-044:\n{}", bad.join("\n"));
}

// ---------------------------------------------------------------- meta

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/crystal")
}

/// The rule id on a fixture's `# trips: RCR-nnn` line (first three lines), if any.
fn trips(text: &str) -> Option<String> {
    text.lines().take(3).find_map(|l| {
        l.trim_start_matches(['#', ' ', '-', ';'])
            .strip_prefix("trips: ")
            .map(|r| r.trim().to_string())
    })
}

#[test]
fn rcr_090_fixtures_trip_exactly_their_rule() {
    let dir = fixture_dir();
    let mut seen_rules = BTreeSet::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("tests/fixtures/crystal")
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    assert!(
        entries
            .iter()
            .any(|p| p.file_name().is_some_and(|n| n == "good.cr")),
        "RCR-090: fixtures/crystal/good.cr is missing"
    );
    for p in entries {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        if name.ends_with(".lock") {
            continue; // read with its .yml
        }
        let text = std::fs::read_to_string(&p).unwrap();
        let want: BTreeSet<String> = if name.starts_with("good") {
            BTreeSet::new()
        } else {
            trips(&text).into_iter().collect()
        };
        assert!(
            name.starts_with("good") || !want.is_empty(),
            "RCR-090: {name} needs a `# trips: RCR-nnn` line"
        );
        let rel = format!("{FIXTURES}{name}");
        let mut got: BTreeSet<String> = BTreeSet::new();
        if name.ends_with(".cr") {
            let s = Src::new(&rel, &text);
            for (id, f) in FILE_SCANNERS {
                // bad-format.cr and bad-ameba.cr are checked by the tools below, not by a scanner
                if !f(&s).is_empty() {
                    got.insert((*id).to_string());
                }
            }
            if name.starts_with("bad-format") || name.starts_with("bad-ameba") {
                assert!(
                    got.is_empty(),
                    "RCR-090: {name} must look clean to the scanners, trips {got:?}"
                );
                continue;
            }
        } else if name.ends_with(".yml") {
            let stem = name.trim_end_matches(".yml");
            if name.contains("lock-missing") {
                if !scan_shard_lock(&text, None).is_empty() {
                    got.insert("RCR-041".into());
                }
            } else if name.contains("lock") {
                let lock = std::fs::read_to_string(dir.join(format!("{stem}.lock"))).ok();
                if !scan_shard_lock(&text, lock.as_deref()).is_empty() {
                    got.insert("RCR-041".into());
                }
            }
            if !scan_shard_yml(&text).is_empty() {
                got.insert("RCR-040".into());
            }
        } else if name.ends_with(".build") {
            if !scan_shards_network(&text).is_empty() {
                got.insert("RCR-044".into());
            }
        } else {
            panic!("RCR-090: do not know how to check fixture {name}");
        }
        assert_eq!(
            got, want,
            "RCR-090: fixture {name} trips {got:?}, expected {want:?}"
        );
        seen_rules.extend(want);
    }
    // every scanner has a bad fixture
    let mut need: BTreeSet<String> = FILE_SCANNERS
        .iter()
        .map(|(i, _)| (*i).to_string())
        .collect();
    need.extend(["RCR-040".into(), "RCR-041".into(), "RCR-044".into()]);
    let missing: Vec<&String> = need.iter().filter(|r| !seen_rules.contains(*r)).collect();
    assert!(
        missing.is_empty(),
        "RCR-090: no bad fixture for {missing:?}"
    );

    // the tool rules prove their fixtures when the tools exist
    if let Some((maj, min)) = crystal_version().filter(|v| *v >= CRYSTAL_FLOOR) {
        let ok = |f: &str| {
            Command::new("crystal")
                .args(["tool", "format", "--check"])
                .arg(dir.join(f))
                .output()
                .is_ok_and(|o| o.status.success())
        };
        assert!(
            ok("good.cr"),
            "RCR-090: good.cr is not crystal-format clean on {maj}.{min}"
        );
        assert!(
            !ok("bad-format.cr"),
            "RCR-090: bad-format.cr passes the formatter"
        );
    } else {
        eprintln!(
            "RCR-090: formatter fixtures skipped, no crystal >= {}.{}",
            CRYSTAL_FLOOR.0, CRYSTAL_FLOOR.1
        );
    }
    if ameba_runs() {
        let run = |f: &str| {
            Command::new("ameba")
                .args(["--up-to-version", AMEBA_VERSION, "--no-color"])
                .arg(dir.join(f))
                .current_dir(repo_dir())
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        assert!(run("good.cr"), "RCR-090: good.cr is not ameba clean");
        assert!(
            !run("bad-ameba.cr"),
            "RCR-090: bad-ameba.cr passes ameba (is the config live?)"
        );
    } else {
        eprintln!("RCR-090: ameba fixtures skipped, ameba is not installed");
    }
}

#[test]
fn rcr_091_doc_and_tests_agree() {
    let doc = read_repo("docs/RULES-crystal.md").unwrap_or_default();
    assert!(!doc.is_empty(), "docs/RULES-crystal.md");
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/rules_crystal.rs"),
    )
    .unwrap();
    let mut tests: BTreeMap<String, String> = BTreeMap::new();
    for l in src.lines() {
        if let Some(r) = l.strip_prefix("fn rcr_") {
            let name = r.split('(').next().unwrap();
            tests.insert(format!("RCR-{}", &name[..3]), format!("rcr_{name}"));
        }
    }
    let mut documented = BTreeMap::new();
    let mut manual = BTreeSet::new();
    for l in doc.lines().filter(|l| l.starts_with("| RCR-")) {
        let c: Vec<&str> = l.split('|').map(str::trim).collect();
        // | id | area | rule | test | status |
        if c.len() >= 6 {
            if c[5] == "manual" {
                manual.insert(c[1].to_string());
            } else {
                documented.insert(c[1].to_string(), c[4].trim_matches('`').to_string());
            }
        }
    }
    for (id, fnname) in &documented {
        assert_eq!(
            tests.get(id),
            Some(fnname),
            "{id}: the document names {fnname}, the tests have {:?}",
            tests.get(id)
        );
    }
    for id in tests.keys() {
        assert!(
            documented.contains_key(id),
            "{id} has a test but no row in docs/RULES-crystal.md"
        );
    }
    for id in &manual {
        assert!(
            !tests.contains_key(id),
            "{id} is marked manual but has a test"
        );
    }
    // each baseline constant is named in the document
    for b in [
        "B_PURPOSE",
        "B_UNSAFE",
        "B_PROCESS_SHELL",
        "B_HTTP_TIMEOUT",
        "B_UNTYPED_PARSE",
        "B_CATCH_ALL",
        "ALLOW_CATCH_ALL",
        "B_TMP",
        "B_WEAK_HASH",
        "B_RANDOM",
        "B_HOME",
        "B_AMEBA_EXCLUDED",
        "FFI_ALLOW",
        "ALLOWED_SHARDS",
        "SHARD_CENSUS",
    ] {
        assert!(
            doc.contains(b),
            "docs/RULES-crystal.md does not mention {b}"
        );
    }
}

#[test]
fn rcr_092_baselines_are_sorted_and_unique() {
    for (n, b) in [
        ("B_PURPOSE", B_PURPOSE),
        ("B_UNSAFE", B_UNSAFE),
        ("B_PROCESS_SHELL", B_PROCESS_SHELL),
        ("B_HTTP_TIMEOUT", B_HTTP_TIMEOUT),
        ("B_TMP", B_TMP),
        ("B_WEAK_HASH", B_WEAK_HASH),
        ("B_RANDOM", B_RANDOM),
        ("B_HOME", B_HOME),
        ("B_AMEBA_EXCLUDED", B_AMEBA_EXCLUDED),
        ("FFI_ALLOW", FFI_ALLOW),
        ("ALLOWED_SHARDS", ALLOWED_SHARDS),
        ("STDLIB", STDLIB),
    ] {
        let mut s = b.to_vec();
        s.sort();
        s.dedup();
        assert_eq!(s, b, "{n} must be sorted and free of duplicates");
    }
    let allow: Vec<&str> = ALLOW_CATCH_ALL.iter().map(|(k, _, _)| *k).collect();
    let mut sorted = allow.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted, allow, "ALLOW_CATCH_ALL must be sorted and unique");
    assert!(
        ALLOW_CATCH_ALL.iter().all(|(_, _, r)| !r.is_empty()),
        "every ALLOW_CATCH_ALL entry needs a reason"
    );
    for (n, b) in [
        ("B_UNTYPED_PARSE", B_UNTYPED_PARSE),
        ("B_CATCH_ALL", B_CATCH_ALL),
    ] {
        let keys: Vec<&str> = b.iter().map(|(k, _)| *k).collect();
        let mut s = keys.clone();
        s.sort();
        s.dedup();
        assert_eq!(s, keys, "{n} must be sorted and free of duplicates");
    }
}

#[test]
fn rcr_093_every_crystal_file_is_found() {
    let n = crystal_files().len();
    assert!(
        n >= 39,
        "RCR-093: only {n} Crystal files found; the walk lost a tree"
    );
    let dirs: BTreeSet<&str> = crystal_files()
        .iter()
        .filter_map(|(f, _)| f.split('/').next())
        .collect();
    for must in ["wopkg", "hue", "crates"] {
        assert!(
            dirs.contains(must),
            "RCR-093: no Crystal file under {must}/"
        );
    }
    assert!(
        fixture_dir().join("good.cr").exists(),
        "RCR-093: tests/fixtures/crystal/good.cr"
    );
}
