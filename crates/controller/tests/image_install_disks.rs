//! image/files/usr/local/libexec/wolos/install-disks: disks the installer may erase.
//! Never the boot media: the memstick, the ISO (iso9660/WOLOS) or a Ventoy stick (msdosfs/VTOYEFI).
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

fn run(name: &str, disks: &str, labels: &str) -> String {
    let bin = std::env::temp_dir().join(format!("wol-idisks-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&bin);
    fs::create_dir_all(&bin).unwrap();
    for (n, body) in [
        ("sysctl", format!("#!/bin/sh\necho '{disks}'\n")),
        ("glabel", format!("#!/bin/sh\nprintf '%s' '{labels}'\n")),
    ] {
        let p = bin.join(n);
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../image/files/usr/local/libexec/wolos/install-disks");
    let out = Command::new("sh")
        .arg(script)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn boot_media_is_never_an_install_target() {
    let hdr = "                                      Name  Status  Components\n";
    for (name, disks, labels, want) in [
        ("plain", "nda0 cd0 md0", "", "nda0\n"),
        (
            "memstick",
            "nda0 da0",
            "ufs/FreeBSD_Install     N/A  da0s2a\n",
            "nda0\n",
        ),
        (
            "ventoy",
            "nda0 da0 cd0",
            "msdosfs/VTOYEFI     N/A  da0p2\n",
            "nda0\n",
        ),
        (
            "iso-on-disk",
            "ada0 da1",
            "iso9660/WOLOS     N/A  da1\n",
            "ada0\n",
        ),
    ] {
        assert_eq!(run(name, disks, &format!("{hdr}{labels}")), want, "{name}");
    }
}
