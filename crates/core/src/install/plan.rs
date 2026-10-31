// SPDX-License-Identifier: GPL-2.0-or-later
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
}

impl Plan {
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
    },
    New {
        bytes: u64,
        mount: String,
        encrypted: bool,
    },
    /// An existing startup partition gets WolOS's startup files.
    ReuseEsp,
    Keep {
        name: String,
        mount: String,
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
            });
        }
    }
    for op in &c.ops {
        if let Op::Create { index, sectors, .. } = op {
            let mount = l.create[*index].mount.clone();
            let encrypted = protect != Protect::Off && mount == "/";
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
            });
        }
    }
    if c.slots == Slots::One {
        v.push(Change::OneCopy);
    }
    let named: Vec<&str> = l
        .delete
        .iter()
        .chain(l.resize.iter().map(|r| &r.part))
        .chain(l.assign.iter().map(|a| &a.part))
        .map(String::as_str)
        .collect();
    let mut seen = Vec::new();
    for p in d.parts.iter().filter(|p| !named.contains(&p.uuid.as_str())) {
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

/// xkb layout -> console keymap (the disk password is typed before any desktop exists).
pub fn console_keymap(xkb: &str) -> &'static str {
    match xkb {
        "br" => "br-abnt2",
        "de" => "de-latin1",
        "fr" => "fr-latin1",
        "es" => "es",
        "gb" => "uk",
        "it" => "it",
        "pt" => "pt-latin1",
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
            }),
        };
        let text = serde_json::to_string(&p).unwrap();
        assert!(!text.to_lowercase().contains("password\":\""));
        assert_eq!(serde_json::from_str::<Plan>(&text).unwrap(), p);
        assert_eq!(Protect::Password.word(), "passphrase");
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

    #[test]
    fn every_keyboard_has_a_console_map() {
        for (xkb, _) in crate::prefs::LAYOUTS {
            assert!(!console_keymap(xkb).is_empty());
        }
        assert_eq!(console_keymap("br"), "br-abnt2");
        assert_eq!(console_keymap("nothing"), "us");
    }
}
