// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol seal` and `wol encrypt`: the installer and updater call these (logic in wol_core).
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use std::path::PathBuf;
use wol_core::seal::Manifest;

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
    let m = Manifest::parse(&man.1);
    assert_eq!(m.0.len(), 2, "{}", man.1);
    assert_eq!(m.0["usr/bin/tool"], d('a'));
    let dig = env
        .written
        .iter()
        .find(|w| w.0 == std::path::Path::new("/efi/wolos-b/verity"))
        .unwrap();
    assert_eq!(dig.1.trim(), m.root_digest());
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
                "title WolOS\n".to_string(),
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
        files: vec![(PathBuf::from("/proc/cmdline"), TPM_CMDLINE.into())],
        ..Default::default()
    };
    assert_eq!(run("encrypt reseal", &mut chip), 0);
    assert!(chip.execs.is_empty());
    let mut plain = FakeEnv {
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
        files: vec![(PathBuf::from("/proc/cmdline"), other.into())],
        keys: vec![("cryptsetup".into(), b"pw\0".to_vec())],
        ..Default::default()
    };
    assert_eq!(run("encrypt reseal", &mut env), 0);
    assert!(env.execs.is_empty());
    let both = "rd.luks.name=U-1=wolosroot rd.luks.options=U-1=discard,tpm2-device=auto\n";
    let mut env = FakeEnv {
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
