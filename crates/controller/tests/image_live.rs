//! Live ISO (ADR 0009): default boot is the WolOS desktop on a read-only cd9660 root, writable state
//! in memory, "Install WolOS" runs the existing installer.
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}

/// Runs libexec/wolos/live with fake mount/chown; root fs type `rootfs`. Returns the command log.
fn run_live(rootfs: &str) -> String {
    let dir = std::env::temp_dir().join(format!("wol-live-{}-{rootfs}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let bin = dir.join("bin");
    let root = dir.join("root");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&root).unwrap();
    let log = dir.join("log");
    let l = log.display();
    for (name, body) in [
        (
            "mount",
            format!(
                "#!/bin/sh\nif [ \"$1\" = -p ]; then echo '/dev/x / {rootfs} ro 0 0'; else echo mount \"$@\" >> {l}; fi\n"
            ),
        ),
        ("chown", format!("#!/bin/sh\necho chown \"$@\" >> {l}\n")),
        ("logger", "#!/bin/sh\n:\n".into()),
    ] {
        let p = bin.join(name);
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let out = Command::new("sh")
        .arg(repo("image/files/usr/local/libexec/wolos/live"))
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("WOLOS_LIVE_ROOT", &root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    if rootfs == "cd9660" {
        assert!(root.join("home/live").is_dir(), "no live home");
    }
    fs::read_to_string(&log).unwrap_or_default()
}

#[test]
fn live_overlays_memory_only_on_cd9660_root() {
    let s = run_live("cd9660");
    for want in [
        "mount -t tmpfs tmpfs ",
        "mount -t unionfs ",
        "/etc\n",
        "/usr/local/etc\n",
        "/home\n",
        "chown -R 1000:1000 ",
    ] {
        assert!(s.contains(want), "live log lacks {want:?}:\n{s}");
    }
    // Only memory filesystems: never a disk device.
    assert!(!s.contains("/dev/"), "live mounted a device:\n{s}");
    assert_eq!(run_live("ufs"), "", "installed/USB root must be left alone");
}

#[test]
fn build_ships_live_user_autologin_and_install_launcher() {
    let b = fs::read_to_string(repo("image/build.sh")).unwrap();
    for want in [
        "useradd live -u 1000",
        "useradd _wollogin -u 911",
        r#"{"autologin":"live"}"#,
        "wolos_live_enable=YES",
        "Install WolOS",
        "wol-install",
        "/etc/wolos-live",
    ] {
        assert!(b.contains(want), "image/build.sh lacks {want}");
    }
    let rc = fs::read_to_string(repo("image/files/etc/rc.d/wolos_live")).unwrap();
    assert!(rc.contains("BEFORE: wolos_gpu wolos_wifi wolos_firstboot LOGIN"));
    let inst = fs::read_to_string(repo("image/files/usr/local/libexec/wolos/wol-install")).unwrap();
    assert!(inst.contains("bsdinstall script /etc/installerconfig"));
    for p in [
        "image/files/usr/local/libexec/wolos/live",
        "image/files/usr/local/libexec/wolos/wol-install",
        "image/files/etc/rc.d/wolos_live",
    ] {
        assert!(
            Command::new("sh")
                .arg("-n")
                .arg(repo(p))
                .status()
                .unwrap()
                .success()
        );
    }
}
