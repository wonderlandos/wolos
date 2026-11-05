// SPDX-License-Identifier: BSD-2-Clause
//! Disks: read and write rates per whole device from `/proc/diskstats` (sectors are 512 bytes
//! there whatever the device's own sector size), space per mounted device from `df -kP`
//! (`system::stats::DiskUsage`, one fork, only while the Disks page is open).
use super::Root;
pub use crate::system::stats::DiskUsage;

/// Cumulative bytes of one block device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counters {
    pub name: String,
    pub read: u64,
    pub written: u64,
}

/// Read and write rates of one disk since the last sample.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Io {
    pub name: String,
    /// Bytes per second since the previous sample.
    pub read_rate: f64,
    pub write_rate: f64,
}

/// `/proc/diskstats` sector unit.
const SECTOR: u64 = 512;

/// Devices that are not disks a person has: loop images, RAM disks, the compressed swap,
/// optical and floppy drives, network block devices.
const NOT_DISKS: [&str; 6] = ["loop", "ram", "zram", "sr", "fd", "nbd"];

/// `major minor name reads merged sectors_read ms writes merged sectors_written ...`.
pub fn parse_diskstats(text: &str) -> Vec<Counters> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_ascii_whitespace().collect();
            let n = |i: usize| f.get(i)?.parse::<u64>().ok();
            Some(Counters {
                name: (*f.get(2)?).to_owned(),
                read: n(5)?.saturating_mul(SECTOR),
                written: n(9)?.saturating_mul(SECTOR),
            })
        })
        .collect()
}

/// A whole disk (in `/sys/block`, so not a partition) that is not one of [`NOT_DISKS`].
pub fn is_disk(root: &Root, name: &str) -> bool {
    !NOT_DISKS.iter().any(|p| name.starts_with(p)) && root.exists(&format!("/sys/block/{name}"))
}

/// Mounted devices with their space, one row per device (a btrfs `/` and `/home` are one disk);
/// tmpfs, overlays and the like are not listed.
pub fn mounts(root: &Root) -> Vec<DiskUsage> {
    let mut seen = std::collections::HashSet::new();
    DiskUsage::read(root)
        .into_iter()
        .filter(|d| d.device.starts_with("/dev/") && seen.insert(d.device.clone()))
        .collect()
}
