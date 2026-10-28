//! System update with A/B root slots (ADR 0016): snapshot the running root into the other slot,
//! pin it to the new release's Arch Linux Archive date, `pacman -Syu` inside it, rebuild its
//! initramfs and boot entry (boot counting), boot it next. Packages the person installed with
//! pacman come along: the slot is a copy of the running root, never an image.
//! Pure: produces argv lists; the caller runs them in order and stops on the first failure,
//! then runs [`rollback`] (the new slot is deleted; the running slot stays the restore point).

pub mod check;

/// The two root slots (btrfs subvolumes `@wolos-a`, `@wolos-b`).
pub const SLOTS: [&str; 2] = ["wolos-a", "wolos-b"];
/// The btrfs top level (subvolid=5), mounted by the image.
pub const TOP: &str = "/run/wolos/top";
/// The EFI system partition.
pub const ESP: &str = "/efi";
/// Kernel and initramfs every slot boots (`linux-wolos`).
pub const KERNEL: &str = "vmlinuz-linux-wolos";
pub const INITRAMFS: &str = "initramfs-linux-wolos.img";
/// Boot counting: tries before systemd-boot falls back to the other slot.
pub const TRIES: u32 = 3;
/// pacman mirrorlist `wol` owns: one Arch Linux Archive date (`/repos/YYYY/MM/DD/`).
pub const MIRRORLIST: &str = "/etc/pacman.d/mirrorlist";

/// Pending package names from `pacman -Qu` (`name old -> new`); None when output unknown.
/// Inside one release only `[wolos]` moves: everything else is pinned to the snapshot date.
pub fn parse_pending(output: Option<&str>) -> Option<Vec<String>> {
    Some(
        output?
            .lines()
            .filter(|l| l.contains(" -> "))
            .filter_map(|l| l.split_whitespace().next().map(str::to_owned))
            .collect(),
    )
}

pub const PENDING_ARGV: [&str; 2] = ["pacman", "-Qu"];

/// "2 package updates: a, b" / "+N more".
pub fn summary(names: &[String], max: usize) -> String {
    let shown: Vec<&str> = names.iter().take(max).map(String::as_str).collect();
    let mut s = format!("{} package updates: {}", names.len(), shown.join(", "));
    if names.len() > max {
        s.push_str(&format!(", +{} more", names.len() - max));
    }
    s
}

/// A root slot name ([`SLOTS`]).
pub fn is_valid_be(name: &str) -> bool {
    SLOTS.contains(&name)
}

/// Arch Linux Archive date `YYYY/MM/DD`.
pub fn is_snapshot_date(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() == 10
        && b[4] == b'/'
        && b[7] == b'/'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// Running slot from `/proc/cmdline` (`rootflags=subvol=@wolos-a`).
pub fn running_slot(cmdline: &str) -> Option<&'static str> {
    let sub = cmdline.split_whitespace().find_map(|w| {
        w.strip_prefix("rootflags=")?
            .split(',')
            .find_map(|o| o.strip_prefix("subvol="))
    })?;
    let sub = sub.trim_start_matches('/').trim_start_matches('@');
    SLOTS.into_iter().find(|s| *s == sub)
}

/// The slot that is not `running`.
pub fn other_slot(running: &str) -> Option<&'static str> {
    is_valid_be(running).then(|| SLOTS.into_iter().find(|s| *s != running))?
}

/// systemd-boot entry for `slot` (written as `<slot>+3.conf`: boot counting).
pub fn entry(slot: &str, release: &str) -> String {
    format!(
        "title WolOS\nversion {release}\nlinux /{slot}/{KERNEL}\ninitrd /{slot}/{INITRAMFS}\n\
options root=LABEL=wolos rootflags=subvol=@{slot} rw quiet splash\n"
    )
}

/// Entry file name with the boot counter.
pub fn entry_file(slot: &str) -> String {
    format!("{ESP}/loader/entries/{slot}+{TRIES}.conf")
}

/// One update step: a fixed argv (never a shell) or a whole-file write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Run(Vec<String>),
    Write { path: String, text: String },
}

impl std::fmt::Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Step::Run(a) => f.write_str(&a.join(" ")),
            Step::Write { path, .. } => write!(f, "write {path}"),
        }
    }
}

/// Steps to update into `slot` (the inactive one). `replace`: the slot holds the previous
/// restore point, deleted first (slot count stays two). `date`: the new release's snapshot
/// (None = same release, `[wolos]` fixes only). The default entry always boots linux-wolos;
/// kernels the person added with pacman stay installed.
pub fn plan(slot: &str, replace: bool, date: Option<&str>) -> Option<Vec<Step>> {
    if !is_valid_be(slot) || date.is_some_and(|d| !is_snapshot_date(d)) {
        return None;
    }
    let v = |a: &[&str]| Step::Run(a.iter().map(|s| (*s).to_owned()).collect());
    let root = format!("{TOP}/@{slot}");
    let mut steps = Vec::new();
    if replace {
        steps.push(v(&["btrfs", "subvolume", "delete", &root]));
    }
    steps.push(v(&["btrfs", "subvolume", "snapshot", "/", &root]));
    if let Some(d) = date {
        steps.push(v(&[
            "sed",
            "-i",
            "-E",
            &format!("s#/repos/[0-9]{{4}}/[0-9]{{2}}/[0-9]{{2}}/#/repos/{d}/#"),
            &format!("{root}{MIRRORLIST}"),
        ]));
    }
    steps.push(v(&["arch-chroot", &root, "pacman", "-Syu", "--noconfirm"]));
    steps.push(v(&["arch-chroot", &root, "mkinitcpio", "-P"]));
    let esp = format!("{ESP}/{slot}");
    steps.push(v(&["mkdir", "-p", &esp]));
    steps.push(v(&[
        "cp",
        &format!("{root}/boot/{KERNEL}"),
        &format!("{root}/boot/{INITRAMFS}"),
        &esp,
    ]));
    steps.push(Step::Write {
        path: entry_file(slot),
        text: entry(slot, date.unwrap_or("current")),
    });
    steps.push(v(&["bootctl", "set-default", &format!("{slot}.conf")]));
    Some(steps)
}

/// After a failed step: delete the half-made slot and its entry.
pub fn rollback(slot: &str) -> Vec<Vec<String>> {
    vec![
        vec![
            "btrfs".into(),
            "subvolume".into(),
            "delete".into(),
            format!("{TOP}/@{slot}"),
        ],
        vec!["rm".into(), "-f".into(), entry_file(slot)],
    ]
}

/// Restore points = the two root slots, as systemd-boot lists them.
pub mod be {
    use serde::{Deserialize, Serialize};

    /// `bootctl list --json=short`: every boot entry, the selected (running) and default one.
    pub const LIST_ARGV: [&str; 3] = ["bootctl", "list", "--json=short"];
    /// Slots kept: the running one and the previous one.
    pub const KEEP: usize = 2;

    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct BootEnv {
        pub name: String,
        /// Running now.
        pub active: bool,
        /// Boots next.
        pub next_boot: bool,
        pub mountpoint: Option<String>,
        /// Boot tries left (empty when the slot booted fine).
        pub space: String,
        /// Release snapshot date of the slot (entry `version`).
        pub created: String,
    }

    /// WolOS slots from `bootctl list --json=short`.
    pub fn parse(text: &str) -> Vec<BootEnv> {
        let Ok(serde_json::Value::Array(entries)) = serde_json::from_str(text) else {
            return Vec::new();
        };
        entries
            .iter()
            .filter_map(|e| {
                let id = e["id"].as_str()?;
                let name = id.trim_end_matches(".conf");
                let name = name.split_once('+').map_or(name, |(n, _)| n);
                if !super::is_valid_be(name) {
                    return None;
                }
                let active = e["isSelected"].as_bool().unwrap_or(false);
                Some(BootEnv {
                    name: name.into(),
                    active,
                    next_boot: e["isDefault"].as_bool().unwrap_or(false),
                    mountpoint: active.then(|| "/".into()),
                    space: e["triesLeft"]
                        .as_u64()
                        .map_or_else(String::new, |t| format!("{t} tries left")),
                    created: e["version"].as_str().unwrap_or("").into(),
                })
            })
            .collect()
    }

    /// Slot to go back to: the running one while an update waits for its first boot,
    /// otherwise the other slot.
    pub fn rollback_target(bes: &[BootEnv]) -> Option<&BootEnv> {
        let running = bes.iter().find(|b| b.active);
        if bes.iter().any(|b| b.next_boot && !b.active) {
            return running;
        }
        bes.iter().find(|b| !b.active)
    }

    /// Two slots only: nothing to prune.
    pub fn prune(_bes: &[BootEnv], _keep: usize) -> Vec<String> {
        Vec::new()
    }

    pub fn activate_argv(name: &str) -> Vec<String> {
        vec![
            "bootctl".into(),
            "set-default".into(),
            format!("{name}.conf"),
        ]
    }

    pub fn destroy_argv(name: &str) -> Vec<String> {
        vec![
            "btrfs".into(),
            "subvolume".into(),
            "delete".into(),
            format!("{}/@{name}", super::TOP),
        ]
    }

    pub fn read(host: &dyn crate::system::Host) -> Option<Vec<BootEnv>> {
        host.run(&LIST_ARGV).map(|t| parse(&t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_table() {
        let cases = [
            (None, None),
            (Some(""), Some("")),
            (
                Some("wol-shell 0.12.1-1 -> 0.12.2-1\nwolos-keyring 1-1 -> 2-1\n"),
                Some("wol-shell,wolos-keyring"),
            ),
            (Some("error: no database\n"), Some("")),
        ];
        for (out, want) in cases {
            assert_eq!(parse_pending(out).map(|l| l.join(",")).as_deref(), want);
        }
        assert_eq!(PENDING_ARGV, ["pacman", "-Qu"]);
    }

    #[test]
    fn summary_table() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(summary(&n(&["a", "b"]), 5), "2 package updates: a, b");
        assert_eq!(
            summary(&n(&["a", "b", "c", "d", "e", "f", "g"]), 5),
            "7 package updates: a, b, c, d, e, +2 more"
        );
    }

    #[test]
    fn slots_from_the_kernel_command_line() {
        let c =
            "initrd=/wolos-a/initramfs root=LABEL=wolos rootflags=subvol=@wolos-a,compress=zstd rw";
        assert_eq!(running_slot(c), Some("wolos-a"));
        assert_eq!(running_slot("rootflags=subvol=/@wolos-b"), Some("wolos-b"));
        assert_eq!(running_slot("rootflags=subvol=@root"), None);
        assert_eq!(running_slot(""), None);
        assert_eq!(other_slot("wolos-a"), Some("wolos-b"));
        assert_eq!(other_slot("wolos-b"), Some("wolos-a"));
        assert_eq!(other_slot("x"), None);
        for (d, ok) in [
            ("2026/10/20", true),
            ("2026-10-20", false),
            ("2026/10/2", false),
            ("../10/2026", false),
        ] {
            assert_eq!(is_snapshot_date(d), ok, "{d}");
        }
    }

    #[test]
    fn update_snapshots_the_running_root_into_the_other_slot() {
        let p = plan("wolos-b", true, Some("2026/11/02")).unwrap();
        let s: Vec<String> = p.iter().map(Step::to_string).collect();
        assert_eq!(
            s,
            [
                "btrfs subvolume delete /run/wolos/top/@wolos-b",
                "btrfs subvolume snapshot / /run/wolos/top/@wolos-b",
                "sed -i -E s#/repos/[0-9]{4}/[0-9]{2}/[0-9]{2}/#/repos/2026/11/02/# \
/run/wolos/top/@wolos-b/etc/pacman.d/mirrorlist",
                "arch-chroot /run/wolos/top/@wolos-b pacman -Syu --noconfirm",
                "arch-chroot /run/wolos/top/@wolos-b mkinitcpio -P",
                "mkdir -p /efi/wolos-b",
                "cp /run/wolos/top/@wolos-b/boot/vmlinuz-linux-wolos \
/run/wolos/top/@wolos-b/boot/initramfs-linux-wolos.img /efi/wolos-b",
                "write /efi/loader/entries/wolos-b+3.conf",
                "bootctl set-default wolos-b.conf",
            ]
        );
        // First update: no previous slot; same release: no new date.
        let first = plan("wolos-a", false, None).unwrap();
        assert_eq!(
            first[0].to_string(),
            "btrfs subvolume snapshot / /run/wolos/top/@wolos-a"
        );
        assert!(!first.iter().any(|s| s.to_string().starts_with("sed ")));
        assert!(
            matches!(&p[7], Step::Write { text, .. } if *text == entry("wolos-b", "2026/11/02"))
        );
        for (slot, date) in [("x", None), ("wolos-a", Some("2026-11-02")), ("", None)] {
            assert!(plan(slot, false, date).is_none(), "{slot} {date:?}");
        }
        let r: Vec<String> = rollback("wolos-b").iter().map(|a| a.join(" ")).collect();
        assert_eq!(
            r,
            [
                "btrfs subvolume delete /run/wolos/top/@wolos-b",
                "rm -f /efi/loader/entries/wolos-b+3.conf"
            ]
        );
    }

    #[test]
    fn boot_entry_counts_tries() {
        let e = entry("wolos-b", "2026/11/02");
        assert!(e.contains("linux /wolos-b/vmlinuz-linux-wolos\n"));
        assert!(e.contains("rootflags=subvol=@wolos-b "));
        assert!(e.contains("version 2026/11/02\n"));
        assert_eq!(entry_file("wolos-b"), "/efi/loader/entries/wolos-b+3.conf");
    }

    pub(crate) const BOOTCTL: &str = r#"[
 {"id":"wolos-a.conf","version":"2026/10/20","isDefault":false,"isSelected":true},
 {"id":"wolos-b+2-1.conf","version":"2026/11/02","isDefault":true,"isSelected":false,"triesLeft":2},
 {"id":"auto-efi-shell","isDefault":false,"isSelected":false}
]"#;

    #[test]
    fn slots_from_bootctl() {
        let b = be::parse(BOOTCTL);
        assert_eq!(b.len(), 2, "only WolOS slots");
        assert_eq!(
            (
                b[0].name.as_str(),
                b[0].active,
                b[0].next_boot,
                b[0].mountpoint.as_deref()
            ),
            ("wolos-a", true, false, Some("/"))
        );
        assert_eq!(
            (
                b[1].name.as_str(),
                b[1].next_boot,
                b[1].space.as_str(),
                b[1].created.as_str()
            ),
            ("wolos-b", true, "2 tries left", "2026/11/02")
        );
        assert!(be::parse("not json").is_empty());
        // Update applied, not rebooted yet: rollback = keep booting the running slot.
        assert_eq!(be::rollback_target(&b).unwrap().name, "wolos-a");
        let booted = be::parse(&BOOTCTL.replace("\"isDefault\":true", "\"isDefault\":false"));
        assert_eq!(be::rollback_target(&booted).unwrap().name, "wolos-b");
        assert!(be::prune(&b, be::KEEP).is_empty());
        assert_eq!(
            be::activate_argv("wolos-a").join(" "),
            "bootctl set-default wolos-a.conf"
        );
        assert_eq!(
            be::destroy_argv("wolos-b").join(" "),
            "btrfs subvolume delete /run/wolos/top/@wolos-b"
        );
    }

    #[test]
    fn read_from_host() {
        let h = crate::system::fake::FakeHost::new(false);
        assert_eq!(be::read(&h), None, "no bootctl");
        h.command("bootctl list --json=short", BOOTCTL);
        assert_eq!(be::read(&h).unwrap().len(), 2);
    }
}
