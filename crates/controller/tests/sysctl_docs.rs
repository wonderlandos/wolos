// SPDX-License-Identifier: GPL-2.0-or-later
//! Doc-vs-code for kernel hardening (audit A15): every sysctl docs/modules/security.md names in its
//! "Kernel hardening" line, and every row of `wol_core::security::HARDENING` (what `wol security`
//! reports), is set in files/usr/lib/sysctl.d with the value the code expects. The audit found
//! settings the documentation promised and no file set.
#[path = "support/siblings.rs"]
mod siblings;

use std::collections::BTreeMap;

/// Every `key = value` line of the shipped sysctl.d files; a later file wins (systemd-sysctl).
fn shipped() -> BTreeMap<String, String> {
    let dir = siblings::root().join("files/usr/lib/sysctl.d");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .expect("sysctl.d")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "conf"))
        .collect();
    names.sort();
    let mut m = BTreeMap::new();
    for p in names {
        for l in std::fs::read_to_string(&p).unwrap_or_default().lines() {
            let l = l.trim();
            if l.starts_with('#') || l.starts_with(';') {
                continue;
            }
            if let Some((k, v)) = l.split_once('=') {
                m.insert(k.trim().to_owned(), v.trim().to_owned());
            }
        }
    }
    m
}

/// Backticked names on the doc's "Kernel hardening" paragraph, as full sysctl keys.
fn documented() -> Vec<String> {
    let doc = std::fs::read_to_string(siblings::root().join("docs/modules/security.md"))
        .expect("docs/modules/security.md");
    let start = doc
        .find("Kernel hardening")
        .expect("a Kernel hardening line");
    let para = doc[start..].split("\n\n").next().unwrap_or("");
    let para = para.split("\n- ").next().unwrap_or(para);
    let mut out = Vec::new();
    for (i, tick) in para.split('`').enumerate() {
        if i % 2 == 0 || tick.starts_with('/') || tick.contains(' ') {
            continue;
        }
        let name = tick.split('=').next().unwrap_or(tick);
        // `fs.protected_hardlinks/symlinks` names two keys: the second shares `fs.protected_`.
        let mut parts = name.split('/');
        let first = parts.next().unwrap_or(name);
        let stem = first.rfind('_').map_or("", |i| &first[..=i]);
        out.push(full_key(first));
        for p in parts {
            out.push(full_key(&format!("{stem}{p}")));
        }
    }
    out
}

/// The doc shortens kernel.* names (`dmesg_restrict`, `yama.ptrace_scope`).
fn full_key(k: &str) -> String {
    if k.starts_with("kernel.")
        || k.starts_with("fs.")
        || k.starts_with("net.")
        || k.starts_with("vm.")
    {
        k.to_owned()
    } else {
        format!("kernel.{k}")
    }
}

#[test]
fn documented_sysctls_are_set() {
    let set = shipped();
    let docs = documented();
    assert!(docs.len() >= 6, "parsed the doc: {docs:?}");
    for k in &docs {
        assert!(
            set.contains_key(k),
            "docs/modules/security.md promises {k}; no sysctl.d file sets it"
        );
    }
}

#[test]
fn what_wol_security_reports_is_what_boot_sets() {
    let set = shipped();
    for (k, want, _) in wol_core::security::HARDENING {
        assert_eq!(set.get(*k).map(String::as_str), Some(*want), "{k}");
    }
}

#[test]
fn audit_a15_settings_are_set() {
    let set = shipped();
    for (k, v) in [
        ("kernel.unprivileged_bpf_disabled", "1"),
        ("net.ipv4.tcp_syncookies", "1"),
        ("fs.protected_fifos", "1"),
        ("fs.protected_regular", "1"),
        ("net.core.bpf_jit_harden", "1"),
        ("kernel.kexec_load_disabled", "1"),
    ] {
        assert_eq!(set.get(k).map(String::as_str), Some(v), "{k}");
    }
}
