//! kernel/drm: WolOS i915kms (drm-kmod fork, ADR 0008) is pinned to the pkg's tag, patched, shipped
//! in the offline repo and locked so pkg upgrade never reverts it.
use std::path::PathBuf;

fn read(p: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

#[test]
fn fork_is_pinned_patched_shipped_and_locked() {
    let b = read("kernel/drm/build.sh");
    for want in [
        "drm_v6.12.85_3",
        "664bc6d77ddc0e103d523563a804644aca14b46d8a3c1af268ea79846139ef60",
        "\"$P\"/*.patch",
        "SYSDIR=",
    ] {
        assert!(b.contains(want), "build.sh lacks {want}");
    }
    // The patches live in this tree, next to the scripts that apply and verify them.
    assert!(b.contains("$K/patches") && b.contains("$K/verify.sh"));
    let k = read("kernel/build.sh");
    for want in ["$K/patches", "$K/verify.sh", "exit 3"] {
        assert!(k.contains(want), "kernel/build.sh lacks {want}");
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for dir in ["kernel/patches", "kernel/drm/patches"] {
        let n = std::fs::read_dir(root.join(dir))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|x| x == "patch")
            })
            .count();
        assert!(n > 0, "{dir} has no patches");
    }
    // Every patch the verifiers check for is in the tree.
    let marks = [
        (
            "kernel/patches/0001-linux-futex_waitv.patch",
            "linux_futex_waitv(",
        ),
        (
            "kernel/patches/0002-linux-sysret-fast-return.patch",
            "linux_sysret_fast",
        ),
        (
            "kernel/patches/0003-linux-futex-wait-spin.patch",
            "linux_futex_spin != 0",
        ),
        (
            "kernel/drm/patches/0001-i915-deadline-boost-foreground-waitboost.patch",
            "module_param_named(wol_waitboost",
        ),
        (
            "kernel/drm/patches/0002-i915-vbt-file-fallback.patch",
            "\"i915/vbt.bin\"",
        ),
    ];
    for (p, m) in marks {
        assert!(read(p).contains(m), "{p} lacks {m}");
        assert!(
            read(p).starts_with("# SPDX-License-Identifier: LicenseRef-Wonderland-1.0"),
            "{p}: no SPDX line"
        );
    }
    assert!(read("kernel/verify.sh").contains("sysret_fast"));
    assert!(read("kernel/drm/verify.sh").contains("wol_waitboost"));
    let img = read("image/build.sh");
    assert!(
        img.contains("kernel/drm/build.sh"),
        "image/build.sh does not build the fork"
    );
    assert!(img.contains("drm-fork.tar"));
    let inst = read("image/files/usr/local/libexec/wolos/install-pkgs");
    assert!(inst.contains("drm-fork.tar") && inst.contains("pkg lock -y drm-612-kmod"));
    assert!(read("archicture/adr/0008-i915-frame-deadline-boost.md").contains("Status: accepted"));
}
