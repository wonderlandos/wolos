use super::fixtures::*;
use super::*;
use crate::system::fake::FakeHost;

const USB_ARGV: &str = "usbconfig dump_device_desc dump_curr_config_desc show_ifdrv";

/// (bus, vendor, device, classes) -> (name, klds, firmware).
type Case = (Bus, Option<u16>, Option<u16>, &'static [&'static str], Want);
type Want = Option<(
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
)>;

#[test]
fn catalogue_lookup_table() {
    use Bus::*;
    #[rustfmt::skip]
    let cases: &[Case] = &[
        // GPU: fwget pci_video_intel generations; exact ids beat ranges (Broxton 5a84 vs Cannon Lake 5a*).
        (Pci, Some(0x8086), Some(0x46a8), &["030000"], Some(("Intel Iris Xe (Alder/Raptor Lake)", &["i915kms"], &["gpu-firmware-intel-kmod-alderlake", "gpu-firmware-intel-kmod-tigerlake"]))),
        (Pci, Some(0x8086), Some(0x46a8), &["038000"], Some(("Intel Iris Xe (Alder/Raptor Lake)", &["i915kms"], &["gpu-firmware-intel-kmod-alderlake", "gpu-firmware-intel-kmod-tigerlake"]))),
        (Pci, Some(0x8086), Some(0xa7a0), &["030000"], Some(("Intel Iris Xe (Alder/Raptor Lake)", &["i915kms"], &["gpu-firmware-intel-kmod-alderlake", "gpu-firmware-intel-kmod-tigerlake"]))),
        (Pci, Some(0x8086), Some(0x5a84), &["030000"], Some(("Intel HD Graphics (Broxton)", &["i915kms"], &["gpu-firmware-intel-kmod-broxton"]))),
        (Pci, Some(0x8086), Some(0x5a40), &["030000"], Some(("Intel Graphics (Cannon Lake)", &["i915kms"], &["gpu-firmware-intel-kmod-cannonlake"]))),
        (Pci, Some(0x8086), Some(0x3ea0), &["030000"], Some(("Intel UHD Graphics (Kaby/Coffee/Comet Lake)", &["i915kms"], &["gpu-firmware-intel-kmod-kabylake"]))),
        (Pci, Some(0x8086), Some(0x7d55), &["030000"], Some(("Intel Arc Graphics (Meteor/Arrow Lake)", &["i915kms"], &["gpu-firmware-intel-kmod-meteorlake"]))),
        (Pci, Some(0x8086), Some(0x0412), &["030000"], Some(("Intel graphics", &["i915kms"], &[]))),
        (Pci, Some(0x8086), Some(0x64a0), &["030000"], Some(("Intel Xe2 graphics (Lunar Lake, Battlemage)", &[], &[]))),
        (Pci, Some(0x1002), Some(0x15bf), &["030000"], Some(("AMD Radeon", &["amdgpu"], &["gpu-firmware-kmod"]))),
        (Pci, Some(0x1002), Some(0x67df), &["030000"], Some(("AMD Radeon", &["amdgpu"], &["gpu-firmware-kmod"]))),
        (Pci, Some(0x1002), Some(0x98e4), &["030000"], Some(("AMD Radeon", &["amdgpu"], &["gpu-firmware-kmod"]))),
        (Pci, Some(0x1002), Some(0x6798), &["030000"], Some(("AMD Radeon (radeonkms)", &["radeonkms"], &["gpu-firmware-kmod"]))),
        (Pci, Some(0x1002), Some(0x68b8), &["030000"], Some(("AMD Radeon (radeonkms)", &["radeonkms"], &["gpu-firmware-kmod"]))),
        // NVIDIA: 595 for Turing and newer; 580 (last for Maxwell/Pascal/Volta); Kepler has no KMS driver.
        (Pci, Some(0x10de), Some(0x2520), &["030000"], Some(("NVIDIA GeForce", &["nvidia-modeset", "nvidia-drm"], &[]))),
        (Pci, Some(0x10de), Some(0x1e07), &["030000"], Some(("NVIDIA GeForce", &["nvidia-modeset", "nvidia-drm"], &[]))),
        (Pci, Some(0x10de), Some(0x1c8d), &["030200"], Some(("NVIDIA GeForce (Maxwell/Pascal/Volta)", &["nvidia-modeset", "nvidia-drm"], &[]))),
        (Pci, Some(0x10de), Some(0x1340), &["030000"], Some(("NVIDIA GeForce (Maxwell/Pascal/Volta)", &["nvidia-modeset", "nvidia-drm"], &[]))),
        (Pci, Some(0x10de), Some(0x1180), &["030000"], Some(("NVIDIA GeForce (Kepler or older)", &[], &[]))),
        (Pci, Some(0x1234), Some(0x1111), &["030000"], Some(("QEMU standard VGA", &[], &[]))),
        // Wi-Fi: fwget pci_network_* (15.1).
        (Pci, Some(0x8086), Some(0x51f0), &["028000"], Some(("Intel Wi-Fi 6E AX210/AX211", &["if_iwlwifi"], &["wifi-firmware-iwlwifi-kmod-ax210"]))),
        (Pci, Some(0x8086), Some(0xa0f0), &["028000"], Some(("Intel Wi-Fi 6 AX200/AX201", &["if_iwlwifi"], &["wifi-firmware-iwlwifi-kmod-22000"]))),
        (Pci, Some(0x8086), Some(0x24fd), &["028000"], Some(("Intel Wireless 8260/8265", &["if_iwlwifi"], &["wifi-firmware-iwlwifi-kmod-8000"]))),
        (Pci, Some(0x8086), Some(0x08b1), &["028000"], Some(("Intel Wireless 7260/3160/7265/3165/3168", &["if_iwlwifi"], &["wifi-firmware-iwlwifi-kmod-7000"]))),
        (Pci, Some(0x8086), Some(0x9df0), &["028000"], Some(("Intel Wireless-AC 9260/9461/9462/9560", &["if_iwlwifi"], &["wifi-firmware-iwlwifi-kmod-9000"]))),
        (Pci, Some(0x8086), Some(0x272b), &["028000"], Some(("Intel Wi-Fi 7 BE200/BE202", &["if_iwlwifi"], &["wifi-firmware-iwlwifi-kmod-bz"]))),
        (Pci, Some(0x8086), Some(0xe440), &["028000"], Some(("Intel Wi-Fi 7 (Lunar/Panther Lake)", &["if_iwlwifi"], &["wifi-firmware-iwlwifi-kmod-sc"]))),
        (Pci, Some(0x8086), Some(0x1234), &["028000"], Some(("Intel Wi-Fi", &["if_iwlwifi"], &[]))),
        (Pci, Some(0x10ec), Some(0xc822), &["028000"], Some(("Realtek RTL8822CE", &["if_rtw88"], &["wifi-firmware-rtw88-kmod-rtw8822c"]))),
        (Pci, Some(0x10ec), Some(0xb852), &["028000"], Some(("Realtek RTL8852BE", &["if_rtw89"], &["wifi-firmware-rtw89-kmod-rtw8852b"]))),
        (Pci, Some(0x10ec), Some(0x8176), &["028000"], Some(("Realtek RTL8188CE/8192CE/8188EE", &["if_rtwn_pci"], &[]))),
        (Pci, Some(0x10ec), Some(0xabcd), &["028000"], Some(("Realtek Wi-Fi", &["if_rtw88", "if_rtw89"], &[]))),
        (Pci, Some(0x168c), Some(0x003e), &["028000"], Some(("Qualcomm QCA6174 (Killer 1435/1535)", &["if_ath10k"], &[]))),
        (Pci, Some(0x168c), Some(0x0032), &["028000"], Some(("Atheros 802.11n", &["if_ath"], &[]))),
        (Pci, Some(0x17cb), Some(0x1103), &["028000"], Some(("Qualcomm WCN6855 (QCNFA765)", &["if_ath11k"], &[]))),
        (Pci, Some(0x14c3), Some(0x7961), &["028000"], Some(("MediaTek MT7921/MT7922", &["if_mt7921"], &["wifi-firmware-mt76-kmod-mt7921"]))),
        (Pci, Some(0x14c3), Some(0x0717), &["028000"], Some(("MediaTek MT7925", &["if_mt7925"], &[]))),
        (Pci, Some(0x14e4), Some(0x43a0), &["028000"], Some(("Broadcom Wi-Fi", &[], &[]))),
        // Ethernet: RTL8125 needs the vendor driver; RTL8168 and Intel are in the kernel.
        (Pci, Some(0x10ec), Some(0x8125), &["020000"], Some(("Realtek RTL8125/8126/8127 2.5/5/10GbE", &[], &[]))),
        (Pci, Some(0x10ec), Some(0x8168), &["020000"], Some(("Realtek RTL8111/8168/8101 Ethernet", &["if_re"], &[]))),
        (Pci, Some(0x8086), Some(0x15f3), &["020000"], Some(("Intel Ethernet I225/I226", &["if_igc"], &[]))),
        (Pci, Some(0x8086), Some(0x15bb), &["020000"], Some(("Intel Ethernet", &["if_em"], &[]))),
        (Pci, Some(0x1af4), Some(0x1000), &["020000"], None),
        // Bluetooth (USB).
        (Usb, Some(0x8087), Some(0x0026), &["e00101"], Some(("Intel Bluetooth", &["ng_ubt"], &["iwmbt-firmware"]))),
        (Usb, Some(0x0bda), Some(0xb85c), &["ef0201", "e00101"], Some(("Realtek Bluetooth", &["ng_ubt"], &["rtlbt-firmware"]))),
        (Usb, Some(0x0bda), Some(0x8179), &["000000", "ffffff"], None),
        (Usb, Some(0x0a5c), Some(0x2033), &["ffffff"], Some(("Broadcom BCM2033 Bluetooth", &["ubtbcmfw", "ng_ubt"], &["bluez-firmware"]))),
        (Usb, Some(0x13d3), Some(0x3567), &["e00101"], Some(("Bluetooth", &["ng_ubt"], &[]))),
        // Camera before USB audio: a camera with a microphone is a camera.
        (Usb, Some(0x04f2), Some(0xb6dd), &["ef0201", "0e0100", "010100"], Some(("USB camera", &["cuse"], &[]))),
        (Usb, Some(0x046d), Some(0x0a44), &["000000", "010100"], Some(("USB audio", &["snd_uaudio"], &[]))),
        // Audio.
        (Pci, Some(0x8086), Some(0x51c8), &["040380"], Some(("Intel Smart Sound / SOF audio DSP", &[], &[]))),
        (Pci, Some(0x8086), Some(0xa0c8), &["040300"], Some(("HD Audio", &["snd_hda"], &[]))),
        (Pci, Some(0x1002), Some(0x1640), &["040300"], Some(("HD Audio", &["snd_hda"], &[]))),
        (Pci, Some(0x1022), Some(0x15e2), &["048000"], Some(("AMD audio coprocessor (ACP)", &[], &[]))),
        // Input, card readers.
        (Acpi, None, None, &["ELAN0678", "PNP0C50"], Some(("I2C HID touchpad/touchscreen", &["ig4", "iichid", "hmt"], &[]))),
        (Acpi, None, None, &["AMDI0010"], Some(("AMD I2C controller", &["ig4"], &[]))),
        (Acpi, None, None, &["PNP0A08", "PNP0A03"], None),
        (Pci, Some(0x8086), Some(0x51e8), &["0c8000"], Some(("Intel Serial IO I2C", &["ig4"], &[]))),
        (Pci, Some(0x10ec), Some(0x525a), &["ff0000"], Some(("Realtek PCIe card reader", &["rtsx", "mmcsd"], &[]))),
        (Pci, Some(0x1217), Some(0x8621), &["080501"], Some(("SD card reader", &["sdhci_pci", "mmcsd"], &[]))),
        // CPU, backlight.
        (Cpu, Some(0x8086), None, &[], Some(("Intel processor", &["coretemp", "cpuctl"], &["cpu-microcode-intel"]))),
        (Cpu, Some(0x1022), None, &[], Some(("AMD processor", &["amdtemp", "cpuctl"], &["cpu-microcode-amd"]))),
        (Cpu, None, None, &[], None),
        (Backlight, None, None, &[], Some(("Display backlight", &[], &[]))),
    ];
    for (bus, v, d, classes, want) in cases {
        let got = lookup(*bus, *v, *d, classes).map(|e| (e.name, e.klds, e.firmware));
        assert_eq!(got, *want, "{bus:?} {v:04x?}:{d:04x?} {classes:?}");
    }
}

#[test]
fn catalogue_is_well_formed() {
    let word = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-.".contains(&b))
    };
    for e in catalog::CATALOG {
        assert!(!e.name.is_empty(), "{e:?}");
        for x in e
            .klds
            .iter()
            .chain(e.firmware)
            .chain(e.packages)
            .chain(e.drivers)
        {
            assert!(word(x), "{}: {x:?}", e.name);
        }
        for (lo, hi) in e.devices {
            assert!(lo <= hi, "{}", e.name);
        }
        for l in e.loader {
            let (k, v) = l.split_once('=').unwrap_or_default();
            assert!(
                word(k) && v.len() > 2 && v.starts_with('"') && v.ends_with('"'),
                "{l}"
            );
        }
        let does_something = !e.klds.is_empty() || !e.packages.is_empty() || !e.drivers.is_empty();
        assert!(
            !e.supported || does_something || e.bus == Bus::Backlight,
            "{}: supported but no driver",
            e.name
        );
        assert!(
            e.supported || !e.notes.is_empty(),
            "{}: say why unsupported",
            e.name
        );
    }
    assert!(is_catalogue_kld("if_iwlwifi") && is_catalogue_kld("i915kms"));
    assert!(!is_catalogue_kld("evil") && !is_catalogue_kld("if_iwlwifi; rm"));
}

/// The ADL laptop, as root: pciconf/devinfo/usbconfig/kldstat fixtures, some packages missing.
fn adl_host(installed: &str) -> FakeHost {
    let h = FakeHost::new(true);
    h.set_root(true);
    h.command("pciconf -lv", PCICONF_ADL)
        .command("devinfo -v", DEVINFO_ADL)
        .command(USB_ARGV, USBCONFIG_LAPTOP)
        .command("kldstat -v", KLDSTAT)
        .command("pkg query %n", installed)
        .string("hw.model", "12th Gen Intel(R) Core(TM) i5-1235U")
        .int("dev.cpu.0.freq", 1300);
    for k in [
        "coretemp",
        "cpuctl",
        "ig4",
        "iichid",
        "hmt",
        "mmcsd",
        "ng_ubt",
        "cuse",
        "if_iwlwifi",
        "rtsx",
        "zfs",
    ] {
        h.file(&format!("/boot/kernel/{k}.ko"), "");
    }
    h.file("/boot/modules/i915kms.ko", "")
        .file("/boot/firmware/intel-ucode.bin", "")
        .file("/dev/backlight/backlight0", "");
    h
}

const ADL_PKGS: &str = "drm-kmod\ngpu-firmware-intel-kmod-alderlake\ngpu-firmware-intel-kmod-tigerlake\n\
wifi-firmware-iwlwifi-kmod-ax210\ncpu-microcode-intel\n";

fn summary(s: &HardwareSnapshot) -> Vec<String> {
    s.devices
        .iter()
        .map(|d| {
            format!(
                "{} {} | {} | {} | {}",
                d.kind.name(),
                d.name,
                d.driver.as_deref().unwrap_or("-"),
                d.state.name(),
                d.hint
            )
        })
        .collect()
}

#[test]
fn adl_laptop_snapshot() {
    let s = read(&adl_host(ADL_PKGS));
    assert_eq!(
        summary(&s),
        vec![
            "cpu 12th Gen Intel(R) Core(TM) i5-1235U | hwpstate_intel0 | needs-driver | load coretemp cpuctl: wol hw fix",
            "gpu Intel Iris Xe (Alder/Raptor Lake) | drmn0 | ready | ",
            "wifi Intel Wi-Fi 6E AX210/AX211 | iwlwifi0 | ready | ",
            "ethernet Realtek RTL8111/8168/8101 Ethernet | re0 | ready | ",
            "bluetooth Intel Bluetooth | ubt0 | needs-firmware | install iwmbt-firmware: wol hw fix --yes",
            "audio Intel Smart Sound / SOF audio DSP | - | unsupported | no FreeBSD driver for the SOF DSP: pick HD Audio in the firmware setup if offered; USB/Bluetooth audio work",
            "webcam Chicony Electronics Co.,Ltd. Integrated Camera | - | needs-driver | install webcamd: wol hw fix --yes",
            "input I2C HID touchpad/touchscreen | - | needs-driver | load iichid hmt: wol hw fix",
            "input Alder Lake PCH Serial IO I2C Controller #0 | ig4iic0 | ready | ",
            "card-reader Realtek PCIe card reader | rtsx0 | needs-driver | load mmcsd: wol hw fix",
            "backlight Display backlight | - | ready | ",
        ]
    );
    let cpu = &s.devices[0];
    assert_eq!((cpu.bus.as_str(), cpu.id.as_str()), ("cpu", "GenuineIntel"));
    assert_eq!(
        cpu.loader,
        vec![
            "cpu_microcode_load=\"YES\"",
            "cpu_microcode_name=\"/boot/firmware/intel-ucode.bin\""
        ]
    );
    assert_eq!(cpu.rc, vec!["powerd_enable=\"NO\""]);
    assert!(cpu.notes.iter().any(|n| n.starts_with("Speed Shift")));
    let wifi = &s.devices[2];
    assert_eq!(
        (wifi.id.as_str(), wifi.location.as_str(), wifi.bus.as_str()),
        ("8086:51f0", "pci0:0:20:3", "pci")
    );
    let tp = &s.devices[7];
    assert_eq!(
        (tp.bus.as_str(), tp.id.as_str()),
        ("acpi", "ELAN0678,PNP0C50")
    );
    // Only the listed problems.
    assert_eq!(s.problems().count(), 6);
    assert_eq!(
        fixes(&s),
        (
            vec![
                "coretemp".into(),
                "cpuctl".into(),
                "cuse".into(),
                "iichid".into(),
                "hmt".into(),
                "mmcsd".into()
            ],
            vec!["iwmbt-firmware".into(), "webcamd".into()]
        )
    );
}

#[test]
fn test_vm_snapshot_virtual_gpu_and_no_cpufreq() {
    // The real test VM: QEMU std VGA, virtio NIC, host CPU model without cpufreq.
    let h = FakeHost::new(true);
    h.command(
        "pciconf -lv",
        "vgapci0@pci0:0:1:0:     class=0x030000 rev=0x02 hdr=0x00 vendor=0x1234 device=0x1111 subvendor=0x1af4 subdevice=0x1100\n    class      = display\n    subclass   = VGA\nvirtio_pci0@pci0:0:2:0: class=0x020000 rev=0x00 hdr=0x00 vendor=0x1af4 device=0x1000 subvendor=0x1af4 subdevice=0x0001\n    vendor     = 'Red Hat, Inc.'\n    device     = 'Virtio network device'\n",
    )
    .command("kldstat -v", KLDSTAT)
    .command("pkg query %n", "")
    .string("hw.model", "12th Gen Intel(R) Core(TM) i5-1235U");
    h.file("/boot/kernel/coretemp.ko", "")
        .file("/boot/kernel/cpuctl.ko", "");
    let s = read(&h);
    assert_eq!(
        summary(&s),
        vec![
            "cpu 12th Gen Intel(R) Core(TM) i5-1235U | - | needs-firmware | install cpu-microcode-intel: wol hw fix --yes",
            "gpu QEMU standard VGA | vgapci0 | unsupported | virtual GPU: no DRM driver on FreeBSD, framebuffer only (no compositor)",
            "ethernet Virtio network device | virtio_pci0 | ready | ",
        ]
    );
    let cpu = &s.devices[0];
    assert_eq!(cpu.rc, vec!["powerd_enable=\"NO\""]);
    assert!(
        cpu.loader.is_empty(),
        "no microcode package: no loader lines"
    );
    assert!(cpu.notes[0].starts_with("no cpufreq(4)"));
}

#[test]
fn status_rules() {
    let set = |v: &[&str]| {
        Some(
            v.iter()
                .map(|s| (*s).to_owned())
                .collect::<HashSet<String>>(),
        )
    };
    let pci = |vendor, device, class| {
        let e = lookup(Bus::Pci, Some(vendor), Some(device), &[class]);
        (
            e,
            Device {
                name: "x".into(),
                ..Default::default()
            },
        )
    };
    let ctx = |loaded: &[&str], disk: &[&str], pkgs: &[&str], files: &[&str]| Ctx {
        loaded: set(loaded),
        available: set(disk),
        installed: set(pkgs),
        files: set(files),
    };
    // NVIDIA, driver package missing: install it; no modeset line yet.
    let (e, d) = pci(0x10de, 0x2520, "030000");
    let r = resolve(e, d, &["vgapci0"], &ctx(&["kernel"], &["zfs"], &[], &[]));
    assert_eq!(
        (r.state, r.hint.as_str()),
        (
            State::NeedsDriver,
            "install nvidia-drm-kmod: wol hw fix --yes"
        )
    );
    assert!(r.loader.is_empty());
    // ... installed, module not loaded yet: load it; modeset for the next boot.
    let (e, d) = pci(0x10de, 0x2520, "030000");
    let r = resolve(
        e,
        d,
        &["vgapci0"],
        &ctx(
            &["kernel"],
            &["nvidia", "nvidia-modeset", "nvidia-drm"],
            &["nvidia-drm-kmod"],
            &[],
        ),
    );
    assert_eq!(
        (r.state, r.missing_klds.clone()),
        (
            State::NeedsDriver,
            vec!["nvidia-modeset".to_string(), "nvidia-drm".to_string()]
        )
    );
    assert_eq!(r.loader, vec!["hw.nvidiadrm.modeset=\"1\""]);
    // Pascal: the 580 legacy branch package; Kepler: no driver with modesetting.
    let (e, d) = pci(0x10de, 0x1c8d, "030200");
    let r = resolve(e, d, &["vgapci1"], &ctx(&["kernel"], &[], &[], &[]));
    assert_eq!(r.hint, "install nvidia-drm-kmod-580: wol hw fix --yes");
    let (e, d) = pci(0x10de, 0x1180, "030000");
    let r = resolve(e, d, &["vgapci0"], &ctx(&["kernel"], &[], &[], &[]));
    assert_eq!(r.state, State::Unsupported);
    assert!(r.hint.contains("470"), "{}", r.hint);
    // ath10k/ath11k: unsupported even when someone built the module (not functional on 15.1).
    for (dev, why) in [(0x003e, "ath10k"), (0x0042, "ath10k")] {
        let (e, d) = pci(0x168c, dev, "028000");
        let r = resolve(e, d, &[], &ctx(&["kernel"], &["if_ath10k"], &[], &[]));
        assert_eq!(r.state, State::Unsupported);
        assert!(r.hint.contains(why) && r.firmware.is_empty(), "{r:?}");
    }
    let (e, d) = pci(0x17cb, 0x1103, "028000");
    let r = resolve(e, d, &[], &ctx(&["kernel"], &["if_ath11k"], &[], &[]));
    assert_eq!(
        (r.state, r.hint.as_str()),
        (
            State::Unsupported,
            "no ath11k/ath12k driver in FreeBSD 15.1: LinuxKPI has no MHI/QRTR transport for its firmware"
        )
    );
    // MT7921: WolKernel builds if_mt7921; without the module on disk it is unsupported.
    let (e, d) = pci(0x14c3, 0x7961, "028000");
    let r = resolve(e, d, &[], &ctx(&["kernel"], &["if_iwlwifi"], &[], &[]));
    assert_eq!(
        (r.state, r.hint.as_str()),
        (State::Unsupported, "no if_mt7921 module in this kernel")
    );
    let (e, d) = pci(0x14c3, 0x7961, "028000");
    let r = resolve(
        e,
        d,
        &["mt79210"], // LinuxKPI unit: KBUILD_MODNAME "mt7921" + unit number
        &ctx(
            &["if_mt7921"],
            &["if_mt7921"],
            &["wifi-firmware-mt76-kmod-mt7921"],
            &[],
        ),
    );
    assert_eq!(r.state, State::Ready, "{r:?}");
    assert!(r.notes.iter().any(|n| n.contains("802.11a/b/g")), "{r:?}");
    // RTL8125 with the vendor package: loader lines, attaches after a restart.
    let (e, d) = pci(0x10ec, 0x8125, "020000");
    let r = resolve(
        e,
        d,
        &[],
        &ctx(
            &["kernel"],
            &[],
            &["realtek-re-kmod"],
            &["/boot/modules/if_re.ko"],
        ),
    );
    assert_eq!(
        (r.state, r.hint.as_str()),
        (
            State::NeedsDriver,
            "restart to load the driver (loader.conf)"
        )
    );
    assert_eq!(r.loader.len(), 2);
    // ... same, but the module file is not there: no loader line pointing at nothing.
    let (e, d) = pci(0x10ec, 0x8125, "020000");
    let r = resolve(
        e,
        d,
        &["re0"],
        &ctx(
            &["kernel"],
            &[],
            &["realtek-re-kmod"],
            &["/boot/modules/other.ko"],
        ),
    );
    assert!(r.loader.is_empty());
    assert_eq!(r.state, State::Ready);
    // Unknown device of a listed class, no driver.
    let r = resolve(None, Device::default(), &[], &Ctx::default());
    assert_eq!(
        (r.state, r.hint.as_str()),
        (State::Unsupported, "no driver attached for this device")
    );
    // Nothing known about the host (tools failed): never reports anything missing.
    let (e, d) = pci(0x8086, 0x51f0, "028000");
    let r = resolve(e, d, &["iwlwifi0"], &Ctx::default());
    assert_eq!(
        (r.state, r.missing_klds.len(), r.missing_packages.len()),
        (State::Ready, 0, 0)
    );
    // Driver loaded, firmware installed, nothing attached.
    let (e, d) = pci(0x8086, 0x51f0, "028000");
    let r = resolve(
        e,
        d,
        &["none2"],
        &ctx(
            &["if_iwlwifi"],
            &["if_iwlwifi"],
            &["wifi-firmware-iwlwifi-kmod-ax210"],
            &[],
        ),
    );
    assert_eq!(
        (r.state, r.hint.as_str()),
        (
            State::NeedsDriver,
            "driver loaded but not attached: see dmesg"
        )
    );
    // In-kernel driver: module `re` satisfies kld `if_re`.
    let (e, d) = pci(0x10ec, 0x8168, "020000");
    let r = resolve(e, d, &["re0"], &ctx(&["re"], &[], &[], &[]));
    assert_eq!(r.state, State::Ready);
}

#[test]
fn boot_plan_loads_modules_and_writes_only_changes() {
    let s = read(&adl_host(ADL_PKGS));
    let p = boot_plan(&s, &|_| None);
    assert_eq!(
        p.kldload,
        ["coretemp", "cpuctl", "cuse", "iichid", "hmt", "mmcsd"]
    );
    let loader = format!(
        "{HEADER}cpu_microcode_load=\"YES\"\ncpu_microcode_name=\"/boot/firmware/intel-ucode.bin\"\n"
    );
    let powerd = format!("{HEADER}powerd_enable=\"NO\"\n");
    assert_eq!(
        p.writes,
        vec![
            (LOADER_CONF.to_string(), loader.clone()),
            ("/etc/rc.conf.d/powerd".to_string(), powerd.clone())
        ]
    );
    // Same content: nothing to write.
    let same = |path: &str| match path {
        LOADER_CONF => Some(loader.clone()),
        "/etc/rc.conf.d/powerd" => Some(powerd.clone()),
        _ => None,
    };
    assert!(boot_plan(&s, &same).writes.is_empty());
    // A user's own rc.conf.d/powerd (no header) is left alone.
    let user = |path: &str| {
        (path == "/etc/rc.conf.d/powerd").then(|| "powerd_enable=\"YES\"\n".to_string())
    };
    assert!(
        boot_plan(&s, &user)
            .writes
            .iter()
            .all(|(p, _)| p != "/etc/rc.conf.d/powerd")
    );
    // Disk moved to a machine without loader needs: the old file is emptied, an absent one stays absent.
    let none = HardwareSnapshot::default();
    let old = |path: &str| (path == LOADER_CONF).then(|| loader.clone());
    assert_eq!(
        boot_plan(&none, &old).writes,
        vec![(LOADER_CONF.to_string(), HEADER.to_string())]
    );
    assert_eq!(boot_plan(&none, &|_| None), BootPlan::default());
}

#[test]
fn boot_plan_sets_tunables_before_loading_a_module() {
    // nvidia-drm reads hw.nvidiadrm.modeset only at load: set it with kenv(1) on this boot, not
    // just in loader.conf for the next one. Loader directives (cpu_microcode_load) are not tunables.
    let dev = |klds: &[&str], loader: &[&str]| Device {
        missing_klds: klds.iter().map(|s| s.to_string()).collect(),
        loader: loader.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    let s = HardwareSnapshot {
        devices: vec![
            dev(&["coretemp"], &["cpu_microcode_load=\"YES\""]),
            dev(
                &["nvidia-modeset", "nvidia-drm"],
                &["hw.nvidiadrm.modeset=\"1\""],
            ),
            dev(&[], &["hw.other.tunable=\"1\""]),
        ],
    };
    let p = boot_plan(&s, &|_| None);
    assert_eq!(
        p.kenv,
        vec![("hw.nvidiadrm.modeset".to_string(), "1".to_string())]
    );
    assert_eq!(p.kldload, ["coretemp", "nvidia-modeset", "nvidia-drm"]);
}

#[test]
fn powerd_runs_only_with_a_software_cpufreq_driver() {
    // AMD hwpstate (or Intel est): powerd adjusts the speed.
    let h = FakeHost::new(true);
    h.command(
        "devinfo -v",
        "nexus0\n  acpi0\n    cpu0\n      hwpstate0\n      cpufreq0\n",
    )
    .string("hw.model", "AMD Ryzen 7 7840U w/ Radeon 780M Graphics")
    .int("dev.cpu.0.freq", 3300);
    let s = read(&h);
    let cpu = &s.devices[0];
    assert_eq!(
        (cpu.name.as_str(), cpu.id.as_str()),
        ("AMD Ryzen 7 7840U w/ Radeon 780M Graphics", "AuthenticAMD")
    );
    assert_eq!(cpu.driver.as_deref(), Some("hwpstate0"));
    assert_eq!(cpu.rc, vec!["powerd_enable=\"YES\""]);
}

#[test]
fn non_root_reuses_usb_devices_from_the_boot_snapshot() {
    let h = adl_host(ADL_PKGS);
    h.set_root(false);
    h.command(USB_ARGV, "No device match or lack of permissions.\n");
    let boot = read(&adl_host(ADL_PKGS));
    h.file(SNAPSHOT_PATH, &serde_json::to_string(&boot).unwrap());
    let s = read(&h);
    let usb: Vec<&str> = s
        .devices
        .iter()
        .filter(|d| d.bus == "usb")
        .map(|d| d.location.as_str())
        .collect();
    assert_eq!(usb, ["ugen0.3", "ugen0.4"]);
}

#[test]
fn non_freebsd_host_has_no_snapshot() {
    assert!(read(&FakeHost::new(false)).devices.is_empty());
}
