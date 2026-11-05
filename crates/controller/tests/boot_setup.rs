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
        .join("../../files/usr/lib/wolos")
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
    setup_on(name, tier, screen, sbctl_fails, &[])
}

/// `esp`: files already on the startup partition (another system's), path and content.
fn setup_on(
    name: &str,
    tier: &str,
    screen: &str,
    sbctl_fails: bool,
    esp_files: &[(&str, &str)],
) -> (PathBuf, String) {
    setup_with(name, tier, screen, sbctl_fails, esp_files, &[])
}

/// [`setup_on`] with more of install-run's environment (`WOLOS_ESP_SHARED`, `WOLOS_SECURE`).
fn setup_with(
    name: &str,
    tier: &str,
    screen: &str,
    sbctl_fails: bool,
    esp_files: &[(&str, &str)],
    env: &[(&str, &str)],
) -> (PathBuf, String) {
    let (d, o) = setup_out(name, tier, screen, sbctl_fails, esp_files, env, &[]);
    assert!(
        o.status.success(),
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    (d, String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

/// [`setup_with`] that hands back the run as it ended; `fakes` replace the default tools.
fn setup_out(
    name: &str,
    tier: &str,
    screen: &str,
    sbctl_fails: bool,
    esp_files: &[(&str, &str)],
    env: &[(&str, &str)],
    fakes: &[(&str, &str)],
) -> (PathBuf, std::process::Output) {
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
    for (f, c) in esp_files {
        let p = esp.join(f);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, c).unwrap();
    }
    fake(
        &d,
        "openssl",
        concat!(
            "case $1 in passwd) cat > /dev/null; echo '$6$salt$hashed'; exit 0 ;; esac; ",
            "while [ $# -gt 0 ]; do case $1 in -keyout|-out) echo x > \"$2\"; shift ;; esac; shift; done",
        ),
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
    // The real bootctl install always writes the fallback loader too.
    fake(
        &d,
        "bootctl",
        "case \"$*\" in *install*) echo sd-boot > \"${1#--esp-path=}/EFI/BOOT/BOOTX64.EFI\" ;; esac",
    );
    // A firmware with one entry already, ours: printed when asked, so the number is known.
    fake(
        &d,
        "efibootmgr",
        "case \"$*\" in \"\") echo 'BootCurrent: 0001'; echo 'Boot0001* UEFI OS'; echo 'Boot0003* WolOS' ;; esac",
    );
    // The ESP's device: partition 2 of /dev/x.
    fake(
        &d,
        "lsblk",
        "case \"$*\" in *PARTN*) echo 2 ;; *PKNAME*) echo /dev/x ;; esac",
    );
    fake(&d, "mount", "");
    fake(&d, "mountpoint", "exit 1");
    // Refusing firmware: enroll-keys "succeeds" but the firmware stays in Setup Mode (sbctl has
    // returned 0 on a firmware that kept nothing), so the status is what counts.
    fake(
        &d,
        "sbctl",
        if sbctl_fails {
            "case \"$*\" in *status*) printf 'Installed:\\tsbctl is installed\\nSetup Mode:\\t\\342\\234\\223 Enabled\\n' ;; esac"
        } else {
            "case \"$*\" in *status*) printf 'Setup Mode:\\tDisabled\\nSecure Boot:\\tEnabled\\n' ;; esac"
        },
    );
    fake(
        &d,
        "mokutil",
        &format!(
            "case \"$*\" in --import*) cp \"$4\" {}/hash-seen ;; esac",
            d.display()
        ),
    );
    fake(
        &d,
        "wol",
        &format!(
            "case \"$*\" in \"boot detect\"*) echo {tier} ;; \"boot key-screen\"*) echo {screen} ;; \"boot words\"*) echo protected ;; \"boot password\"*) echo s3cr3t-MOK-pw ;; \"boot note\"*) echo \"$3 $4\" ;; esac"
        ),
    );
    for (n, body) in fakes {
        fake(&d, n, body);
    }
    let o = Command::new("sh")
        .arg(lib("boot-setup"))
        .arg(&t)
        .arg(&esp)
        .envs(env.iter().copied())
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("WOLOS_SB_DIR", d.join("sb"))
        .env("WOLOS_SB_RUN", d.join("run"))
        .env("WOLOS_TPM_DIR", d.join("no-tpm"))
        .env("WOLOS_BOOT_NOTE", d.join("note"))
        .env("WOLOS_SBSIGN", d.join("bin/sbsign-fake"))
        .env("WOLOS_BOOT_DISK", "/dev/x")
        .env("WOLOS_ESP_DEV", "/dev/x2")
        .output()
        .unwrap();
    (d, o)
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
    assert!(
        l.contains("bootctl --esp-path=") && !l.contains("--no-variables"),
        "own keys: the firmware entry for our loader is wanted: {l}"
    );
    assert!(!d.join("sb/key-screen").exists(), "no key screen to come");
    assert!(
        l.contains("sbctl --config") && l.contains(" status"),
        "enrollment is checked: {l}"
    );
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
    no_password_in_argv(&d, &l);
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
    // The firmware entry points at shim, on the partition the ESP really is (not always 1), is the
    // next thing started, and our loader gets no entry of its own: with Secure Boot on it would
    // be refused and show the firmware's error screen before anything else.
    assert!(
        l.contains("efibootmgr --create --disk /dev/x --part 2 --label WolOS"),
        "{l}"
    );
    assert!(l.contains("lsblk -no PARTN /dev/x2"), "{l}");
    assert!(l.contains("efibootmgr --quiet --bootnext 0003"), "{l}");
    assert!(
        !l.contains("--bootnum 0001"),
        "another system's entry is never touched: {l}"
    );
    assert!(l.contains("install --no-variables"), "{l}");
    assert_eq!(
        fs::read_to_string(d.join("note")).unwrap().trim(),
        "mok yes"
    );
    assert!(
        d.join("sb/key-screen").exists(),
        "the first-boot unit knows the screen is still to come"
    );
    // QA-DOC-29: the key is also at the top of the startup partition under the stick's name,
    // where 'Enroll key from disk' finds it when the first blue screen was missed.
    assert!(esp.join("ENROL_THIS_KEY_FOR_WOLOS.cer").exists());
    assert!(esp.join("EFI/wolos/wolos-key.cer").exists());
}

/// Secure Boot on, only Windows trusted (audit2 B10): the signed first stage and the queued key
/// are laid out anyway, so allowing third-party certificates in the firmware is the only step
/// left; no "Linux Boot Manager" entry the firmware would refuse.
#[test]
fn the_firmware_tier_lays_out_the_key_screen_chain_for_after_the_setting() {
    let (d, out) = setup("firmware", "firmware", "yes", false);
    assert_eq!(out, "firmware");
    let l = fs::read_to_string(d.join("log")).unwrap();
    let esp = d.join("t/efi");
    assert_eq!(
        fs::read_to_string(esp.join("EFI/wolos/shimx64.efi")).unwrap(),
        "first"
    );
    assert_eq!(
        fs::read_to_string(esp.join("EFI/BOOT/BOOTX64.EFI")).unwrap(),
        "first"
    );
    assert!(esp.join("EFI/wolos/mmx64.efi").exists());
    assert!(l.contains("mokutil --import"), "{l}");
    assert!(l.contains("install --no-variables"), "{l}");
    assert!(
        l.contains("efibootmgr --create --disk /dev/x --part 2 --label WolOS"),
        "{l}"
    );
    assert!(!l.contains("sbctl"), "{l}");
    assert!(d.join("sb/key-screen").exists());
    assert_eq!(
        fs::read_to_string(d.join("note")).unwrap().trim(),
        "firmware yes"
    );
}

/// A Firmware-tier install whose key went in on the key screen (the firmware setting was changed)
/// moves to the mok tier on its own: the words reach the boot images, then the tier file.
#[test]
fn an_enrolled_key_moves_a_firmware_install_to_the_key_screen_tier() {
    let (d, _) = setup("firmware-up", "firmware", "yes", false);
    assert_eq!(
        fs::read_to_string(d.join("sb/tier")).unwrap().trim(),
        "firmware"
    );
    fs::write(d.join("sb/cmdline"), "").unwrap();
    fs::write(d.join("log"), "").unwrap();
    fake(
        &d,
        "mokutil",
        "case \"$*\" in --test-key*) echo 'db.der is already enrolled' ;; esac",
    );
    let run = |d: &Path| {
        Command::new("sh")
            .arg(lib("boot-enroll"))
            .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
            .env("WOLOS_SB_DIR", d.join("sb"))
            .env("WOLOS_SB_RUN", d.join("run"))
            .env("WOLOS_TPM_DIR", d.join("no-tpm"))
            .env("WOLOS_RUN_DIR", d.join("runstate"))
            .env("WOLOS_SBSIGN", d.join("bin/sbsign-fake"))
            .env("WOLOS_ROOT", d.join("t"))
            .env("WOLOS_ESP", d.join("t/efi"))
            .output()
            .unwrap()
    };
    let o = run(&d);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(String::from_utf8_lossy(&o.stdout), "enrolled\n");
    assert_eq!(fs::read_to_string(d.join("sb/tier")).unwrap().trim(), "mok");
    assert_eq!(
        fs::read_to_string(d.join("sb/cmdline")).unwrap().trim(),
        "protected"
    );
    let l = fs::read_to_string(d.join("log")).unwrap();
    assert!(
        l.lines()
            .any(|x| x.starts_with("ukify ") && x.contains("protected")),
        "the images carry the mok words: {l}"
    );
    assert!(d.join("sb/enrolled").exists());
    // A rebuild that fails leaves the firmware tier and its words as they were.
    fs::write(d.join("sb/tier"), "firmware\n").unwrap();
    fs::write(d.join("sb/cmdline"), "\n").unwrap();
    fake(&d, "ukify", "exit 1");
    let o = run(&d);
    assert!(!o.status.success());
    assert_eq!(
        fs::read_to_string(d.join("sb/tier")).unwrap().trim(),
        "firmware"
    );
    assert_eq!(fs::read_to_string(d.join("sb/cmdline")).unwrap().trim(), "");
    // Other tiers are left alone.
    fs::write(d.join("sb/tier"), "own-keys\n").unwrap();
    let o = run(&d);
    assert!(o.status.success());
    assert_eq!(
        fs::read_to_string(d.join("sb/tier")).unwrap().trim(),
        "own-keys"
    );
}

/// Install Alongside reusing another Linux's startup partition (audit2 B16): its loader.conf
/// (default, timeout) and its fallback loader stay its own.
#[test]
fn a_shared_startup_partition_keeps_the_other_systems_menu_and_fallback() {
    let theirs = "default fedora-*.conf\ntimeout 5\n";
    let (d, _) = setup_on(
        "shared",
        "off",
        "no",
        false,
        &[
            ("loader/loader.conf", theirs),
            ("EFI/BOOT/BOOTX64.EFI", "their shim"),
            ("EFI/BOOT/grubx64.efi", "their grub"),
        ],
    );
    let esp = d.join("t/efi");
    assert_eq!(
        fs::read_to_string(esp.join("loader/loader.conf")).unwrap(),
        theirs
    );
    assert_eq!(
        fs::read_to_string(esp.join("EFI/BOOT/BOOTX64.EFI")).unwrap(),
        "their shim"
    );
    assert!(
        d.join("sb/esp-shared").exists(),
        "--enroll later keeps the answer"
    );
    assert!(
        esp.join("EFI/wolos/wolos-a.efi").exists(),
        "WolOS still starts"
    );
    // Without a timeout of its own the menu gets a moment, so both systems can be picked.
    let (d, _) = setup_on(
        "shared-mok",
        "mok",
        "yes",
        false,
        &[
            ("loader/loader.conf", "default fedora-*.conf\n"),
            ("EFI/BOOT/BOOTX64.EFI", "their shim"),
            ("EFI/BOOT/grubx64.efi", "their grub"),
        ],
    );
    let esp = d.join("t/efi");
    assert_eq!(
        fs::read_to_string(esp.join("loader/loader.conf")).unwrap(),
        "default fedora-*.conf\ntimeout 3\n"
    );
    assert_eq!(
        fs::read_to_string(esp.join("EFI/BOOT/BOOTX64.EFI")).unwrap(),
        "their shim"
    );
    assert_eq!(
        fs::read_to_string(esp.join("EFI/BOOT/grubx64.efi")).unwrap(),
        "their grub",
        "their shim loads its own second stage"
    );
    assert_eq!(
        fs::read_to_string(esp.join("EFI/wolos/shimx64.efi")).unwrap(),
        "first",
        "ours lives in EFI/wolos"
    );
}

/// audit3 R2B-4: the plan says whether the startup partition is shared, not a file on it. A
/// fallback loader left by a system the plan removed is ours to replace; a GRUB-only partition of
/// a system that stays is someone else's even without one.
#[test]
fn the_plan_decides_whether_the_startup_partition_is_shared() {
    let (d, _) = setup_with(
        "shared-no",
        "off",
        "no",
        false,
        &[("EFI/BOOT/BOOTX64.EFI", "orphaned bootmgfw")],
        &[("WOLOS_ESP_SHARED", "no")],
    );
    let esp = d.join("t/efi");
    assert!(!d.join("sb/esp-shared").exists());
    assert!(
        fs::read_to_string(esp.join("EFI/BOOT/BOOTX64.EFI"))
            .unwrap()
            .starts_with("signed"),
        "ours: the firmware's fallback path starts WolOS"
    );
    let theirs = "default ubuntu\n";
    let (d, _) = setup_with(
        "shared-yes",
        "off",
        "no",
        false,
        &[
            ("loader/loader.conf", theirs),
            ("EFI/ubuntu/shimx64.efi", "their shim"),
            ("EFI/ubuntu/grubx64.efi", "their grub"),
            ("EFI/debian/grubx64.efi", "grub only"),
            ("EFI/Microsoft/Boot/bootmgfw.efi", "windows"),
        ],
        &[("WOLOS_ESP_SHARED", "yes")],
    );
    let esp = d.join("t/efi");
    assert!(d.join("sb/esp-shared").exists());
    assert_eq!(
        fs::read_to_string(esp.join("loader/loader.conf")).unwrap(),
        "default ubuntu\ntimeout 3\n"
    );
    // Nothing of WolOS's first stage goes to EFI/BOOT on a shared partition.
    assert!(!esp.join("EFI/BOOT/shimx64.efi").exists());
    // audit3 R2B-11: the other Linux is in the menu, through its own first stage.
    assert_eq!(
        fs::read_to_string(esp.join("loader/entries/ubuntu.conf")).unwrap(),
        "title Ubuntu\nefi /EFI/ubuntu/shimx64.efi\n"
    );
    assert_eq!(
        fs::read_to_string(esp.join("loader/entries/debian.conf")).unwrap(),
        "title Debian\nefi /EFI/debian/grubx64.efi\n"
    );
    for not in ["Microsoft", "BOOT", "systemd", "wolos"] {
        assert!(
            !esp.join(format!("loader/entries/{not}.conf")).exists(),
            "{not}"
        );
    }
    // An entry already there is the other system's or the person's: kept.
    let (d, _) = setup_with(
        "shared-kept-entry",
        "off",
        "no",
        false,
        &[
            ("EFI/fedora/shimx64.efi", "their shim"),
            ("loader/entries/fedora.conf", "title Mine\n"),
        ],
        &[("WOLOS_ESP_SHARED", "yes")],
    );
    assert_eq!(
        fs::read_to_string(d.join("t/efi/loader/entries/fedora.conf")).unwrap(),
        "title Mine\n"
    );
    // Not shared: no entries for what is on it.
    let (d, _) = setup_with(
        "not-shared-entries",
        "off",
        "no",
        false,
        &[("EFI/ubuntu/shimx64.efi", "left over")],
        &[("WOLOS_ESP_SHARED", "no")],
    );
    assert!(!d.join("t/efi/loader/entries/ubuntu.conf").exists());
}

/// audit3 R2B-8: protection turned off at install stays off. A Firmware-tier install whose key
/// gets enrolled later is not moved to the key-screen tier (lockdown) behind the person's back.
#[test]
fn a_firmware_install_without_protection_is_not_promoted() {
    let (d, _) = setup_with(
        "firmware-declined",
        "firmware",
        "yes",
        false,
        &[],
        &[("WOLOS_SECURE", "no")],
    );
    assert_eq!(fs::read_to_string(d.join("sb/want")).unwrap(), "no\n");
    fake(
        &d,
        "mokutil",
        "case \"$*\" in --test-key*) echo 'db.der is already enrolled' ;; esac",
    );
    let o = Command::new("sh")
        .arg(lib("boot-enroll"))
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("WOLOS_SB_DIR", d.join("sb"))
        .env("WOLOS_SB_RUN", d.join("run"))
        .env("WOLOS_RUN_DIR", d.join("runstate"))
        .env("WOLOS_SBSIGN", d.join("bin/sbsign-fake"))
        .env("WOLOS_ROOT", d.join("t"))
        .env("WOLOS_ESP", d.join("t/efi"))
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        fs::read_to_string(d.join("sb/tier")).unwrap().trim(),
        "firmware"
    );
    assert!(d.join("sb/enrolled").exists());
    let (d, _) = setup("firmware-wanted", "firmware", "yes", false);
    assert_eq!(fs::read_to_string(d.join("sb/want")).unwrap(), "yes\n");
}

/// A startup partition of WolOS's own (new, empty) gets the fallback and the loader settings.
#[test]
fn a_new_startup_partition_gets_our_fallback_and_menu() {
    let (d, _) = setup("fresh", "off", "no", false);
    let esp = d.join("t/efi");
    assert!(!d.join("sb/esp-shared").exists());
    assert!(
        fs::read_to_string(esp.join("EFI/BOOT/BOOTX64.EFI"))
            .unwrap()
            .starts_with("signed"),
        "the fallback is our signed loader"
    );
    assert!(
        fs::read_to_string(esp.join("loader/loader.conf"))
            .unwrap()
            .contains("editor no")
    );
}

#[test]
fn nothing_is_asked_of_the_firmware_when_protection_is_off() {
    let (d, out) = setup("off", "off", "no", false);
    assert_eq!(out, "off");
    let l = fs::read_to_string(d.join("log")).unwrap();
    assert!(!l.contains("sbctl") && !l.contains("mokutil"), "{l}");
    assert!(!d.join("sb/key-screen").exists());
    assert!(d.join("sb/db.crt").exists(), "keys are ready for later");
    assert!(d.join("t/efi/EFI/wolos/wolos-a.efi").exists());
}

#[test]
fn a_firmware_that_refuses_the_keys_leaves_a_system_that_starts() {
    let (d, out) = setup("refuse", "own-keys", "no", true);
    assert_eq!(out, "off");
    assert_eq!(fs::read_to_string(d.join("sb/tier")).unwrap().trim(), "off");
    assert_eq!(fs::read_to_string(d.join("sb/cmdline")).unwrap().trim(), "");
    // The boot images were built with the chain's words before the firmware said no: they are
    // built again without them, else lockdown=integrity would refuse the machine's own modules.
    let l = fs::read_to_string(d.join("log")).unwrap();
    let ukify: Vec<&str> = l.lines().filter(|x| x.starts_with("ukify ")).collect();
    assert_eq!(ukify.len(), 2, "{l}");
    assert!(
        ukify[0].contains("protected") && !ukify[1].contains("protected"),
        "{l}"
    );
    assert!(
        !d.join("run/db.key").exists(),
        "RAM copy wiped after the second build too"
    );
}

#[test]
fn a_module_built_here_is_signed_with_this_computers_key() {
    let d = std::env::temp_dir().join(format!("wol-dkms-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("bin")).unwrap();
    fs::create_dir_all(d.join("sb")).unwrap();
    fs::write(d.join("sb/db.der"), "cert").unwrap();
    // The whole set (without a TPM): `boot-keys open` refuses one with a key missing (C9-3).
    for k in ["db.key", "PK.key", "KEK.key", "tpm2-pcr-private.pem"] {
        fs::write(d.join("sb").join(k), "key").unwrap();
    }
    fake(&d, "sign-file", "");
    let out = Command::new("sh")
        .arg(lib("dkms-sign"))
        .args(["sha512", "/ignored.key", "/ignored.der", "/m.ko"])
        .env("WOLOS_SB_DIR", d.join("sb"))
        .env("WOLOS_SB_RUN", d.join("run"))
        .env("WOLOS_TPM_DIR", d.join("no-tpm"))
        .env("WOLOS_SIGN_FILE", d.join("bin/sign-file"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let log = fs::read_to_string(d.join("log")).unwrap();
    assert!(
        log.contains(&format!(
            "sha256 {}/db.key {}/db.der /m.ko",
            d.join("run").display(),
            d.join("sb").display()
        )),
        "{log}"
    );
    assert!(
        !d.join("run/db.key").exists(),
        "the key leaves RAM after signing"
    );
}

/// QA-A9-IMAGE-12: the MOK password that protects enrolling this computer's key is never a command
/// line argument of any program (visible in /proc/*/cmdline to every local process): `mokutil
/// --generate-hash=PASSWORD` was. The hash is made from stdin and handed over as a file, and what
/// mokutil got is a crypt(3) SHA-512 hash, not the password.
fn no_password_in_argv(d: &std::path::Path, log: &str) {
    assert!(
        !log.contains("s3cr3t-MOK-pw") && !log.contains("--generate-hash"),
        "the MOK password is on a command line: {log}"
    );
    let h = fs::read_to_string(d.join("hash-seen")).expect("mokutil --import got a hash file");
    assert!(
        h.trim().starts_with("$6$") && !h.contains("s3cr3t-MOK-pw"),
        "hash file: {h}"
    );
}

fn enroll(name: &str, test_key: &str) -> (PathBuf, std::process::Output) {
    let d = std::env::temp_dir().join(format!("wol-enroll-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("bin")).unwrap();
    fs::create_dir_all(d.join("sb")).unwrap();
    fs::write(d.join("sb/db.der"), "cert").unwrap();
    fake(
        &d,
        "mokutil",
        &format!(
            "case \"$*\" in --test-key*) echo '{test_key}' ;; --import*) cp \"$4\" {}/hash-seen ;; esac",
            d.display()
        ),
    );
    fake(
        &d,
        "wol",
        "case \"$*\" in 'boot password') echo s3cr3t-MOK-pw ;; esac",
    );
    let o = Command::new("sh")
        .arg(lib("boot-enroll"))
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("WOLOS_SB_DIR", d.join("sb"))
        .env("WOLOS_RUN_DIR", d.join("run"))
        .output()
        .unwrap();
    (d, o)
}

#[test]
fn enrollment_queues_itself_and_the_blue_screen_waits_for_the_person() {
    let (d, o) = enroll("q", "not enrolled");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(String::from_utf8_lossy(&o.stdout), "queued\n");
    let l = fs::read_to_string(d.join("log")).unwrap();
    no_password_in_argv(&d, &l);
    assert!(
        l.contains(&format!(
            "--import {}/db.der --hash-file",
            d.join("sb").display()
        )),
        "{l}"
    );
    assert!(
        l.contains("mokutil --timeout -1"),
        "no ten second race: {l}"
    );
    assert!(
        d.join("run/mok-pending").exists(),
        "the Setup assistant's first-boot card knows the screen is still to come"
    );
}

#[test]
fn an_enrolled_key_is_left_alone() {
    let (d, o) = enroll("done", "db.der is already enrolled");
    assert_eq!(String::from_utf8_lossy(&o.stdout), "enrolled\n");
    let l = fs::read_to_string(d.join("log")).unwrap();
    assert!(!l.contains("--import"), "{l}");
    assert!(
        d.join("sb/enrolled").exists(),
        "the first-boot unit stops asking"
    );
    assert!(!d.join("run/mok-pending").exists());
}

/// The first-boot unit runs only where the key screen is the way in.
#[test]
fn the_enroll_unit_runs_only_when_a_key_screen_is_to_come() {
    let u = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../files/usr/lib/systemd/system/wolos-mok-enroll.service"),
    )
    .unwrap();
    for want in [
        "ConditionPathExists=/var/lib/wolos/secureboot/key-screen",
        "ConditionPathExists=!/var/lib/wolos/secureboot/enrolled",
        "ExecStart=/usr/lib/wolos/boot-enroll",
    ] {
        assert!(
            u.lines().any(|l| l == want),
            "wolos-mok-enroll lacks {want}"
        );
    }
}

/// audit3 R2B-8: `wol boot bless` and the key screen's unit (esp-sync, after a Firmware-tier key is
/// enrolled) rewrite the same boot entries; bless waits while that unit is still running.
#[test]
fn a_good_boot_blesses_only_after_the_key_screen_unit_is_done() {
    let d = std::env::temp_dir().join(format!("wol-bootok-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("bin")).unwrap();
    // Two answers "activating", then "inactive": the counter lives next to the fakes.
    fake(
        &d,
        "systemctl",
        &format!(
            "case \"$*\" in *ActiveState*) n=$(cat {c} 2>/dev/null || echo 0); echo $((n + 1)) > {c}; \
             if [ \"$n\" -lt 2 ]; then echo activating; else echo inactive; fi ;; esac",
            c = d.join("count").display()
        ),
    );
    fake(&d, "sleep", "");
    fake(&d, "wol", "");
    fake(&d, "mountpoint", "exit 1");
    // QA-SYS-3: the login window is on the screen (a fake /proc), no layers to settle.
    fs::create_dir_all(d.join("proc/100")).unwrap();
    fs::write(d.join("proc/100/comm"), "wol-loginwindow\n").unwrap();
    fs::create_dir_all(d.join("ext")).unwrap();
    fs::write(d.join("cmdline"), "").unwrap();
    let o = Command::new("sh")
        .arg(lib("boot-ok"))
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("WOLOS_PROC", d.join("proc"))
        .env("WOLOS_CMDLINE_FILE", d.join("cmdline"))
        .env("WOLOS_LAYERS", d.join("ext"))
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let l = fs::read_to_string(d.join("log")).unwrap();
    let asks = l.matches("ActiveState").count();
    assert_eq!(asks, 3, "{l}");
    let last_ask = l.rfind("ActiveState").unwrap();
    assert!(l.find("wol boot bless").unwrap() > last_ask, "{l}");
}

/// audit5 C9-2: bootctl writes the fallback loader and then fails (a full partition). On a shared
/// partition the other system's loader is put back all the same, and no copy is left behind.
#[test]
fn a_failed_boot_loader_install_puts_the_other_systems_fallback_back() {
    let tmp = std::env::temp_dir().join(format!("wol-bs-fb-fail-tmp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    let tmp_s = tmp.display().to_string();
    let (d, o) = setup_out(
        "fb-fail",
        "own-keys",
        "no",
        false,
        &[("EFI/BOOT/BOOTX64.EFI", "other system's shim")],
        &[("WOLOS_ESP_SHARED", "yes"), ("TMPDIR", tmp_s.as_str())],
        &[(
            "bootctl",
            "case \"$*\" in *install*) echo sd-boot > \"${1#--esp-path=}/EFI/BOOT/BOOTX64.EFI\"; exit 1 ;; esac",
        )],
    );
    assert!(!o.status.success(), "bootctl failed: the run fails");
    assert_eq!(
        fs::read_to_string(d.join("t/efi/EFI/BOOT/BOOTX64.EFI")).unwrap(),
        "other system's shim",
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!d.join("run/db.key").exists(), "the keys left RAM too");
    assert_eq!(
        fs::read_dir(&tmp).unwrap().count(),
        0,
        "the kept copy went once it was back"
    );
}

/// audit5 C9-3: boot-keys with a TPM whose encryption fails for one key. The run fails, keeps no
/// half set that a later run would take as made, leaves no clear key in RAM, and `open` refuses
/// a set with a key missing.
#[test]
fn a_half_made_key_set_is_never_taken_as_made() {
    let d = std::env::temp_dir().join(format!("wol-bk-half-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("bin")).unwrap();
    fs::create_dir_all(d.join("tpm")).unwrap();
    fs::write(d.join("tpm/tpm_version_major"), "2\n").unwrap();
    fake(
        &d,
        "openssl",
        "while [ $# -gt 0 ]; do case $1 in -keyout|-out) echo x > \"$2\"; shift ;; esac; shift; done",
    );
    fake(&d, "mount", "");
    fake(&d, "mountpoint", "exit 1");
    // encrypt FILE OUT / decrypt CRED OUT: the last two arguments; KEK.key fails while `fail` exists.
    fake(
        &d,
        "systemd-creds",
        &format!(
            "for a; do src=$dst; dst=$a; done; case \"$*\" in *encrypt*--name=KEK.key*) [ ! -e {} ] || exit 1 ;; esac; cp \"$src\" \"$dst\"",
            d.join("fail").display()
        ),
    );
    let run = |verb: &str| {
        Command::new("sh")
            .arg(lib("boot-keys"))
            .arg(verb)
            .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
            .env("WOLOS_SB_DIR", d.join("sb"))
            .env("WOLOS_SB_RUN", d.join("run"))
            .env("WOLOS_TPM_DIR", d.join("tpm"))
            .output()
            .unwrap()
    };
    fs::write(d.join("fail"), "").unwrap();
    assert!(!run("create").status.success(), "the TPM refused one key");
    assert!(
        !d.join("run/KEK.key").exists() && !d.join("run/db.key").exists(),
        "no clear key left in RAM"
    );
    assert!(
        !run("create").status.success(),
        "a second run makes the whole set or fails"
    );
    // An older half set: db.crt there, a private key missing.
    fs::create_dir_all(d.join("sb")).unwrap();
    fs::write(d.join("sb/db.crt"), "x\n").unwrap();
    fs::write(d.join("sb/db.key.cred"), "x\n").unwrap();
    let o = run("open");
    assert!(!o.status.success(), "open refuses a set with keys missing");
    assert!(
        !d.join("run/db.key").exists(),
        "a refused open leaves nothing in RAM"
    );
    fs::remove_file(d.join("fail")).unwrap();
    let o = run("create");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    for f in [
        "PK.crt",
        "KEK.crt",
        "db.crt",
        "db.der",
        "tpm2-pcr-public.pem",
        "db.key.cred",
        "KEK.key.cred",
        "PK.key.cred",
        "tpm2-pcr-private.pem.cred",
    ] {
        assert!(d.join("sb").join(f).exists(), "{f}");
    }
    assert!(
        !d.join("run/db.key").exists(),
        "create leaves no clear key in RAM"
    );
    let o = run("open");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(d.join("run/KEK.key").exists());
    assert!(run("close").status.success());
    assert!(!d.join("run/KEK.key").exists());
    // Done: nothing is made again. A new set would be new keys, so new certificate bytes; the
    // file's identity and time say it was not even rewritten with the same bytes.
    let stamp = || {
        use std::os::unix::fs::MetadataExt;
        let m = fs::metadata(d.join("sb/db.crt")).unwrap();
        (
            m.ino(),
            m.modified().unwrap(),
            fs::read(d.join("sb/db.crt")).unwrap(),
        )
    };
    let before = stamp();
    assert!(run("create").status.success());
    assert!(stamp() == before, "create made the set again");
    let left: Vec<_> = fs::read_dir(&d)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("sb.new."))
        .collect();
    assert!(left.is_empty(), "no temporary set left: {left:?}");
}
