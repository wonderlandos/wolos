// SPDX-License-Identifier: GPL-2.0-or-later
//! Kernel marks hold on mounts. In a mount namespace of its own the guard would mark copies nobody
//! else uses and stop nothing, without a word. Found by running the real unit.
#[test]
fn the_guard_unit_stays_in_the_mount_namespace_everyone_else_uses() {
    let unit = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../image/files/usr/lib/systemd/system/wolos-xguard.service"
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
