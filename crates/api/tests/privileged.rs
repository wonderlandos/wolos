// SPDX-License-Identifier: GPL-2.0-or-later
//! Guard for audit A22/B12, R2C-16/R2C-17: every program a root process starts goes through
//! `wol_core::privileged` (re-exported as `api::privileged`: cleared environment, fixed PATH, no
//! shell). This test reads every file under crates/api/src and crates/controller/src/bin (the root
//! binaries wol-login, wol-session-pam and the setuid-free rest), drops every `#[cfg(test)]` item
//! by brace matching (not only the first), and counts each launch per file. A file may launch
//! another way only as often as its ALLOWED row says, each row with the reason it is safe; a row
//! whose count no longer matches fails too, so a second launch in an allowed file is caught.
use std::path::{Path, PathBuf};

/// Calls that start a process with the daemon's inherited environment.
const LAUNCHES: &[&str] = &[
    "Command::new(",
    "run_with_timeout(",
    "run_with_input(",
    "run_tracked(",
    "run_tracked_cmd(",
    "\"/bin/sh\"",
    "\"sh\"",
    // Launches through a trait or another crate (audit S14): the text guard cannot follow them.
    "PosixHost",
    "Controls::new(",
    "wol_core::system::run",
    "wait_output(",
];

/// The trees read: (name, directory relative to crates/api).
const ROOTS: &[(&str, &str)] = &[("api", "src"), ("controller-bin", "../controller/src/bin")];

/// (tree, file under it, needle, how many times, reason).
const ALLOWED: &[(&str, &str, &str, usize, &str)] = &[
    (
        "api",
        "os.rs",
        "Command::new(",
        1,
        "job runner: the program and environment come from a root-owned job file; account jobs \
         are rebuilt by as_account with a fixed PATH",
    ),
    (
        "api",
        "admind.rs",
        "run_tracked_cmd(",
        1,
        "takes the Command admind built with privileged_command",
    ),
    (
        "api",
        "privileged.rs",
        "PosixHost",
        7,
        "PrivilegedHost reads files through PosixHost; its run and run_slow go through capture",
    ),
    (
        "controller-bin",
        "wol-session-pam.rs",
        "Command::new(",
        1,
        "the account's login shell after PAM opened the session: env_clear, the session's \
         environment, and setuid to the account before exec (not a root child)",
    ),
];

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Byte index just past the item that starts at `from`: the matching `}` of its first `{`, or its
/// `;` when that comes first (`#[cfg(test)] mod tests;`). Strings, chars and comments are skipped.
fn item_end(b: &[u8], from: usize) -> usize {
    let mut i = from;
    let mut depth = 0usize;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            b'r' if (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_'))
                && matches!(b.get(i + 1), Some(b'#' | b'"')) =>
            {
                // A raw string r"..." / r#"..."#: it ends at a quote followed by as many #.
                let mut j = i + 1;
                while b.get(j) == Some(&b'#') {
                    j += 1;
                }
                if b.get(j) == Some(&b'"') {
                    let hashes = j - i - 1;
                    j += 1;
                    while j < b.len()
                        && !(b[j] == b'"'
                            && b.len() > j + hashes
                            && b[j + 1..=j + hashes].iter().all(|c| *c == b'#'))
                    {
                        j += 1;
                    }
                    i = j + hashes;
                }
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'\'' if b.get(i + 2) == Some(&b'\'') || b.get(i + 1) == Some(&b'\\') => {
                // A char literal ('{', '\''), not a lifetime.
                i += 1;
                while i < b.len() && b[i] != b'\'' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            b';' if depth == 0 => return i + 1,
            _ => {}
        }
        i += 1;
    }
    b.len()
}

/// `foo/tests.rs` is a test item only when `foo.rs` (or `foo/mod.rs`) declares it as
/// `#[cfg(test)]` directly followed by `mod tests;`: any other `tests.rs` is shipped code.
fn is_test_module_file(path: &Path) -> bool {
    if path.file_name().is_none_or(|n| n != "tests.rs") {
        return false;
    }
    let Some(dir) = path.parent() else {
        return false;
    };
    let Some(name) = dir.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let parents = [dir.with_file_name(format!("{name}.rs")), dir.join("mod.rs")];
    parents
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .any(|text| declares_test_module(&text))
}

/// True when `text` has `#[cfg(test)]` on the line right before `mod tests;`.
fn declares_test_module(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    lines
        .windows(2)
        .any(|w| w[0] == "#[cfg(test)]" && w[1] == "mod tests;")
}

/// `text` without its `#[cfg(test)]` items.
fn without_tests(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..at]);
        let after = at + "#[cfg(test)]".len();
        let end = after + item_end(&rest.as_bytes()[after..], 0);
        rest = &rest[end.min(rest.len())..];
    }
    out.push_str(rest);
    out
}

/// (tree, file, needle) -> count, over the non-test code, comment lines skipped.
fn launches() -> Vec<(String, String, &'static str, usize, Vec<String>)> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    for (tree, dir) in ROOTS {
        let root = base.join(dir);
        let mut all = Vec::new();
        files(&root, &mut all);
        assert!(!all.is_empty(), "found the sources of {tree}");
        for p in &all {
            let rel = p
                .strip_prefix(&root)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            if is_test_module_file(p) {
                continue;
            }
            let code = without_tests(&std::fs::read_to_string(p).unwrap_or_default());
            for needle in LAUNCHES {
                let lines: Vec<String> = code
                    .lines()
                    .map(str::trim_start)
                    .filter(|l| !l.starts_with("//") && l.contains(needle))
                    .map(str::to_owned)
                    .collect();
                if !lines.is_empty() {
                    found.push(((*tree).to_owned(), rel.clone(), *needle, lines.len(), lines));
                }
            }
        }
    }
    found
}

#[test]
fn no_root_side_process_launch_bypasses_privileged_command() {
    let mut bad = Vec::new();
    for (tree, file, needle, n, lines) in launches() {
        let allowed = ALLOWED
            .iter()
            .find(|(t, f, x, _, _)| *t == tree && *f == file && *x == needle)
            .map_or(0, |r| r.3);
        if n > allowed {
            bad.push(format!(
                "{tree}/{file}: {needle} {n} times, {allowed} allowed:\n  {}",
                lines.join("\n  ")
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "use wol_core::privileged::privileged_command (or an ALLOWED row with the reason):\n{}",
        bad.join("\n")
    );
}

#[test]
fn every_allowed_row_is_exact() {
    let found = launches();
    for (tree, file, needle, count, reason) in ALLOWED {
        let n = found
            .iter()
            .find(|(t, f, x, _, _)| t == tree && f == file && x == needle)
            .map_or(0, |r| r.3);
        assert_eq!(
            n, *count,
            "{tree}/{file}: {needle} is used {n} times, the row says {count} ({reason})"
        );
    }
}

#[test]
fn test_items_are_dropped_by_brace_matching() {
    let text = "a();\n#[cfg(test)]\nmod t { fn x() { let s = \"}\"; let c = '}'; let r = r#\"\"}\"#; Command::new(\"x\"); } }\n\
                b();\n#[cfg(test)]\nuse y;\nc();\n#[cfg(test)]\nfn z() { Command::new(\"y\"); }\nd();\n";
    let code = without_tests(text);
    assert!(!code.contains("Command::new"), "{code}");
    for keep in ["a();", "b();", "c();", "d();"] {
        assert!(code.contains(keep), "{keep} in {code}");
    }
}

#[test]
fn only_a_declared_test_module_file_is_skipped() {
    assert!(declares_test_module(
        "fn a() {}\n#[cfg(test)]\nmod tests;\n"
    ));
    assert!(
        !declares_test_module("mod tests;\n"),
        "not behind cfg(test)"
    );
    assert!(!declares_test_module(
        "#[cfg(test)]\nfn f() {}\nmod tests;\n"
    ));
    assert!(!declares_test_module("#[cfg(unix)]\nmod tests;\n"));
    // The real files: admind/tests.rs is the test module of admind.rs, a made-up one is not.
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(is_test_module_file(&src.join("admind/tests.rs")));
    assert!(!is_test_module_file(&src.join("admind.rs")));
    assert!(!is_test_module_file(&src.join("nothing/tests.rs")));
}
