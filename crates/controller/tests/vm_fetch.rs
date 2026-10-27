//! image/vm/fetch.sh resumes an interrupted copy-back and verifies sha256 (a 3.5 GB `ssh cat` stopped
//! at 2.8 GB once and left a short file).
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

#[test]
fn resumes_after_a_cut_and_verifies_sha256() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("wol-fetch-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let data: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
    fs::write(dir.join("remote.bin"), &data).unwrap();
    let sum = Command::new("sha256sum")
        .arg(dir.join("remote.bin"))
        .output()
        .unwrap();
    let sum = String::from_utf8_lossy(&sum.stdout);
    fs::write(
        dir.join("remote.bin.sha256"),
        format!("{} remote.bin\n", &sum[..64]),
    )
    .unwrap();
    // Fake ssh: runs the command locally; the first data transfer is cut after 100000 bytes.
    let ssh = dir.join("ssh");
    fs::write(
        &ssh,
        format!(
            "#!/bin/sh\ncd {d}\nif [ ! -f cut ] && echo \"$*\" | grep -q 'tail -c'; then touch cut; sh -c \"$*\" | head -c 100000; exit 255; fi\nexec sh -c \"$*\"\n",
            d = dir.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    let out = Command::new("sh")
        .arg(root.join("image/vm/fetch.sh"))
        .arg("remote.bin")
        .arg(dir.join("local.bin"))
        .env("WOLOS_SSH", &ssh)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    assert_eq!(fs::read(dir.join("local.bin")).unwrap(), data);
    assert!(dir.join("local.bin.sha256").exists());
    // A corrupted local file that is already full length is refetched, not trusted.
    let mut bad = data.clone();
    bad[5] ^= 1;
    fs::write(dir.join("local.bin"), &bad).unwrap();
    let out = Command::new("sh")
        .arg(root.join("image/vm/fetch.sh"))
        .arg("remote.bin")
        .arg(dir.join("local.bin"))
        .env("WOLOS_SSH", &ssh)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(fs::read(dir.join("local.bin")).unwrap(), data);
    let _ = fs::remove_dir_all(&dir);
}
