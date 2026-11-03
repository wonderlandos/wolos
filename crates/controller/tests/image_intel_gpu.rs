// SPDX-License-Identifier: GPL-2.0-or-later
//! Intel iGPU stack on first boot: firmware + user-space packages offline, i915 tunables, session env,
//! native-boot proof script. Evidence: docs/modules/intel-graphics.md.
#[path = "support/repo.rs"]
mod repo;
use repo::{read, repo};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn session_sources_gpu_env() {
    let bin = std::env::temp_dir().join(format!("wol-sess-{}", std::process::id()));
    let _ = fs::remove_dir_all(&bin);
    fs::create_dir_all(bin.join("home")).unwrap();
    let out = bin.join("env.txt");
    for (name, body) in [
        (
            "wol-realtime",
            "#!/bin/sh\n[ \"$*\" = 'gpu env' ] && printf \"export LIBVA_DRIVER_NAME='iHD'\\nexport AQ_DRM_DEVICES='/dev/dri/card0'\\n\"\n".to_string(),
        ),
        ("dbus-run-session", format!("#!/bin/sh\nenv > {}\n", out.display())),
    ] {
        let p = bin.join(name);
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let st = Command::new("sh")
        .arg(repo("desktop/overlay/usr/bin/wol-session"))
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("HOME", bin.join("home"))
        .env("XDG_RUNTIME_DIR", &bin)
        .env("XDG_CONFIG_HOME", bin.join("home/.config"))
        .status()
        .unwrap();
    assert!(st.success());
    let env = fs::read_to_string(&out).unwrap();
    assert!(env.lines().any(|l| l == "LIBVA_DRIVER_NAME=iHD"), "{env}");
    assert!(
        env.lines().any(|l| l == "AQ_DRM_DEVICES=/dev/dri/card0"),
        "{env}"
    );
}

#[test]
fn verify_gpu_script_records_the_proof() {
    let p = "files/usr/lib/wolos/verify-gpu.sh";
    let s = read(p);
    assert!(
        Command::new("sh")
            .arg("-n")
            .arg(repo(p))
            .status()
            .unwrap()
            .success()
    );
    for want in [
        "/var/log/wolos-gpu-report.txt",
        "lspci -nnk",
        "/sys/module/i915/parameters",
        "/sys/kernel/wolos",
        "/sys/class/powercap/intel-rapl:0",
        "modprobe -q intel_rapl_msr",
        "dmesg",
        "guc",
        "huc",
        "eglinfo",
        "glxinfo",
        "vulkaninfo --summary",
        "vainfo",
        "vkmark",
        "glmark2",
        "vkpeak",
        "wol-realtime power show",
        "*boost*",
        "noccs",
    ] {
        assert!(s.contains(want), "{p} lacks {want}");
    }
    // The script came from the FreeBSD base (ADR 0016 moved to Linux): none of its tools remain.
    for code in s.lines().filter(|l| !l.trim_start().starts_with('#')) {
        for gone in [
            "kldload",
            "kldstat",
            "pciconf",
            "cpucontrol",
            "cpuctl",
            "sysctl ",
            "compat.linuxkpi",
            "/boot/modules",
            "/var/log/messages",
            "/usr/local/libexec",
            "wolos-release",
        ] {
            assert!(!code.contains(gone), "{p}: FreeBSD `{gone}` in: {code}");
        }
    }
}

/// The proof script runs to its report on any Linux box: every section, no FreeBSD tool missing.
/// Not as root (it would mount debugfs and sample RAPL for 10 s).
#[test]
fn verify_gpu_script_runs_on_linux() {
    let uid = Command::new("id").arg("-u").output().unwrap();
    if String::from_utf8_lossy(&uid.stdout).trim() == "0" {
        eprintln!("root: verify-gpu.sh run skipped (it writes debugfs and samples RAPL)");
        return;
    }
    let dir = std::env::temp_dir().join(format!("wol-verify-gpu-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let report = dir.join("report.txt");
    let o = Command::new("sh")
        .arg(repo("files/usr/lib/wolos/verify-gpu.sh"))
        .env("WOL_REPORT", &report)
        .env("WOL_BENCH", "0")
        .env("WOL_AB", "1")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("verify-gpu: wrote"));
    let r = fs::read_to_string(&report).unwrap();
    for sec in [
        "system",
        "driver + firmware",
        "WolKernel hints",
        "idle power",
        "A/B (WOL_AB=1)",
    ] {
        assert!(r.contains(&format!("===== {sec}")), "section {sec}: {r}");
    }
    for gone in ["kldload", "kldstat", "pciconf", "cpucontrol", "sysctl"] {
        assert!(
            !r.contains(&format!("{gone}: not found")),
            "{gone} called: {r}"
        );
    }
    fs::remove_dir_all(&dir).ok();
}
