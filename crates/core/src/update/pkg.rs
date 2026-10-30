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

pub fn classify(args: &[String]) -> Kind {
    let mut op = None;
    let mut shorts = String::new();
    let mut longs: Vec<&str> = Vec::new();
    let mut targets = Vec::new();
    let mut skip_value = false;
    for a in args {
        if std::mem::take(&mut skip_value) {
            continue;
        }
        if let Some(l) = a.strip_prefix("--") {
            let name = l.split('=').next().unwrap_or(l);
            if ELSEWHERE.contains(&a.split('=').next().unwrap_or(a)) {
                return Kind::Pass;
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
                return Kind::Pass;
            }
            for c in s.chars() {
                if "SRUDQFTVh".contains(c) {
                    op = Some(c);
                } else {
                    shorts.push(c);
                }
            }
        } else {
            targets.push(a.clone());
        }
    }
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
            (
                if upgrade && targets.is_empty() {
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
    let pacman: Vec<&str> = args.iter().map(String::as_str).collect();
    super::build(&Build {
        slot,
        from,
        prep: &|_| Vec::new(),
        pacman: &pacman,
        release,
        cmdline,
    })
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
        (Verb::Upgrade, _) | (_, true) => format!("Updated the system.{tail}"),
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
        let r = "/run/wolos/top/@wolos-b";
        assert_eq!(t[0], format!("btrfs subvolume snapshot / {r}"));
        assert_eq!(t[1], format!("wol layers fold {r}"));
        assert_eq!(t[2], format!("arch-chroot {r} /usr/bin/pacman -S vim"));
        assert_eq!(t[3], format!("arch-chroot {r} mkinitcpio -P"));
        let seal = t
            .iter()
            .position(|x| x.starts_with("wol seal apply"))
            .unwrap();
        let entry = t
            .iter()
            .position(|x| x == "write /efi/loader/entries/wolos-b+3.conf")
            .unwrap();
        assert!(seal < entry);
        assert_eq!(t.last().unwrap(), "bootctl set-default wolos-b.conf");
        assert!(!t.iter().any(|x| x.contains("delete")));
    }

    #[test]
    fn stacked_change_is_built_aside_and_swapped_in_last() {
        let s = plan("wolos-b", Base::Pending, &a("-S git"), "current", None).unwrap();
        let t: Vec<String> = s.iter().map(|x| x.to_string()).collect();
        let (r, w) = ("/run/wolos/top/@wolos-b", "/run/wolos/top/@wolos-tx");
        assert_eq!(t[0], format!("btrfs subvolume snapshot {r} {w}"));
        assert_eq!(t[1], format!("wol layers fold {w}"));
        assert_eq!(t[2], format!("arch-chroot {w} /usr/bin/pacman -S git"));
        let del = t
            .iter()
            .position(|x| x == &format!("btrfs subvolume delete {r}"))
            .unwrap();
        let seal = t
            .iter()
            .position(|x| x.starts_with("wol seal apply"))
            .unwrap();
        assert!(
            seal < del,
            "the first change is dropped only once the second is sealed"
        );
        assert_eq!(t[del + 1], format!("mv {w} {r}"));
        assert_eq!(
            super::super::rollback_from("wolos-b", Base::Pending),
            vec![a(&format!("btrfs subvolume delete {w}"))]
        );
        assert_eq!(
            super::super::rollback_from("wolos-b", Base::Running { replace: true }),
            super::super::rollback("wolos-b")
        );
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
