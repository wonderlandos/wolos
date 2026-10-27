//! image/vm/update-disk.sh updates the owner's installed disk in a new boot environment: it must refuse a
//! disk in use, never copy or reformat it, keep homes and accounts, and leave the old BE as fallback.
use std::{fs, path::PathBuf, process::Command};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn read(p: &str) -> String {
    fs::read_to_string(repo().join(p)).unwrap()
}

#[test]
fn scripts_parse() {
    for s in [
        "image/vm/update-disk.sh",
        "image/vm/update-be.sh",
        "image/vm/sync.sh",
        "image/build.sh",
    ] {
        let out = Command::new("sh")
            .arg("-n")
            .arg(repo().join(s))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{s}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn host_side_is_safe() {
    let s = read("image/vm/update-disk.sh");
    assert!(
        s.contains("qemu-system") && s.contains("vm.pid"),
        "in-use check"
    );
    assert!(s.contains("--dry-run"));
    assert!(s.contains("serial=$SER"), "disk found by serial, not order");
    assert!(s.contains("WOLOS_STAGE_ONLY=1"), "reuses build.sh staging");
    for bad in [
        "cp \"$DISK\"",
        "dd ",
        "qemu-img convert",
        "truncate",
        "zpool create",
        "gpart create",
    ] {
        assert!(!s.contains(bad), "update-disk.sh must not {bad}");
    }
}

#[test]
fn vm_side_keeps_user_data() {
    let s = read("image/vm/update-be.sh");
    assert!(s.contains("zpool import -f -R \"$ALT\" -N -d \"/dev/$P\" \"$GUID\""));
    assert!(s.contains("bectl -r $POOL/ROOT") && s.contains("create -e \"$ACTIVE\" \"$BE\""));
    assert!(s.contains("activate \"$BE\"") && s.contains("zpool export"));
    for keep in [
        "./home",
        "./usr/home",
        "./etc/master.passwd",
        "./etc/group",
        "./etc/rc.conf",
        "./etc/fstab",
    ] {
        assert!(s.contains(keep), "exclude {keep}");
    }
    for bad in [
        "zpool destroy",
        "bectl destroy",
        "zfs destroy",
        "newfs",
        "gpart create",
        "zpool import -a",
    ] {
        assert!(!s.contains(bad), "update-be.sh must not {bad}");
    }
    assert!(
        s.contains("install-pkgs"),
        "packages via the installer's own step"
    );
}

fn run(vm: &PathBuf, args: &[&str]) -> std::process::Output {
    Command::new("sh")
        .arg(repo().join("image/vm/update-disk.sh"))
        .args(args)
        .env("WOLOS_VM", vm)
        .env("WOLOS_HOST_MIN_MB", "0")
        .output()
        .unwrap()
}

#[test]
fn dry_run_plans_and_refuses_disk_in_use() {
    let vm = std::env::temp_dir().join(format!("wol-upd-{}", std::process::id()));
    let _ = fs::remove_dir_all(&vm);
    fs::create_dir_all(vm.join("test")).unwrap();
    fs::write(vm.join("test/disk.raw"), b"").unwrap();
    let out = run(&vm, &["--dry-run"]);
    let o = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{o}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        o.contains("dry run") && o.contains("wolos-update-") && o.contains("/home"),
        "{o}"
    );
    // A live pid file next to the disk (run-installed.sh / test-install.sh) means in use.
    fs::write(vm.join("test/vm.pid"), std::process::id().to_string()).unwrap();
    let out = run(&vm, &["--dry-run"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("in use"));
    let _ = fs::remove_dir_all(&vm);
}
