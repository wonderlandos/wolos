// SPDX-License-Identifier: GPL-2.0-or-later
//! boot-setup with every tool faked: which chain each tier gets and what it asks of the firmware.
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

fn lib(n: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../image/files/usr/lib/wolos")
        .join(n)
}
fn fake(d: &Path, name: &str, body: &str) {
    let p = d.join("bin").join(name);
    fs::write(
        &p,
        format!(
            "#!/bin/sh\necho \"{name} $*\" >> {}\n{body}\n",
            d.join("log").display()
        ),
    )
    .unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
}

fn setup(name: &str, tier: &str, screen: &str, sbctl_fails: bool) -> (PathBuf, String) {
    let d = std::env::temp_dir().join(format!("wol-bs-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    let (t, esp) = (d.join("t"), d.join("t/efi"));
    for p in [
        d.join("bin"),
        t.join("usr/lib/systemd/boot/efi"),
        t.join("usr/share/wolos/shim"),
        t.join("etc/wolos"),
        t.join("etc/kernel"),
        t.join("boot"),
        t.join("usr/lib"),
        esp.join("EFI/systemd"),
        esp.join("EFI/BOOT"),
    ] {
        fs::create_dir_all(p).unwrap();
    }
    for (f, c) in [
        ("usr/lib/systemd/boot/efi/systemd-bootx64.efi", "loader"),
        ("usr/share/wolos/shim/shimx64.efi", "first"),
        ("usr/share/wolos/shim/mmx64.efi", "mm"),
        ("etc/wolos/slot", "a\n"),
        (
            "etc/kernel/cmdline",
            "root=x rootflags=subvol=@wolos-a ro\n",
        ),
        ("boot/vmlinuz-linux-wolos", "k"),
        ("boot/initramfs-linux-wolos.img", "i"),
        ("usr/lib/os-release", "NAME=WolOS\n"),
    ] {
        fs::write(t.join(f), c).unwrap();
    }
    fake(
        &d,
        "openssl",
        "while [ $# -gt 0 ]; do case $1 in -keyout|-out) echo x > \"$2\"; shift ;; esac; shift; done",
    );
    fake(
        &d,
        "sbsign-fake",
        "for a; do case $a in --output=*) cp \"${a##*=}\" /dev/null; echo \"signed $(cat \"$2\" 2>/dev/null)\" > \"${a#--output=}\" ;; esac; done",
    );
    fake(
        &d,
        "ukify",
        "for a; do case $a in --output=*) echo image > \"${a#--output=}\" ;; esac; done",
    );
    fake(&d, "bootctl", "");
    fake(&d, "efibootmgr", "");
    fake(&d, "mount", "");
    fake(&d, "mountpoint", "exit 1");
    fake(&d, "sbctl", if sbctl_fails { "exit 1" } else { "" });
    fake(
        &d,
        "mokutil",
        "case \"$*\" in *generate-hash*) echo hash ;; esac",
    );
    fake(
        &d,
        "wol",
        &format!(
            "case \"$*\" in \"boot detect\"*) echo {tier} ;; \"boot key-screen\"*) echo {screen} ;; \"boot words\"*) echo protected ;; \"boot password\"*) echo boot ;; \"boot note\"*) echo \"$3 $4\" ;; esac"
        ),
    );
    let o = Command::new("sh")
        .arg(lib("boot-setup"))
        .arg(&t)
        .arg(&esp)
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("WOLOS_SB_DIR", d.join("sb"))
        .env("WOLOS_SB_RUN", d.join("run"))
        .env("WOLOS_TPM_DIR", d.join("no-tpm"))
        .env("WOLOS_BOOT_NOTE", d.join("note"))
        .env("WOLOS_SBSIGN", d.join("bin/sbsign-fake"))
        .env("WOLOS_BOOT_DISK", "/dev/x")
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    (d, String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

#[test]
fn own_keys_keep_microsoft_and_need_no_key_screen() {
    let (d, out) = setup("own", "own-keys", "no", false);
    assert_eq!(out, "own-keys");
    let l = fs::read_to_string(d.join("log")).unwrap();
    assert!(l.contains("enroll-keys --microsoft"), "{l}");
    assert!(!l.contains("mokutil"), "{l}");
    assert!(!d.join("t/efi/EFI/wolos/shimx64.efi").exists());
    assert_eq!(
        fs::read_to_string(d.join("sb/cmdline")).unwrap().trim(),
        "protected"
    );
    assert_eq!(
        fs::read_to_string(d.join("t/efi/EFI/BOOT/BOOTX64.EFI"))
            .unwrap()
            .trim(),
        "signed"
    );
    // Private keys never stay on the ESP and the RAM copies are gone.
    assert!(!d.join("run/db.key").exists(), "RAM copy wiped");
}

#[test]
fn the_key_screen_route_uses_the_unaltered_first_stage_and_queues_our_key() {
    let (d, out) = setup("mok", "mok", "yes", false);
    assert_eq!(out, "mok");
    let l = fs::read_to_string(d.join("log")).unwrap();
    assert!(!l.contains("sbctl"), "{l}");
    assert!(
        l.contains("mokutil --import") && l.contains("--hash-file"),
        "{l}"
    );
    assert!(l.contains("mokutil --generate-hash=boot"), "{l}");
    let esp = d.join("t/efi");
    assert_eq!(
        fs::read_to_string(esp.join("EFI/BOOT/BOOTX64.EFI")).unwrap(),
        "first"
    );
    assert_eq!(
        fs::read_to_string(esp.join("EFI/wolos/shimx64.efi")).unwrap(),
        "first"
    );
    assert_eq!(
        fs::read_to_string(esp.join("EFI/wolos/mmx64.efi")).unwrap(),
        "mm"
    );
    assert!(
        fs::read_to_string(esp.join("EFI/wolos/grubx64.efi"))
            .unwrap()
            .starts_with("signed")
    );
    assert!(l.contains("efibootmgr --create --disk /dev/x"), "{l}");
    assert_eq!(
        fs::read_to_string(d.join("note")).unwrap().trim(),
        "mok yes"
    );
}

#[test]
fn nothing_is_asked_of_the_firmware_when_protection_is_off() {
    let (d, out) = setup("off", "off", "no", false);
    assert_eq!(out, "off");
    let l = fs::read_to_string(d.join("log")).unwrap();
    assert!(!l.contains("sbctl") && !l.contains("mokutil"), "{l}");
    assert!(d.join("sb/db.crt").exists(), "keys are ready for later");
    assert!(d.join("t/efi/EFI/wolos/wolos-a.efi").exists());
}

#[test]
fn a_firmware_that_refuses_the_keys_leaves_a_system_that_starts() {
    let (d, out) = setup("refuse", "own-keys", "no", true);
    assert_eq!(out, "off");
    assert_eq!(fs::read_to_string(d.join("sb/tier")).unwrap().trim(), "off");
    assert_eq!(fs::read_to_string(d.join("sb/cmdline")).unwrap().trim(), "");
}

#[test]
fn the_image_carries_the_tools_and_the_first_stage_stays_pinned() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let list = std::fs::read_to_string(format!("{root}/image/arch/packages.x86_64")).unwrap();
    for p in ["systemd-ukify", "sbctl", "mokutil", "wolos-shim"] {
        assert!(list.lines().any(|l| l == p), "{p}");
    }
    let repo = std::fs::read_to_string(format!("{root}/image/repo/packages")).unwrap();
    assert!(repo.lines().any(|l| l.starts_with("wolos-shim ")));
    let b = std::fs::read_to_string(format!("{root}/image/repo/wolos-shim/PKGBUILD")).unwrap();
    assert!(!b.contains("SKIP"), "every source is pinned");
    assert!(b.contains("351e131d3c4a636704e9cae7d1296baf8a05beca9922cfebbfa48933c9bf1ad3"));
    assert!(
        !b.contains("sbsign"),
        "the first stage is shipped as it was signed"
    );
}

#[test]
fn the_sealed_system_can_not_enable_units_at_first_start() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let i = std::fs::read_to_string(format!(
        "{root}/image/repo/wolos-system/wolos-system.install"
    ))
    .unwrap();
    for u in [
        "wolos-verity-check.service",
        "wolos-tpm-bind.service",
        "wolos-boot-ok.service",
    ] {
        assert!(i.contains(u), "{u} is enabled when the package installs");
    }
}
