// SPDX-License-Identifier: BSD-2-Clause
//! The slot update plan as a person's restore point sees it (qa100 B4): what an interrupted or
//! failed update leaves behind, step by step.
use wol_core::update::{self as up, Base, Step};

fn text(steps: &[Step]) -> Vec<String> {
    steps.iter().map(ToString::to_string).collect()
}

/// A plan into wolos-b while wolos-b holds the earlier restore point.
fn replace() -> Vec<Step> {
    up::plan_for("wolos-b", true, Some("2026/11/02"), None).unwrap()
}

fn touches_slot_b(line: &str) -> bool {
    line.contains("/run/wolos/top/@wolos-b")
        || line.contains("-name @wolos-b ")
        || line.contains("/loader/entries")
        || line.contains("EFI/wolos")
}

/// QA-STORE-3: nothing of the earlier restore point (its subvolume, its entries, its image)
/// changes before the new copy is complete and sealed; pacman runs in the scratch copy.
#[test]
fn replace_has_no_delete_before_pacman() {
    let t = text(&replace());
    let seal = t
        .iter()
        .position(|s| s.starts_with("wol seal apply"))
        .unwrap();
    let pacman = t
        .iter()
        .position(|s| s.contains("/usr/bin/pacman -Syu"))
        .unwrap();
    assert!(pacman < seal, "{t:?}");
    assert!(
        t[pacman].contains("@wolos-tx"),
        "built aside, not in the slot: {}",
        t[pacman]
    );
    for line in &t[..=seal] {
        assert!(!touches_slot_b(line), "before the seal: {line}");
    }
    // The seal record goes next to the slot's own until the swap.
    assert!(t[seal].ends_with("/efi/wolos-b/verity.new"), "{}", t[seal]);
}

/// QA-STORE-3: a failure at any step leaves no boot entry naming a missing slot or image. Before
/// the swap the slot is kept and nothing of it is touched; from the swap on, every entry of the
/// slot (blessed `wolos-b.conf` too, not only `+3`) goes before any image or the subvolume.
#[test]
fn rollback_never_removes_referenced_image() {
    let steps = replace();
    let t = text(&steps);
    let swap = t
        .iter()
        .position(|s| s.starts_with("find /efi/loader/entries"))
        .expect("the swap removes the old entries first");
    for (i, step) in t.iter().enumerate() {
        let undo = up::rollback_at("wolos-b", Base::Running { replace: true }, &steps, i);
        let u: Vec<String> = undo.argv.iter().map(|a| a.join(" ")).collect();
        if i < swap {
            assert!(undo.kept, "step {i} {step}: {u:?}");
            for line in &u {
                assert!(!touches_slot_b(line), "step {i}: {line}");
            }
            continue;
        }
        if undo.done {
            // After esp-sync: the new slot is complete and has its entry; nothing is removed.
            assert!(u.iter().all(|l| !touches_slot_b(l)), "step {i}: {u:?}");
            continue;
        }
        assert!(!undo.kept, "step {i}");
        let entries = u
            .iter()
            .position(|l| l.contains("/loader/entries"))
            .unwrap();
        assert!(
            u[entries].contains("-name wolos-b.conf"),
            "the blessed entry too: {}",
            u[entries]
        );
        for (k, line) in u.iter().enumerate() {
            if line.contains("EFI/wolos") || line.contains("-name @wolos-b ") {
                assert!(entries < k, "step {i}: {line} before the entries");
            }
        }
    }
}

/// QA-STORE-8, QA-LIVE4-8: a scratch copy a power cut left is unmounted and deleted before the
/// new snapshot (the retry failed with "Could not create subvolume: File exists"); with none
/// there the steps succeed (find runs nothing).
#[test]
fn leftover_tx_cleaned_before_snapshot() {
    for from in [Base::Running { replace: true }, Base::Pending] {
        let t = text(&up::plan_from("wolos-b", from, None, None, &[]).unwrap());
        let snap = t
            .iter()
            .position(|s| s.starts_with("btrfs subvolume snapshot"))
            .unwrap();
        assert!(
            t[snap].ends_with(" /run/wolos/top/@wolos-tx"),
            "{}",
            t[snap]
        );
        let clean: Vec<&String> = t[..snap]
            .iter()
            .filter(|s| s.contains("@wolos-tx"))
            .collect();
        assert_eq!(
            clean,
            [
                "find /run/wolos/top -maxdepth 1 -name @wolos-tx -exec umount -R {} ;",
                "find /run/wolos/top -maxdepth 1 -name @wolos-tx -exec btrfs subvolume delete {} +",
            ],
            "{t:?}"
        );
    }
}

/// QA-STORE-46, RH-192: pacman runs in the copy through `wol update chroot` under `unshare`,
/// which binds the shared package cache there (no second download, no copy kept in the slot) in
/// a mount namespace of that one process: the plan mounts nothing itself, so a failed step or a
/// cut leaves nothing mounted; an undo still unmounts the copy with everything under it before
/// deleting it.
#[test]
fn cache_bind_and_umount_in_rollback() {
    let steps = replace();
    let t = text(&steps);
    let tx = "/run/wolos/top/@wolos-tx";
    assert!(
        !t.iter()
            .any(|s| s.starts_with("mount ") || s.starts_with("umount ")),
        "no mount outside a private namespace: {t:?}"
    );
    let pacman = t
        .iter()
        .position(|s| {
            *s == format!(
                "unshare --mount --propagation private wol update chroot {tx} -- \
/usr/bin/pacman -Syu --noconfirm"
            )
        })
        .expect("pacman in the copy, privately");
    let undo = up::rollback_at("wolos-b", Base::Running { replace: true }, &steps, pacman);
    let u: Vec<String> = undo.argv.iter().map(|a| a.join(" ")).collect();
    let umount = u
        .iter()
        .position(|l| l.contains("@wolos-tx -exec umount -R"))
        .unwrap();
    let delete = u
        .iter()
        .position(|l| l.contains("@wolos-tx -exec btrfs subvolume delete"))
        .unwrap();
    assert!(umount < delete, "{u:?}");
}

/// QA-STORE-49: held packages stay at their version in every slot build: `plan_from` takes the
/// holds, and pacman in the slot ignores them (as its pacman.conf `IgnorePkg` would).
#[test]
fn holds_in_slot_pacman_conf() {
    let holds = ["firefox".to_owned(), "gimp".to_owned()];
    for from in [Base::Running { replace: true }, Base::Pending] {
        let t = text(&up::plan_from("wolos-b", from, Some("2026/11/02"), None, &holds).unwrap());
        assert!(
            t.iter()
                .any(|s| s.ends_with("/usr/bin/pacman -Syu --noconfirm --ignore=firefox,gimp")),
            "{t:?}"
        );
    }
}

/// QA-LIVE4-14: a build marks itself in the shared /var first and unmarks itself last; a mark
/// found while no build holds the update lock is an update that was cut off.
#[test]
fn interrupted_build_leaves_a_marker() {
    let s = replace();
    assert_eq!(
        s.first(),
        Some(&Step::Write {
            path: up::BUILDING.into(),
            text: "wolos-b 2026/11/02\n".into()
        })
    );
    // Removed once the new slot can boot (after esp-sync), before the default and the last sync.
    assert_eq!(
        s[s.len() - 3].to_string(),
        format!("rm -f {}", up::BUILDING)
    );
    let undo = up::rollback_at("wolos-b", Base::Running { replace: true }, &s, 5);
    assert!(
        undo.argv
            .iter()
            .any(|a| a.contains(&up::BUILDING.to_owned()))
    );
    assert_eq!(
        up::interrupted(Some("wolos-b 2026/11/02\n"), false),
        Some("wolos-b")
    );
    assert_eq!(
        up::interrupted(Some("wolos-b current\n"), true),
        None,
        "running now"
    );
    assert_eq!(up::interrupted(None, false), None);
    assert_eq!(up::interrupted(Some("../x\n"), false), None);
}

/// QA-UPG-11: the copy's /etc/kernel/cmdline names its own subvolume (it said @wolos-a in b).
#[test]
fn slot_cmdline_names_own_subvol() {
    let t = text(&replace());
    let tx = "/run/wolos/top/@wolos-tx";
    let fix = t
        .iter()
        .find(|s| s.contains("/etc/kernel/cmdline"))
        .expect("cmdline fixed");
    assert!(fix.starts_with(&format!("find {tx}/etc ")), "{fix}");
    assert!(
        fix.contains("s#subvol=/?@wolos-[ab]([ ,]|$)#subvol=@wolos-b\\1#"),
        "{fix}"
    );
    let seal = t
        .iter()
        .position(|s| s.starts_with("wol seal apply"))
        .unwrap();
    assert!(t.iter().position(|s| s == fix).unwrap() < seal);
}

/// QA-LIVE3-1 (critical, data loss): `wol layers clear` on the new slot's good boot used to delete
/// the layers it folded in, so a rollback to the restore point lost every program installed since
/// the install. Now they are kept out of the merge until both slots hold them, and a slot that
/// does not hold a kept one gets it back into the merge.
#[test]
fn rollback_keeps_or_lists_folded_layers() {
    use wol_core::update::layers::{self, Tidy, tidy, tidy_steps};
    let htop = "wolos-layer-0001".to_owned();
    // The new slot's first good boot: folded, out of the merge, kept (never deleted).
    let t = tidy(
        "wolos-layer-0001\n",
        Some(""),
        "wolos-layer-0001\n",
        "",
        &[],
    );
    assert_eq!(
        t,
        Tidy {
            keep: vec![htop.clone()],
            ..Tidy::default()
        }
    );
    let s: Vec<String> = tidy_steps(&t).iter().map(|a| a.join(" ")).collect();
    assert!(
        s.iter()
            .any(|l| *l == format!("mv -T {}/{htop} {}/{htop}", layers::DIR, layers::KEPT_DIR))
    );
    assert!(
        !s.iter()
            .any(|l| l.contains(&format!("rm -rf {}/{htop}", layers::DIR))
                || l.contains(&format!("rm -rf {}/{htop}", layers::KEPT_DIR))),
        "{s:?}"
    );
    // After a rollback: the restore point's list lacks it, so it goes back into the merge.
    let back = tidy(
        "",
        Some("wolos-layer-0001\n"),
        "",
        "wolos-layer-0001\n",
        &[],
    );
    assert_eq!(back.restore, std::slice::from_ref(&htop));
    let s: Vec<String> = tidy_steps(&back).iter().map(|a| a.join(" ")).collect();
    assert_eq!(
        s[0],
        format!("mv -T {}/{htop} {}/{htop}", layers::KEPT_DIR, layers::DIR)
    );
    assert_eq!(s[1], layers::refresh_step().join(" "));
    // Both slots hold it: gone for good; the other slot unknown: kept.
    assert_eq!(
        tidy(
            "wolos-layer-0001\n",
            Some("wolos-layer-0001\n"),
            "",
            "wolos-layer-0001\n",
            &[]
        )
        .prune,
        std::slice::from_ref(&htop)
    );
    assert!(
        tidy("wolos-layer-0001\n", None, "", "wolos-layer-0001\n", &[])
            .prune
            .is_empty()
    );
    // An unfinished layer is still removed, not kept.
    let u = tidy(
        "",
        None,
        "wolos-layer-0002\n",
        "",
        &["wolos-layer-0002".to_owned()],
    );
    assert_eq!(u.drop, ["wolos-layer-0002"]);
    assert!(u.keep.is_empty());
}

/// QA-LIVE4-13: Settings and the daily check read the live /etc/pacman.conf (with a repo the
/// person added); the slot build used the sealed copy under the overlay and found nothing to do.
/// The copy gets the live file before its repos are dated and pacman runs.
#[test]
fn slot_pacman_conf_equals_live() {
    let t = text(&replace());
    let tx = "/run/wolos/top/@wolos-tx";
    let cp = t
        .iter()
        .position(|s| {
            *s == format!("cp --remove-destination /etc/pacman.conf {tx}/etc/pacman.conf")
        })
        .expect("live pacman.conf copied");
    let date = t
        .iter()
        .position(|s| s.contains("/repos/2026/11/02/"))
        .unwrap();
    let pacman = t
        .iter()
        .position(|s| s.contains("/usr/bin/pacman -Syu"))
        .unwrap();
    assert!(cp < date && date < pacman, "{t:?}");
    // A change stacked onto the pending slot keeps that slot's own file (review of B4).
    let stacked = text(&up::plan_from("wolos-b", Base::Pending, None, None, &[]).unwrap());
    assert!(
        !stacked
            .iter()
            .any(|s| s.starts_with("cp --remove-destination /etc/pacman.conf")),
        "{stacked:?}"
    );
}

/// Review MUST 1: the ESP is FAT; removed entries are on the disk only after a flush. Every step
/// that deletes boot entries is followed at once by `sync -f /efi`, before an image or a root
/// subvolume goes, in the swap and in the undo: a power cut never leaves an entry naming a
/// deleted root.
#[test]
fn entries_are_flushed_before_their_slot_goes() {
    let check = |lines: &[String], what: &str| {
        let del = lines
            .iter()
            .position(|l| l.starts_with("find /efi/loader/entries") && l.ends_with("-delete"))
            .unwrap_or_else(|| panic!("{what}: no entries step: {lines:?}"));
        assert_eq!(lines[del + 1], "sync -f /efi", "{what}: {lines:?}");
        for (i, l) in lines.iter().enumerate() {
            if l.contains("EFI/wolos") || l.contains("subvolume delete") && l.contains("@wolos-b") {
                assert!(i > del + 1, "{what}: {l} before the flush");
            }
        }
    };
    check(&text(&replace()), "swap");
    let steps = replace();
    let t = text(&steps);
    let swap = t
        .iter()
        .position(|s| s.starts_with("find /efi/loader/entries"))
        .unwrap();
    let undo = up::rollback_at("wolos-b", Base::Running { replace: true }, &steps, swap + 2);
    let u: Vec<String> = undo.argv.iter().map(|a| a.join(" ")).collect();
    check(&u, "undo");
}

/// Review 5: a cut after the new slot's image and entry are on the ESP (it boots next) is no
/// interrupted update: the mark goes right after esp-sync, before anything else.
#[test]
fn the_mark_goes_once_the_slot_can_boot() {
    let t = text(&replace());
    let esp = t
        .iter()
        .position(|s| s.ends_with("esp-sync --new"))
        .unwrap();
    assert_eq!(t[esp + 1], format!("rm -f {}", up::BUILDING), "{t:?}");
}

/// The released mirrorlist files (wolos-pkgbuilds rel/0.12.0-3 and rel/0.12.1).
fn mirrorlist(version: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mirrorlist")
        .join(format!("wolos-mirrorlist-{version}"));
    std::fs::read_to_string(p).unwrap()
}

const TESTING_CONF: &str = "[options]\nSigLevel = Required\n\n[core]\nServer = https://archive.archlinux.org/repos/2026/10/31/$repo/os/$arch\n\n\
[wolos-testing]\nInclude = /etc/pacman.d/wolos-mirrorlist\n";

/// A scratch root with that pacman.conf and mirrorlist.
fn repo_root(name: &str, conf: &str, list: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("wol-upg6-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("etc/pacman.d")).unwrap();
    std::fs::write(d.join("etc/pacman.conf"), conf).unwrap();
    std::fs::write(d.join("etc/pacman.d/wolos-mirrorlist"), list).unwrap();
    d
}

/// Runs the plan's `sed` steps on the copy's repo files, as the update does, but on `root`.
fn run_repo_steps(steps: &[Step], root: &std::path::Path) {
    let work = "/run/wolos/top/@wolos-tx";
    for s in steps {
        let Step::Run(a) = s else { continue };
        let Some(file) = a.last() else { continue };
        if a[0] != "sed"
            || !(file.ends_with("/etc/pacman.conf") || file.ends_with("wolos-mirrorlist"))
        {
            continue;
        }
        let at = file.strip_prefix(work).expect(file);
        let mut c = std::process::Command::new(&a[0]);
        c.args(&a[1..a.len() - 1])
            .arg(root.join(at.trim_start_matches('/')));
        let o = c.output().unwrap();
        assert!(
            o.status.success(),
            "{a:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

/// QA-UPG-6: the 0.12.0 release image installed `[wolos-testing]` (install-channel) and a list
/// ending in `wolos-testing`, a tag without a date: the update's sed matched nothing, the 0.12.1
/// list landed as `.pacnew` and every later slot kept the testing channel. A release update now
/// leaves the copy on the release's own repo, `[wolos]` and the list of that release's package;
/// for a stable 0.12.0 list and for the 0.12.1 one moving on to the next release alike.
#[test]
fn release_update_moves_to_the_release_repo() {
    let repo = |p: &str| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(p)
    };
    // The 0.12.0 testing install, made by the real install-channel from the live medium's
    // pacman.conf (`[wolos-testing]` + `Server = .../wolos-testing`).
    let testing = repo_root(
        "testing",
        &TESTING_CONF.replace(
            "Include = /etc/pacman.d/wolos-mirrorlist",
            "Server = https://github.com/wonderlandos/wolos-packages/releases/download/wolos-testing",
        ),
        &mirrorlist("2026.10.31-3"),
    );
    let o = std::process::Command::new("sh")
        .arg(repo("files/usr/lib/wolos/install-channel"))
        .arg(&testing)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let installed = std::fs::read_to_string(testing.join("etc/pacman.d/wolos-mirrorlist")).unwrap();
    assert!(
        installed.contains("download/wolos-testing\n"),
        "{installed}"
    );
    assert!(
        std::fs::read_to_string(testing.join("etc/pacman.conf"))
            .unwrap()
            .contains("[wolos-testing]")
    );

    let to_0121 = up::plan_for("wolos-b", true, Some("2026/11/02"), None).unwrap();
    let stable = repo_root(
        "stable",
        &TESTING_CONF.replace("[wolos-testing]", "[wolos]"),
        &mirrorlist("2026.10.31-3"),
    );
    for root in [&testing, &stable] {
        run_repo_steps(&to_0121, root);
        let list = std::fs::read_to_string(root.join("etc/pacman.d/wolos-mirrorlist")).unwrap();
        assert_eq!(list, mirrorlist("2026.11.02-1"), "{}", root.display());
        let conf = std::fs::read_to_string(root.join("etc/pacman.conf")).unwrap();
        assert!(
            conf.contains("\n[wolos]\nInclude = /etc/pacman.d/wolos-mirrorlist\n"),
            "{conf}"
        );
        assert!(!conf.contains("wolos-testing"), "{conf}");
        assert!(conf.contains("/repos/2026/11/02/"), "{conf}");
    }
    // The 0.12.1 file moving to a later release: its tag and dates follow.
    let later = up::plan_for("wolos-a", true, Some("2026/12/01"), None).unwrap();
    let r = repo_root(
        "later",
        &TESTING_CONF.replace("[wolos-testing]", "[wolos]"),
        &mirrorlist("2026.11.02-1"),
    );
    run_repo_steps(&later, &r);
    let list = std::fs::read_to_string(r.join("etc/pacman.d/wolos-mirrorlist")).unwrap();
    assert_eq!(
        list,
        mirrorlist("2026.11.02-1").replace("2026-11-02", "2026-12-01")
    );
    // A same-release update (no date) leaves the channel and the list alone.
    let same = up::plan_for("wolos-a", true, None, None).unwrap();
    assert!(
        !text(&same)
            .iter()
            .any(|s| s.contains("wolos-mirrorlist") && s.starts_with("sed")),
        "{:?}",
        text(&same)
    );
    for d in [testing, stable, r] {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// QA-UPG-6: a list pacman still found changed (ranked, edited) leaves the release's file as
/// `.pacnew`; right after pacman the release's file replaces it, before the copy is sealed. Only
/// for a release update.
#[test]
fn pacnew_mirrorlist_replaced() {
    let t = text(&replace());
    let pacman = t
        .iter()
        .position(|s| s.contains("/usr/bin/pacman -Syu"))
        .unwrap();
    let seal = t
        .iter()
        .position(|s| s.starts_with("wol seal apply"))
        .unwrap();
    let pacnew = t
        .iter()
        .position(|s| {
            s == "find /run/wolos/top/@wolos-tx/etc/pacman.d -maxdepth 1 -name wolos-mirrorlist.pacnew \
-exec mv -f {} /run/wolos/top/@wolos-tx/etc/pacman.d/wolos-mirrorlist ;"
        })
        .unwrap_or_else(|| panic!("{t:?}"));
    assert!(pacman < pacnew && pacnew < seal, "{t:?}");
    let same = text(&up::plan_for("wolos-b", true, None, None).unwrap());
    assert!(!same.iter().any(|s| s.contains(".pacnew")), "{same:?}");
}

/// QA-LIVE4-7: the updater said "updated into wolos-b" while the new root and its record were
/// not on the disk yet; a cut then booted an entry with an empty image or no root. The root and
/// its seal record are synced before esp-sync writes an entry, and the plan ends with a sync, so
/// "updated" (said after the last step) is true. A cut-off build's seal leftovers go first.
#[test]
fn updated_only_after_everything_is_on_the_disk() {
    let t = text(&replace());
    let swap = t
        .iter()
        .position(|s| s == "mv /run/wolos/top/@wolos-tx /run/wolos/top/@wolos-b")
        .unwrap();
    let digest = t
        .iter()
        .position(|s| s == "mv -f /efi/wolos-b/verity.new /efi/wolos-b/verity")
        .unwrap();
    let esp = t
        .iter()
        .position(|s| s.ends_with("esp-sync --new"))
        .unwrap();
    let sync = t[digest..esp]
        .iter()
        .position(|s| s == "sync")
        .map(|i| i + digest);
    assert!(
        sync.is_some_and(|i| swap < i && digest < i && i < esp),
        "root and record on the disk before an entry names them: {t:?}"
    );
    assert_eq!(t.last().map(String::as_str), Some("sync"), "{t:?}");
    let leftovers = t
        .iter()
        .position(|s| {
            s == "find /efi -mindepth 2 -maxdepth 2 ( -path /efi/wolos-b/verity.new -o -path \
/efi/wolos-b/verity*.wol-tmp ) -delete"
        })
        .unwrap_or_else(|| panic!("{t:?}"));
    let seal = t
        .iter()
        .position(|s| s.starts_with("wol seal apply"))
        .unwrap();
    assert!(leftovers < seal, "{t:?}");
}
