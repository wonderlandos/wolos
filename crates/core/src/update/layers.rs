//! Live installs: the sealed slot stays read-only, a package change lands in a layer. The real
//! pacman runs in a chroot whose `/usr` is an overlay over the live one (`/etc` and
//! `/var/lib/pacman` are the live, writable ones), the overlay's upper directory is a
//! systemd-sysext extension, and `systemd-sysext refresh` merges it over `/usr` at once.
//! Removing a package that is in the sealed base leaves overlay whiteouts in the layer.
//! The next system update folds the layers into the new slot ([`super::build`]) and clears them.
//! Pure: argv lists, texts and decisions. `wol pkg` and `wol layers fold` run them.
use super::{PACMAN, Step};

/// Extensions systemd-sysext looks in; each layer is a directory here.
pub const DIR: &str = "/var/lib/extensions";
pub const PREFIX: &str = "wolos-layer-";
/// Overlay work directories (same file system as [`DIR`]).
pub const WORK_DIR: &str = "/var/lib/wolos/layers";
/// The chroot the real pacman runs in.
pub const CHROOT: &str = "/run/wolos/layer-root";
/// What a layer holds besides `usr/`: the packages it added and removed.
pub const RECORD: &str = "wolos-layer.json";
/// Layers already copied into a slot (so a later fold does not undo that slot's own upgrades).
pub const FOLDED: &str = "var/lib/wolos/folded";

pub fn name(n: u32) -> String {
    format!("{PREFIX}{n:04}")
}

/// Next layer number from `ls` of [`DIR`]; later layers sort after earlier ones.
pub fn next_index(ls: &str) -> u32 {
    ls.lines()
        .filter_map(|l| l.trim().strip_prefix(PREFIX)?.parse::<u32>().ok())
        .max()
        .map_or(1, |m| m + 1)
}

/// The extension matches any system (`ID=_any`): a layer follows the person to the next slot
/// only by being folded into it, so it never needs to name the release.
pub fn extension_release() -> String {
    "ID=_any\nARCHITECTURE=x86-64\n".into()
}

fn v(a: &[&str]) -> Step {
    Step::Run(a.iter().map(|s| (*s).to_owned()).collect())
}

/// Steps up to and including the real pacman in the layer's chroot.
pub fn begin(n: u32, args: &[String]) -> Vec<Step> {
    let l = format!("{DIR}/{}", name(n));
    let w = format!("{WORK_DIR}/{}", name(n));
    let rel = format!(
        "{l}/usr/lib/extension-release.d/extension-release.{}",
        name(n)
    );
    let opts = format!("lowerdir={CHROOT}/usr,upperdir={l}/usr,workdir={w}");
    let mut chroot = vec!["arch-chroot", CHROOT, PACMAN];
    chroot.extend(args.iter().map(String::as_str));
    vec![
        v(&[
            "mkdir",
            "-p",
            &format!("{l}/usr/lib/extension-release.d"),
            &w,
            CHROOT,
        ]),
        Step::Write {
            path: rel,
            text: extension_release(),
        },
        v(&["mount", "--rbind", "/", CHROOT]),
        v(&["mount", "--make-rprivate", CHROOT]),
        v(&[
            "mount",
            "-t",
            "overlay",
            "overlay",
            "-o",
            &opts,
            &format!("{CHROOT}/usr"),
        ]),
        v(&chroot),
    ]
}

/// Steps after pacman succeeded: unmount, keep the record, merge the layer.
pub fn finish(n: u32, rec: &Record) -> Vec<Step> {
    let l = format!("{DIR}/{}", name(n));
    vec![
        v(&["umount", "-R", CHROOT]),
        v(&["rm", "-rf", &format!("{WORK_DIR}/{}", name(n))]),
        Step::Write {
            path: format!("{l}/{RECORD}"),
            text: rec.to_json(),
        },
        v(&["systemd-sysext", "refresh"]),
    ]
}

/// After a failed step: nothing of the layer is left, the running system is untouched.
pub fn abort(n: u32) -> Vec<Vec<String>> {
    let s = |a: &[&str]| a.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    vec![
        s(&["umount", "-R", CHROOT]),
        s(&["rm", "-rf", &format!("{DIR}/{}", name(n))]),
        s(&["rm", "-rf", &format!("{WORK_DIR}/{}", name(n))]),
    ]
}

/// Packages a layer added and removed, as `name-version` (the directory names in the pacman database).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Record {
    pub installed: Vec<String>,
    pub removed: Vec<String>,
}

impl Record {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default() + "\n"
    }
    pub fn parse(text: &str) -> Record {
        serde_json::from_str(text).unwrap_or_default()
    }
}

/// `pacman -Q` before and after.
pub fn delta(before: &str, after: &str) -> Record {
    let set = |t: &str| -> std::collections::BTreeSet<String> {
        t.lines()
            .filter_map(|l| {
                let mut w = l.split_whitespace();
                Some(format!("{}-{}", w.next()?, w.next()?))
            })
            .collect()
    };
    let (b, a) = (set(before), set(after));
    Record {
        installed: a.difference(&b).cloned().collect(),
        removed: b.difference(&a).cloned().collect(),
    }
}

/// Package name of a database directory `name-version-release`.
pub fn pkg_name(dir: &str) -> &str {
    let mut it = dir.rsplitn(3, '-');
    it.next();
    it.next();
    it.next().unwrap_or(dir)
}

/// Kernel, initramfs and init packages: nothing can swap them in a running system.
pub fn needs_restart(name: &str) -> bool {
    !name.contains("headers")
        && (name.starts_with("linux")
            || name.starts_with("systemd")
            || name.starts_with("mkinitcpio")
            || name.ends_with("-ucode")
            || name == "dracut")
}

/// An overlay whiteout: a character device 0:0.
pub fn is_whiteout(is_char_device: bool, rdev: u64) -> bool {
    is_char_device && rdev == 0
}

/// Layer directories in the order they apply (oldest first).
pub fn ordered(mut names: Vec<String>) -> Vec<String> {
    names.retain(|n| n.starts_with(PREFIX));
    names.sort();
    names
}

/// Database directories to copy into a slot and package names to drop from it.
pub fn db_changes(recs: &[Record]) -> (Vec<String>, Vec<String>) {
    let mut add: Vec<String> = Vec::new();
    let mut drop: Vec<String> = Vec::new();
    for r in recs {
        for d in &r.removed {
            add.retain(|a| pkg_name(a) != pkg_name(d));
            drop.push(pkg_name(d).to_owned());
        }
        for d in &r.installed {
            drop.retain(|x| x != pkg_name(d));
            add.retain(|a| pkg_name(a) != pkg_name(d));
            add.push(d.clone());
        }
    }
    (add, drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_number_after_the_last() {
        assert_eq!(next_index(""), 1);
        assert_eq!(next_index("wolos-layer-0001\nwolos-layer-0007\nother\n"), 8);
        assert_eq!(name(12), "wolos-layer-0012");
    }

    #[test]
    fn pacman_runs_in_an_overlay_chroot_and_the_layer_is_merged_at_once() {
        let args: Vec<String> = ["-S", "vim"].map(String::from).to_vec();
        let t: Vec<String> = begin(3, &args).iter().map(|s| s.to_string()).collect();
        let l = "/var/lib/extensions/wolos-layer-0003";
        assert!(t[0].starts_with(&format!("mkdir -p {l}/usr/lib/extension-release.d")));
        assert!(t[1].contains("extension-release.wolos-layer-0003"));
        assert_eq!(t[2], "mount --rbind / /run/wolos/layer-root");
        assert!(t[4].contains(&format!(
            "lowerdir=/run/wolos/layer-root/usr,upperdir={l}/usr,workdir=/var/lib/wolos/layers/wolos-layer-0003"
        )));
        assert_eq!(
            t.last().unwrap(),
            "arch-chroot /run/wolos/layer-root /usr/bin/pacman -S vim"
        );
        let rec = Record {
            installed: vec!["vim-9.1-1".into()],
            removed: vec![],
        };
        let f: Vec<String> = finish(3, &rec).iter().map(|s| s.to_string()).collect();
        assert_eq!(f[0], "umount -R /run/wolos/layer-root");
        assert_eq!(f.last().unwrap(), "systemd-sysext refresh");
        assert!(
            f.iter()
                .any(|s| s == &format!("write {l}/wolos-layer.json"))
        );
    }

    #[test]
    fn a_failed_install_leaves_nothing_behind() {
        let a: Vec<String> = abort(2).iter().map(|v| v.join(" ")).collect();
        assert_eq!(a[0], "umount -R /run/wolos/layer-root");
        assert!(a[1].starts_with("rm -rf /var/lib/extensions/wolos-layer-0002"));
        assert!(a[2].starts_with("rm -rf /var/lib/wolos/layers/wolos-layer-0002"));
    }

    #[test]
    fn what_a_layer_added_and_removed() {
        let r = delta(
            "bash 5.2-1\nvim 9.0-1\nold 1-1\n",
            "bash 5.2-1\nvim 9.1-1\ngit 2.4-1\n",
        );
        assert_eq!(r.installed, ["git-2.4-1", "vim-9.1-1"]);
        assert_eq!(r.removed, ["old-1-1", "vim-9.0-1"]);
        assert_eq!(Record::parse(&r.to_json()), r);
        assert_eq!(Record::parse("junk"), Record::default());
        assert_eq!(pkg_name("linux-api-headers-6.1-1"), "linux-api-headers");
    }

    #[test]
    fn only_kernel_and_init_packages_wait_for_a_restart() {
        for n in [
            "linux-wolos",
            "linux-firmware",
            "systemd",
            "systemd-libs",
            "mkinitcpio",
            "intel-ucode",
        ] {
            assert!(needs_restart(n), "{n}");
        }
        for n in [
            "vim",
            "firefox",
            "linux-api-headers",
            "libsystemd-fake",
            "util-linux",
        ] {
            assert!(!needs_restart(n), "{n}");
        }
    }

    #[test]
    fn whiteouts_and_order() {
        assert!(is_whiteout(true, 0));
        assert!(!is_whiteout(true, 5));
        assert!(!is_whiteout(false, 0));
        let o = ordered(vec![
            "wolos-layer-0002".into(),
            "x".into(),
            "wolos-layer-0001".into(),
        ]);
        assert_eq!(o, ["wolos-layer-0001", "wolos-layer-0002"]);
    }

    #[test]
    fn database_changes_follow_the_layers_in_order() {
        let r = |i: &[&str], d: &[&str]| Record {
            installed: i.iter().map(|s| s.to_string()).collect(),
            removed: d.iter().map(|s| s.to_string()).collect(),
        };
        let (add, drop) = db_changes(&[
            r(&["vim-9.0-1", "git-2-1"], &[]),
            r(&["vim-9.1-1"], &["vim-9.0-1"]),
            r(&[], &["git-2-1"]),
        ]);
        assert_eq!(add, ["vim-9.1-1"]);
        assert_eq!(drop, ["git"]);
    }
}
