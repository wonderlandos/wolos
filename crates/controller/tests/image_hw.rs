//! Boot wiring for the hardware catalogue: rc.d `wolos_hw`, shipped packages, rc/sysctl/loader defaults.
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};
use wol_core::hardware::{Bus, catalog::CATALOG, lookup};

fn path(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(path(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn header(rc: &str, key: &str) -> Vec<String> {
    rc.lines()
        .find_map(|l| l.strip_prefix(&format!("# {key}:")))
        .map(|v| v.split_whitespace().map(String::from).collect())
        .unwrap_or_default()
}

#[test]
fn wolos_hw_runs_early_and_before_the_drivers_users() {
    let rc = read("image/files/etc/rc.d/wolos_hw");
    assert_eq!(header(&rc, "PROVIDE"), ["wolos_hw"]);
    assert_eq!(header(&rc, "REQUIRE"), ["FILESYSTEMS", "wolos_live"]);
    let before = header(&rc, "BEFORE");
    // devmatch runs before FILESYSTEMS (rcorder on 15.1): its autoloads count as loaded.
    for b in ["netif", "wolos_wifi", "wolos_gpu"] {
        assert!(before.iter().any(|x| x == b), "BEFORE {b}");
    }
    assert!(rc.contains("/usr/local/bin/wol hw boot"));
    // `wol hw boot` is a compat row in cli.rs; the snapshot path is the shared contract.
    assert_eq!(shared::hardware::SNAPSHOT_PATH, "/var/run/wolos-hw.json");
    assert!(read("image/build.sh").contains("\"$STAGE/etc/rc.d/wolos_hw\""));
    for f in [
        "image/files/etc/rc.d/wolos_hw",
        "image/files/usr/local/libexec/wolos/wifi",
        "image/files/usr/local/libexec/wolos/gpu",
        "desktop/overlay/etc/dhclient-exit-hooks",
    ] {
        let st = Command::new("sh").arg("-n").arg(path(f)).status().unwrap();
        assert!(st.success(), "sh -n {f}");
    }
}

#[test]
fn catalogue_keeps_the_drivers_the_boot_scripts_loaded() {
    // Former per-vendor choices in libexec/wolos/{gpu,wifi} (image_gpu/image_wifi before wolos_hw).
    for (vendor, device, class, kld) in [
        (0x8086, 0x46a8, "030000", "i915kms"),
        (0x8086, 0x46a8, "038000", "i915kms"), // passed-through iGPU, display class 0x0380
        (0x1002, 0x15bf, "030000", "amdgpu"),
        (0x10de, 0x2520, "030000", "nvidia-drm"),
        (0x8086, 0x51f0, "028000", "if_iwlwifi"),
        (0x10ec, 0xc822, "028000", "if_rtw88"),
        (0x168c, 0x003e, "028000", "if_ath10k"),
        (0x17cb, 0x1103, "028000", "if_ath11k"),
        (0x14c3, 0x7961, "028000", "if_mt7921"),
    ] {
        let e = lookup(Bus::Pci, Some(vendor), Some(device), &[class]).unwrap();
        assert!(
            e.klds.contains(&kld),
            "{vendor:04x}:{device:04x} -> {:?}",
            e.klds
        );
    }
}

/// Catalogue packages deliberately not in the offline repo.
const NOT_SHIPPED: &[(&str, &str)] = &[
    (
        "nvidia-drm-kmod",
        "~520 MB with nvidia-kmod + nvidia-driver; wolos_firstboot installs it online",
    ),
    (
        "wifi-firmware-rtw88-kmod-rtw8814a",
        "desktop USB/PCIe AC1900 cards only",
    ),
    (
        "nvidia-drm-kmod-580",
        "Maxwell/Pascal/Volta: the 580 branch, online like the 595 one",
    ),
];

fn shipped() -> Vec<String> {
    read("desktop/packages.txt")
        .lines()
        .map(|l| l.split('#').next().unwrap().trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn every_catalogue_package_ships_offline_or_is_listed_as_not_shipped() {
    let pkgs = shipped();
    // Intel GPU generations not listed one by one come with drm-kmod's gpu-firmware-kmod.
    let covered = |p: &str| {
        pkgs.iter().any(|x| x == p)
            || (p.starts_with("gpu-firmware-") && pkgs.iter().any(|x| x == "gpu-firmware-kmod"))
    };
    for e in CATALOG {
        for p in e.firmware.iter().chain(e.packages) {
            let skip = NOT_SHIPPED.iter().any(|(n, _)| n == p);
            assert!(
                covered(p) || skip,
                "{}: {p} neither shipped nor in NOT_SHIPPED",
                e.name
            );
            assert!(
                !(covered(p) && skip),
                "{p} is shipped: drop it from NOT_SHIPPED"
            );
        }
    }
    for want in [
        "cpu-microcode-intel",
        "cpu-microcode-amd",
        "iwmbt-firmware",
        "rtlbt-firmware",
        "bluez-firmware",
        "gpu-firmware-kmod",
        "realtek-re-kmod",
        "webcamd",
        "pipewire-spa-oss",
    ] {
        assert!(pkgs.iter().any(|p| p == want), "{want}");
    }
    assert!(
        !pkgs.iter().any(|p| p.starts_with("nvidia-")),
        "NVIDIA stays an online install"
    );
}

#[test]
fn ntpd_waits_for_the_network_then_refreshes_the_leap_file() {
    // dhclient-script sources /etc/dhclient-exit-hooks with $reason after a lease is applied.
    let dir = std::env::temp_dir().join(format!("wol-ntp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let log = dir.join("log");
    let service = dir.join("service");
    let run = |reason: &str, running: bool| -> String {
        let _ = fs::remove_file(&log);
        fs::write(
            &service,
            format!(
                "#!/bin/sh\necho \"$*\" >> {}\n[ \"$2\" = status ] && exit {}\nexit 0\n",
                log.display(),
                if running { 0 } else { 1 }
            ),
        )
        .unwrap();
        fs::set_permissions(&service, fs::Permissions::from_mode(0o755)).unwrap();
        let hook = path("desktop/overlay/etc/dhclient-exit-hooks");
        let st = Command::new("sh")
            .arg("-c")
            .arg(format!(". {}; wait", hook.display()))
            .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
            .env("reason", reason)
            .status()
            .unwrap();
        assert!(st.success());
        fs::read_to_string(&log).unwrap_or_default()
    };
    assert_eq!(
        run("BOUND", true),
        "ntpd status\nntpd fetch\nntpd restart\n",
        "lease while ntpd runs: refresh leap file, re-resolve pool"
    );
    assert_eq!(
        run("REBOOT", true),
        "ntpd status\nntpd fetch\nntpd restart\n"
    );
    assert_eq!(
        run("BOUND", false),
        "ntpd status\n",
        "boot: ntpd starts after netif itself"
    );
    assert_eq!(run("RENEW", true), "", "renewals change nothing");
    assert_eq!(run("EXPIRE", true), "");
}

/// A new lease also retries an online-only driver (NVIDIA) missed at first boot: in the background,
/// one run at a time (lockf -t 0: a second lease while it runs does nothing), never on renewals.
#[test]
fn network_up_retries_a_pending_driver_in_the_background() {
    let dir = std::env::temp_dir().join(format!("wol-hwnet-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let log = dir.join("log");
    let lockf = dir.join("lockf");
    fs::write(
        &lockf,
        format!("#!/bin/sh\necho \"$*\" >> {}\n", log.display()),
    )
    .unwrap();
    fs::set_permissions(&lockf, fs::Permissions::from_mode(0o755)).unwrap();
    let run = |reason: &str| -> String {
        let _ = fs::remove_file(&log);
        let hook = path("desktop/overlay/etc/dhclient-exit-hooks");
        let st = Command::new("sh")
            .arg("-c")
            .arg(format!(". {}; wait", hook.display()))
            .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
            .env("reason", reason)
            .status()
            .unwrap();
        assert!(st.success());
        fs::read_to_string(&log).unwrap_or_default()
    };
    let want = "-s -t 0 /var/run/wolos-hw-online.lock /usr/local/bin/wol hw online\n";
    assert_eq!(run("BOUND"), want);
    assert_eq!(run("REBOOT"), want);
    assert_eq!(run("RENEW"), "");
    assert_eq!(run("EXPIRE"), "");
    let _ = fs::remove_dir_all(&dir);
}

/// `key=value` lines; every setting carries its reason (inline `#` or the comment right above).
fn settings(p: &str) -> Vec<(String, String)> {
    let mut v = Vec::new();
    let mut why_above = false;
    for l in read(p).lines().map(str::trim) {
        if l.is_empty() {
            why_above = false;
            continue;
        }
        if l.starts_with('#') {
            why_above = l.len() > 12;
            continue;
        }
        let (kv, why) = l.split_once('#').unwrap_or((l, ""));
        assert!(
            why_above || why.trim().len() > 10,
            "{p}: no reason for {l:?}"
        );
        let (k, val) = kv.trim().split_once('=').unwrap_or_else(|| panic!("{l}"));
        v.push((k.trim().to_owned(), val.trim().trim_matches('"').to_owned()));
        why_above = false;
    }
    v
}

#[test]
fn desktop_tunables_are_few_known_and_explained() {
    let sysctl = settings("desktop/overlay/etc/sysctl.conf.local");
    let loader = settings("image/files/boot/loader.conf.d/wolos-desktop.conf");
    for (k, v) in [
        ("kern.coredump", "0"),
        ("kern.sched.ule.preempt_thresh", "224"),
        ("vfs.usermount", "1"),
        ("kern.vt.kbd_debug", "0"),
    ] {
        assert!(sysctl.contains(&(k.into(), v.into())), "sysctl {k}={v}");
    }
    assert!(loader.contains(&("cuse_load".into(), "YES".into())));
    // 15.1 defaults already: no duplicates (checked on the test VM, 2026-10-27).
    for k in [
        "kern.ipc.shm_allow_removed",
        "kern.vt.enable_bell",
        "hw.usb.usbhid.enable",
        "kern.sched.preempt_thresh",
    ] {
        assert!(!sysctl.iter().chain(&loader).any(|(x, _)| x == k), "{k}");
    }
    // Set once, in build.sh's loader.conf.wolos.
    for k in ["autoboot_delay", "hw.usb.no_boot_wait"] {
        assert!(!loader.iter().any(|(x, _)| x == k), "{k}");
    }
}

#[test]
fn webcams_start_from_devd_for_the_video_group() {
    // webcamd's devd rule calls its rc.d on attach; it starts only with webcamd_enable.
    let rc = settings("desktop/overlay/etc/rc.conf.d/webcamd");
    assert!(rc.contains(&("webcamd_enable".into(), "YES".into())));
    // /dev/video* for group video (base gid 44), which WolOS accounts join, not webcamd.
    assert!(rc.contains(&("webcamd_group".into(), "video".into())));
    let e = lookup(Bus::Usb, Some(0x04f2), Some(0xb6dd), &["0e0100"]).unwrap();
    assert!(
        e.notes.iter().any(|n| n.contains("group video")),
        "{:?}",
        e.notes
    );
}

#[test]
fn only_wol_hw_boot_decides_powerd() {
    // rc.conf.d/powerd from `wol hw boot` is the decision; a static powerd_enable=YES in rc.conf
    // would start it where cpufreq is absent if wolos_hw ever failed.
    for f in ["image/rcconf.sh", "image/installerconfig"] {
        assert!(!read(f).contains("powerd_enable"), "{f} sets powerd_enable");
    }
    // Drivers come from the catalogue (wolos_hw), not from wolos_gpu or firstboot.
    let fb = read("image/files/usr/local/libexec/wolos/firstboot");
    assert!(
        !fb.contains("wolos_gpu rc.d loads"),
        "stale firstboot comment"
    );
}
