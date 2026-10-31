//! linux-wolos (ADR 0016): the WolOS kernel fork at a pinned commit + Arch's config at the snapshot +
//! `kernel/conf/wolos.config` + extra patches from `$WOLOS_OVERLAY/kernel/linux/patches` when present.
//! The kernel builds with makepkg; these tests pin what the recipe and its prepare step must do.
use std::{fs, path::PathBuf, process::Command};

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}
fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}
fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-kernel-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// Sealed slots need fs-verity (btrfs supports it when the kernel does) and an overlay for /etc;
/// encrypted disks need dm-crypt and the TPM drivers.
#[test]
fn fragment_has_verity_overlay_and_disk_encryption() {
    let c = read("kernel/conf/wolos.config");
    for on in [
        "CONFIG_FS_VERITY=y",
        "CONFIG_OVERLAY_FS=y",
        "CONFIG_DM_CRYPT=y",
        "CONFIG_TCG_TPM=y",
        "CONFIG_TCG_CRB=y",
    ] {
        assert!(c.lines().any(|l| l == on), "wolos.config lacks {on}");
    }
    assert!(c.lines().any(|l| l == "CONFIG_BTRFS_FS=y"));
    assert!(!c.contains("# CONFIG_FS_VERITY is not set"));
}

#[test]
fn fragment_sets_the_desktop_options_each_with_a_reason() {
    let c = read("kernel/conf/wolos.config");
    for want in [
        "CONFIG_PREEMPT=y",
        "CONFIG_HZ_1000=y",
        "CONFIG_HZ=1000",
        "CONFIG_SCHED_CLASS_EXT=y",
        "CONFIG_BPF_SYSCALL=y",
        "CONFIG_DEBUG_INFO_BTF=y",
        "CONFIG_BTRFS_FS=y",
        "CONFIG_LOCALVERSION=\"-wolos\"",
    ] {
        assert!(c.lines().any(|l| l == want), "wolos.config lacks {want}");
    }
    // Every option has a reason: a comment line above it (or above the group it belongs to).
    let mut prev = "";
    for l in c.lines() {
        let option = l.starts_with("CONFIG_") || l.ends_with(" is not set");
        if option {
            assert!(
                prev.starts_with('#') || prev.starts_with("CONFIG_"),
                "{l}: no reason above it"
            );
        } else {
            assert!(l.is_empty() || l.starts_with('#'), "not an option: {l}");
        }
        prev = l;
    }
}

#[test]
fn recipe_builds_the_fork_at_the_recorded_commit() {
    let r = read("kernel/linux-wolos/ref");
    let commit = r.trim();
    assert!(
        commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit()),
        "ref is a full commit id: {commit:?}"
    );
    let p = read("kernel/linux-wolos/PKGBUILD");
    for want in [
        "pkgbase=linux-wolos",
        "https://github.com/wonderlandos/linux/archive/$_commit.tar.gz",
        "$(<\"$startdir/ref\")",
        "wolos.config",
        "config.x86_64",
        "prepare.sh",
        "WOLOS_OVERLAY",
    ] {
        assert!(p.contains(want), "PKGBUILD lacks {want}");
    }
    // The WolKernel patches live on the fork's wolos branch; no default patch directory
    // (they would apply twice).
    assert!(
        !p.contains("../wolos-overlay"),
        "PKGBUILD defaults to a patch dir"
    );
    let h = read("kernel/linux-wolos-api-headers/PKGBUILD");
    assert!(
        h.contains(&format!("_ref={commit}")),
        "api headers pin another commit"
    );
    let st = Command::new("bash")
        .arg("-n")
        .arg(repo("kernel/linux-wolos/PKGBUILD"))
        .status()
        .unwrap();
    assert!(st.success());
}

#[test]
fn a_new_fork_commit_ships_under_a_new_pkgrel() {
    // The package mirror keys files by name: the same version with new contents would be
    // served stale next to a fresh signature. 1:1.0+7.3-1 was fork 3f88c73.
    let commit = read("kernel/linux-wolos/ref").trim().to_string();
    for p in [
        "kernel/linux-wolos/PKGBUILD",
        "kernel/linux-wolos-api-headers/PKGBUILD",
    ] {
        let rel: u32 = read(p)
            .lines()
            .find_map(|l| l.strip_prefix("pkgrel="))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or_else(|| panic!("{p} has no numeric pkgrel"));
        if commit != "3f88c73ec9e6a0c8fe14a3c9a124a23630f59db9" {
            assert!(
                rel >= 2,
                "{p}: fork moved past the -1 build but pkgrel is {rel}"
            );
        }
    }
    // f60fabe9: lazy audit pairs the syscall entry and exit hooks (no auditsc warning).
    assert_ne!(commit, "3f88c73ec9e6a0c8fe14a3c9a124a23630f59db9");
}

#[test]
fn freebsd_kernel_parts_are_gone() {
    for p in [
        "kernel/conf/WolKernel",
        "kernel/wolos_qos",
        "kernel/drm",
        "kernel/fetch-src.sh",
        "kernel/check.sh",
        "kernel/build.sh",
    ] {
        assert!(!repo(p).exists(), "{p} still present");
    }
}

fn fake_tree(d: &std::path::Path) -> (PathBuf, PathBuf, PathBuf) {
    let src = d.join("src");
    fs::create_dir_all(src.join("scripts/kconfig")).unwrap();
    fs::write(src.join("hello.c"), "one\n").unwrap();
    let log = d.join("log");
    // merge_config.sh -m .config FRAGMENT: append, like the real one without -r.
    let merge = format!(
        "#!/bin/sh\necho \"merge $*\" >> {}\ncat \"$3\" >> .config\n",
        log.display()
    );
    fs::write(src.join("scripts/kconfig/merge_config.sh"), merge).unwrap();
    let bin = d.join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(
        bin.join("make"),
        format!("#!/bin/sh\necho \"make $*\" >> {}\n", log.display()),
    )
    .unwrap();
    for f in [
        bin.join("make"),
        src.join("scripts/kconfig/merge_config.sh"),
    ] {
        Command::new("chmod").arg("+x").arg(f).status().unwrap();
    }
    fs::write(d.join("base.config"), "CONFIG_HZ_300=y\n").unwrap();
    (src, bin, log)
}

fn prepare(
    d: &std::path::Path,
    src: &std::path::Path,
    bin: &std::path::Path,
    ov: &str,
) -> std::process::Output {
    Command::new("sh")
        .arg(repo("kernel/linux-wolos/prepare.sh"))
        .arg(src)
        .arg(d.join("base.config"))
        .arg(repo("kernel/conf/wolos.config"))
        .env("WOLOS_OVERLAY", d.join(ov))
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .output()
        .unwrap()
}

/// prepare.sh SRC BASECONFIG FRAGMENT: patches in name order (`patch -Np1`), a patch that does not
/// apply stops with exit 3, then the fragment is merged over the Arch config and `make olddefconfig`.
#[test]
fn prepare_applies_extra_patches_in_order_then_merges_the_fragment() {
    let d = scratch("prepare");
    let (src, bin, log) = fake_tree(&d);
    let ov = d.join("overlay/kernel/linux/patches");
    fs::create_dir_all(&ov).unwrap();
    fs::write(
        ov.join("0002-b.patch"),
        "--- a/hello.c\n+++ b/hello.c\n@@ -1,2 +1,2 @@\n one\n-two\n+three\n",
    )
    .unwrap();
    fs::write(
        ov.join("0001-a.patch"),
        "--- a/hello.c\n+++ b/hello.c\n@@ -1 +1,2 @@\n one\n+two\n",
    )
    .unwrap();
    // The fork carries its own copy of the fragment: merged first, ours on top.
    fs::create_dir_all(src.join("kernel/configs")).unwrap();
    fs::write(src.join("kernel/configs/wolos.config"), "CONFIG_FORK=y\n").unwrap();
    let o = prepare(&d, &src, &bin, "overlay");
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success(),
        "{out}{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(
        fs::read_to_string(src.join("hello.c")).unwrap(),
        "one\nthree\n"
    );
    assert!(out.contains("extra patches: 2"), "{out}");
    let l = fs::read_to_string(&log).unwrap();
    assert!(l.contains("merge -m .config"), "{l}");
    assert!(l.trim_end().ends_with("make olddefconfig"), "{l}");
    let cfg = fs::read_to_string(src.join(".config")).unwrap();
    assert!(cfg.starts_with("CONFIG_HZ_300=y") && cfg.contains("CONFIG_HZ_1000=y"));
    let fork = cfg.find("CONFIG_FORK=y").expect("fork fragment merged");
    assert!(
        fork < cfg.find("CONFIG_HZ_1000=y").unwrap(),
        "ours goes last"
    );
    // A patch the fork already carries (it moved into the tree) is skipped, not a failure.
    let carried = d.join("carried/kernel/linux/patches");
    fs::create_dir_all(&carried).unwrap();
    fs::write(
        carried.join("0001-c.patch"),
        "--- a/hello.c\n+++ b/hello.c\n@@ -1,2 +1,2 @@\n one\n-two\n+three\n",
    )
    .unwrap();
    let o = prepare(&d, &src, &bin, "carried");
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success(),
        "{out}{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(out.contains("already in tree: 0001-c.patch"), "{out}");
    assert!(out.contains("extra patches: 0"), "{out}");
    assert_eq!(
        fs::read_to_string(src.join("hello.c")).unwrap(),
        "one\nthree\n"
    );
    // A patch that no longer applies stops the build.
    fs::write(src.join("hello.c"), "other\n").unwrap();
    assert_eq!(prepare(&d, &src, &bin, "overlay").status.code(), Some(3));
    // No overlay: the fork as is, zero extra patches.
    fs::write(src.join("hello.c"), "one\n").unwrap();
    let o = prepare(&d, &src, &bin, "none");
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stdout).contains("extra patches: 0"));
}

/// kernel/VERSION is the one place the product version lives: "WolKernel 1.0 (7.3)" =
/// WolKernel release + the Linux base in parentheses. PKGBUILDs, uname -v and the README follow it.
#[test]
fn version_has_one_source_and_shows_the_linux_base_in_parentheses() {
    let v = read("kernel/VERSION");
    let get = |k: &str| {
        v.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .unwrap_or_else(|| panic!("VERSION lacks {k}="))
            .to_string()
    };
    let (wk, linux) = (get("wolkernel"), get("linux"));
    assert_eq!((wk.as_str(), linux.as_str()), ("1.0", "7.3"));
    let shown = format!("WolKernel {wk} ({linux})");
    assert!(
        read("kernel/README.md").contains(&shown),
        "README lacks {shown}"
    );
    for pkg in [
        "kernel/linux-wolos/PKGBUILD",
        "kernel/linux-wolos-api-headers/PKGBUILD",
    ] {
        let p = read(pkg);
        assert!(p.contains("VERSION"), "{pkg} does not read kernel/VERSION");
        assert!(!p.contains("pkgver=7."), "{pkg} hard-codes the version");
        let dir = repo(pkg).parent().unwrap().to_path_buf();
        let out = Command::new("bash")
            .arg("-c")
            .arg("startdir=$PWD; source ./PKGBUILD >/dev/null; echo \"$pkgver|${KBUILD_BUILD_VERSION:-}\"")
            .current_dir(&dir)
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&out.stdout);
        let (pv, uv) = out.trim().split_once('|').unwrap();
        assert_eq!(pv, format!("{wk}+{linux}"), "{pkg} pkgver");
        if pkg.ends_with("linux-wolos/PKGBUILD") {
            // kbuild pastes KBUILD_BUILD_VERSION unquoted into a shell line (the bzImage "is
            // ready" message): parentheses or quotes there stop the build.
            let plain = format!("WolKernel {wk} Linux {linux}");
            assert!(uv.contains(&plain), "uname -v lacks {plain}: {uv:?}");
            assert!(
                !uv.contains(['(', ')', '\'', '"']),
                "uname -v breaks kbuild: {uv:?}"
            );
        }
    }
}

fn option_set(c: &str, opt: &str) -> bool {
    c.lines().any(|l| l == opt)
}

/// Desktop tuning and debloat: what the fragment must set, and what it must never switch off
/// (Steam/Proton, Flatpak, Docker, KVM, DKMS, laptops).
#[test]
fn fragment_tunes_for_desktops_and_keeps_compatibility() {
    let c = read("kernel/conf/wolos.config");
    for want in [
        "CONFIG_NTSYNC=m",
        "CONFIG_TRANSPARENT_HUGEPAGE_MADVISE=y",
        "CONFIG_LRU_GEN_ENABLED=y",
        "CONFIG_TCP_CONG_BBR=y",
        "CONFIG_DEFAULT_BBR=y",
        "CONFIG_LEGACY_VSYSCALL_NONE=y",
        "# CONFIG_PCMCIA is not set",
        "# CONFIG_BLK_DEV_FD is not set",
        "# CONFIG_PARPORT is not set",
        "# CONFIG_FB_VESA is not set",
        "CONFIG_DRM_SIMPLEDRM=y",
        "CONFIG_ZRAM=m",
        "CONFIG_NR_CPUS=512",
        // Fork code: lazy audit syscall work, int $0x80 only for 32-bit tasks, retbleed stuffing
        // on Intel without eIBRS.
        "CONFIG_WOLOS_AUDIT_LAZY=y",
        "CONFIG_WOLOS_NO_INT80_64=y",
        "CONFIG_WOLOS_RETBLEED_STUFF=y",
        // Extra series: CPU floor right after an input event.
        "CONFIG_WOLOS_INPUT_BOOST=y",
    ] {
        assert!(option_set(&c, want), "wolos.config lacks {want}");
    }
    for keep in [
        "IA32_EMULATION",
        "MODIFY_LDT_SYSCALL",
        "BINFMT_MISC",
        "OVERLAY_FS",
        "USER_NS",
        "KVM",
        "KVM_INTEL",
        "KVM_AMD",
        "MODULES",
        "MODVERSIONS",
        "VETH",
        "BRIDGE",
        "FUTEX",
        "IO_URING",
        "SECCOMP",
        "DRM_I915",
        "DRM_AMDGPU",
        "DRM_XE",
        "IWLWIFI",
        "BT",
        "THINKPAD_ACPI",
        "ACPI_BATTERY",
    ] {
        let off = format!("# CONFIG_{keep} is not set");
        assert!(!option_set(&c, &off), "wolos.config turns off {keep}");
    }
    assert!(!option_set(&c, "CONFIG_IA32_EMULATION_DEFAULT_DISABLED=y"));
    // Every removed userspace interface is listed for `wol doctor`.
    let abi = read("kernel/abi-removed.txt");
    for name in [
        "X86_X32_ABI",
        "LEGACY_VSYSCALL",
        "WOLOS_NO_INT80_64",
        "LOOP_CHANGE_FD",
        "IPV6_ADDRFORM",
    ] {
        assert!(
            abi.lines()
                .any(|l| l.split_whitespace().nth(1) == Some(name)),
            "abi-removed.txt lacks {name}"
        );
    }
}

/// Userspace half of the tuning: sysctl values, I/O schedulers and zram defaults ship in the image.
#[test]
fn image_ships_memory_and_latency_sysctls_and_zram() {
    let s = read("image/files/usr/lib/sysctl.d/70-wolos-performance.conf");
    for kv in [
        "vm.swappiness = 100",
        "vm.page-cluster = 0",
        "vm.vfs_cache_pressure = 50",
        "vm.dirty_bytes = 268435456",
        "vm.dirty_background_bytes = 67108864",
        "kernel.nmi_watchdog = 0",
        "kernel.sched_autogroup_enabled = 0",
        "kernel.watchdog_thresh = 30",
    ] {
        assert!(s.lines().any(|l| l == kv), "sysctl lacks {kv}");
    }
    let z = read("image/files/usr/lib/systemd/zram-generator.conf");
    assert!(z.contains("[zram0]") && z.contains("compression-algorithm = zstd"));
    for f in ["image/arch/packages.x86_64", "image/packages-base.txt"] {
        assert!(
            read(f).lines().any(|l| l == "zram-generator"),
            "{f} lacks zram-generator"
        );
    }
    let t = read("image/files/usr/lib/tmpfiles.d/70-wolos-performance.conf");
    assert!(
        t.contains("/sys/kernel/mm/lru_gen/min_ttl_ms")
            && t.contains("transparent_hugepage/defrag")
    );
    let u = read("image/files/usr/lib/udev/rules.d/60-wolos-ioscheduler.rules");
    assert!(u.contains("none") && u.contains("bfq"));
    assert!(
        u.lines()
            .any(|l| l.contains("nvme") && l.contains("ATTR{queue/read_ahead_kb}=\"256\"")),
        "NVMe read_ahead_kb default"
    );
    // A slow USB stick has its own strict dirty budget: copying to it never stalls the NVMe.
    let w = read("image/files/usr/lib/udev/rules.d/70-wolos-usb-writeback.rules");
    for want in [
        "ENV{ID_BUS}==\"usb\"",
        "ATTR{removable}==\"1\"",
        "ATTR{bdi/strict_limit}=\"1\"",
        "ATTR{bdi/max_bytes}=\"33554432\"",
    ] {
        assert!(w.contains(want), "usb writeback rule lacks {want}");
    }
}

/// Lean core and Apple hardware: options the fragment turns off or on.
#[test]
fn fragment_trims_unused_code_and_supports_apple_devices() {
    let c = read("kernel/conf/wolos.config");
    for off in [
        "ACPI_DEBUG",
        "SECURITY_SMACK",
        "SECURITY_TOMOYO",
        "SECURITY_IPE",
        "SECURITY_SAFESETID",
        "SECURITY_LOADPIN",
        "FUNCTION_ERROR_INJECTION",
        "MMIOTRACE",
        "HWLAT_TRACER",
        "OSNOISE_TRACER",
        "TIMERLAT_TRACER",
        "SCHED_STACK_END_CHECK",
    ] {
        let line = format!("# CONFIG_{off} is not set");
        assert!(c.lines().any(|l| l == line), "wolos.config keeps {off}");
    }
    for on in [
        "CONFIG_USB_IPHETH=m",
        "CONFIG_HID_APPLE=m",
        "CONFIG_HID_MAGICMOUSE=m",
        "CONFIG_HID_BATTERY_STRENGTH=y",
        "CONFIG_HIDRAW=y",
        "CONFIG_FUSE_FS=m",
        "CONFIG_APFS_FS=m",
        "CONFIG_TUN=m",
    ] {
        assert!(c.lines().any(|l| l == on), "wolos.config lacks {on}");
    }
}

/// Build options for developers and CI: CPU level (generic default, x86-64-v3 flavour), Clang ThinLTO,
/// a fast build of only the loaded modules, ccache and parallel make.
#[test]
fn recipe_has_build_flavours_and_fast_developer_builds() {
    let p = read("kernel/linux-wolos/PKGBUILD");
    for want in [
        "_march=${_march:-generic}",
        "x86-64-v3",
        "_use_llvm_lto=${_use_llvm_lto:-none}",
        "LTO_CLANG_THIN",
        "LLVM=1",
        "_localmodcfg",
        "localmodconfig",
        "modprobed.db",
        "ccache",
        "MAKEFLAGS",
        "nproc",
    ] {
        assert!(p.contains(want), "PKGBUILD lacks {want}");
    }
}

/// A branch profile feeds the next build: AutoFDO and Propeller stay off without one.
#[test]
fn recipe_takes_autofdo_and_propeller_profiles() {
    let p = read("kernel/linux-wolos/PKGBUILD");
    for want in [
        "_autofdo_profile=${_autofdo_profile:-}",
        "_propeller_prefix=${_propeller_prefix:-}",
        "-e AUTOFDO_CLANG",
        "-e PROPELLER_CLANG",
        "CLANG_AUTOFDO_PROFILE=",
        "CLANG_PROPELLER_PROFILE_PREFIX=",
    ] {
        assert!(p.contains(want), "PKGBUILD lacks {want}");
    }
}

/// CI builds the kernel in an Arch container and releases packages on wolkernel-* tags.
#[test]
fn workflow_builds_and_releases_the_kernel() {
    let w = read(".github/workflows/kernel.yml");
    for want in [
        "archlinux",
        "wolkernel-*",
        "ccache",
        "actions/cache",
        "makepkg",
        "gh release create",
        "df -h",
    ] {
        assert!(w.contains(want), "kernel.yml lacks {want}");
    }
    let low = w.to_lowercase();
    for word in ["private", "proprietary", "closed", "cachyos", "overlay"] {
        assert!(!low.contains(word), "kernel.yml mentions {word}");
    }
}
