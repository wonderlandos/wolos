//! VM scripts on a Linux host (ADR 0016): QEMU/KVM with OVMF and virtio-gpu (GL), so Hyprland runs
//! in the VM. build.sh fails fast short of disk; test-install.sh installs unattended from the ISO
//! (wolos.autoinstall) next to a bystander disk that must come out byte-identical.
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}
fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

#[test]
fn scripts_parse_and_freebsd_vm_tooling_is_gone() {
    for s in [
        "image/vm/build.sh",
        "image/vm/run.sh",
        "image/vm/run-installed.sh",
        "image/vm/test-install.sh",
        "image/vm/run-gpu.sh",
    ] {
        let o = Command::new("sh").arg("-n").arg(repo(s)).output().unwrap();
        assert!(
            o.status.success(),
            "{s}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let mode = fs::metadata(repo(s)).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "{s} executable");
        let t = read(s);
        for gone in ["ssh.sh", "build-in-vm", "bsdinstall", "zpool", "memstick"] {
            assert!(!t.contains(gone), "{s} still uses {gone}");
        }
    }
    for gone in [
        "image/vm/build-in-vm.sh",
        "image/vm/ssh.sh",
        "image/vm/update-disk.sh",
        "image/vm/up.sh",
    ] {
        assert!(!repo(gone).exists(), "{gone}");
    }
}

/// A full disk paused a build mid-way once and the build died silently: refuse up front.
#[test]
fn build_refuses_to_start_short_of_disk() {
    let d = std::env::temp_dir().join(format!("wol-vmbuild-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("bin")).unwrap();
    for (name, body) in [
        (
            "df",
            "#!/bin/sh\necho 'Filesystem 1M-blocks Used Available Use% Mounted'\necho '/dev/x 400000 395000 5000 99% /home'\n",
        ),
        (
            "mkarchiso",
            "#!/bin/sh\necho mkarchiso-called >&2\nexit 9\n",
        ),
    ] {
        let p = d.join("bin").join(name);
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let out = Command::new("sh")
        .arg(repo("image/vm/build.sh"))
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("WOLOS_VM", &d)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(err.contains("host has 5000 MB free"), "{err}");
    assert!(!err.contains("mkarchiso-called"), "{err}");
}

#[test]
fn vms_use_ovmf_kvm_and_virtio_gpu() {
    for s in [
        "image/vm/run.sh",
        "image/vm/run-installed.sh",
        "image/vm/test-install.sh",
    ] {
        let t = read(s);
        for want in [
            "qemu-system-x86_64",
            "-enable-kvm",
            "OVMF_CODE.4m.fd",
            "OVMF_VARS.4m.fd",
            "${WOLOS_VM:-$HOME/WolOS-vm}",
        ] {
            assert!(t.contains(want), "{s} lacks {want}");
        }
        // Interactive VMs draw with GL; the unattended test takes QMP screenshots, which cannot
        // read a GL scanout, so it uses the 2D device.
        let gpu = if s.ends_with("test-install.sh") {
            "-device virtio-vga "
        } else {
            "virtio-vga-gl"
        };
        assert!(t.contains(gpu), "{s} lacks {gpu}");
    }
}

#[test]
fn install_test_is_unattended_and_proves_the_bystander_disk_untouched() {
    let t = read("image/vm/test-install.sh");
    for want in [
        "wolos.autoinstall=/dev/vda",
        "console=ttyS0",
        "archisobasedir=wolos",
        "-kernel",
        "-initrd",
        "other.raw",
        "BOOT PASS",
        "wolos-install: done",
        "sfdisk -d",
        "WOLOS_KEEP_DISK",
        "$T/disk.raw",
    ] {
        assert!(t.contains(want), "test-install.sh lacks {want}");
    }
}
