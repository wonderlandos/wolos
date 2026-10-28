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
        "git+https://github.com/wonderlandos/linux.git#commit=",
        "$(<\"$startdir/ref\")",
        "wolos.config",
        "config.x86_64",
        "prepare.sh",
        "WOLOS_OVERLAY",
    ] {
        assert!(p.contains(want), "PKGBUILD lacks {want}");
    }
    let st = Command::new("bash")
        .arg("-n")
        .arg(repo("kernel/linux-wolos/PKGBUILD"))
        .status()
        .unwrap();
    assert!(st.success());
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
    // A patch that no longer applies stops the build.
    fs::write(src.join("hello.c"), "other\n").unwrap();
    assert_eq!(prepare(&d, &src, &bin, "overlay").status.code(), Some(3));
    // No overlay: the fork as is, zero extra patches.
    fs::write(src.join("hello.c"), "one\n").unwrap();
    let o = prepare(&d, &src, &bin, "none");
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stdout).contains("extra patches: 0"));
}
