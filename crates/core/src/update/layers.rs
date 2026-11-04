// SPDX-License-Identifier: BSD-2-Clause
//! Live installs: the sealed slot stays read-only, a package change lands in a layer. The real
//! pacman runs in a chroot whose `/usr` is an overlay over the live one (`/etc` is the live,
//! writable one), the overlay's upper directory is a systemd-sysext extension, and
//! `systemd-sysext refresh` merges it over `/usr` at once. The package database is wherever
//! pacman.conf's `DBPath` puts it ([`db_path`]): an installed slot keeps it in `/usr/lib/pacman`
//! (ADR 0016), so a layer carries its own database entries in its `usr/`.
//! Removing a package that is in the sealed base leaves overlay whiteouts in the layer.
//! The next system update folds the layers into the new slot ([`super::build`]) and names them
//! in the slot's [`FOLDED`]. The live layers stay while the old slot runs; when the new slot
//! boots, `wol layers clear` ([`to_clear`]) removes the ones it already holds; the layers that
//! stay are unmerged only for systemd-sysext's own swap ([`clear_steps`]).
//! Pure: argv lists, texts and decisions. `wol pkg` and `wol layers fold|clear` run them.
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
/// Layers already copied into a slot (so a later fold does not undo that slot's own upgrades),
/// relative to the slot's root. In the slot's own sealed `/usr` (QA-STORE-44): not under `/var`
/// (the shared `@var` at boot) and not in `/etc`, which is the shared overlay at boot, so a
/// copy written there once lands in its upper directory and shadows every later slot's list.
/// Written by the fold before the seal, read-only afterwards: `wol layers clear` never writes
/// it (layer names are never handed out twice, [`next_index`], so a listed name that is gone
/// from [`DIR`] matches nothing again).
pub const FOLDED: &str = "usr/lib/wolos/folded-layers";
/// Where [`FOLDED`] was kept by slots folded before QA-STORE-44 (the slot's `/etc`).
pub const ETC_FOLDED: &str = "etc/wolos/folded-layers";
/// Where [`FOLDED`] was kept before that (inside the slot's `var` mount point, hidden at boot).
pub const LEGACY_FOLDED: &str = "var/lib/wolos/folded";

/// The booted slot's folded list: [`FOLDED`] when the slot has one (`usr`), else the list a
/// slot from before QA-STORE-44 kept in `/etc` (`etc`, read through the overlay).
pub fn folded_list<'a>(usr: Option<&'a str>, etc: Option<&'a str>) -> &'a str {
    usr.or(etc).unwrap_or("")
}
/// The highest layer number ever handed out (decimal text). Layer names are never reused: a
/// cleared layer's name may still be in a slot's [`FOLDED`] list (R2C-1).
pub const COUNTER: &str = "/var/lib/wolos/layer-counter";
/// pacman's own default `DBPath`.
pub const DEFAULT_DB_PATH: &str = "/var/lib/pacman";

/// pacman.conf's `DBPath` without the trailing `/` (an installed slot: `/usr/lib/pacman`);
/// [`DEFAULT_DB_PATH`] when the file has none or names a relative path or `..`.
pub fn db_path(pacman_conf: &str) -> String {
    pacman_conf
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == "DBPath").then(|| v.trim().trim_end_matches('/').to_owned())
        })
        .filter(|p| p.starts_with('/') && p.len() > 1 && !p.split('/').any(|c| c == ".."))
        .unwrap_or_else(|| DEFAULT_DB_PATH.to_owned())
}

/// The installed-package records (`<DBPath>/local`) of a pacman.conf.
pub fn local_db(pacman_conf: &str) -> String {
    format!("{}/local", db_path(pacman_conf))
}

/// `pacman -Q` of the layer while its chroot is still mounted: the chroot's `/usr` is the
/// layer's overlay, so a database under `/usr` shows the change before the layer is merged.
pub fn after_query(db: &str) -> Vec<String> {
    [PACMAN, "--dbpath", &format!("{CHROOT}{db}/"), "-Q"]
        .map(String::from)
        .to_vec()
}

/// A layer directory name: [`PREFIX`] and digits only (never a path).
pub fn is_layer_name(n: &str) -> bool {
    n.strip_prefix(PREFIX)
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// Live layers the booted slot already holds: names in its [`FOLDED`] text that are still in
/// `ls` of [`DIR`]. Anything that is not a layer name is ignored; empty when nothing matches.
pub fn to_clear(folded: &str, ls: &str) -> Vec<String> {
    let live: Vec<&str> = ls.lines().map(str::trim).collect();
    folded
        .lines()
        .map(str::trim)
        .filter(|n| is_layer_name(n) && live.contains(n))
        .map(str::to_owned)
        .collect()
}

/// Where [`clear_steps`] moves the layers it clears until the new merge is up (same file system
/// as [`DIR`], so the move is a rename and copies nothing).
pub const CLEARED_DIR: &str = "/var/lib/wolos/layers-cleared";

/// The step that merges [`DIR`] as it is now over `/usr`, in place of the merge that is up.
/// systemd-sysext swaps the two itself; it documents a brief moment between them (systemd 262),
/// far shorter than an unmerge, a removal and a merge one after the other.
/// Through systemd-sysext.service (its reload is `systemd-sysext refresh`), not by running the
/// tool here: `wol layers clear` runs from wolos-boot-ok.service, whose sandbox is its own mount
/// namespace, and a merge made there never reaches the system (the stale merge after a clear,
/// store-update-pkg-r2 R2-10). Not running: started, which merges.
pub fn refresh_step() -> Vec<String> {
    ["systemctl", "reload-or-restart", "systemd-sysext.service"]
        .map(String::from)
        .to_vec()
}

/// Steps that remove the layers [`to_clear`] found while the login window may already be up
/// (audit core-install #5): each moves out of [`DIR`] into [`CLEARED_DIR`] (a rename: nothing in
/// it changes under the live merge), [`refresh_step`] merges what is left, and only then are
/// the moved layers and their work directories removed, so the layers that stay are never
/// unmerged for as long as a removal takes. Empty when there is nothing to remove.
/// A caller that stops before the refresh still runs it; one that stops at or after it runs
/// nothing more (a failed refresh may still be using the moved layers).
pub fn clear_steps(names: &[String]) -> Vec<Vec<String>> {
    let s = |a: &[&str]| a.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    let names: Vec<&String> = names.iter().filter(|n| is_layer_name(n)).collect();
    if names.is_empty() {
        return Vec::new();
    }
    let mut out = vec![s(&["mkdir", "-p", CLEARED_DIR])];
    out.extend(names.iter().map(|n| {
        s(&[
            "mv",
            "-T",
            &format!("{DIR}/{n}"),
            &format!("{CLEARED_DIR}/{n}"),
        ])
    }));
    out.push(refresh_step());
    out.push(s(&["rm", "-rf", CLEARED_DIR]));
    out.extend(
        names
            .iter()
            .map(|n| s(&["rm", "-rf", &format!("{WORK_DIR}/{n}")])),
    );
    out
}

pub fn name(n: u32) -> String {
    format!("{PREFIX}{n:04}")
}

/// Next layer number: one above the highest of `ls` of [`DIR`], the persisted [`COUNTER`] and
/// every name in the booted slot's [`FOLDED`] list, so a name a slot still lists is never handed
/// out again (R2C-1). Later layers sort after earlier ones.
pub fn next_index(ls: &str, counter: &str, folded: &str) -> u32 {
    let num = |l: &str| l.trim().strip_prefix(PREFIX)?.parse::<u32>().ok();
    ls.lines()
        .chain(folded.lines())
        .filter_map(num)
        .chain(counter.trim().parse::<u32>().ok())
        .max()
        .map_or(1, |m| m.saturating_add(1))
}

/// The extension matches any system (`ID=_any`): a layer follows the person to the next slot
/// only by being folded into it, so it never needs to name the release.
pub fn extension_release() -> String {
    "ID=_any\nARCHITECTURE=x86-64\n".into()
}

fn v(a: &[&str]) -> Step {
    Step::Run(a.iter().map(|s| (*s).to_owned()).collect())
}

/// `umount -R` of [`CHROOT`], detached lazily when a process still holds it (QA-STORE-53): a
/// chroot left mounted would have the next `wol pkg` stack its mounts on top of it.
fn umount_argv() -> Vec<String> {
    vec![
        "sh".into(),
        "-c".into(),
        format!("umount -R {CHROOT} || umount -l -R {CHROOT}"),
    ]
}

/// The overlay of the layer over one top directory of the chroot (`usr`, `opt`).
fn overlay(l: &str, w: &str, top: &str) -> String {
    format!("lowerdir={CHROOT}/{top},upperdir={l}/{top},workdir={w}/{top}")
}

/// Steps up to and including the real pacman in the layer's chroot. The layer has no
/// extension-release yet: until [`finish`] writes it, systemd-sysext ignores the directory, so a
/// layer cut off mid-pacman (power loss, the 1800 s cap) never merges (QA-STORE-45). `/usr` and
/// `/opt` are overlays (both merge as system extensions): a package that installs under /opt
/// would otherwise meet the sealed, read-only root (QA-L2-32).
pub fn begin(n: u32, args: &[String]) -> Vec<Step> {
    let l = format!("{DIR}/{}", name(n));
    let w = format!("{WORK_DIR}/{}", name(n));
    let mut chroot = vec!["arch-chroot", CHROOT, PACMAN];
    chroot.extend(args.iter().map(String::as_str));
    vec![
        v(&[
            "mkdir",
            "-p",
            &format!("{l}/usr/lib/extension-release.d"),
            &format!("{l}/opt"),
            &format!("{w}/usr"),
            &format!("{w}/opt"),
            CHROOT,
        ]),
        // A chroot an earlier run left behind (QA-STORE-53).
        v(&[
            "sh",
            "-c",
            &format!("! mountpoint -q {CHROOT} || umount -l -R {CHROOT}"),
        ]),
        v(&["mount", "--rbind", "/", CHROOT]),
        v(&["mount", "--make-rprivate", CHROOT]),
        v(&[
            "mount",
            "-t",
            "overlay",
            "overlay",
            "-o",
            &overlay(&l, &w, "usr"),
            &format!("{CHROOT}/usr"),
        ]),
        v(&[
            "mount",
            "-t",
            "overlay",
            "overlay",
            "-o",
            &overlay(&l, &w, "opt"),
            &format!("{CHROOT}/opt"),
        ]),
        v(&chroot),
    ]
}

/// Steps after pacman succeeded: keep the record, unmount, make the layer an extension (its
/// extension-release, written last: QA-STORE-45), merge it. pacman has committed by now, so a
/// failure here never removes the layer (its files and database entries are the change): the
/// caller stops, and the next good boot finishes it (`boot-ok` writes a missing
/// extension-release of a layer that has its record) and merges it.
pub fn finish(n: u32, rec: &Record) -> Vec<Step> {
    let l = format!("{DIR}/{}", name(n));
    vec![
        Step::Write {
            path: format!("{l}/{RECORD}"),
            text: rec.to_json(),
        },
        Step::Run(umount_argv()),
        Step::Write {
            path: release_path(n),
            text: extension_release(),
        },
        v(&["rm", "-rf", &format!("{WORK_DIR}/{}", name(n))]),
        v(&["systemd-sysext", "refresh"]),
    ]
}

/// A layer's extension-release file.
pub fn release_path(n: u32) -> String {
    format!(
        "{DIR}/{0}/usr/lib/extension-release.d/extension-release.{0}",
        name(n)
    )
}

/// After a failed [`begin`] step (pacman did not commit): nothing of the layer is left, the
/// running system is untouched.
pub fn abort(n: u32) -> Vec<Vec<String>> {
    let s = |a: &[&str]| a.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    vec![
        umount_argv(),
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
        assert_eq!(next_index("", "", ""), 1);
        assert_eq!(
            next_index("wolos-layer-0001\nwolos-layer-0007\nother\n", "", ""),
            8
        );
        // R2C-1: after a clear removed 0001-0003, neither the counter nor the folded list
        // lets 0001 come back.
        assert_eq!(next_index("", "3\n", ""), 4);
        assert_eq!(next_index("", "", "wolos-layer-0003\n"), 4);
        assert_eq!(next_index("wolos-layer-0002\n", "9", "junk\n"), 10);
        assert_eq!(next_index("", "x", ""), 1);
        assert_eq!(name(12), "wolos-layer-0012");
    }

    #[test]
    fn pacman_runs_in_an_overlay_chroot_and_the_layer_is_merged_at_once() {
        let args: Vec<String> = ["-S", "vim"].map(String::from).to_vec();
        let t: Vec<String> = begin(3, &args).iter().map(|s| s.to_string()).collect();
        let l = "/var/lib/extensions/wolos-layer-0003";
        assert!(t[0].starts_with(&format!("mkdir -p {l}/usr/lib/extension-release.d")));
        assert_eq!(t[2], "mount --rbind / /run/wolos/layer-root");
        assert!(t[4].contains(&format!(
            "lowerdir=/run/wolos/layer-root/usr,upperdir={l}/usr,workdir=/var/lib/wolos/layers/wolos-layer-0003/usr"
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
        assert_eq!(f[0], format!("write {l}/wolos-layer.json"));
        assert!(f[1].contains("umount -R /run/wolos/layer-root"), "{f:?}");
        assert_eq!(f.last().unwrap(), "systemd-sysext refresh");
        assert!(
            f.iter()
                .any(|s| s == &format!("write {l}/wolos-layer.json"))
        );
    }

    /// QA-STORE-45: a layer becomes a system extension only once pacman committed and its
    /// record is written: begin writes no extension-release, finish writes it after the record.
    /// A power cut or the 1800 s cap mid-pacman leaves a directory sysext ignores.
    #[test]
    fn extension_release_written_last() {
        let args: Vec<String> = ["-S", "vim"].map(String::from).to_vec();
        let b: Vec<String> = begin(3, &args).iter().map(|s| s.to_string()).collect();
        assert!(
            !b.iter()
                .any(|s| s.contains("extension-release.wolos-layer")),
            "{b:?}"
        );
        let f: Vec<String> = finish(3, &Record::default())
            .iter()
            .map(|s| s.to_string())
            .collect();
        let rec = f
            .iter()
            .position(|s| s.ends_with("/wolos-layer.json"))
            .unwrap();
        let rel = f
            .iter()
            .position(|s| {
                s == "write /var/lib/extensions/wolos-layer-0003/usr/lib/extension-release.d/\
extension-release.wolos-layer-0003"
            })
            .unwrap();
        assert!(rec < rel, "{f:?}");
        let refresh = f
            .iter()
            .position(|s| s == "systemd-sysext refresh")
            .unwrap();
        assert!(rel < refresh, "{f:?}");
    }

    /// QA-STORE-53: a chroot that cannot be unmounted at once (a process still in it) is
    /// detached lazily, never left stacked; begin detaches one an earlier run left.
    #[test]
    fn umount_lazy_fallback() {
        let lazy = format!("umount -R {CHROOT} || umount -l -R {CHROOT}");
        let f: Vec<String> = finish(3, &Record::default())
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(f.iter().any(|s| s.contains(&lazy)), "{f:?}");
        let a: Vec<String> = abort(3).iter().map(|v| v.join(" ")).collect();
        assert!(a[0].contains(&lazy), "{a:?}");
        let args: Vec<String> = ["-S", "vim"].map(String::from).to_vec();
        let b: Vec<String> = begin(3, &args).iter().map(|s| s.to_string()).collect();
        let stale = b
            .iter()
            .position(|s| s.contains(&format!("umount -l -R {CHROOT}")))
            .unwrap();
        let bind = b
            .iter()
            .position(|s| s == &format!("mount --rbind / {CHROOT}"))
            .unwrap();
        assert!(stale < bind, "{b:?}");
    }

    /// QA-L2-32: pacman sees /opt writable in the layer chroot too (ROCm and other large
    /// packages install there; systemd-sysext merges /opt as well as /usr).
    #[test]
    fn opt_is_layered_too() {
        let args: Vec<String> = ["-S", "blender"].map(String::from).to_vec();
        let b: Vec<String> = begin(4, &args).iter().map(|s| s.to_string()).collect();
        let l = "/var/lib/extensions/wolos-layer-0004";
        assert!(
            b.iter().any(|s| s.contains(&format!("upperdir={l}/opt"))
                && s.ends_with(&format!("{CHROOT}/opt"))),
            "{b:?}"
        );
    }

    /// QA-STORE-44: the folded list lives in the slot's sealed /usr (never in the shared /etc
    /// overlay, whose upper copy would shadow every later slot's list); a slot sealed before
    /// keeps it in /etc and is read there when /usr has none.
    #[test]
    fn folded_path_usr_lib_with_legacy() {
        assert_eq!(FOLDED, "usr/lib/wolos/folded-layers");
        assert!(!FOLDED.starts_with("etc/"));
        let new = "wolos-layer-0004\n";
        let old = "wolos-layer-0002\n";
        assert_eq!(
            folded_list(Some(new), Some(old)),
            new,
            "the slot's own list wins"
        );
        assert_eq!(
            folded_list(None, Some(old)),
            old,
            "a slot from before: /etc"
        );
        assert_eq!(folded_list(None, None), "");
        // An empty list in the /etc upper (what clear wrote before) hides nothing any more.
        assert_eq!(folded_list(Some(new), Some("")), new);
    }

    #[test]
    fn a_failed_install_leaves_nothing_behind() {
        let a: Vec<String> = abort(2).iter().map(|v| v.join(" ")).collect();
        assert!(a[0].contains("umount -R /run/wolos/layer-root"), "{a:?}");
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
    fn the_package_database_is_where_pacman_conf_says() {
        let slot = "[options]\nHoldPkg = pacman\nDBPath = /usr/lib/pacman/\n[core]\n";
        assert_eq!(db_path(slot), "/usr/lib/pacman");
        assert_eq!(local_db(slot), "/usr/lib/pacman/local");
        assert_eq!(
            db_path("[options]\n#DBPath = /usr/lib/pacman/\n"),
            DEFAULT_DB_PATH
        );
        assert_eq!(db_path(""), "/var/lib/pacman");
        assert_eq!(db_path("DBPath = relative/\n"), DEFAULT_DB_PATH);
        assert_eq!(db_path("DBPath = /usr/../etc\n"), DEFAULT_DB_PATH);
        assert_eq!(
            after_query("/usr/lib/pacman").join(" "),
            "/usr/bin/pacman --dbpath /run/wolos/layer-root/usr/lib/pacman/ -Q"
        );
    }

    #[test]
    fn the_booted_slot_clears_only_the_layers_it_holds() {
        let folded = "wolos-layer-0001\nwolos-layer-0002\n../../etc\nwolos-layer-x\n";
        let ls = "wolos-layer-0002\nwolos-layer-0003\n";
        assert_eq!(to_clear(folded, ls), ["wolos-layer-0002"]);
        let s: Vec<String> = clear_steps(&to_clear(folded, ls))
            .iter()
            .map(|a| a.join(" "))
            .collect();
        // audit core-install #5: no unmerge; the layer moves aside, the new merge replaces the
        // old one, and only then is anything deleted.
        assert_eq!(
            s,
            [
                "mkdir -p /var/lib/wolos/layers-cleared",
                "mv -T /var/lib/extensions/wolos-layer-0002 /var/lib/wolos/layers-cleared/wolos-layer-0002",
                "systemctl reload-or-restart systemd-sysext.service",
                "rm -rf /var/lib/wolos/layers-cleared",
                "rm -rf /var/lib/wolos/layers/wolos-layer-0002",
            ]
        );
        assert!(!s.iter().any(|x| x.contains("unmerge")));
        assert!(clear_steps(&[]).is_empty());
        assert!(clear_steps(&["../x".into()]).is_empty());
        assert!(to_clear("", ls).is_empty());
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
