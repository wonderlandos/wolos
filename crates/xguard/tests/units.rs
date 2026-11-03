// SPDX-License-Identifier: GPL-2.0-or-later
//! Kernel marks hold on mounts. In a mount namespace of its own the guard would mark copies nobody
//! else uses and stop nothing, without a word. Found by running the real unit.
#[test]
fn the_guard_unit_stays_in_the_mount_namespace_everyone_else_uses() {
    let unit = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../files/usr/lib/systemd/system/wolos-xguard.service"
    );
    let text = std::fs::read_to_string(unit).unwrap();
    for option in [
        "PrivateNetwork",
        "ProtectSystem",
        "ProtectHome",
        "PrivateTmp",
        "PrivateMounts",
        "PrivateDevices",
        "ReadWritePaths",
        "ReadOnlyPaths",
        "InaccessiblePaths",
        "TemporaryFileSystem",
        "BindPaths",
        "ProtectKernelTunables",
        "ProtectKernelModules",
        "ProtectKernelLogs",
        "ProtectControlGroups",
        "ProtectProc",
        "RootDirectory",
        "RootImage",
    ] {
        let used = text.lines().any(|l| l.starts_with(&format!("{option}=")));
        assert!(!used, "{option} gives the guard a mount namespace");
    }
}

/// YARA-X turns its rules into machine code when they load (wasmtime). With
/// MemoryDenyWriteExecute the deep scan forked by the guard, and the sweep's, cannot load them
/// and every script, ELF or AppImage passes on hashes alone (audit S4).
#[test]
fn units_that_run_the_deep_scan_may_make_code() {
    for unit in ["wolos-xguard.service", "wolos-xguard-sweep.service"] {
        let path = format!(
            "{}/../../files/usr/lib/systemd/system/{unit}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(path).unwrap();
        let mdwe = text
            .lines()
            .any(|l| l.trim_start().starts_with("MemoryDenyWriteExecute="));
        assert!(
            !mdwe,
            "{unit}: MemoryDenyWriteExecute breaks the YARA rules"
        );
    }
}

/// The guard's worst case (every thread holding a whole file, and its worker another, plus the
/// signatures and rules) fits the unit's MemoryMax: an OOM kill would leave every open
/// unanswered until the restart, which the kernel lets through (audit S8).
#[test]
fn the_guard_fits_its_memory_limit() {
    let unit = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../files/usr/lib/systemd/system/wolos-xguard.service"
    );
    let text = std::fs::read_to_string(unit).unwrap();
    let max = text
        .lines()
        .find_map(|l| l.strip_prefix("MemoryMax="))
        .expect("wolos-xguard.service sets MemoryMax");
    let (num, unit_shift) = match max.trim().chars().last() {
        Some('K') => (&max[..max.len() - 1], 10),
        Some('M') => (&max[..max.len() - 1], 20),
        Some('G') => (&max[..max.len() - 1], 30),
        _ => (max, 0),
    };
    let bytes = num.trim().parse::<u64>().unwrap() << unit_shift;
    assert!(
        xguard::guard::MEMORY_NEED <= bytes,
        "{} threads x 2 x {} + {} > MemoryMax={max}",
        xguard::guard::THREADS,
        xguard::guard::WHOLE,
        xguard::guard::RESERVE
    );
}

/// Audit S4-3: the socket queue and the workers fit the unit's descriptor limit with room for the
/// kernel's questions (one descriptor each while queued). Out of descriptors, `fanotify_read`
/// fails and the kernel refuses every pending exec.
#[test]
fn the_guard_fits_its_descriptor_limit() {
    let unit = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../files/usr/lib/systemd/system/wolos-xguard.service"
    );
    let text = std::fs::read_to_string(unit).unwrap();
    let limit: usize = text
        .lines()
        .find_map(|l| l.strip_prefix("LimitNOFILE="))
        .expect("wolos-xguard.service sets LimitNOFILE")
        .trim()
        .parse()
        .expect("one number (soft = hard)");
    use xguard::guard::{FD_NEED, QUEUE_CAP, THREADS};
    assert!(
        QUEUE_CAP * 2 + THREADS * 4 < limit,
        "QUEUE_CAP * 2 + THREADS * 4 >= LimitNOFILE={limit}"
    );
    // At least 4x headroom for kernel questions waiting on the three workers.
    assert!(
        FD_NEED * 4 <= limit,
        "FD_NEED {FD_NEED} x 4 > LimitNOFILE={limit}"
    );
}

/// Audit P6-2: the sweep parses hostile files in every home. It reads them (CAP_DAC_READ_SEARCH)
/// and sees homes read-only. Audit R7-1 / DT-6: each account's findings list is handed to the
/// guard over its socket (`findings <uid>`), which writes it as root, so the sweep holds no
/// CAP_CHOWN, and its code changes no owner (lesson: a dropped capability means grepping the code
/// for the calls it gated).
#[test]
fn the_sweep_reads_homes_and_cannot_change_them() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../files/usr/lib/systemd/system/wolos-xguard-sweep.service"
    );
    let text = std::fs::read_to_string(path).unwrap();
    let value = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("the sweep unit sets {key}"))
            .to_owned()
    };
    for key in ["AmbientCapabilities", "CapabilityBoundingSet"] {
        let caps = value(key);
        let mut caps: Vec<&str> = caps.split_whitespace().collect();
        caps.sort_unstable();
        assert_eq!(caps, ["CAP_DAC_READ_SEARCH"], "{key}: read; nothing else");
    }
    assert!(
        !text.contains("CAP_CHOWN="),
        "no capability line grants CAP_CHOWN"
    );
    // The findings arrive through the guard: the pass after it, handing lists over its socket,
    // and never writing one for another account itself.
    assert!(
        text.lines()
            .any(|l| l.trim() == format!("After={}", wol_core::malware::UNIT)),
        "the sweep runs after the guard, whose socket takes its lists"
    );
    let bin = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/bin/wol-xguard.rs"
    ))
    .unwrap();
    let body = &bin[bin.find("fn sweep(").expect("the sweep")..];
    let body = &body[..body.find("\n}\n").expect("its end")];
    assert!(
        body.contains("xguard::ctl::hand_findings("),
        "lists go to the guard"
    );
    for gated in ["write_as", "chown", "fchown"] {
        assert!(
            !body.contains(gated),
            "the sweep calls {gated}: it needs CAP_CHOWN again"
        );
    }
    assert_eq!(value("User"), "_xguard");
    assert_eq!(value("ProtectSystem"), "strict");
    assert_eq!(value("ProtectHome"), "read-only");
    assert_eq!(value("ReadWritePaths"), wol_core::malware::SWEEP_DIR);
    for key in [
        "PrivateTmp",
        "PrivateDevices",
        "ProtectKernelTunables",
        "ProtectControlGroups",
        "NoNewPrivileges",
    ] {
        assert_eq!(
            value(key),
            "yes",
            "{key}: the pass reaches its own folder only"
        );
    }
}

/// Audit F10: nothing `_xguard` runs may replace what the guard keeps. The top folder, which the
/// full database's updater (as `_xguard`) writes, is sticky; the guard's folder is root's alone
/// and not writable by others; the background pass has its own, the only one its unit may write.
#[test]
fn the_guards_state_is_out_of_xguards_reach() {
    let conf = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../files/usr/lib/tmpfiles.d/wolos.conf"
    ))
    .unwrap();
    let line = |path: &str| {
        conf.lines()
            .map(|l| l.split_whitespace().collect::<Vec<_>>())
            .find(|w| w.first() == Some(&"d") && w.get(1) == Some(&path))
            .unwrap_or_else(|| panic!("tmpfiles makes {path}"))
            .iter()
            .skip(2)
            .take(3)
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    };
    use wol_core::malware::{GUARD_DIR, STATE_DIR, SWEEP_DIR};
    assert_eq!(line(STATE_DIR), ["1775", "root", "_xguard"], "sticky");
    assert_eq!(line(GUARD_DIR), ["0755", "root", "root"]);
    assert_eq!(line(SWEEP_DIR), ["0755", "_xguard", "_xguard"]);
    for d in [GUARD_DIR, SWEEP_DIR] {
        assert_eq!(
            std::path::Path::new(d).parent(),
            Some(std::path::Path::new(STATE_DIR))
        );
    }
    // Files from before the split are removed, not adopted: any of them may be _xguard's making.
    for old in ["verdicts", "status.json", "findings-*.json", "sweep-*"] {
        assert!(
            conf.lines()
                .any(|l| l.trim() == format!("r {STATE_DIR}/{old}")),
            "tmpfiles removes the old {old}"
        );
    }
    // DT-6: the pass's own lists are the guard's to write now; the old ones go, its memory stays.
    assert!(
        conf.lines()
            .any(|l| l.trim() == format!("r {SWEEP_DIR}/sweep-*.json")),
        "tmpfiles removes the pass's old lists"
    );
    assert!(
        !conf.lines().any(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            matches!(w.first(), Some(&"r") | Some(&"R"))
                && w.get(1).is_some_and(|p| {
                    p.ends_with("sweep-verdicts") || *p == format!("{SWEEP_DIR}/*")
                })
        }),
        "the pass keeps its memory"
    );
    for old in ["rules", "local"] {
        assert!(
            conf.lines()
                .any(|l| l.trim() == format!("R {STATE_DIR}/{old}")),
            "tmpfiles removes the old {old}/ (_xguard could write there)"
        );
    }
    // The full database: fetched as _xguard, built by root only from signed files, before the
    // guard restarts onto it.
    let clam = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../files/usr/lib/systemd/system/wolos-clam-update.service"
    ))
    .unwrap();
    let post: Vec<&str> = clam
        .lines()
        .filter_map(|l| l.strip_prefix("ExecStartPost="))
        .collect();
    assert_eq!(
        post.first().copied(),
        Some("+/usr/bin/wol-xguard rules full"),
        "the root build comes first: {post:?}"
    );
    assert!(clam.lines().any(|l| l.trim() == "User=_xguard"));
    let fetch = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../files/usr/lib/wolos/clam-update"
    ))
    .unwrap();
    assert!(
        fetch.contains("ScriptedUpdates no"),
        "whole signed files only"
    );
    assert!(
        !fetch.contains("rules build"),
        "the fetch builds nothing the guard loads"
    );
    // The guard and the pass default to their own folders.
    let bin = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/bin/wol-xguard.rs"
    ))
    .unwrap();
    assert!(bin.contains("state: wol_core::malware::GUARD_DIR.into()"));
    assert!(bin.contains(".map_or(wol_core::malware::SWEEP_DIR, String::as_str)"));
    // No other unit that runs as _xguard may write the guard's folder.
    let units = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../files/usr/lib/systemd/system"
    );
    for e in std::fs::read_dir(units).unwrap().flatten() {
        let text = std::fs::read_to_string(e.path()).unwrap_or_default();
        if text.lines().any(|l| l.trim() == "User=_xguard") {
            assert!(
                !text.contains(GUARD_DIR),
                "{}: an _xguard unit names the guard's folder",
                e.path().display()
            );
        }
    }
}
