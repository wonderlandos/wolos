//! Intel iGPU stack on first boot: firmware + user-space packages offline, i915 tunables, session env,
//! native-boot proof script. Evidence: archicture/modules/intel-graphics.md.
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn packages() -> Vec<String> {
    let mut v = Vec::new();
    for f in ["image/packages-base.txt", "desktop/packages.txt"] {
        for l in read(f).lines() {
            let p = l.split('#').next().unwrap().trim();
            if !p.is_empty() {
                v.push(p.to_owned());
            }
        }
    }
    v
}

#[test]
fn offline_repo_has_intel_firmware_and_userspace() {
    let pkgs = packages();
    for want in [
        // DMC/GuC/HuC .ko for ADL-P (46a8), ADL-S/RPL, TGL (+ HuC for ADL), RKL, MTL.
        "gpu-firmware-intel-kmod-alderlake",
        "gpu-firmware-intel-kmod-tigerlake",
        "gpu-firmware-intel-kmod-rocketlake",
        "gpu-firmware-intel-kmod-meteorlake",
        // GL (iris) + Vulkan (anv) + VA-API (iHD).
        "mesa-dri",
        "mesa-libs",
        "vulkan-loader",
        "libva",
        "libva-intel-media-driver",
        // Proof tools for image/hw/verify-gpu.sh.
        "mesa-demos",
        "vulkan-tools",
        "libva-utils",
        "glmark2",
        "vkmark",
        // Compute: no Intel OpenCL/Level Zero runtime on FreeBSD; Vulkan compute (anv) peak + BW.
        "vkpeak",
    ] {
        assert!(pkgs.iter().any(|p| p == want), "missing package {want}");
    }
}

#[test]
fn i915_tunables_are_upstream_defaults_only() {
    let s = read("image/files/boot/loader.conf.d/wolos-i915.conf");
    let kv: Vec<(&str, &str)> = s
        .lines()
        .map(|l| l.split('#').next().unwrap().trim())
        .filter(|l| !l.is_empty())
        .map(|l| l.split_once('=').expect(l))
        .collect();
    for (k, _) in &kv {
        assert!(
            k.starts_with("compat.linuxkpi.i915_"),
            "{k}: Intel-scoped only"
        );
    }
    for want in [
        ("compat.linuxkpi.i915_enable_guc", "\"0\""),
        ("compat.linuxkpi.i915_enable_fbc", "\"-1\""),
        ("compat.linuxkpi.i915_enable_psr", "\"-1\""),
        ("compat.linuxkpi.i915_enable_dc", "\"-1\""),
    ] {
        assert!(kv.contains(&want), "want {want:?} in {kv:?}");
    }
    let doc = read("archicture/modules/intel-graphics.md");
    for (k, _) in &kv {
        assert!(
            doc.contains(k.trim_start_matches("compat.linuxkpi.")),
            "{k} undocumented"
        );
    }
}

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
        .arg(repo("desktop/overlay/usr/local/bin/wol-session"))
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
    let p = "image/hw/verify-gpu.sh";
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
        "kldstat",
        "sysctl hw.dri",
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
        "i915_wol_waitboost",
        "noccs",
    ] {
        assert!(s.contains(want), "{p} lacks {want}");
    }
}

#[test]
fn install_test_checks_intel_stack() {
    let s = read("image/vm/test-install.sh");
    for want in [
        "i915_adlp_dmc_bin.ko",
        "libva-intel-media-driver",
        "intel_icd",
        "iris_dri.so",
        "strings /boot/modules/i915kms.ko | grep -c wol_fg_pid",
        "wol-realtime gpu caps",
        "wol-realtime power show",
        "vkpeak",
    ] {
        assert!(s.contains(want), "test-install.sh lacks {want}");
    }
    // Never touch the kept disk of earlier runs (run-installed.sh): scratch disk has its own name.
    assert!(
        !s.contains("rm -f serial.log inst.img disk.raw"),
        "scratch run deletes disk.raw"
    );
}
