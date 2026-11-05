// SPDX-License-Identifier: BSD-2-Clause
//! Guard for STANDARDS 7.1 "Identity" (R2C-22): nothing reads who a process is through
//! `/proc/<pid>` by number. A peer's `/proc/<pid>` is opened as a directory once, at accept, and its
//! executable is read through that descriptor (`/proc/self/fd/<n>/exe`: `api::peer::PeerIdentity`,
//! the shell socket's `Pinned` in crates/shell/src/ui/ipc.rs). A pid number can be reused by the
//! time it is looked up again, and the lookup then names the newcomer. This test reads the
//! non-test code of every crate's `src/` and fails on any `/proc/<number>/<file>` read built from a
//! pid number (`exe`, `environ`, `cgroup`, `cmdline`: all go through the pinned descriptor, QA-A9-SEC-6).
use std::path::{Path, PathBuf};

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Known exceptions: (file under crates/, reason). Each is an open item, not a pattern to copy.
const ALLOWED: &[(&str, &str)] = &[
    (
        "core/src/monitor/procs.rs",
        "the System Monitor's process list reads the cgroup of every process it lists, to group \
         them into apps; the text is shown, it decides no grant and no identity",
    ),
    (
        "shared/src/sys/mod.rs",
        "sys::executable(pid): the realtime server's per-connection identity \
         (crates/realtime/src/identity.rs) still looks the pid up; it should pin /proc/<pid> at \
         accept like api::peer (reported with R2C-22)",
    ),
];

/// The files of `/proc/<pid>` that say who a process is or what it carries. (Statistics such as
/// `stat`, `smaps_rollup` and `oom_score_adj` describe a process for a monitor or a tuner; they
/// grant nothing.)
const IDENTITY_FILES: &[&str] = &[
    "exe", "environ", "cgroup", "cmdline", "maps", "status", "comm", "root", "cwd",
];

/// A line that builds `/proc/<number>/<file>` from a number (`/proc/{pid}/exe`, `/proc/{pid}/environ`,
/// `/proc/{}/cgroup`), not from `self` or a descriptor. Opening the directory itself
/// (`/proc/{pid}`) is how a process is pinned, so a path that ends there is not a read.
fn reads_exe_by_number(line: &str) -> bool {
    let l = line.trim_start();
    if l.starts_with("//") {
        return false;
    }
    let mut from = 0;
    while let Some(at) = l[from..].find("/proc/{") {
        let rest = &l[from + at..];
        from += at + "/proc/{".len();
        if rest.starts_with("/proc/self") {
            continue;
        }
        // `/proc/{pid}/file`: an identity file follows the closing brace of the placeholder.
        if let Some(close) = rest.find('}')
            && let Some(file) = rest[close + 1..].strip_prefix('/')
            && IDENTITY_FILES.iter().any(|f| {
                file.strip_prefix(f)
                    .is_some_and(|r| !r.starts_with(|c: char| c.is_ascii_alphanumeric()))
            })
        {
            return true;
        }
    }
    false
}

#[test]
fn the_rule_sees_what_it_should() {
    assert!(reads_exe_by_number(
        r#"std::fs::read_link(format!("/proc/{pid}/exe"))"#
    ));
    assert!(reads_exe_by_number(r#"format!("/proc/{}/exe", p.pid)"#));
    // QA-A9-SEC-6: the two reads that were missed because only `exe` counted.
    assert!(reads_exe_by_number(
        r#"    std::fs::File::open(format!("/proc/{pid}/environ"))"#
    ));
    assert!(reads_exe_by_number(
        r#"    std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()"#
    ));
    assert!(!reads_exe_by_number(
        r#"format!("/proc/self/fd/{}/exe", dir.as_raw_fd())"#
    ));
    assert!(!reads_exe_by_number(
        r#"File::open(format!("/proc/{pid}"))"#
    ));
    assert!(!reads_exe_by_number(r#"// format!("/proc/{pid}/exe")"#));
}

#[test]
fn nothing_reads_a_proc_file_by_pid_number() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for e in std::fs::read_dir(&crates).expect("crates/").flatten() {
        rs_files(&e.path().join("src"), &mut files);
    }
    assert!(files.len() > 100, "found the sources");
    let mut bad = Vec::new();
    for f in &files {
        let rel = f
            .strip_prefix(&crates)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if ALLOWED.iter().any(|(a, _)| *a == rel) {
            continue;
        }
        let text = std::fs::read_to_string(f).unwrap_or_default();
        // Test modules look processes up on purpose (sleep children, exec races).
        let code = text.split("#[cfg(test)]").next().unwrap_or("");
        for (n, line) in code.lines().enumerate() {
            if reads_exe_by_number(line) {
                bad.push(format!("{}:{}: {}", f.display(), n + 1, line.trim()));
            }
        }
    }
    for (a, why) in ALLOWED {
        let text = std::fs::read_to_string(crates.join(a)).unwrap_or_default();
        assert!(
            text.lines().any(reads_exe_by_number),
            "{a} no longer reads by number ({why}): drop the row"
        );
    }
    assert!(
        bad.is_empty(),
        "read the executable through a /proc/<pid> descriptor opened at accept \
         (api::peer::PeerIdentity), never by pid number:\n{}",
        bad.join("\n")
    );
}
