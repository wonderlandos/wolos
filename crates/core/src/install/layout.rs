// SPDX-License-Identifier: GPL-2.0-or-later
//! What the installer will do to a disk (ADR 0023). A [`Layout`] is what the person chose; `compile`
//! turns it into [`Op`]s, or a [`Refusal`]. Only partitions the layout names are ever touched:
//! [`verify`] checks that independently of `compile`, and [`unchanged`] checks it again against the
//! real partition table after every step of a run.
use super::disk::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Smallest room WolOS installs in (`shell::install::MIN_BYTES`).
pub const MIN_ROOT: u64 = 16_000_000_000;
/// Under this WolOS works but fills up soon: the page warns.
pub const ROOMY_ROOT: u64 = 40_000_000_000;
/// A new startup partition.
pub const ESP_BYTES: u64 = GIB;
/// Free room an existing startup partition needs for WolOS's two kernels (measured: ADR 0023).
pub const ESP_FREE_NEEDED: u64 = 500 * MIB;
/// Room left to another system beyond what it uses today.
pub const SPARE_BYTES: u64 = 4 * GIB;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fs {
    Btrfs,
    Ext4,
    Xfs,
    Vfat,
    Swap,
}

impl Fs {
    pub fn word(self) -> &'static str {
        match self {
            Fs::Btrfs => "btrfs",
            Fs::Ext4 => "ext4",
            Fs::Xfs => "xfs",
            Fs::Vfat => "vfat",
            Fs::Swap => "swap",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resize {
    pub part: String,
    pub bytes: u64,
}

/// An existing partition gets a job. `format: None` keeps what is on it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assign {
    pub part: String,
    pub mount: String,
    pub format: Option<Fs>,
}

/// A new partition. `bytes: None`: all the room that is left.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct New {
    pub bytes: Option<u64>,
    pub mount: String,
    pub fs: Fs,
}

/// What the person chose.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    /// Erase Disk: everything goes, systemd-repart lays out the disk (ADR 0018).
    pub wipe: bool,
    pub delete: Vec<String>,
    pub resize: Vec<Resize>,
    pub assign: Vec<Assign>,
    pub create: Vec<New>,
}

impl Layout {
    pub fn erase() -> Layout {
        Layout {
            wipe: true,
            ..Layout::default()
        }
    }

    /// Partitions whose contents the person agreed to change: deleted, resized or reformatted.
    pub fn chosen(&self) -> BTreeSet<&str> {
        let mut s: BTreeSet<&str> = self.delete.iter().map(String::as_str).collect();
        s.extend(self.resize.iter().map(|r| r.part.as_str()));
        s.extend(
            self.assign
                .iter()
                .filter(|a| a.format.is_some())
                .map(|a| a.part.as_str()),
        );
        s
    }
}

/// Where a mount or a format lands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    Existing(String),
    /// Index into `Layout::create`.
    New(usize),
}

/// One step, in the order it runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Op {
    /// Erase Disk.
    Repart,
    /// A blank disk gets a GPT.
    NewTable,
    Delete {
        uuid: String,
        number: u32,
    },
    ShrinkFs {
        uuid: String,
        path: String,
        fstype: String,
        bytes: u64,
    },
    ShrinkPart {
        uuid: String,
        number: u32,
        sectors: u64,
    },
    GrowPart {
        uuid: String,
        number: u32,
        sectors: u64,
    },
    GrowFs {
        uuid: String,
        path: String,
        fstype: String,
    },
    Create {
        index: usize,
        start: u64,
        sectors: u64,
        kind: String,
        name: String,
    },
    Format {
        target: Target,
        fs: Fs,
        /// LUKS2 under the file system.
        encrypt: bool,
    },
}

/// How many system copies this layout keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Slots {
    /// Slots a and b on btrfs, sealed.
    Two,
    /// The system partition isn't btrfs: one copy, updates in place, no seal.
    One,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mounted {
    pub target: Target,
    pub path: String,
    pub fs: Fs,
    pub format: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Compiled {
    pub ops: Vec<Op>,
    pub mounts: Vec<Mounted>,
    pub slots: Slots,
    /// Size of the partition WolOS goes on.
    pub root_bytes: u64,
}

/// Why a layout can't run. `shell::copy::install_refusal` says each in one sentence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// The disk is not what the person looked at.
    DiskChanged,
    /// The MBR or foreign table: only Erase works.
    NotGpt,
    TooSmall,
    UnknownPartition,
    /// One partition, two jobs.
    Conflict(String),
    Mounted(String),
    /// Can't get that small (or at all, see `Part::blocked`).
    CantResize(String),
    /// Needs more room than the space next to it.
    NoRoom,
    NoRoot,
    NoEsp,
    /// Existing partition set as the system without erasing it.
    RootMustFormat,
    /// Asked to erase a startup partition another system uses.
    EspInUse,
    /// That partition isn't a startup partition, or can't hold the kernels.
    NotAnEsp(String),
    BadMount(String),
    DuplicateMount(String),
    /// An existing file system WolOS can't use here.
    BadFilesystem(String),
}

/// Where a mount may go: the fixed roles, or a plain folder at the top.
pub fn mount_allowed(m: &str) -> bool {
    const SYSTEM: [&str; 13] = [
        "bin", "boot", "dev", "etc", "lib", "lib64", "proc", "root", "run", "sbin", "sys", "tmp",
        "usr",
    ];
    if matches!(m, "/" | "/efi" | "/home" | "/var" | "swap") {
        return true;
    }
    let Some(name) = m.strip_prefix('/') else {
        return false;
    };
    !name.is_empty()
        && !SYSTEM.contains(&name)
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// How far `p` may be made smaller: (smallest it may become, its size), in bytes. None: it can't.
pub fn shrink_range(d: &Disk, p: &Part) -> Option<(u64, u64)> {
    if p.blocked.is_some() || p.mounted || !can_shrink(&p.fstype) {
        return None;
    }
    let lo = p.min_bytes? + SPARE_BYTES;
    let size = d.bytes_of(p);
    (lo < size).then_some((lo, size))
}

fn can_shrink(fstype: &str) -> bool {
    matches!(fstype, "ntfs" | "ext2" | "ext3" | "ext4" | "btrfs")
}

fn can_grow(fstype: &str) -> bool {
    can_shrink(fstype) || fstype == "xfs"
}

/// GPT type and name of a new partition.
fn identity(mount: &str) -> (&'static str, &'static str) {
    match mount {
        "/" => (ROOT_TYPE, "WolOS"),
        "/efi" => (ESP_TYPE, "WOLOS"),
        "/home" => (HOME_TYPE, "Home"),
        "/var" => (VAR_TYPE, "Var"),
        "swap" => (SWAP_TYPE, "Swap"),
        _ => (DATA_TYPE, "Data"),
    }
}

fn whole_mib(bytes: u64, sector: u64) -> u64 {
    bytes.div_ceil(MIB) * MIB / sector
}

/// One job per partition, and only partitions that exist and are free to change.
fn check_names(l: &Layout, d: &Disk) -> Result<(), Refusal> {
    let mut deleted = BTreeSet::new();
    for id in &l.delete {
        d.part(id).ok_or(Refusal::UnknownPartition)?;
        if !deleted.insert(id.as_str()) {
            return Err(Refusal::Conflict(id.clone()));
        }
    }
    let mut resized = BTreeSet::new();
    for r in &l.resize {
        d.part(&r.part).ok_or(Refusal::UnknownPartition)?;
        if deleted.contains(r.part.as_str()) || !resized.insert(r.part.as_str()) {
            return Err(Refusal::Conflict(r.part.clone()));
        }
    }
    let mut assigned = BTreeSet::new();
    for a in &l.assign {
        d.part(&a.part).ok_or(Refusal::UnknownPartition)?;
        if deleted.contains(a.part.as_str()) || !assigned.insert(a.part.as_str()) {
            return Err(Refusal::Conflict(a.part.clone()));
        }
    }
    let busy = l
        .delete
        .iter()
        .chain(l.resize.iter().map(|r| &r.part))
        .chain(l.assign.iter().map(|a| &a.part));
    for id in busy {
        if d.part(id).is_some_and(|p| p.mounted) {
            return Err(Refusal::Mounted(id.clone()));
        }
    }
    Ok(())
}

/// Mount points, each once, each on something that can hold it.
fn check_mounts(l: &Layout, d: &Disk) -> Result<Vec<Mounted>, Refusal> {
    let mut out: Vec<Mounted> = Vec::new();
    for a in &l.assign {
        let p = d.part(&a.part).ok_or(Refusal::UnknownPartition)?;
        if !mount_allowed(&a.mount) {
            return Err(Refusal::BadMount(a.mount.clone()));
        }
        let bad = || Refusal::BadFilesystem(a.part.clone());
        let fs = match (a.mount.as_str(), a.format) {
            ("/", None) => return Err(Refusal::RootMustFormat),
            ("/efi", _) if !p.is_esp() || p.fstype != "vfat" => {
                return Err(Refusal::NotAnEsp(a.part.clone()));
            }
            ("/efi", None) => {
                if p.free_bytes.unwrap_or(0) < ESP_FREE_NEEDED {
                    return Err(Refusal::NotAnEsp(a.part.clone()));
                }
                Fs::Vfat
            }
            ("/efi", Some(Fs::Vfat)) => {
                if !d.others().is_empty() {
                    return Err(Refusal::EspInUse);
                }
                Fs::Vfat
            }
            ("/efi", Some(_)) => return Err(Refusal::NotAnEsp(a.part.clone())),
            ("swap", None) if p.fstype == "swap" => Fs::Swap,
            ("swap", Some(Fs::Swap)) => Fs::Swap,
            ("swap", _) => return Err(bad()),
            ("/var", None) => return Err(bad()),
            (_, Some(f @ (Fs::Btrfs | Fs::Ext4 | Fs::Xfs))) => f,
            (_, None) => match p.fstype.as_str() {
                "btrfs" => Fs::Btrfs,
                "ext4" => Fs::Ext4,
                "xfs" => Fs::Xfs,
                _ => return Err(bad()),
            },
            _ => return Err(bad()),
        };
        out.push(Mounted {
            target: Target::Existing(a.part.clone()),
            path: a.mount.clone(),
            fs,
            format: a.format.is_some(),
        });
    }
    for (i, n) in l.create.iter().enumerate() {
        if !mount_allowed(&n.mount) {
            return Err(Refusal::BadMount(n.mount.clone()));
        }
        let fits = match n.mount.as_str() {
            "/efi" => n.fs == Fs::Vfat,
            "swap" => n.fs == Fs::Swap,
            _ => matches!(n.fs, Fs::Btrfs | Fs::Ext4 | Fs::Xfs),
        };
        if !fits {
            return Err(Refusal::BadFilesystem(n.mount.clone()));
        }
        out.push(Mounted {
            target: Target::New(i),
            path: n.mount.clone(),
            fs: n.fs,
            format: true,
        });
    }
    let mut seen = BTreeSet::new();
    for m in &out {
        if !seen.insert(m.path.as_str()) {
            return Err(Refusal::DuplicateMount(m.path.clone()));
        }
    }
    if !seen.contains("/") {
        return Err(Refusal::NoRoot);
    }
    if !seen.contains("/efi") {
        return Err(Refusal::NoEsp);
    }
    Ok(out)
}

/// Steps for the resizes, and the extents everything kept ends up on.
/// Ops that make room, ops that fill it afterwards, and the sectors the first ones free.
type ResizeOps = (Vec<Op>, Vec<Op>, Vec<(u64, u64)>);

fn resize_ops(l: &Layout, d: &Disk) -> Result<ResizeOps, Refusal> {
    let (mut shrink, mut grow) = (Vec::new(), Vec::new());
    let mut kept = Vec::new();
    for p in &d.parts {
        if l.delete.contains(&p.uuid) {
            continue;
        }
        let Some(r) = l.resize.iter().find(|r| r.part == p.uuid) else {
            kept.push((p.start, p.sectors));
            continue;
        };
        let sectors = whole_mib(r.bytes, d.sector);
        kept.push((p.start, sectors));
        let reformatted = l
            .assign
            .iter()
            .any(|a| a.part == p.uuid && a.format.is_some());
        let bad = || Refusal::CantResize(p.uuid.clone());
        if sectors < p.sectors {
            if !reformatted {
                let (lo, _) = shrink_range(d, p).ok_or_else(bad)?;
                if r.bytes < lo {
                    return Err(bad());
                }
                shrink.push(Op::ShrinkFs {
                    uuid: p.uuid.clone(),
                    path: p.path.clone(),
                    fstype: p.fstype.clone(),
                    bytes: r.bytes.div_ceil(MIB) * MIB,
                });
            }
            shrink.push(Op::ShrinkPart {
                uuid: p.uuid.clone(),
                number: p.number,
                sectors,
            });
        } else if sectors > p.sectors {
            if !reformatted && (p.blocked.is_some() || !can_grow(&p.fstype)) {
                return Err(bad());
            }
            grow.push(Op::GrowPart {
                uuid: p.uuid.clone(),
                number: p.number,
                sectors,
            });
            if !reformatted {
                grow.push(Op::GrowFs {
                    uuid: p.uuid.clone(),
                    path: p.path.clone(),
                    fstype: p.fstype.clone(),
                });
            }
        }
    }
    let mut sorted = kept.clone();
    sorted.sort();
    let overlap = sorted.windows(2).any(|w| w[0].0 + w[0].1 > w[1].0);
    if overlap || sorted.iter().any(|(s, n)| s + n - 1 > d.last) {
        return Err(Refusal::NoRoom);
    }
    Ok((shrink, grow, kept))
}

/// First fit for sized partitions, then the largest gap left for one that takes the rest. New
/// partitions never leave free space: an extent is only ever cut from `free_between`.
fn place(l: &Layout, d: &Disk, kept: &[(u64, u64)]) -> Result<Vec<(u64, u64)>, Refusal> {
    let mut free = free_between(d.first, d.last, d.sector, kept);
    let mut at = vec![(0, 0); l.create.len()];
    for (i, n) in l.create.iter().enumerate() {
        let Some(b) = n.bytes else { continue };
        let want = whole_mib(b, d.sector);
        let gap = free
            .iter_mut()
            .find(|(_, s)| *s >= want)
            .ok_or(Refusal::NoRoom)?;
        at[i] = (gap.0, want);
        *gap = (gap.0 + want, gap.1 - want);
    }
    for (i, n) in l.create.iter().enumerate() {
        if n.bytes.is_none() {
            let gap = free
                .iter_mut()
                .max_by_key(|(_, s)| *s)
                .ok_or(Refusal::NoRoom)?;
            if gap.1 == 0 {
                return Err(Refusal::NoRoom);
            }
            at[i] = *gap;
            *gap = (gap.0 + gap.1, 0);
        }
    }
    Ok(at)
}

/// Free extents once the layout's deletes, resizes and new partitions are done.
pub fn room(l: &Layout, d: &Disk) -> Vec<(u64, u64)> {
    let kept: Vec<(u64, u64)> = d
        .parts
        .iter()
        .filter(|p| !l.delete.contains(&p.uuid))
        .map(|p| {
            let r = l.resize.iter().find(|r| r.part == p.uuid);
            (
                p.start,
                r.map_or(p.sectors, |r| whole_mib(r.bytes, d.sector)),
            )
        })
        .collect();
    let mut free = free_between(d.first, d.last, d.sector, &kept);
    for n in &l.create {
        match n.bytes {
            Some(b) => {
                let want = whole_mib(b, d.sector);
                if let Some(gap) = free.iter_mut().find(|(_, s)| *s >= want) {
                    *gap = (gap.0 + want, gap.1 - want);
                }
            }
            None => {
                if let Some(gap) = free.iter_mut().max_by_key(|(_, s)| *s) {
                    gap.1 = 0;
                }
            }
        }
    }
    free.retain(|(_, s)| *s > 0);
    free
}

pub fn compile(l: &Layout, d: &Disk, encrypt: bool) -> Result<Compiled, Refusal> {
    if l.wipe {
        if d.bytes < MIN_ROOT {
            return Err(Refusal::TooSmall);
        }
        if !(l.delete.is_empty()
            && l.resize.is_empty()
            && l.assign.is_empty()
            && l.create.is_empty())
        {
            return Err(Refusal::Conflict(String::new()));
        }
        return Ok(Compiled {
            ops: vec![Op::Repart],
            mounts: Vec::new(),
            slots: Slots::Two,
            root_bytes: d.bytes,
        });
    }
    let blank = d.table == Table::None && d.parts.is_empty();
    if d.table != Table::Gpt && !blank {
        return Err(Refusal::NotGpt);
    }
    check_names(l, d)?;
    let mounts = check_mounts(l, d)?;
    let (shrink, grow, kept) = resize_ops(l, d)?;
    let at = place(l, d, &kept)?;
    let mut ops = Vec::new();
    if blank {
        ops.push(Op::NewTable);
    }
    for id in &l.delete {
        let p = d.part(id).ok_or(Refusal::UnknownPartition)?;
        ops.push(Op::Delete {
            uuid: id.clone(),
            number: p.number,
        });
    }
    ops.extend(shrink);
    ops.extend(grow);
    for (i, n) in l.create.iter().enumerate() {
        let (kind, name) = identity(&n.mount);
        ops.push(Op::Create {
            index: i,
            start: at[i].0,
            sectors: at[i].1,
            kind: kind.into(),
            name: name.into(),
        });
    }
    for m in mounts.iter().filter(|m| m.format) {
        ops.push(Op::Format {
            target: m.target.clone(),
            fs: m.fs,
            encrypt: encrypt && m.path == "/",
        });
    }
    let root = mounts
        .iter()
        .find(|m| m.path == "/")
        .ok_or(Refusal::NoRoot)?;
    let root_bytes = match &root.target {
        Target::New(i) => at[*i].1 * d.sector,
        Target::Existing(id) => {
            let p = d.part(id).ok_or(Refusal::UnknownPartition)?;
            kept_size(l, d, p)
        }
    };
    if root_bytes < MIN_ROOT {
        return Err(Refusal::TooSmall);
    }
    let slots = if root.fs == Fs::Btrfs {
        Slots::Two
    } else {
        Slots::One
    };
    Ok(Compiled {
        ops,
        mounts,
        slots,
        root_bytes,
    })
}

fn kept_size(l: &Layout, d: &Disk, p: &Part) -> u64 {
    l.resize
        .iter()
        .find(|r| r.part == p.uuid)
        .map_or(d.bytes_of(p), |r| whole_mib(r.bytes, d.sector) * d.sector)
}

/// The compiled steps stay inside what the layout names: no step touches a partition outside
/// [`Layout::chosen`], and new partitions land on free space only.
pub fn verify(c: &Compiled, l: &Layout, d: &Disk) -> Result<(), Refusal> {
    let chosen = l.chosen();
    let named = |uuid: &str| -> Result<&Part, Refusal> {
        if !chosen.contains(uuid) {
            return Err(Refusal::Conflict(uuid.to_string()));
        }
        d.part(uuid).ok_or(Refusal::UnknownPartition)
    };
    let same = |p: &Part, number: u32| {
        if p.number == number {
            Ok(())
        } else {
            Err(Refusal::Conflict(p.uuid.clone()))
        }
    };
    let mut kept: Vec<(u64, u64)> = d
        .parts
        .iter()
        .filter(|p| !l.delete.contains(&p.uuid))
        .map(|p| (p.start, p.sectors))
        .collect();
    let mut new = Vec::new();
    for op in &c.ops {
        match op {
            Op::Repart if l.wipe => {}
            Op::Repart => return Err(Refusal::Conflict(String::new())),
            Op::NewTable if d.table == Table::None && d.parts.is_empty() => {}
            Op::NewTable => return Err(Refusal::NotGpt),
            Op::Delete { uuid, number } => {
                same(named(uuid)?, *number)?;
                if !l.delete.contains(uuid) {
                    return Err(Refusal::Conflict(uuid.clone()));
                }
            }
            Op::ShrinkFs { uuid, path, .. } | Op::GrowFs { uuid, path, .. } => {
                if &named(uuid)?.path != path {
                    return Err(Refusal::Conflict(uuid.clone()));
                }
            }
            Op::ShrinkPart {
                uuid,
                number,
                sectors,
            }
            | Op::GrowPart {
                uuid,
                number,
                sectors,
            } => {
                let p = named(uuid)?;
                same(p, *number)?;
                let e = kept.iter_mut().find(|e| e.0 == p.start);
                e.ok_or(Refusal::UnknownPartition)?.1 = *sectors;
            }
            Op::Create { start, sectors, .. } => new.push((*start, *sectors)),
            Op::Format {
                target: Target::Existing(id),
                ..
            } => {
                named(id)?;
            }
            Op::Format {
                target: Target::New(i),
                ..
            } => {
                l.create.get(*i).ok_or(Refusal::UnknownPartition)?;
            }
        }
    }
    let g = grain(d.sector);
    let mut all = kept;
    for e in &new {
        if e.0 % g != 0 || e.1 % g != 0 || e.0 < d.first || e.0 + e.1 - 1 > d.last {
            return Err(Refusal::NoRoom);
        }
    }
    all.extend(new);
    all.sort();
    if all.windows(2).any(|w| w[0].0 + w[0].1 > w[1].0) {
        return Err(Refusal::NoRoom);
    }
    Ok(())
}

/// After a step of a run: every partition outside `allowed` still is where it was, same size,
/// same identity, and nothing new overlaps it. Err names the first one that moved.
pub fn unchanged(before: &Disk, after: &Disk, allowed: &BTreeSet<&str>) -> Result<(), String> {
    for p in before
        .parts
        .iter()
        .filter(|p| !allowed.contains(p.uuid.as_str()))
    {
        let now = after.part(&p.uuid).ok_or_else(|| p.uuid.clone())?;
        let same = now.start == p.start
            && now.sectors == p.sectors
            && now.kind == p.kind
            && now.number == p.number
            && now.fsuuid == p.fsuuid
            && now.fstype == p.fstype;
        if !same {
            return Err(p.uuid.clone());
        }
        let hit = after.parts.iter().any(|o| {
            o.uuid != p.uuid && o.start < p.start + p.sectors && p.start < o.start + o.sectors
        });
        if hit {
            return Err(p.uuid.clone());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1_000_000_000;

    /// Windows-like 500 GB disk: ESP 100 MB, MSR, C: 400 GB, recovery; free tail.
    fn windows() -> Disk {
        let p = |n: u32, start: u64, sectors: u64, kind: &str, fs: &str| Part {
            number: n,
            path: format!("/dev/vda{n}"),
            start,
            sectors,
            kind: kind.into(),
            uuid: format!("u{n}"),
            fstype: fs.into(),
            fsuuid: format!("f{n}"),
            ..Part::default()
        };
        let mut c = p(
            3,
            239616,
            781250000,
            "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7",
            "ntfs",
        );
        c.min_bytes = Some(100 * GB);
        c.used_bytes = Some(99 * GB);
        let mut esp = p(1, 2048, 204800, ESP_TYPE, "vfat");
        esp.free_bytes = Some(60 * MIB);
        Disk {
            path: "/dev/vda".into(),
            bytes: 500 * GB,
            sector: 512,
            table: Table::Gpt,
            id: "t".into(),
            first: 2048,
            last: 976562466,
            parts: vec![
                esp,
                p(2, 206848, 32768, "e3c9e316-0b5c-4db8-817d-f92df00215ae", ""),
                c,
                p(
                    4,
                    781489616,
                    2097152,
                    "de94bba4-06d1-4d40-a16a-bfd50179d6ac",
                    "ntfs",
                ),
            ],
            ..Disk::default()
        }
    }

    fn root_new(bytes: Option<u64>) -> New {
        New {
            bytes,
            mount: "/".into(),
            fs: Fs::Btrfs,
        }
    }
    fn esp_new() -> New {
        New {
            bytes: Some(ESP_BYTES),
            mount: "/efi".into(),
            fs: Fs::Vfat,
        }
    }

    /// Alongside Windows: C: shrinks to 250 GB, new ESP (the old one is 100 MB), WolOS in the rest.
    fn alongside() -> Layout {
        Layout {
            resize: vec![Resize {
                part: "u3".into(),
                bytes: 250 * GB,
            }],
            create: vec![esp_new(), root_new(None)],
            ..Layout::default()
        }
    }

    #[test]
    fn erase_is_repart_and_nothing_else() {
        let c = compile(&Layout::erase(), &windows(), false).unwrap();
        assert_eq!(c.ops, [Op::Repart]);
        assert_eq!(c.slots, Slots::Two);
    }

    #[test]
    fn alongside_shrinks_the_file_system_first_and_only_then_the_partition() {
        let d = windows();
        let c = compile(&alongside(), &d, false).unwrap();
        let kinds: Vec<String> = c
            .ops
            .iter()
            .map(|o| {
                serde_json::to_value(o)
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .keys()
                    .next()
                    .unwrap()
                    .clone()
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "shrink-fs",
                "shrink-part",
                "create",
                "create",
                "format",
                "format"
            ]
        );
        // The size is a whole number of MiB in the partition, and at least the file system's size.
        let Op::ShrinkPart { sectors, .. } = &c.ops[1] else {
            panic!()
        };
        assert!(sectors * 512 >= 250 * GB && sectors % 2048 == 0);
        // New partitions fall where the shrink left room, before the recovery partition.
        let Op::Create {
            start,
            sectors: n,
            kind,
            ..
        } = &c.ops[2]
        else {
            panic!()
        };
        assert_eq!(kind, ESP_TYPE);
        assert!(start * 512 >= 250 * GB && (start + n) <= 781489616);
        verify(&c, &alongside(), &d).unwrap();
    }

    #[test]
    fn a_shrink_can_not_cut_into_what_windows_uses() {
        let d = windows();
        let mut l = alongside();
        l.resize[0].bytes = 100 * GB; // the minimum, without spare room
        assert_eq!(
            compile(&l, &d, false),
            Err(Refusal::CantResize("u3".into()))
        );
        let (lo, hi) = shrink_range(&d, &d.parts[2]).unwrap();
        assert_eq!(lo, 100 * GB + SPARE_BYTES);
        assert_eq!(hi, 400 * GB);
        l.resize[0].bytes = lo;
        compile(&l, &d, false).unwrap();
    }

    #[test]
    fn systems_that_can_not_be_resized_say_so() {
        let mut d = windows();
        d.parts[2].blocked = Some(Blocked::BitLocker);
        assert_eq!(shrink_range(&d, &d.parts[2]), None);
        assert_eq!(
            compile(&alongside(), &d, false),
            Err(Refusal::CantResize("u3".into()))
        );
        let mut d = windows();
        d.parts[2].mounted = true;
        assert_eq!(
            compile(&alongside(), &d, false),
            Err(Refusal::Mounted("u3".into()))
        );
        let mut d = windows();
        d.parts[2].fstype = "xfs".into();
        assert_eq!(shrink_range(&d, &d.parts[2]), None, "xfs only grows");
    }

    #[test]
    fn manual_layouts_need_a_system_a_startup_partition_and_a_real_table() {
        let d = windows();
        let mut l = alongside();
        l.create.remove(1);
        assert_eq!(compile(&l, &d, false), Err(Refusal::NoRoot));
        let mut l = alongside();
        l.create.remove(0);
        assert_eq!(compile(&l, &d, false), Err(Refusal::NoEsp));
        let mut mbr = windows();
        mbr.table = Table::Other;
        assert_eq!(compile(&alongside(), &mbr, false), Err(Refusal::NotGpt));
        assert_eq!(
            compile(&Layout::erase(), &mbr, false).unwrap().ops,
            [Op::Repart],
            "Erase works on any table"
        );
        let mut small = windows();
        small.bytes = 15 * GB;
        assert_eq!(
            compile(&Layout::erase(), &small, false),
            Err(Refusal::TooSmall)
        );
    }

    #[test]
    fn reusing_a_startup_partition_needs_room_and_writes_no_table() {
        let mut d = windows();
        let mut l = alongside();
        l.create.remove(0);
        l.assign.push(Assign {
            part: "u1".into(),
            mount: "/efi".into(),
            format: None,
        });
        assert_eq!(
            compile(&l, &d, false),
            Err(Refusal::NotAnEsp("u1".into())),
            "only 60 MB free"
        );
        d.parts[0].free_bytes = Some(ESP_FREE_NEEDED);
        let c = compile(&l, &d, false).unwrap();
        assert!(!c.ops.iter().any(|o| matches!(
            o,
            Op::Format {
                target: Target::Existing(_),
                ..
            }
        )));
        assert!(
            !l.chosen().contains("u1"),
            "its files are added to, nothing is erased"
        );
        // Erasing one that Windows starts from is refused.
        l.assign[0].format = Some(Fs::Vfat);
        assert_eq!(compile(&l, &d, false), Err(Refusal::EspInUse));
    }

    #[test]
    fn keeping_home_formats_nothing_there() {
        let mut d = windows();
        d.parts[2].fstype = "ext4".into();
        d.parts[2].kind = DATA_TYPE.into();
        let mut l = alongside();
        l.resize.clear();
        l.create = vec![esp_new(), root_new(Some(50 * GB))];
        l.assign.push(Assign {
            part: "u3".into(),
            mount: "/home".into(),
            format: None,
        });
        // Free tail only: 500 GB disk, partitions up to ~401 GB.
        let c = compile(&l, &d, false).unwrap();
        assert!(l.chosen().is_empty());
        verify(&c, &l, &d).unwrap();
        assert!(c.mounts.iter().any(|m| m.path == "/home" && !m.format));
        assert!(!c.ops.iter().any(|o| matches!(
            o,
            Op::Format {
                target: Target::Existing(_),
                ..
            }
        )));
        // The system can't be kept that way.
        l.assign[0].mount = "/".into();
        l.create.remove(1);
        assert_eq!(compile(&l, &d, false), Err(Refusal::RootMustFormat));
        // NTFS can't be home.
        let mut d = windows();
        d.parts[3].fstype = "ntfs".into();
        let l = Layout {
            create: vec![esp_new(), root_new(Some(30 * GB))],
            assign: vec![Assign {
                part: "u3".into(),
                mount: "/home".into(),
                format: None,
            }],
            ..Layout::default()
        };
        assert_eq!(
            compile(&l, &d, false),
            Err(Refusal::BadFilesystem("u3".into()))
        );
    }

    #[test]
    fn a_root_that_is_not_btrfs_keeps_one_copy() {
        let d = windows();
        let mut l = alongside();
        l.create[1].fs = Fs::Ext4;
        assert_eq!(compile(&l, &d, false).unwrap().slots, Slots::One);
        assert_eq!(compile(&alongside(), &d, false).unwrap().slots, Slots::Two);
    }

    #[test]
    fn encryption_covers_the_system_partition() {
        let d = windows();
        let c = compile(&alongside(), &d, true).unwrap();
        let enc: Vec<bool> = c
            .ops
            .iter()
            .filter_map(|o| match o {
                Op::Format { fs, encrypt, .. } => Some((*fs == Fs::Btrfs) == *encrypt),
                _ => None,
            })
            .collect();
        assert_eq!(enc, [true, true]);
    }

    #[test]
    fn mount_points_are_checked() {
        for ok in ["/", "/efi", "/home", "/var", "swap", "/data", "/games-2"] {
            assert!(mount_allowed(ok), "{ok}");
        }
        for bad in [
            "", "/usr", "/etc", "/boot", "home", "/a/b", "/A", "/ ", "/proc",
        ] {
            assert!(!mount_allowed(bad), "{bad}");
        }
        let d = windows();
        let mut l = alongside();
        l.create.push(New {
            bytes: Some(GB),
            mount: "/usr".into(),
            fs: Fs::Ext4,
        });
        assert_eq!(
            compile(&l, &d, false),
            Err(Refusal::BadMount("/usr".into()))
        );
        l.create[2].mount = "/".into();
        assert_eq!(
            compile(&l, &d, false),
            Err(Refusal::DuplicateMount("/".into()))
        );
    }

    #[test]
    fn plans_that_do_not_fit_or_contradict_themselves_are_refused() {
        let d = windows();
        let l = Layout {
            create: vec![esp_new(), root_new(Some(400 * GB))],
            ..Layout::default()
        };
        assert_eq!(compile(&l, &d, false), Err(Refusal::NoRoom));
        let mut l = alongside();
        l.delete.push("u3".into());
        assert_eq!(compile(&l, &d, false), Err(Refusal::Conflict("u3".into())));
        let mut l = alongside();
        l.delete.push("nope".into());
        assert_eq!(compile(&l, &d, false), Err(Refusal::UnknownPartition));
        let mut l = alongside();
        l.create[1].bytes = Some(GB);
        assert_eq!(compile(&l, &d, false), Err(Refusal::TooSmall));
        let mut mounted = windows();
        mounted.parts[3].mounted = true;
        let mut l = alongside();
        l.delete.push("u4".into());
        assert_eq!(
            compile(&l, &mounted, false),
            Err(Refusal::Mounted("u4".into()))
        );
    }

    #[test]
    fn verify_refuses_steps_outside_the_choice() {
        let d = windows();
        let l = alongside();
        let good = compile(&l, &d, false).unwrap();
        let mut bad = good.clone();
        bad.ops.push(Op::Delete {
            uuid: "u4".into(),
            number: 4,
        });
        assert!(verify(&bad, &l, &d).is_err(), "a delete nobody chose");
        let mut bad = good.clone();
        bad.ops.push(Op::Format {
            target: Target::Existing("u2".into()),
            fs: Fs::Ext4,
            encrypt: false,
        });
        assert!(verify(&bad, &l, &d).is_err(), "a format nobody chose");
        let mut bad = good.clone();
        if let Some(Op::Create { start, .. }) =
            bad.ops.iter_mut().find(|o| matches!(o, Op::Create { .. }))
        {
            *start = 2048;
        }
        assert!(
            verify(&bad, &l, &d).is_err(),
            "a new partition over the startup partition"
        );
        let mut bad = good.clone();
        bad.ops.push(Op::ShrinkPart {
            uuid: "u3".into(),
            number: 4,
            sectors: 1,
        });
        assert!(
            verify(&bad, &l, &d).is_err(),
            "right name, another partition's number"
        );
    }

    #[test]
    fn unchanged_notices_anything_outside_the_choice_moving() {
        let before = windows();
        let allowed: BTreeSet<&str> = ["u3"].into_iter().collect();
        let mut after = before.clone();
        after.parts[2].sectors -= 2048;
        assert_eq!(unchanged(&before, &after, &allowed), Ok(()));
        for hurt in [1usize, 3] {
            let mut after = before.clone();
            after.parts[hurt].sectors -= 2048;
            assert!(unchanged(&before, &after, &allowed).is_err());
            let mut after = before.clone();
            after.parts[hurt].start += 2048;
            assert!(unchanged(&before, &after, &allowed).is_err());
            let mut after = before.clone();
            after.parts[hurt].fsuuid = "other".into();
            assert!(unchanged(&before, &after, &allowed).is_err());
            let mut after = before.clone();
            after.parts.remove(hurt);
            assert!(unchanged(&before, &after, &allowed).is_err());
        }
    }

    /// Small repeatable generator: no dependency, same run every time.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }
    }

    fn random_disk(r: &mut Rng) -> Disk {
        let mut d = Disk {
            path: "/dev/vda".into(),
            sector: 512,
            bytes: (100 + r.below(1900)) * GB,
            table: Table::Gpt,
            id: "t".into(),
            first: 2048,
            ..Disk::default()
        };
        d.last = d.bytes / 512 - 34;
        let mut at = 2048 + r.below(4) * 2048;
        for n in 1..=(1 + r.below(7) as u32) {
            let sectors = (1 + r.below(200)) * GB / 512 / 2048 * 2048;
            if at + sectors >= d.last {
                break;
            }
            let (kind, fs) = match r.below(6) {
                0 => (ESP_TYPE, "vfat"),
                1 => ("ebd0a0a2-b9e5-4433-87c0-68b6b72699c7", "ntfs"),
                2 => (DATA_TYPE, "ext4"),
                3 => (DATA_TYPE, "btrfs"),
                4 => (DATA_TYPE, "xfs"),
                _ => (SWAP_TYPE, "swap"),
            };
            let bytes = sectors * 512;
            d.parts.push(Part {
                number: n,
                path: format!("/dev/vda{n}"),
                start: at,
                sectors,
                kind: kind.into(),
                uuid: format!("u{n}"),
                fstype: fs.into(),
                fsuuid: format!("f{n}"),
                min_bytes: Some(bytes / (2 + r.below(8))),
                free_bytes: Some(r.below(2000) * MIB),
                mounted: r.below(10) == 0,
                ..Part::default()
            });
            at += sectors + r.below(3) * 2048 * 1024;
        }
        d
    }

    fn random_layout(r: &mut Rng, d: &Disk) -> Layout {
        let mut l = Layout::default();
        let fss = [Fs::Btrfs, Fs::Ext4, Fs::Xfs];
        for p in &d.parts {
            match r.below(6) {
                0 => l.delete.push(p.uuid.clone()),
                1 => l.resize.push(Resize {
                    part: p.uuid.clone(),
                    bytes: (d.bytes_of(p) / (1 + r.below(4))) / MIB * MIB,
                }),
                2 => l.assign.push(Assign {
                    part: p.uuid.clone(),
                    mount: ["/home", "/var", "/data", "/efi", "/"][r.below(5) as usize].into(),
                    format: [None, Some(Fs::Ext4)][r.below(2) as usize],
                }),
                _ => {}
            }
        }
        if r.below(2) == 0 {
            l.create.push(esp_new());
        }
        l.create.push(New {
            bytes: if r.below(2) == 0 {
                None
            } else {
                Some((16 + r.below(100)) * GB)
            },
            mount: "/".into(),
            fs: fss[r.below(3) as usize],
        });
        if r.below(3) == 0 {
            l.create.push(New {
                bytes: Some((1 + r.below(50)) * GB),
                mount: "/home".into(),
                fs: Fs::Ext4,
            });
        }
        l
    }

    /// Thousands of random disks and layouts: whatever compiles names only the chosen
    /// partitions, and laying it out leaves every other partition where it was.
    #[test]
    fn no_layout_touches_a_partition_it_does_not_name() {
        let mut r = Rng(0x9E37_79B9_7F4A_7C15);
        let (mut accepted, mut refused) = (0, 0);
        for _ in 0..20_000 {
            let d = random_disk(&mut r);
            let l = random_layout(&mut r, &d);
            let Ok(c) = compile(&l, &d, r.below(2) == 0) else {
                refused += 1;
                continue;
            };
            accepted += 1;
            verify(&c, &l, &d).unwrap();
            let chosen = l.chosen();
            let mut after = d.clone();
            for op in &c.ops {
                match op {
                    Op::Delete { uuid, .. } => {
                        assert!(chosen.contains(uuid.as_str()));
                        after.parts.retain(|p| &p.uuid != uuid);
                    }
                    Op::ShrinkPart { uuid, sectors, .. } | Op::GrowPart { uuid, sectors, .. } => {
                        assert!(chosen.contains(uuid.as_str()));
                        after
                            .parts
                            .iter_mut()
                            .find(|p| &p.uuid == uuid)
                            .unwrap()
                            .sectors = *sectors;
                    }
                    Op::ShrinkFs { uuid, .. } | Op::GrowFs { uuid, .. } => {
                        assert!(chosen.contains(uuid.as_str()))
                    }
                    Op::Format {
                        target: Target::Existing(u),
                        ..
                    } => {
                        assert!(chosen.contains(u.as_str()))
                    }
                    _ => {}
                }
            }
            unchanged(&d, &after, &chosen).unwrap();
            // Nothing overlaps once the new partitions are in, and all stay on the disk.
            let mut ext: Vec<(u64, u64)> =
                after.parts.iter().map(|p| (p.start, p.sectors)).collect();
            ext.extend(c.ops.iter().filter_map(|o| match o {
                Op::Create { start, sectors, .. } => Some((*start, *sectors)),
                _ => None,
            }));
            ext.sort();
            assert!(ext.windows(2).all(|w| w[0].0 + w[0].1 <= w[1].0), "{ext:?}");
            assert!(
                ext.iter()
                    .all(|(s, n)| *s >= d.first && s + n - 1 <= d.last)
            );
        }
        assert!(
            accepted > 300 && refused > 300,
            "{accepted} accepted, {refused} refused"
        );
    }

    #[test]
    fn room_shows_what_a_delete_or_a_shrink_frees() {
        let d = room_disk();
        let before: u64 = room(&Layout::default(), &d).iter().map(|e| e.1).sum();
        let l = Layout {
            delete: vec!["a".into()],
            ..Layout::default()
        };
        let after: u64 = room(&l, &d).iter().map(|e| e.1).sum();
        assert_eq!(after, before + d.parts[0].sectors);
        let l = Layout {
            resize: vec![Resize {
                part: "a".into(),
                bytes: 2 * GIB,
            }],
            create: vec![New {
                bytes: Some(GIB),
                mount: "/efi".into(),
                fs: Fs::Vfat,
            }],
            ..Layout::default()
        };
        let sized: u64 = room(&l, &d).iter().map(|e| e.1).sum();
        assert_eq!(sized, before + d.parts[0].sectors - 3 * GIB / d.sector);
        let rest = Layout {
            create: vec![New {
                bytes: None,
                mount: "/".into(),
                fs: Fs::Btrfs,
            }],
            ..Layout::default()
        };
        assert!(room(&rest, &d).is_empty(), "the rest of the disk is taken");
    }

    fn room_disk() -> Disk {
        let s = 512;
        Disk {
            path: "/dev/x".into(),
            bytes: 100 * GIB,
            sector: s,
            table: Table::Gpt,
            first: 2048,
            last: 100 * GIB / s - 34,
            parts: vec![Part {
                number: 1,
                start: 2048,
                sectors: 10 * GIB / s,
                uuid: "a".into(),
                ..Part::default()
            }],
            ..Disk::default()
        }
    }
}
