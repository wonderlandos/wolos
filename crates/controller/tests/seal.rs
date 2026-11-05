// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol seal` and `wol encrypt`: the installer and updater call these (logic in wol_core).
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use std::path::PathBuf;
use wol_core::seal::Manifest;

#[path = "../../core/tests/support/loopfs.rs"]
mod loopfs;

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

fn d(c: char) -> String {
    c.to_string().repeat(64)
}

#[test]
fn apply_enables_verity_records_digests_and_makes_the_slot_read_only() {
    let root = std::env::temp_dir().join(format!("wol-seal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::create_dir_all(root.join("usr/lib")).unwrap();
    std::fs::create_dir_all(root.join("etc")).unwrap();
    std::fs::write(root.join("usr/bin/tool"), "#!/bin/sh\n").unwrap();
    std::fs::write(root.join("usr/lib/libx.so"), b"\x7fELFxx").unwrap();
    std::fs::write(root.join("usr/lib/os-release"), "NAME=x").unwrap();
    std::fs::write(root.join("etc/passwd"), "x").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        root.join("usr/bin/tool"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::set_permissions(
        root.join("usr/lib/libx.so"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    std::fs::set_permissions(
        root.join("usr/lib/os-release"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let r = root.to_string_lossy().into_owned();
    let mut env = FakeEnv {
        root: true,
        captures: vec![
            (
                format!("fsverity measure {r}/usr/bin/tool"),
                format!("sha256:{} x", d('a')),
            ),
            (
                format!("fsverity measure {r}/usr/lib/libx.so"),
                format!("sha256:{} x", d('b')),
            ),
            // QA-F11: a complete seal covers data under /usr/lib too.
            (
                format!("fsverity measure {r}/usr/lib/os-release"),
                format!("sha256:{} x", d('c')),
            ),
        ],
        ..Default::default()
    };
    assert_eq!(
        run(&format!("seal apply {r} /efi/wolos-b/verity"), &mut env),
        0,
        "{}",
        env.err
    );
    let man = env
        .written
        .iter()
        .find(|w| w.0.ends_with("usr/lib/wolos/verity.manifest"))
        .unwrap();
    let (m, complete) = Manifest::parse_text(&man.1);
    assert!(complete, "{}", man.1);
    assert_eq!(m.0.len(), 3, "{}", man.1);
    assert_eq!(m.0["usr/bin/tool"], d('a'));
    assert_eq!(m.0["usr/lib/os-release"], d('c'));
    let dig = env
        .written
        .iter()
        .find(|w| w.0 == std::path::Path::new("/efi/wolos-b/verity"))
        .unwrap();
    assert_eq!(dig.1.trim(), m.complete_root_digest());
    let last = env.execs.last().unwrap().join(" ");
    assert_eq!(last, format!("btrfs property set -ts {r} ro true"));
    assert!(
        !env.execs
            .iter()
            .any(|e| e.join(" ") == "fsverity enable ".to_owned() + &r + "/usr/bin/tool"),
        "already measured: no enable"
    );
}

#[test]
fn apply_needs_root_and_stops_if_verity_cannot_be_enabled() {
    let mut env = FakeEnv::default();
    assert_ne!(run("seal apply /x /y", &mut env), 0);
    assert!(env.execs.is_empty());
}

#[test]
fn boot_check_with_a_tampered_manifest_tells_the_person() {
    let m = Manifest::parse(&format!("{} usr/bin/a\n", d('a')));
    let mut env = FakeEnv {
        files: vec![
            (
                PathBuf::from("/proc/cmdline"),
                "root=x rootflags=subvol=@wolos-b ro".into(),
            ),
            (
                PathBuf::from("/efi/wolos-b/verity"),
                format!("{}\n", d('0')),
            ),
            (PathBuf::from("/usr/lib/wolos/verity.manifest"), m.to_text()),
        ],
        ..Default::default()
    };
    assert_ne!(run("seal check", &mut env), 0);
    assert!(
        env.err.contains("did not pass its safety check"),
        "{}",
        env.err
    );
    assert!(
        env.written
            .iter()
            .any(|w| w.0 == std::path::Path::new(wol_core::seal::NOTICE))
    );
}

/// A seal failure restarts into the other slot only when it has a menu entry and failed neither
/// early-boot check (its seal, or the boot check's floor: audit core-install #4).
#[test]
fn a_failed_seal_falls_back_only_to_a_slot_that_failed_no_check() {
    let m = Manifest::parse(&format!("{} usr/bin/a\n", d('a')));
    let tampered = |extra: &[(&str, &str)]| {
        let mut files = vec![
            (
                PathBuf::from("/proc/cmdline"),
                "root=x rootflags=subvol=@wolos-b ro".to_string(),
            ),
            (
                PathBuf::from("/efi/wolos-b/verity"),
                format!("{}\n", d('0')),
            ),
            (PathBuf::from("/usr/lib/wolos/verity.manifest"), m.to_text()),
            (
                PathBuf::from("/efi/loader/entries/wolos-a.conf"),
                "title WolOS\nuki /EFI/wolos/wolos-a.efi\n".to_string(),
            ),
            // The entry's image is there (BOOT2-2: a ghost entry is no fallback).
            (
                PathBuf::from("/efi/EFI/wolos/wolos-a.efi"),
                "MZ".to_string(),
            ),
        ];
        files.extend(extra.iter().map(|(p, t)| (PathBuf::from(p), t.to_string())));
        FakeEnv {
            files,
            ..Default::default()
        }
    };
    let mut env = tampered(&[]);
    assert_eq!(run("seal check", &mut env), 1);
    let ran: Vec<String> = env.execs.iter().map(|a| a.join(" ")).collect();
    assert_eq!(
        ran,
        ["bootctl set-default wolos-a.conf", "systemctl reboot"]
    );
    for mark in [
        "/var/lib/wolos/verity-failed-wolos-a",
        "/var/lib/wolos/epoch-failed-wolos-a",
    ] {
        let mut env = tampered(&[(mark, "failed\n")]);
        assert_eq!(run("seal check", &mut env), 1);
        assert!(env.execs.is_empty(), "{mark}: {:?}", env.execs);
        assert!(env.err.contains("no other version"), "{}", env.err);
    }
}

/// A slot that passes loses any old mark of its own (F10: a check that could not see
/// /usr/lib/modules marked every fresh slot failed, and the mark then barred the fallback to it).
#[test]
fn boot_check_that_passes_clears_the_slots_old_mark() {
    let m = Manifest::parse(&format!(
        "{} usr/bin/a\n{} usr/lib/modules/7.3.0-5-wolos/build/scripts/x\n",
        d('a'),
        d('b')
    ));
    let mut env = FakeEnv {
        files: vec![
            (
                PathBuf::from("/proc/cmdline"),
                format!("rootflags=subvol=@wolos-a wolos.seal={}", m.root_digest()),
            ),
            (PathBuf::from("/usr/lib/wolos/verity.manifest"), m.to_text()),
        ],
        captures: vec![(
            "fsverity measure /usr/bin/a /usr/lib/modules/7.3.0-5-wolos/build/scripts/x".into(),
            format!(
                "sha256:{} /usr/bin/a\nsha256:{} /usr/lib/modules/7.3.0-5-wolos/build/scripts/x\n",
                d('a'),
                d('b')
            ),
        )],
        // Nothing is mounted on /usr: no layers (findmnt exits 1). An unscripted probe is
        // said on stderr ("it did not run"), which this test asserts is empty.
        statuses: vec![("findmnt -n -o FSTYPE /usr".into(), 1, String::new())],
        ..Default::default()
    };
    assert_eq!(run("seal check", &mut env), 0, "{}", env.err);
    assert!(env.err.is_empty(), "{}", env.err);
    assert!(
        env.removed
            .iter()
            .any(|p| p == std::path::Path::new("/var/lib/wolos/verity-failed-wolos-a")),
        "{:?}",
        env.removed
    );
    assert!(
        !env.written
            .iter()
            .any(|w| w.0.to_string_lossy().contains("verity-failed")),
        "a pass writes no mark"
    );
}

/// QA-LIVE3-16: the safety notice written for a failed slot went on for good. A pass clears it
/// once no slot carries a failed mark; while the other slot still does, the notice stays.
#[test]
fn a_pass_clears_the_notice_when_no_slot_stays_marked() {
    let m = Manifest::parse(&format!("{} usr/bin/a\n", d('a')));
    let passing = |other_mark: Option<&str>| {
        let mut files = vec![
            (
                PathBuf::from("/proc/cmdline"),
                format!("rootflags=subvol=@wolos-a wolos.seal={}", m.root_digest()),
            ),
            (PathBuf::from("/usr/lib/wolos/verity.manifest"), m.to_text()),
        ];
        if let Some(mark) = other_mark {
            files.push((PathBuf::from(mark), "failed\n".into()));
        }
        FakeEnv {
            files,
            captures: vec![(
                "fsverity measure /usr/bin/a".into(),
                format!("sha256:{} /usr/bin/a\n", d('a')),
            )],
            ..Default::default()
        }
    };
    let notice = std::path::Path::new(wol_core::seal::NOTICE);
    let mut env = passing(None);
    assert_eq!(run("seal check", &mut env), 0, "{}", env.err);
    assert!(env.removed.iter().any(|p| p == notice), "{:?}", env.removed);
    for mark in [
        "/var/lib/wolos/verity-failed-wolos-b",
        "/var/lib/wolos/epoch-failed-wolos-b",
    ] {
        let mut env = passing(Some(mark));
        assert_eq!(run("seal check", &mut env), 0, "{}", env.err);
        assert!(
            !env.removed.iter().any(|p| p == notice),
            "{mark} still marks a slot: {:?}",
            env.removed
        );
    }
}

/// QA-LIVE3-16, the other cause: `wol boot bless` writes the notice for a damaged other image
/// without a failed mark. A passing check keeps it while that image stays damaged and clears it
/// once the image verifies again (re-sealed) or no digest is kept.
#[test]
fn a_pass_keeps_the_notice_while_the_other_image_is_damaged() {
    let m = Manifest::parse(&format!("{} usr/bin/a\n", d('a')));
    let notice = std::path::Path::new(wol_core::seal::NOTICE);
    let env_with = |kept: Option<char>, got: char| {
        let mut files = vec![
            (
                PathBuf::from("/proc/cmdline"),
                format!("rootflags=subvol=@wolos-a wolos.seal={}", m.root_digest()),
            ),
            (PathBuf::from("/usr/lib/wolos/verity.manifest"), m.to_text()),
            (
                PathBuf::from(wol_core::seal::NOTICE),
                "The other version is damaged.\n".into(),
            ),
        ];
        if let Some(c) = kept {
            files.push((
                PathBuf::from("/efi/wolos-b/image.sha256"),
                format!("{}\n", d(c)),
            ));
        }
        FakeEnv {
            files,
            captures: vec![
                (
                    "fsverity measure /usr/bin/a".into(),
                    format!("sha256:{} /usr/bin/a\n", d('a')),
                ),
                (
                    "sha256sum /efi/EFI/wolos/wolos-b.efi".into(),
                    format!("{}  /efi/EFI/wolos/wolos-b.efi\n", d(got)),
                ),
            ],
            ..Default::default()
        }
    };
    let mut damaged = env_with(Some('a'), 'f');
    assert_eq!(run("seal check", &mut damaged), 0, "{}", damaged.err);
    assert!(
        !damaged.removed.iter().any(|p| p == notice),
        "{:?}",
        damaged.removed
    );
    let mut resealed = env_with(Some('a'), 'a');
    assert_eq!(run("seal check", &mut resealed), 0);
    assert!(
        resealed.removed.iter().any(|p| p == notice),
        "{:?}",
        resealed.removed
    );
    let mut no_digest = env_with(None, 'f');
    assert_eq!(run("seal check", &mut no_digest), 0);
    assert!(no_digest.removed.iter().any(|p| p == notice));
}

#[test]
fn boot_check_passes_a_slot_that_was_never_sealed() {
    let mut env = FakeEnv {
        files: vec![(
            PathBuf::from("/proc/cmdline"),
            "rootflags=subvol=@wolos-a".into(),
        )],
        ..Default::default()
    };
    assert_eq!(run("seal check", &mut env), 0);
    assert!(env.execs.is_empty());
}

#[test]
fn encryption_default_follows_the_tpm() {
    let tpm = std::env::temp_dir().join(format!("wol-tpm-{}", std::process::id()));
    std::fs::create_dir_all(&tpm).unwrap();
    std::fs::write(tpm.join("tpm_version_major"), "2\n").unwrap();
    let mut with = FakeEnv {
        vars: vec![("WOLOS_TPM_DIR".into(), tpm.to_string_lossy().into_owned())],
        files: vec![(tpm.join("tpm_version_major"), "2\n".into())],
        ..Default::default()
    };
    assert_eq!(run("encrypt decide auto", &mut with), 0);
    assert_eq!(with.out, "tpm2\n");
    // Encrypted by default (core/src/encrypt.rs default_on): no TPM means a passphrase is
    // needed, never a plain disk by accident; with one, auto unlocks with it.
    let mut none = FakeEnv::default();
    assert_ne!(run("encrypt decide auto", &mut none), 0);
    assert!(
        none.out.is_empty() && none.err.contains("passphrase"),
        "{}",
        none.err
    );
    let pass = std::env::temp_dir().join(format!("wol-pass-{}", std::process::id()));
    let mut typed = FakeEnv {
        vars: vec![(
            "WOLOS_PASSPHRASE_FILE".into(),
            pass.to_string_lossy().into_owned(),
        )],
        files: vec![(pass.clone(), "correct horse\n".into())],
        ..Default::default()
    };
    assert_eq!(run("encrypt decide auto", &mut typed), 0);
    assert_eq!(typed.out, "passphrase\n");
    let mut no = FakeEnv::default();
    assert_eq!(run("encrypt decide no", &mut no), 0);
    assert_eq!(no.out, "off\n");
    let mut need = FakeEnv::default();
    assert_ne!(run("encrypt decide yes", &mut need), 0);
    assert!(need.err.contains("passphrase"));
}

#[test]
fn enroll_keeps_the_recovery_key_in_a_file_and_never_in_arguments() {
    let out = std::env::temp_dir().join(format!("wol-rk-{}", std::process::id()));
    let mut env = FakeEnv {
        vars: vec![(
            "WOLOS_RECOVERY_OUT".into(),
            out.to_string_lossy().into_owned(),
        )],
        captures: vec![(
            "systemd-cryptenroll --recovery-key --unlock-key-file=/run/k /dev/p2".into(),
            "abcd-efgh-ijkl\n".into(),
        )],
        ..Default::default()
    };
    assert_eq!(
        run("encrypt enroll /dev/p2 /run/k tpm2", &mut env),
        0,
        "{}",
        env.err
    );
    assert_eq!(env.written[0].1, "abcd-efgh-ijkl\n");
    let cmds: Vec<String> = env.execs.iter().map(|e| e.join(" ")).collect();
    assert_eq!(
        cmds[0],
        "systemd-cryptenroll --tpm2-device=auto --tpm2-pcrs=7 --unlock-key-file=/run/k /dev/p2"
    );
    assert!(cmds.last().unwrap().contains("luksRemoveKey"));
    assert!(!cmds.join("\n").contains("abcd-efgh"));
    assert!(!env.out.contains("abcd") && !env.err.contains("abcd"));
}

const CMDLINE: &str = "root=/dev/mapper/wolosroot rd.luks.name=U-1=wolosroot quiet\n";
/// A TPM install (`encrypt::cmdline_words`, Unlock::Tpm2) and a passphrase-only one.
const TPM_CMDLINE: &str = "rd.luks.name=U-1=wolosroot rd.luks.options=U-1=tpm2-device=auto,discard root=/dev/mapper/wolosroot quiet\n";
const PASS_CMDLINE: &str =
    "rd.luks.name=U-1=wolosroot rd.luks.options=U-1=discard root=/dev/mapper/wolosroot quiet\n";

/// The chip refused (Secure Boot turned off: PCR 7 changed), the person typed their password:
/// systemd-cryptsetup's cached copy unlocks the volume once more and the TPM slot is sealed to the
/// state measured now, so the next start is automatic. The secret is never an argument.
#[test]
fn reseal_binds_the_chip_again_with_what_was_typed_and_forgets_it() {
    let mut env = FakeEnv {
        root: true,
        files: vec![
            (PathBuf::from("/proc/cmdline"), TPM_CMDLINE.into()),
            (
                PathBuf::from("/var/lib/wolos/secureboot/tpm2-pcr-public.pem"),
                "PEM".into(),
            ),
        ],
        keys: vec![("cryptsetup".into(), b"wrong\0hunter2 horse\0".to_vec())],
        exec_codes: vec![0],
        ..Default::default()
    };
    assert_eq!(run("encrypt reseal", &mut env), 0, "{}", env.err);
    assert_eq!(
        env.execs[0].join(" "),
        "systemd-cryptenroll --wipe-slot=tpm2 --tpm2-device=auto --tpm2-pcrs=7 \
--tpm2-public-key=/var/lib/wolos/secureboot/tpm2-pcr-public.pem --tpm2-public-key-pcrs=11 \
/dev/disk/by-uuid/U-1"
    );
    assert_eq!(
        env.secrets,
        [("PASSWORD".to_string(), "hunter2 horse".to_string())],
        "the last try is the one that opened the disk"
    );
    assert!(env.keys.is_empty(), "the cached secret is dropped");
    assert!(!env.execs.concat().join(" ").contains("hunter2"));
}

#[test]
fn reseal_does_nothing_when_the_chip_opened_the_disk() {
    let mut chip = FakeEnv {
        root: true,
        files: vec![(PathBuf::from("/proc/cmdline"), TPM_CMDLINE.into())],
        ..Default::default()
    };
    assert_eq!(run("encrypt reseal", &mut chip), 0);
    assert!(chip.execs.is_empty());
    let mut plain = FakeEnv {
        root: true,
        keys: vec![("cryptsetup".into(), b"pw\0".to_vec())],
        ..Default::default()
    };
    assert_eq!(
        run("encrypt reseal", &mut plain),
        0,
        "not an encrypted disk"
    );
    assert!(plain.execs.is_empty());
    let mut refused = FakeEnv {
        root: true,
        files: vec![(PathBuf::from("/proc/cmdline"), TPM_CMDLINE.into())],
        keys: vec![("cryptsetup".into(), b"a\0b\0".to_vec())],
        exec_codes: vec![1, 1],
        ..Default::default()
    };
    assert_ne!(run("encrypt reseal", &mut refused), 0);
    assert_eq!(refused.execs.len(), 2, "every try");
    assert_eq!(
        refused.execs[0].join(" "),
        "systemd-cryptenroll --wipe-slot=tpm2 --tpm2-device=auto --tpm2-pcrs=7 /dev/disk/by-uuid/U-1",
        "no policy key on this machine: PCR 7 alone"
    );
    assert!(refused.keys.is_empty());
}

/// Audit I6-1: a passphrase-only install (`rd.luks.options=<uuid>=discard`) never asked for the
/// chip. Even with the typed password cached, reseal does nothing and exits 0: no failed unit on a
/// PC without a TPM, no TPM slot added behind the person's choice on a PC with one. The cache is
/// left to expire.
#[test]
fn reseal_leaves_a_passphrase_only_install_alone() {
    for cmdline in [PASS_CMDLINE, CMDLINE] {
        let mut env = FakeEnv {
            root: true,
            files: vec![(PathBuf::from("/proc/cmdline"), cmdline.into())],
            keys: vec![("cryptsetup".into(), b"hunter2\0".to_vec())],
            exec_codes: vec![1],
            ..Default::default()
        };
        assert_eq!(run("encrypt reseal", &mut env), 0, "{}", env.err);
        assert!(env.execs.is_empty(), "no systemd-cryptenroll: {cmdline}");
        assert!(env.secrets.is_empty());
        assert!(env.err.is_empty(), "{}", env.err);
    }
}

/// The chip is asked for by the options of THIS disk's UUID only.
#[test]
fn reseal_reads_the_options_of_its_own_disk() {
    let other = "rd.luks.name=U-1=wolosroot rd.luks.options=U-2=tpm2-device=auto,discard rd.luks.options=U-1=discard\n";
    let mut env = FakeEnv {
        root: true,
        files: vec![(PathBuf::from("/proc/cmdline"), other.into())],
        keys: vec![("cryptsetup".into(), b"pw\0".to_vec())],
        ..Default::default()
    };
    assert_eq!(run("encrypt reseal", &mut env), 0);
    assert!(env.execs.is_empty());
    let both = "rd.luks.name=U-1=wolosroot rd.luks.options=U-1=discard,tpm2-device=auto\n";
    let mut env = FakeEnv {
        root: true,
        files: vec![(PathBuf::from("/proc/cmdline"), both.into())],
        keys: vec![("cryptsetup".into(), b"pw\0".to_vec())],
        exec_codes: vec![0],
        ..Default::default()
    };
    assert_eq!(run("encrypt reseal", &mut env), 0, "{}", env.err);
    assert_eq!(env.execs.len(), 1);
}

/// The disk's fallback is the account password: a new password replaces that slot (the chip
/// unlocks it), only for the account the installer named.
#[test]
fn a_new_account_password_replaces_the_disk_password_slot() {
    let files = || {
        vec![
            (PathBuf::from("/proc/cmdline"), CMDLINE.into()),
            (
                PathBuf::from("/var/lib/wolos/disk-unlock-user"),
                "ann\n".into(),
            ),
        ]
    };
    let mut env = FakeEnv {
        files: files(),
        root: true,
        stdin: vec!["new secret pw".into()],
        exec_codes: vec![1],
        ..Default::default()
    };
    assert_eq!(run("encrypt password ann", &mut env), 0, "{}", env.err);
    assert_eq!(
        env.execs[0].join(" "),
        "systemd-cryptenroll --unlock-tpm2-device=auto --wipe-slot=password --password \
/dev/disk/by-uuid/U-1"
    );
    assert_eq!(
        env.secrets,
        [("NEWPASSWORD".to_string(), "new secret pw".to_string())]
    );
    assert!(
        env.err.contains("previous password"),
        "a refused chip is said, never fails the account change"
    );
    let mut other = FakeEnv {
        files: files(),
        root: true,
        stdin: vec!["bobs pw".into()],
        ..Default::default()
    };
    assert_eq!(run("encrypt password bob", &mut other), 0);
    assert!(
        other.execs.is_empty(),
        "someone else's password never opens the disk"
    );
    let mut no_marker = FakeEnv {
        files: vec![(PathBuf::from("/proc/cmdline"), CMDLINE.into())],
        root: true,
        stdin: vec!["x".into()],
        ..Default::default()
    };
    assert_eq!(run("encrypt password ann", &mut no_marker), 0);
    assert!(
        no_marker.execs.is_empty(),
        "a passphrase disk keeps its own"
    );
}

/// A tampered wolos-b (its manifest's digest is not the sealed one), plus `extra` files.
fn tampered_b(extra: &[(&str, &str)]) -> FakeEnv {
    let m = Manifest::parse(&format!("{} usr/bin/a\n", d('a')));
    let mut files = vec![
        (
            PathBuf::from("/proc/cmdline"),
            "root=x rootflags=subvol=@wolos-b ro".to_string(),
        ),
        (
            PathBuf::from("/efi/wolos-b/verity"),
            format!("{}\n", d('0')),
        ),
        (PathBuf::from("/usr/lib/wolos/verity.manifest"), m.to_text()),
    ];
    files.extend(extra.iter().map(|(p, t)| (PathBuf::from(p), t.to_string())));
    FakeEnv {
        files,
        ..Default::default()
    }
}

const A_ENTRY: &str = "title WolOS\nversion 1-x\nuki /EFI/wolos/wolos-a.efi\n";

/// QA-SYS-12: the other slot's entry half way through its tries (`+1-2`) is still a slot to
/// start; the seal check falls back to it instead of staying on the broken one. The default it sets
/// is that entry's id, the counter taken out (systemd >= 259 boot.c `boot_entry_parse_tries`,
/// matched by `efi_fnmatch(LoaderEntryDefault, id)`; IMAGE-013-3 gate-rollback).
#[test]
fn accepts_counted_entries() {
    let mut env = tampered_b(&[
        ("/efi/loader/entries/wolos-a+1-2.conf", A_ENTRY),
        ("/efi/EFI/wolos/wolos-a.efi", "MZ"),
    ]);
    assert_eq!(run("seal check", &mut env), 1);
    let ran: Vec<String> = env.execs.iter().map(|a| a.join(" ")).collect();
    assert_eq!(
        ran,
        ["bootctl set-default wolos-a.conf", "systemctl reboot"],
        "{}",
        env.err
    );
}

/// QA-STORE-51 / BOOT2-2: the fallback goes only to an entry that was found and can start: its
/// image is on the ESP, its tries are not used up, and this slot did not already fall back once
/// (its own mark from an earlier boot: the other one did not start, so restarting loops). The
/// default it sets is the entry's id: its file name without the counter (systemd >= 259; QA-A9-CORE-5
/// had switched it to the counted name from systemd 257's boot.c, and on 262 that matched nothing).
#[test]
fn set_default_entry_found() {
    let mut found = tampered_b(&[
        ("/efi/loader/entries/wolos-a+3.conf", A_ENTRY),
        ("/efi/EFI/wolos/wolos-a.efi", "MZ"),
    ]);
    assert_eq!(run("seal check", &mut found), 1);
    assert_eq!(
        found.execs.first().map(|a| a.join(" ")).as_deref(),
        Some("bootctl set-default wolos-a.conf")
    );
    for (why, extra) in [
        (
            "ghost entry: its image is gone",
            vec![("/efi/loader/entries/wolos-a+3.conf", A_ENTRY)],
        ),
        (
            "tries used up",
            vec![
                ("/efi/loader/entries/wolos-a+0-3.conf", A_ENTRY),
                ("/efi/EFI/wolos/wolos-a.efi", "MZ"),
            ],
        ),
        (
            "entry starts nothing",
            vec![
                ("/efi/loader/entries/wolos-a.conf", "title WolOS\n"),
                ("/efi/EFI/wolos/wolos-a.efi", "MZ"),
            ],
        ),
        (
            "this slot fell back once already",
            vec![
                ("/efi/loader/entries/wolos-a.conf", A_ENTRY),
                ("/efi/EFI/wolos/wolos-a.efi", "MZ"),
                ("/var/lib/wolos/verity-failed-wolos-b", "failed\n"),
            ],
        ),
    ] {
        let mut env = tampered_b(&extra);
        assert_eq!(run("seal check", &mut env), 1, "{why}");
        assert!(env.execs.is_empty(), "{why}: {:?}", env.execs);
        assert!(env.err.contains("no other version"), "{why}: {}", env.err);
    }
}

/// A slot root on disk for `seal apply` / `seal check` (WOLOS_SEAL_ROOT): coreutils with a
/// symlinked `true`, a library, a Python module (no exec bit), locale data, usr/sbin as a link
/// to bin (Arch) and a polkit rule.
fn slot_root(name: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("wol-seal2-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for dir in [
        "usr/bin",
        "usr/lib/locale",
        "usr/lib/wolos",
        "usr/share/polkit-1/rules.d",
    ] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(root.join("usr/bin/coreutils"), b"\x7fELFcore").unwrap();
    std::fs::set_permissions(
        root.join("usr/bin/coreutils"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::os::unix::fs::symlink("coreutils", root.join("usr/bin/true")).unwrap();
    std::fs::write(root.join("usr/lib/os.py"), "import sys\n").unwrap();
    std::fs::write(root.join("usr/lib/locale/locale-archive"), "data").unwrap();
    std::os::unix::fs::symlink("bin", root.join("usr/sbin")).unwrap();
    std::fs::write(
        root.join("usr/share/polkit-1/rules.d/50-x.rules"),
        "polkit.addRule()",
    )
    .unwrap();
    root
}

fn measures(root: &str, paths: &[(&str, char)]) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = paths
        .iter()
        .map(|(p, c)| {
            (
                format!("fsverity measure {root}/{p}"),
                format!("sha256:{} {root}/{p}\n", d(*c)),
            )
        })
        .collect();
    let all: Vec<String> = paths.iter().map(|(p, _)| format!("{root}/{p}")).collect();
    let out: String = paths
        .iter()
        .map(|(p, c)| format!("sha256:{} {root}/{p}\n", d(*c)))
        .collect();
    v.push((format!("fsverity measure {}", all.join(" ")), out));
    v
}

/// QA-F11 / BOOT2-5: the seal lists every file under usr/bin and usr/lib (a Python module too),
/// its symlinks with their targets, and names locale data without sealing it; the check then
/// fails a swapped symlink and a file that was added after the seal.
#[test]
fn the_seal_covers_symlinks_and_added_files() {
    let root = slot_root("cover");
    let r = root.to_string_lossy().into_owned();
    let sealed = [
        ("usr/bin/coreutils", 'a'),
        ("usr/lib/locale/locale-archive", 'c'),
        ("usr/lib/os.py", 'b'),
    ];
    let mut env = FakeEnv {
        root: true,
        captures: measures(&r, &sealed),
        ..Default::default()
    };
    assert_eq!(
        run(&format!("seal apply {r} /efi/wolos-b/verity"), &mut env),
        0,
        "{}",
        env.err
    );
    let text = env
        .written
        .iter()
        .find(|w| w.0.ends_with("usr/lib/wolos/verity.manifest"))
        .map(|w| w.1.clone())
        .unwrap();
    let (m, complete) = Manifest::parse_text(&text);
    assert!(complete, "{text}");
    assert_eq!(m.0["usr/lib/os.py"], d('b'), "{text}");
    assert_eq!(
        m.0["usr/bin/true"],
        wol_core::seal::link_field(b"coreutils")
    );
    // D2: locale data is sealed like the rest.
    assert_eq!(m.0["usr/lib/locale/locale-archive"], d('c'));
    // D1: a directory that is a link is one entry; policy files carry a content hash.
    assert_eq!(m.0["usr/sbin"], wol_core::seal::link_field(b"bin"));
    assert!(!m.0.contains_key("usr/sbin/coreutils"), "{text}");
    assert_eq!(
        m.0["usr/share/polkit-1/rules.d/50-x.rules"],
        wol_core::seal::hash_field(b"polkit.addRule()")
    );
    let digest = env
        .written
        .iter()
        .find(|w| w.0 == std::path::Path::new("/efi/wolos-b/verity"))
        .map(|w| w.1.trim().to_owned())
        .unwrap();
    std::fs::write(root.join("usr/lib/wolos/verity.manifest"), &text).unwrap();
    let check = |root: &PathBuf| {
        let mut env = FakeEnv {
            vars: vec![("WOLOS_SEAL_ROOT".into(), r.clone())],
            files: vec![
                (
                    PathBuf::from("/proc/cmdline"),
                    format!("rootflags=subvol=@wolos-b wolos.seal={digest}"),
                ),
                (
                    PathBuf::from(format!("{r}/usr/lib/wolos/verity.manifest")),
                    text.clone(),
                ),
            ],
            captures: measures(&root.to_string_lossy(), &sealed),
            ..Default::default()
        };
        (run("seal check", &mut env), env)
    };
    let (code, env) = check(&root);
    assert_eq!(code, 0, "untouched slot passes: {}", env.err);
    // An added file.
    std::fs::write(root.join("usr/lib/qa-tamper"), "x").unwrap();
    let (code, env) = check(&root);
    assert_eq!(code, 1, "an added file fails");
    assert!(env.err.contains("1 protected"), "{}", env.err);
    std::fs::remove_file(root.join("usr/lib/qa-tamper")).unwrap();
    // A polkit rule rewritten (D1).
    let rule = root.join("usr/share/polkit-1/rules.d/50-x.rules");
    std::fs::write(&rule, "polkit.addRule(yes)").unwrap();
    let (code, _) = check(&root);
    assert_eq!(code, 1, "a rewritten polkit rule fails");
    std::fs::write(&rule, "polkit.addRule()").unwrap();
    let (code, env) = check(&root);
    assert_eq!(code, 0, "restored: {}", env.err);
    // `true` re-pointed at a script.
    std::fs::remove_file(root.join("usr/bin/true")).unwrap();
    std::os::unix::fs::symlink("/tmp/evil", root.join("usr/bin/true")).unwrap();
    let (code, _) = check(&root);
    assert_eq!(code, 1, "a re-pointed link fails");
    let _ = std::fs::remove_dir_all(&root);
}

/// QA-LIVE3-2: run with layers merged over /usr (a check by hand, or any order), the slot is
/// read through a plain bind of `/`, never through the merge: a layer is not a changed file.
#[test]
fn layer_shadowed_files_not_changed() {
    let m = Manifest::parse(&format!("{} usr/bin/a\n", d('a')));
    let view = "/run/wolos/seal-root";
    let mut env = FakeEnv {
        root: true,
        files: vec![
            (
                PathBuf::from("/proc/cmdline"),
                format!("rootflags=subvol=@wolos-a wolos.seal={}", m.root_digest()),
            ),
            (
                PathBuf::from(format!("{view}/usr/lib/wolos/verity.manifest")),
                m.to_text(),
            ),
        ],
        captures: vec![
            ("findmnt -n -o FSTYPE /usr".into(), "overlay\n".into()),
            (
                format!("fsverity measure {view}/usr/bin/a"),
                format!("sha256:{} {view}/usr/bin/a\n", d('a')),
            ),
        ],
        ..Default::default()
    };
    assert_eq!(run("seal check", &mut env), 0, "{}", env.err);
    let ran: Vec<String> = env.execs.iter().map(|a| a.join(" ")).collect();
    assert_eq!(
        ran,
        [
            "mkdir -p /run/wolos/seal-root",
            "mount --bind / /run/wolos/seal-root",
            "umount /run/wolos/seal-root"
        ]
    );
}

/// D7: layers merged over /usr and the bind of `/` fails: no verdict through the merge, no
/// mark, no restart into the other slot.
#[test]
fn a_failed_bind_with_layers_merged_stays() {
    let m = Manifest::parse(&format!("{} usr/bin/a\n", d('a')));
    let mut env = FakeEnv {
        root: true,
        files: vec![
            (
                PathBuf::from("/proc/cmdline"),
                format!("rootflags=subvol=@wolos-b wolos.seal={}", m.root_digest()),
            ),
            (PathBuf::from("/usr/lib/wolos/verity.manifest"), m.to_text()),
            (
                PathBuf::from("/efi/loader/entries/wolos-a.conf"),
                A_ENTRY.into(),
            ),
            (PathBuf::from("/efi/EFI/wolos/wolos-a.efi"), "MZ".into()),
        ],
        captures: vec![("findmnt -n -o FSTYPE /usr".into(), "overlay\n".into())],
        // mkdir ok, mount --bind fails.
        exec_codes: vec![0, 1],
        ..Default::default()
    };
    assert_eq!(run("seal check", &mut env), 1);
    let ran: Vec<String> = env.execs.iter().map(|a| a.join(" ")).collect();
    assert_eq!(
        ran,
        [
            "mkdir -p /run/wolos/seal-root",
            "mount --bind / /run/wolos/seal-root"
        ]
    );
    assert!(env.written.is_empty(), "{:?}", env.written);
    assert!(env.err.contains("not checked"), "{}", env.err);
}

/// D3: a sealed slot whose manifest was deleted fails its check (the unit no longer skips the
/// check when the file is gone).
#[test]
fn a_deleted_manifest_fails_the_check() {
    let m = Manifest::parse(&format!("{} usr/bin/a\n", d('a')));
    let mut env = FakeEnv {
        files: vec![(
            PathBuf::from("/proc/cmdline"),
            format!("rootflags=subvol=@wolos-b wolos.seal={}", m.root_digest()),
        )],
        ..Default::default()
    };
    assert_eq!(run("seal check", &mut env), 1);
    assert!(env.err.contains("protected file list"), "{}", env.err);
}

/// QA-BOOT-1: the chip refused, the person typed the recovery key, and the cached copy is there
/// but this step may not read it: that is a failure said in the journal (the unit fails), never
/// "nothing to reseal", which left the disk asking for the recovery key at every later start.
#[test]
fn reseal_fails_when_the_typed_secret_cannot_be_read() {
    let mut env = FakeEnv {
        root: true,
        files: vec![(PathBuf::from("/proc/cmdline"), TPM_CMDLINE.into())],
        unreadable_keys: vec!["cryptsetup".into()],
        ..Default::default()
    };
    let rc = run("encrypt reseal", &mut env);
    assert_ne!(rc, 0, "{}", env.out);
    assert!(env.execs.is_empty(), "{:?}", env.execs);
    assert!(env.err.contains("recovery key"), "{}", env.err);
    assert!(
        env.err.contains("the kernel refused the read"),
        "{}",
        env.err
    );
}

/// QA-LIVE3-32: `wol encrypt reseal` as a standard user was a silent success. It is root's (the
/// service runs it): anyone else is told so, with the denied exit, and nothing runs.
#[test]
fn reseal_as_a_user_says_it_is_for_administrators() {
    let mut env = FakeEnv {
        root: false,
        files: vec![(PathBuf::from("/proc/cmdline"), TPM_CMDLINE.into())],
        keys: vec![("cryptsetup".into(), b"pw\0".to_vec())],
        ..Default::default()
    };
    let rc = run("encrypt reseal", &mut env);
    assert_eq!(rc, 77, "{}", env.err);
    assert!(env.err.contains("administrator"), "{}", env.err);
    assert!(env.execs.is_empty());
    assert_eq!(env.keys.len(), 1, "the cached secret is not touched");
}

/// fs-verity as the kernel keeps it: per inode, not per path. `fsverity enable` on an inode that
/// already has it fails with EEXIST ("File exists"); `fsverity measure` answers only when every
/// file named has it (else exit 1, no answer). `raced` paths get verity from someone else between
/// the measure and the enable (their enable then fails with EEXIST too).
struct VerityFs {
    fake: FakeEnv,
    on: std::collections::HashMap<(u64, u64), String>,
    raced: Vec<String>,
    /// Paths whose enable fails without verity (a file system that cannot have it).
    refused: Vec<String>,
    enables: Vec<String>,
    /// `turn_on` asks the kernel (a loop-mounted btrfs) instead of numbering the inodes.
    real: bool,
    /// `seal apply` uses the in-process ioctls, as the real environment does.
    inproc: bool,
}

impl VerityFs {
    fn new() -> Self {
        VerityFs {
            fake: FakeEnv {
                root: true,
                ..Default::default()
            },
            on: Default::default(),
            raced: Vec::new(),
            refused: Vec::new(),
            enables: Vec::new(),
            real: false,
            inproc: false,
        }
    }
    fn inode(p: &str) -> (u64, u64) {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::symlink_metadata(p).unwrap();
        (m.dev(), m.ino())
    }
    fn turn_on(&mut self, p: &str) {
        let n = self.on.len() + 1;
        let d = if self.real {
            wol_core::seal::verity::seal_file(std::path::Path::new(p)).unwrap()
        } else {
            format!("{n:064x}")
        };
        self.on.insert(Self::inode(p), d);
    }
}

impl controller::run::Env for VerityFs {
    fn out(&mut self, s: &str) {
        self.fake.out(s)
    }
    fn err(&mut self, s: &str) {
        self.fake.err(s)
    }
    fn stdout_tty(&self) -> bool {
        false
    }
    fn var(&self, n: &str) -> Option<String> {
        self.fake.var(n)
    }
    fn is_root(&self) -> bool {
        true
    }
    fn verity_in_process(&self) -> bool {
        self.inproc
    }
    fn request(&mut self, s: &std::path::Path, r: &[u8]) -> std::io::Result<String> {
        self.fake.request(s, r)
    }
    fn exec(&mut self, a: &[String]) -> std::io::Result<i32> {
        if let [t, v, p] = a
            && t == "fsverity"
            && v == "enable"
        {
            self.fake.execs.push(a.to_vec());
            self.enables.push(p.clone());
            if self.refused.contains(p) {
                self.fake.err(&format!(
                    "ERROR: FS_IOC_ENABLE_VERITY failed on '{p}': Operation not supported\n"
                ));
                return Ok(1);
            }
            if let Some(i) = self.raced.iter().position(|r| r == p) {
                self.raced.remove(i);
                self.turn_on(p);
            }
            if self.on.contains_key(&Self::inode(p)) {
                self.fake.err(&format!(
                    "ERROR: FS_IOC_ENABLE_VERITY failed on '{p}': File exists\n"
                ));
                return Ok(1);
            }
            self.turn_on(p);
            return Ok(0);
        }
        self.fake.exec(a)
    }
    fn capture(&mut self, a: &[&str]) -> Option<String> {
        if let ["fsverity", "measure", paths @ ..] = a {
            let mut out = String::new();
            for p in paths {
                out.push_str(&format!("sha256:{} {p}\n", self.on.get(&Self::inode(p))?));
            }
            return Some(out);
        }
        self.fake.capture(a)
    }
    fn stdin_line(&mut self) -> Option<String> {
        None
    }
    fn read_file(&self, p: &std::path::Path) -> Option<String> {
        self.fake.read_file(p)
    }
    fn sys_topic(&mut self, t: &str) -> Option<serde_json::Value> {
        self.fake.sys_topic(t)
    }
    fn now_secs(&self) -> u64 {
        0
    }
    fn config_home(&self) -> PathBuf {
        PathBuf::new()
    }
    fn write_file(&mut self, p: &std::path::Path, t: &str) -> std::io::Result<()> {
        self.fake.write_file(p, t)
    }
}

fn seal_apply(env: &mut VerityFs, r: &str) -> (i32, Manifest) {
    let line = format!("seal apply {r} /efi/wolos-b/verity");
    let argv: Vec<&str> = line.split_whitespace().collect();
    let code = execute(&parse(&argv).unwrap(), env);
    let text = env
        .fake
        .written
        .iter()
        .find(|w| w.0.ends_with("usr/lib/wolos/verity.manifest"))
        .map(|w| w.1.clone())
        .unwrap_or_default();
    (code, Manifest::parse_text(&text).0)
}

/// IMG013-SEAL-HARDLINK: perl 5.44 ships usr/bin/core_perl/perlthanks as a hard link of perlbug.
/// The seal enables verity once per inode, and every path of it (both links) gets the digest;
/// before, the second link's enable failed with EEXIST and every install stopped at "seal 92%".
#[test]
fn a_hard_linked_pair_is_sealed_once_and_both_paths_get_the_digest() {
    let root = slot_root("hardlink");
    std::fs::create_dir_all(root.join("usr/bin/core_perl")).unwrap();
    std::fs::write(root.join("usr/bin/core_perl/perlbug"), "#!/usr/bin/perl\n").unwrap();
    std::fs::hard_link(
        root.join("usr/bin/core_perl/perlbug"),
        root.join("usr/bin/core_perl/perlthanks"),
    )
    .unwrap();
    let r = root.to_string_lossy().into_owned();
    let mut env = VerityFs::new();
    let (code, m) = seal_apply(&mut env, &r);
    assert_eq!(code, 0, "{}", env.fake.err);
    let bug = &m.0["usr/bin/core_perl/perlbug"];
    assert_eq!(
        &m.0["usr/bin/core_perl/perlthanks"], bug,
        "one inode, one digest"
    );
    assert_ne!(&m.0["usr/bin/coreutils"], bug);
    let perl: Vec<&String> = env
        .enables
        .iter()
        .filter(|p| p.contains("core_perl"))
        .collect();
    assert_eq!(perl.len(), 1, "verity enabled once per inode: {perl:?}");
    // Complete: every file under the sealed dirs is in the manifest with a digest.
    for f in [
        "usr/bin/coreutils",
        "usr/lib/os.py",
        "usr/lib/locale/locale-archive",
    ] {
        assert!(m.0.contains_key(f), "{f} missing: {:?}", m.0.keys());
    }
    assert_eq!(
        env.fake.execs.last().unwrap().join(" "),
        format!("btrfs property set -ts {r} ro true")
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// An inode that got verity after it was measured (another link of it the walk could not tell
/// apart, a seal still running): its enable fails with EEXIST and the seal takes the digest it
/// already has. A file that still has no digest after a failed enable stops the seal, named.
#[test]
fn an_inode_already_sealed_when_enabled_takes_its_digest_and_a_real_failure_stops() {
    let root = slot_root("eexist");
    let r = root.to_string_lossy().into_owned();
    let mut env = VerityFs::new();
    env.raced.push(format!("{r}/usr/lib/os.py"));
    let (code, m) = seal_apply(&mut env, &r);
    assert_eq!(code, 0, "{}", env.fake.err);
    assert_eq!(m.0["usr/lib/os.py"].len(), 64, "{:?}", m.0);
    // A file verity cannot be enabled on (no digest after the failure): the seal stops.
    let mut env = VerityFs::new();
    env.refused.push(format!("{r}/usr/lib/os.py"));
    let (code, _) = seal_apply(&mut env, &r);
    assert_ne!(code, 0);
    assert!(
        env.fake
            .err
            .contains("could not be enabled on usr/lib/os.py"),
        "{}",
        env.fake.err
    );
    assert!(
        !env.fake.execs.iter().any(|e| e[0] == "btrfs"),
        "not made read-only half sealed"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A slot tree with what a real one has: hard links (in one folder and across two), symlinks, a
/// link to a folder, an empty file, files of many sizes, a hashed policy file, a data file.
fn big_tree(root: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    for dir in [
        "usr/bin/core_perl",
        "usr/lib/wolos",
        "usr/lib/locale",
        "usr/libexec",
        "usr/share/polkit-1/rules.d",
    ] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    let body =
        |i: usize| -> Vec<u8> { (0..(i * 131) % 20_000).map(|j| (j * 7 + i) as u8).collect() };
    for i in 0..150 {
        let d = root.join(format!("usr/lib/lib{}", i % 9));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(format!("x{i}.so")), body(i)).unwrap();
    }
    std::fs::write(root.join("usr/bin/coreutils"), b"\x7fELFcore").unwrap();
    std::fs::set_permissions(
        root.join("usr/bin/coreutils"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::os::unix::fs::symlink("coreutils", root.join("usr/bin/true")).unwrap();
    std::fs::write(root.join("usr/bin/core_perl/perlbug"), "#!/usr/bin/perl\n").unwrap();
    std::fs::hard_link(
        root.join("usr/bin/core_perl/perlbug"),
        root.join("usr/bin/core_perl/perlthanks"),
    )
    .unwrap();
    // A link across folders, and a third link of the same inode.
    std::fs::hard_link(
        root.join("usr/lib/lib0/x0.so"),
        root.join("usr/lib/lib8/zz-link.so"),
    )
    .unwrap();
    std::fs::hard_link(
        root.join("usr/lib/lib0/x0.so"),
        root.join("usr/libexec/helper"),
    )
    .unwrap();
    std::fs::write(root.join("usr/lib/empty"), b"").unwrap();
    std::fs::write(root.join("usr/lib/locale/locale-archive"), "data").unwrap();
    std::os::unix::fs::symlink("bin", root.join("usr/sbin")).unwrap();
    std::fs::write(
        root.join("usr/share/polkit-1/rules.d/50-x.rules"),
        "polkit.addRule()",
    )
    .unwrap();
}

fn written(env: &VerityFs, ends: &str) -> String {
    env.fake
        .written
        .iter()
        .find(|w| w.0.to_string_lossy().ends_with(ends))
        .map(|w| w.1.clone())
        .unwrap_or_default()
}

/// IMG013-3-SEAL-SLOW: the in-process seal (four threads, the kernel's ioctls) writes the same
/// manifest and root digest, byte for byte, as the `fsverity` program run one file at a time did,
/// on the same tree (hard links of one inode in one and in two folders, links, an empty file).
/// Both runs ask the same kernel (a loop-mounted btrfs), so the digests are the real ones.
#[test]
fn the_in_process_seal_writes_the_manifest_the_fsverity_program_did() {
    let Some(fs) = loopfs::LoopFs::new("seal-eq") else {
        return;
    };
    let (a, b) = (fs.path().join("slot-a"), fs.path().join("slot-b"));
    for r in [&a, &b] {
        big_tree(r);
    }
    let mut old = VerityFs::new();
    old.real = true;
    let (code, _) = seal_apply(&mut old, &a.to_string_lossy());
    assert_eq!(code, 0, "{}", old.fake.err);
    let mut new = VerityFs::new();
    new.inproc = true;
    let (code, _) = seal_apply(&mut new, &b.to_string_lossy());
    assert_eq!(code, 0, "{}", new.fake.err);
    assert!(
        new.enables.is_empty(),
        "no fsverity process in the new path"
    );
    assert!(!old.enables.is_empty());
    let (mo, mn) = (
        written(&old, "usr/lib/wolos/verity.manifest"),
        written(&new, "usr/lib/wolos/verity.manifest"),
    );
    assert!(
        mo.starts_with("wolos-seal 2\n") && mo.lines().count() > 150,
        "{mo}"
    );
    assert_eq!(mo, mn, "manifest bytes");
    assert_eq!(
        written(&old, "/efi/wolos-b/verity"),
        written(&new, "/efi/wolos-b/verity"),
        "root digest"
    );
    // The links of one inode have one digest; the empty file and the link have theirs.
    let m = Manifest::parse_text(&mn).0;
    assert_eq!(m.0["usr/lib/lib0/x0.so"], m.0["usr/libexec/helper"]);
    assert_eq!(
        m.0["usr/bin/core_perl/perlbug"],
        m.0["usr/bin/core_perl/perlthanks"]
    );
    assert_eq!(m.0["usr/lib/lib8/zz-link.so"], m.0["usr/lib/lib0/x0.so"]);
    assert_ne!(m.0["usr/lib/empty"], m.0["usr/bin/coreutils"]);
}

/// SEAL-FLAKE: a file the kernel will not seal is reported as that, with the system's reason,
/// at the step that failed; never "no fs-verity digest" (which a killed program also produced).
#[test]
fn an_in_process_failure_names_the_step_and_the_reason_and_seals_nothing() {
    // /dev/shm is tmpfs: no fs-verity.
    let root =
        std::path::PathBuf::from("/dev/shm").join(format!("wol-seal-none-{}", std::process::id()));
    if !std::path::Path::new("/dev/shm").is_dir() {
        return;
    }
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("usr/lib")).unwrap();
    std::fs::write(root.join("usr/lib/os.py"), "import sys\n").unwrap();
    let mut env = VerityFs::new();
    env.inproc = true;
    let (code, _) = seal_apply(&mut env, &root.to_string_lossy());
    let _ = std::fs::remove_dir_all(&root);
    assert_ne!(code, 0);
    let err = &env.fake.err;
    assert!(
        err.contains("usr/lib/os.py: fs-verity could not be enabled: "),
        "{err}"
    );
    assert!(!err.contains("no fs-verity digest"), "{err}");
    assert!(
        !env.fake.execs.iter().any(|e| e[0] == "btrfs"),
        "not made read-only"
    );
}
