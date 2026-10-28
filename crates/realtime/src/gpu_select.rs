//! Which GPU composites the desktop. Pure: device facts in, choice out.
//!
//! Rule: composite on the GPU that drives the display (usually the integrated GPU / APU on hybrid
//! laptops): no per-frame copies between GPUs and lower power. Apps may still render on a discrete
//! GPU (PRIME). Without a display, prefer integrated. `WOL_GPU=<node>` overrides.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuInfo {
    /// Render node, e.g. /dev/dri/renderD128.
    pub node: String,
    /// Its KMS node, e.g. /dev/dri/card1 (not always render minus 128: simpledrm may hold card0).
    /// Empty when unknown.
    pub card: String,
    /// Kernel driver of the PCI device, e.g. i915, xe, amdgpu, nvidia, virtio-pci.
    pub driver: String,
    /// Firmware marked it as the boot display GPU (boot_vga).
    pub boot_vga: bool,
    /// A connected display hangs off this GPU.
    pub drives_display: bool,
    /// Integrated in the CPU package (iGPU / APU).
    pub integrated: bool,
}

pub fn choose<'a>(gpus: &'a [GpuInfo], override_node: Option<&str>) -> Option<&'a GpuInfo> {
    if let Some(node) = override_node {
        return gpus.iter().find(|g| g.node == node);
    }
    gpus.iter().max_by_key(|g| {
        (
            g.drives_display,
            g.boot_vga,
            g.integrated,
            std::cmp::Reverse(g.node.clone()),
        )
    })
}

/// Integrated-GPU heuristic from the driver: Intel graphics are integrated; AMD may be either
/// (APU vs discrete), so the caller passes what the platform reports for AMD.
pub fn integrated_by_driver(driver: &str, amd_is_apu: bool) -> bool {
    match driver {
        "i915" | "xe" => true,
        "amdgpu" => amd_is_apu,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(
        node: &str,
        driver: &str,
        boot_vga: bool,
        drives_display: bool,
        integrated: bool,
    ) -> GpuInfo {
        GpuInfo {
            node: node.into(),
            card: String::new(),
            driver: driver.into(),
            boot_vga,
            drives_display,
            integrated,
        }
    }

    #[test]
    fn hybrid_laptop_composites_on_the_igpu_driving_the_panel() {
        let gpus = [
            gpu("/dev/dri/renderD129", "nvidia-drm", false, false, false),
            gpu("/dev/dri/renderD128", "i915", true, true, true),
        ];
        assert_eq!(choose(&gpus, None).unwrap().driver, "i915");
    }

    #[test]
    fn desktop_with_monitor_on_the_discrete_gpu_composites_there() {
        let gpus = [
            gpu("/dev/dri/renderD128", "i915", false, false, true),
            gpu("/dev/dri/renderD129", "amdgpu", true, true, false),
        ];
        assert_eq!(choose(&gpus, None).unwrap().driver, "amdgpu");
    }

    #[test]
    fn headless_prefers_integrated_for_power() {
        let gpus = [
            gpu("/dev/dri/renderD128", "amdgpu", false, false, false),
            gpu("/dev/dri/renderD129", "amdgpu", false, false, true),
        ];
        assert_eq!(choose(&gpus, None).unwrap().node, "/dev/dri/renderD129");
    }

    #[test]
    fn override_wins_and_unknown_override_gives_none() {
        let gpus = [
            gpu("/dev/dri/renderD128", "i915", true, true, true),
            gpu("/dev/dri/renderD129", "amdgpu", false, false, false),
        ];
        assert_eq!(
            choose(&gpus, Some("/dev/dri/renderD129")).unwrap().driver,
            "amdgpu"
        );
        assert!(choose(&gpus, Some("/dev/dri/renderD200")).is_none());
        assert!(choose(&[], None).is_none());
    }

    #[test]
    fn intel_is_integrated_amd_depends_on_apu() {
        assert!(integrated_by_driver("i915", false));
        assert!(integrated_by_driver("xe", false));
        assert!(integrated_by_driver("amdgpu", true));
        assert!(!integrated_by_driver("amdgpu", false));
        assert!(!integrated_by_driver("nvidia-drm", false));
    }
}
