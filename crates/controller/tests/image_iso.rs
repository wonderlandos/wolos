//! image/iso.sh: Ventoy boots FreeBSD ISOs (not raw .img), so the installer image is also packed as a
//! hybrid UEFI/BIOS ISO with the release tooling, with a cd9660 root.
use std::{path::PathBuf, process::Command};

#[test]
fn iso_uses_release_tooling_and_cd9660_root() {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../image/iso.sh");
    let s = std::fs::read_to_string(&p).unwrap_or_default();
    assert!(
        Command::new("sh")
            .arg("-n")
            .arg(&p)
            .status()
            .unwrap()
            .success()
    );
    for want in [
        "release/amd64/mkisoimages.sh",
        "/dev/iso9660/$LABEL / cd9660 ro",
        "root_rw_mount=\"NO\"",
        "sha256",
    ] {
        assert!(s.contains(want), "image/iso.sh lacks {want}");
    }
}
