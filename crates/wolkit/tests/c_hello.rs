// SPDX-License-Identifier: BSD-2-Clause
//! A C program on libWolkit (ADR 0039): `tests/c/hello.c` is compiled with `cc` against
//! `include/wolkit.h`, linked against the built `libwolkit.so.1` (by soname) and run. Proves the C
//! ABI end to end on the headless backend: a window, a patch built by hand, a broken patch that
//! changes nothing, a click injected at a node's rectangle, and the event it makes.

#[path = "support/mod.rs"]
mod common;

#[test]
fn hello_from_c() {
    let dir = common::temp("hello");
    let exe = common::build("hello", &dir);
    let out = std::process::Command::new(&exe).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (a, b, c) = wolkit::VERSION;
    let want = [
        format!("version: {a}.{b}.{c} {} 1.0.0", (a << 16) | (b << 8) | c),
        "backend: 1".to_owned(),
        "fd: yes".to_owned(),
        "bad id: 1 An app id is reverse-DNS, like org.example.hello (letters, digits, '.', '-', '_').".to_owned(),
        "broken patch: 1 Part 99 is not a part of this window (it was never created, or was freed).".to_owned(),
        "button text: Add".to_owned(),
        "hit: 2".to_owned(),
        "events pending: yes".to_owned(),
        "click: node 2 window ok seq 1".to_owned(),
        "clicks: 1".to_owned(),
        "a11y: a11y nodes=3".to_owned(),
        "stats: one patch".to_owned(),
        "wrong handle: 1".to_owned(),
        "done".to_owned(),
    ];
    let lines: Vec<&str> = stdout.lines().collect();
    for w in &want {
        assert!(lines.contains(&w.as_str()), "missing {w:?} in\n{stdout}");
    }
    assert!(
        stdout.contains("    2 button role=button name=\"Add\" focused"),
        "{stdout}"
    );
    // The scene survives a broken patch: the button text is still there, the sequence unchanged.
    assert_eq!(lines.last(), Some(&"done"));
}
