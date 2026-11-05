// SPDX-License-Identifier: BSD-2-Clause
//! Graphics cards from `/sys/class/drm/cardN` (connectors `cardN-DP-1` and render nodes are
//! skipped). Load and video memory are what the driver exposes: amdgpu has both, i915 and
//! nouveau have neither, and the page says so instead of drawing zero.
use super::Root;

/// One graphics card as the kernel reports it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Gpu {
    /// `card0`.
    pub card: String,
    /// Kernel driver (`DRIVER=` in the device's uevent): amdgpu, i915, nouveau, ...
    pub driver: String,
    /// Percent busy (`gpu_busy_percent`), when the driver reports it.
    pub busy: Option<f64>,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
}

/// `DRIVER=` of a uevent file.
pub fn driver(uevent: &str) -> Option<&str> {
    uevent
        .lines()
        .find_map(|l| l.strip_prefix("DRIVER="))
        .map(str::trim)
}

fn card(name: &str) -> bool {
    name.strip_prefix("card")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
}

/// The graphics cards under `/sys/class/drm`, with their busy figure where the driver gives one.
pub fn read(root: &Root) -> Vec<Gpu> {
    root.names("/sys/class/drm")
        .into_iter()
        .filter(|n| card(n))
        .map(|n| {
            let dev = format!("/sys/class/drm/{n}/device");
            let num = |f: &str| {
                root.read(&format!("{dev}/{f}"))
                    .and_then(|t| t.trim().parse::<u64>().ok())
            };
            Gpu {
                driver: root
                    .read(&format!("{dev}/uevent"))
                    .as_deref()
                    .and_then(driver)
                    .unwrap_or_default()
                    .to_owned(),
                busy: num("gpu_busy_percent").map(|b| b.min(100) as f64),
                vram_used: num("mem_info_vram_used"),
                vram_total: num("mem_info_vram_total").filter(|t| *t > 0),
                card: n,
            }
        })
        .collect()
}
