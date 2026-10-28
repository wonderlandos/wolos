use super::fixtures::*;
use super::*;
use crate::system::fake::FakeHost;

/// (bus, vendor, device, classes) -> (name, modules, firmware).
type Case = (Bus, Option<u16>, Option<u16>, &'static [&'static str], Want);
type Want = Option<(
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
)>;

const FWI: &[&str] = &["linux-firmware-intel"];
const FWR: &[&str] = &["linux-firmware-realtek"];
const FWA: &[&str] = &["linux-firmware-atheros"];
const NV: &[&str] = &["nvidia", "nvidia_modeset", "nvidia_drm", "nvidia_uvm"];

#[test]
fn catalogue_lookup_table() {
    use Bus::*;
    #[rustfmt::skip]
    let cases: &[Case] = &[
        // GPU: exact ids beat ranges (Broxton 5a84 vs Cannon Lake 5a*).
        (Pci, Some(0x8086), Some(0x46a8), &["030000"], Some(("Intel Iris Xe (Alder/Raptor Lake)", &["i915"], FWI))),
        (Pci, Some(0x8086), Some(0xa7a0), &["030000"], Some(("Intel Iris Xe (Alder/Raptor Lake)", &["i915"], FWI))),
        (Pci, Some(0x8086), Some(0x5a84), &["030000"], Some(("Intel HD Graphics (Broxton)", &["i915"], FWI))),
        (Pci, Some(0x8086), Some(0x5a40), &["030000"], Some(("Intel Graphics (Cannon Lake)", &["i915"], FWI))),
        (Pci, Some(0x8086), Some(0x7d55), &["030000"], Some(("Intel Arc Graphics (Meteor/Arrow Lake)", &["i915"], FWI))),
        (Pci, Some(0x8086), Some(0x0412), &["030000"], Some(("Intel graphics", &["i915"], FWI))),
        (Pci, Some(0x8086), Some(0x64a0), &["030000"], Some(("Intel Xe2 graphics (Lunar Lake, Battlemage)", &["xe"], FWI))),
        (Pci, Some(0x8086), Some(0xe20b), &["030000"], Some(("Intel Xe2 graphics (Lunar Lake, Battlemage)", &["xe"], FWI))),
        (Pci, Some(0x1002), Some(0x15bf), &["030000"], Some(("AMD Radeon", &["amdgpu"], &["linux-firmware-amdgpu"]))),
        (Pci, Some(0x1002), Some(0x6798), &["030000"], Some(("AMD Radeon (radeon)", &["radeon"], &["linux-firmware-radeon"]))),
        // NVIDIA: open modules for Turing and newer; older cards get nouveau.
        (Pci, Some(0x10de), Some(0x2520), &["030000"], Some(("NVIDIA GeForce", NV, &[]))),
        (Pci, Some(0x10de), Some(0x1e07), &["030000"], Some(("NVIDIA GeForce", NV, &[]))),
        (Pci, Some(0x10de), Some(0x1c8d), &["030200"], Some(("NVIDIA GeForce (Maxwell or older)", &["nouveau"], &["linux-firmware-nvidia"]))),
        // Virtual GPUs have DRM drivers on Linux.
        (Pci, Some(0x1af4), Some(0x1050), &["030000"], Some(("virtio-gpu", &["virtio_gpu"], &[]))),
        (Pci, Some(0x1234), Some(0x1111), &["030000"], Some(("QEMU standard VGA", &["bochs"], &[]))),
        // Wi-Fi.
        (Pci, Some(0x8086), Some(0x51f0), &["028000"], Some(("Intel Wi-Fi 6E AX210/AX211", &["iwlwifi"], FWI))),
        (Pci, Some(0x8086), Some(0x272b), &["028000"], Some(("Intel Wi-Fi 7 BE200/BE202", &["iwlwifi"], FWI))),
        (Pci, Some(0x8086), Some(0x1234), &["028000"], Some(("Intel Wi-Fi", &["iwlwifi"], FWI))),
        (Pci, Some(0x10ec), Some(0xc822), &["028000"], Some(("Realtek RTL8822CE", &["rtw88_8822ce"], FWR))),
        (Pci, Some(0x10ec), Some(0xb852), &["028000"], Some(("Realtek RTL8852BE", &["rtw89_8852be"], FWR))),
        (Pci, Some(0x10ec), Some(0xabcd), &["028000"], Some(("Realtek Wi-Fi", &[], FWR))),
        (Pci, Some(0x168c), Some(0x003e), &["028000"], Some(("Qualcomm QCA6174 (Killer 1435/1535)", &["ath10k_pci"], FWA))),
        (Pci, Some(0x168c), Some(0x0032), &["028000"], Some(("Atheros 802.11n", &["ath9k"], &[]))),
        (Pci, Some(0x17cb), Some(0x1103), &["028000"], Some(("Qualcomm WCN6855 (QCNFA765)", &["ath11k_pci"], FWA))),
        (Pci, Some(0x17cb), Some(0x1107), &["028000"], Some(("Qualcomm WCN7850 (FastConnect 7800)", &["ath12k"], FWA))),
        (Pci, Some(0x14c3), Some(0x7961), &["028000"], Some(("MediaTek MT7921/MT7922", &["mt7921e"], &["linux-firmware-mediatek"]))),
        (Pci, Some(0x14c3), Some(0x0717), &["028000"], Some(("MediaTek MT7925", &["mt7925e"], &["linux-firmware-mediatek"]))),
        // Ethernet.
        (Pci, Some(0x10ec), Some(0x8125), &["020000"], Some(("Realtek RTL8125/8126/8127 2.5/5/10GbE", &["r8169"], FWR))),
        (Pci, Some(0x8086), Some(0x15f3), &["020000"], Some(("Intel Ethernet I225/I226", &["igc"], &[]))),
        (Pci, Some(0x1af4), Some(0x1000), &["020000"], None),
        // Bluetooth (USB), camera before USB audio.
        (Usb, Some(0x8087), Some(0x0aaa), &["e00101"], Some(("Intel Bluetooth", &["btusb"], FWI))),
        (Usb, Some(0x0bda), Some(0xb85c), &["ef0201", "e00101"], Some(("Realtek Bluetooth", &["btusb"], FWR))),
        (Usb, Some(0x13d3), Some(0x3567), &["e00101"], Some(("Bluetooth", &["btusb"], &[]))),
        (Usb, Some(0x04f2), Some(0xb78e), &["ef0201", "0e0100", "010100"], Some(("USB camera", &["uvcvideo"], &[]))),
        (Usb, Some(0x046d), Some(0x0a44), &["000000", "010100"], Some(("USB audio", &["snd_usb_audio"], &[]))),
        (Usb, Some(0x046d), Some(0xc00e), &["000000", "030102"], None),
        // Audio, input, card readers, CPU, backlight.
        (Pci, Some(0x8086), Some(0x51c8), &["040380"], Some(("Intel Smart Sound / SOF audio DSP", &[], &["sof-firmware"]))),
        (Pci, Some(0x8086), Some(0xa0c8), &["040300"], Some(("HD Audio", &["snd_hda_intel"], &[]))),
        (Acpi, None, None, &["ELAN0678", "PNP0C50"], Some(("I2C HID touchpad/touchscreen", &["i2c_hid_acpi"], &[]))),
        (Acpi, None, None, &["PNP0A08", "PNP0A03"], None),
        (Pci, Some(0x10ec), Some(0x525a), &["ff0000"], Some(("Realtek PCIe card reader", &["rtsx_pci", "rtsx_pci_sdmmc"], &[]))),
        (Cpu, Some(0x8086), None, &[], Some(("Intel processor", &["coretemp"], &["intel-ucode"]))),
        (Cpu, Some(0x1022), None, &[], Some(("AMD processor", &["k10temp"], &["amd-ucode"]))),
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
        for k in e.klds {
            assert!(!k.contains('-'), "{}: module {k} uses `_`", e.name);
        }
        for (lo, hi) in e.devices {
            assert!(lo <= hi, "{}", e.name);
        }
        for l in e.loader {
            let w: Vec<&str> = l.split_whitespace().collect();
            assert!(
                w.len() >= 3 && w[0] == "options" && w[2..].iter().all(|o| o.contains('=')),
                "{l}"
            );
        }
        let does_something = !e.klds.is_empty()
            || !e.packages.is_empty()
            || !e.drivers.is_empty()
            || !e.firmware.is_empty();
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
    assert!(is_catalogue_kld("iwlwifi") && is_catalogue_kld("i915"));
    assert!(!is_catalogue_kld("evil") && !is_catalogue_kld("iwlwifi; rm"));
}

const ADL_PKGS: &str = "linux-firmware-intel\nvulkan-intel\nintel-ucode\n";

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
            "cpu 12th Gen Intel(R) Core(TM) i5-1235U | intel_pstate | needs-driver | load coretemp: wol hw fix",
            "gpu Intel Iris Xe (Alder/Raptor Lake) | i915 | ready | ",
            "wifi Intel Wi-Fi 6E AX210/AX211 | iwlwifi | ready | ",
            "bluetooth Intel Bluetooth | btusb | ready | ",
            "audio Alder Lake PCH-P High Definition Audio Controller | snd_hda_intel | needs-firmware | install sof-firmware: wol hw fix --yes",
            "webcam Integrated Camera | uvcvideo | ready | ",
            "input Alder Lake PCH Serial IO I2C Controller #0 | intel-lpss | ready | ",
            "input I2C HID touchpad/touchscreen | - | needs-driver | load i2c_hid_acpi: wol hw fix",
            "card-reader GL9750 SD Host Controller | sdhci-pci | ready | ",
            "backlight Display backlight | - | ready | ",
        ]
    );
    let cpu = &s.devices[0];
    assert_eq!((cpu.bus.as_str(), cpu.id.as_str()), ("cpu", "GenuineIntel"));
    assert!(cpu.rc.is_empty() && cpu.loader.is_empty());
    let wifi = &s.devices[2];
    assert_eq!(
        (wifi.id.as_str(), wifi.location.as_str(), wifi.bus.as_str()),
        ("8086:51f0", "0000:00:14.3", "pci")
    );
    let bt = &s.devices[3];
    assert_eq!((bt.bus.as_str(), bt.location.as_str()), ("usb", "3-10"));
    let tp = &s.devices[7];
    assert_eq!(
        (tp.bus.as_str(), tp.id.as_str()),
        ("acpi", "SYNA2BA6,PNP0C50")
    );
    assert_eq!(s.problems().count(), 3);
    assert_eq!(
        fixes(&s),
        (
            vec!["coretemp".into(), "i2c_hid_acpi".into()],
            vec!["sof-firmware".into()]
        )
    );
}

#[test]
fn vm_snapshot_virtio_gpu_and_no_cpufreq() {
    // QEMU with virtio-gpu (Hyprland runs) and a virtio NIC, no cpufreq driver.
    let h = FakeHost::linux();
    sysfs(
        &h,
        &[
            ("0000:00:01.0", "0x1af4", "0x1050", "0x030000", "virtio-pci"),
            ("0000:00:02.0", "0x1af4", "0x1000", "0x020000", "virtio-pci"),
        ],
        &[],
        &[],
    );
    h.file("/proc/cpuinfo", CPUINFO_ADL)
        .file("/proc/modules", "virtio_gpu 98304 1 - Live 0x0\n")
        .command("pacman -Qq", "");
    let s = read(&h);
    assert_eq!(
        summary(&s),
        vec![
            "cpu 12th Gen Intel(R) Core(TM) i5-1235U | - | needs-firmware | install intel-ucode: wol hw fix --yes",
            "gpu virtio-gpu | virtio-pci | ready | ",
            "ethernet 1af4:1000 | virtio-pci | ready | ",
        ]
    );
    assert!(s.devices[0].notes[0].starts_with("no cpufreq driver"));
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
    let ctx = |loaded: &[&str], disk: &[&str], pkgs: &[&str]| Ctx {
        loaded: set(loaded),
        available: set(disk),
        installed: set(pkgs),
    };
    // NVIDIA, driver packages missing: install them; no modprobe options yet.
    let (e, d) = pci(0x10de, 0x2520, "030000");
    let r = resolve(e, d, &[], &ctx(&[], &[], &[]));
    assert_eq!(
        (r.state, r.hint.as_str()),
        (
            State::NeedsDriver,
            "install nvidia-open nvidia-utils: wol hw fix --yes"
        )
    );
    assert!(r.loader.is_empty());
    // ... installed, modules not loaded yet: load them; modeset options apply when they load.
    let (e, d) = pci(0x10de, 0x2520, "030000");
    let r = resolve(e, d, &[], &ctx(&[], NV, &["nvidia-open", "nvidia-utils"]));
    assert_eq!((r.state, r.missing_klds.len()), (State::NeedsDriver, 4));
    assert_eq!(r.loader, vec!["options nvidia_drm modeset=1 fbdev=1"]);
    // ... loaded and bound.
    let (e, d) = pci(0x10de, 0x2520, "030000");
    let r = resolve(
        e,
        d,
        &["nvidia"],
        &ctx(NV, NV, &["nvidia-open", "nvidia-utils"]),
    );
    assert_eq!(r.state, State::Ready, "{r:?}");
    // A module this kernel lacks: unsupported, nothing to load.
    let (e, d) = pci(0x14c3, 0x7961, "028000");
    let r = resolve(
        e,
        d,
        &[],
        &ctx(&[], &["iwlwifi"], &["linux-firmware-mediatek"]),
    );
    assert_eq!(
        (r.state, r.hint.as_str()),
        (State::Unsupported, "no mt7921e module in this kernel")
    );
    // Module names compare with `-` and `_` alike (sysfs `snd-usb-audio`, module `snd_usb_audio`).
    let e = lookup(Bus::Usb, None, None, &["010100"]);
    let r = resolve(
        e,
        Device::default(),
        &["snd-usb-audio"],
        &ctx(&["snd_usb_audio"], &[], &[]),
    );
    assert_eq!(r.state, State::Ready);
    // Unknown device of a listed class, no driver.
    let r = resolve(None, Device::default(), &[], &Ctx::default());
    assert_eq!(
        (r.state, r.hint.as_str()),
        (State::Unsupported, "no driver attached for this device")
    );
    // Nothing known about the host: never reports anything missing.
    let (e, d) = pci(0x8086, 0x51f0, "028000");
    let r = resolve(e, d, &["iwlwifi"], &Ctx::default());
    assert_eq!(
        (r.state, r.missing_klds.len(), r.missing_packages.len()),
        (State::Ready, 0, 0)
    );
    // Module loaded, firmware installed, device not bound.
    let (e, d) = pci(0x8086, 0x51f0, "028000");
    let r = resolve(
        e,
        d,
        &[],
        &ctx(&["iwlwifi"], &["iwlwifi"], &["linux-firmware-intel"]),
    );
    assert_eq!(
        (r.state, r.hint.as_str()),
        (
            State::NeedsDriver,
            "driver loaded but not attached: see the kernel log"
        )
    );
}

#[test]
fn boot_plan_writes_options_and_module_list_only_on_change() {
    let dev = |klds: &[&str], missing: &[&str], loader: &[&str], state| Device {
        klds: klds.iter().map(|s| s.to_string()).collect(),
        missing_klds: missing.iter().map(|s| s.to_string()).collect(),
        loader: loader.iter().map(|s| s.to_string()).collect(),
        state,
        ..Default::default()
    };
    let s = HardwareSnapshot {
        devices: vec![
            dev(&["coretemp"], &["coretemp"], &[], State::NeedsDriver),
            dev(
                &["nvidia", "nvidia_drm"],
                &["nvidia_drm"],
                &["options nvidia_drm modeset=1 fbdev=1"],
                State::NeedsDriver,
            ),
            dev(&["mt7921e"], &[], &[], State::Unsupported),
        ],
    };
    let p = boot_plan(&s, &|_| None);
    assert_eq!(p.modprobe, ["coretemp", "nvidia_drm"]);
    let opts = format!("{HEADER}options nvidia_drm modeset=1 fbdev=1\n");
    let mods = format!("{HEADER}coretemp\nnvidia\nnvidia_drm\n");
    assert_eq!(
        p.writes,
        vec![
            (MODPROBE_CONF.to_string(), opts.clone()),
            (MODULES_LOAD.to_string(), mods.clone())
        ]
    );
    let same = |path: &str| match path {
        MODPROBE_CONF => Some(opts.clone()),
        MODULES_LOAD => Some(mods.clone()),
        _ => None,
    };
    assert!(boot_plan(&s, &same).writes.is_empty());
    // Disk moved to a machine without needs: an old file is emptied, an absent one stays absent.
    let none = HardwareSnapshot::default();
    let old = |path: &str| (path == MODPROBE_CONF).then(|| opts.clone());
    assert_eq!(
        boot_plan(&none, &old).writes,
        vec![(MODPROBE_CONF.to_string(), HEADER.to_string())]
    );
    assert_eq!(boot_plan(&none, &|_| None), BootPlan::default());
}

#[test]
fn empty_host_has_no_devices() {
    assert!(read(&FakeHost::linux()).devices.is_empty());
}

#[test]
fn package_list_and_pci_names_are_read_concurrently() {
    let h = adl_host("");
    h.command_delay(std::time::Duration::from_millis(150));
    let t = std::time::Instant::now();
    let fan = read(&h);
    assert!(
        t.elapsed() < std::time::Duration::from_millis(290),
        "{:?}",
        t.elapsed()
    );
    h.command_delay(std::time::Duration::ZERO);
    assert_eq!(fan, read(&h), "same snapshot, same order");
    assert!(!fan.devices.is_empty());
}

#[test]
fn real_host_lists_its_cpu() {
    let s = read(&crate::system::PosixHost);
    let cpu = s.devices.iter().find(|d| d.kind == Kind::Cpu);
    assert!(cpu.is_some_and(|c| !c.name.is_empty()), "{s:?}");
}
