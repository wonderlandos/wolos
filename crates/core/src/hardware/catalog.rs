//! The hardware catalogue: device ids -> driver, firmware, tunables, quirks. Data only.
//! Sources: fwget(8) maps on 15.1 (`/usr/libexec/fwget/pci_*`), drm-kmod 6.12 / Linux radeon/amdgpu
//! defaults, the 15.1 module set (`/boot/kernel`), port pkg-messages. archicture/modules/hardware.md.
use super::{BASE, Bus, Entry};
use shared::hardware::Kind::*;

const IWL: &[&str] = &["iwlwifi", "iwm", "iwx"];
const DRM: &[&str] = &["drmn"];
// LinuxKPI Wi-Fi on 15.1 (man pages, FreeBSD status report 2026Q2, sys/compat/linuxkpi): kernel/README.md.
const ATH10K_NOTE: &[&str] =
    &["no ath10k driver in FreeBSD 15.1: not in the release build, upstream port unfinished"];
const ATH11K_NOTE: &[&str] = &[
    "no ath11k/ath12k driver in FreeBSD 15.1: LinuxKPI has no MHI/QRTR transport for its firmware",
];
const MT7921_NOTE: &[&str] =
    &["802.11a/b/g only on FreeBSD 15.1 (mt7921(4)): up to 54 Mbit/s; module built by WolKernel"];
const NV_KLDS: &[&str] = &["nvidia-modeset", "nvidia-drm"];
const NV_DRIVERS: &[&str] = &["nvidia", "drmn"];
/// nvidia-drm-kmod pkg-message: KMS (Wayland) needs this before nvidia-drm.ko loads.
const NV_MODESET: &[&str] = &["hw.nvidiadrm.modeset=\"1\""];

const fn intel_gpu(
    name: &'static str,
    devices: &'static [(u16, u16)],
    fw: &'static [&'static str],
) -> Entry {
    Entry {
        name,
        kind: Gpu,
        vendor: Some(0x8086),
        devices,
        class: "03",
        drivers: DRM,
        klds: &["i915kms"],
        firmware: fw,
        ..BASE
    }
}

const fn iwlwifi(
    name: &'static str,
    devices: &'static [(u16, u16)],
    fw: &'static [&'static str],
) -> Entry {
    Entry {
        name,
        kind: Wifi,
        vendor: Some(0x8086),
        devices,
        class: "0280",
        drivers: IWL,
        klds: &["if_iwlwifi"],
        firmware: fw,
        ..BASE
    }
}

const fn wifi(
    name: &'static str,
    vendor: u16,
    devices: &'static [(u16, u16)],
    drivers: &'static [&'static str],
    klds: &'static [&'static str],
    fw: &'static [&'static str],
) -> Entry {
    Entry {
        name,
        kind: Wifi,
        vendor: Some(vendor),
        devices,
        class: "0280",
        drivers,
        klds,
        firmware: fw,
        ..BASE
    }
}

/// A card with no working FreeBSD 15.1 driver: nothing to install, say why.
const fn no_wifi(
    name: &'static str,
    vendor: u16,
    devices: &'static [(u16, u16)],
    klds: &'static [&'static str],
    notes: &'static [&'static str],
) -> Entry {
    Entry {
        name,
        kind: Wifi,
        vendor: Some(vendor),
        devices,
        class: "0280",
        klds,
        notes,
        supported: false,
        ..BASE
    }
}

const fn bt(
    name: &'static str,
    vendor: Option<u16>,
    fw: &'static [&'static str],
    notes: &'static [&'static str],
) -> Entry {
    Entry {
        name,
        kind: Bluetooth,
        bus: Bus::Usb,
        vendor,
        class: "e00101",
        drivers: &["ubt"],
        klds: &["ng_ubt"],
        firmware: fw,
        notes,
        ..BASE
    }
}

const fn virt(name: &'static str, vendor: u16, devices: &'static [(u16, u16)]) -> Entry {
    Entry {
        name,
        kind: Gpu,
        vendor: Some(vendor),
        devices,
        class: "03",
        notes: &["virtual GPU: no DRM driver on FreeBSD, framebuffer only (no compositor)"],
        supported: false,
        ..BASE
    }
}

/// Radeon (pre-GCN, SI, CIK): radeonkms is the default driver for these (amdgpu needs si/cik_support).
const RADEON: &[(u16, u16)] = &[
    (0x1304, 0x131d), // Kaveri
    (0x4100, 0x5fff), // R100-R400
    (0x6600, 0x667f), // Oland, Bonaire, Hainan
    (0x6700, 0x67bf), // Cayman, Barts, Turks, Caicos, Tahiti, Hawaii
    (0x6800, 0x683f), // Pitcairn, Verde
    (0x6880, 0x68ff), // Evergreen
    (0x7100, 0x72ff), // R500
    (0x9400, 0x971f), // R600, RV6xx, RV7xx, RS780/880, Sumo
    (0x9800, 0x980f), // Palm
    (0x9830, 0x985f), // Kabini, Mullins
    (0x9900, 0x99ff), // Aruba
];

pub const CATALOG: &[Entry] = &[
    // ---- CPU (bus Cpu: vendor from hw.model) ----
    Entry {
        name: "Intel processor",
        kind: Cpu,
        bus: Bus::Cpu,
        vendor: Some(0x8086),
        drivers: &["hwpstate_intel", "est"],
        klds: &["coretemp", "cpuctl"],
        firmware: &["cpu-microcode-intel"],
        loader: &[
            "cpu_microcode_load=\"YES\"",
            "cpu_microcode_name=\"/boot/firmware/intel-ucode.bin\"",
        ],
        ..BASE
    },
    Entry {
        name: "AMD processor",
        kind: Cpu,
        bus: Bus::Cpu,
        vendor: Some(0x1022),
        drivers: &["hwpstate"],
        klds: &["amdtemp", "cpuctl"],
        firmware: &["cpu-microcode-amd"],
        loader: &[
            "cpu_microcode_load=\"YES\"",
            "cpu_microcode_name=\"/boot/firmware/amd-ucode.bin\"",
        ],
        ..BASE
    },
    // ---- GPU (fwget pci_video_intel; RPL/MTL/DG2 added from i915_pciids.h) ----
    intel_gpu(
        "Intel HD Graphics (Skylake)",
        &[(0x1900, 0x19ff)],
        &["gpu-firmware-intel-kmod-skylake"],
    ),
    intel_gpu(
        "Intel HD Graphics (Broxton)",
        &[(0x0a00, 0x0aff), (0x1a00, 0x1aff), (0x5a84, 0x5a85)],
        &["gpu-firmware-intel-kmod-broxton"],
    ),
    intel_gpu(
        "Intel UHD Graphics (Gemini Lake)",
        &[(0x3180, 0x318f)],
        &["gpu-firmware-intel-kmod-geminilake"],
    ),
    intel_gpu(
        "Intel UHD Graphics (Kaby/Coffee/Comet Lake)",
        &[
            (0x3e00, 0x3eff),
            (0x5900, 0x59ff),
            (0x8700, 0x87ff),
            (0x9b00, 0x9bff),
        ],
        &["gpu-firmware-intel-kmod-kabylake"],
    ),
    intel_gpu(
        "Intel Graphics (Cannon Lake)",
        &[(0x5a00, 0x5aff)],
        &["gpu-firmware-intel-kmod-cannonlake"],
    ),
    intel_gpu(
        "Intel Iris Plus (Ice Lake)",
        &[(0x8a00, 0x8aff)],
        &["gpu-firmware-intel-kmod-icelake"],
    ),
    intel_gpu(
        "Intel UHD Graphics (Elkhart/Jasper Lake)",
        &[(0x4500, 0x45ff), (0x4e00, 0x4eff)],
        &["gpu-firmware-intel-kmod-elkhartlake"],
    ),
    intel_gpu(
        "Intel Iris Xe (Tiger Lake)",
        &[(0x9a00, 0x9aff)],
        &["gpu-firmware-intel-kmod-tigerlake"],
    ),
    intel_gpu(
        "Intel UHD Graphics (Rocket Lake)",
        &[(0x4c00, 0x4cff)],
        &[
            "gpu-firmware-intel-kmod-rocketlake",
            "gpu-firmware-intel-kmod-tigerlake",
        ],
    ),
    intel_gpu(
        "Intel Iris Xe MAX (DG1)",
        &[(0x4900, 0x49ff)],
        &["gpu-firmware-intel-kmod-dg1"],
    ),
    intel_gpu(
        "Intel Iris Xe (Alder/Raptor Lake)",
        &[(0x4600, 0x46ff), (0xa700, 0xa7ff)],
        &[
            "gpu-firmware-intel-kmod-alderlake",
            "gpu-firmware-intel-kmod-tigerlake",
        ],
    ),
    intel_gpu(
        "Intel Arc (DG2)",
        &[(0x5600, 0x56ff)],
        &["gpu-firmware-intel-kmod-dg2"],
    ),
    intel_gpu(
        "Intel Arc Graphics (Meteor/Arrow Lake)",
        &[(0x7d00, 0x7dff)],
        &["gpu-firmware-intel-kmod-meteorlake"],
    ),
    Entry {
        name: "Intel Xe2 graphics (Lunar Lake, Battlemage)",
        kind: Gpu,
        vendor: Some(0x8086),
        devices: &[(0x6400, 0x64ff), (0xe200, 0xe2ff)],
        class: "03",
        notes: &["needs the xe driver, which drm-kmod 6.12 does not ship: framebuffer only"],
        supported: false,
        ..BASE
    },
    intel_gpu("Intel graphics", &[], &[]),
    Entry {
        name: "AMD Radeon (radeonkms)",
        kind: Gpu,
        vendor: Some(0x1002),
        devices: RADEON,
        class: "03",
        drivers: DRM,
        klds: &["radeonkms"],
        firmware: &["gpu-firmware-kmod"],
        ..BASE
    },
    Entry {
        name: "AMD Radeon",
        kind: Gpu,
        vendor: Some(0x1002),
        class: "03",
        drivers: DRM,
        klds: &["amdgpu"],
        firmware: &["gpu-firmware-kmod"],
        ..BASE
    },
    // NVIDIA (pkg nvidia-drm-kmod = nvidia-drm-612-kmod + nvidia-kmod + nvidia-driver 595, ~520 MB:
    // installed online at first boot). 595 dropped Maxwell/Pascal/Volta (580 is their last branch).
    Entry {
        name: "NVIDIA GeForce (Kepler or older)",
        kind: Gpu,
        vendor: Some(0x10de),
        devices: &[(0x0000, 0x133f)],
        class: "03",
        notes: &["needs the legacy 470 driver, which has no modesetting (Wayland) on FreeBSD"],
        supported: false,
        ..BASE
    },
    Entry {
        name: "NVIDIA GeForce (Maxwell/Pascal/Volta)",
        kind: Gpu,
        vendor: Some(0x10de),
        devices: &[(0x1340, 0x1dff)],
        class: "03",
        drivers: NV_DRIVERS,
        klds: NV_KLDS,
        packages: &["nvidia-drm-kmod-580"],
        loader: NV_MODESET,
        notes: &["legacy 580 driver (595 dropped these GPUs); installed online at first boot"],
        ..BASE
    },
    Entry {
        name: "NVIDIA GeForce",
        kind: Gpu,
        vendor: Some(0x10de),
        class: "03",
        drivers: NV_DRIVERS,
        klds: NV_KLDS,
        packages: &["nvidia-drm-kmod"],
        loader: NV_MODESET,
        notes: &["driver 595, not shipped offline (~520 MB): installed online at first boot"],
        ..BASE
    },
    virt("QEMU standard VGA", 0x1234, &[(0x1111, 0x1111)]),
    virt("QEMU QXL", 0x1b36, &[(0x0100, 0x0100)]),
    virt("virtio-gpu", 0x1af4, &[(0x1050, 0x1050)]),
    virt("VMware SVGA", 0x15ad, &[(0x0405, 0x0405)]),
    virt("VirtualBox display", 0x80ee, &[(0xbeef, 0xbeef)]),
    // ---- Wi-Fi (fwget pci_network_* on 15.1) ----
    iwlwifi(
        "Intel Wireless 7260/3160/7265/3165/3168",
        &[
            (0x08b1, 0x08b4),
            (0x095a, 0x095b),
            (0x24fb, 0x24fb),
            (0x3165, 0x3166),
        ],
        &["wifi-firmware-iwlwifi-kmod-7000"],
    ),
    iwlwifi(
        "Intel Wireless 8260/8265",
        &[(0x24f3, 0x24f6), (0x24fd, 0x24fd)],
        &["wifi-firmware-iwlwifi-kmod-8000"],
    ),
    iwlwifi(
        "Intel Wireless-AC 9260/9461/9462/9560",
        &[
            (0x2526, 0x2526),
            (0x271b, 0x271c),
            (0x30dc, 0x30dc),
            (0x31dc, 0x31dc),
            (0x9df0, 0x9df0),
            (0xa370, 0xa370),
        ],
        &["wifi-firmware-iwlwifi-kmod-9000"],
    ),
    iwlwifi(
        "Intel Wi-Fi 6 AX200/AX201",
        &[
            (0x02f0, 0x02f0),
            (0x06f0, 0x06f0),
            (0x2723, 0x2723),
            (0x34f0, 0x34f0),
            (0x3df0, 0x3df0),
            (0x43f0, 0x43f0),
            (0x4df0, 0x4df0),
            (0xa0f0, 0xa0f0),
        ],
        &["wifi-firmware-iwlwifi-kmod-22000"],
    ),
    iwlwifi(
        "Intel Wi-Fi 6E AX210/AX211",
        &[
            (0x2725, 0x2726),
            (0x2729, 0x2729),
            (0x51f0, 0x51f1),
            (0x54f0, 0x54f0),
            (0x7a70, 0x7a70),
            (0x7af0, 0x7af0),
            (0x7e40, 0x7e40),
            (0x7f70, 0x7f70),
        ],
        &["wifi-firmware-iwlwifi-kmod-ax210"],
    ),
    iwlwifi(
        "Intel Wi-Fi 7 BE200/BE202",
        &[
            (0x2727, 0x2727),
            (0x272b, 0x272b),
            (0x272d, 0x272d),
            (0x4d40, 0x4d40),
            (0x7740, 0x7740),
            (0xa840, 0xa840),
        ],
        &["wifi-firmware-iwlwifi-kmod-bz"],
    ),
    iwlwifi(
        "Intel Wi-Fi 7 (Lunar/Panther Lake)",
        &[
            (0x6e70, 0x6e70),
            (0xd240, 0xd240),
            (0xd340, 0xd340),
            (0xe340, 0xe340),
            (0xe440, 0xe440),
        ],
        &["wifi-firmware-iwlwifi-kmod-sc"],
    ),
    iwlwifi("Intel Wi-Fi", &[], &[]),
    wifi(
        "Realtek RTL8723DE",
        0x10ec,
        &[(0xd723, 0xd723)],
        &["rtw88"],
        &["if_rtw88"],
        &["wifi-firmware-rtw88-kmod-rtw8723d"],
    ),
    wifi(
        "Realtek RTL8814AE",
        0x10ec,
        &[(0x8813, 0x8813)],
        &["rtw88"],
        &["if_rtw88"],
        &["wifi-firmware-rtw88-kmod-rtw8814a"],
    ),
    wifi(
        "Realtek RTL8821CE",
        0x10ec,
        &[(0xb821, 0xb821), (0xc821, 0xc821)],
        &["rtw88"],
        &["if_rtw88"],
        &["wifi-firmware-rtw88-kmod-rtw8821c"],
    ),
    wifi(
        "Realtek RTL8822BE",
        0x10ec,
        &[(0xb822, 0xb822)],
        &["rtw88"],
        &["if_rtw88"],
        &["wifi-firmware-rtw88-kmod-rtw8822b"],
    ),
    wifi(
        "Realtek RTL8822CE",
        0x10ec,
        &[(0xc822, 0xc822), (0xc82f, 0xc82f)],
        &["rtw88"],
        &["if_rtw88"],
        &["wifi-firmware-rtw88-kmod-rtw8822c"],
    ),
    wifi(
        "Realtek RTL8851BE",
        0x10ec,
        &[(0xb851, 0xb851)],
        &["rtw89"],
        &["if_rtw89"],
        &["wifi-firmware-rtw89-kmod-rtw8851b"],
    ),
    wifi(
        "Realtek RTL8852AE",
        0x10ec,
        &[(0x8852, 0x8852), (0xa85a, 0xa85a)],
        &["rtw89"],
        &["if_rtw89"],
        &["wifi-firmware-rtw89-kmod-rtw8852a"],
    ),
    wifi(
        "Realtek RTL8852BE",
        0x10ec,
        &[(0xb852, 0xb852), (0xb85b, 0xb85b)],
        &["rtw89"],
        &["if_rtw89"],
        &["wifi-firmware-rtw89-kmod-rtw8852b"],
    ),
    wifi(
        "Realtek RTL8852BTE",
        0x10ec,
        &[(0xb520, 0xb520)],
        &["rtw89"],
        &["if_rtw89"],
        &["wifi-firmware-rtw89-kmod-rtw8852bt"],
    ),
    wifi(
        "Realtek RTL8852CE",
        0x10ec,
        &[(0xc852, 0xc852)],
        &["rtw89"],
        &["if_rtw89"],
        &["wifi-firmware-rtw89-kmod-rtw8852c"],
    ),
    wifi(
        "Realtek RTL8922AE",
        0x10ec,
        &[(0x8922, 0x8922), (0x892b, 0x892b)],
        &["rtw89"],
        &["if_rtw89"],
        &["wifi-firmware-rtw89-kmod-rtw8922a"],
    ),
    wifi(
        "Realtek RTL8188CE/8192CE/8188EE",
        0x10ec,
        &[(0x8176, 0x8179)],
        &["rtwn"],
        &["if_rtwn_pci"],
        &[],
    ),
    wifi(
        "Realtek Wi-Fi",
        0x10ec,
        &[],
        &["rtw88", "rtw89"],
        &["if_rtw88", "if_rtw89"],
        &[],
    ),
    wifi(
        "Atheros 802.11n",
        0x168c,
        &[(0x0007, 0x0037)],
        &["ath"],
        &["if_ath"],
        &[],
    ),
    no_wifi(
        "Qualcomm QCA988x",
        0x168c,
        &[(0x003c, 0x003c)],
        &["if_ath10k"],
        ATH10K_NOTE,
    ),
    no_wifi(
        "Qualcomm QCA6174 (Killer 1435/1535)",
        0x168c,
        &[(0x003e, 0x003e), (0x0041, 0x0041)],
        &["if_ath10k"],
        ATH10K_NOTE,
    ),
    no_wifi(
        "Qualcomm QCA9377",
        0x168c,
        &[(0x0042, 0x0042)],
        &["if_ath10k"],
        ATH10K_NOTE,
    ),
    no_wifi(
        "Qualcomm QCA6390",
        0x17cb,
        &[(0x1101, 0x1101)],
        &["if_ath11k"],
        ATH11K_NOTE,
    ),
    no_wifi(
        "Qualcomm WCN6855 (QCNFA765)",
        0x17cb,
        &[(0x1103, 0x1103)],
        &["if_ath11k"],
        ATH11K_NOTE,
    ),
    no_wifi(
        "Qualcomm WCN7850 (FastConnect 7800)",
        0x17cb,
        &[(0x1107, 0x1107)],
        &["if_ath12k"],
        ATH11K_NOTE,
    ),
    no_wifi("Qualcomm Wi-Fi", 0x17cb, &[], &["if_ath11k"], ATH11K_NOTE),
    Entry {
        notes: MT7921_NOTE,
        ..wifi(
            "MediaTek MT7921/MT7922",
            0x14c3,
            &[
                (0x0608, 0x0608),
                (0x0616, 0x0616),
                (0x7920, 0x7920),
                (0x7922, 0x7922),
                (0x7961, 0x7961),
            ],
            &["mt7921"],
            &["if_mt7921"],
            &["wifi-firmware-mt76-kmod-mt7921"],
        )
    },
    no_wifi(
        "MediaTek MT7925",
        0x14c3,
        &[(0x0717, 0x0717), (0x7925, 0x7925)],
        &["if_mt7925"],
        &["mt7925 is unstable on FreeBSD 15.1 (status report 2026Q2): not built"],
    ),
    no_wifi(
        "MediaTek Wi-Fi",
        0x14c3,
        &[],
        &[],
        &["no driver for this MediaTek chip in FreeBSD 15.1"],
    ),
    Entry {
        name: "Broadcom Wi-Fi",
        kind: Wifi,
        vendor: Some(0x14e4),
        class: "0280",
        drivers: &["bwn", "bwi"],
        notes: &["most Broadcom Wi-Fi has no FreeBSD driver: use a supported USB adapter"],
        supported: false,
        ..BASE
    },
    // ---- Ethernet ----
    Entry {
        name: "Realtek RTL8125/8126/8127 2.5/5/10GbE",
        kind: Ethernet,
        vendor: Some(0x10ec),
        devices: &[(0x8125, 0x8127), (0x5000, 0x5000)],
        class: "0200",
        drivers: &["re"],
        packages: &["realtek-re-kmod"],
        loader: &[
            "if_re_load=\"YES\"",
            "if_re_name=\"/boot/modules/if_re.ko\"",
        ],
        notes: &[
            "vendor driver (realtek-re-kmod) loaded by the loader: base re(4) lacks these chips",
        ],
        ..BASE
    },
    Entry {
        name: "Realtek RTL8111/8168/8101 Ethernet",
        kind: Ethernet,
        vendor: Some(0x10ec),
        devices: &[(0x8136, 0x8136), (0x8161, 0x8161), (0x8168, 0x8169)],
        class: "0200",
        drivers: &["re"],
        klds: &["if_re"],
        ..BASE
    },
    Entry {
        name: "Intel Ethernet I225/I226",
        kind: Ethernet,
        vendor: Some(0x8086),
        devices: &[
            (0x0d9f, 0x0d9f),
            (0x125b, 0x125f),
            (0x15f2, 0x15f3),
            (0x15f7, 0x15f8),
            (0x15fd, 0x15fd),
        ],
        class: "0200",
        drivers: &["igc"],
        klds: &["if_igc"],
        ..BASE
    },
    Entry {
        name: "Intel Ethernet",
        kind: Ethernet,
        vendor: Some(0x8086),
        class: "0200",
        drivers: &["em", "igb", "ix", "ixl", "ice"],
        klds: &["if_em"],
        ..BASE
    },
    // ---- Bluetooth (USB; firmware loaded by iwmbtfw(8)/rtlbtfw(8)/bcmfw(8) from devd) ----
    bt(
        "Intel Bluetooth",
        Some(0x8087),
        &["iwmbt-firmware"],
        &["firmware upload: iwmbtfw(8) via devd"],
    ),
    bt(
        "Realtek Bluetooth",
        Some(0x0bda),
        &["rtlbt-firmware"],
        &["firmware upload: rtlbtfw(8) via devd"],
    ),
    Entry {
        name: "Broadcom BCM2033 Bluetooth",
        vendor: Some(0x0a5c),
        devices: &[(0x2033, 0x2033)],
        class: "",
        drivers: &["ubtbcmfw", "ubt"],
        klds: &["ubtbcmfw", "ng_ubt"],
        notes: &["firmware upload: bcmfw(8)"],
        ..bt("", None, &["bluez-firmware"], &[])
    },
    bt("Bluetooth", None, &[], &[]),
    // ---- Audio ----
    Entry {
        name: "Intel Smart Sound / SOF audio DSP",
        kind: Audio,
        vendor: Some(0x8086),
        class: "040380",
        notes: &[
            "no FreeBSD driver for the SOF DSP: pick HD Audio in the firmware setup if offered; USB/Bluetooth audio work",
        ],
        supported: false,
        ..BASE
    },
    Entry {
        name: "AMD audio coprocessor (ACP)",
        kind: Audio,
        vendor: Some(0x1022),
        class: "0480",
        notes: &["no FreeBSD driver for ACP (digital microphones); HD Audio outputs work"],
        supported: false,
        ..BASE
    },
    Entry {
        name: "HD Audio",
        kind: Audio,
        class: "0403",
        drivers: &["hdac"],
        klds: &["snd_hda"],
        ..BASE
    },
    // ---- Webcam (UVC in userland: webcamd + cuse); before USB audio: a camera with a mic is a camera ----
    Entry {
        name: "USB camera",
        kind: Webcam,
        bus: Bus::Usb,
        class: "0e",
        klds: &["cuse"],
        packages: &["webcamd"],
        notes: &["/dev/video* is for members of group video"],
        ..BASE
    },
    Entry {
        name: "USB audio",
        kind: Audio,
        bus: Bus::Usb,
        class: "01",
        drivers: &["uaudio"],
        klds: &["snd_uaudio"],
        ..BASE
    },
    // ---- Input ----
    Entry {
        name: "I2C HID touchpad/touchscreen",
        kind: Input,
        bus: Bus::Acpi,
        class: "PNP0C50",
        klds: &["ig4", "iichid", "hmt"],
        ..BASE
    },
    Entry {
        name: "I2C HID touchpad/touchscreen",
        kind: Input,
        bus: Bus::Acpi,
        class: "ACPI0C50",
        klds: &["ig4", "iichid", "hmt"],
        ..BASE
    },
    Entry {
        name: "AMD I2C controller",
        kind: Input,
        bus: Bus::Acpi,
        class: "AMDI0010",
        klds: &["ig4"],
        ..BASE
    },
    Entry {
        name: "Intel Serial IO I2C",
        kind: Input,
        vendor: Some(0x8086),
        class: "0c80",
        drivers: &["ig4iic"],
        klds: &["ig4"],
        ..BASE
    },
    // ---- Card readers ----
    Entry {
        name: "Realtek PCIe card reader",
        kind: CardReader,
        vendor: Some(0x10ec),
        devices: &[
            (0x5209, 0x5209),
            (0x5227, 0x522a),
            (0x5249, 0x524a),
            (0x525a, 0x525a),
            (0x5260, 0x5260),
        ],
        drivers: &["rtsx"],
        klds: &["rtsx", "mmcsd"],
        ..BASE
    },
    Entry {
        name: "SD card reader",
        kind: CardReader,
        class: "0805",
        drivers: &["sdhci_pci"],
        klds: &["sdhci_pci", "mmcsd"],
        ..BASE
    },
    // ---- Backlight (backlight(4) devices) ----
    Entry {
        name: "Display backlight",
        kind: Backlight,
        bus: Bus::Backlight,
        notes: &["raised from 0 at boot (wolos_gpu)"],
        ..BASE
    },
];
