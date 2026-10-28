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
    for f in ["image/arch/packages.x86_64", "desktop/packages.txt"] {
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
fn image_has_intel_firmware_and_userspace() {
    let pkgs = packages();
    for want in [
        // i915/xe firmware (DMC, GuC, HuC) and Intel CPU microcode.
        "linux-firmware",
        "intel-ucode",
        // GL (iris) + Vulkan (anv) + VA-API (iHD), 64- and 32-bit for Steam.
        "mesa",
        "lib32-mesa",
        "vulkan-intel",
        "lib32-vulkan-intel",
        "intel-media-driver",
        // Proof tools for image/hw/verify-gpu.sh.
        "mesa-utils",
        "vulkan-tools",
        "libva-utils",
        "glmark2",
        "vkmark",
    ] {
        assert!(pkgs.iter().any(|p| p == want), "missing package {want}");
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
