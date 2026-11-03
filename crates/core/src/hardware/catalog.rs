// SPDX-License-Identifier: BSD-2-Clause
//! The hardware catalogue: device ids -> Linux driver, kernel modules, Arch firmware/driver packages,
//! modprobe options, quirks. Data only. Sources: kernel driver id tables (i915/xe, amdgpu/radeon,
//! iwlwifi, rtw88/rtw89, ath1xk, mt76), Arch linux-firmware split packages, NVIDIA open-module support
//! list (Turing and newer). docs/modules/hardware.md.
use super::{BASE, Bus, Entry};
use shared::hardware::HardwareSnapshot;
use shared::hardware::Kind::*;

const FW_INTEL: &[&str] = &["linux-firmware-intel"];
const FW_AMD: &[&str] = &["linux-firmware-amdgpu"];
const FW_RADEON: &[&str] = &["linux-firmware-radeon"];
const FW_REALTEK: &[&str] = &["linux-firmware-realtek"];
const FW_ATH: &[&str] = &["linux-firmware-atheros"];
const FW_MTK: &[&str] = &["linux-firmware-mediatek"];
/// nvidia_drm: KMS for Wayland, fbdev for the console; read when the module loads.
const NV_OPTIONS: &[&str] = &["options nvidia_drm modeset=1 fbdev=1"];

const fn intel_gpu(name: &'static str, devices: &'static [(u16, u16)]) -> Entry {
    Entry {
        name,
        kind: Gpu,
        vendor: Some(0x8086),
        devices,
        class: "03",
        drivers: &["i915"],
        klds: &["i915"],
        firmware: FW_INTEL,
        packages: &["vulkan-intel"],
        ..BASE
    }
}

const fn iwlwifi(name: &'static str, devices: &'static [(u16, u16)]) -> Entry {
    wifi(name, 0x8086, devices, &["iwlwifi"], FW_INTEL)
}

/// PCI Wi-Fi whose driver and module share a name.
const fn wifi(
    name: &'static str,
    vendor: u16,
    devices: &'static [(u16, u16)],
    module: &'static [&'static str],
    fw: &'static [&'static str],
) -> Entry {
    Entry {
        name,
        kind: Wifi,
        vendor: Some(vendor),
        devices,
        class: "0280",
        drivers: module,
        klds: module,
        firmware: fw,
        ..BASE
    }
}

const fn bt(name: &'static str, vendor: Option<u16>, fw: &'static [&'static str]) -> Entry {
    Entry {
        name,
        kind: Bluetooth,
        bus: Bus::Usb,
        vendor,
        class: "e00101",
        drivers: &["btusb"],
        klds: &["btusb"],
        firmware: fw,
        ..BASE
    }
}

/// Virtual GPU with an in-kernel DRM driver: the compositor runs in the VM.
const fn virt(
    name: &'static str,
    vendor: u16,
    devices: &'static [(u16, u16)],
    drivers: &'static [&'static str],
    klds: &'static [&'static str],
) -> Entry {
    Entry {
        name,
        kind: Gpu,
        vendor: Some(vendor),
        devices,
        class: "03",
        drivers,
        klds,
        ..BASE
    }
}

/// Radeon (pre-GCN, SI, CIK): radeon is the kernel's default for these (amdgpu needs si/cik_support).
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
    // ---- CPU (bus Cpu: vendor from /proc/cpuinfo; driver = cpufreq scaling_driver) ----
    Entry {
        name: "Intel processor",
        kind: Cpu,
        bus: Bus::Cpu,
        vendor: Some(0x8086),
        drivers: &["intel_pstate", "intel_cpufreq", "acpi-cpufreq"],
        klds: &["coretemp"],
        firmware: &["intel-ucode"],
        ..BASE
    },
    Entry {
        name: "AMD processor",
        kind: Cpu,
        bus: Bus::Cpu,
        vendor: Some(0x1022),
        drivers: &["amd-pstate-epp", "amd-pstate", "acpi-cpufreq"],
        klds: &["k10temp"],
        firmware: &["amd-ucode"],
        ..BASE
    },
    // ---- GPU: Intel (i915 up to Meteor/Arrow Lake; xe for Xe2 and newer) ----
    intel_gpu("Intel HD Graphics (Skylake)", &[(0x1900, 0x19ff)]),
    intel_gpu(
        "Intel HD Graphics (Broxton)",
        &[(0x0a00, 0x0aff), (0x1a00, 0x1aff), (0x5a84, 0x5a85)],
    ),
    intel_gpu("Intel UHD Graphics (Gemini Lake)", &[(0x3180, 0x318f)]),
    intel_gpu(
        "Intel UHD Graphics (Kaby/Coffee/Comet Lake)",
        &[
            (0x3e00, 0x3eff),
            (0x5900, 0x59ff),
            (0x8700, 0x87ff),
            (0x9b00, 0x9bff),
        ],
    ),
    intel_gpu("Intel Graphics (Cannon Lake)", &[(0x5a00, 0x5aff)]),
    intel_gpu("Intel Iris Plus (Ice Lake)", &[(0x8a00, 0x8aff)]),
    intel_gpu(
        "Intel UHD Graphics (Elkhart/Jasper Lake)",
        &[(0x4500, 0x45ff), (0x4e00, 0x4eff)],
    ),
    intel_gpu("Intel Iris Xe (Tiger Lake)", &[(0x9a00, 0x9aff)]),
    intel_gpu("Intel UHD Graphics (Rocket Lake)", &[(0x4c00, 0x4cff)]),
    intel_gpu("Intel Iris Xe MAX (DG1)", &[(0x4900, 0x49ff)]),
    intel_gpu(
        "Intel Iris Xe (Alder/Raptor Lake)",
        &[(0x4600, 0x46ff), (0xa700, 0xa7ff)],
    ),
    intel_gpu("Intel Arc (DG2)", &[(0x5600, 0x56ff)]),
    intel_gpu(
        "Intel Arc Graphics (Meteor/Arrow Lake)",
        &[(0x7d00, 0x7dff)],
    ),
    Entry {
        name: "Intel Xe2 graphics (Lunar Lake, Battlemage)",
        kind: Gpu,
        vendor: Some(0x8086),
        devices: &[(0x6400, 0x64ff), (0xe200, 0xe2ff), (0xb080, 0xb0ff)],
        class: "03",
        drivers: &["xe"],
        klds: &["xe"],
        firmware: FW_INTEL,
        packages: &["vulkan-intel"],
        ..BASE
    },
    intel_gpu("Intel graphics", &[]),
    // ---- GPU: AMD ----
    Entry {
        name: "AMD Radeon (radeon)",
        kind: Gpu,
        vendor: Some(0x1002),
        devices: RADEON,
        class: "03",
        drivers: &["radeon"],
        klds: &["radeon"],
        firmware: FW_RADEON,
        ..BASE
    },
    Entry {
        name: "AMD Radeon",
        kind: Gpu,
        vendor: Some(0x1002),
        class: "03",
        drivers: &["amdgpu"],
        klds: &["amdgpu"],
        firmware: FW_AMD,
        packages: &["vulkan-radeon"],
        ..BASE
    },
    // ---- GPU: NVIDIA (open kernel modules need Turing or newer; older cards get nouveau) ----
    Entry {
        name: "NVIDIA GeForce (Maxwell or older)",
        kind: Gpu,
        vendor: Some(0x10de),
        devices: &[(0x0000, 0x1dff)],
        class: "03",
        drivers: &["nouveau"],
        klds: &["nouveau"],
        firmware: &["linux-firmware-nvidia"],
        notes: &[
            "nouveau driver: the NVIDIA driver supports Turing (GeForce 16/RTX 20) and newer only; games run slower",
        ],
        ..BASE
    },
    Entry {
        name: "NVIDIA GeForce",
        kind: Gpu,
        vendor: Some(0x10de),
        class: "03",
        drivers: &["nvidia"],
        klds: &["nvidia", "nvidia_modeset", "nvidia_drm", "nvidia_uvm"],
        packages: &["nvidia-open", "nvidia-utils"],
        loader: NV_OPTIONS,
        notes: &["NVIDIA driver (open kernel modules); installed online at first boot if missing"],
        ..BASE
    },
    virt(
        "QEMU standard VGA",
        0x1234,
        &[(0x1111, 0x1111)],
        &["bochs-drm"],
        &["bochs"],
    ),
    virt("QEMU QXL", 0x1b36, &[(0x0100, 0x0100)], &["qxl"], &["qxl"]),
    virt(
        "virtio-gpu",
        0x1af4,
        &[(0x1050, 0x1050)],
        &["virtio-pci"],
        &["virtio_gpu"],
    ),
    virt(
        "VMware SVGA",
        0x15ad,
        &[(0x0405, 0x0405)],
        &["vmwgfx"],
        &["vmwgfx"],
    ),
    virt(
        "VirtualBox display",
        0x80ee,
        &[(0xbeef, 0xbeef)],
        &["vboxvideo"],
        &["vboxvideo"],
    ),
    // ---- Wi-Fi ----
    iwlwifi(
        "Intel Wireless 7260/3160/7265/3165/3168",
        &[
            (0x08b1, 0x08b4),
            (0x095a, 0x095b),
            (0x24fb, 0x24fb),
            (0x3165, 0x3166),
        ],
    ),
    iwlwifi(
        "Intel Wireless 8260/8265",
        &[(0x24f3, 0x24f6), (0x24fd, 0x24fd)],
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
    ),
    iwlwifi("Intel Wi-Fi", &[]),
    wifi(
        "Realtek RTL8723DE",
        0x10ec,
        &[(0xd723, 0xd723)],
        &["rtw88_8723de"],
        FW_REALTEK,
    ),
    wifi(
        "Realtek RTL8814AE",
        0x10ec,
        &[(0x8813, 0x8813)],
        &["rtw88_8814ae"],
        FW_REALTEK,
    ),
    wifi(
        "Realtek RTL8821CE",
        0x10ec,
        &[(0xb821, 0xb821), (0xc821, 0xc821)],
        &["rtw88_8821ce"],
        FW_REALTEK,
    ),
    wifi(
        "Realtek RTL8822BE",
        0x10ec,
        &[(0xb822, 0xb822)],
        &["rtw88_8822be"],
        FW_REALTEK,
    ),
    wifi(
        "Realtek RTL8822CE",
        0x10ec,
        &[(0xc822, 0xc822), (0xc82f, 0xc82f)],
        &["rtw88_8822ce"],
        FW_REALTEK,
    ),
    wifi(
        "Realtek RTL8851BE",
        0x10ec,
        &[(0xb851, 0xb851)],
        &["rtw89_8851be"],
        FW_REALTEK,
    ),
    wifi(
        "Realtek RTL8852AE",
        0x10ec,
        &[(0x8852, 0x8852), (0xa85a, 0xa85a)],
        &["rtw89_8852ae"],
        FW_REALTEK,
    ),
    wifi(
        "Realtek RTL8852BE",
        0x10ec,
        &[(0xb852, 0xb852), (0xb85b, 0xb85b)],
        &["rtw89_8852be"],
        FW_REALTEK,
    ),
    wifi(
        "Realtek RTL8852BTE",
        0x10ec,
        &[(0xb520, 0xb520)],
        &["rtw89_8852bte"],
        FW_REALTEK,
    ),
    wifi(
        "Realtek RTL8852CE",
        0x10ec,
        &[(0xc852, 0xc852)],
        &["rtw89_8852ce"],
        FW_REALTEK,
    ),
    wifi(
        "Realtek RTL8922AE",
        0x10ec,
        &[(0x8922, 0x8922), (0x892b, 0x892b)],
        &["rtw89_8922ae"],
        FW_REALTEK,
    ),
    wifi(
        "Realtek RTL8188CE/8192CE/8188EE",
        0x10ec,
        &[(0x8176, 0x8179)],
        &["rtl8192ce"],
        FW_REALTEK,
    ),
    Entry {
        name: "Realtek Wi-Fi",
        kind: Wifi,
        vendor: Some(0x10ec),
        class: "0280",
        firmware: FW_REALTEK,
        ..BASE
    },
    wifi(
        "Atheros 802.11n",
        0x168c,
        &[(0x0007, 0x0037)],
        &["ath9k"],
        &[],
    ),
    wifi(
        "Qualcomm QCA988x",
        0x168c,
        &[(0x003c, 0x003c)],
        &["ath10k_pci"],
        FW_ATH,
    ),
    wifi(
        "Qualcomm QCA6174 (Killer 1435/1535)",
        0x168c,
        &[(0x003e, 0x003e), (0x0041, 0x0041)],
        &["ath10k_pci"],
        FW_ATH,
    ),
    wifi(
        "Qualcomm QCA9377",
        0x168c,
        &[(0x0042, 0x0042)],
        &["ath10k_pci"],
        FW_ATH,
    ),
    wifi(
        "Qualcomm QCA6390",
        0x17cb,
        &[(0x1101, 0x1101)],
        &["ath11k_pci"],
        FW_ATH,
    ),
    wifi(
        "Qualcomm WCN6855 (QCNFA765)",
        0x17cb,
        &[(0x1103, 0x1103)],
        &["ath11k_pci"],
        FW_ATH,
    ),
    wifi(
        "Qualcomm WCN7850 (FastConnect 7800)",
        0x17cb,
        &[(0x1107, 0x1107)],
        &["ath12k"],
        FW_ATH,
    ),
    wifi("Qualcomm Wi-Fi", 0x17cb, &[], &["ath11k_pci"], FW_ATH),
    wifi(
        "MediaTek MT7921/MT7922",
        0x14c3,
        &[
            (0x0608, 0x0608),
            (0x0616, 0x0616),
            (0x7920, 0x7920),
            (0x7922, 0x7922),
            (0x7961, 0x7961),
        ],
        &["mt7921e"],
        FW_MTK,
    ),
    wifi(
        "MediaTek MT7925",
        0x14c3,
        &[(0x0717, 0x0717), (0x7925, 0x7925)],
        &["mt7925e"],
        FW_MTK,
    ),
    Entry {
        name: "MediaTek Wi-Fi",
        kind: Wifi,
        vendor: Some(0x14c3),
        class: "0280",
        firmware: FW_MTK,
        ..BASE
    },
    Entry {
        name: "Broadcom Wi-Fi",
        kind: Wifi,
        vendor: Some(0x14e4),
        class: "0280",
        drivers: &["brcmfmac", "wl", "b43", "bcma-pci-bridge"],
        firmware: &["linux-firmware-broadcom"],
        notes: &["some Broadcom cards need the broadcom-wl driver"],
        ..BASE
    },
    // ---- Ethernet ----
    Entry {
        name: "Realtek RTL8125/8126/8127 2.5/5/10GbE",
        kind: Ethernet,
        vendor: Some(0x10ec),
        devices: &[(0x8125, 0x8127), (0x5000, 0x5000)],
        class: "0200",
        drivers: &["r8169"],
        klds: &["r8169"],
        firmware: FW_REALTEK,
        ..BASE
    },
    Entry {
        name: "Realtek RTL8111/8168/8101 Ethernet",
        kind: Ethernet,
        vendor: Some(0x10ec),
        devices: &[(0x8136, 0x8136), (0x8161, 0x8161), (0x8168, 0x8169)],
        class: "0200",
        drivers: &["r8169"],
        klds: &["r8169"],
        firmware: FW_REALTEK,
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
        klds: &["igc"],
        ..BASE
    },
    Entry {
        name: "Intel Ethernet",
        kind: Ethernet,
        vendor: Some(0x8086),
        class: "0200",
        drivers: &["e1000e", "e1000", "igb", "ixgbe", "i40e", "ice"],
        ..BASE
    },
    // ---- Bluetooth (USB, btusb; firmware loaded by the kernel) ----
    bt("Intel Bluetooth", Some(0x8087), FW_INTEL),
    bt("Realtek Bluetooth", Some(0x0bda), FW_REALTEK),
    bt("Qualcomm Bluetooth", Some(0x0cf3), FW_ATH),
    bt("MediaTek Bluetooth", Some(0x0e8d), FW_MTK),
    bt("Bluetooth", None, &[]),
    // ---- Audio ----
    Entry {
        name: "Intel Smart Sound / SOF audio DSP",
        kind: Audio,
        vendor: Some(0x8086),
        class: "040380",
        drivers: &[
            "snd_hda_intel",
            "sof-audio-pci-intel-tgl",
            "sof-audio-pci-intel-mtl",
            "sof-audio-pci-intel-lnl",
            "sof-audio-pci-intel-cnl",
            "sof-audio-pci-intel-icl",
            "sof-audio-pci-intel-apl",
        ],
        firmware: &["sof-firmware"],
        ..BASE
    },
    Entry {
        name: "AMD audio coprocessor (ACP)",
        kind: Audio,
        vendor: Some(0x1022),
        class: "0480",
        drivers: &[
            "snd_acp_pci",
            "snd_pci_acp3x",
            "snd_rn_pci_acp3x",
            "snd_pci_acp5x",
            "snd_pci_acp6x",
            "snd_pci_ps",
        ],
        firmware: &["sof-firmware"],
        ..BASE
    },
    Entry {
        name: "HD Audio",
        kind: Audio,
        class: "0403",
        drivers: &["snd_hda_intel"],
        klds: &["snd_hda_intel"],
        ..BASE
    },
    // ---- Webcam (UVC); before USB audio: a camera with a mic is a camera ----
    Entry {
        name: "USB camera",
        kind: Webcam,
        bus: Bus::Usb,
        class: "0e",
        drivers: &["uvcvideo"],
        klds: &["uvcvideo"],
        notes: &["/dev/video* is for members of group video"],
        ..BASE
    },
    Entry {
        name: "USB audio",
        kind: Audio,
        bus: Bus::Usb,
        class: "01",
        drivers: &["snd-usb-audio"],
        klds: &["snd_usb_audio"],
        ..BASE
    },
    // ---- Input ----
    Entry {
        name: "I2C HID touchpad/touchscreen",
        kind: Input,
        bus: Bus::Acpi,
        class: "PNP0C50",
        klds: &["i2c_hid_acpi"],
        ..BASE
    },
    Entry {
        name: "I2C HID touchpad/touchscreen",
        kind: Input,
        bus: Bus::Acpi,
        class: "ACPI0C50",
        klds: &["i2c_hid_acpi"],
        ..BASE
    },
    Entry {
        name: "AMD I2C controller",
        kind: Input,
        bus: Bus::Acpi,
        class: "AMDI0010",
        klds: &["i2c_designware_platform"],
        ..BASE
    },
    Entry {
        name: "Intel Serial IO I2C",
        kind: Input,
        vendor: Some(0x8086),
        class: "0c80",
        drivers: &["intel-lpss"],
        klds: &["intel_lpss_pci"],
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
        drivers: &["rtsx_pci"],
        klds: &["rtsx_pci", "rtsx_pci_sdmmc"],
        ..BASE
    },
    Entry {
        name: "SD card reader",
        kind: CardReader,
        class: "0805",
        drivers: &["sdhci-pci"],
        klds: &["sdhci_pci"],
        ..BASE
    },
    // ---- Backlight (/sys/class/backlight) ----
    Entry {
        name: "Display backlight",
        kind: Backlight,
        bus: Bus::Backlight,
        notes: &["raised from 0 at boot"],
        ..BASE
    },
];

/// `wol hw report`: one computer's devices as the public hardware matrix wants them (site page
/// /hardware, docs/modules/hardware.md "Hardware reports"). Only what identifies a MODEL, never a
/// machine or a person: kind, bus, the `vvvv:dddd` id (or ACPI id / CPU vendor), the catalogue's
/// name for the row, the bound driver, the state and the missing catalogue packages. Left out on
/// purpose: `location` (bus paths), the bus's own description (USB strings can carry an owner's
/// name or a serial), hints and notes; an id or driver that is not a plain token is dropped too.
pub fn report(snap: &HardwareSnapshot, os: &str) -> serde_json::Value {
    let token = |s: &str, max: usize| {
        (!s.is_empty()
            && s.len() <= max
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_:-.".contains(&b)))
        .then(|| s.to_owned())
    };
    let devices: Vec<serde_json::Value> = snap
        .devices
        .iter()
        .map(|d| {
            let name = CATALOG.iter().find(|e| e.name == d.name).map(|e| e.name);
            serde_json::json!({
                "kind": d.kind.name(),
                "bus": token(&d.bus, 12),
                "id": token(&d.id, 16),
                "name": name,
                "driver": d.driver.as_deref().and_then(|v| token(v, 32)),
                "state": d.state.name(),
                "missing": d.missing_packages.iter().filter(|p| is_catalogue_package(p)).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::json!({ "format": 1, "os": token(os, 32), "devices": devices })
}

/// A package name the catalogue itself names (firmware or driver): the only package names a
/// report may carry.
fn is_catalogue_package(p: &str) -> bool {
    CATALOG
        .iter()
        .any(|e| e.firmware.contains(&p) || e.packages.contains(&p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::hardware::{Device, Kind, State};

    fn dev(kind: Kind, name: &str, bus: &str, id: &str, location: &str) -> Device {
        Device {
            kind,
            name: name.into(),
            bus: bus.into(),
            id: id.into(),
            location: location.into(),
            ..Default::default()
        }
    }

    #[test]
    fn report_names_models_and_carries_no_personal_data() {
        let mut wifi = dev(
            Kind::Wifi,
            "Intel Wi-Fi 6E AX210/AX211",
            "pci",
            "8086:2725",
            "0000:02:00.0",
        );
        wifi.driver = Some("iwlwifi".into());
        wifi.missing_packages = vec!["linux-firmware-intel".into(), "alice-private-pkg".into()];
        wifi.state = State::NeedsFirmware;
        wifi.hint = "install linux-firmware-intel for alice".into();
        wifi.notes = vec!["seen on alice-laptop".into()];
        // A USB device whose own description is the owner's naming, with a serial in the id slot.
        let cam = dev(
            Kind::Webcam,
            "Alice's Camera SN 0042-77",
            "usb",
            "046d:085e serial=ABC 123",
            "usb-3-2",
        );
        let snap = HardwareSnapshot {
            devices: vec![wifi, cam],
        };
        let r = report(&snap, "0.12.0");
        let text = r.to_string();
        for secret in [
            "alice",
            "Alice",
            "0000:02:00.0",
            "usb-3-2",
            "serial",
            "ABC",
            "0042",
        ] {
            assert!(!text.contains(secret), "{secret} leaked: {text}");
        }
        assert_eq!(r["format"], 1);
        assert_eq!(r["os"], "0.12.0");
        let d = &r["devices"][0];
        assert_eq!(d["kind"], "wifi");
        assert_eq!(d["id"], "8086:2725");
        assert_eq!(d["name"], "Intel Wi-Fi 6E AX210/AX211");
        assert_eq!(d["driver"], "iwlwifi");
        assert_eq!(d["state"], "needs-firmware");
        assert_eq!(d["missing"], serde_json::json!(["linux-firmware-intel"]));
        let c = &r["devices"][1];
        assert_eq!(c["kind"], "webcam");
        assert!(c["name"].is_null() && c["id"].is_null() && c["driver"].is_null());
        // Exactly these keys per device: a new Device field does not reach the report unseen.
        let keys: Vec<&str> = d.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            ["bus", "driver", "id", "kind", "missing", "name", "state"]
        );
    }

    #[test]
    fn an_empty_computer_is_an_empty_report() {
        let r = report(&HardwareSnapshot::default(), "");
        assert_eq!(r["devices"], serde_json::json!([]));
        assert!(r["os"].is_null());
    }
}
