// SPDX-License-Identifier: GPL-2.0-or-later
//! rsh rules (docs/modules/rsh.md "Rules", ids RSH-nnn; ADR 0041): the shell language in rsh/.
//! The KINDS row `wol script` uses for it matches the document, every `.rsh` file in the tree has
//! the header and parses, the interpreter is fetched at one pinned commit with a checksum and never
//! kept in the tree, and upstream names stay out of rsh's own text. `rsh -n` runs when a built
//! binary is found (rsh/build/rsh, or `rsh` on PATH); WOLOS_REQUIRE_RSH=1 turns a missing binary
//! into a failure (CI sets it). `rsh_100_doc_and_tests_agree` ties the tests to the document.
#[path = "support/repo.rs"]
mod repo;

use controller::script::{KINDS, kind_of};
use repo::{read, repo};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Folders that hold no source of ours (the build tree, cargo's output, git).
const SKIP_DIRS: &[&str] = &["build", "target", ".git", "node_modules"];
/// Names of upstream projects that must not appear in rsh's own text (lowercase, whole words).
const UPSTREAM: &[&str] = &["mruby", "matz", "cruby"];
/// Size limit for the stripped program, bytes (docs/modules/rsh.md).
const MAX_BYTES: u64 = 3 * 1024 * 1024;

/// Every file under `dir` (repo-relative to `root`), skipping [`SKIP_DIRS`].
fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut ents: Vec<_> = rd.flatten().collect();
    ents.sort_by_key(|e| e.file_name());
    for e in ents {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                walk(root, &p, out);
            }
        } else {
            out.push(p.strip_prefix(root).unwrap().to_path_buf());
        }
    }
}

/// All `.rsh` files and all files with an rsh shebang, outside the scanned folders' fixtures.
fn rsh_files() -> Vec<String> {
    let root = repo("");
    let mut all = Vec::new();
    for top in ["rsh", "files", "desktop", "tools", "docs", "wopkg"] {
        walk(&root, &root.join(top), &mut all);
    }
    let mut found = Vec::new();
    for rel in all {
        let rel = rel.to_string_lossy().into_owned();
        let Ok(text) = std::fs::read_to_string(root.join(&rel)) else {
            continue;
        };
        let first = text.lines().next().unwrap_or("");
        if rel.ends_with(".rsh") || first.starts_with("#!/usr/bin/rsh") {
            found.push(rel);
        }
    }
    found
}

fn require_tools() -> bool {
    std::env::var("WOLOS_REQUIRE_RSH").is_ok_and(|v| v == "1")
}

/// The built rsh: the one `make -C rsh` produced, else one on PATH.
fn rsh_binary() -> Option<PathBuf> {
    let built = repo("rsh/build/rsh");
    if built.is_file() {
        return Some(built);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("rsh"))
        .find(|p| p.is_file())
}

fn need_binary(rule: &str) -> Option<PathBuf> {
    let b = rsh_binary();
    if b.is_none() {
        eprintln!("{rule}: rsh is not built (make -C rsh); skipped");
        assert!(
            !require_tools(),
            "{rule}: rsh missing (WOLOS_REQUIRE_RSH=1)"
        );
    }
    b
}

fn doc() -> String {
    read("docs/modules/rsh.md")
}

#[test]
fn rsh_001_the_kinds_row_matches_the_document() {
    let k = KINDS.iter().find(|k| k.id == "rsh").expect("KINDS has rsh");
    assert_eq!(k.interpreters, ["rsh"]);
    assert_eq!(k.extensions, [".rsh"]);
    assert_eq!(k.program, "/usr/bin/rsh");
    assert_eq!(k.syntax, ["-n"]);
    assert_eq!(k.stdin_args, ["-s", "--"]);
    assert_eq!(k.lint, Some(("rsh", &["--lint"][..])));
    let d = doc();
    for needle in [
        "| `rsh` | `.rsh` | `/usr/bin/rsh` | `rsh -n` | `rsh --lint` |",
        "#!/usr/bin/rsh",
        "#!/usr/bin/env rsh",
    ] {
        assert!(d.contains(needle), "docs/modules/rsh.md lacks {needle}");
    }
    assert_eq!(kind_of("x", "#!/usr/bin/rsh\n").unwrap().id, "rsh");
    assert_eq!(kind_of("x.rsh", "echo\n").unwrap().id, "rsh");
    let scripting = read("docs/modules/scripting.md");
    assert!(
        scripting.contains("| `rsh` | `.rsh` |"),
        "scripting.md table"
    );
}

#[test]
fn rsh_002_every_rsh_file_has_the_header() {
    let files = rsh_files();
    assert!(files.len() >= 13, "golden scripts and examples: {files:?}");
    for f in files {
        let text = read(&f);
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("#!/usr/bin/rsh"), "{f}: line 1");
        assert_eq!(
            lines.next(),
            Some("# SPDX-License-Identifier: GPL-2.0-or-later"),
            "{f}: line 2"
        );
        assert!(
            lines.next().is_some_and(|l| l.starts_with("# ")),
            "{f}: line 3 says what the script is for"
        );
    }
}

#[test]
fn rsh_003_every_rsh_file_parses() {
    let Some(rsh) = need_binary("RSH-003") else {
        return;
    };
    for f in rsh_files() {
        let o = Command::new(&rsh).arg("-n").arg(repo(&f)).output().unwrap();
        assert!(
            o.status.success(),
            "rsh -n {f}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

#[test]
fn rsh_004_the_interpreter_is_pinned_and_never_in_the_tree() {
    let m = read("rsh/Makefile");
    let value = |key: &str| {
        m.lines()
            .find_map(|l| l.strip_prefix(key)?.trim_start().strip_prefix('='))
            .map(|v| v.trim().to_string())
            .unwrap_or_else(|| panic!("rsh/Makefile has no {key}"))
    };
    let commit = value("MRUBY_COMMIT");
    let sum = value("MRUBY_SHA256");
    let hex = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit());
    assert!(hex(&commit, 40), "a full commit id: {commit}");
    assert!(hex(&sum, 64), "a sha256: {sum}");
    assert!(m.contains("sha256sum -c"), "the download is checked");
    assert!(
        m.contains("MRUBY_TARBALL ?="),
        "the package hands in its own copy"
    );
    // Only these folders belong to rsh; a copy of the interpreter would be a new one.
    let root = repo("rsh");
    let mut names: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "build")
        .collect();
    names.sort();
    let want = [
        ".gitignore",
        "Makefile",
        "VERSION",
        "build_config.rb",
        "build_config.rb.lock",
        "rsh.1",
        "src",
        "tests",
    ];
    assert!(
        names.iter().all(|n| want.contains(&n.as_str())),
        "rsh/ holds only {want:?}: {names:?}"
    );
    let ignore = read("rsh/.gitignore");
    assert!(ignore.lines().any(|l| l == "build/"), "build/ is ignored");
}

/// Words of `line` (letters and digits), lowercased.
fn words(line: &str) -> Vec<String> {
    line.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

#[test]
fn rsh_005_upstream_names_stay_out_of_rsh_text() {
    let mut files: Vec<String> = vec!["rsh/rsh.1".into(), "docs/modules/rsh.md".into()];
    let root = repo("");
    let mut all = Vec::new();
    walk(&root, &root.join("rsh/src"), &mut all);
    walk(&root, &root.join("rsh/tests"), &mut all);
    files.extend(all.iter().map(|p| p.to_string_lossy().into_owned()));
    for f in files {
        let Ok(text) = std::fs::read_to_string(root.join(&f)) else {
            continue;
        };
        for (n, l) in text.lines().enumerate() {
            // The headers of the embedded interpreter and the regex library are build inputs.
            if l.trim_start().starts_with("#include <mruby") || l.contains("PCRE2_") {
                continue;
            }
            for w in words(l) {
                assert!(
                    !UPSTREAM.contains(&w.as_str()),
                    "{f}:{}: names an upstream project ({w}); say \"the interpreter\"",
                    n + 1
                );
            }
        }
    }
}

#[test]
fn rsh_006_the_binary_is_small_and_stripped() {
    let Some(rsh) = need_binary("RSH-006") else {
        return;
    };
    let len = std::fs::metadata(&rsh).unwrap().len();
    assert!(
        len < MAX_BYTES,
        "{} is {len} bytes, limit {MAX_BYTES}",
        rsh.display()
    );
}

#[test]
fn rsh_007_the_man_page_and_the_package_text_exist() {
    let man = read("rsh/rsh.1");
    assert!(man.starts_with(".\\\" SPDX-License-Identifier: GPL-2.0-or-later"));
    for section in ["NAME", "SYNOPSIS", "DESCRIPTION", "EXIT STATUS", "SEE ALSO"] {
        assert!(man.contains(&format!(".SH {section}")), "rsh.1: {section}");
    }
    assert!(doc().contains("docs/modules/rsh.PKGBUILD"));
    let pkg = read("docs/modules/rsh.PKGBUILD");
    assert!(pkg.contains("depends=(glibc pcre2)") && pkg.contains("sha256sums="));
    assert!(
        pkg.contains("LINK=dynamic"),
        "no static regex library on the base"
    );
    let mk = read("rsh/Makefile");
    assert!(
        mk.contains("rsh.1"),
        "make install puts the man page in place"
    );
}

#[test]
fn rsh_008_the_terminal_menu_lists_rsh_beside_the_others() {
    use wol_core::terminal_shell as ts;
    let ids: Vec<&str> = ts::ids().collect();
    assert_eq!(ids, ["zsh", "bash", "rsh", "fish", "nu", "pwsh"]);
    let launcher = read("desktop/overlay/usr/bin/wol-term-shell");
    assert!(launcher.contains("rsh) p=$root/usr/bin/rsh ;;"));
}

#[test]
fn rsh_009_the_exec_bit_list_holds_every_rsh_script() {
    let list = read("tools/exec-bits.txt");
    let listed: Vec<&str> = list.lines().collect();
    for f in rsh_files() {
        assert!(
            listed.contains(&f.as_str()),
            "tools/exec-bits.txt lacks {f}"
        );
    }
}

#[test]
fn rsh_100_doc_and_tests_agree() {
    let d = doc();
    let src =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/rules_rsh.rs"))
            .unwrap();
    let mut tests: BTreeMap<String, String> = BTreeMap::new();
    for l in src.lines() {
        if let Some(r) = l.strip_prefix("fn rsh_") {
            let name = r.split('(').next().unwrap();
            if !name[..3].bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            tests.insert(format!("RSH-{}", &name[..3]), format!("rsh_{name}"));
        }
    }
    let mut documented = BTreeMap::new();
    for l in d.lines().filter(|l| l.starts_with("| RSH-")) {
        let c: Vec<&str> = l.split('|').map(str::trim).collect();
        // | id | rule | test |
        if c.len() >= 4 {
            documented.insert(c[1].to_string(), c[3].trim_matches('`').to_string());
        }
    }
    assert_eq!(
        documented, tests,
        "docs/modules/rsh.md rules (id, test) and the rsh_* tests differ"
    );
}
