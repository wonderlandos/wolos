//! Find the machine's GPUs (render nodes) and the facts `gpu_select` needs.

use crate::gpu_select::{GpuInfo, integrated_by_driver};

/// PCI (vendor, device) of a DRM minor, when drm-kmod reports it.
pub type PciId = Option<(u16, u16)>;

#[cfg(target_os = "freebsd")]
pub fn gpus() -> Vec<GpuInfo> {
    drm_kmod_gpus()
}

/// FreeBSD (drm-kmod): `pciconf -lv` facts + `hw.pci.default_vgapci_unit`, render nodes in
/// /dev/dri matched to their PCI device through `sysctl dev.drm`. Compiled on every host.
#[cfg_attr(not(target_os = "freebsd"), allow(dead_code))]
fn drm_kmod_gpus() -> Vec<GpuInfo> {
    let run = |argv: &[&str]| {
        std::process::Command::new(argv[0])
            .args(&argv[1..])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    let boot = run(&["sysctl", "-n", "hw.pci.default_vgapci_unit"])
        .trim()
        .parse()
        .ok();
    let names: Vec<String> = std::fs::read_dir("/dev/dri")
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let nodes = render_nodes(&names, &run(&["sysctl", "dev.drm"]));
    parse_pciconf(&run(&["pciconf", "-lv"]), boot, &nodes)
}

/// Render minors present in /dev/dri (`names`), ascending, with the PCI (vendor, device) drm-kmod
/// reports for each in `sysctl dev.drm` (`dev.drm.<minor>.PCI_ID: 8086:9a49`).
pub fn render_nodes(names: &[String], dev_drm: &str) -> Vec<(u32, PciId)> {
    let id = |minor: u32| {
        dev_drm.lines().find_map(|l| {
            let (k, v) = l.split_once(": ")?;
            (k.strip_prefix("dev.drm.")?.strip_suffix(".PCI_ID")? == minor.to_string())
                .then_some(())?;
            let (vid, pid) = v.trim().split_once(':')?;
            Some((
                u16::from_str_radix(vid, 16).ok()?,
                u16::from_str_radix(pid, 16).ok()?,
            ))
        })
    };
    let mut minors: Vec<u32> = names
        .iter()
        .filter_map(|n| n.strip_prefix("renderD")?.parse().ok())
        .collect();
    minors.sort_unstable();
    minors.into_iter().map(|m| (m, id(m))).collect()
}

/// Linux: /sys/class/drm.
#[cfg(not(target_os = "freebsd"))]
pub fn gpus() -> Vec<GpuInfo> {
    let Ok(entries) = std::fs::read_dir("/dev/dri") else {
        return Vec::new();
    };
    let mut out: Vec<GpuInfo> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("renderD").then(|| probe(&name))
        })
        .collect();
    out.sort_by(|a, b| a.node.cmp(&b.node));
    out
}

#[cfg(not(target_os = "freebsd"))]
fn probe(render: &str) -> GpuInfo {
    let sys = format!("/sys/class/drm/{render}/device");
    let driver = std::fs::read_link(format!("{sys}/driver"))
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let boot_vga =
        std::fs::read_to_string(format!("{sys}/boot_vga")).is_ok_and(|s| s.trim() == "1");
    // A display is driven by this GPU if any of its card's connectors is connected.
    let drives_display = std::fs::read_dir(format!("{sys}/drm"))
        .map(|cards| {
            cards.flatten().any(|card| {
                let card = card.file_name().to_string_lossy().into_owned();
                std::fs::read_dir("/sys/class/drm").is_ok_and(|all| {
                    all.flatten().any(|c| {
                        let n = c.file_name().to_string_lossy().into_owned();
                        n.starts_with(&format!("{card}-"))
                            && std::fs::read_to_string(c.path().join("status"))
                                .is_ok_and(|s| s.trim() == "connected")
                    })
                })
            })
        })
        .unwrap_or(false);
    // AMD: APUs report no dedicated VRAM beyond a small carve-out; treat <= 2 GiB as integrated.
    let amd_is_apu = std::fs::read_to_string(format!("{sys}/mem_info_vram_total"))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .is_some_and(|b| b <= 2 << 30);
    GpuInfo {
        node: format!("/dev/dri/{render}"),
        integrated: integrated_by_driver(&driver, amd_is_apu),
        driver,
        boot_vga,
        drives_display,
    }
}

/// GPUs from `pciconf -lv` (display class 0x03) that have a render node in `nodes`
/// ([`render_nodes`]): the node with the GPU's PCI id, else the next node without an id, in PCI
/// order. A GPU without a node (no driver attached) is left out.
/// `boot_unit`: `hw.pci.default_vgapci_unit` (the vgapciN the firmware booted on).
/// Display connection is not visible here; the boot GPU stands in for it.
pub fn parse_pciconf(text: &str, boot_unit: Option<u32>, nodes: &[(u32, PciId)]) -> Vec<GpuInfo> {
    let mut found: Vec<(PciId, GpuInfo)> = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((name, rest)) = line.split_once('@') else {
            continue;
        };
        if !rest.contains("class=0x03") {
            continue;
        }
        let field = |k: &str| {
            rest.split_whitespace()
                .find_map(|f| f.strip_prefix(k))
                .unwrap_or("")
        };
        let vendor = field("vendor=");
        let mut desc = String::new();
        while let Some(l) = lines.peek().filter(|l| l.starts_with(char::is_whitespace)) {
            if let Some(d) = l
                .trim()
                .strip_prefix("device")
                .and_then(|d| d.trim_start().strip_prefix('='))
            {
                desc = d.trim().trim_matches('\'').to_owned();
            }
            lines.next();
        }
        let driver = match vendor {
            "0x8086" => "i915",
            "0x1002" => "amdgpu",
            "0x10de" => "nvidia-drm",
            _ => "unknown",
        };
        const APU: [&str; 12] = [
            "Renoir",
            "Cezanne",
            "Rembrandt",
            "Phoenix",
            "Raphael",
            "Picasso",
            "Raven",
            "Lucienne",
            "Barcelo",
            "Mendocino",
            "Hawk Point",
            "Strix",
        ];
        let apu = APU.iter().any(|a| desc.contains(a));
        let unit: Option<u32> = name.strip_prefix("vgapci").and_then(|u| u.parse().ok());
        let boot_vga = unit.is_some() && unit == boot_unit;
        let hex = |k: &str| u16::from_str_radix(field(k).trim_start_matches("0x"), 16).ok();
        let pci = hex("vendor=").zip(hex("device="));
        found.push((
            pci,
            GpuInfo {
                node: String::new(),
                integrated: integrated_by_driver(driver, apu),
                driver: driver.into(),
                boot_vga,
                drives_display: boot_vga,
            },
        ));
    }
    let mut free: Vec<bool> = vec![true; nodes.len()];
    let mut take = |want: &dyn Fn(&PciId) -> bool| {
        let i = (0..nodes.len()).find(|&i| free[i] && want(&nodes[i].1))?;
        free[i] = false;
        Some(nodes[i].0)
    };
    // Exact PCI id first (for every GPU), then nodes without an id in PCI order.
    let exact: Vec<Option<u32>> = found
        .iter()
        .map(|(pci, _)| take(&|n| pci.is_some() && n == pci))
        .collect();
    found
        .into_iter()
        .zip(exact)
        .filter_map(|((_, mut g), minor)| {
            let minor = minor.or_else(|| take(&|n| n.is_none()))?;
            g.node = format!("/dev/dri/renderD{minor}");
            Some(g)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HYBRID: &str = "hostb0@pci0:0:0:0:\tclass=0x060000 rev=0x01 hdr=0x00 vendor=0x8086 device=0x9a14
    vendor     = 'Intel Corporation'
vgapci0@pci0:0:2:0:\tclass=0x030000 rev=0x01 hdr=0x00 vendor=0x8086 device=0x9a49 subvendor=0x1043 subdevice=0x1e02
    vendor     = 'Intel Corporation'
    device     = 'TigerLake-LP GT2 [Iris Xe Graphics]'
    class      = display
    subclass   = VGA
vgapci1@pci0:1:0:0:\tclass=0x030200 rev=0xa1 hdr=0x00 vendor=0x10de device=0x25a2
    vendor     = 'NVIDIA Corporation'
    device     = 'GA107M [GeForce RTX 3050 Mobile]'
    class      = display
vgapci0@pci0:5:0:0:\tclass=0x030000 rev=0xc5 hdr=0x00 vendor=0x1002 device=0x1638
    vendor     = 'Advanced Micro Devices, Inc. [AMD/ATI]'
    device     = 'Cezanne [Radeon Vega Series / Radeon Vega Mobile Series]'
";

    /// Render nodes that exist and carry no PCI id: PCI order (renderD128 + i).
    const ANON: [(u32, PciId); 3] = [(128, None), (129, None), (130, None)];

    #[test]
    fn pciconf_table() {
        let g = parse_pciconf(HYBRID, Some(0), &ANON);
        let got: Vec<_> = g
            .iter()
            .map(|g| (g.node.as_str(), g.driver.as_str(), g.integrated, g.boot_vga))
            .collect();
        assert_eq!(
            got,
            vec![
                ("/dev/dri/renderD128", "i915", true, true),
                ("/dev/dri/renderD129", "nvidia-drm", false, false),
                ("/dev/dri/renderD130", "amdgpu", true, true),
            ]
        );
        assert!(parse_pciconf("", None, &ANON).is_empty());
        assert!(
            parse_pciconf(HYBRID, None, &ANON)
                .iter()
                .all(|g| !g.boot_vga)
        );
        assert_eq!(
            crate::gpu_select::choose(&g[..2], None).unwrap().driver,
            "i915"
        );
    }

    /// drm-kmod names each minor's PCI device (`dev.drm.<minor>.PCI_ID`, unpadded hex); the kernel
    /// numbers minors in driver load order, not PCI order (nvidia-drm can load before i915kms).
    #[test]
    fn render_nodes_follow_the_drm_pci_ids() {
        let sysctl = "dev.drm.129.PCI_ID: 8086:9a49\ndev.drm.1.PCI_ID: 8086:9a49\n\
dev.drm.128.PCI_ID: 10de:25a2\ndev.drm.0.PCI_ID: 10de:25a2\ndev.drm.0.%desc: x\n";
        let names = ["card0", "card1", "renderD128", "renderD129", "renderD12x"].map(String::from);
        let nodes = render_nodes(&names, sysctl);
        assert_eq!(
            nodes,
            vec![(128, Some((0x10de, 0x25a2))), (129, Some((0x8086, 0x9a49)))]
        );
        let two = |text: &str, nodes: &[(u32, PciId)]| {
            parse_pciconf(text, Some(0), nodes)
                .into_iter()
                .map(|g| (g.node, g.driver))
                .collect::<Vec<_>>()
        };
        let laptop = &HYBRID[..HYBRID.find("vgapci0@pci0:5").unwrap()];
        let n = |s: &str, d: &str| (s.to_string(), d.to_string());
        assert_eq!(
            two(laptop, &nodes),
            vec![
                n("/dev/dri/renderD129", "i915"),
                n("/dev/dri/renderD128", "nvidia-drm")
            ]
        );
        // NVIDIA driver not installed yet: its GPU has no render node and is left out.
        assert_eq!(
            two(laptop, &[(128, Some((0x8086, 0x9a49)))]),
            vec![n("/dev/dri/renderD128", "i915")]
        );
        // No DRM driver loaded: no nodes, no GPUs (the compositor finds its own).
        assert!(two(laptop, &[]).is_empty());
        // Ids unknown for some nodes: matched ones first, the rest in PCI order.
        assert_eq!(
            two(laptop, &[(128, None), (129, Some((0x8086, 0x9a49)))]),
            vec![
                n("/dev/dri/renderD129", "i915"),
                n("/dev/dri/renderD128", "nvidia-drm")
            ]
        );
    }
}
