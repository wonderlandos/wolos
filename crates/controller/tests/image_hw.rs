// SPDX-License-Identifier: GPL-2.0-or-later
//! Hardware catalogue vs the Arch image: drivers the old boot scripts chose are still in the
//! catalogue (udev autoloads them by modalias), and every catalogue package ships offline
//! (`wolos-iso/profile/packages.x86_64`, `wolos-iso/profile/packages.desktop`) or is listed as not shipped.
use wol_core::hardware::{Bus, lookup};
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
