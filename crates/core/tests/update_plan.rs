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
    // Removed once the new slot can boot (after esp-sync), before the last step.
    assert_eq!(
        s[s.len() - 2].to_string(),
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
