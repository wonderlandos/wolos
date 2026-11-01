// SPDX-License-Identifier: GPL-2.0-or-later
//! One app cannot take the desktop with it (ADR 0027): the defaults that keep the session alive
//! under a fork bomb or a memory bomb. wolos-iso/tests/test/limits proves them in a guest.
use std::path::PathBuf;
fn read(p: &str) -> String {
    let f = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p);
    std::fs::read_to_string(&f).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn has(file: &str, lines: &[&str]) {
    let s = read(file);
    for l in lines {
        assert!(s.lines().any(|x| x.trim() == *l), "{file} lacks `{l}`");
    }
}

const SYS: &str = "files/usr/lib/systemd/system";

#[test]
fn the_session_is_protected_from_its_apps() {
    has(
        &format!("{SYS}/session-.scope.d/70-wolos-limits.conf"),
        &[
            "[Scope]",
            "CPUWeight=1000",
            "MemoryMin=256M",
            "TasksMax=4096",
        ],
    );
    // memory.min only reaches a scope through its parents.
    has(
        &format!("{SYS}/user-.slice.d/70-wolos-limits.conf"),
        &["[Slice]", "MemoryMin=256M", "TasksMax=16384"],
    );
    has(
        &format!("{SYS}/user.slice.d/70-wolos-limits.conf"),
        &["[Slice]", "MemoryMin=256M"],
    );
    has(
        &format!("{SYS}/system.slice.d/70-wolos-limits.conf"),
        &["[Slice]", "MemoryMin=128M"],
    );
}

#[test]
fn background_services_of_the_user_are_capped_together() {
    has(
        &format!("{SYS}/user@.service.d/70-wolos-limits.conf"),
        &[
            "[Service]",
            "TasksMax=8192",
            "MemoryMax=80%",
            "MemorySwapMax=25%",
        ],
    );
}

#[test]
fn files_are_limited_by_memory_not_by_a_shared_count() {
    has(
        "files/usr/lib/sysctl.d/70-wolos-performance.conf",
        &["fs.file-max = 9223372036854775807"],
    );
}

#[test]
fn tmpfs_cannot_take_most_of_ram() {
    has(
        &format!("{SYS}/tmp.mount.d/70-wolos-limits.conf"),
        &[
            "[Mount]",
            "Options=mode=1777,strictatime,nosuid,nodev,size=25%%,nr_inodes=1m",
        ],
    );
    has(
        &format!("{SYS}/wolos-shm-limit.service"),
        &["ExecStart=/usr/bin/mount -o remount,size=25%% /dev/shm"],
    );
    has(
        "files/usr/lib/systemd/system-preset/50-wolos.preset",
        &["enable wolos-shm-limit.service"],
    );
}
