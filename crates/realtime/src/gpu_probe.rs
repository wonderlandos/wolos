//! Find the machine's GPUs (render nodes) and the facts `gpu_select` needs, from /dev/dri and
//! /sys/class/drm (i915, xe, amdgpu, nouveau, nvidia-drm, virtio_gpu).

use crate::gpu_select::{GpuInfo, integrated_by_driver};
use std::path::Path;

/// GPUs of this machine, by render node.
pub fn gpus() -> Vec<GpuInfo> {
    gpus_in(Path::new("/"))
}

/// GPUs under `root` (a filesystem root: `/` on the machine, a fixture tree in tests).
pub fn gpus_in(root: &Path) -> Vec<GpuInfo> {
    let Ok(entries) = std::fs::read_dir(root.join("dev/dri")) else {
        return Vec::new();
    };
    let mut out: Vec<GpuInfo> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("renderD").then(|| probe(root, &name))
        })
        .collect();
    out.sort_by(|a, b| a.node.cmp(&b.node));
    out
}

fn probe(root: &Path, render: &str) -> GpuInfo {
    let drm = root.join("sys/class/drm");
    let sys = drm.join(render).join("device");
    let driver = std::fs::read_link(sys.join("driver"))
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let boot_vga = std::fs::read_to_string(sys.join("boot_vga")).is_ok_and(|s| s.trim() == "1");
    // A display is driven by this GPU if any of its card's connectors is connected.
    let drives_display = std::fs::read_dir(sys.join("drm"))
        .map(|cards| {
            cards.flatten().any(|card| {
                let card = card.file_name().to_string_lossy().into_owned();
                std::fs::read_dir(&drm).is_ok_and(|all| {
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
    let amd_is_apu = std::fs::read_to_string(sys.join("mem_info_vram_total"))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .is_some_and(|b| b <= 2 << 30);
    let card = std::fs::read_dir(sys.join("drm"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| {
            n.strip_prefix("card")
                .is_some_and(|d| d.parse::<u32>().is_ok())
        })
        .map(|c| format!("/dev/dri/{c}"))
        .unwrap_or_default();
    GpuInfo {
        node: format!("/dev/dri/{render}"),
        card,
        integrated: integrated_by_driver(&driver, amd_is_apu),
        driver,
        boot_vga,
        drives_display,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    /// A sysfs-shaped tree: (render minor, card, driver, boot_vga, connector status, vram bytes).
    type Gpu<'a> = (u32, u32, &'a str, bool, &'a str, Option<u64>);

    fn tree(gpus: &[Gpu]) -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "wolos-gpu-probe-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("dev/dri")).unwrap();
        for &(minor, card, driver, boot, status, vram) in gpus {
            fs::write(root.join(format!("dev/dri/renderD{minor}")), "").unwrap();
            fs::write(root.join(format!("dev/dri/card{card}")), "").unwrap();
            let dev = root.join(format!("sys/devices/pci0000:00/gpu{card}"));
            fs::create_dir_all(dev.join(format!("drm/card{card}"))).unwrap();
            fs::create_dir_all(dev.join(format!("drm/renderD{minor}"))).unwrap();
            fs::create_dir_all(root.join(format!("sys/bus/pci/drivers/{driver}"))).unwrap();
            symlink(
                root.join(format!("sys/bus/pci/drivers/{driver}")),
                dev.join("driver"),
            )
            .unwrap();
            fs::write(dev.join("boot_vga"), if boot { "1\n" } else { "0\n" }).unwrap();
            if let Some(v) = vram {
                fs::write(dev.join("mem_info_vram_total"), format!("{v}\n")).unwrap();
            }
            for node in [format!("renderD{minor}"), format!("card{card}")] {
                fs::create_dir_all(root.join(format!("sys/class/drm/{node}"))).unwrap();
                symlink(&dev, root.join(format!("sys/class/drm/{node}/device"))).unwrap();
            }
            let conn = root.join(format!("sys/class/drm/card{card}-eDP-1"));
            fs::create_dir_all(&conn).unwrap();
            fs::write(conn.join("status"), format!("{status}\n")).unwrap();
        }
        root
    }

    #[test]
    fn hybrid_laptop_from_sysfs() {
        let root = tree(&[
            (128, 0, "i915", true, "connected", None),
            (129, 1, "nvidia", false, "disconnected", None),
        ]);
        let g = gpus_in(&root);
        assert_eq!(
            g,
            [
                GpuInfo {
                    node: "/dev/dri/renderD128".into(),
                    card: "/dev/dri/card0".into(),
                    driver: "i915".into(),
                    boot_vga: true,
                    drives_display: true,
                    integrated: true,
                },
                GpuInfo {
                    node: "/dev/dri/renderD129".into(),
                    card: "/dev/dri/card1".into(),
                    driver: "nvidia".into(),
                    boot_vga: false,
                    drives_display: false,
                    integrated: false,
                },
            ]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn amd_apu_by_vram_and_missing_tree() {
        let root = tree(&[(128, 0, "amdgpu", true, "connected", Some(512 << 20))]);
        assert!(gpus_in(&root)[0].integrated, "APU carve-out");
        let _ = fs::remove_dir_all(root);
        assert!(gpus_in(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn simpledrm_card0_shifts_the_card() {
        // simpledrm kept card0: the i915 render node renderD128 belongs to card1.
        let root = tree(&[(128, 1, "i915", true, "connected", None)]);
        assert_eq!(gpus_in(&root)[0].card, "/dev/dri/card1");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn real_host_render_nodes_have_drivers_and_cards() {
        for g in gpus() {
            assert!(g.node.starts_with("/dev/dri/renderD"), "{g:?}");
            assert!(!g.driver.is_empty(), "{g:?}");
            assert!(g.card.starts_with("/dev/dri/card"), "{g:?}");
        }
    }
}
