//! image/files/usr/local/libexec/wolos/wifi: wlan0 for the shell (wpa_cli -i wlan0) every boot. The driver is
//! loaded before it by rc.d wolos_hw from the hardware catalogue (tests/image_hw.rs).
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../image/files/usr/local/libexec/wolos/wifi")
}

struct Run {
    log: String,
    conf: Option<String>,
}

/// `nics`: (vendor, device) of network-class 0x0280 PCI devices; `wlan_devs`: sysctl net.wlan.devices;
/// `existing`: wlan0 already present; `conf`: pre-existing wpa_supplicant.conf.
fn run(
    name: &str,
    nics: &[(&str, &str)],
    wlan_devs: &str,
    existing: bool,
    conf: Option<&str>,
) -> Run {
    run_wait(name, nics, wlan_devs, existing, conf, "0")
}

fn run_wait(
    name: &str,
    nics: &[(&str, &str)],
    wlan_devs: &str,
    existing: bool,
    conf: Option<&str>,
    wait: &str,
) -> Run {
    let bin = &std::env::temp_dir().join(format!("wol-wifi-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(bin);
    fs::create_dir_all(bin).unwrap();
    let mut pci = String::from(
        "vgapci0@pci0:0:2:0:\tclass=0x030000 rev=0x0c hdr=0x00 vendor=0x8086 device=0x46a8 subvendor=0x17aa subdevice=0x3802\n",
    );
    for (i, (v, d)) in nics.iter().enumerate() {
        pci += &format!(
            "none{i}@pci0:0:20:{i}:\tclass=0x028000 rev=0x01 hdr=0x00 vendor=0x{v} device=0x{d} subvendor=0x8086 subdevice=0x0094\n"
        );
    }
    let log = bin.join("log");
    let cfg = bin.join("wpa_supplicant.conf");
    if let Some(c) = conf {
        fs::write(&cfg, c).unwrap();
    }
    let l = log.display();
    for (n, body) in [
        ("pciconf", format!("#!/bin/sh\nprintf '%s' '{pci}'\n")),
        (
            "kldload",
            format!("#!/bin/sh\necho kldload \"$@\" >> {l}\n"),
        ),
        ("logger", "#!/bin/sh\n:\n".into()),
        (
            "sysctl",
            format!("#!/bin/sh\n[ \"$*\" = '-n net.wlan.devices' ] && echo '{wlan_devs}'\n"),
        ),
        (
            "ifconfig",
            format!(
                "#!/bin/sh\nif [ \"$1\" = wlan0 ] && [ $# -eq 1 ]; then {}; fi\necho ifconfig \"$@\" >> {l}\n",
                if existing { "exit 0" } else { "exit 1" }
            ),
        ),
        (
            "sysrc",
            format!("#!/bin/sh\n[ \"$1\" = -n ] && exit 1\necho sysrc \"$@\" >> {l}\n"),
        ),
    ] {
        let p = bin.join(n);
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let out = Command::new("sh")
        .arg(script())
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("WOLOS_WPA_CONF", &cfg)
        .env("WOLOS_WIFI_WAIT", wait)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Run {
        log: fs::read_to_string(&log).unwrap_or_default(),
        conf: fs::read_to_string(&cfg).ok(),
    }
}

/// LinuxKPI drivers (iwlwifi) stay down under netif's WPA path: wpa_supplicant never scans until wlan0 is UP.
const NET: &str = "sysrc -q ifconfig_wlan0=WPA DHCP\nifconfig wlan0 up\n";

#[test]
fn creates_wlan0_on_the_first_wireless_device_each_boot() {
    for (name, nics, devs, want) in [
        // Alder Lake-P CNVi 8086:51f0 (AX201/AX211): iwlwifi, loaded by wolos_hw.
        (
            "adl",
            &[("8086", "51f0")][..],
            "iwlwifi0",
            "ifconfig wlan0 create wlandev iwlwifi0\n",
        ),
        (
            "rtw",
            &[("10ec", "c822")][..],
            "rtw880",
            "ifconfig wlan0 create wlandev rtw880\n",
        ),
        (
            "mt",
            &[("14c3", "7961")][..],
            "mt76_70",
            "ifconfig wlan0 create wlandev mt76_70\n",
        ),
        // First device wins: one wlan0, the name the Control Center and Setup Assistant use.
        (
            "two",
            &[("8086", "51f0")][..],
            "iwlwifi0 rtw880",
            "ifconfig wlan0 create wlandev iwlwifi0\n",
        ),
        // USB or devmatch-loaded device with no PCI Wi-Fi function: still gets wlan0.
        (
            "auto",
            &[][..],
            "iwm0",
            "ifconfig wlan0 create wlandev iwm0\n",
        ),
    ] {
        let r = run(name, nics, devs, false, None);
        assert_eq!(r.log, format!("{want}{NET}"), "{name}");
    }
}

#[test]
fn drivers_are_not_loaded_here_and_no_nic_means_no_wait() {
    // wolos_hw (`wol hw boot`) owns driver loading; this script only waits for the device.
    let body = fs::read_to_string(script()).unwrap();
    assert!(
        !body.contains("kldload"),
        "driver selection lives in the hardware catalogue"
    );
    // No wireless PCI function: no 10 s wait for a device that cannot appear.
    let t = std::time::Instant::now();
    let r = run_wait("nowait", &[], "", false, None, "10");
    assert_eq!(r.log, "");
    assert!(
        t.elapsed() < std::time::Duration::from_secs(3),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn no_wireless_device_changes_nothing() {
    let r = run("none", &[], "", false, None);
    assert_eq!(r.log, "");
    assert_eq!(r.conf, None);
}

#[test]
fn existing_wlan0_is_not_recreated() {
    let r = run("exists", &[("8086", "51f0")], "iwlwifi0", true, None);
    assert_eq!(r.log, NET);
}

#[test]
fn wpa_conf_header_matches_what_the_shell_writes() {
    // wol_core::wifi upserts networks into this file and wpa_cli talks to its ctrl_interface.
    let r = run("conf", &[("8086", "51f0")], "iwlwifi0", false, None);
    let core = wol_core::wifi::upsert_network(None, "x", None);
    let header = r.conf.expect("conf created");
    assert!(core.starts_with(&header), "{header:?} vs {core:?}");
    assert_eq!(wol_core::wifi::CONF_PATH, "/etc/wpa_supplicant.conf");
    // A user conf is never replaced.
    let r = run(
        "keep",
        &[("8086", "51f0")],
        "iwlwifi0",
        false,
        Some("network={\n}\n"),
    );
    assert_eq!(r.conf.as_deref(), Some("network={\n}\n"));
}

fn read(p: &str) -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(p),
    )
    .unwrap()
}

#[test]
fn offline_repo_ships_wifi_firmware_and_rc_runs_before_netif() {
    let pkgs: Vec<String> = read("desktop/packages.txt")
        .lines()
        .map(|l| l.split('#').next().unwrap().trim().to_owned())
        .collect();
    for want in [
        "wifi-firmware-iwlwifi-kmod-ax210", // 8086:51f0 per fwget pci_network_intel
        "wifi-firmware-iwlwifi-kmod-22000",
        "wifi-firmware-iwlwifi-kmod-9000",
        "wifi-firmware-iwlwifi-kmod-bz",
        "wifi-firmware-rtw88-kmod-rtw8822c",
        "wifi-firmware-rtw89-kmod-rtw8852b",
        "wifi-firmware-mt76-kmod-mt7921", // if_mt7921 is built by kernel/build.sh
    ] {
        assert!(pkgs.iter().any(|p| p == want), "{want}");
    }
    // No firmware for cards without a working 15.1 driver: it would only promise Wi-Fi.
    for gone in ["-ath10k-", "-ath11k-", "-ath12k-", "-mt7925"] {
        assert!(!pkgs.iter().any(|p| p.contains(gone)), "{gone} shipped");
    }
    let rc = read("image/files/etc/rc.d/wolos_wifi");
    assert!(rc.contains("BEFORE: netif") && rc.contains("libexec/wolos/wifi"));
    assert!(
        rc.contains("REQUIRE: FILESYSTEMS wolos_hw"),
        "drivers first"
    );
    assert!(read("image/build.sh").contains("\"$STAGE/etc/rc.d/wolos_wifi\""));
}
