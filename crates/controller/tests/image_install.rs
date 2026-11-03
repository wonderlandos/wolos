// SPDX-License-Identifier: GPL-2.0-or-later
//! The WolOS installer on Arch (ADR 0016, ADR 0013): `install-run <disk>` from the Install WolOS app
//! (or `wolos.autoinstall=<disk>` on the kernel command line) erases only the picked disk, makes a
//! GPT with an ESP and one btrfs partition, subvolumes `@wolos-a` `@home` `@var` (the first update
//! makes `@wolos-b`), copies the live root into slot a, and boots it with systemd-boot and boot
//! counting. Every tool is a fake that logs its arguments: none may ever name another disk.
#[path = "support/repo.rs"]
mod repo;
use repo::{read, repo};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};
fn lib(name: &str) -> PathBuf {
    repo(&format!("files/usr/lib/wolos/{name}"))
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-install-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("bin")).unwrap();
    d
}

/// The read-only `wol install` verbs scripts take disk facts from run for real in every fake
/// `wol` (audit3 R2B-16/19): install-check and install-run parse no lsblk output themselves.
const REAL_WOL: &str = "case \"$*\" in \"install facts\"* | \"install mounts\"* | \"install hostname\"*) exec @WOL@ \"$@\" ;; esac";

fn fake(d: &Path, name: &str, body: &str) {
    let p = d.join("bin").join(name);
    let log = d.join("log");
    let real = if name == "wol" {
        REAL_WOL.replace("@WOL@", env!("CARGO_BIN_EXE_wol")) + "\n"
    } else {
        String::new()
    };
    fs::write(
        &p,
        format!(
            "#!/bin/sh\necho \"{name} $*\" >> {}\n{real}{body}\n",
            log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
}

fn log(d: &Path) -> String {
    fs::read_to_string(d.join("log")).unwrap_or_default()
}

fn path(d: &Path) -> String {
    format!("{}:/usr/bin:/bin", d.join("bin").display())
}

/// lsblk -dnpbo NAME,SIZE,TYPE,RO: three disks, a loop device, a CD, zram; the live medium is sdb.
const LSBLK: &str = "/dev/nvme0n1 512110190592 disk 0\n/dev/sda 1000204886016 disk 0\n\
/dev/sdb 32010928128 disk 0\n/dev/loop0 900000000 loop 1\n/dev/sr0 1000000 rom 0\n\
/dev/zram0 8000000000 disk 0\n/dev/sdc 64000000000 disk 1\n";

/// `lsblk --json` for one disk, as `wol install facts|mounts` asks: its size from [`LSBLK`], one
/// partition per line of `$WOLOS_TEST_PARTS` (`<type> [<fstype>]`), one mounted volume per line
/// of `$WOLOS_TEST_MOUNTS` (`[SWAP]` for a swap partition).
const LSBLK_JSON: &str = r#"*--json*) for a; do l=$a; done
  z=$(printf '@LSBLK@' | while read -r n s _; do [ "$n" = "$l" ] && echo "$s"; done)
  printf '{"blockdevices":[{"path":"%s","size":%s,"children":[' "$l" "${z:-0}"
  i=0
  if [ -f "$WOLOS_TEST_PARTS" ]; then
    while read -r t f; do
      i=$((i + 1)); [ "$i" = 1 ] || printf ,
      printf '{"path":"%sp%s","parttype":"%s","fstype":"%s","mountpoints":[null]}' "$l" "$i" "$t" "$f"
    done < "$WOLOS_TEST_PARTS"
  fi
  if [ -f "$WOLOS_TEST_MOUNTS" ]; then
    while read -r m; do
      i=$((i + 1)); [ "$i" = 1 ] || printf ,
      printf '{"path":"%sp%s","fstype":"ext4","mountpoints":["%s"]}' "$l" "$i" "$m"
    done < "$WOLOS_TEST_MOUNTS"
  fi
  printf ']}]}\n' ;;"#;

fn disks_fakes(d: &Path) {
    fake(
        d,
        "lsblk",
        &format!(
            "case \"$*\" in\n{}\n*-dnpbo*) printf '{}' ;;\n*PKNAME*) echo /dev/sdb ;;\n*PARTN*) echo 1 ;;\nesac",
            LSBLK_JSON.replace("@LSBLK@", &LSBLK.replace('\n', "\\n")),
            LSBLK.replace('\n', "\\n")
        ),
    );
    fake(
        d,
        "findmnt",
        "case \"$*\" in\n*bootmnt*) echo /dev/sdb1 ;;\nesac",
    );
    // The live kernel command line: no archisolabel, so the label lookup finds nothing.
    fs::write(d.join("cmdline"), "quiet\n").unwrap();
    fs::create_dir_all(d.join("efivars")).unwrap();
}

#[test]
fn boot_medium_loop_cd_zram_and_read_only_disks_are_never_targets() {
    let d = scratch("disks");
    disks_fakes(&d);
    let o = Command::new("sh")
        .arg(lib("install-disks"))
        .env("PATH", path(&d))
        .env("WOLOS_CMDLINE", d.join("cmdline"))
        .output()
        .unwrap();
    assert!(o.status.success());
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        "/dev/nvme0n1\n/dev/sda\n"
    );
    // A disk the ISO's label names (a persistent live USB) or one with the live system on it is
    // the medium too, whatever bootmnt says.
    fs::write(d.join("cmdline"), "archisolabel=WOLOS_X quiet\n").unwrap();
    fake(&d, "blkid", "case \"$*\" in *-L*) echo /dev/sda3 ;; esac");
    fake(
        &d,
        "lsblk",
        &format!(
            "case \"$*\" in\n*-dnpbo*) printf '{}' ;;\n*PKNAME*sda3*) echo /dev/sda ;;\n*PKNAME*) echo /dev/sdb ;;\n\
             *MOUNTPOINTS*nvme0n1*) echo /run/archiso/cowspace ;;\nesac",
            LSBLK.replace('\n', "\\n")
        ),
    );
    let o = Command::new("sh")
        .arg(lib("install-disks"))
        .env("PATH", path(&d))
        .env("WOLOS_CMDLINE", d.join("cmdline"))
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        "",
        "nothing left to erase"
    );
}

fn check(d: &Path, disk: &str) -> (Option<i32>, String) {
    let o = Command::new("sh")
        .arg(lib("install-check"))
        .arg(disk)
        .env("PATH", path(d))
        .env("WOLOS_TEST_PARTS", d.join("parts"))
        .env("WOLOS_TEST_MOUNTS", d.join("mounts"))
        .env("WOLOS_EFIVARS", d.join("efivars"))
        .env("WOLOS_CMDLINE", d.join("cmdline"))
        .output()
        .unwrap();
    (
        o.status.code(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
    )
}

#[test]
fn check_names_other_systems_and_refuses_unsafe_disks() {
    let d = scratch("check");
    disks_fakes(&d);
    fake(&d, "wol", "echo firmware");
    // Windows (Microsoft basic data + reserved) and a locked Linux volume (LUKS on a basic data
    // partition: some Linux's, not unknown) on the big disk.
    fs::write(
        d.join("parts"),
        "e3c9e316-0b5c-4db8-817d-f92df00215ae\nebd0a0a2-b9e5-4433-87c0-68b6b72699c7 ntfs\n\
         ebd0a0a2-b9e5-4433-87c0-68b6b72699c7 crypto_LUKS\n",
    )
    .unwrap();
    fs::write(
        d.join("efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c"),
        [6u8, 0, 0, 0, 1],
    )
    .unwrap();
    let (code, out) = check(&d, "/dev/sda");
    assert_eq!(code, Some(0), "{out}");
    assert_eq!(
        out,
        "size 1000204886016\nos Windows\nos Linux\nbitlocker no\nboot firmware\nboot-off firmware\n"
    );
    // BitLocker named (audit3 R2B-9): its recovery key is asked for after a key change.
    fs::write(
        d.join("parts"),
        "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7 BitLocker\n",
    )
    .unwrap();
    assert_eq!(
        check(&d, "/dev/sda").1,
        "size 1000204886016\nos Windows\nbitlocker yes\nboot firmware\nboot-off firmware\n"
    );
    fs::write(
        d.join("parts"),
        "e3c9e316-0b5c-4db8-817d-f92df00215ae\nebd0a0a2-b9e5-4433-87c0-68b6b72699c7 ntfs\n\
         ebd0a0a2-b9e5-4433-87c0-68b6b72699c7 crypto_LUKS\n",
    )
    .unwrap();
    assert_eq!(
        check(&d, "/dev/sdb").1,
        "refuse not-listed\n",
        "boot medium"
    );
    assert_eq!(check(&d, "/dev/sdx").1, "refuse not-listed\n");
    assert_eq!(check(&d, "").0, Some(2));
    // Started without UEFI: the installed system could never boot here.
    fs::remove_dir_all(d.join("efivars")).unwrap();
    let (code, out) = check(&d, "/dev/sda");
    assert_eq!(code, Some(2));
    assert!(out.ends_with("refuse bios\n"), "{out}");
    fs::create_dir_all(d.join("efivars")).unwrap();
    // Anything mounted from the disk (a partition opened in Files, a btrfs subvolume, an opened
    // encrypted volume: lsblk lists them all): refused until install-run lets it go.
    fs::write(d.join("mounts"), "/run/media/live/usb\n").unwrap();
    let (code, out) = check(&d, "/dev/sda");
    assert_eq!(code, Some(2));
    assert!(out.ends_with("refuse in-use\n"), "{out}");
    assert!(
        !log(&d).contains("repart") && !log(&d).contains("mkfs"),
        "the check only reads"
    );
}

/// A full install against fakes: target root in a temp dir, mount/umount do nothing.
fn install(d: &Path, disk: &str) -> Output {
    install_with(d, disk, "off")
}

/// Tool fakes that leave the file a later step expects.
const OPENSSL: &str =
    "while [ $# -gt 0 ]; do case $1 in -keyout|-out) echo x > \"$2\"; shift ;; esac; shift; done";

const SBSIGN: &str =
    "for a; do case $a in --output=*) echo signed > \"${a#--output=}\" ;; esac; done";

const UKIFY: &str =
    "for a; do case $a in --output=*) echo image > \"${a#--output=}\" ;; esac; done";

fn install_with(d: &Path, disk: &str, unlock: &str) -> Output {
    disks_fakes(d);
    let t = d.join("target");
    fs::create_dir_all(&t).unwrap();
    for (n, body) in [
        ("systemd-repart", ""),
        ("udevadm", ""),
        ("mount", ""),
        ("umount", ""),
        ("swapoff", ""),
        ("btrfs", ""),
        ("sync", ""),
        ("cryptsetup", ""),
        (
            "wol",
            "case \"$*\" in\n\"encrypt decide\"*) [ \"$WOLOS_TEST_UNLOCK\" = refuse ] && exit 1; echo ${WOLOS_TEST_UNLOCK:-off} ;;\n\
             \"encrypt words\"*) echo \"rd.luks.name=$3=wolosroot root=/dev/mapper/wolosroot\" ;;\n\
             \"encrypt enroll\"*) [ -z \"$6\" ] || cat \"$6\" > \"$WOLOS_KEYDIR/../pass-seen\" ;;\n\
             \"boot detect\"*) echo off ;;\n\"boot key-screen\"*) echo no ;;\n\"boot note\"*) echo note ;;\nesac",
        ),
        ("userdel", ""),
        // Offline unless the test leaves a default route next to the fakes.
        (
            "ip",
            "case \"$*\" in *-4*) cat \"${0%/bin/ip}/route\" 2>/dev/null ;; esac",
        ),
        (
            "bootctl",
            "mkdir -p \"${2#--esp-path=}/EFI/systemd\" \"${2#--esp-path=}/EFI/BOOT\"",
        ),
        ("openssl", OPENSSL),
        ("sbsign-fake", SBSIGN),
        ("ukify", UKIFY),
        (
            "blkid",
            "case \"$*\" in *nvme0n1p1*) echo AAAA-BBBB ;; *) echo 1111-2222 ;; esac",
        ),
        (
            "unsquashfs",
            // The live root: modules with the kernel, live-only files, archiso bits.
            "R=$3\nmkdir -p $R/var/lib/pacman/local $R/usr/lib/modules/7.3.0-wolos $R/etc/wolos $R/etc/sysusers.d $R/etc/sudoers.d \
             $R/etc/mkinitcpio.conf.d $R/etc/mkinitcpio.d $R/boot $R/home/live\n\
             echo kernel > $R/usr/lib/modules/7.3.0-wolos/vmlinuz\necho linux-wolos > $R/usr/lib/modules/7.3.0-wolos/pkgbase\n\
             touch $R/etc/sysusers.d/wolos-live.conf $R/etc/sudoers.d/wolos-live $R/etc/wolos/login.json $R/etc/wolos-live \
             $R/etc/mkinitcpio.conf.d/archiso.conf\necho live > $R/etc/wolos/shell.json\necho pkg > $R/etc/wolos/shell.json.pacnew\n\
             echo 2026-10-20 > $R/etc/wolos/snapshot\nprintf '[options]\\nHoldPkg = pacman\\n' > $R/etc/pacman.conf",
        ),
        (
            "arch-chroot",
            "case \"$*\" in *mkinitcpio*) echo initrd > \"$1/boot/initramfs-linux-wolos.img\" ;; esac",
        ),
    ] {
        fake(d, n, body);
    }
    let mut c = Command::new("sh");
    // A passphrase file next to the fakes is the autoinstall's wolos.passphrase= file.
    if d.join("passphrase").exists() {
        c.env("WOLOS_PASSPHRASE_FILE", d.join("passphrase"));
    }
    c.arg(lib("install-run"))
        .arg(disk)
        .env("WOLOS_INSTALL_PATH", path(d))
        .env("WOLOS_INSTALL_ROOT", &t)
        .env("WOLOS_SOURCE_SFS", d.join("airootfs.sfs"))
        .env("WOLOS_TEST_PARTS", d.join("parts"))
        .env("WOLOS_TEST_MOUNTS", d.join("mounts"))
        .env("WOLOS_EFIVARS", d.join("efivars"))
        .env("WOLOS_NO_INHIBIT", "1")
        .env("WOLOS_TEST_UNLOCK", unlock)
        .env("WOLOS_KEYDIR", d.join("keys"))
        .env("WOLOS_SB_RUN", d.join("sbrun"))
        .env("WOLOS_TPM_DIR", d.join("no-tpm"))
        .env("WOLOS_BOOT_NOTE", d.join("note"))
        .env("WOLOS_SBSIGN", d.join("bin/sbsign-fake"))
        .env("WOLOS_INSTALL_LOG", d.join("install.log"))
        .env("WOLOS_RESULT_DIR", d.join("results"))
        .output()
        .unwrap()
}

#[test]
fn install_writes_slot_a_home_var_and_a_counted_boot_entry() {
    let d = scratch("full");
    let o = install(&d, "/dev/nvme0n1");
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(
        o.status.success(),
        "{out}{}{}",
        String::from_utf8_lossy(&o.stderr),
        log(&d)
    );
    let l = log(&d);
    // Only the picked disk and its partitions are ever named (read-only queries may name the
    // boot medium to leave it out).
    for line in l.lines().filter(|x| {
        !x.starts_with("lsblk ")
            && !x.starts_with("findmnt ")
            && !x.starts_with("wol install facts ")
            && !x.starts_with("wol install mounts ")
    }) {
        for other in ["/dev/sda", "/dev/sdb", "/dev/sdc"] {
            assert!(!line.contains(other), "touched {other}: {line}");
        }
    }
    // systemd-repart lays out the disk from declarative definitions: ESP + btrfs root with the
    // slot subvolumes, nothing hand-partitioned.
    let repart = l
        .lines()
        .find(|x| x.starts_with("systemd-repart "))
        .unwrap_or_else(|| panic!("no systemd-repart: {l}"));
    assert!(
        repart.contains("--empty=force") && repart.contains("--dry-run=no"),
        "{repart}"
    );
    assert!(repart.ends_with(" /dev/nvme0n1"), "{repart}");
    assert!(
        repart.contains("--definitions=") && repart.contains("repart.d"),
        "{repart}"
    );
    for gone in ["sgdisk", "mkfs.", "btrfs subvolume create", "partprobe"] {
        assert!(!l.contains(gone), "{gone} is systemd-repart's job: {l}");
    }
    assert!(l.contains("subvol=@wolos-a"), "{l}");
    assert!(l.contains("bootctl --esp-path="), "{l}");
    let t = d.join("target");
    // Sealed after everything is written: the digest goes next to the slot's kernel on the ESP.
    assert!(
        l.contains(&format!(
            "wol seal apply {} {}/efi/wolos-a/verity",
            t.display(),
            t.display()
        )),
        "{l}"
    );
    assert!(
        !l.contains("cryptsetup"),
        "plain install: no encryption: {l}"
    );
    // /etc stays writable on the read-only slot.
    let fstab0 = fs::read_to_string(t.join("etc/fstab")).unwrap();
    // The lower is the slot's own /etc, another one after each update (FIX-slot 0.12.1): the
    // line overlayfs takes a changed lower with, the one every slot build writes.
    assert!(
        fstab0.lines().any(|l| l == wol_core::update::ETC_FSTAB),
        "{fstab0}"
    );
    assert!(fstab0.contains(",index=off,metacopy=off,"), "{fstab0}");
    assert!(
        fstab0.contains(" /run/wolos/top btrfs subvolid=5,noauto,x-systemd.automount"),
        "{fstab0}"
    );
    // Live-only files are gone; the package's shell.json comes back.
    for gone in [
        "etc/sysusers.d/wolos-live.conf",
        "etc/sudoers.d/wolos-live",
        "etc/wolos/login.json",
        "etc/wolos-live",
        "etc/mkinitcpio.conf.d/archiso.conf",
        "home/live",
    ] {
        assert!(
            !t.join(gone).exists(),
            "{gone} left on the installed system"
        );
    }
    assert_eq!(
        fs::read_to_string(t.join("etc/wolos/shell.json")).unwrap(),
        "pkg\n"
    );
    assert_eq!(fs::read_to_string(t.join("etc/wolos/slot")).unwrap(), "a\n");
    // The package database moves from the shared @var into the slot (pacman.conf DBPath).
    assert!(t.join("usr/lib/pacman/local").is_dir() && !t.join("var/lib/pacman/local").exists());
    assert_eq!(
        fs::read_to_string(t.join("etc/pacman.conf")).unwrap(),
        "[options]\nDBPath = /usr/lib/pacman/\nHoldPkg = pacman\n"
    );
    let cmdline = fs::read_to_string(t.join("etc/kernel/cmdline")).unwrap();
    assert!(
        cmdline.starts_with("root=UUID=1111-2222 rootflags=subvol=@wolos-a ro quiet splash"),
        "{cmdline}"
    );
    assert!(cmdline.contains(" audit=0 "), "{cmdline}");
    let fstab = fs::read_to_string(t.join("etc/fstab")).unwrap();
    for want in [
        "UUID=1111-2222 /home btrfs subvol=/@home,",
        "UUID=1111-2222 /var btrfs subvol=/@var,",
        "UUID=AAAA-BBBB /efi vfat ",
    ] {
        assert!(fstab.contains(want), "fstab lacks {want}:\n{fstab}");
    }
    assert!(
        !fstab
            .lines()
            .any(|x| x.split_whitespace().nth(1) == Some("/")),
        "root comes from the boot entry"
    );
    // systemd-boot: slot a, three tries, quiet; kernel and initramfs next to it on the ESP.
    let esp = t.join("efi");
    let entry = fs::read_to_string(esp.join("loader/entries/wolos-a+3.conf")).unwrap();
    assert!(entry.contains("uki /EFI/wolos/wolos-a.efi"), "{entry}");
    assert!(
        !entry.contains("options "),
        "the image holds the command line: {entry}"
    );
    assert!(esp.join("EFI/wolos/wolos-a.efi").exists());
    let ukify = l.lines().find(|x| x.starts_with("ukify ")).unwrap();
    assert!(ukify.contains(cmdline.trim()), "{ukify}");
    assert!(ukify.contains("wolos.epoch=1"), "{ukify}");
    // The signature covers kernel, initramfs and command line; the seal's digest is in it.
    assert!(
        ukify.contains("--secureboot-private-key=") && ukify.contains("--pcr-private-key="),
        "{ukify}"
    );
    let loader = fs::read_to_string(esp.join("loader/loader.conf")).unwrap();
    assert!(
        !loader.contains("default") && loader.contains("timeout 0"),
        "a pinned default would keep booting a failed slot: {loader}"
    );
    let order = |s: &str| l.find(s).unwrap_or_else(|| panic!("{s}: {l}"));
    assert!(
        order("wol seal apply") < order("ukify "),
        "the image carries the digest"
    );
    assert_eq!(fs::read_to_string(d.join("note")).unwrap().trim(), "note");
    // Progress for the app, one line per step with its percent, ending in done.
    let stages: Vec<(&str, &str)> = out
        .lines()
        .filter_map(|x| x.strip_prefix("wolos-install: "))
        .map(|x| {
            let mut w = x.split(' ');
            (w.next().unwrap(), w.next().unwrap())
        })
        .collect();
    assert_eq!(
        stages,
        [
            ("prepare", "0"),
            ("disk", "5"),
            ("files", "15"),
            ("keys", "40"),
            ("initramfs", "70"),
            ("settings", "90"),
            ("seal", "92"),
            ("boot", "95"),
            ("done", "100"),
        ],
        "{out}"
    );
    // Nothing a tool says reaches stdout; the log keeps it, and the system keeps the log.
    assert!(
        out.lines().all(|x| x.starts_with("wolos-install: ")),
        "{out}"
    );
    let installed_log = fs::read_to_string(t.join("var/log/wolos-install.log")).unwrap();
    assert!(
        installed_log.contains("wolos-install: files 15"),
        "{installed_log}"
    );
    // The ESP's device goes to boot-setup, which asks it for the partition number.
    assert!(l.contains("lsblk -no PARTN /dev/nvme0n1p1"), "{l}");
    // Defaults without a plan: host name wolos, zram swap, no auto-login.
    assert_eq!(
        fs::read_to_string(t.join("etc/hostname")).unwrap(),
        "wolos\n"
    );
    assert!(!t.join("etc/systemd/zram-generator.conf").exists());
    assert!(!t.join("etc/wolos/login.json").exists());
    assert!(
        !l.contains("wol mirror gate") && !l.contains("pacman -Sy"),
        "offline: no mirror step to wait for: {l}"
    );
}

/// A stop says which stage and carries the tool's own words, and the log stays where the app
/// (or the person) can find it.
#[test]
fn a_stop_names_the_stage_and_the_real_error() {
    let d = scratch("stop");
    let o = install(&d, "/dev/nvme0n1");
    assert!(o.status.success());
    fake(
        &d,
        "unsquashfs",
        "echo 'FATAL ERROR: write failed: No space left on device' >&2; exit 1",
    );
    let o = rerun(&d, "/dev/nvme0n1");
    assert_eq!(o.status.code(), Some(1));
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    let last = out.lines().last().unwrap();
    assert!(
        last.starts_with("wolos-install: fail files could not copy the system: "),
        "{out}"
    );
    assert!(last.contains("No space left on device"), "{last}");
    assert!(!last.contains('|'), "one line, no pipes: {last}");
    assert!(
        fs::read_to_string(d.join("install.log"))
            .unwrap()
            .contains("No space left on device")
    );
    assert!(
        !log(&d).contains("mkinitcpio"),
        "nothing runs after the stop"
    );
}

/// install-run again on the fakes [`install_with`] left (some replaced since), on an empty target.
fn rerun(d: &Path, disk: &str) -> Output {
    let t = d.join("target");
    let _ = fs::remove_dir_all(&t);
    fs::create_dir_all(&t).unwrap();
    let _ = fs::remove_file(d.join("log"));
    Command::new("sh")
        .arg(lib("install-run"))
        .arg(disk)
        .env("WOLOS_INSTALL_PATH", path(d))
        .env("WOLOS_INSTALL_ROOT", &t)
        .env("WOLOS_SOURCE_SFS", d.join("airootfs.sfs"))
        .env("WOLOS_TEST_PARTS", d.join("parts"))
        .env("WOLOS_TEST_MOUNTS", d.join("mounts"))
        .env("WOLOS_EFIVARS", d.join("efivars"))
        .env("WOLOS_NO_INHIBIT", "1")
        .env("WOLOS_KEYDIR", d.join("keys"))
        .env("WOLOS_SB_RUN", d.join("sbrun"))
        .env("WOLOS_TPM_DIR", d.join("no-tpm"))
        .env("WOLOS_BOOT_NOTE", d.join("note"))
        .env("WOLOS_SBSIGN", d.join("bin/sbsign-fake"))
        .env("WOLOS_INSTALL_LOG", d.join("install.log"))
        .env("WOLOS_RESULT_DIR", d.join("results"))
        .output()
        .unwrap()
}

/// audit5 C9-4: a partition blkid has no UUID for yet (even when probed) stops the install; it
/// never writes `root=UUID=` with nothing after it into the boot words and fstab.
#[test]
fn a_partition_without_a_uuid_stops_the_install() {
    let d = scratch("no-uuid");
    let o = install(&d, "/dev/nvme0n1");
    assert!(o.status.success());
    fake(
        &d,
        "blkid",
        "case \"$*\" in *nvme0n1p1*) echo AAAA-BBBB ;; *nvme0n1p2*) ;; *) echo 1111-2222 ;; esac",
    );
    let o = rerun(&d, "/dev/nvme0n1");
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert_eq!(o.status.code(), Some(1), "{out}");
    assert!(
        out.lines().last().unwrap().starts_with(
            "wolos-install: fail disk could not read the new partitions (/dev/nvme0n1p2)"
        ),
        "{out}"
    );
    let l = log(&d);
    assert!(
        l.contains("blkid -p -s UUID -o value /dev/nvme0n1p2"),
        "asked again with a probe: {l}"
    );
    assert!(!l.contains("unsquashfs"), "nothing copied: {l}");
    assert!(!d.join("target/etc/kernel/cmdline").exists());
    // The probe answers: the install goes on with that UUID.
    fake(
        &d,
        "blkid",
        "case \"$*\" in *nvme0n1p1*) echo AAAA-BBBB ;; *-p*nvme0n1p2*) echo 3333-4444 ;; *nvme0n1p2*) ;; *) echo 1111-2222 ;; esac",
    );
    let o = rerun(&d, "/dev/nvme0n1");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    let cmdline = fs::read_to_string(d.join("target/etc/kernel/cmdline")).unwrap();
    assert!(cmdline.starts_with("root=UUID=3333-4444 "), "{cmdline}");
}

/// A plan with the fake `wol` answering as the real one does.
fn install_plan(d: &Path, wol: &str) -> (Output, PathBuf) {
    install_plan_on(d, wol, |_| {})
}

/// [`install_plan`], with `prep` run on the target and the plan folder just before install-run
/// (what an earlier system left on the reused startup partition, what sits next to the plan).
fn install_plan_on(d: &Path, wol: &str, prep: impl FnOnce(&Path)) -> (Output, PathBuf) {
    install_with(d, "/dev/nvme0n1", "off");
    let t = d.join("target");
    let _ = fs::remove_dir_all(&t);
    fs::create_dir_all(&t).unwrap();
    let _ = fs::remove_file(d.join("log"));
    let _ = fs::remove_dir_all(d.join("results"));
    fake(d, "wol", wol);
    fake(d, "chown", "");
    let dir = d.join("plan");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("plan.json"), "{}").unwrap();
    fs::write(dir.join("account-password"), "pw\n").unwrap();
    prep(d);
    let o = Command::new("sh")
        .arg(lib("install-run"))
        .arg("--plan")
        .arg(dir.join("plan.json"))
        .env("WOLOS_INSTALL_PATH", path(d))
        .env("WOLOS_INSTALL_ROOT", &t)
        .env("WOLOS_SOURCE_SFS", d.join("airootfs.sfs"))
        .env("WOLOS_TEST_PARTS", d.join("parts"))
        .env("WOLOS_TEST_MOUNTS", d.join("mounts"))
        .env("WOLOS_EFIVARS", d.join("efivars"))
        .env("WOLOS_NO_INHIBIT", "1")
        .env("WOLOS_KEYDIR", d.join("keys"))
        .env("WOLOS_SB_RUN", d.join("sbrun"))
        .env("WOLOS_TPM_DIR", d.join("no-tpm"))
        .env("WOLOS_BOOT_NOTE", d.join("note"))
        .env("WOLOS_SBSIGN", d.join("bin/sbsign-fake"))
        .env("WOLOS_RESULT_DIR", d.join("results"))
        .output()
        .unwrap();
    (o, t)
}

const WOL_ALONGSIDE: &str = "case \"$*\" in\n\
\"install info\"*) printf \"disk=/dev/nvme0n1\\nwipe=0\\nWOLOS_ENCRYPT=no\\n\" ;;\n\
\"encrypt decide\"*) echo off ;;\n\
\"boot detect\"*) echo off ;;\n\
\"boot key-screen\"*) echo no ;;\n\
\"install layout\"*) printf \"esp=/dev/nvme0n1p5\\nroot=/dev/nvme0n1p6\\nfsdev=/dev/nvme0n1p6\\nslots=two\\nothers=1\\nesp_reused=1\\nhome_dev=/dev/nvme0n1p3\\nextra_fstab=UUID=H /home ext4 defaults 0 2\\n\" ;;\n\
esac";

const WOL_TPM_PLAN: &str = "case \"$*\" in\n\
\"install info\"*) printf \"disk=/dev/nvme0n1\\nwipe=1\\nWOLOS_ENCRYPT=yes\\naccount=ann\\nWOLOS_HOSTNAME=anns-pc\\nWOLOS_AUTOLOGIN=yes\\nWOLOS_SWAP=swapfile:4\\n\" ;;\n\
\"encrypt decide\"*) echo tpm2 ;;\n\
\"encrypt words\"*) echo \"rd.luks.name=$3=wolosroot root=/dev/mapper/wolosroot\" ;;\n\
\"encrypt enroll\"*) cat \"$6\" > \"$WOLOS_KEYDIR/../fallback-seen\"; echo KEY-1 > \"$WOLOS_RECOVERY_OUT\" ;;\n\
\"boot detect\"*) echo off ;;\n\
\"boot key-screen\"*) echo no ;;\n\
\"boot note\"*) echo note ;;\n\
esac";

/// A TPM disk the chip later refuses (Secure Boot turned off changes PCR 7) still opens with the
/// account's own password, not only the 64-character recovery key.
#[test]
fn a_tpm_disk_gets_the_account_password_as_its_fallback() {
    let d = scratch("plan-tpm");
    let (o, t) = install_plan(&d, WOL_TPM_PLAN);
    let l = log(&d);
    assert!(
        o.status.success(),
        "{}{}{l}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let enroll = l
        .lines()
        .find(|x| x.starts_with("wol encrypt enroll "))
        .unwrap();
    assert!(
        enroll.contains(" tpm2 ") && enroll.ends_with("/plan/account-password"),
        "the app's file, read as typed by wol (encrypt::typed_secret): {enroll}"
    );
    assert!(
        !l.split_whitespace().any(|w| w == "pw"),
        "the password is a file, never an argument: {l}"
    );
    assert_eq!(
        fs::read_to_string(d.join("fallback-seen")).unwrap(),
        "pw\n",
        "install-run passes the file through; the one reading of it is wol's"
    );
    assert_eq!(
        fs::read_to_string(t.join("var/lib/wolos/disk-unlock-user")).unwrap(),
        "ann\n"
    );
    assert!(!d.join("keys").exists(), "the copy goes with the keys");
    // The plan's other choices: host name, auto-login for the account, a swap file instead of zram.
    assert_eq!(
        fs::read_to_string(t.join("etc/hostname")).unwrap(),
        "anns-pc\n"
    );
    assert_eq!(
        fs::read_to_string(t.join("etc/wolos/login.json")).unwrap(),
        "{\"autologin\":\"ann\"}\n"
    );
    assert!(
        l.contains(&format!(
            "btrfs filesystem mkswapfile --size 4g {}/var/swap/swapfile",
            t.display()
        )),
        "a btrfs swap file is made without copy-on-write: {l}"
    );
    let fstab = fs::read_to_string(t.join("etc/fstab")).unwrap();
    assert!(
        fstab.contains("/var/swap/swapfile none swap defaults 0 0"),
        "{fstab}"
    );
    assert!(
        fs::read_to_string(t.join("etc/systemd/zram-generator.conf"))
            .unwrap()
            .starts_with('#'),
        "zram off when a swap file was chosen"
    );
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    let (keys, encrypt) = (
        out.find("wolos-install: keys 40").unwrap(),
        out.find("wolos-install: encrypt 45").unwrap(),
    );
    assert!(
        keys < encrypt,
        "the keys come before the disk unlock: {out}"
    );
}

/// A plan the check refuses (a settings value, a reserved account name, a weak password) stops
/// before the erase, with the reason.
#[test]
fn a_plan_the_check_refuses_writes_nothing() {
    let d = scratch("plan-refused");
    let wol = WOL_ALONGSIDE.replace(
        "esac",
        "\"install check\"*) echo refuse ReservedName; exit 2 ;;\nesac",
    );
    let (o, _) = install_plan_on(&d, &wol, |d| {
        fs::write(d.join("plan/disk-password"), "disk pw\n").unwrap();
    });
    assert_eq!(o.status.code(), Some(2));
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(
        out.contains("wolos-install: fail prepare the disk is not what the plan was made for (refuse ReservedName)"),
        "{out}"
    );
    // audit5 C9-6: the plan's passwords go with the run, however it ends.
    for f in ["account-password", "disk-password"] {
        assert!(!d.join("plan").join(f).exists(), "{f} left after a refusal");
    }
    assert!(
        d.join("plan/plan.json").exists(),
        "the plan itself stays the caller's"
    );
    let l = log(&d);
    assert!(
        !l.contains("install layout") && !l.contains("unsquashfs"),
        "{l}"
    );
    assert!(
        !d.join("plan/install.log").exists(),
        "nothing is written into the caller's folder"
    );
    assert!(
        fs::read_to_string(d.join("results/log"))
            .unwrap()
            .contains("refuse ReservedName"),
        "the log is in the results folder"
    );
}

/// audit5 C9-6, S8-8: an install that stops after the erase leaves no password next to the plan.
#[test]
fn a_plan_that_stops_halfway_leaves_no_password_behind() {
    let d = scratch("plan-stops");
    let (o, _) = install_plan_on(&d, WOL_ALONGSIDE, |d| {
        fs::write(d.join("plan/disk-password"), "disk pw\n").unwrap();
        fake(d, "unsquashfs", "echo 'write failed' >&2; exit 1");
    });
    assert_eq!(o.status.code(), Some(1));
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(
        out.contains("wolos-install: fail files could not copy the system"),
        "{out}"
    );
    for f in ["account-password", "disk-password"] {
        assert!(!d.join("plan").join(f).exists(), "{f} left after a stop");
    }
}

#[test]
fn a_plan_keeps_its_hands_off_the_disk_layout_and_the_kept_home() {
    let d = scratch("plan");
    let (o, t) = install_plan(&d, WOL_ALONGSIDE);
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(
        o.status.success(),
        "{out}{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let l = log(&d);
    assert!(!l.contains("systemd-repart"), "only Erase uses it: {l}");
    let at = |w: &str| l.find(w).unwrap_or_else(|| panic!("{w} in {l}"));
    assert!(
        at("wol install check") < at("wol install layout"),
        "checked before any write"
    );
    // The kept /home is mounted only after everything that could write there, before the account.
    assert!(at("mount /dev/nvme0n1p3") > at("wol install settings"));
    // The keymap is in the initramfs the disk password is typed in (audit2 B1).
    assert!(
        at("wol install settings") < at("mkinitcpio -P"),
        "vconsole.conf before the initramfs: {l}"
    );
    assert!(at("mount /dev/nvme0n1p3") < at("wol install account"));
    assert!(
        !l.contains("subvol=@home"),
        "/home has its own partition: {l}"
    );
    let fstab = fs::read_to_string(t.join("etc/fstab")).unwrap();
    assert!(fstab.contains("UUID=H /home ext4 defaults 0 2"), "{fstab}");
    assert!(!fstab.contains("subvol=/@home"), "{fstab}");
    assert!(fstab.contains("subvol=/@var"), "{fstab}");
    assert!(l.contains("wol seal apply"), "two slots stay sealed");
    // Another system stays: the menu shows for a moment.
    let loader = fs::read_to_string(t.join("efi/loader/loader.conf")).unwrap();
    assert!(loader.contains("timeout 3"), "{loader}");
    assert!(
        !d.join("plan/account-password").exists(),
        "no password stays behind"
    );
}

/// audit2 B1: `mkinitcpio -P` baked the live default keymap (us) into the initramfs because the
/// plan's settings (etc/vconsole.conf) were written only at 90 %: a German or French disk
/// password could never be typed at the start prompt.
#[test]
fn the_keyboard_is_set_before_the_initramfs_is_built() {
    let run = read("files/usr/lib/wolos/install-run");
    let at = |w: &str| {
        run.lines()
            .position(|x| x.trim_start().starts_with(w))
            .unwrap_or_else(|| panic!("{w} not in install-run"))
    };
    assert!(at("wol install settings") < at("arch-chroot \"$T\" mkinitcpio -P"));
    assert!(at("arch-chroot \"$T\" mkinitcpio -P") < at("wol install account"));
    assert_eq!(
        run.matches("wol install settings").count(),
        1,
        "written once, before the initramfs"
    );
}

/// audit2 B4: `echo 'white rabbit' > passphrase` made the LUKS key "white rabbit\n", which no
/// one can type at the prompt. What reaches the disk is the passphrase without the line end.
#[test]
fn a_passphrase_file_ending_in_a_newline_opens_the_disk_with_what_is_typed() {
    for (name, file) in [("lf", "white rabbit\n"), ("crlf", "white rabbit\r\n")] {
        let d = scratch(&format!("pass-{name}"));
        fs::write(d.join("passphrase"), file).unwrap();
        let o = install_with(&d, "/dev/nvme0n1", "passphrase");
        assert!(
            o.status.success(),
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        // install-run passes the file through; `wol encrypt enroll` reads it as typed
        // (tests/install_secret.rs: what reaches the disk is "white rabbit").
        assert_eq!(
            fs::read_to_string(d.join("pass-seen")).unwrap(),
            file,
            "{name}"
        );
        let enroll = log(&d)
            .lines()
            .find(|x| x.starts_with("wol encrypt enroll "))
            .unwrap()
            .to_owned();
        assert!(enroll.ends_with("/passphrase"), "{enroll}");
        assert!(!d.join("keys").exists(), "the copy goes with the keys");
        assert_eq!(
            fs::read_to_string(d.join("passphrase")).unwrap(),
            file,
            "untouched"
        );
    }
}

/// audit2 B5: Erase Disk on a disk with Windows kept the 3-second menu for a system that is gone.
#[test]
fn an_erased_disk_shows_no_menu_for_the_systems_it_had() {
    let d = scratch("erase-windows");
    fs::write(
        d.join("parts"),
        "e3c9e316-0b5c-4db8-817d-f92df00215ae\nebd0a0a2-b9e5-4433-87c0-68b6b72699c7 ntfs\n",
    )
    .unwrap();
    let o = install(&d, "/dev/nvme0n1");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    let loader = fs::read_to_string(d.join("target/efi/loader/loader.conf")).unwrap();
    assert!(loader.contains("timeout 0"), "{loader}");
}

/// audit2 B11: the online kernel swap was `timeout 600 pacman -S`: a slow mirror killed it halfway,
/// leaving pacman's lock (and half a kernel) sealed into the slot. Now only the download is bounded;
/// a download cut short leaves no lock and the install goes on with the image's kernel.
#[test]
fn a_kernel_download_cut_short_leaves_no_lock_in_the_slot() {
    let d = scratch("v3-timeout");
    let t = d.join("target");
    fs::write(d.join("route"), "default via 10.0.2.2 dev eth0\n").unwrap();
    // Every bounded step "times out" the way timeout(1) does, after pacman took its lock.
    fake(
        &d,
        "timeout",
        "case \"$*\" in\n*'mirror kernel'*) echo linux-wolos-v3 ;;\n\
         *pacman*) mkdir -p \"$WOLOS_INSTALL_ROOT/usr/lib/pacman\"; : > \"$WOLOS_INSTALL_ROOT/usr/lib/pacman/db.lck\"; exit 124 ;;\n\
         *) exit 124 ;;\nesac",
    );
    let o = install_with(&d, "/dev/nvme0n1", "off");
    let l = log(&d);
    assert!(
        o.status.success(),
        "{}{l}",
        String::from_utf8_lossy(&o.stdout)
    );
    assert!(l.contains("pacman -Sw --noconfirm linux-wolos-v3"), "{l}");
    assert!(
        !l.contains("pacman -S --noconfirm linux-wolos-v3"),
        "no swap without the whole download: {l}"
    );
    assert!(
        !t.join("usr/lib/pacman/db.lck").exists(),
        "the lock is gone"
    );
    // Downloaded in full: the swap itself runs to the end, without a timeout.
    let _ = fs::remove_dir_all(&t);
    fs::create_dir_all(&t).unwrap();
    let _ = fs::remove_file(d.join("log"));
    fake(
        &d,
        "timeout",
        "case \"$*\" in\n*'mirror kernel'*) echo linux-wolos-v3 ;;\n*) exit 0 ;;\nesac",
    );
    let o = install_with(&d, "/dev/nvme0n1", "off");
    let l = log(&d);
    assert!(o.status.success(), "{l}");
    assert!(
        l.lines().any(|x| x.starts_with("arch-chroot ")
            && x.ends_with("pacman -S --noconfirm linux-wolos-v3")),
        "{l}"
    );
    assert!(
        !l.lines()
            .any(|x| x.starts_with("timeout ") && x.contains("pacman -S --noconfirm")),
        "the transaction is never killed: {l}"
    );
}

/// W2-J (msgs 1094/1216): the plan's Secure Boot choice reaches boot-setup, the ticked modules and
/// the media formats go to modules-install: installed online, queued for the first start offline
/// (the installer's "added at first start with a network").
#[test]
fn a_plan_carries_secure_boot_modules_and_media_formats() {
    let wol = WOL_ALONGSIDE.replace(
        "WOLOS_ENCRYPT=no\\n",
        "WOLOS_ENCRYPT=no\\nWOLOS_SECURE=no\\nmodules=dev\\nextra_packages=gst-libav lame\\n",
    );
    let registry = |d: &Path| {
        let r = d.join("target/usr/share/wolos/modules");
        fs::create_dir_all(&r).unwrap();
        fs::write(r.join("dev.json"), "{}").unwrap();
    };
    // Offline: the module and the media formats are queued for the first start, no pacman at all.
    let d = scratch("plan-modules-off");
    let (o, t) = install_plan_on(&d, &wol, registry);
    let l = log(&d);
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(
        o.status.success(),
        "{out}{}{l}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        l.contains("wol boot detect no"),
        "WOLOS_SECURE=no reaches boot-setup: {l}"
    );
    assert_eq!(
        fs::read_to_string(t.join("var/lib/wolos/modules-pending")).unwrap(),
        "dev\n"
    );
    assert_eq!(
        fs::read_to_string(t.join("var/lib/wolos/packages-pending")).unwrap(),
        "gst-libav\nlame\n"
    );
    assert!(out.contains("wolos-install: packages 80"), "{out}");
    assert!(!l.contains("pacman -S"), "offline: nothing downloaded: {l}");
    // Online: the module and the media formats are installed into the target.
    let d = scratch("plan-modules-on");
    let (o, t) = install_plan_on(&d, &wol, |d| {
        registry(d);
        fs::write(d.join("route"), "default via 10.0.2.2 dev eth0\n").unwrap();
    });
    let l = log(&d);
    assert!(
        o.status.success(),
        "{}{l}",
        String::from_utf8_lossy(&o.stderr)
    );
    let tt = t.display().to_string();
    assert!(
        l.contains(&format!(
            "arch-chroot {tt} pacman -S --noconfirm --needed wolos-module-dev"
        )),
        "{l}"
    );
    assert!(
        l.contains(&format!(
            "arch-chroot {tt} pacman -Sw --noconfirm --needed gst-libav lame"
        )) && l.contains(&format!(
            "arch-chroot {tt} pacman -S --noconfirm --needed gst-libav lame"
        )),
        "{l}"
    );
    for q in ["modules-pending", "packages-pending"] {
        assert!(
            !t.join("var/lib/wolos").join(q).exists(),
            "nothing left queued: {q}"
        );
    }
    // Media formats alone, offline: queued too, at their own step.
    let d = scratch("plan-formats-off");
    let (o, t) = install_plan_on(
        &d,
        &WOL_ALONGSIDE.replace(
            "WOLOS_ENCRYPT=no\\n",
            "WOLOS_ENCRYPT=no\\nextra_packages=gst-libav\\n",
        ),
        |_| {},
    );
    assert!(o.status.success(), "{}", log(&d));
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(out.contains("wolos-install: packages 85"), "{out}");
    assert_eq!(
        fs::read_to_string(t.join("var/lib/wolos/packages-pending")).unwrap(),
        "gst-libav\n"
    );
    assert!(!t.join("var/lib/wolos/modules-pending").exists());
    // Without the plan's keys: Secure Boot wanted, no modules, no media formats (today's install).
    let d = scratch("plan-modules-none");
    let (o, _) = install_plan(&d, WOL_ALONGSIDE);
    let l = log(&d);
    assert!(o.status.success(), "{l}");
    assert!(l.contains("wol boot detect yes"), "{l}");
    assert!(!String::from_utf8_lossy(&o.stdout).contains("wolos-install: packages"));
    let run = read("files/usr/lib/wolos/install-run");
    assert!(run.contains("WOLOS_SECURE=$WOLOS_SECURE sh \"$here/install-boot\""));
    let check = read("files/usr/lib/wolos/install-check");
    let (b, off) = (
        check.find("echo \"boot $(wol boot detect)\"").unwrap(),
        check
            .find("echo \"boot-off $(wol boot detect no)\"")
            .expect("install-check boot-off"),
    );
    assert!(b < off);
}

#[test]
fn a_layout_that_can_not_keep_two_copies_says_so_on_disk() {
    let d = scratch("plan-one");
    let wol = WOL_ALONGSIDE
        .replace("slots=two", "slots=one")
        .replace("home_dev=/dev/nvme0n1p3", "home_dev=")
        .replace("extra_fstab=UUID=H /home ext4 defaults 0 2", "extra_fstab=");
    let (o, t) = install_plan(&d, &wol);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let l = log(&d);
    assert!(!l.contains("wol seal"), "nothing to seal: {l}");
    assert!(t.join("etc/wolos/single-copy").exists());
    let cmd = fs::read_to_string(t.join("etc/kernel/cmdline")).unwrap();
    assert!(cmd.contains(" rw ") && !cmd.contains("subvol"), "{cmd}");
    let fstab = fs::read_to_string(t.join("etc/fstab")).unwrap();
    assert!(!fstab.contains("overlay"), "{fstab}");
}

#[test]
fn a_plan_the_disk_no_longer_matches_writes_nothing() {
    let d = scratch("plan-stale");
    let wol = WOL_ALONGSIDE.replace(
        "esac",
        "\"install check\"*) echo refuse DiskChanged; exit 2 ;;\nesac",
    );
    let (o, _) = install_plan(&d, &wol);
    assert_eq!(o.status.code(), Some(2));
    let l = log(&d);
    assert!(
        !l.contains("install layout") && !l.contains("mount "),
        "{l}"
    );
}

/// The partition definitions: an exact 1 GiB ESP (vfat) and a btrfs root filling the rest with
/// `@wolos-a` `@home` `@var` as subvolumes, made when the file system is made.
/// `install-run` hands repart one directory: each must describe the whole disk.
#[test]
fn an_encrypted_erase_still_makes_a_startup_partition() {
    for dir in ["repart.d", "repart.d-crypt"] {
        let path = lib(dir);
        let mut names: Vec<_> = fs::read_dir(&path)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        names.sort();
        let kinds: Vec<String> = names
            .iter()
            .map(|p| fs::read_to_string(p).unwrap())
            .collect();
        assert_eq!(kinds.len(), 2, "{dir}");
        assert!(
            kinds[0].contains("Type=esp"),
            "{dir}: the startup partition comes first"
        );
        assert!(kinds[1].contains("Type=root"), "{dir}: then the system");
    }
}

#[test]
fn repart_definitions_describe_esp_and_slot_root() {
    let esp = read("files/usr/lib/wolos/repart.d/10-esp.conf");
    for want in [
        "Type=esp",
        "Format=vfat",
        "Label=WOLOS",
        "SizeMinBytes=1G",
        "SizeMaxBytes=1G",
    ] {
        assert!(esp.lines().any(|x| x == want), "esp lacks {want}:\n{esp}");
    }
    let root = read("files/usr/lib/wolos/repart.d/20-root.conf");
    for want in [
        "Type=root",
        "Format=btrfs",
        "Label=WolOS",
        "MakeDirectories=/@wolos-a /@home /@var",
        "Subvolumes=/@wolos-a /@home /@var",
    ] {
        assert!(
            root.lines().any(|x| x == want),
            "root lacks {want}:\n{root}"
        );
    }
}

#[test]
fn a_refused_disk_is_left_exactly_as_it_was() {
    let d = scratch("refused");
    let o = install(&d, "/dev/sdb");
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stdout).contains("wolos-install: fail prepare not a disk"));
    let l = log(&d);
    for w in ["systemd-repart", "mkfs", "btrfs", "unsquashfs", "bootctl"] {
        assert!(!l.contains(w), "{w} ran on a refused disk: {l}");
    }
}

/// esp-sync: every kernel in the slot's /boot is copied next to its entries; linux-wolos is the
/// counted default entry, another kernel (linux-zen, a user's build) gets its own entry.
#[test]
fn esp_sync_gives_other_kernels_their_own_entry() {
    let d = scratch("esp");
    let root = d.join("root");
    let esp = d.join("efi");
    fs::create_dir_all(root.join("boot")).unwrap();
    fs::create_dir_all(root.join("etc/kernel")).unwrap();
    fs::create_dir_all(root.join("etc/wolos")).unwrap();
    fs::create_dir_all(esp.join("loader/entries")).unwrap();
    fs::write(root.join("etc/wolos/slot"), "b\n").unwrap();
    fs::write(
        root.join("etc/kernel/cmdline"),
        "root=UUID=x rootflags=subvol=@wolos-b rw\n",
    )
    .unwrap();
    for k in ["linux-wolos", "linux-zen"] {
        fs::write(root.join(format!("boot/vmlinuz-{k}")), k).unwrap();
        fs::write(root.join(format!("boot/initramfs-{k}.img")), k).unwrap();
    }
    // An earlier counted entry for slot b keeps its name (and counter) on a plain sync.
    fs::write(esp.join("loader/entries/wolos-b+1-2.conf"), "old").unwrap();
    fake(&d, "ukify", UKIFY);
    fs::create_dir_all(d.join("sb")).unwrap();
    let run = |args: &[&str]| {
        let st = Command::new("sh")
            .arg(lib("esp-sync"))
            .args(args)
            .env("WOLOS_ROOT", &root)
            .env("WOLOS_ESP", &esp)
            .env("WOLOS_SB_DIR", d.join("sb"))
            .env("PATH", path(&d))
            .status()
            .unwrap();
        assert!(st.success());
    };
    run(&[]);
    let e = esp.join("loader/entries");
    let main = fs::read_to_string(e.join("wolos-b+1-2.conf")).unwrap();
    assert!(main.contains("uki /EFI/wolos/wolos-b.efi") && main.contains("sort-key wolos\n"));
    let zen = fs::read_to_string(e.join("wolos-b-linux-zen.conf")).unwrap();
    assert!(
        zen.contains("uki /EFI/wolos/wolos-b-linux-zen.efi")
            && zen.contains("title WolOS (linux-zen)")
            && zen.contains("sort-key wolos-linux-zen"),
        "{zen}"
    );
    assert!(esp.join("EFI/wolos/wolos-b-linux-zen.efi").exists());
    assert!(main.contains("version 1-"), "first generation: {main}");
    // A plain sync keeps the generation.
    run(&[]);
    assert!(
        fs::read_to_string(e.join("wolos-b+1-2.conf"))
            .unwrap()
            .contains("version 1-")
    );
    // --new: a fresh slot, three tries again, one generation above every other entry: the
    // loader's sort is what boots it, and what falls back from it.
    fs::write(e.join("wolos-a.conf"), "title A\nversion 6-x\n").unwrap();
    run(&["--new"]);
    assert!(e.join("wolos-b+3.conf").exists() && !e.join("wolos-b+1-2.conf").exists());
    assert!(
        fs::read_to_string(e.join("wolos-b+3.conf"))
            .unwrap()
            .contains("version 7-")
    );
    // The slot's sealed digest and the tier's words end up inside the signed image.
    fs::create_dir_all(esp.join("wolos-b")).unwrap();
    fs::write(esp.join("wolos-b/verity"), "abcd").unwrap();
    fs::write(d.join("sb/cmdline"), "lockdown=integrity\n").unwrap();
    run(&[]);
    let u = log(&d);
    let last = u.lines().rfind(|x| x.contains("linux-wolos")).unwrap();
    assert!(
        last.contains("wolos.seal=abcd") && last.contains("lockdown=integrity"),
        "{last}"
    );
    let hook = read("files/usr/share/libalpm/hooks/95-wolos-esp.hook");
    assert!(
        hook.contains("Exec = /usr/lib/wolos/esp-sync")
            && hook.contains("Target = usr/lib/modules/*/vmlinuz")
    );
}

#[test]
fn installer_scripts_parse() {
    for s in [
        "install-run",
        "install-check",
        "install-disks",
        "esp-sync",
        "boot-keys",
        "boot-setup",
        "boot-enroll",
        "dkms-sign",
        "autoinstall",
        "wol-install",
    ] {
        let st = Command::new("sh").arg("-n").arg(lib(s)).status().unwrap();
        assert!(st.success(), "{s}");
        let mode = fs::metadata(lib(s)).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "{s} executable");
    }
}

/// Boot counting: a slot is good once the login window's supervisor runs (boot-complete.target);
/// systemd-bless-boot then drops the try counter. Three boots that never get there fall back.
#[test]
fn a_boot_that_reaches_the_login_window_blesses_the_slot() {
    let u = read("files/usr/lib/systemd/system/wolos-boot-ok.service");
    for want in [
        "Requires=wold.service",
        "After=wold.service",
        // audit3 R2B-8: bless after the key screen's unit rewrote the same entries.
        "After=wolos-mok-enroll.service",
        "Before=boot-complete.target",
        "RequiredBy=boot-complete.target",
    ] {
        assert!(u.lines().any(|l| l == want), "wolos-boot-ok lacks {want}");
    }
    let preset = read("files/usr/lib/systemd/system-preset/50-wolos.preset");
    assert!(preset.lines().any(|l| l == "enable wolos-boot-ok.service"));
}

/// Encrypt disk: LUKS2 definitions, the volume opened with a throw-away key, the real unlock
/// enrolled by `wol encrypt`, the boot entry names the mapper device and nothing secret is logged.
#[test]
fn encrypted_install_unlocks_through_the_initramfs() {
    let d = scratch("crypt");
    let o = install_with(&d, "/dev/nvme0n1", "tpm2");
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    let l = log(&d);
    assert!(
        o.status.success(),
        "{out}{}{l}",
        String::from_utf8_lossy(&o.stderr)
    );
    let repart = l
        .lines()
        .find(|x| x.starts_with("systemd-repart "))
        .unwrap();
    assert!(
        repart.contains("repart.d-crypt") && repart.contains("--key-file="),
        "{repart}"
    );
    assert!(l.contains("cryptsetup open --key-file="), "{l}");
    assert!(
        l.contains("mount -o subvol=@wolos-a,compress=zstd:1,noatime /dev/mapper/wolosroot"),
        "{l}"
    );
    assert!(l.contains("wol encrypt hooks "), "{l}");
    let enroll = l
        .lines()
        .find(|x| x.starts_with("wol encrypt enroll "))
        .unwrap();
    assert!(
        enroll.contains("/dev/nvme0n1p2 ") && enroll.ends_with(" tpm2"),
        "{enroll}"
    );
    let cmdline = fs::read_to_string(d.join("target/etc/kernel/cmdline")).unwrap();
    assert!(
        cmdline.starts_with("rd.luks.name=1111-2222=wolosroot root=/dev/mapper/wolosroot rootflags=subvol=@wolos-a ro"),
        "{cmdline}"
    );
    assert!(!d.join("keys").exists(), "the throw-away key is removed");
    assert!(out.contains("wolos-install: encrypt 45 "), "{out}");
    let crypt = read("files/usr/lib/wolos/repart.d-crypt/20-root.conf");
    // The slot is sealed: a machine id written at first boot would never reach the disk.
    assert!(!read("files/usr/lib/wolos/install-run").contains("uninitialized"));
    assert_eq!(
        read("files/usr/lib/wolos/repart.d-crypt/10-esp.conf"),
        read("files/usr/lib/wolos/repart.d/10-esp.conf"),
        "an encrypted disk starts from the same startup partition"
    );
    assert!(
        crypt.lines().any(|x| x == "Encrypt=key-file")
            && crypt.lines().any(|x| x == "Format=btrfs")
    );
}

#[test]
fn encryption_that_cannot_be_set_up_stops_before_any_write() {
    let d = scratch("crypt-refused");
    let o = install_with(&d, "/dev/nvme0n1", "refuse");
    assert_eq!(o.status.code(), Some(2));
    let l = log(&d);
    assert!(
        !l.contains("systemd-repart") && !l.contains("cryptsetup"),
        "{l}"
    );
}

/// audit3 R2B-1: install-run is a root helper the live user starts with sudo; it must never
/// create, replace or change the owner of anything in the plan's folder, which that user owns.
/// The log and the recovery key go to a folder it makes itself; `install-check --result` hands
/// them out, the key once.
#[test]
fn results_land_in_a_folder_the_installer_made_never_next_to_the_plan() {
    let d = scratch("results");
    fs::create_dir_all(d.join("elsewhere")).unwrap();
    fs::write(d.join("elsewhere/precious"), "keep\n").unwrap();
    let (o, _) = install_plan_on(&d, WOL_TPM_PLAN, |d| {
        // What a caller could leave at the result names: a file and a link to something of root's.
        fs::write(d.join("plan/install.log"), "decoy\n").unwrap();
        std::os::unix::fs::symlink(d.join("elsewhere/precious"), d.join("plan/recovery-key"))
            .unwrap();
        // And at the results folder's own name: a link to another folder.
        std::os::unix::fs::symlink(d.join("elsewhere"), d.join("results")).unwrap();
    });
    assert!(
        o.status.success(),
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(
        fs::read_to_string(d.join("plan/install.log")).unwrap(),
        "decoy\n"
    );
    assert!(
        fs::symlink_metadata(d.join("plan/recovery-key"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(d.join("elsewhere/precious")).unwrap(),
        "keep\n"
    );
    assert!(
        !d.join("elsewhere/recovery-key").exists(),
        "the link was not followed"
    );
    assert!(!log(&d).contains("chown"), "{}", log(&d));
    let r = d.join("results");
    let meta = fs::symlink_metadata(&r).unwrap();
    assert!(meta.is_dir(), "a folder of its own, not the link");
    assert_eq!(meta.permissions().mode() & 0o777, 0o700);
    assert_eq!(
        fs::read_to_string(r.join("recovery-key")).unwrap(),
        "KEY-1\n"
    );
    assert!(
        fs::read_to_string(r.join("log"))
            .unwrap()
            .contains("wolos-install: done 100")
    );
    let result = || {
        let o = Command::new("sh")
            .arg(lib("install-check"))
            .arg("--result")
            .env("WOLOS_RESULT_DIR", &r)
            .output()
            .unwrap();
        assert!(o.status.success());
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let first = result();
    assert!(first.starts_with("recovery-key KEY-1\nlog "), "{first}");
    assert!(first.contains("\nlog wolos-install: done 100"), "{first}");
    let second = result();
    assert!(!second.contains("recovery-key"), "shown once: {second}");
    assert!(second.starts_with("log "), "{second}");
}

/// audit3 R2B-12: Windows keeps the hardware clock in local time; a WolOS next to it does too.
#[test]
fn the_clock_is_kept_the_windows_way_while_windows_stays() {
    let d = scratch("adjtime");
    let wol = WOL_ALONGSIDE
        .replace("others=1", "others=1\\nwindows=1\\nbitlocker=1")
        .replace(
            "\"boot detect\"*) echo off ;;",
            "\"boot detect\"*) echo \"seen windows=$WOLOS_WINDOWS bitlocker=$WOLOS_BITLOCKER shared=$WOLOS_ESP_SHARED\" >> \"${0%/bin/wol}/log\"; echo off ;;",
        );
    let (o, t) = install_plan(&d, &wol);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    assert_eq!(
        fs::read_to_string(t.join("etc/adjtime")).unwrap(),
        "0.0 0 0\n0\nLOCAL\n"
    );
    // boot-setup hears it from the plan: Windows stays (tier), BitLocker (note), shared ESP.
    let l = log(&d);
    assert!(
        l.contains("seen windows=yes bitlocker=yes shared=yes"),
        "{l}"
    );
    let d = scratch("adjtime-erase");
    let o = install(&d, "/dev/nvme0n1");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    assert!(!d.join("target/etc/adjtime").exists(), "no Windows: UTC");
}

/// audit3 R2B-5: the startup files of a WolOS the plan replaces are cleared before ours go on;
/// any other WolOS's are still refused.
#[test]
fn an_old_wolos_on_the_reused_startup_partition_is_cleared_first() {
    let old = |d: &Path| {
        let e = d.join("target/efi");
        fs::create_dir_all(e.join("EFI/wolos")).unwrap();
        fs::create_dir_all(e.join("loader/entries")).unwrap();
        fs::create_dir_all(e.join("wolos-b")).unwrap();
        fs::write(e.join("EFI/wolos/wolos-a.efi"), "old").unwrap();
        fs::write(e.join("EFI/wolos/wolos-b.efi"), "old").unwrap();
        fs::write(e.join("loader/entries/wolos-b+1.conf"), "old").unwrap();
        fs::write(e.join("loader/entries/fedora.conf"), "theirs").unwrap();
    };
    let d = scratch("esp-wolos");
    let wol = WOL_ALONGSIDE.replace("others=1", "others=1\\nesp_wolos=1");
    let (o, t) = install_plan_on(&d, &wol, old);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    let e = t.join("efi");
    assert!(!e.join("EFI/wolos/wolos-b.efi").exists());
    assert!(!e.join("wolos-b").exists());
    assert!(!e.join("loader/entries/wolos-b+1.conf").exists());
    assert_eq!(
        fs::read_to_string(e.join("loader/entries/fedora.conf")).unwrap(),
        "theirs"
    );
    assert!(e.join("loader/entries/wolos-a+3.conf").exists());
    let d = scratch("esp-wolos-kept");
    let (o, _) = install_plan_on(&d, WOL_ALONGSIDE, old);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("another WolOS already starts"),
        "{}",
        String::from_utf8_lossy(&o.stdout)
    );
}

/// audit3 R2B-14 (STANDARDS 2: no eval): the install path runs no generated text as shell; what
/// `wol install info|layout` print is read as KEY=VALUE lines against a list of keys.
/// verify-gpu.sh is a manual diagnostic whose eval runs its own constant setup strings.
#[test]
fn the_installer_scripts_run_no_generated_shell() {
    let dir = repo("files/usr/lib/wolos");
    let mut checked = 0;
    for e in fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        if !p.is_file() || name == "verify-gpu.sh" {
            continue;
        }
        let text = fs::read_to_string(&p).unwrap();
        if !text.starts_with("#!") {
            continue;
        }
        checked += 1;
        for (n, line) in text.lines().enumerate() {
            let code = line.split(" #").next().unwrap_or("").trim_start();
            if code.starts_with('#') {
                continue;
            }
            let words: Vec<&str> = code
                .split(|c: char| c.is_whitespace() || c == ';' || c == '&' || c == '|')
                .collect();
            assert!(
                !words.contains(&"eval") && !words.contains(&"source"),
                "{name}:{}: {line}",
                n + 1
            );
            assert!(
                !code.starts_with(". ") && !code.contains("; . ") && !code.contains("&& . "),
                "{name}:{}: sources a file: {line}",
                n + 1
            );
        }
    }
    assert!(checked > 10, "{checked}");
    let run = read("files/usr/lib/wolos/install-run");
    assert!(
        run.contains("take info <<EOF") && run.contains("take layout < \"$KEYDIR/layout.env\"")
    );
}

/// audit3 R2B-17: sizes live in wol_core::install::layout; the scripts and repart definitions that
/// repeat them are checked against it.
#[test]
fn the_scripts_sizes_are_the_layouts() {
    use wol_core::install::layout::{ESP_BYTES, MIN_ROOT};
    let check = read("files/usr/lib/wolos/install-check");
    let min = check
        .lines()
        .find_map(|l| l.strip_prefix("MIN_BYTES="))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap();
    assert_eq!(min, MIN_ROOT);
    for dir in ["repart.d", "repart.d-crypt"] {
        let esp = read(&format!("files/usr/lib/wolos/{dir}/10-esp.conf"));
        let size = |k: &str| {
            esp.lines()
                .find_map(|l| l.strip_prefix(k))
                .and_then(|v| v.strip_suffix('G'))
                .and_then(|v| v.parse::<u64>().ok())
                .map(|g| g << 30)
        };
        assert_eq!(size("SizeMinBytes="), Some(ESP_BYTES), "{dir}");
        assert_eq!(size("SizeMaxBytes="), Some(ESP_BYTES), "{dir}");
    }
}

/// audit3 R2B-8: a kill of every process rooted at the target never reaches the live system.
#[test]
fn the_cleanup_of_stray_processes_never_targets_the_live_root() {
    let run = read("files/usr/lib/wolos/install-run");
    let body = &run[run.find("stray() {").unwrap()..];
    let body = &body[..body.find("\n}").unwrap()];
    let guard = body.find("[ \"${T%/}\" != \"\" ] || return 0").unwrap();
    assert!(guard < body.find("kill -9").unwrap(), "{body}");
    assert!(run.contains("rm -rf \"${KEYDIR:?}\""));
    assert!(run.contains("rm -rf \"${R:?}\""));
}

/// audit3 R2B-19: a swap partition on the picked disk is switched off, not "unmounted"; the live
/// medium's own mounts stay. Facts come from `wol install mounts`.
#[test]
fn what_is_in_use_on_the_disk_is_let_go_of_swap_too() {
    let d = scratch("in-use");
    fs::write(
        d.join("mounts"),
        "/run/media/live/usb\n[SWAP]\n/run/archiso/cowspace\n",
    )
    .unwrap();
    let o = install(&d, "/dev/nvme0n1");
    // The fakes do not really let go, so install-check still refuses: what matters is the order.
    assert_eq!(
        o.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&o.stdout)
    );
    let l = log(&d);
    assert!(l.contains("umount /run/media/live/usb"), "{l}");
    assert!(l.contains("swapoff /dev/nvme0n1p2"), "{l}");
    assert!(
        !l.contains("umount [SWAP]") && !l.contains("umount /run/archiso"),
        "{l}"
    );
    let run = read("files/usr/lib/wolos/install-run");
    let check = read("files/usr/lib/wolos/install-check");
    for (name, text) in [("install-run", &run), ("install-check", &check)] {
        assert!(!text.contains("lsblk"), "{name} reads lsblk itself");
        assert!(!text.contains("e3c9e316"), "{name} copies a partition type");
    }
    assert!(check.contains("wol install facts") && check.contains("wol install mounts"));
}

/// audit3 R2B-18: the computer name follows one rule (wol_core::settings::hostname), the one
/// Settings uses: an autoinstall's name is checked by `wol install hostname`.
#[test]
fn the_autoinstall_name_follows_the_settings_rule() {
    for (given, kept) in [
        ("anns-pc", "anns-pc"),
        ("Anns.PC", "wolos"),
        ("a;reboot", "wolos"),
    ] {
        let o = Command::new(env!("CARGO_BIN_EXE_wol"))
            .args(["install", "hostname", given])
            .output()
            .unwrap();
        assert!(o.status.success());
        assert_eq!(
            String::from_utf8_lossy(&o.stdout),
            format!("{kept}\n"),
            "{given}"
        );
    }
    assert!(
        read("files/usr/lib/wolos/install-run")
            .contains("wol install hostname \"$WOLOS_HOSTNAME\"")
    );
}

/// The first start (or the network-up hook) drains both queues: a module id and the media formats.
/// A download that fails keeps them queued; one that works empties the queues. A queued line that
/// is not a package name (an option, a path) is dropped, never handed to pacman.
#[test]
fn the_first_start_installs_the_queued_media_formats() {
    let d = scratch("drain-formats");
    let r = d.join("root");
    let w = r.join("var/lib/wolos");
    fs::create_dir_all(r.join("usr/share/wolos/modules")).unwrap();
    fs::write(r.join("usr/share/wolos/modules/dev.json"), "{}").unwrap();
    fs::create_dir_all(&w).unwrap();
    fs::write(w.join("modules-pending"), "dev\n").unwrap();
    fs::write(w.join("packages-pending"), "gst-libav\n--root=/x\nlame\n").unwrap();
    // pacman inside the root: the download fails while `offline` exists.
    fake(
        &d,
        "arch-chroot",
        &format!(
            "case \"$*\" in *-Sw*) [ ! -e {} ] || exit 1 ;; esac",
            d.join("offline").display()
        ),
    );
    fake(&d, "logger", "");
    let run = || {
        Command::new("sh")
            .arg(lib("modules-install"))
            .arg("--root")
            .arg(&r)
            .arg("--drain")
            .env("PATH", path(&d))
            .output()
            .unwrap()
    };
    fs::write(d.join("offline"), "").unwrap();
    let o = run();
    assert_eq!(o.status.code(), Some(1), "something stays queued");
    assert_eq!(
        fs::read_to_string(w.join("modules-pending")).unwrap(),
        "dev\n"
    );
    assert_eq!(
        fs::read_to_string(w.join("packages-pending")).unwrap(),
        "gst-libav\nlame\n",
        "kept, the bad line dropped"
    );
    assert!(
        !log(&d)
            .lines()
            .any(|x| x.starts_with("arch-chroot") && x.contains("--root=/x")),
        "{}",
        log(&d)
    );
    fs::remove_file(d.join("offline")).unwrap();
    let _ = fs::remove_file(d.join("log"));
    let o = run();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let l = log(&d);
    let rr = r.display().to_string();
    assert!(
        l.contains(&format!(
            "arch-chroot {rr} pacman -S --noconfirm --needed wolos-module-dev"
        )),
        "{l}"
    );
    assert!(
        l.contains(&format!(
            "arch-chroot {rr} pacman -S --noconfirm --needed gst-libav lame"
        )),
        "{l}"
    );
    assert!(!w.join("modules-pending").exists() && !w.join("packages-pending").exists());
    // A name that is not a package is refused before anything runs.
    let o = Command::new("sh")
        .arg(lib("modules-install"))
        .arg("--root")
        .arg(&r)
        .arg("--package")
        .arg("-Syu")
        .env("PATH", path(&d))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}
