// SPDX-License-Identifier: BSD-2-Clause
//! Transactional pacman. The running slot is sealed read-only: a change goes into a live layer
//! ([`super::layers`]); only kernel, initramfs and init packages are made in a copy of the system: [`classify`] decides whether an invocation changes anything,
//! [`plan`] builds the steps through the same [`super::build`] the system update uses, and a
//! second change before the restart stacks onto the slot the first one made.
//! Pure: argv lists. `wol pkg` and `/usr/local/bin/pacman` run them.
use super::{Base, Build, Step, be::BootEnv, running_slot};

/// What one `pacman` invocation asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Queries, searches, `-p` previews, `-Sy`, cache cleaning, other roots: the real pacman on the live root.
    Pass,
    /// Installs, removes, upgrades: made in a copy of the system.
    Change(Change),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// Package names or files given.
    pub targets: Vec<String>,
    pub verb: Verb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Install,
    Remove,
    Upgrade,
}

/// Flags that point pacman somewhere else than the running system.
const ELSEWHERE: [&str; 5] = ["--root", "--sysroot", "--dbpath", "--config", "--gpgdir"];

/// One pacman command line, read the way pacman reads it.
struct Scan<'a> {
    op: Option<char>,
    shorts: String,
    longs: Vec<&'a str>,
    /// Indexes of the targets in the arguments.
    targets: Vec<usize>,
}

/// None: the command points pacman somewhere else ([`ELSEWHERE`], `-r`, `-b`).
fn scan(args: &[String]) -> Option<Scan<'_>> {
    let mut op = None;
    let mut shorts = String::new();
    let mut longs: Vec<&str> = Vec::new();
    let mut targets = Vec::new();
    let mut skip_value = false;
    for (i, a) in args.iter().enumerate() {
        if std::mem::take(&mut skip_value) {
            continue;
        }
        if let Some(l) = a.strip_prefix("--") {
            let name = l.split('=').next().unwrap_or(l);
            if ELSEWHERE.contains(&a.split('=').next().unwrap_or(a)) {
                return None;
            }
            if matches!(
                name,
                "cachedir" | "logfile" | "hookdir" | "ignore" | "ignoregroup" | "assume-installed"
            ) && !l.contains('=')
            {
                skip_value = true;
            }
            match name {
                "sync" => op = Some('S'),
                "remove" => op = Some('R'),
                "upgrade" => op = Some('U'),
                "database" => op = Some('D'),
                "query" => op = Some('Q'),
                "files" => op = Some('F'),
                "deptest" => op = Some('T'),
                "version" => op = Some('V'),
                "help" => op = Some('h'),
                _ => longs.push(name),
            }
        } else if let Some(s) = a.strip_prefix('-').filter(|s| !s.is_empty()) {
            if s.contains(['r', 'b']) {
                return None;
            }
            for c in s.chars() {
                if "SRUDQFTVh".contains(c) {
                    op = Some(c);
                } else {
                    shorts.push(c);
                }
            }
        } else {
            targets.push(i);
        }
    }
    Some(Scan {
        op,
        shorts,
        longs,
        targets,
    })
}

/// `pacman -Sw ...` (download only, no preview) aimed at the running system: it changes no
/// package, but pacman takes its database lock for it, and on a sealed system the database is on
/// the read-only slot (QA-STORE-72). The caller runs it against a temporary copy of the database.
/// False for a command that names its own database or root.
pub fn needs_scratch_db(args: &[String]) -> bool {
    let Some(sc) = scan(args) else {
        return false;
    };
    let has = |c: char, l: &str| sc.shorts.contains(c) || sc.longs.contains(&l);
    sc.op == Some('S')
        && has('w', "downloadonly")
        && !has('p', "print")
        && !has('p', "print-format")
        && !has('s', "search")
        && !has('i', "info")
        && !has('l', "list")
        && !has('g', "groups")
        && !has('c', "clean")
}

pub fn classify(args: &[String]) -> Kind {
    let Some(Scan {
        op,
        shorts,
        longs,
        targets,
    }) = scan(args)
    else {
        return Kind::Pass;
    };
    let targets: Vec<String> = targets.into_iter().map(|i| args[i].clone()).collect();
    let has = |c: char, l: &str| shorts.contains(c) || longs.contains(&l);
    let preview = has('p', "print") || has('p', "print-format");
    let (verb, change) = match op {
        Some('S') => {
            let read = has('s', "search")
                || has('i', "info")
                || has('l', "list")
                || has('g', "groups")
                || has('c', "clean")
                || has('w', "downloadonly");
            let upgrade = has('u', "sysupgrade");
            // `-Syu vim` (Arch's install idiom) is a full upgrade too: it goes to the slot.
            (
                if upgrade {
                    Verb::Upgrade
                } else {
                    Verb::Install
                },
                !read && (upgrade || !targets.is_empty()),
            )
        }
        Some('R') => (Verb::Remove, !targets.is_empty()),
        Some('U') => (Verb::Install, !targets.is_empty()),
        Some('D') => (Verb::Install, !has('k', "check") && !targets.is_empty()),
        _ => (Verb::Install, false),
    };
    if change && !preview {
        Kind::Change(Change { targets, verb })
    } else {
        Kind::Pass
    }
}

/// Where the change is made.
pub fn base(bes: &[BootEnv], running: &str) -> Option<(String, Base)> {
    let other = super::other_slot(running)?;
    let there = bes.iter().find(|b| b.name == other);
    Some(match there {
        Some(b) if b.next_boot && !b.active => (other.into(), Base::Pending),
        Some(_) => (other.into(), Base::Running { replace: true }),
        None => (other.into(), Base::Running { replace: false }),
    })
}

/// Steps for `args` (already a [`Kind::Change`]) in the slot chosen by [`base`].
pub fn plan(
    slot: &str,
    from: Base,
    args: &[String],
    release: &str,
    cmdline: Option<&str>,
) -> Option<Vec<Step>> {
    let (pacman, files) = stage(args);
    let pacman: Vec<&str> = pacman.iter().map(String::as_str).collect();
    super::build(&Build {
        slot,
        from,
        prep: &|_| Vec::new(),
        after: &|_| Vec::new(),
        pacman: &pacman,
        files: &files,
        release,
        cmdline,
    })
}

/// A change made in the other root slot, with the restore point it leaves: the running slot,
/// which no step touches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotChange {
    /// The running slot: what the person goes back to (Settings > Restore, `wol restore`).
    pub restore_point: &'static str,
    /// Where the change is made and booted next.
    pub slot: String,
    pub from: Base,
    pub steps: Vec<Step>,
}

/// The one way into the other slot, shared by the pacman wrapper and Update Now: `cmdline` is
/// `/proc/cmdline`, `bootctl` the output of [`super::be::LIST_ARGV`]. None when this system does
/// not boot a WolOS root slot (one copy, ADR 0023), so there is no restore point to keep.
pub fn slot_change(
    cmdline: Option<&str>,
    bootctl: Option<&str>,
    args: &[String],
) -> Option<SlotChange> {
    let restore_point = cmdline.and_then(running_slot)?;
    let bes = super::be::parse(bootctl.unwrap_or_default());
    let (slot, from) = base(&bes, restore_point)?;
    let steps = plan(&slot, from, args, "current", cmdline)?;
    Some(SlotChange {
        restore_point,
        slot,
        from,
        steps,
    })
}

/// pacman's arguments for Update Now inside the pinned release (only `[wolos]` moves); `holds`
/// (validated package names) stay at their version.
pub fn update_now_args(holds: &[String]) -> Vec<String> {
    let mut a = vec!["-Syu".to_owned(), "--noconfirm".to_owned()];
    if !holds.is_empty() {
        a.push(format!("--ignore={}", holds.join(",")));
    }
    a
}

/// Update Now on a computer with one copy of WolOS (ADR 0023, [`super::SINGLE_COPY`]): the
/// running system is updated in place, with nothing to go back to (the page says so first).
pub fn update_now_in_place(holds: &[String]) -> Vec<Step> {
    let mut a = vec![super::PACMAN.to_owned()];
    a.extend(update_now_args(holds));
    vec![Step::Run(a)]
}

/// What wol-admind answers when Update Now could not be made, for the Software Update page
/// (`shared::admin::failed_line`). The refusal: neither a root slot nor a one-copy install.
pub const NO_RESTORE_POINT: &str = concat!(
    "Update Now was not started: WolOS could not find a copy of the system to keep as a ",
    "restore point. Nothing was changed."
);

/// A slot update that stopped before its swap: the half-made copy is gone, the running system
/// and the restore point are as they were. No slot name (QA-STORE-6: people do not know
/// `wolos-a`); the log names it.
pub const SLOT_FAILED: &str =
    "The update did not finish, so nothing was changed. Your restore point is kept.";

/// A slot update whose new slot is complete and has its entry, but whose last step (clearing a
/// default chosen earlier) failed: installed, said so, nothing removed.
pub const INSTALLED_NOT_DEFAULT: &str = concat!(
    "The update is installed and starts at the next restart, unless an earlier choice in ",
    "Settings > Restore keeps the other copy as the default."
);

/// Free space a slot update needs on the system disk before it starts (QA-STORE-11, QA-ACC-23):
/// the copy shares every unchanged block with the running system, so what it takes is the
/// downloaded packages (shared cache), their unpacked files and a new initramfs and boot image.
/// An estimate with headroom (the 0.12.0 -> 0.12.1 upgrade downloaded 860 MiB); not measured
/// per update.
pub const SLOT_ROOM_BYTES: u64 = 4 << 30;

/// Where [`room_needed`] is measured: `df -B1 --output=avail` of the btrfs top level, the one file
/// system the slots, `/var` and `/home` share.
pub const ROOM_ARGV: [&str; 4] = ["df", "-B1", "--output=avail", super::TOP];

/// The sentence for an update that would run out of room, from `free` bytes ([`ROOM_ARGV`]'s
/// number); None when there is room or the free space is unknown (the build's own errors still
/// stop it then). Names how much more is needed, in whole GB rounded up.
pub fn room_needed(free: Option<u64>) -> Option<String> {
    let free = free?;
    let short = SLOT_ROOM_BYTES.checked_sub(free).filter(|s| *s > 0)?;
    let gb = short.div_ceil(1 << 30);
    Some(format!(
        "Update Now was not started: the system disk needs about {gb} GB more free space for \
         the update. Free some space in Settings > Storage, then try again. Nothing was changed."
    ))
}

/// An in-place update (one copy) that stopped: part of it may be installed.
pub const IN_PLACE_FAILED: &str = concat!(
    "The update did not finish. This computer has one copy of WolOS, so some packages may ",
    "already be updated. Check that you are online, then try again."
);

/// The most steps an Update Now may run (a change stacked onto a pending slot), so a client
/// waits as long as wol-admind may take (S4-4).
pub fn update_now_most_steps(holds: &[String]) -> usize {
    let args = update_now_args(holds);
    [Base::Pending, Base::Running { replace: true }]
        .into_iter()
        .filter_map(|from| plan(super::SLOTS[1], from, &args, "current", None))
        .map(|s| s.len())
        .max()
        .unwrap_or(1)
}

/// `pacman -U FILE...`: the files are copied into the copy (pacman runs there in a chroot that
/// has no `/var`, `/home` or `/tmp` of the running system and starts in `/`), so pacman is given
/// their [`super::staged`] paths. Packages named by URL are fetched by pacman itself.
fn stage(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut pacman = args.to_vec();
    let mut files = Vec::new();
    if let Some(s) = scan(args).filter(|s| s.op == Some('U')) {
        for i in s.targets {
            if args[i].contains("://") {
                continue;
            }
            pacman[i] = super::staged(files.len(), &args[i]);
            files.push(args[i].clone());
        }
    }
    (pacman, files)
}

/// The slot is read-only: `btrfs property get -ts / ro`.
pub const RO_ARGV: [&str; 5] = ["btrfs", "property", "get", "-ts", "/"];

/// This is a WolOS slot whose root is sealed (else pacman writes straight to the live root).
pub fn is_sealed(cmdline: Option<&str>, ro_output: Option<&str>) -> bool {
    cmdline.and_then(running_slot).is_some() && ro_output.is_none_or(|o| !o.contains("ro=false"))
}

/// Where a change is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// A layer merged over `/usr` now.
    Layer,
    /// The pending slot: kernel, initramfs or init packages, or a full upgrade.
    Slot,
}

/// `names`: every package the change touches (`pacman -p --print-format %n`), else the targets.
pub fn route(c: &Change, names: &[String]) -> Route {
    if c.verb == Verb::Upgrade || names.iter().any(|n| super::layers::needs_restart(n)) {
        Route::Slot
    } else {
        Route::Layer
    }
}

/// `pacman -p --print-format %n` for the same change.
pub fn names_argv(args: &[String]) -> Vec<String> {
    let mut a = vec![super::PACMAN.to_owned()];
    a.extend(args.iter().cloned());
    a.extend(["-p", "--print-format", "%n"].map(String::from));
    a
}

/// One plain sentence for the person.
pub fn done(c: &Change, route: Route) -> String {
    let names = c.targets.join(", ");
    let tail = match route {
        Route::Layer => "",
        Route::Slot => " It takes effect at the next restart.",
    };
    match (c.verb, c.targets.is_empty()) {
        (Verb::Upgrade, false) => format!("Updated the system and installed {names}.{tail}"),
        (Verb::Upgrade, true) | (_, true) => format!("Updated the system.{tail}"),
        (Verb::Install, _) => format!("Installed {names}.{tail}"),
        (Verb::Remove, _) => format!("Removed {names}.{tail}"),
    }
}

/// `wol seal off`: developer mode. The running root is writable again and its seal record is
/// dropped so the boot check does not read the edits as tampering.
pub fn unseal_steps(slot: &str) -> Vec<Vec<String>> {
    vec![
        ["btrfs", "property", "set", "-ts", "/", "ro", "false"]
            .map(String::from)
            .to_vec(),
        vec!["rm".into(), "-f".into(), crate::seal::digest_file(slot)],
    ]
}

pub const UNSEAL_WARNING: &str = "Developer mode: this system is no longer sealed. Files can change and nothing checks them at boot until the next system update.";

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }
    /// QA-STORE-72: download-only runs need a scratch database on a sealed system; nothing else
    /// does, and a command with its own database or root is left alone.
    #[test]
    fn download_only_needs_a_scratch_database() {
        for yes in [
            "-Sw vim",
            "-Syw vim",
            "-Suw",
            "-Syuw --noconfirm",
            "--sync --downloadonly vim",
        ] {
            assert!(needs_scratch_db(&a(yes)), "{yes}");
        }
        for no in [
            "-S vim",
            "-Syu",
            "-Q",
            "-Sy",
            "-Sc",
            "-Ss vim",
            "-Swp vim",
            "-Sw --dbpath /x vim",
            "-Sw -b /x vim",
            "-Sw --root /mnt vim",
            "",
        ] {
            assert!(!needs_scratch_db(&a(no)), "{no}");
        }
    }

    fn c(verb: Verb, t: &str) -> Kind {
        Kind::Change(Change {
            targets: t.split_whitespace().map(String::from).collect(),
            verb,
        })
    }

    #[test]
    fn which_operations_change_the_system() {
        let cases = [
            ("-S vim", c(Verb::Install, "vim")),
            (
                "-S --noconfirm --needed vim git",
                c(Verb::Install, "vim git"),
            ),
            ("-Syu", c(Verb::Upgrade, "")),
            ("-Syyu --noconfirm", c(Verb::Upgrade, "")),
            ("--sync --refresh --sysupgrade", c(Verb::Upgrade, "")),
            ("-Sy vim", c(Verb::Install, "vim")),
            ("-Syu vim", c(Verb::Upgrade, "vim")),
            ("-Syu --needed vim git", c(Verb::Upgrade, "vim git")),
            ("-R vim", c(Verb::Remove, "vim")),
            ("-Rns vim", c(Verb::Remove, "vim")),
            (
                "-U /tmp/x.pkg.tar.zst",
                c(Verb::Install, "/tmp/x.pkg.tar.zst"),
            ),
            ("-D --asdeps vim", c(Verb::Install, "vim")),
            ("-Q", Kind::Pass),
            ("-Qi vim", Kind::Pass),
            ("-Ss vim", Kind::Pass),
            ("-Si vim", Kind::Pass),
            ("-Sl", Kind::Pass),
            ("-F bin/ls", Kind::Pass),
            ("-Fy", Kind::Pass),
            ("-Sy", Kind::Pass),
            ("-Sc", Kind::Pass),
            ("-Sw vim", Kind::Pass),
            ("-Sp vim", Kind::Pass),
            ("-Rp vim", Kind::Pass),
            ("-S --print vim", Kind::Pass),
            ("-V", Kind::Pass),
            ("-h", Kind::Pass),
            ("-T bash", Kind::Pass),
            ("-Dk", Kind::Pass),
            ("", Kind::Pass),
            ("-S --root /mnt base", Kind::Pass),
            ("-r /mnt -S base", Kind::Pass),
            ("-S --sysroot=/mnt vim", Kind::Pass),
            ("--cachedir /x -S vim", c(Verb::Install, "vim")),
        ];
        for (line, want) in cases {
            assert_eq!(classify(&a(line)), want, "{line}");
        }
    }

    /// FIX-slot 0.12.1: `pacman -U FILE` failed in the copy ("could not determine root mount
    /// point", and the file is not there: no `/var`, `/home` or `/tmp` of the running system).
    #[test]
    fn local_package_files_are_copied_into_the_copy_and_named_there() {
        let args = a("-U --noconfirm /var/cache/k.pkg.tar.zst ./h.pkg.tar.zst \
https://example.org/x.pkg.tar.zst --cachedir /c");
        let s = plan(
            "wolos-b",
            Base::Running { replace: true },
            &args,
            "current",
            None,
        )
        .unwrap();
        let t: Vec<String> = s.iter().map(|x| x.to_string()).collect();
        let r = "/run/wolos/top/@wolos-tx";
        let st = format!("{r}/var/tmp/wolos-pkg");
        let at = |want: &str| {
            t.iter()
                .position(|x| x == want)
                .unwrap_or_else(|| panic!("{want}: {t:?}"))
        };
        let mkdir = at(&format!("mkdir -p {st}/0 {st}/1"));
        let cp0 = at(&format!(
            "cp --reflink=auto -t {st}/0 -- /var/cache/k.pkg.tar.zst"
        ));
        let sig0 = at(&format!(
            "find -L /var/cache -maxdepth 1 -type f -name k.pkg.tar.zst.sig -exec cp \
--reflink=auto -t {st}/0 -- {{}} +"
        ));
        let cp1 = at(&format!("cp --reflink=auto -t {st}/1 -- ./h.pkg.tar.zst"));
        let sig1 = at(&format!(
            "find -L . -maxdepth 1 -type f -name h.pkg.tar.zst.sig -exec cp --reflink=auto \
-t {st}/1 -- {{}} +"
        ));
        let pacman = at(&format!(
            "unshare --mount --propagation private wol update chroot {r} -- /usr/bin/pacman -U \
--noconfirm /var/tmp/wolos-pkg/0/k.pkg.tar.zst /var/tmp/wolos-pkg/1/h.pkg.tar.zst \
https://example.org/x.pkg.tar.zst --cachedir /c"
        ));
        let rm = at(&format!("rm -rf -- {st}"));
        let seal = at(&format!("wol seal apply {r} /efi/wolos-b/verity.new"));
        assert!(mkdir < cp0 && cp0 < sig0 && sig0 < cp1 && cp1 < sig1 && sig1 < pacman);
        assert!(pacman < rm && rm < seal, "{t:?}");
        // Repo installs and upgrades copy nothing.
        for line in ["-S vim", "-Syu", "-R linux-lts", "-S --needed ./not-a-file"] {
            let s = plan("wolos-b", Base::Pending, &a(line), "current", None).unwrap();
            assert!(
                !s.iter().any(|x| x.to_string().contains("wolos-pkg")),
                "{line}: {s:?}"
            );
        }
        assert_eq!(stage(&a("--upgrade x.zst")).1, ["x.zst"]);
        assert_eq!(stage(&a("-Udd x.zst y.zst")).1, ["x.zst", "y.zst"]);
    }

    fn be(name: &str, active: bool, next: bool) -> BootEnv {
        BootEnv {
            name: name.into(),
            active,
            next_boot: next,
            ..Default::default()
        }
    }

    #[test]
    fn a_second_change_stacks_onto_the_pending_slot() {
        let idle = [be("wolos-a", true, true)];
        assert_eq!(
            base(&idle, "wolos-a"),
            Some(("wolos-b".into(), Base::Running { replace: false }))
        );
        let old = [be("wolos-a", true, true), be("wolos-b", false, false)];
        assert_eq!(
            base(&old, "wolos-a"),
            Some(("wolos-b".into(), Base::Running { replace: true }))
        );
        let pending = [be("wolos-a", true, false), be("wolos-b", false, true)];
        assert_eq!(
            base(&pending, "wolos-a"),
            Some(("wolos-b".into(), Base::Pending))
        );
        assert_eq!(base(&pending, "nope"), None);
    }

    #[test]
    fn first_change_copies_the_running_root_and_seals_the_result() {
        let s = plan(
            "wolos-b",
            Base::Running { replace: false },
            &a("-S vim"),
            "current",
            None,
        )
        .unwrap();
        let t: Vec<String> = s.iter().map(|x| x.to_string()).collect();
        let (w, r) = ("/run/wolos/top/@wolos-tx", "/run/wolos/top/@wolos-b");
        let at = |want: &str| {
            t.iter()
                .position(|x| x == want)
                .unwrap_or_else(|| panic!("{want}: {t:?}"))
        };
        let snap = at(&format!("btrfs subvolume snapshot / {w}"));
        assert_eq!(t[snap + 1], format!("wol layers fold {w}"));
        assert!(
            t[snap + 2].ends_with(&format!(" {w}/etc/fstab")),
            "{}",
            t[snap + 2]
        );
        assert_eq!(t[snap + 3], format!("write {w}/etc/wolos/slot"));
        assert_eq!(t[snap + 4], super::super::ETC_SETTLE);
        let pacman = at(&format!(
            "unshare --mount --propagation private wol update chroot {w} -- /usr/bin/pacman -S vim"
        ));
        assert_eq!(
            t[pacman + 1],
            format!("unshare --mount --propagation private wol update chroot {w} -- mkinitcpio -P")
        );
        let seal = at(&format!("wol seal apply {w} /efi/wolos-b/verity.new"));
        let mv = at(&format!("mv {w} {r}"));
        let entry = t
            .iter()
            .position(|x| x.ends_with("esp-sync --new"))
            .unwrap();
        assert!(pacman < seal && seal < mv && mv < entry, "{t:?}");
        assert_eq!(t[t.len() - 2], "bootctl set-default ");
        assert_eq!(
            t[t.len() - 1],
            "sync",
            "on the disk before \"updated\" (QA-LIVE4-7)"
        );
    }

    #[test]
    fn stacked_change_is_built_aside_and_swapped_in_last() {
        let s = plan("wolos-b", Base::Pending, &a("-S git"), "current", None).unwrap();
        let t: Vec<String> = s.iter().map(|x| x.to_string()).collect();
        let (r, w) = ("/run/wolos/top/@wolos-b", "/run/wolos/top/@wolos-tx");
        let at = |want: &str| {
            t.iter()
                .position(|x| x == want)
                .unwrap_or_else(|| panic!("{want}: {t:?}"))
        };
        let snap = at(&format!("btrfs subvolume snapshot {r} {w}"));
        assert_eq!(t[snap + 1], format!("wol layers fold {w}"));
        at(&format!(
            "unshare --mount --propagation private wol update chroot {w} -- /usr/bin/pacman -S git"
        ));
        let del =
            at("find /run/wolos/top -maxdepth 1 -name @wolos-b -exec btrfs subvolume delete {} +");
        let seal = t
            .iter()
            .position(|x| x.starts_with("wol seal apply"))
            .unwrap();
        assert!(
            seal < del,
            "the first change is dropped only once the second is sealed"
        );
        assert_eq!(t[del + 1], format!("mv {w} {r}"));
        // Undo before the swap: the scratch copy, its digest and the mark, nothing of the slot.
        let undo: Vec<String> = super::super::rollback_from("wolos-b", Base::Pending)
            .iter()
            .map(|x| x.join(" "))
            .collect();
        assert_eq!(
            undo,
            [
                "find /run/wolos/top -maxdepth 1 -name @wolos-tx -exec umount -R {} ;".to_string(),
                "find /run/wolos/top -maxdepth 1 -name @wolos-tx -exec btrfs subvolume delete {} +"
                    .to_string(),
                "rm -f /efi/wolos-b/verity.new /var/lib/wolos/update-building".into(),
            ]
        );
        assert_eq!(
            super::super::rollback_from("wolos-b", Base::Running { replace: true }),
            super::super::rollback_from("wolos-b", Base::Pending)
        );
    }

    /// Update Now and the pacman wrapper share one way into the other slot: the running slot is
    /// the restore point and no step touches it; without a slot there is no restore point.
    #[test]
    fn a_slot_change_keeps_the_running_slot_as_the_restore_point() {
        let cmd = "root=LABEL=wolos rootflags=subvol=@wolos-a rw";
        let list = r#"[{"id":"wolos-a.conf","isSelected":true,"isDefault":true},{"id":"wolos-b.conf","isSelected":false,"isDefault":false}]"#;
        let args = update_now_args(&[]);
        assert_eq!(args, ["-Syu", "--noconfirm"]);
        let c = slot_change(Some(cmd), Some(list), &args).unwrap();
        assert_eq!(c.restore_point, "wolos-a");
        assert_eq!(c.slot, "wolos-b");
        assert_eq!(c.from, Base::Running { replace: true });
        assert_eq!(
            c.steps,
            plan("wolos-b", c.from, &args, "current", Some(cmd)).unwrap()
        );
        let t: Vec<String> = c.steps.iter().map(ToString::to_string).collect();
        // QA-STORE-3: the old restore point in wolos-b is deleted only after the new copy is
        // sealed; the copy is built aside.
        let snap = t
            .iter()
            .position(|x| x == "btrfs subvolume snapshot / /run/wolos/top/@wolos-tx")
            .unwrap();
        let del = t
            .iter()
            .position(|x| x.contains("-name @wolos-b -exec btrfs subvolume delete"))
            .unwrap();
        let seal = t
            .iter()
            .position(|x| x.starts_with("wol seal apply"))
            .unwrap();
        assert!(snap < seal && seal < del, "{t:?}");
        assert!(t.contains(
            &"unshare --mount --propagation private wol update chroot /run/wolos/top/@wolos-tx -- /usr/bin/pacman -Syu --noconfirm".into()
        ));
        assert!(
            t.iter().all(|x| !x.contains("@wolos-a")),
            "the restore point is never written: {t:?}"
        );
        // No WolOS slot on the command line (one copy, ADR 0023): no restore point, no plan.
        assert_eq!(
            slot_change(Some("root=/dev/sda2 rw"), Some(list), &args),
            None
        );
        assert_eq!(slot_change(None, Some(list), &args), None);
        // Holds stay at their version.
        assert_eq!(
            update_now_args(&["firefox".into(), "gimp".into()]),
            ["-Syu", "--noconfirm", "--ignore=firefox,gimp"]
        );
        assert!(update_now_most_steps(&[]) >= c.steps.len());
        // One copy: the running system, in place, holds kept.
        let one: Vec<String> = update_now_in_place(&["gimp".into()])
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(one, ["/usr/bin/pacman -Syu --noconfirm --ignore=gimp"]);
        assert!(
            !SLOT_FAILED.contains("wolos-"),
            "no slot name for the person (QA-STORE-6)"
        );
    }

    /// audit8 stability2 #4: a stacked change that fails before its first change is deleted keeps
    /// the slot; one that fails at the `mv` or later (boot image, entry) removes it, scratch too.
    #[test]
    fn a_stacked_change_failing_after_the_swap_removes_the_slot() {
        use super::super::{Undo, rollback, rollback_at};
        for from in [Base::Pending, Base::Running { replace: true }] {
            let s = plan("wolos-b", from, &a("-S git"), "current", None).unwrap();
            let t: Vec<String> = s.iter().map(|x| x.to_string()).collect();
            let (r, w) = ("/run/wolos/top/@wolos-b", "/run/wolos/top/@wolos-tx");
            let swap = t
                .iter()
                .position(|x| x.starts_with("find /efi/loader/entries "))
                .unwrap();
            let mv = t.iter().position(|x| x == &format!("mv {w} {r}")).unwrap();
            let esp = t
                .iter()
                .position(|x| x.ends_with("esp-sync --new"))
                .unwrap();
            assert!(swap < mv && mv < esp, "{t:?}");
            let scratch = super::super::rollback_from("wolos-b", from);
            for kept in [0, 3, swap - 1] {
                assert_eq!(
                    rollback_at("wolos-b", from, &s, kept),
                    Undo {
                        argv: scratch.clone(),
                        kept: true,
                        done: false
                    },
                    "step {kept}: {}",
                    t[kept]
                );
            }
            let mut gone = rollback("wolos-b");
            gone.extend(scratch);
            for lost in [swap, mv, esp] {
                assert_eq!(
                    rollback_at("wolos-b", from, &s, lost),
                    Undo {
                        argv: gone.clone(),
                        kept: false,
                        done: false
                    },
                    "step {lost}: {}",
                    t[lost]
                );
            }
            // After esp-sync the new slot is complete: a failed `set-default` removes nothing.
            for late in [esp + 1, t.len() - 1] {
                let u = rollback_at("wolos-b", from, &s, late);
                assert!(u.done && !u.kept, "step {late}: {}", t[late]);
                assert!(
                    u.argv
                        .iter()
                        .all(|a| a[0] == "rm" && !a.join(" ").contains("@wolos"))
                );
            }
        }
    }

    #[test]
    fn sealed_means_a_wolos_slot_with_a_read_only_root() {
        let c = "root=LABEL=wolos rootflags=subvol=@wolos-a rw";
        assert!(is_sealed(Some(c), Some("ro=true\n")));
        assert!(is_sealed(Some(c), None));
        assert!(!is_sealed(Some(c), Some("ro=false\n")));
        assert!(!is_sealed(Some("root=/dev/sda1"), Some("ro=true")));
        assert!(!is_sealed(None, None));
    }

    #[test]
    fn plain_words_for_the_result() {
        let ch = |v, t: &str| Change {
            targets: t.split_whitespace().map(String::from).collect(),
            verb: v,
        };
        let l = Route::Layer;
        assert_eq!(
            done(&ch(Verb::Install, "vim git"), l),
            "Installed vim, git."
        );
        assert_eq!(done(&ch(Verb::Remove, "vim"), l), "Removed vim.");
        assert_eq!(
            done(&ch(Verb::Install, "linux-wolos"), Route::Slot),
            "Installed linux-wolos. It takes effect at the next restart."
        );
        assert_eq!(
            done(&ch(Verb::Upgrade, ""), Route::Slot),
            "Updated the system. It takes effect at the next restart."
        );
        let n = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        assert_eq!(
            route(&ch(Verb::Install, "vim"), &n("vim libx")),
            Route::Layer
        );
        assert_eq!(
            route(&ch(Verb::Install, "nvidia"), &n("nvidia linux-wolos")),
            Route::Slot
        );
        assert_eq!(route(&ch(Verb::Upgrade, ""), &[]), Route::Slot);
        // CORE-4: `pacman -Syu vim` is a full upgrade, never a layer.
        let Kind::Change(syu) = classify(&a("-Syu vim")) else {
            panic!("-Syu vim changes the system")
        };
        assert_eq!(route(&syu, &n("vim")), Route::Slot);
        assert_eq!(
            done(&syu, Route::Slot),
            "Updated the system and installed vim. It takes effect at the next restart."
        );
        assert_eq!(
            names_argv(&n("-S vim")).join(" "),
            "/usr/bin/pacman -S vim -p --print-format %n"
        );
    }

    #[test]
    fn developer_mode_unseals_and_drops_the_seal_record() {
        let s = unseal_steps("wolos-a");
        assert_eq!(s[0].join(" "), "btrfs property set -ts / ro false");
        assert_eq!(s[1].join(" "), "rm -f /efi/wolos-a/verity");
    }
}
