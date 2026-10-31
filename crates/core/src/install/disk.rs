// SPDX-License-Identifier: GPL-2.0-or-later
//! What is on a disk, as the installer sees it (ADR 0023): the partition table from `sfdisk --json`,
//! file systems and mounts from `lsblk --json`, and read-only facts a probe adds (how small a file
//! system can get, what system lives on it). Partitions are named by their GPT UUID, never by a
//! number or a device path: those change under a plan, the UUID does not.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MIB: u64 = 1 << 20;
pub const GIB: u64 = 1 << 30;

pub const ESP_TYPE: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
pub const ROOT_TYPE: &str = "4f68bce3-e8cd-4db1-96e7-fbcaf984b709";
pub const HOME_TYPE: &str = "933ac7e1-2eb4-4f13-b844-0e14e2aef915";
pub const VAR_TYPE: &str = "4d21b016-b534-45c2-a9fb-5c16e091fd2d";
pub const SWAP_TYPE: &str = "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f";
pub const DATA_TYPE: &str = "0fc63daf-8483-4772-8e79-3d69d8477de4";
const WINDOWS_TYPES: [&str; 2] = [
    "e3c9e316-0b5c-4db8-817d-f92df00215ae",
    "de94bba4-06d1-4d40-a16a-bfd50179d6ac",
];
const MACOS_TYPES: [&str; 2] = [
    "48465300-0000-11aa-aa11-00306543ecac",
    "7c3457ef-0000-11aa-aa11-00306543ecac",
];
const FREEBSD_TYPES: [&str; 2] = [
    "516e7cb6-6ecf-11d6-8ff8-00022d09712b",
    "516e7cba-6ecf-11d6-8ff8-00022d09712b",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Table {
    /// Blank disk: no partition table at all.
    #[default]
    None,
    Gpt,
    /// MBR or anything else: only Erase can use it.
    Other,
}

/// Why a file system can't be made smaller from here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Blocked {
    /// BitLocker: Windows has to turn it off first.
    BitLocker,
    /// Hibernated, Fast Startup or an unclean journal: Windows has to shut down fully.
    NotShutDown,
    /// APFS and HFS+: only macOS resizes these.
    MacOs,
    /// The probe could not read it.
    Unreadable,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    pub number: u32,
    pub path: String,
    /// In sectors of [`Disk::sector`].
    pub start: u64,
    pub sectors: u64,
    /// GPT type, lowercase.
    pub kind: String,
    /// GPT partition UUID, lowercase: the name a plan uses.
    pub uuid: String,
    pub name: String,
    pub fstype: String,
    pub label: String,
    pub fsuuid: String,
    /// It, or something inside it, is mounted.
    pub mounted: bool,
    /// Smallest size the file system can shrink to, bytes (probe).
    pub min_bytes: Option<u64>,
    /// Bytes in use (probe).
    pub used_bytes: Option<u64>,
    /// Bytes free inside the file system (probe, ESPs).
    pub free_bytes: Option<u64>,
    /// Pretty name of the Linux system inside ("Ubuntu 24.04.1 LTS") (probe).
    pub os: Option<String>,
    pub blocked: Option<Blocked>,
}

impl Part {
    pub fn is_esp(&self) -> bool {
        self.kind == ESP_TYPE
    }

    /// The system this partition belongs to, as named for people.
    pub fn system(&self) -> Option<String> {
        let (t, f) = (self.kind.as_str(), self.fstype.as_str());
        if WINDOWS_TYPES.contains(&t) || matches!(f, "ntfs" | "BitLocker") {
            Some("Windows".into())
        } else if MACOS_TYPES.contains(&t) || matches!(f, "apfs" | "hfsplus") {
            Some("macOS".into())
        } else if FREEBSD_TYPES.contains(&t) || matches!(f, "zfs_member" | "ufs") {
            Some("FreeBSD".into())
        } else if let Some(os) = &self.os {
            Some(os.clone())
        } else if t == ROOT_TYPE || matches!(f, "ext2" | "ext3" | "ext4" | "btrfs" | "xfs" | "f2fs")
        {
            Some("Linux".into())
        } else {
            None
        }
    }

    /// What a person calls this partition.
    pub fn title(&self) -> String {
        if self.is_esp() {
            "Startup partition".into()
        } else if let Some(s) = self.system() {
            s
        } else if !self.label.is_empty() {
            self.label.clone()
        } else if !self.name.is_empty() {
            self.name.clone()
        } else {
            format!("Partition {}", self.number)
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disk {
    pub path: String,
    pub model: String,
    pub bytes: u64,
    pub sector: u64,
    pub table: Table,
    /// Partition table id, lowercase.
    pub id: String,
    /// First and last usable sector.
    pub first: u64,
    pub last: u64,
    pub parts: Vec<Part>,
}

/// One MiB in sectors: every start and size the installer writes is aligned to it.
pub fn grain(sector: u64) -> u64 {
    MIB / sector.max(1)
}

impl Disk {
    pub fn part(&self, uuid: &str) -> Option<&Part> {
        self.parts.iter().find(|p| p.uuid == uuid)
    }

    pub fn bytes_of(&self, p: &Part) -> u64 {
        p.sectors * self.sector
    }

    /// Other systems, each once, in disk order.
    pub fn others(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for s in self.parts.iter().filter_map(Part::system) {
            if !v.contains(&s) {
                v.push(s);
            }
        }
        v
    }

    /// Free space as (start, sectors), aligned to [`grain`] and at least one MiB.
    pub fn free(&self) -> Vec<(u64, u64)> {
        let used: Vec<(u64, u64)> = self.parts.iter().map(|p| (p.start, p.sectors)).collect();
        free_between(self.first, self.last, self.sector, &used)
    }

    /// Changes whenever the table or a file system identity does: a plan made against one
    /// fingerprint never runs against another.
    pub fn fingerprint(&self) -> String {
        let mut h = Sha256::new();
        h.update(format!(
            "{}|{:?}|{}|{}|{}\n",
            self.path, self.table, self.id, self.sector, self.bytes
        ));
        let mut parts: Vec<&Part> = self.parts.iter().collect();
        parts.sort_by_key(|p| p.start);
        for p in parts {
            h.update(format!(
                "{}|{}|{}|{}|{}|{}|{}\n",
                p.number, p.uuid, p.start, p.sectors, p.kind, p.fstype, p.fsuuid
            ));
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// The gaps in `first..=last` left by `used` extents, aligned to a MiB and at least a MiB long.
pub fn free_between(first: u64, last: u64, sector: u64, used: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let g = grain(sector);
    let mut used: Vec<(u64, u64)> = used.to_vec();
    used.sort();
    let mut out = Vec::new();
    let mut at = first;
    let mut gap = |from: u64, to_excl: u64| {
        let start = from.div_ceil(g) * g;
        let end = to_excl / g * g;
        if end > start {
            out.push((start, end - start));
        }
    };
    for (s, n) in used {
        if s > at {
            gap(at, s);
        }
        at = at.max(s + n);
    }
    gap(at, last + 1);
    out
}

#[derive(Deserialize)]
struct SfOut {
    partitiontable: SfTable,
}
#[derive(Deserialize)]
struct SfTable {
    label: String,
    id: Option<String>,
    firstlba: Option<u64>,
    lastlba: Option<u64>,
    sectorsize: Option<u64>,
    #[serde(default)]
    partitions: Vec<SfPart>,
}
#[derive(Deserialize)]
struct SfPart {
    node: String,
    start: u64,
    size: u64,
    #[serde(rename = "type")]
    kind: String,
    uuid: Option<String>,
    name: Option<String>,
}
#[derive(Deserialize)]
struct LsOut {
    blockdevices: Vec<LsDev>,
}
#[derive(Deserialize, Default)]
struct LsDev {
    path: Option<String>,
    size: Option<u64>,
    model: Option<String>,
    fstype: Option<String>,
    label: Option<String>,
    uuid: Option<String>,
    #[serde(default)]
    mountpoints: Vec<Option<String>>,
    #[serde(default)]
    children: Vec<LsDev>,
}

fn any_mount(d: &LsDev) -> bool {
    d.mountpoints.iter().flatten().any(|m| !m.is_empty()) || d.children.iter().any(any_mount)
}

fn partition_number(node: &str) -> u32 {
    let tail: String = node
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    tail.parse().unwrap_or(0)
}

/// `sfdisk --json <disk>` (None: no partition table) and
/// `lsblk --json -b -o PATH,SIZE,MODEL,FSTYPE,LABEL,UUID,MOUNTPOINTS <disk>`.
pub fn parse(sfdisk: Option<&str>, lsblk: &str) -> Option<Disk> {
    let ls: LsOut = serde_json::from_str(lsblk).ok()?;
    let top = ls.blockdevices.first()?;
    let mut d = Disk {
        path: top.path.clone()?,
        model: top.model.clone().unwrap_or_default().trim().to_string(),
        bytes: top.size?,
        sector: 512,
        ..Disk::default()
    };
    let Some(text) = sfdisk else {
        d.first = grain(d.sector);
        d.last = (d.bytes / d.sector).saturating_sub(34);
        return Some(d);
    };
    let t = serde_json::from_str::<SfOut>(text).ok()?.partitiontable;
    d.table = if t.label == "gpt" {
        Table::Gpt
    } else {
        Table::Other
    };
    d.sector = t.sectorsize.unwrap_or(512);
    d.id = t.id.unwrap_or_default().to_lowercase();
    d.first = t.firstlba.unwrap_or(grain(d.sector));
    d.last = t.lastlba.unwrap_or((d.bytes / d.sector).saturating_sub(34));
    for p in t.partitions {
        let fs = top
            .children
            .iter()
            .find(|c| c.path.as_deref() == Some(&p.node));
        let f = fs.map(|f| {
            (
                f.fstype.clone().unwrap_or_default(),
                f.label.clone().unwrap_or_default(),
                f.uuid.clone().unwrap_or_default(),
                any_mount(f),
            )
        });
        let (fstype, label, fsuuid, mounted) = f.unwrap_or_default();
        d.parts.push(Part {
            number: partition_number(&p.node),
            path: p.node,
            start: p.start,
            sectors: p.size,
            kind: p.kind.to_lowercase(),
            uuid: p.uuid.unwrap_or_default().to_lowercase(),
            name: p.name.unwrap_or_default(),
            fstype,
            label,
            fsuuid,
            mounted,
            ..Part::default()
        });
    }
    Some(d)
}

/// `ntfsresize --info --force --no-progress-bars <part>`: bytes it could shrink to.
pub fn parse_ntfsresize(out: &str) -> Option<u64> {
    let rest = out.split("You might resize at ").nth(1)?;
    rest.split_whitespace().next()?.parse().ok()
}

/// `ntfsresize` refused: Windows did not shut down fully, or the volume can't be read.
pub fn ntfs_blocked(out: &str) -> Blocked {
    let l = out.to_lowercase();
    if ["hibernat", "unclean", "unsafe", "dirty", "fast restart"]
        .iter()
        .any(|w| l.contains(w))
    {
        Blocked::NotShutDown
    } else {
        Blocked::Unreadable
    }
}

/// `resize2fs -P <part>`: smallest size, blocks.
pub fn parse_resize2fs(out: &str) -> Option<u64> {
    let rest = out.split("minimum size of the filesystem:").nth(1)?;
    rest.split_whitespace().next()?.parse().ok()
}

/// `dumpe2fs -h <part>`: (block size, blocks, free blocks).
pub fn parse_dumpe2fs(out: &str) -> Option<(u64, u64, u64)> {
    let field = |k: &str| -> Option<u64> {
        out.lines()
            .find_map(|l| l.strip_prefix(k))?
            .trim_start_matches(':')
            .trim()
            .parse()
            .ok()
    };
    Some((
        field("Block size")?,
        field("Block count")?,
        field("Free blocks")?,
    ))
}

/// `btrfs inspect-internal min-dev-size <mount>`: bytes.
pub fn parse_btrfs_min(out: &str) -> Option<u64> {
    out.split_whitespace().next()?.parse().ok()
}

/// `df -B1 --output=used,avail <mount>`: (used, available).
pub fn parse_df(out: &str) -> Option<(u64, u64)> {
    let mut w = out.lines().nth(1)?.split_whitespace();
    Some((w.next()?.parse().ok()?, w.next()?.parse().ok()?))
}

/// PRETTY_NAME (else NAME) of an os-release file.
pub fn parse_os_release(text: &str) -> Option<String> {
    let get = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(k)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').to_string())
            .filter(|v| !v.is_empty())
    };
    get("PRETTY_NAME").or_else(|| get("NAME"))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const SF: &str = r#"{"partitiontable":{"label":"gpt","id":"AAAA-1","device":"/dev/nvme0n1","unit":"sectors","firstlba":2048,"lastlba":1953525134,"sectorsize":512,"partitions":[
      {"node":"/dev/nvme0n1p1","start":2048,"size":204800,"type":"C12A7328-F81F-11D2-BA4B-00A0C93EC93B","uuid":"E1E1","name":"EFI system partition"},
      {"node":"/dev/nvme0n1p2","start":206848,"size":32768,"type":"E3C9E316-0B5C-4DB8-817D-F92DF00215AE","uuid":"E2E2","name":"Microsoft reserved"},
      {"node":"/dev/nvme0n1p3","start":239616,"size":900000000,"type":"EBD0A0A2-B9E5-4433-87C0-68B6B72699C7","uuid":"E3E3","name":"Basic data"},
      {"node":"/dev/nvme0n1p4","start":1000000000,"size":2097152,"type":"DE94BBA4-06D1-4D40-A16A-BFD50179D6AC","uuid":"E4E4"}]}}"#;
    pub const LS: &str = r#"{"blockdevices":[{"path":"/dev/nvme0n1","size":1000204886016,"model":"Samsung SSD 980 PRO  ","fstype":null,"label":null,"uuid":null,"mountpoints":[null],
      "children":[
      {"path":"/dev/nvme0n1p1","size":104857600,"model":null,"fstype":"vfat","label":"SYSTEM","uuid":"AB12","mountpoints":[null]},
      {"path":"/dev/nvme0n1p2","size":16777216,"model":null,"fstype":null,"label":null,"uuid":null,"mountpoints":[null]},
      {"path":"/dev/nvme0n1p3","size":460800000000,"model":null,"fstype":"ntfs","label":"Windows","uuid":"CD34","mountpoints":[null]},
      {"path":"/dev/nvme0n1p4","size":1073741824,"model":null,"fstype":"ntfs","label":"Recovery","uuid":"EF56","mountpoints":["/media/x"]}]}]}"#;

    #[test]
    fn a_windows_disk_reads_as_people_name_it() {
        let d = parse(Some(SF), LS).unwrap();
        assert_eq!(d.model, "Samsung SSD 980 PRO");
        assert_eq!((d.table, d.sector, d.parts.len()), (Table::Gpt, 512, 4));
        assert_eq!(d.others(), ["Windows"]);
        assert_eq!(d.parts[0].title(), "Startup partition");
        assert!(d.parts[0].is_esp());
        assert_eq!(d.parts[2].title(), "Windows");
        assert_eq!(d.parts[2].number, 3);
        assert_eq!(d.parts[2].uuid, "e3e3");
        assert!(d.parts[3].mounted && !d.parts[2].mounted);
    }

    #[test]
    fn free_space_is_aligned_and_ordered() {
        let d = parse(Some(SF), LS).unwrap();
        let free = d.free();
        // The gap after Windows, and the tail of the disk.
        assert_eq!(free[0].0 % grain(512), 0);
        assert_eq!(free[0].0, 900239616u64.div_ceil(2048) * 2048);
        assert!(free[0].0 + free[0].1 <= 1000000000);
        assert!(free[1].0 >= 1002097152);
        assert!(free.iter().all(|(s, n)| s % 2048 == 0 && n % 2048 == 0));
    }

    #[test]
    fn fingerprint_follows_every_identity_but_not_order_of_listing() {
        let a = parse(Some(SF), LS).unwrap();
        let mut b = a.clone();
        b.parts.reverse();
        assert_eq!(a.fingerprint(), b.fingerprint());
        let changes: [fn(&mut Disk); 5] = [
            |d| d.parts[2].sectors -= 2048,
            |d| d.parts[2].start += 2048,
            |d| d.parts[2].uuid = "other".into(),
            |d| d.parts[2].fsuuid = "other".into(),
            |d| d.id = "other".into(),
        ];
        for change in changes {
            let mut c = a.clone();
            change(&mut c);
            assert_ne!(a.fingerprint(), c.fingerprint());
        }
    }

    #[test]
    fn blank_and_old_disks() {
        let ls = r#"{"blockdevices":[{"path":"/dev/vda","size":34359738368,"model":null}]}"#;
        let d = parse(None, ls).unwrap();
        assert_eq!(d.table, Table::None);
        assert_eq!(d.free().len(), 1);
        let mbr = r#"{"partitiontable":{"label":"dos","id":"0x1234","device":"/dev/vda","unit":"sectors","sectorsize":512,"partitions":[{"node":"/dev/vda1","start":2048,"size":4096,"type":"83"}]}}"#;
        assert_eq!(parse(Some(mbr), ls).unwrap().table, Table::Other);
    }

    #[test]
    fn systems_are_named_from_types_file_systems_and_os_release() {
        let p = |kind: &str, fstype: &str| Part {
            kind: kind.into(),
            fstype: fstype.into(),
            ..Part::default()
        };
        assert_eq!(p(MACOS_TYPES[1], "apfs").system().unwrap(), "macOS");
        assert_eq!(p(DATA_TYPE, "ext4").system().unwrap(), "Linux");
        assert_eq!(p(DATA_TYPE, "").system(), None);
        assert_eq!(p(SWAP_TYPE, "swap").system(), None);
        assert_eq!(p(FREEBSD_TYPES[1], "").system().unwrap(), "FreeBSD");
        let mut ubuntu = p(DATA_TYPE, "ext4");
        ubuntu.os = Some("Ubuntu 24.04.1 LTS".into());
        assert_eq!(ubuntu.title(), "Ubuntu 24.04.1 LTS");
        assert_eq!(p(DATA_TYPE, "").title(), "Partition 0");
    }

    #[test]
    fn tool_output_parses() {
        assert_eq!(
            parse_ntfsresize(
                "Space in use : 12 MB\nYou might resize at 25603538944 bytes or 25604 MB (freeing 4 MB).\n"
            ),
            Some(25603538944)
        );
        assert_eq!(parse_ntfsresize("nothing"), None);
        assert_eq!(
            ntfs_blocked("Windows is hibernated, refused to mount."),
            Blocked::NotShutDown
        );
        assert_eq!(
            ntfs_blocked("The NTFS journal file is unclean"),
            Blocked::NotShutDown
        );
        assert_eq!(
            ntfs_blocked("Failed to startup volume"),
            Blocked::Unreadable
        );
        assert_eq!(
            parse_resize2fs("Estimated minimum size of the filesystem: 37704\n"),
            Some(37704)
        );
        assert_eq!(
            parse_dumpe2fs(
                "Block count:              262144\nFree blocks:              200000\nBlock size:               4096\n"
            ),
            Some((4096, 262144, 200000))
        );
        assert_eq!(
            parse_btrfs_min("1103101952 bytes (1.03GiB)\n"),
            Some(1103101952)
        );
        assert_eq!(parse_df("  Used Avail\n 1000  2000\n"), Some((1000, 2000)));
        assert_eq!(
            parse_os_release("NAME=\"Fedora\"\nPRETTY_NAME=\"Fedora Linux 41\"\n").unwrap(),
            "Fedora Linux 41"
        );
        assert_eq!(parse_os_release("NAME=Arch\n").unwrap(), "Arch");
    }
}
