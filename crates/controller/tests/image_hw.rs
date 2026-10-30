//! Hardware catalogue vs the Arch image: drivers the old boot scripts chose are still in the
//! catalogue (udev autoloads them by modalias), and every catalogue package ships offline
//! (`image/arch/packages.x86_64`, `desktop/packages.txt`) or is listed as not shipped.
use std::{fs, path::PathBuf};
use wol_core::hardware::{Bus, catalog::CATALOG, lookup};

fn read(p: &str) -> String {
    let f = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p);
    fs::read_to_string(&f).unwrap_or_else(|e| panic!("{p}: {e}"))
}

#[test]
fn catalogue_keeps_the_drivers_the_boot_scripts_loaded() {
    for (vendor, device, class, module) in [
        (0x8086, 0x46a8, "030000", "i915"),
        (0x8086, 0x46a8, "038000", "i915"), // passed-through iGPU, display class 0x0380
        (0x1002, 0x15bf, "030000", "amdgpu"),
        (0x10de, 0x2520, "030000", "nvidia_drm"),
    ] {
        let e = lookup(Bus::Pci, Some(vendor), Some(device), &[class]).unwrap();
        assert!(
            e.klds.contains(&module),
            "{vendor:04x}:{device:04x} -> {:?}",
            e.klds
        );
    }
}

/// Catalogue packages deliberately not in the offline image.
const NOT_SHIPPED: &[(&str, &str)] = &[
    ("nvidia-open", "large; wolos-firstboot installs it online"),
    ("nvidia-utils", "comes with nvidia-open online"),
];

/// `linux-firmware` is a meta package over these split firmware packages.
const LINUX_FIRMWARE_PARTS: &[&str] = &[
    "linux-firmware-amdgpu",
    "linux-firmware-atheros",
    "linux-firmware-broadcom",
    "linux-firmware-intel",
    "linux-firmware-mediatek",
    "linux-firmware-nvidia",
    "linux-firmware-radeon",
    "linux-firmware-realtek",
];

fn shipped() -> Vec<String> {
    let mut v = Vec::new();
    for f in ["image/arch/packages.x86_64", "desktop/packages.txt"] {
        for l in read(f).lines() {
            let l = l.split('#').next().unwrap().trim();
            if !l.is_empty() {
                v.push(l.to_owned());
            }
        }
    }
    v
}

#[test]
fn every_catalogue_package_ships_offline_or_is_listed_as_not_shipped() {
    let pkgs = shipped();
    let has = |p: &str| pkgs.iter().any(|x| x == p);
    let covered = |p: &str| has(p) || (LINUX_FIRMWARE_PARTS.contains(&p) && has("linux-firmware"));
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
        "linux-firmware",
        "intel-ucode",
        "amd-ucode",
        "sof-firmware",
        "vulkan-intel",
        "vulkan-radeon",
    ] {
        assert!(has(want), "{want}");
    }
    assert!(
        !pkgs.iter().any(|p| p.starts_with("nvidia-")),
        "NVIDIA stays an online install"
    );
}

/// An iPhone plugged in shows its photos and files, shares its internet and pairs; AirPlay
/// speakers show up as outputs. All from the official repos, nothing to set up.
#[test]
fn iphone_and_airplay_work_offline() {
    let pkgs = shipped();
    for p in [
        "usbmuxd",
        "libimobiledevice",
        "ifuse",
        "gvfs-afc",
        "libheif",
        "pipewire-zeroconf",
    ] {
        assert!(pkgs.iter().any(|x| x == p), "{p} ships offline");
    }
}

/// One sound server: JACK apps, cameras (V4L2 apps), GStreamer players and AirPlay from an
/// iPhone all go through PipeWire, so one volume and one device list cover everything.
#[test]
fn pipewire_carries_all_audio_and_video() {
    let pkgs = shipped();
    for p in [
        "pipewire-jack",
        "pipewire-v4l2",
        "pipewire-libcamera",
        "gst-plugin-pipewire",
        "shairport-sync",
    ] {
        assert!(pkgs.iter().any(|x| x == p), "{p} ships offline");
    }
    for p in ["jack2", "pulseaudio"] {
        assert!(!pkgs.iter().any(|x| x == p), "{p} would be a second sound server");
    }
    let f = "image/files/usr/share/pipewire/pipewire.conf.d/71-wolos-airplay.conf";
    let conf = std::fs::read_to_string(format!("{}/../../{f}", env!("CARGO_MANIFEST_DIR")))
        .unwrap_or_default();
    assert!(conf.contains("libpipewire-module-raop-discover"), "{f}: AirPlay speakers listed");
}
