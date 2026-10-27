//! image/vm/build-in-vm.sh fails fast when the host is short of disk (a full /home paused the build VM
//! mid-install with an io-error and the build died silently).
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

#[test]
fn refuses_to_start_under_six_gb_free() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../image/vm/build-in-vm.sh");
    let bin = std::env::temp_dir().join(format!("wol-bvm-{}", std::process::id()));
    let _ = fs::remove_dir_all(&bin);
    fs::create_dir_all(&bin).unwrap();
    for (name, body) in [
        // df -Pm: 5000 MB available.
        (
            "df",
            "#!/bin/sh\necho 'Filesystem 1M-blocks Used Available Capacity Mounted'\necho '/dev/x 400000 395000 5000 99% /home'\n",
        ),
        ("ssh", "#!/bin/sh\necho ssh-called >&2\nexit 9\n"),
    ] {
        let p = bin.join(name);
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let out = Command::new("sh")
        .arg(&script)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("WOLOS_VM", &bin)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(err.contains("host has 5000 MB free"), "{err}");
    assert!(!err.contains("ssh-called"), "{err}");
}
