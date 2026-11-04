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

/// audit3 R2B-9: Setup Mode next to Windows, with Microsoft's third-party CA still in db, gets the
/// key screen (Windows' PCR 7 stays); the BitLocker line comes with a tier that changes the keys.
#[test]
fn next_to_windows_setup_mode_takes_the_key_screen_and_bitlocker_is_said() {
    let ms = b"xx Microsoft Corporation UEFI CA 2011 xx";
    let d = efivars("win-setup", 0, 1, ms);
    let mut e = env_for(&d);
    assert_eq!(run("boot detect yes", &mut e), 0, "{}", e.err);
    assert_eq!(e.out.trim(), "own-keys", "no Windows: own keys");
    let mut e = env_for(&d);
    e.vars.push(("WOLOS_WINDOWS".into(), "yes".into()));
    run("boot detect yes", &mut e);
    assert_eq!(e.out.trim(), "mok", "Windows: the key screen");
    let mut e = FakeEnv {
        vars: vec![("WOLOS_BITLOCKER".into(), "yes".into())],
        ..Default::default()
    };
    run("boot note own-keys no", &mut e);
    assert!(
        e.out.contains("BitLocker") && e.out.contains("recovery key"),
        "{}",
        e.out
    );
    let mut e = FakeEnv {
        vars: vec![("WOLOS_BITLOCKER".into(), "yes".into())],
        ..Default::default()
    };
    run("boot note mok yes", &mut e);
    assert!(
        !e.out.contains("BitLocker"),
        "the key screen leaves db as it is: {}",
        e.out
    );
    let mut e = FakeEnv::default();
    run("boot note own-keys no", &mut e);
    assert!(!e.out.contains("BitLocker"), "{}", e.out);
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
    // Firmware tier with the first stage laid out: only the firmware setting is left, then the
    // key screen; no second install (audit2 B10).
    let mut e = FakeEnv::default();
    run("boot note firmware yes", &mut e);
    assert!(
        e.out.contains("third-party certificates")
            && e.out.contains("Enroll MOK")
            && !e.out.contains("install again"),
        "{}",
        e.out
    );
    let mut e = FakeEnv::default();
    run("boot note firmware no", &mut e);
    // No first stage laid out: turning Secure Boot off is the way in; never a second install.
    assert!(
        e.out.contains("turn Secure Boot off")
            && !e.out.contains("install again")
            && !e.out.contains("Enroll MOK"),
        "{}",
        e.out
    );
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
            (PathBuf::from("/efi/EFI/wolos/wolos-a.efi"), "MZ".into()),
            (PathBuf::from("/efi/EFI/wolos/wolos-b.efi"), "MZ".into()),
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

/// audit core-install #4: the boot check falls back only to a slot that failed no check itself
/// (the rule `wol seal check` keeps). With both slots under the floor the first refusal marks
/// its slot, the second finds that mark and stays instead of restarting back: no restart loop.
#[test]
fn two_slots_under_the_floor_do_not_restart_into_each_other() {
    let mark = |s: &str| PathBuf::from(format!("/var/lib/wolos/epoch-failed-{s}"));
    // Slot a is refused first: b has no mark, so the computer restarts into b, and a is marked.
    let mut e = booted("rootflags=subvol=@wolos-a wolos.epoch=5", "6\n");
    assert_eq!(run("boot check", &mut e), 1);
    assert_eq!(e.execs.len(), 2, "{:?}", e.execs);
    assert!(
        e.written.iter().any(|(p, _)| *p == mark("wolos-a")),
        "{:?}",
        e.written
    );
    // Slot b is under the floor too, and a carries its mark: b stays up and says why.
    let mut e = booted("rootflags=subvol=@wolos-b wolos.epoch=5", "6\n");
    e.files.push((
        PathBuf::from("/efi/loader/entries/wolos-a.conf"),
        "title WolOS 1.1\n".into(),
    ));
    e.files
        .push((mark("wolos-a"), "older than the floor\n".into()));
    assert_eq!(run("boot check", &mut e), 1);
    assert!(e.execs.is_empty(), "no restart: {:?}", e.execs);
    assert!(e.written.iter().any(|(p, _)| *p == mark("wolos-b")));
    assert!(
        e.written
            .iter()
            .any(|(p, t)| p.ends_with("verity-notice") && t.contains("no newer one")),
        "{:?}",
        e.written
    );
    // A slot the seal check refused is no way out either.
    let mut e = booted("rootflags=subvol=@wolos-a wolos.epoch=5", "6\n");
    e.files.push((
        PathBuf::from("/var/lib/wolos/verity-failed-wolos-b"),
        "failed\n".into(),
    ));
    assert_eq!(run("boot check", &mut e), 1);
    assert!(e.execs.is_empty(), "{:?}", e.execs);
    // Nor is a slot with no menu entry.
    let mut e = booted("rootflags=subvol=@wolos-a wolos.epoch=5", "6\n");
    e.files.retain(|(p, _)| !p.ends_with("wolos-b+3.conf"));
    assert_eq!(run("boot check", &mut e), 1);
    assert!(e.execs.is_empty(), "{:?}", e.execs);
    // Updated past the floor since: the slot passes and loses its old mark.
    let mut e = booted("rootflags=subvol=@wolos-a wolos.epoch=7", "6\n");
    e.files
        .push((mark("wolos-a"), "older than the floor\n".into()));
    assert_eq!(run("boot check", &mut e), 0);
    assert!(e.execs.is_empty());
    assert!(e.removed.contains(&mark("wolos-a")), "{:?}", e.removed);
}

/// QA-SYS-7: `wol boot check` asks the same question as the seal check before restarting: the
/// other slot must carry no failed mark of either check and have an entry whose image is there
/// (BOOT2-2: a ghost entry the loader cannot start brings this slot back, a restart loop).
#[test]
fn other_ok_needs_failed_mark() {
    let under = || booted("rootflags=subvol=@wolos-a wolos.epoch=5", "6\n");
    let mut e = under();
    assert_eq!(run("boot check", &mut e), 1);
    assert_eq!(
        e.execs.first().map(|a| a.join(" ")).as_deref(),
        Some("bootctl set-default wolos-b.conf")
    );
    for mark in ["verity-failed-wolos-b", "epoch-failed-wolos-b"] {
        let mut e = under();
        e.files.push((
            PathBuf::from(format!("/var/lib/wolos/{mark}")),
            "x\n".into(),
        ));
        assert_eq!(run("boot check", &mut e), 1);
        assert!(e.execs.is_empty(), "{mark}: {:?}", e.execs);
    }
    let mut e = under();
    e.files.retain(|(p, _)| !p.ends_with("wolos-b.efi"));
    assert_eq!(run("boot check", &mut e), 1);
    assert!(e.execs.is_empty(), "ghost entry: {:?}", e.execs);
}

/// QA-LIVE4-4 (live-4 F4): the newest slot's image was damaged, the firmware could not load it
/// and the person started the previous version from the menu. Its good boot sees the other
/// image no longer matches the digest esp-sync kept, makes itself the default and says so;
/// an intact other image changes nothing.
#[test]
fn manual_fallback_sets_default() {
    let sum = |c: char| c.to_string().repeat(64);
    let with = |got: Option<char>| {
        let mut e = booted("rootflags=subvol=@wolos-a wolos.epoch=7", "6\n");
        e.files.push((
            PathBuf::from("/efi/wolos-b/image.sha256"),
            format!("{}\n", sum('a')),
        ));
        if let Some(c) = got {
            e.captures.push((
                "sha256sum /efi/EFI/wolos/wolos-b.efi".into(),
                format!("{}  /efi/EFI/wolos/wolos-b.efi\n", sum(c)),
            ));
        }
        e
    };
    let mut e = with(Some('f'));
    assert_eq!(run("boot bless", &mut e), 0, "{}", e.err);
    let ran: Vec<String> = e.execs.iter().map(|a| a.join(" ")).collect();
    assert_eq!(ran, ["bootctl set-default wolos-a.conf"], "{}", e.err);
    assert!(
        e.written
            .iter()
            .any(|(p, t)| p.ends_with("verity-notice") && t.contains("damaged")),
        "{:?}",
        e.written
    );
    // Unreadable counts as damaged too.
    let mut e = with(None);
    assert_eq!(run("boot bless", &mut e), 0);
    assert_eq!(e.execs.len(), 1, "{:?}", e.execs);
    // Intact: nothing pinned.
    let mut e = with(Some('a'));
    assert_eq!(run("boot bless", &mut e), 0);
    assert!(e.execs.is_empty(), "{:?}", e.execs);
    // No digest kept (an image from before): nothing to compare.
    let mut e = booted("rootflags=subvol=@wolos-a wolos.epoch=7", "6\n");
    assert_eq!(run("boot bless", &mut e), 0);
    assert!(e.execs.is_empty(), "{:?}", e.execs);
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
