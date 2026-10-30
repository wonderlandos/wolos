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
    let mut none = FakeEnv::default();
    assert_eq!(run("encrypt decide auto", &mut none), 0);
    assert_eq!(none.out, "off\n");
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
