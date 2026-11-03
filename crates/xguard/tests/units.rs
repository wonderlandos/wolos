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
/// and never changes an owner, so it holds no CAP_CHOWN and sees homes read-only.
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
        let caps: Vec<&str> = caps.split_whitespace().collect();
        assert_eq!(caps, ["CAP_DAC_READ_SEARCH"], "{key}: read, nothing else");
    }
    assert_eq!(value("ProtectHome"), "read-only");
    assert_eq!(value("ReadWritePaths"), "/var/lib/wolos/xguard");
}
