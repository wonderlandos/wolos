// SPDX-License-Identifier: GPL-2.0-or-later
//! Runtime tuning for the JavaScript engines under third-party apps (ADR 0061,
//! docs/modules/runtime-tuning.md): the image runs `wolos-mm apply` at boot, after debugfs is
//! mounted, and the knobs it writes are the table in `wol_core::jsruntime`; the measured numbers
//! behind each are in docs/bench/js-runtime-x.md.
#[path = "support/repo.rs"]
mod repo;
use repo::read;
use wol_core::jsruntime::{KSM_RUN, MM_WRITES};

const UNIT: &str = "files/usr/lib/systemd/system/wolos-mm.service";

#[test]
fn the_unit_runs_apply_after_debugfs_is_mounted() {
    let u = read(UNIT);
    for l in [
        "Type=oneshot",
        "ExecStart=/usr/bin/wolos-mm apply",
        "Wants=sys-kernel-debug.mount",
        "After=sys-kernel-debug.mount",
    ] {
        assert!(u.lines().any(|x| x == l), "{UNIT} lacks `{l}`");
    }
    let preset = read("files/usr/lib/systemd/system-preset/50-wolos.preset");
    assert!(preset.lines().any(|l| l == "enable wolos-mm.service"));
}

#[test]
fn nothing_is_left_in_tmpfiles_because_ksm_depends_on_the_accounts() {
    // The first version wrote `run=1` from a tmpfiles file; the account rule (one human account)
    // cannot be a static line, so one program writes every knob.
    assert!(!repo::repo("files/usr/lib/tmpfiles.d/71-wolos-mm.conf").exists());
    assert!(
        !repo::repo(
            "files/usr/lib/systemd/system/systemd-tmpfiles-setup.service.d/71-wolos-mm.conf"
        )
        .exists()
    );
}

#[test]
fn every_knob_is_a_kernel_interface_that_was_measured() {
    for w in MM_WRITES {
        assert!(
            w.path.starts_with("/sys/kernel/debug/fault_around_bytes")
                || w.path.starts_with("/sys/kernel/mm/ksm/"),
            "{}: only the interfaces measured",
            w.path
        );
        assert!(!w.why.is_empty());
    }
    assert!(KSM_RUN.starts_with("/sys/kernel/mm/ksm/"));
}

#[test]
fn the_existing_tuning_is_not_overwritten() {
    // 70-wolos-performance.conf owns MGLRU min_ttl and THP defrag; this change sets neither
    // (THP always, MALLOC and arena settings measured 0 MiB: docs/bench/js-runtime-x.md).
    for w in MM_WRITES {
        for p in ["lru_gen", "transparent_hugepage", "swappiness", "zswap"] {
            assert!(!w.path.contains(p), "{}", w.path);
        }
    }
}
