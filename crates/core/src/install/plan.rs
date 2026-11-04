// SPDX-License-Identifier: BSD-2-Clause
//! The whole install as one file (ADR 0023): the Install WolOS app writes a [`Plan`], `install-run`
//! carries it out. [`changes`] is what the summary page shows, so what the person reads and what
//! runs come from the same data. Passwords are side files, never in the plan.
use super::disk::*;
use super::layout::*;
use serde::{Deserialize, Serialize};

/// How the system partition is protected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protect {
    #[default]
    Off,
    /// LUKS2, a password at every start, and a recovery key.
    Password,
    /// LUKS2, unlocked by the security chip (TPM2), and a recovery key.
    Tpm,
}

impl Protect {
    /// `WOLOS_ENCRYPT` for install-run (`wol encrypt decide`).
    pub fn word(self) -> &'static str {
        match self {
            Protect::Off => "no",
            Protect::Password => "passphrase",
            Protect::Tpm => "yes",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub full_name: String,
    /// Monogram colour (`shared::login::AVATAR_TONES`).
    pub tone: String,
    /// The password hint from the account page ("" = none). A plan written before 0.13 has
    /// none (QA-LIVE-24: the installer dropped it, the lock screen never showed one).
    #[serde(default)]
    pub hint: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub disk: String,
    /// [`Disk::fingerprint`] of what the person looked at.
    pub fingerprint: String,
    pub layout: Layout,
    pub protect: Protect,
    /// `ll_CC.UTF-8`.
    pub locale: String,
    /// xkb layout.
    pub keyboard: String,
    /// tz database name.
    pub zone: String,
    pub account: Option<Account>,
    /// Secure Boot protection wanted (`WOLOS_SECURE`). None: a plan written before the installer
    /// asked, which keeps the old answer, yes. The installer asks with the default off (lead,
    /// 2026-11-02: the one-time key screen is opt-in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secure_boot: Option<bool>,
    /// Optional modules ticked on the Modules page (ADR 0032 B), registry ids
    /// ([`crate::modules::valid_id`]); none by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<String>,
    /// Store kits added on the Extras page (`codecs-kit`, [`crate::store::KITS`]); none by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extras: Vec<String>,
}

impl Plan {
    /// The modules to install: valid ids, first mention first. A bad id is dropped, never run.
    pub fn module_ids(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for m in &self.modules {
            if crate::modules::valid_id(m) && !v.contains(&m.as_str()) {
                v.push(m);
            }
        }
        v
    }

    /// The packages of the Store kits on the plan (known kits that install packages only), first
    /// mention first, each a checked package name. An unknown kit adds nothing.
    pub fn extra_packages(&self) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = Vec::new();
        for k in self.extras.iter().filter_map(|id| crate::store::kit(id)) {
            if k.source != crate::store::KitSource::Packages {
                continue;
            }
            for p in k.packages() {
                if crate::store::is_pkg_name(p) && !v.contains(&p) {
                    v.push(p);
                }
            }
        }
        v
    }

    /// Re-checked against the disk as it is now, just before anything is written.
    pub fn check(&self, now: &Disk) -> Result<Compiled, Refusal> {
        if now.path != self.disk || now.fingerprint() != self.fingerprint {
            return Err(Refusal::DiskChanged);
        }
        let c = compile(&self.layout, now, self.protect != Protect::Off)?;
        verify(&c, &self.layout, now)?;
        Ok(c)
    }
}

/// One line of the summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// The whole disk goes; `others`: the systems on it.
    EraseDisk {
        bytes: u64,
        others: Vec<String>,
    },
    Delete {
        name: String,
        bytes: u64,
    },
    Resize {
        name: String,
        from: u64,
        to: u64,
    },
    Format {
        name: String,
        mount: String,
        encrypted: bool,
    },
    New {
        bytes: u64,
        mount: String,
        encrypted: bool,
    },
    /// An existing startup partition gets WolOS's startup files.
    ReuseEsp,
    /// `plain`: the install is encrypted but this volume, kept as it is, is not.
    Keep {
        name: String,
        mount: String,
        plain: bool,
    },
    /// Stays exactly as it is.
    Untouched {
        name: String,
    },
    /// The layout keeps one copy of WolOS: no undo for updates.
    OneCopy,
}

/// What will change, biggest first: the summary page lists exactly this.
pub fn changes(l: &Layout, c: &Compiled, d: &Disk, protect: Protect) -> Vec<Change> {
    let mut v = Vec::new();
    if l.wipe {
        v.push(Change::EraseDisk {
            bytes: d.bytes,
            others: d.others(),
        });
        v.push(Change::New {
            bytes: d.bytes,
            mount: "/".into(),
            encrypted: protect != Protect::Off,
        });
        return v;
    }
    for id in &l.delete {
        if let Some(p) = d.part(id) {
            v.push(Change::Delete {
                name: p.title(),
                bytes: d.bytes_of(p),
            });
        }
    }
    for r in &l.resize {
        if let Some(p) = d.part(&r.part) {
            v.push(Change::Resize {
                name: p.title(),
                from: d.bytes_of(p),
                to: r.bytes,
            });
        }
    }
    for a in l.assign.iter().filter(|a| a.format.is_some()) {
        if let Some(p) = d.part(&a.part) {
            v.push(Change::Format {
                name: p.title(),
                mount: a.mount.clone(),
                encrypted: protect != Protect::Off && a.mount != "/efi",
            });
        }
    }
    for op in &c.ops {
        if let Op::Create { index, sectors, .. } = op {
            let mount = l.create[*index].mount.clone();
            let encrypted = protect != Protect::Off && mount != "/efi";
            v.push(Change::New {
                bytes: sectors * d.sector,
                mount,
                encrypted,
            });
        }
    }
    for a in l.assign.iter().filter(|a| a.format.is_none()) {
        if a.mount == "/efi" {
            v.push(Change::ReuseEsp);
        } else if let Some(p) = d.part(&a.part) {
            v.push(Change::Keep {
                name: p.title(),
                mount: a.mount.clone(),
                plain: protect != Protect::Off,
            });
        }
    }
    if c.slots == Slots::One {
        v.push(Change::OneCopy);
    }
    // The systems that stay ([`Layout::stays_foreign`]), less the ones a Resize line already names.
    let mut seen = Vec::new();
    let untouched = d
        .parts
        .iter()
        .filter(|p| l.stays_foreign(p) && !l.resize.iter().any(|r| r.part == p.uuid));
    for p in untouched {
        if let Some(name) = p.system().filter(|n| !seen.contains(n)) {
            seen.push(name.clone());
            v.push(Change::Untouched { name });
        }
    }
    v
}

/// An existing startup partition with room for WolOS, if the disk has one.
pub fn reusable_esp(d: &Disk) -> Option<&Part> {
    d.parts.iter().find(|p| {
        p.is_esp()
            && p.fstype == "vfat"
            && !p.mounted
            && p.free_bytes.unwrap_or(0) >= ESP_FREE_NEEDED
    })
}

/// The partition Install Alongside makes room in: the biggest one that can shrink.
pub fn alongside_target(d: &Disk) -> Option<&Part> {
    d.parts
        .iter()
        .filter(|p| shrink_range(d, p).is_some())
        .max_by_key(|p| p.sectors)
}

/// Why Install Alongside can't shrink the biggest system, when something blocks it.
pub fn alongside_blocked(d: &Disk) -> Option<Blocked> {
    let big = d.parts.iter().max_by_key(|p| p.sectors)?;
    big.blocked
}

/// Install Alongside, keeping `keep` bytes of `target` (None: in the free space as it is).
/// The startup partition is reused when it has room, else WolOS gets its own.
pub fn alongside(d: &Disk, shrink: Option<(&str, u64)>) -> Layout {
    let mut l = Layout::default();
    if let Some((part, bytes)) = shrink {
        l.resize.push(Resize {
            part: part.to_string(),
            bytes,
        });
    }
    match reusable_esp(d) {
        Some(p) => l.assign.push(Assign {
            part: p.uuid.clone(),
            mount: "/efi".into(),
            format: None,
        }),
        None => l.create.push(New {
            bytes: Some(ESP_BYTES),
            mount: "/efi".into(),
            fs: Fs::Vfat,
        }),
    }
    l.create.push(New {
        bytes: None,
        mount: "/".into(),
        fs: Fs::Btrfs,
    });
    l
}

/// Replace: `part` is erased for the system; the startup partition is reused when it has room,
/// else WolOS gets its own. A disk with no free gap for that one (the usual 100 MiB Windows
/// startup partition, a full disk) gives it the tail of the replaced partition: a partition being
/// reformatted shrinks without a file system shrink.
pub fn replace(d: &Disk, part: &str) -> Layout {
    let mut l = Layout::default();
    l.assign.push(Assign {
        part: part.to_string(),
        mount: "/".into(),
        format: Some(Fs::Btrfs),
    });
    if let Some(p) = reusable_esp(d) {
        l.assign.push(Assign {
            part: p.uuid.clone(),
            mount: "/efi".into(),
            format: None,
        });
        return l;
    }
    l.create.push(New {
        bytes: Some(ESP_BYTES),
        mount: "/efi".into(),
        fs: Fs::Vfat,
    });
    let fits = room(
        &Layout {
            create: Vec::new(),
            ..l.clone()
        },
        d,
    )
    .iter()
    .any(|(_, sectors)| sectors * d.sector >= ESP_BYTES);
    if let Some(p) = d.part(part).filter(|_| !fits) {
        // A MiB more than the startup partition: the gap is cut to whole MiB on both ends.
        let keep = d.bytes_of(p).saturating_sub(ESP_BYTES + MIB) / MIB * MIB;
        l.resize.push(Resize {
            part: part.to_string(),
            bytes: keep,
        });
    }
    l
}

/// The system the person must be told to start from the computer's startup menu (F12, F9, Esc)
/// after this install: another system stays with its own startup partition while WolOS gets a new
/// one (audit2 B3, audit3 R2B-11). systemd-boot lists only what is on its own startup partition,
/// so that system is not in WolOS's menu. Its name as [`Disk::others_after`] gives it ("Windows",
/// "Fedora Linux 41"), for a system that can start (Windows, or a Linux the probe found an
/// os-release in); None: nothing to say (Erase Disk, a reused startup partition, nothing that
/// starts stays). The Install app's summary and Done page say it.
pub fn other_boot(l: &Layout, d: &Disk) -> Option<String> {
    if l.wipe || !l.create.iter().any(|n| n.mount == "/efi") {
        return None;
    }
    if !d.parts.iter().any(|p| p.is_esp() && l.stays(p)) {
        return None;
    }
    // Only what can start: Windows, or a Linux whose os-release the probe read. A bare ext4
    // data partition is "Linux" by its file system but starts nothing.
    let starts: Vec<&Part> = d
        .parts
        .iter()
        .filter(|p| l.stays_foreign(p))
        .filter(|p| p.os.is_some() || p.system().as_deref() == Some("Windows"))
        .collect();
    // Windows first: it is the one whose own menu never lists WolOS either.
    starts
        .iter()
        .find(|p| p.system().as_deref() == Some("Windows"))
        .or(starts.first())
        .and_then(|p| p.system())
}

/// [`other_boot`] for Windows only: `firmware-menu` when Windows is the system to start from the
/// computer's startup menu. Kept for the app until it reads [`other_boot`].
pub fn windows_boot(l: &Layout, d: &Disk) -> Option<&'static str> {
    other_boot(l, d)
        .filter(|s| s == "Windows")
        .map(|_| "firmware-menu")
}

/// Windows stays on this disk after `l`: its clock convention (local time in the RTC) is kept and
/// its BitLocker may ask for its recovery key after a Secure Boot key change (audit3 R2B-9/12).
pub fn keeps_windows(l: &Layout, d: &Disk) -> bool {
    d.others_after(l).iter().any(|s| s == "Windows")
}

/// A Windows that stays after `l` is protected by BitLocker ([`Part::bitlocker`]).
pub fn keeps_bitlocker(l: &Layout, d: &Disk) -> bool {
    d.parts
        .iter()
        .any(|p| p.bitlocker() && l.stays(p) && !l.assign.iter().any(|a| a.part == p.uuid))
}

/// What the slider moves between: the most and the least of `p` that can be kept so that WolOS
/// still fits. None: there is no such size.
pub fn alongside_range(d: &Disk, p: &Part) -> Option<(u64, u64)> {
    let (lo, size) = shrink_range(d, p)?;
    let lo = lo.div_ceil(MIB) * MIB;
    let fits = |keep: u64| compile(&alongside(d, Some((&p.uuid, keep))), d, false).is_ok();
    if !fits(lo) {
        return None;
    }
    // The largest keep that still fits, to the MiB.
    let (mut good, mut bad) = (lo, size);
    while bad - good > MIB {
        let mid = (good + (bad - good) / 2) / MIB * MIB;
        if mid == good {
            break;
        }
        if fits(mid) {
            good = mid;
        } else {
            bad = mid;
        }
    }
    Some((lo, good))
}

/// Where the slider opens: half way between [`alongside_range`]'s ends, unless that leaves WolOS
/// less than [`ROOMY_ROOT`] while Windows could give it that much (QA-DOC-32: on a 64 GB disk
/// the half way split opened at 39 GB under the page's own 'leaves room' warning). Then the
/// most Windows can keep with WolOS at [`ROOMY_ROOT`], to the MiB.
pub fn alongside_default(d: &Disk, p: &Part) -> Option<u64> {
    let (lo, hi) = alongside_range(d, p)?;
    let root = |keep: u64| {
        compile(&alongside(d, Some((&p.uuid, keep))), d, false)
            .map(|c| c.root_bytes)
            .unwrap_or(0)
    };
    let mid = lo + (hi - lo) / 2;
    if root(mid) >= ROOMY_ROOT || root(lo) < ROOMY_ROOT {
        return Some(mid);
    }
    // root(lo) is roomy, root(mid) is not: the largest roomy keep between them.
    let (mut good, mut bad) = (lo, mid);
    while bad - good > MIB {
        let m = (good + (bad - good) / 2) / MIB * MIB;
        if m == good {
            break;
        }
        if root(m) >= ROOMY_ROOT {
            good = m;
        } else {
            bad = m;
        }
    }
    Some(good)
}

/// A password typed at the disk's prompt, before any desktop exists, on the console keymap:
/// printable ASCII is what every keymap types the same.
pub fn console_typable(p: &str) -> bool {
    p.chars().all(|c| (' '..='~').contains(&c))
}

/// Why the plan's passwords would not open the disk at the start prompt; None: they would.
/// With the security chip the account password is the disk's own fallback (when a firmware
/// change makes the chip refuse), so it must be typable there too (audit2 B9).
/// `UntypablePassword` is the word `wol install check` refuses with; the app says
/// `shell::copy::INSTALL_PASSPHRASE_ASCII` on the account password field.
pub fn unlock_problem(protect: Protect, account_password: Option<&str>) -> Option<&'static str> {
    match (protect, account_password) {
        (Protect::Tpm, Some(pw))
            if !crate::encrypt::typed_secret(pw.as_bytes()).is_ok_and(|t| console_typable(&t)) =>
        {
            Some("UntypablePassword")
        }
        _ => None,
    }
}

/// xkb layout -> console keymap (the disk password is typed before any desktop exists).
pub fn console_keymap(xkb: &str) -> &'static str {
    match xkb {
        "br" => "br-abnt2",
        "de" => "de-latin1",
        "fr" => "fr-latin1",
        "es" => "es",
        "gb" => "uk",
        "it" => "it",
        "jp" => "jp106",
        "pt" => "pt-latin1",
        "ru" => "ru",
        _ => "us",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1_000_000_000;

    fn part(n: u32, start_gb: u64, gb: u64, kind: &str, fs: &str) -> Part {
        Part {
            number: n,
            path: format!("/dev/vda{n}"),
            start: start_gb * GB / 512 / 2048 * 2048 + 2048,
            sectors: gb * GB / 512 / 2048 * 2048,
            kind: kind.into(),
            uuid: format!("u{n}"),
            fstype: fs.into(),
            fsuuid: format!("f{n}"),
            ..Part::default()
        }
    }

    /// 200 GB disk: small ESP, Windows 100 GB (50 GB used), Linux home on ext4 40 GB, free tail.
    fn disk() -> Disk {
        let mut esp = part(1, 0, 1, ESP_TYPE, "vfat");
        esp.free_bytes = Some(800 * MIB);
        let mut win = part(2, 2, 100, "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7", "ntfs");
        win.min_bytes = Some(50 * GB);
        let mut linux = part(3, 110, 40, DATA_TYPE, "ext4");
        linux.os = Some("Fedora Linux 41".into());
        linux.min_bytes = Some(5 * GB);
        Disk {
            path: "/dev/vda".into(),
            bytes: 200 * GB,
            sector: 512,
            table: Table::Gpt,
            id: "t".into(),
            first: 2048,
            last: 200 * GB / 512 - 34,
            parts: vec![esp, win, linux],
            ..Disk::default()
        }
    }

    #[test]
    fn with_the_chip_the_account_password_must_be_typable_at_the_start_prompt() {
        assert_eq!(
            unlock_problem(Protect::Tpm, Some("caf\u{e9}12345")),
            Some("UntypablePassword")
        );
        assert_eq!(unlock_problem(Protect::Tpm, Some("cafe 12345~\n")), None);
        assert_eq!(unlock_problem(Protect::Tpm, None), None, "no account");
        // Without the chip the account password never opens the disk.
        for p in [Protect::Off, Protect::Password] {
            assert_eq!(unlock_problem(p, Some("caf\u{e9}12345")), None);
        }
        assert!(console_typable("Abc 123 ~!@#") && !console_typable("tab\there"));
    }

    #[test]
    fn a_new_startup_partition_next_to_windows_is_said_how_windows_starts() {
        let mut d = disk();
        assert_eq!(
            windows_boot(&alongside(&d, None), &d),
            None,
            "reused: in the menu"
        );
        assert_eq!(windows_boot(&Layout::erase(), &d), None, "Windows goes");
        d.parts[0].free_bytes = Some(60 * MIB);
        let l = alongside(&d, Some(("u2", 60 * GB)));
        assert_eq!(windows_boot(&l, &d), Some("firmware-menu"));
        let mut gone = l.clone();
        gone.delete.push("u2".into());
        assert_eq!(windows_boot(&gone, &d), None, "Windows deleted");
        assert_eq!(
            windows_boot(&replace(&d, "u2"), &d),
            None,
            "Windows replaced"
        );
        assert_eq!(windows_boot(&replace(&d, "u3"), &d), Some("firmware-menu"));
    }

    /// audit3 R2B-2: one "what stays" rule. Replacing the only other Linux leaves Windows alone on
    /// the disk; a kept /home is WolOS's own, not another system; Erase keeps nothing.
    #[test]
    fn what_stays_is_one_rule_for_the_menu_the_summary_and_the_startup_hint() {
        let d = disk();
        assert_eq!(d.others_after(&replace(&d, "u3")), ["Windows"]);
        assert_eq!(d.others_after(&replace(&d, "u2")), ["Fedora Linux 41"]);
        assert!(d.others_after(&Layout::erase()).is_empty());
        let mut home = alongside(&d, Some(("u2", 60 * GB)));
        home.assign.push(Assign {
            part: "u3".into(),
            mount: "/home".into(),
            format: None,
        });
        assert_eq!(
            d.others_after(&home),
            ["Windows"],
            "a kept /home is WolOS's"
        );
        // Windows' helper partitions do not keep "Windows" alive once its system partition goes.
        let mut msr = disk();
        let mut r = part(4, 152, 1, "e3c9e316-0b5c-4db8-817d-f92df00215ae", "");
        r.uuid = "u4".into();
        msr.parts.push(r);
        assert_eq!(msr.others_after(&replace(&msr, "u2")), ["Fedora Linux 41"]);
        // The summary's "stays" lines are the same systems, less the one a Resize line names.
        for l in [
            alongside(&d, Some(("u2", 60 * GB))),
            replace(&d, "u3"),
            replace(&d, "u2"),
            home,
        ] {
            let c = compile(&l, &d, false).unwrap_or_else(|r| panic!("{r:?} {l:?}"));
            let untouched: Vec<String> = changes(&l, &c, &d, Protect::Off)
                .into_iter()
                .filter_map(|c| match c {
                    Change::Untouched { name } => Some(name),
                    _ => None,
                })
                .collect();
            let resized: Vec<String> = l
                .resize
                .iter()
                .filter_map(|r| d.part(&r.part).and_then(Part::system))
                .collect();
            let expect: Vec<String> = d
                .others_after(&l)
                .into_iter()
                .filter(|s| !resized.contains(s))
                .collect();
            assert_eq!(untouched, expect, "{l:?}");
        }
    }

    /// audit3 R2B-11: a Linux kept next to a new startup partition is named too, not only Windows.
    #[test]
    fn a_new_startup_partition_names_the_system_left_on_the_old_one() {
        let mut d = disk();
        d.parts[0].free_bytes = Some(60 * MIB);
        assert_eq!(
            other_boot(&replace(&d, "u2"), &d).as_deref(),
            Some("Fedora Linux 41")
        );
        assert_eq!(
            other_boot(&replace(&d, "u3"), &d).as_deref(),
            Some("Windows")
        );
        assert_eq!(windows_boot(&replace(&d, "u2"), &d), None, "not Windows");
        assert!(keeps_windows(&replace(&d, "u3"), &d));
        assert!(!keeps_windows(&replace(&d, "u2"), &d));
        assert!(!keeps_windows(&Layout::erase(), &d));
        // A data partition with no system on it starts nothing: nothing to say.
        let mut bare = d.clone();
        bare.parts[2].os = None;
        assert_eq!(other_boot(&replace(&bare, "u2"), &bare), None);
        // Reused startup partition: systemd-boot lists what is on it, nothing to say.
        let roomy = disk();
        assert_eq!(other_boot(&replace(&roomy, "u2"), &roomy), None);
    }

    #[test]
    fn bitlocker_is_known_only_while_its_windows_stays() {
        let mut d = disk();
        d.parts[1].fstype = "BitLocker".into();
        d.parts[1].blocked = Some(Blocked::BitLocker);
        assert!(keeps_bitlocker(&replace(&d, "u3"), &d));
        assert!(!keeps_bitlocker(&replace(&d, "u2"), &d), "replaced");
        assert!(!keeps_bitlocker(&Layout::erase(), &d));
        assert!(
            !keeps_bitlocker(&replace(&disk(), "u3"), &disk()),
            "no BitLocker"
        );
    }

    /// audit3 R2B-5: reinstalling WolOS next to Windows reuses the old WolOS's startup partition
    /// when that WolOS goes; it is refused only while another WolOS system stays.
    #[test]
    fn an_old_wolos_startup_partition_is_reused_when_its_wolos_goes() {
        let mut d = disk();
        d.parts[0].wolos = true;
        d.parts[2].os = None;
        d.parts[2].fstype = "btrfs".into();
        d.parts[2].name = "WolOS".into();
        let l = replace(&d, "u3");
        assert_eq!(l.assign[1].part, "u1", "{l:?}");
        assert!(compile(&l, &d, true).is_ok(), "{:?}", compile(&l, &d, true));
        let mut kept = d.clone();
        let mut other = part(4, 152, 20, ROOT_TYPE, "crypto_LUKS");
        other.label = "WolOS".into();
        other.name = String::new();
        kept.parts.push(other);
        assert_eq!(
            compile(&replace(&kept, "u3"), &kept, true),
            Err(Refusal::EspHasWolos("u1".into()))
        );
    }

    /// A full disk: Windows' 100 MiB startup partition (60 MiB free), Windows, a Linux to the end.
    fn full_disk() -> Disk {
        let mut esp = part(1, 0, 1, ESP_TYPE, "vfat");
        esp.sectors = 100 * MIB / 512;
        esp.free_bytes = Some(60 * MIB);
        let mut win = part(2, 1, 100, "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7", "ntfs");
        win.min_bytes = Some(50 * GB);
        let mut linux = part(3, 102, 60, DATA_TYPE, "ext4");
        linux.os = Some("Fedora Linux 41".into());
        let last = linux.start + linux.sectors - 1;
        Disk {
            path: "/dev/vda".into(),
            bytes: (last + 34) * 512,
            sector: 512,
            table: Table::Gpt,
            id: "t".into(),
            first: 2048,
            last,
            parts: vec![esp, win, linux],
            ..Disk::default()
        }
    }

    /// audit2 B6: Replace on a full disk with a small Windows startup partition used to end in
    /// "There isn't enough room for that" every time.
    #[test]
    fn replace_on_a_full_disk_gives_the_startup_partition_the_replaced_ones_tail() {
        let d = full_disk();
        assert!(reusable_esp(&d).is_none());
        assert!(
            room(&Layout::default(), &d)
                .iter()
                .all(|(_, s)| s * 512 < ESP_BYTES)
        );
        let l = replace(&d, "u3");
        let c = compile(&l, &d, true).unwrap_or_else(|r| panic!("{r:?}"));
        verify(&c, &l, &d).unwrap();
        assert_eq!(l.resize.len(), 1, "{l:?}");
        assert_eq!(l.resize[0].part, "u3");
        assert!(c.root_bytes >= MIN_ROOT && c.root_bytes < d.bytes_of(&d.parts[2]));
        // Windows and its startup partition are not touched.
        assert_eq!(l.chosen().into_iter().collect::<Vec<_>>(), ["u3"]);
        // With room to spare nothing shrinks; with a startup partition to reuse, nothing is made.
        let roomy = disk();
        let l = replace(&roomy, "u3");
        assert!(l.resize.is_empty() && l.create.is_empty(), "{l:?}");
        assert!(compile(&l, &roomy, false).is_ok());
        let mut small = disk();
        small.parts[0].free_bytes = Some(60 * MIB);
        let l = replace(&small, "u3");
        assert!(
            l.resize.is_empty() && l.create.len() == 1,
            "free tail: {l:?}"
        );
        assert!(compile(&l, &small, false).is_ok());
    }

    #[test]
    fn a_plan_only_runs_on_the_disk_it_was_made_for() {
        let d = disk();
        let p = Plan {
            disk: d.path.clone(),
            fingerprint: d.fingerprint(),
            layout: alongside(&d, None),
            ..Plan::default()
        };
        assert!(p.check(&d).is_ok());
        let mut moved = d.clone();
        moved.parts[2].sectors -= 2048;
        assert_eq!(p.check(&moved), Err(Refusal::DiskChanged));
        let mut other = d.clone();
        other.path = "/dev/vdb".into();
        assert_eq!(p.check(&other), Err(Refusal::DiskChanged));
    }

    #[test]
    fn the_plan_file_round_trips_and_holds_no_secret() {
        let d = disk();
        let p = Plan {
            disk: d.path.clone(),
            fingerprint: d.fingerprint(),
            layout: alongside(&d, Some(("u2", 60 * GB))),
            protect: Protect::Password,
            locale: "pt_BR.UTF-8".into(),
            keyboard: "br".into(),
            zone: "America/Sao_Paulo".into(),
            account: Some(Account {
                name: "alice".into(),
                full_name: "Alice".into(),
                tone: "violet".into(),
                hint: String::new(),
            }),
            secure_boot: Some(false),
            modules: vec!["dev".into(), "creative".into()],
            extras: vec!["codecs-kit".into()],
        };
        let text = serde_json::to_string(&p).unwrap();
        assert!(!text.to_lowercase().contains("password\":\""));
        assert_eq!(serde_json::from_str::<Plan>(&text).unwrap(), p);
        assert_eq!(Protect::Password.word(), "passphrase");
    }

    #[test]
    fn a_plan_from_before_the_new_pages_keeps_its_old_answers() {
        let old = Plan::default();
        let text = serde_json::to_string(&old).unwrap();
        assert!(
            !text.contains("secure_boot") && !text.contains("modules") && !text.contains("extras"),
            "{text}"
        );
        let back: Plan = serde_json::from_str(&text).unwrap();
        assert_eq!((back.secure_boot, back.modules.len()), (None, 0));
    }

    #[test]
    fn only_checked_modules_and_known_kits_reach_the_run() {
        let p = Plan {
            modules: vec![
                "dev".into(),
                "dev".into(),
                "Bad;rm".into(),
                "../x".into(),
                "creative".into(),
            ],
            extras: vec!["codecs-kit".into(), "nonsense".into(), "craft-suite".into()],
            ..Plan::default()
        };
        assert_eq!(p.module_ids(), ["dev", "creative"]);
        let pk = p.extra_packages();
        // The codecs kit's packages, MP3 first (its priority order); the wopkg kit adds none.
        assert_eq!(pk.first(), Some(&"gst-plugins-good"));
        assert!(pk.contains(&"x265") && pk.iter().all(|n| crate::store::is_pkg_name(n)));
        let mut seen = pk.clone();
        seen.dedup();
        assert_eq!(seen.len(), pk.len());
    }

    #[test]
    fn alongside_reuses_a_startup_partition_with_room_else_makes_its_own() {
        let mut d = disk();
        let l = alongside(&d, None);
        assert_eq!(l.assign[0].part, "u1");
        assert_eq!(l.create.len(), 1);
        d.parts[0].free_bytes = Some(60 * MIB);
        let l = alongside(&d, None);
        assert!(l.assign.is_empty());
        assert_eq!(l.create[0].mount, "/efi");
    }

    #[test]
    fn the_slider_stops_where_either_side_would_not_fit() {
        let d = disk();
        let win = &d.parts[1];
        assert_eq!(
            alongside_target(&d).unwrap().uuid,
            "u2",
            "the biggest that can shrink"
        );
        let (lo, hi) = alongside_range(&d, win).unwrap();
        assert!(
            lo >= 50 * GB + SPARE_BYTES,
            "Windows keeps what it uses and room to work"
        );
        assert!(lo % MIB == 0 && hi % MIB == 0 && lo <= hi);
        // At the top WolOS has just its minimum; one more MiB and it does not fit.
        let at_hi = compile(&alongside(&d, Some(("u2", hi))), &d, false).unwrap();
        assert!(at_hi.root_bytes >= MIN_ROOT);
        assert!(
            compile(&alongside(&d, Some(("u2", hi + 2 * MIB))), &d, false).is_err()
                || hi + 2 * MIB >= d.bytes_of(win)
        );
        // No room at all: the disk is full and nothing can give enough.
        let mut full = disk();
        full.parts[1].min_bytes = Some(99 * GB);
        full.parts[1].sectors = 100 * GB / 512 / 2048 * 2048;
        full.last = (150 * GB + 2048 * 512) / 512;
        assert_eq!(alongside_range(&full, &full.parts[1]), None);
    }

    /// QA-DOC-32: a 64 GB disk, Windows using 344 MB: the slider opened at WolOS 39 GB, under
    /// the page's own '40 GB or more leaves room' warning. It opens at 40 GB or more now; a big
    /// disk keeps the half way split.
    #[test]
    fn alongside_default_is_roomy_on_64gb() {
        let mut esp = part(1, 0, 1, ESP_TYPE, "vfat");
        esp.free_bytes = Some(800 * MIB);
        let mut win = part(2, 1, 62, "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7", "ntfs");
        win.min_bytes = Some(344 * MIB);
        let d = Disk {
            path: "/dev/vda".into(),
            bytes: 64 * GB,
            sector: 512,
            table: Table::Gpt,
            id: "t".into(),
            first: 2048,
            last: 64 * GB / 512 - 34,
            parts: vec![esp, win],
            ..Disk::default()
        };
        let w = &d.parts[1];
        let root = |keep: u64| {
            compile(&alongside(&d, Some(("u2", keep))), &d, false)
                .unwrap()
                .root_bytes
        };
        let (lo, hi) = alongside_range(&d, w).unwrap();
        assert!(
            root(lo + (hi - lo) / 2) < ROOMY_ROOT,
            "the half way split is not roomy here"
        );
        let keep = alongside_default(&d, w).unwrap();
        assert!(root(keep) >= ROOMY_ROOT, "{}", root(keep));
        assert!(
            root(keep + 2 * MIB) < ROOMY_ROOT,
            "and Windows keeps the rest"
        );
        assert!((lo..=hi).contains(&keep) && keep.is_multiple_of(MIB));
        // A big disk: the half way split is roomy and stays.
        let big = disk();
        let (lo, hi) = alongside_range(&big, &big.parts[1]).unwrap();
        assert_eq!(
            alongside_default(&big, &big.parts[1]),
            Some(lo + (hi - lo) / 2)
        );
    }

    #[test]
    fn a_blocked_windows_is_named_not_skipped() {
        let mut d = disk();
        d.parts[1].blocked = Some(Blocked::BitLocker);
        assert_eq!(alongside_blocked(&d), Some(Blocked::BitLocker));
        assert_eq!(
            alongside_target(&d).unwrap().uuid,
            "u3",
            "only the Linux one can move"
        );
        assert_eq!(alongside_blocked(&disk()), None);
    }

    #[test]
    fn the_summary_says_everything_that_happens_and_what_stays() {
        let d = disk();
        let l = alongside(&d, Some(("u2", 60 * GB)));
        let c = compile(&l, &d, true).unwrap();
        let ch = changes(&l, &c, &d, Protect::Tpm);
        assert_eq!(
            ch[0],
            Change::Resize {
                name: "Windows".into(),
                from: d.bytes_of(&d.parts[1]),
                to: 60 * GB
            }
        );
        assert!(matches!(&ch[1], Change::New { mount, encrypted: true, .. } if mount == "/"));
        assert_eq!(ch[2], Change::ReuseEsp);
        assert_eq!(
            ch[3],
            Change::Untouched {
                name: "Fedora Linux 41".into()
            }
        );
        assert_eq!(ch.len(), 4);

        let e = compile(&Layout::erase(), &d, false).unwrap();
        let ch = changes(&Layout::erase(), &e, &d, Protect::Off);
        assert_eq!(
            ch[0],
            Change::EraseDisk {
                bytes: 200 * GB,
                others: vec!["Windows".into(), "Fedora Linux 41".into()]
            }
        );

        let mut l = alongside(&d, Some(("u2", 60 * GB)));
        l.create[0].fs = Fs::Ext4;
        let c = compile(&l, &d, false).unwrap();
        assert!(changes(&l, &c, &d, Protect::Off).contains(&Change::OneCopy));
    }

    /// Encrypted install: new and erased volumes are encrypted too, a kept one is said to stay plain.
    #[test]
    fn the_summary_says_which_volumes_are_encrypted_and_which_stay_plain() {
        let d = disk();
        let mut l = alongside(&d, Some(("u2", 60 * GB)));
        l.assign.push(Assign {
            part: "u3".into(),
            mount: "/home".into(),
            format: None,
        });
        l.create.push(New {
            bytes: Some(2 * GB),
            mount: "swap".into(),
            fs: Fs::Swap,
        });
        let c = compile(&l, &d, true).unwrap();
        let ch = changes(&l, &c, &d, Protect::Password);
        assert!(
            ch.iter().any(
                |c| matches!(c, Change::New { mount, encrypted: true, .. } if mount == "swap")
            ),
            "{ch:?}"
        );
        assert!(
            ch.iter()
                .any(|c| matches!(c, Change::Keep { mount, plain: true, .. } if mount == "/home")),
            "{ch:?}"
        );
        let ch = changes(&l, &c, &d, Protect::Off);
        assert!(
            ch.iter().all(|c| !matches!(
                c,
                Change::New {
                    encrypted: true,
                    ..
                } | Change::Keep { plain: true, .. }
            )),
            "{ch:?}"
        );
        // An erased volume is encrypted too; a new startup partition never is.
        l.assign.retain(|a| a.mount != "/efi");
        for a in l.assign.iter_mut().filter(|a| a.mount == "/home") {
            a.format = Some(Fs::Ext4);
        }
        l.create.insert(
            0,
            New {
                bytes: Some(GB),
                mount: "/efi".into(),
                fs: Fs::Vfat,
            },
        );
        let c = compile(&l, &d, true).unwrap();
        let ch = changes(&l, &c, &d, Protect::Tpm);
        assert!(
            ch.iter().any(
                |c| matches!(c, Change::Format { mount, encrypted: true, .. } if mount == "/home")
            ),
            "{ch:?}"
        );
        assert!(
            ch.iter().any(
                |c| matches!(c, Change::New { mount, encrypted: false, .. } if mount == "/efi")
            ),
            "{ch:?}"
        );
    }

    #[test]
    fn every_keyboard_has_a_console_map() {
        for (xkb, _) in crate::prefs::LAYOUTS {
            assert!(!console_keymap(xkb).is_empty());
        }
        assert_eq!(console_keymap("br"), "br-abnt2");
        assert_eq!(console_keymap("nothing"), "us");
    }
}
