//! Linux fixtures recorded on an Alder Lake-P laptop (ThinkPad-class, Arch kernel 6.x): sysfs ids,
//! driver links, `lspci -mm`, `/proc/modules`, `modules.dep`, `/proc/cpuinfo`.
use crate::system::fake::FakeHost;

pub const LSPCI_MM_ADL: &str = r#"00:00.0 "Host bridge" "Intel Corporation" "Alder Lake-U15 Host and DRAM Controller" -r04 -p00 "Lenovo" "Device 3805"
00:02.0 "VGA compatible controller" "Intel Corporation" "Alder Lake-UP3 GT2 [Iris Xe Graphics]" -r0c -p00 "Lenovo" "Device 3b3a"
00:0d.0 "USB controller" "Intel Corporation" "Alder Lake-P Thunderbolt 4 USB Controller" -r04 -p30 "Lenovo" "Device 3824"
00:14.3 "Network controller" "Intel Corporation" "Alder Lake-P PCH CNVi WiFi" -r01 -p00 "Intel Corporation" "Dual Band Wi-Fi 5(802.11ac) Wireless-AC 9560 80MHz 2x2 [Jefferson Peak]"
00:15.0 "Serial bus controller" "Intel Corporation" "Alder Lake PCH Serial IO I2C Controller #0" -r01 -p00 "Lenovo" "Device 3812"
00:1f.3 "Audio device" "Intel Corporation" "Alder Lake PCH-P High Definition Audio Controller" -r01 -p80 "Lenovo" "Device 3881"
01:00.0 "Non-Volatile memory controller" "SK hynix" "BC901 NVMe Solid State Drive (DRAM-less)" -r03 -p02 "SK hynix" "BC901 NVMe Solid State Drive (DRAM-less)"
02:00.0 "SD Host controller" "Genesys Logic, Inc" "GL9750 SD Host Controller" -r01 -p01 "Lenovo" "Device 384a"
"#;

/// (address, vendor, device, class, driver link basename or "").
pub const PCI_ADL: &[(&str, &str, &str, &str, &str)] = &[
    ("0000:00:00.0", "0x8086", "0x4601", "0x060000", "igen6_edac"),
    ("0000:00:02.0", "0x8086", "0x46a8", "0x030000", "i915"),
    ("0000:00:0d.0", "0x8086", "0x461e", "0x0c0330", "xhci_hcd"),
    ("0000:00:14.3", "0x8086", "0x51f0", "0x028000", "iwlwifi"),
    ("0000:00:15.0", "0x8086", "0x51e8", "0x0c8000", "intel-lpss"),
    (
        "0000:00:1f.3",
        "0x8086",
        "0x51c8",
        "0x040380",
        "snd_hda_intel",
    ),
    ("0000:01:00.0", "0x1c5c", "0x1d59", "0x010802", "nvme"),
    ("0000:02:00.0", "0x17a0", "0x9750", "0x080501", "sdhci-pci"),
];

/// USB devices: (name, idVendor, idProduct, class/sub/proto, product); interfaces:
/// (name, class/sub/proto, driver).
pub const USB_ADL: &[(&str, &str, &str, &str, &str)] = &[
    ("usb3", "1d6b", "0002", "090001", "xHCI Host Controller"),
    ("3-10", "8087", "0aaa", "e00101", ""),
    ("3-8", "04f2", "b78e", "ef0201", "Integrated Camera"),
    ("3-9.4", "046d", "c00e", "000000", "USB-PS/2 Optical Mouse"),
];
pub const USB_IF_ADL: &[(&str, &str, &str)] = &[
    ("3-0:1.0", "090000", "hub"),
    ("3-10:1.0", "e00101", "btusb"),
    ("3-10:1.1", "e00101", "btusb"),
    ("3-8:1.0", "0e0100", "uvcvideo"),
    ("3-8:1.1", "0e0200", "uvcvideo"),
    ("3-8:1.2", "fe0101", ""),
    ("3-9.4:1.0", "030102", "usbhid"),
];

pub const PROC_MODULES: &str = "\
uvcvideo 151552 0 - Live 0x0000000000000000
btusb 77824 0 - Live 0x0000000000000000
iwlwifi 581632 1 iwlmvm, Live 0x0000000000000000
i915 4612096 38 - Live 0x0000000000000000
snd_hda_intel 69632 3 - Live 0x0000000000000000
snd_sof_pci_intel_tgl 12288 0 - Live 0x0000000000000000
intel_lpss_pci 28672 0 - Live 0x0000000000000000
sdhci_pci 86016 0 - Live 0x0000000000000000
";

pub const MODULES_DEP: &str = "\
kernel/drivers/gpu/drm/i915/i915.ko.zst: kernel/drivers/gpu/drm/display/drm_display_helper.ko.zst
kernel/drivers/gpu/drm/xe/xe.ko.zst:
kernel/drivers/net/wireless/intel/iwlwifi/iwlwifi.ko.zst: kernel/net/wireless/cfg80211.ko.zst
kernel/drivers/net/wireless/realtek/rtw88/rtw88_8822ce.ko.zst:
kernel/drivers/hid/i2c-hid/i2c-hid-acpi.ko.zst: kernel/drivers/hid/i2c-hid/i2c-hid.ko.zst
kernel/drivers/hwmon/coretemp.ko.zst:
kernel/drivers/media/usb/uvc/uvcvideo.ko.zst:
kernel/drivers/bluetooth/btusb.ko.zst:
kernel/drivers/mmc/host/sdhci-pci.ko.zst:
kernel/drivers/mfd/intel-lpss-pci.ko.zst:
kernel/sound/pci/hda/snd-hda-intel.ko.zst:
extramodules/nvidia-drm.ko.zst: extramodules/nvidia-modeset.ko.zst
";

pub const CPUINFO_ADL: &str = "\
processor\t: 0
vendor_id\t: GenuineIntel
cpu family\t: 6
model\t\t: 154
model name\t: 12th Gen Intel(R) Core(TM) i5-1235U
flags\t\t: fpu vme hwp hwp_epp

processor\t: 1
vendor_id\t: GenuineIntel
model name\t: 12th Gen Intel(R) Core(TM) i5-1235U
";

pub const RELEASE: &str = "6.17.1-wolos";

/// Fill `h` with a PCI/USB/ACPI tree.
pub fn sysfs(
    h: &FakeHost,
    pci: &[(&str, &str, &str, &str, &str)],
    usb: &[(&str, &str, &str, &str, &str)],
    ifs: &[(&str, &str, &str)],
) {
    for (a, v, d, c, drv) in pci {
        let p = format!("/sys/bus/pci/devices/{a}");
        h.file(&format!("{p}/vendor"), &format!("{v}\n"))
            .file(&format!("{p}/device"), &format!("{d}\n"))
            .file(&format!("{p}/class"), &format!("{c}\n"));
        if !drv.is_empty() {
            h.link(
                &format!("{p}/driver"),
                &format!("../../../bus/pci/drivers/{drv}"),
            );
        }
    }
    for (n, v, d, c, prod) in usb {
        let p = format!("/sys/bus/usb/devices/{n}");
        h.file(&format!("{p}/idVendor"), &format!("{v}\n"))
            .file(&format!("{p}/idProduct"), &format!("{d}\n"))
            .file(&format!("{p}/bDeviceClass"), &format!("{}\n", &c[..2]))
            .file(&format!("{p}/bDeviceSubClass"), &format!("{}\n", &c[2..4]))
            .file(&format!("{p}/bDeviceProtocol"), &format!("{}\n", &c[4..]));
        if !prod.is_empty() {
            h.file(&format!("{p}/product"), &format!("{prod}\n"));
        }
    }
    for (n, c, drv) in ifs {
        let p = format!("/sys/bus/usb/devices/{n}");
        h.file(&format!("{p}/bInterfaceClass"), &format!("{}\n", &c[..2]))
            .file(
                &format!("{p}/bInterfaceSubClass"),
                &format!("{}\n", &c[2..4]),
            )
            .file(
                &format!("{p}/bInterfaceProtocol"),
                &format!("{}\n", &c[4..]),
            );
        if !drv.is_empty() {
            h.link(
                &format!("{p}/driver"),
                &format!("../../../../bus/usb/drivers/{drv}"),
            );
        }
    }
}

/// The laptop, with `installed` packages (`pacman -Qq`).
pub fn adl_host(installed: &str) -> FakeHost {
    let h = FakeHost::linux();
    sysfs(&h, PCI_ADL, USB_ADL, USB_IF_ADL);
    h.command("lspci -mm", LSPCI_MM_ADL)
        .command("pacman -Qq", installed)
        .file("/proc/modules", PROC_MODULES)
        .file("/proc/cpuinfo", CPUINFO_ADL)
        .file("/proc/sys/kernel/osrelease", &format!("{RELEASE}\n"))
        .file(
            &format!("/usr/lib/modules/{RELEASE}/modules.dep"),
            MODULES_DEP,
        )
        .file(
            &format!("/usr/lib/modules/{RELEASE}/modules.builtin"),
            "kernel/drivers/acpi/video.ko\n",
        )
        .file(
            "/sys/devices/system/cpu/cpu0/cpufreq/scaling_driver",
            "intel_pstate\n",
        )
        .file(
            "/sys/bus/acpi/devices/SYNA2BA6:00/modalias",
            "acpi:SYNA2BA6:PNP0C50:\n",
        )
        .file(
            "/sys/bus/acpi/devices/ACPI0003:00/modalias",
            "acpi:ACPI0003:\n",
        )
        .file(
            "/sys/class/backlight/intel_backlight/max_brightness",
            "19393\n",
        );
    h
}
