// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol boot`: the tier a firmware gets, the words the installer shows, the boot-time rollback
//! check and the bless after a good boot.
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use std::path::{Path, PathBuf};
use wol_core::boot::{EFI_DB, EFI_GLOBAL, Status};

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

/// A directory of firmware variables, as efivarfs shows them.
fn efivars(name: &str, sb: u8, setup: u8, db: &[u8]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-boot-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let w = |n: String, v: &[u8]| std::fs::write(d.join(n), v).unwrap();
    w(format!("SecureBoot-{EFI_GLOBAL}"), &[6, 0, 0, 0, sb]);
    w(format!("SetupMode-{EFI_GLOBAL}"), &[6, 0, 0, 0, setup]);
    w(format!("db-{EFI_DB}"), db);
    d
}

fn env_for(d: &Path) -> FakeEnv {
    let _ = std::fs::write(d.join("shimx64.efi"), "");
    FakeEnv {
        vars: vec![
            ("WOLOS_EFIVARS".into(), d.display().to_string()),
            ("WOLOS_PCI_DIR".into(), d.join("none").display().to_string()),
            ("WOLOS_SHIM_DIR".into(), d.display().to_string()),
        ],
        ..Default::default()
    }
}

#[test]
fn detect_reads_the_firmware() {
    let ms = b"xx Microsoft UEFI CA 2023 xx";
    for (name, sb, setup, db, want, line) in [
        ("setup", 0, 1, &b""[..], "own-keys", "boot detect yes"),
        ("user", 1, 0, &ms[..], "mok", "boot detect yes"),
        (
            "winonly",
            1,
            0,
            &b"Windows"[..],
            "firmware",
            "boot detect yes",
        ),
        ("offsb", 0, 0, &b""[..], "off", "boot detect yes"),
        ("optout", 0, 1, &b""[..], "off", "boot detect no"),
    ] {
        let d = efivars(name, sb, setup, db);
        let mut e = env_for(&d);
        assert_eq!(run(line, &mut e), 0, "{}", e.err);
        assert_eq!(e.out.trim(), want, "{name}");
    }
}

#[test]
fn enroll_retries_with_one_command_and_says_what_comes_next() {
    let d = efivars("en", 1, 0, b"");
    let mut e = env_for(&d);
    e.captures
        .push(("/usr/lib/wolos/boot-enroll".into(), "queued\n".into()));
    assert_eq!(run("boot enroll", &mut e), 0, "{}", e.err);
    assert!(e.out.contains("Restart"), "{}", e.out);
    assert!(e.out.contains("type boot"), "{}", e.out);
}

#[test]
fn an_nvidia_card_needs_the_key_screen_even_with_own_keys() {
    let d = efivars("nv", 0, 1, b"");
    let gpu = d.join("pci/0000:01:00.0");
    std::fs::create_dir_all(&gpu).unwrap();
    std::fs::write(gpu.join("vendor"), "0x10de\n").unwrap();
    std::fs::write(gpu.join("class"), "0x030000\n").unwrap();
    let mut e = env_for(&d);
    e.vars[1].1 = d.join("pci").display().to_string();
    run("boot key-screen own-keys", &mut e);
    assert_eq!(e.out.trim(), "yes");
    let mut e = env_for(&d);
    run("boot key-screen own-keys", &mut e);
    assert_eq!(e.out.trim(), "no", "no such card");
}

#[test]
fn words_and_notes_for_the_installer() {
    let mut e = FakeEnv::default();
    run("boot password", &mut e);
    assert_eq!(e.out.trim(), wol_core::boot::MOK_PASSWORD);
    let mut e = FakeEnv::default();
    run("boot words mok", &mut e);
    assert_eq!(e.out.trim(), "lockdown=integrity module.sig_enforce=1");
    let mut e = FakeEnv::default();
    run("boot note mok yes", &mut e);
    assert!(
        e.out.contains("blue screen") && e.out.contains("type boot"),
        "{}",
        e.out
    );
    let mut e = FakeEnv::default();
    run("boot note own-keys no", &mut e);
    assert!(!e.out.contains("Enroll MOK"), "{}", e.out);
    assert_eq!(run("boot words nonsense", &mut FakeEnv::default()), 64);
}

fn booted(cmdline: &str, floor: &str) -> FakeEnv {
    FakeEnv {
        root: true,
        files: vec![
            (PathBuf::from("/proc/cmdline"), cmdline.into()),
            (PathBuf::from("/var/lib/wolos/secureboot/floor"), floor.into()),
            (
                PathBuf::from("/efi/loader/entries/wolos-a.conf"),
                "title WolOS 1.1\nsort-key wolos\nversion 7-2026.11.02\nuki /EFI/wolos/wolos-a.efi\n".into(),
            ),
            (
                PathBuf::from("/efi/loader/entries/wolos-b+3.conf"),
                "title WolOS 1.0 (previous version)\nsort-key wolos\nversion 6-2026.10.20\nuki /EFI/wolos/wolos-b.efi\n"
                    .into(),
            ),
        ],
        captures: vec![(
            "ls -1 /efi/loader/entries".into(),
            "wolos-a.conf\nwolos-b+3.conf\nwolos-a-linux-zen.conf\n".into(),
        )],
        ..Default::default()
    }
}

#[test]
fn a_kernel_older_than_the_floor_does_not_stay_up() {
    let mut e = booted("rootflags=subvol=@wolos-a wolos.epoch=5", "6\n");
    assert_eq!(run("boot check", &mut e), 1);
    let ran: Vec<String> = e.execs.iter().map(|a| a.join(" ")).collect();
    assert_eq!(
        ran,
        ["bootctl set-default wolos-b.conf", "systemctl reboot"]
    );
    assert!(
        e.written
            .iter()
            .any(|(p, t)| p.ends_with("verity-notice") && t.contains("older")),
        "{:?}",
        e.written
    );
    let mut e = booted("rootflags=subvol=@wolos-a wolos.epoch=7", "6\n");
    assert_eq!(run("boot check", &mut e), 0);
    assert!(e.execs.is_empty());
}

#[test]
fn a_good_boot_moves_the_floor_and_names_the_other_slot() {
    let mut e = booted("rootflags=subvol=@wolos-b wolos.epoch=6", "2\n");
    assert_eq!(run("boot bless", &mut e), 0, "{}", e.err);
    let w = |n: &str| {
        e.written
            .iter()
            .find(|(p, _)| p.to_string_lossy().contains(n))
            .map(|(_, t)| t.clone())
            .unwrap_or_else(|| panic!("{n}: {:?}", e.written))
    };
    assert_eq!(w("floor").trim(), "6");
    assert!(w("wolos-a.conf").starts_with("title WolOS 1.1 (previous version)\n"));
    assert!(w("wolos-b+3.conf").starts_with("title WolOS 1.0\n"));
    assert!(
        !e.written
            .iter()
            .any(|(p, _)| p.to_string_lossy().contains("linux-zen"))
    );
}

#[test]
fn status_is_json_for_settings() {
    let d = efivars("status", 1, 0, b"");
    let mut e = env_for(&d);
    e.files = vec![
        (
            PathBuf::from("/proc/cmdline"),
            "wolos.epoch=9 wolos.seal=ab rootflags=subvol=@wolos-a".into(),
        ),
        (
            PathBuf::from("/var/lib/wolos/secureboot/tier"),
            "own-keys\n".into(),
        ),
        (
            PathBuf::from("/var/lib/wolos/secureboot/floor"),
            "8\n".into(),
        ),
        (
            PathBuf::from("/sys/kernel/security/lockdown"),
            "none [integrity] confidentiality\n".into(),
        ),
        (
            PathBuf::from("/sys/class/tpm/tpm0/tpm_version_major"),
            "2\n".into(),
        ),
    ];
    assert_eq!(run("boot status", &mut e), 0);
    let s: Status = serde_json::from_str(&e.out).unwrap();
    assert!(s.secure_boot && s.tpm2 && s.sealed);
    assert_eq!(
        (s.tier.as_str(), s.lockdown.as_str(), s.epoch, s.floor),
        ("own-keys", "integrity", Some(9), 8)
    );
}

/// The seal's root digest travels in the signed command line; a file on the ESP can be swapped.
#[test]
fn the_seal_trusts_the_signed_command_line_over_the_esp() {
    let man = wol_core::seal::Manifest::default();
    let mut e = FakeEnv {
        root: true,
        files: vec![
            (
                PathBuf::from("/proc/cmdline"),
                format!("rootflags=subvol=@wolos-a wolos.seal={}", "0".repeat(64)),
            ),
            (PathBuf::from("/efi/wolos-a/verity"), man.root_digest()),
            (
                PathBuf::from("/usr/lib/wolos/verity.manifest"),
                man.to_text(),
            ),
        ],
        ..Default::default()
    };
    assert_eq!(
        run("seal check", &mut e),
        1,
        "the ESP copy agrees, the signed word does not"
    );
}
