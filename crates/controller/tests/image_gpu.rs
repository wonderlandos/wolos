//! image/files/usr/local/libexec/wolos/gpu: panel backlight after the KMS driver is up. The driver itself is
//! loaded by rc.d wolos_hw from the hardware catalogue (tests/image_hw.rs keeps the per-vendor choice).
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../image/files/usr/local/libexec/wolos/gpu")
}

fn run(vendors: &[&str]) -> String {
    run_bl(vendors, &[])
}

/// `bl`: (backlight device name, brightness the fake `backlight` reports).
fn run_bl(vendors: &[&str], bl: &[(&str, u32)]) -> String {
    let bin = &std::env::temp_dir().join(format!(
        "wol-gpu-{}-{}{}",
        std::process::id(),
        vendors.join("-").replace(':', "_"),
        if bl.is_empty() { "" } else { "-bl" }
    ));
    let _ = fs::remove_dir_all(bin);
    fs::create_dir_all(bin).unwrap();
    let pci: String = vendors
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let (drv, class, v) = match v.split(':').collect::<Vec<_>>()[..] {
                [d, c, v] => (d, c, v),
                _ => ("vgapci", "030000", *v),
            };
            format!("{drv}{i}@pci0:0:{i}:0:\tclass=0x{class} rev=0x0c hdr=0x00 vendor=0x{v} device=0x46a8 subvendor=0x17aa subdevice=0x3802\n")
        })
        .collect();
    let log = bin.join("log");
    let dev = bin.join("backlight.d");
    fs::create_dir_all(&dev).unwrap();
    let mut levels = String::new();
    for (name, level) in bl {
        fs::write(dev.join(name), "").unwrap();
        levels += &format!("*/{name}) echo 'brightness: {level}' ;;\n");
    }
    for (name, body) in [
        ("pciconf", format!("#!/bin/sh\nprintf '%s' '{pci}'\n")),
        (
            "kldload",
            format!("#!/bin/sh\necho kldload \"$@\" >> {}\n", log.display()),
        ),
        ("logger", "#!/bin/sh\n:\n".to_string()),
        (
            "backlight",
            format!(
                "#!/bin/sh\nif [ $# -gt 2 ]; then echo backlight \"$@\" >> {}; exit 0; fi\ncase $2 in\n{levels}esac\n",
                log.display()
            ),
        ),
    ] {
        let p = bin.join(name);
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let out = Command::new("sh")
        .arg(script())
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("WOLOS_BACKLIGHT_DIR", &dev)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    fs::read_to_string(&log).unwrap_or_default()
}

#[test]
fn driver_loading_moved_to_wolos_hw() {
    for vendors in [
        &["8086"][..],
        &["1002"][..],
        &["8086", "1002"][..],
        &["none:038000:8086"][..],
    ] {
        assert_eq!(run(vendors), "", "{vendors:?}");
    }
    let rc = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../image/files/etc/rc.d/wolos_gpu"),
    )
    .unwrap();
    assert!(
        rc.contains("REQUIRE: FILESYSTEMS wolos_hw"),
        "KMS driver first"
    );
}

#[test]
fn dark_panel_backlight_is_raised_once_the_driver_loads() {
    // VFIO passthrough (and some firmware) hands over the eDP backlight at 0: the desktop runs on a black panel.
    let out = run_bl(&["8086"], &[("backlight0", 0), ("backlight1", 40)]);
    let dir = std::env::temp_dir().join(format!("wol-gpu-{}-8086-bl", std::process::id()));
    assert_eq!(
        out,
        format!("backlight -f {}/backlight.d/backlight0 80\n", dir.display())
    );
}

#[test]
fn console_uses_the_panel_native_mode() {
    // Live: a 1366x768 laptop panel logged "User-defined mode not supported: 1920x1080" every boot.
    let b = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../image/build.sh"),
    )
    .unwrap();
    assert!(!b.contains("kern.vt.fb.default_mode"));
}
