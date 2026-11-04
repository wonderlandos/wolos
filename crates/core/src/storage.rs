// SPDX-License-Identifier: BSD-2-Clause
//! Startup disk usage for Settings > General > Storage and About: the root filesystem
//! (`findmnt`), whether it sits on LUKS (`lsblk`), and the root slots kept as restore points
//! (`update::be`). Every source runs unprivileged; df is the fallback.
use crate::system::Host;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Category {
    pub name: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Storage {
    /// Device of the root filesystem, or the mount point from df.
    pub name: String,
    pub total: u64,
    pub used: u64,
    /// Largest first, empty ones dropped (none without btrfs quotas).
    pub categories: Vec<Category>,
    pub encrypted: Option<bool>,
    /// Bytes held only by snapshots.
    pub snapshots: u64,
    /// Root slots (restore points).
    pub boot_environments: u32,
    /// The WolOS startup disk (btrfs with root slots); the name is kept for the shell.
    pub zfs: bool,
}

/// `findmnt -J -b -o SOURCE,FSTYPE,SIZE,USED /`: (source without `[/subvol]`, fstype, size, used).
pub fn parse_findmnt(text: &str) -> Option<(String, String, u64, u64)> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let fs = v["filesystems"].as_array()?.first()?;
    let num = |k: &str| {
        fs[k]
            .as_u64()
            .or_else(|| fs[k].as_str().and_then(|s| s.parse().ok()))
    };
    let src = fs["source"].as_str()?;
    let src = src.split_once('[').map_or(src, |(d, _)| d);
    Some((
        src.into(),
        fs["fstype"].as_str()?.into(),
        num("size")?,
        num("used")?,
    ))
}

/// `lsblk -J -s -o NAME,TYPE <dev>` (the device and what it sits on): any `crypt` layer.
pub fn parse_encryption(text: &str) -> Option<bool> {
    fn any_crypt(v: &serde_json::Value) -> bool {
        v.as_array().is_some_and(|a| {
            a.iter()
                .any(|d| d["type"].as_str() == Some("crypt") || any_crypt(&d["children"]))
        })
    }
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let devs = &v["blockdevices"];
    if devs.as_array()?.is_empty() {
        None
    } else {
        Some(any_crypt(devs))
    }
}

pub fn read(h: &dyn Host) -> Option<Storage> {
    if let Some((dev, fstype, total, used)) = h
        .run(&["findmnt", "-J", "-b", "-o", "SOURCE,FSTYPE,SIZE,USED", "/"])
        .and_then(|t| parse_findmnt(&t))
    {
        let slots = crate::update::be::read(h).unwrap_or_default().len() as u32;
        return Some(Storage {
            encrypted: h
                .run(&["lsblk", "-J", "-s", "-o", "NAME,TYPE", &dev])
                .and_then(|t| parse_encryption(&t)),
            boot_environments: slots,
            zfs: fstype == "btrfs" && slots > 0,
            name: dev,
            total,
            used,
            ..Default::default()
        });
    }
    let root = crate::system::stats::DiskUsage::read(h)
        .into_iter()
        .find(|d| d.mount == "/")?;
    Some(Storage {
        name: root.mount,
        total: root.total.max(0) as u64,
        used: root.used.max(0) as u64,
        ..Default::default()
    })
}

/// "41.2 GB" (decimal units, like the Finder).
pub fn human(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let mut v = bytes as f64;
    let mut unit = "bytes";
    for u in ["KB", "MB", "GB", "TB", "PB"] {
        if v < 1000.0 {
            break;
        }
        v /= 1000.0;
        unit = u;
    }
    let n = if v < 100.0 {
        let s = format!("{v:.1}");
        s.strip_suffix(".0").map_or(s.clone(), str::to_string)
    } else {
        format!("{v:.0}")
    };
    format!("{n} {unit}")
}

/// A removable disk with a medium in it: its kernel name and what the label on it would say
/// (maker and model; empty when the disk does not say).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Removable {
    pub dev: String,
    pub name: String,
}

/// The removable disks under `sys_block` (`/sys/block`): the kernel says `removable` (SD readers,
/// floppy-like drives) or the device hangs off a USB bus; an empty card reader (size 0) is not
/// there until a card is. Read from sysfs, no process, so every poll can afford it.
pub fn removable_disks(sys_block: &std::path::Path) -> Vec<Removable> {
    const NOT_DISKS: [&str; 8] = ["loop", "ram", "zram", "dm-", "md", "sr", "nbd", "fd"];
    let Ok(dir) = std::fs::read_dir(sys_block) else {
        return Vec::new();
    };
    let mut out: Vec<Removable> = dir
        .flatten()
        .filter_map(|e| {
            let dev = e.file_name().to_string_lossy().into_owned();
            if NOT_DISKS.iter().any(|p| dev.starts_with(p)) {
                return None;
            }
            let path = e.path();
            let read = |f: &str| std::fs::read_to_string(path.join(f)).unwrap_or_default();
            let size: u64 = read("size").trim().parse().unwrap_or(0);
            let removable = read("removable").trim() == "1"
                || std::fs::canonicalize(&path).is_ok_and(|p| p.to_string_lossy().contains("/usb"));
            if size == 0 || !removable {
                return None;
            }
            let clean = |s: String| s.split_whitespace().collect::<Vec<_>>().join(" ");
            let name = [read("device/vendor"), read("device/model")]
                .into_iter()
                .map(clean)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            Some(Removable { dev, name })
        })
        .collect();
    out.sort_by(|a, b| a.dev.cmp(&b.dev));
    out
}

#[cfg(test)]
mod tests {
    /// A fake /sys/block: `disk(name, removable, size, vendor, model)`; `usb` puts the device under
    /// a USB bus and links it, as sysfs does.
    fn sysfs(tag: &str, disks: &[(&str, &str, &str, &str, &str, bool)]) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("wol-sysblock-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (block, devices) = (root.join("block"), root.join("devices/pci0/usb1"));
        std::fs::create_dir_all(&block).unwrap();
        std::fs::create_dir_all(&devices).unwrap();
        for (name, removable, size, vendor, model, usb) in disks {
            let real = if *usb {
                devices.join(name)
            } else {
                root.join("devices/pci0").join(name)
            };
            std::fs::create_dir_all(real.join("device")).unwrap();
            std::fs::write(real.join("removable"), format!("{removable}\n")).unwrap();
            std::fs::write(real.join("size"), format!("{size}\n")).unwrap();
            std::fs::write(real.join("device/vendor"), format!("{vendor:<8}\n")).unwrap();
            std::fs::write(real.join("device/model"), format!("{model:<16}\n")).unwrap();
            std::os::unix::fs::symlink(&real, block.join(name)).unwrap();
        }
        block
    }

    /// QA-FDAY-13: a stick plugged in said nothing. Only a removable disk with a medium counts
    /// (USB bus or the removable flag); the system disk, loop devices and an empty card reader do not.
    #[test]
    fn sticks_are_the_removable_disks_with_a_medium() {
        let block = sysfs(
            "list",
            &[
                ("nvme0n1", "0", "500000000", "", "Samsung SSD", false),
                ("sda", "0", "30000000", "Kingston", "DataTraveler 3.0", true),
                ("mmcblk0", "1", "62000000", "", "SD64G", false),
                ("sdb", "1", "0", "Generic", "Card Reader", true),
                ("loop0", "1", "100", "", "", false),
            ],
        );
        let got = removable_disks(&block);
        assert_eq!(
            got,
            vec![
                Removable {
                    dev: "mmcblk0".into(),
                    name: "SD64G".into()
                },
                Removable {
                    dev: "sda".into(),
                    name: "Kingston DataTraveler 3.0".into()
                },
            ],
            "{got:?}"
        );
        assert!(removable_disks(std::path::Path::new("/no/such/sys")).is_empty());
        let _ = std::fs::remove_dir_all(block.parent().unwrap());
    }

    use super::*;
    use crate::system::fake::FakeHost;

    const FINDMNT: &str = r#"{"filesystems": [
 {"source":"/dev/mapper/root[/@wolos-a]","fstype":"btrfs","size":497142202368,"used":131208486912}
]}"#;
    const LSBLK_LUKS: &str = r#"{"blockdevices": [
 {"name":"root","type":"crypt","children":[{"name":"nvme0n1p2","type":"part","children":[{"name":"nvme0n1","type":"disk"}]}]}
]}"#;
    const LSBLK_PLAIN: &str = r#"{"blockdevices": [{"name":"vda2","type":"part","children":[{"name":"vda","type":"disk"}]}]}"#;

    #[test]
    fn root_filesystem_from_findmnt() {
        assert_eq!(
            parse_findmnt(FINDMNT),
            Some((
                "/dev/mapper/root".into(),
                "btrfs".into(),
                497142202368,
                131208486912
            ))
        );
        // Older util-linux prints numbers as strings.
        let s =
            r#"{"filesystems":[{"source":"/dev/vda2","fstype":"ext4","size":"100","used":"40"}]}"#;
        assert_eq!(parse_findmnt(s).map(|f| (f.2, f.3)), Some((100, 40)));
        assert_eq!(parse_findmnt(""), None);
        assert_eq!(parse_findmnt(r#"{"filesystems":[]}"#), None);
    }

    #[test]
    fn encryption_from_lsblk() {
        assert_eq!(parse_encryption(LSBLK_LUKS), Some(true));
        assert_eq!(parse_encryption(LSBLK_PLAIN), Some(false));
        assert_eq!(parse_encryption(r#"{"blockdevices":[]}"#), None);
        assert_eq!(parse_encryption(""), None);
    }

    #[test]
    fn reads_the_startup_disk() {
        let h = FakeHost::new(false);
        h.command("findmnt -J -b -o SOURCE,FSTYPE,SIZE,USED /", FINDMNT)
            .command("lsblk -J -s -o NAME,TYPE /dev/mapper/root", LSBLK_LUKS)
            .command(
                "bootctl list --json=short",
                r#"[{"id":"wolos-a.conf","isSelected":true},{"id":"wolos-b.conf"}]"#,
            );
        let s = read(&h).unwrap();
        assert_eq!(
            (s.name.as_str(), s.total, s.used, s.zfs),
            ("/dev/mapper/root", 497142202368, 131208486912, true)
        );
        assert_eq!(s.encrypted, Some(true));
        assert_eq!(s.boot_environments, 2);
    }

    #[test]
    fn falls_back_to_df() {
        let df = "Filesystem 1024-blocks Used Avail Capacity Mounted on\n/dev/ada0p2 100000000 40000000 60000000 40% /\ndevfs 1 1 0 100% /dev\n";
        let h = FakeHost::new(false);
        h.command("df -kP", df);
        let s = read(&h).unwrap();
        assert_eq!(
            (s.name.as_str(), s.total, s.used, s.zfs),
            ("/", 102400000000, 40960000000, false)
        );
        assert!(s.categories.is_empty());
        assert_eq!(s.encrypted, None);
        assert_eq!(read(&FakeHost::new(false)), None);
    }

    #[test]
    fn human_sizes() {
        for (b, want) in [
            (0, "0 bytes"),
            (999, "999 bytes"),
            (512_000_000, "512 MB"),
            (41_200_000_000, "41.2 GB"),
            (9_990_000, "10 MB"),
            (9_940_000, "9.9 MB"),
            (1_500_000_000_000, "1.5 TB"),
            (123_400_000_000, "123 GB"),
            (2_000, "2 KB"),
        ] {
            assert_eq!(human(b), want, "{b}");
        }
    }
}
