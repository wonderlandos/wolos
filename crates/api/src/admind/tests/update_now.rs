// SPDX-License-Identifier: GPL-2.0-or-later
//! Update Now in wol-admind (UPD-restore): the restore point it keeps, the refusal without one,
//! one copy updated in place, the sentence a failure leaves, and the clean-up of a failed slot.
use super::*;
use shared::admin::encode;

/// Update Now keeps a restore point first (the page says so): the running slot wolos-a is
/// named in the log before anything runs, and the `[wolos]` fixes go into wolos-b through the
/// shared slot builder (snapshot, pacman inside the release, initramfs, seal, boot entry).
/// No step writes the restore point.
#[test]
fn update_now_keeps_a_restore_point_first() {
    let os = slot_os();
    let up = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
    assert_eq!(handle(&up, ROOT, &Shared::default(), &os), AdminReply::Ok);
    let ran: Vec<String> = os.ran.borrow().iter().map(|a| a.join(" ")).collect();
    // QA-STORE-3: built in the scratch copy; wolos-b (the earlier restore point) goes only at
    // the swap, after the seal.
    let b = "/run/wolos/top/@wolos-tx";
    let snap = ran
        .iter()
        .position(|l| *l == format!("btrfs subvolume snapshot / {b}"))
        .expect("snapshot");
    assert_eq!(ran[snap + 1], format!("wol layers fold {b}"));
    let seal = ran
        .iter()
        .position(|l| l.starts_with("wol seal apply"))
        .unwrap();
    let swap = ran
        .iter()
        .position(|l| l.contains("-name @wolos-b -exec btrfs subvolume delete"))
        .unwrap();
    assert!(seal < swap, "{ran:?}");
    // After the fold, the slot's own /etc overlay line (ADR 0044, 0.12.1), then pacman.
    let pacman = format!(
        "unshare --mount --propagation private wol update chroot {b} -- /usr/bin/pacman -Syu --noconfirm"
    );
    let at = ran
        .iter()
        .position(|l| *l == pacman)
        .expect("pacman in the slot");
    assert!(at > snap + 1, "{ran:?}");
    assert_eq!(ran.last().unwrap(), "bootctl set-default ");
    assert!(ran.iter().all(|l| !l.contains("@wolos-a")), "{ran:?}");
    assert!(
        ran.iter().all(|l| l != "pacman -Syu --noconfirm"),
        "never pacman on the live root"
    );
    let log = os.log.borrow();
    assert_eq!(
        log[0],
        "root(0): pkg-upgrade: restore point wolos-a (the running system) is kept; updating into wolos-b"
    );
    assert!(
        log.iter().any(|l| l.contains("updated into wolos-b")),
        "{log:?}"
    );
    // Holds stay at their version, in the slot.
    let held = slot_os();
    let r = req(AdminAction::PkgUpgrade, &["firefox", "gimp"]);
    handle(&encode(&r, b""), ROOT, &Shared::default(), &held);
    assert!(held.ran.borrow().iter().any(|a| a.join(" ")
        == format!("unshare --mount --propagation private wol update chroot {b} -- /usr/bin/pacman -Syu --noconfirm --ignore=firefox,gimp")));
    let wifi = Fake::default();
    let r = req(AdminAction::WifiPower, &["wlan0", "off"]);
    handle(&encode(&r, b""), ROOT, &Shared::default(), &wifi);
    assert!(wifi.captured.borrow().is_empty());
}

/// A machine with one copy of WolOS (no root slot) cannot keep a restore point: Update Now
/// is refused with one sentence and nothing runs, not even pacman on the live root.
#[test]
fn update_now_without_a_restore_point_is_refused() {
    for os in [
        Fake::default(),
        Fake {
            files: vec![("/proc/cmdline".into(), "root=/dev/vda2 rw".into())],
            ..Fake::default()
        },
    ] {
        let up = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
        assert_eq!(
            handle(&up, ROOT, &Shared::default(), &os),
            AdminReply::Failed
        );
        assert!(os.ran.borrow().is_empty(), "{:?}", os.ran.borrow());
        let log = os.log.borrow();
        assert_eq!(
            log[0],
            format!(
                "root(0): pkg-upgrade: {}",
                wol_core::update::pkg::NO_RESTORE_POINT
            )
        );
        drop(log);
        // The page gets the same sentence: the failed reply carries it.
        let mut said = None;
        let again = handle_said(&up, ROOT, &Shared::default(), &os, &mut said);
        assert_eq!(again, AdminReply::Failed);
        assert_eq!(
            said.as_deref(),
            Some(wol_core::update::pkg::NO_RESTORE_POINT)
        );
        let line = reply_line(again, said.as_deref());
        assert_eq!(AdminReply::parse(&line), AdminReply::Failed, "old clients");
        assert_eq!(
            shared::admin::failure_message(&line).as_deref(),
            Some(wol_core::update::pkg::NO_RESTORE_POINT)
        );
    }
}

/// One copy of WolOS (ADR 0023): Update Now updates the running system in place (the page
/// said there is no restore point); a failure says some packages may already be updated.
#[test]
fn update_now_on_one_copy_updates_in_place() {
    let one = || Fake {
        files: vec![
            (wol_core::update::SINGLE_COPY.into(), String::new()),
            ("/proc/cmdline".into(), "root=/dev/vda2 rw".into()),
        ],
        ..Fake::default()
    };
    let os = one();
    let r = req(AdminAction::PkgUpgrade, &["gimp"]);
    let mut said = None;
    assert_eq!(
        handle_said(&encode(&r, b""), ROOT, &Shared::default(), &os, &mut said),
        AdminReply::Ok
    );
    assert_eq!(said, None);
    let ran: Vec<String> = os.ran.borrow().iter().map(|a| a.join(" ")).collect();
    assert_eq!(ran, ["/usr/bin/pacman -Syu --noconfirm --ignore=gimp"]);
    assert!(os.log.borrow()[0].contains("one copy of WolOS, no restore point"));
    assert!(os.captured.borrow().is_empty(), "no slot is looked for");
    let failing = Fake {
        fail_run: true,
        ..one()
    };
    let mut said = None;
    assert_eq!(
        handle_said(
            &encode(&r, b""),
            ROOT,
            &Shared::default(),
            &failing,
            &mut said
        ),
        AdminReply::Failed
    );
    assert_eq!(
        said.as_deref(),
        Some(wol_core::update::pkg::IN_PLACE_FAILED)
    );
}

/// Only a failure carries a sentence; every other reply keeps its plain line.
#[test]
fn reply_lines_carry_a_sentence_only_on_failure() {
    assert_eq!(reply_line(AdminReply::Ok, Some("x")), "ok");
    assert_eq!(reply_line(AdminReply::Failed, None), "failed");
    assert_eq!(
        reply_line(AdminReply::Failed, Some("It broke.")),
        "failed It broke."
    );
    assert_eq!(reply_line(AdminReply::Denied, Some("x")), "denied");
}

/// A step that fails deletes the half-made slot; the restore point is untouched and named.
#[test]
fn a_failed_update_now_cleans_up_and_keeps_the_restore_point() {
    let os = Fake {
        fail_on: Some("mkinitcpio"),
        ..slot_os()
    };
    let up = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
    assert_eq!(
        handle(&up, ROOT, &Shared::default(), &os),
        AdminReply::Failed
    );
    let ran: Vec<String> = os.ran.borrow().iter().map(|a| a.join(" ")).collect();
    let at = ran.iter().position(|l| l.contains("mkinitcpio")).unwrap();
    let after: Vec<&str> = ran[at + 1..].iter().map(String::as_str).collect();
    // QA-STORE-3: before the swap only the scratch copy goes (its mounts first, 0.12.1
    // FIX-stab2); wolos-b, its entries and its image are as they were.
    assert_eq!(
        after,
        [
            "find /run/wolos/top -maxdepth 1 -name @wolos-tx -exec umount -R {} ;",
            "find /run/wolos/top -maxdepth 1 -name @wolos-tx -exec btrfs subvolume delete {} +",
            "rm -f /efi/wolos-b/verity.new /var/lib/wolos/update-building",
        ]
    );
    assert!(
        ran.iter()
            .all(|l| !l.contains("@wolos-a") && !l.contains("set-default"))
    );
    assert!(
        os.log
            .borrow()
            .iter()
            .any(|l| l
                .contains("failed; wolos-b is as it was, restore point wolos-a is unchanged")),
        "{:?}",
        os.log.borrow()
    );
    let mut said = None;
    handle_said(&up, ROOT, &Shared::default(), &os, &mut said);
    assert_eq!(said.as_deref(), Some(wol_core::update::pkg::SLOT_FAILED));
}

/// QA-100 STORE-11, ACC-23: Update Now with too little room on the system disk starts nothing and
/// says how much more it needs in GB (it used to fill the disk and read "check that you're
/// online"); with room, or with the free space unknown, it runs.
#[test]
fn no_room_sentence_with_gb() {
    let up = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
    let df = "df -B1 --output=avail /run/wolos/top";
    let full = Fake {
        probes: vec![
            ("bootctl list --json=short", SLOT_BOOTCTL),
            (df, "Avail\n1073741824\n"),
        ],
        ..slot_os()
    };
    let mut said = None;
    assert_eq!(
        handle_said(&up, ROOT, &Shared::default(), &full, &mut said),
        AdminReply::Failed
    );
    let said = said.unwrap();
    assert!(said.contains("about 3 GB more"), "{said}");
    assert!(
        said.contains("Settings > Storage") && !said.contains("online"),
        "{said}"
    );
    assert!(
        full.ran.borrow().is_empty(),
        "nothing ran: {:?}",
        full.ran.borrow()
    );
    let room = Fake {
        probes: vec![
            ("bootctl list --json=short", SLOT_BOOTCTL),
            (df, "Avail\n64424509440\n"),
        ],
        ..slot_os()
    };
    assert_eq!(handle(&up, ROOT, &Shared::default(), &room), AdminReply::Ok);
}
