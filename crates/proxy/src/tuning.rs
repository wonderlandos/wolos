//! wine defaults per GPU vendor. Defaults only: a variable already in the environment
//! (Steam launch options, the shell) wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vendor {
    Intel,
    Amd,
    Nvidia,
    Other,
}

impl Vendor {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "intel" => Vendor::Intel,
            "amd" => Vendor::Amd,
            "nvidia" => Vendor::Nvidia,
            "other" | "none" => Vendor::Other,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Vendor::Intel => "intel",
            Vendor::Amd => "amd",
            Vendor::Nvidia => "nvidia",
            Vendor::Other => "other",
        }
    }
}

/// The GPU games render on: DXVK and vkd3d-proton pick a discrete card first, so NVIDIA, then
/// AMD, then Intel. `names`: `wol_core::games::doctor::vendor_name` values.
pub fn game_vendor(names: &[&str]) -> Vendor {
    ["nvidia", "amd", "intel"]
        .iter()
        .find(|n| names.contains(n))
        .and_then(|n| Vendor::parse(n))
        .unwrap_or(Vendor::Other)
}

/// Every vendor.
pub const COMMON: [(&str, &str); 3] = [
    ("WINEDEBUG", "-all"),
    // wineserver shared memory for message queues and input (Proton's default).
    ("STAGING_SHARED_MEMORY", "1"),
    // DXVK writes `<game>_d3d11.log` next to the game otherwise.
    ("DXVK_LOG_PATH", "none"),
];
/// Mesa (Intel ANV, AMD RADV): mailbox present = no tearing without FIFO latency.
pub const MESA: [(&str, &str); 1] = [("MESA_VK_WSI_PRESENT_MODE", "mailbox")];
/// NVIDIA driver: keep the shader cache past its 128 MiB default trim (large games recompile).
pub const NVIDIA: [(&str, &str); 1] = [("__GL_SHADER_DISK_CACHE_SKIP_CLEANUP", "1")];

/// Defaults for a vendor. No sync variable: wine 11 dropped esync and uses ntsync when the kernel has it.
pub fn defaults(v: Vendor) -> Vec<(&'static str, &'static str)> {
    let mut d: Vec<(&str, &str)> = COMMON.to_vec();
    match v {
        Vendor::Intel | Vendor::Amd => d.extend(MESA),
        Vendor::Nvidia => d.extend(NVIDIA),
        Vendor::Other => {}
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_choice() {
        for (names, want) in [
            (&["intel", "nvidia"][..], Vendor::Nvidia),
            (&["amd", "intel"], Vendor::Amd),
            (&["intel"], Vendor::Intel),
            (&[], Vendor::Other),
        ] {
            assert_eq!(game_vendor(names), want, "{names:?}");
        }
        for v in [Vendor::Intel, Vendor::Amd, Vendor::Nvidia, Vendor::Other] {
            assert_eq!(Vendor::parse(v.name()), Some(v));
        }
        assert_eq!(Vendor::parse("matrox"), None);
    }

    #[test]
    fn presets_table() {
        let keys = |v| -> Vec<&str> { defaults(v).iter().map(|kv| kv.0).collect() };
        let base = ["WINEDEBUG", "STAGING_SHARED_MEMORY", "DXVK_LOG_PATH"];
        for (vendor, extra) in [
            (Vendor::Intel, &["MESA_VK_WSI_PRESENT_MODE"][..]),
            (Vendor::Amd, &["MESA_VK_WSI_PRESENT_MODE"]),
            (Vendor::Nvidia, &["__GL_SHADER_DISK_CACHE_SKIP_CLEANUP"]),
            (Vendor::Other, &[]),
        ] {
            let want: Vec<&str> = base.iter().chain(extra).copied().collect();
            assert_eq!(keys(vendor), want, "{vendor:?}");
        }
        assert!(
            !defaults(Vendor::Intel)
                .iter()
                .any(|kv| kv.0.contains("SYNC") || kv.0.starts_with("PROTON_")),
            "wine 11 picks ntsync itself; native wine reads no PROTON_* knobs"
        );
    }
}
