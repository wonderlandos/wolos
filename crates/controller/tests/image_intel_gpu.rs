// SPDX-License-Identifier: GPL-2.0-or-later
//! Intel iGPU stack on first boot: firmware + user-space packages offline, i915 tunables, session env,
//! native-boot proof script. Evidence: docs/modules/intel-graphics.md.
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
