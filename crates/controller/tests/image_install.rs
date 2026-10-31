//! The WolOS installer on Arch (ADR 0016, ADR 0013): `install-run <disk>` from the Install WolOS app
//! (or `wolos.autoinstall=<disk>` on the kernel command line) erases only the picked disk, makes a
//! GPT with an ESP and one btrfs partition, subvolumes `@wolos-a` `@home` `@var` (the first update
//! makes `@wolos-b`), copies the live root into slot a, and boots it with systemd-boot and boot
//! counting. Every tool is a fake that logs its arguments: none may ever name another disk.
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}
fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}
fn lib(name: &str) -> PathBuf {
    repo(&format!("image/files/usr/lib/wolos/{name}"))
}
fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-install-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("bin")).unwrap();
    d
}
fn fake(d: &Path, name: &str, body: &str) {
    let p = d.join("bin").join(name);
    let log = d.join("log");
    fs::write(
        &p,
        format!(
            "#!/bin/sh\necho \"{name} $*\" >> {}\n{body}\n",
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

fn disks_fakes(d: &Path) {
    fake(
        d,
        "lsblk",
        &format!(
            "case \"$*\" in\n*-dnpbo*) printf '{}' ;;\n*PKNAME*) echo /dev/sdb ;;\n\
             *PARTTYPE*) cat \"$WOLOS_TEST_PARTS\" 2>/dev/null ;;\nesac",
            LSBLK.replace('\n', "\\n")
        ),
    );
    fake(
        d,
        "findmnt",
        "case \"$*\" in\n*bootmnt*) echo /dev/sdb1 ;;\n*SOURCE*) cat \"$WOLOS_TEST_MOUNTS\" 2>/dev/null ;;\nesac",
    );
}

#[test]
fn boot_medium_loop_cd_zram_and_read_only_disks_are_never_targets() {
    let d = scratch("disks");
    disks_fakes(&d);
    let o = Command::new("sh")
        .arg(lib("install-disks"))
        .env("PATH", path(&d))
        .output()
        .unwrap();
    assert!(o.status.success());
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        "/dev/nvme0n1\n/dev/sda\n"
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
    // Windows (Microsoft basic data + reserved) and Linux partitions on the big disk.
    fs::write(
        d.join("parts"),
        "e3c9e316-0b5c-4db8-817d-f92df00215ae\nebd0a0a2-b9e5-4433-87c0-68b6b72699c7 ntfs\n\
         0fc63daf-8483-4772-8e79-3d69d8477de4 ext4\n",
    )
    .unwrap();
    fs::create_dir_all(d.join("efivars")).unwrap();
    fs::write(
        d.join("efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c"),
        [6u8, 0, 0, 0, 1],
    )
    .unwrap();
    let (code, out) = check(&d, "/dev/sda");
    assert_eq!(code, Some(0), "{out}");
    assert_eq!(
        out,
        "size 1000204886016\nos Windows\nos Linux\nsecureboot on\n"
    );
    assert_eq!(
        check(&d, "/dev/sdb").1,
        "refuse not-listed\n",
        "boot medium"
    );
    assert_eq!(check(&d, "/dev/sdx").1, "refuse not-listed\n");
    assert_eq!(check(&d, "").0, Some(2));
    // A mounted partition of the disk (opened in Files): refused until install-run lets it go.
    fs::write(d.join("mounts"), "/dev/sda2\n").unwrap();
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

fn install_with(d: &Path, disk: &str, unlock: &str) -> Output {
    disks_fakes(d);
    let t = d.join("target");
    fs::create_dir_all(&t).unwrap();
    for (n, body) in [
        ("systemd-repart", ""),
        ("udevadm", ""),
        ("mount", ""),
        ("umount", ""),
        ("btrfs", ""),
        ("sync", ""),
        ("cryptsetup", ""),
        (
            "wol",
            "case \"$*\" in\n\"encrypt decide\"*) [ \"$WOLOS_TEST_UNLOCK\" = refuse ] && exit 1; echo ${WOLOS_TEST_UNLOCK:-off} ;;\n\
             \"encrypt words\"*) echo \"rd.luks.name=$3=wolosroot root=/dev/mapper/wolosroot\" ;;\nesac",
        ),
        ("userdel", ""),
        ("bootctl", ""),
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
        .env("WOLOS_TEST_UNLOCK", unlock)
        .env("WOLOS_KEYDIR", d.join("keys"))
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
    for line in l
        .lines()
        .filter(|x| !x.starts_with("lsblk ") && !x.starts_with("findmnt "))
    {
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
    assert!(
        fstab0.contains("overlay /etc overlay lowerdir=/etc,upperdir=/var/lib/wolos/etc/upper"),
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
    assert!(
        entry.contains("linux /wolos-a/vmlinuz-linux-wolos"),
        "{entry}"
    );
    assert!(
        entry.contains("initrd /wolos-a/initramfs-linux-wolos.img"),
        "{entry}"
    );
    assert!(
        entry.contains(&format!("options {}", cmdline.trim())),
        "{entry}"
    );
    assert!(esp.join("wolos-a/vmlinuz-linux-wolos").exists());
    assert!(esp.join("wolos-a/initramfs-linux-wolos.img").exists());
    let loader = fs::read_to_string(esp.join("loader/loader.conf")).unwrap();
    assert!(
        loader.contains("default wolos-a.conf") && loader.contains("timeout 0"),
        "{loader}"
    );
    // Progress for the app, one line per step, ending in done.
    let stages: Vec<&str> = out
        .lines()
        .filter_map(|x| x.strip_prefix("wolos-install: "))
        .map(|x| x.split(' ').next().unwrap())
        .collect();
    assert_eq!(
        stages,
        [
            "prepare", "disk", "files", "boot", "settings", "finish", "done"
        ],
        "{out}"
    );
}

/// A plan with the fake `wol` answering as the real one does.
fn install_plan(d: &Path, wol: &str) -> (Output, PathBuf) {
    install_with(d, "/dev/nvme0n1", "off");
    let t = d.join("target");
    let _ = fs::remove_dir_all(&t);
    fs::create_dir_all(&t).unwrap();
    let _ = fs::remove_file(d.join("log"));
    fake(d, "wol", wol);
    fake(d, "chown", "");
    let dir = d.join("plan");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("plan.json"), "{}").unwrap();
    fs::write(dir.join("account-password"), "pw\n").unwrap();
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
        .output()
        .unwrap();
    (o, t)
}

const WOL_ALONGSIDE: &str = "case \"$*\" in\n\
\"install info\"*) printf \"disk=/dev/nvme0n1\\nwipe=0\\nWOLOS_ENCRYPT=no\\n\" ;;\n\
\"encrypt decide\"*) echo off ;;\n\
\"install layout\"*) printf \"esp=/dev/nvme0n1p5\\nroot=/dev/nvme0n1p6\\nfsdev=/dev/nvme0n1p6\\nslots=two\\nothers=1\\nhome_dev=/dev/nvme0n1p3\\nextra_fstab='UUID=H /home ext4 defaults 0 2'\\n\" ;;\n\
esac";

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

#[test]
fn a_layout_that_can_not_keep_two_copies_says_so_on_disk() {
    let d = scratch("plan-one");
    let wol = WOL_ALONGSIDE
        .replace("slots=two", "slots=one")
        .replace("home_dev=/dev/nvme0n1p3", "home_dev=")
        .replace(
            "extra_fstab='UUID=H /home ext4 defaults 0 2'",
            "extra_fstab=",
        );
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
#[test]
fn repart_definitions_describe_esp_and_slot_root() {
    let esp = read("image/files/usr/lib/wolos/repart.d/10-esp.conf");
    for want in [
        "Type=esp",
        "Format=vfat",
        "Label=WOLOS",
        "SizeMinBytes=1G",
        "SizeMaxBytes=1G",
    ] {
        assert!(esp.lines().any(|x| x == want), "esp lacks {want}:\n{esp}");
    }
    let root = read("image/files/usr/lib/wolos/repart.d/20-root.conf");
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
    assert!(String::from_utf8_lossy(&o.stdout).contains("wolos-install: failed 0 not a disk"));
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
    let run = |args: &[&str]| {
        let st = Command::new("sh")
            .arg(lib("esp-sync"))
            .args(args)
            .env("WOLOS_ROOT", &root)
            .env("WOLOS_ESP", &esp)
            .status()
            .unwrap();
        assert!(st.success());
    };
    run(&[]);
    let e = esp.join("loader/entries");
    let main = fs::read_to_string(e.join("wolos-b+1-2.conf")).unwrap();
    assert!(
        main.contains("linux /wolos-b/vmlinuz-linux-wolos") && main.contains("sort-key wolos-b")
    );
    let zen = fs::read_to_string(e.join("wolos-b-linux-zen.conf")).unwrap();
    assert!(
        zen.contains("linux /wolos-b/vmlinuz-linux-zen") && zen.contains("title WolOS (linux-zen)")
    );
    assert!(esp.join("wolos-b/initramfs-linux-zen.img").exists());
    // --new: a fresh slot, three tries again.
    run(&["--new"]);
    assert!(e.join("wolos-b+3.conf").exists() && !e.join("wolos-b+1-2.conf").exists());
    let hook = read("image/files/usr/share/libalpm/hooks/95-wolos-esp.hook");
    assert!(
        hook.contains("Exec = /usr/lib/wolos/esp-sync")
            && hook.contains("Target = usr/lib/modules/*/vmlinuz")
    );
}

/// The pacman database lives in the slot (/usr), not in the shared @var: each slot knows its own
/// packages. The live root keeps pacman's default (/var/lib/pacman, where pacstrap writes, and
/// pacman resolves DBPath on the build host); install-run moves it and names it in pacman.conf.
#[test]
fn pacman_database_stays_in_the_slot() {
    assert!(read("image/arch/pacman.conf").contains("\nDBPath = /usr/lib/pacman/\n"));
    assert!(!read("image/arch/airootfs/etc/pacman.conf").contains("\nDBPath"));
    assert!(read("image/files/usr/lib/wolos/install-run").contains("DBPath = /usr/lib/pacman/"));
}

/// Unattended VM install: `wolos.autoinstall=<disk>` on the live command line.
#[test]
fn autoinstall_from_the_kernel_command_line() {
    let u = read("image/arch/airootfs/etc/systemd/system/wolos-autoinstall.service");
    assert!(u.contains("ConditionKernelCommandLine=wolos.autoinstall"));
    assert!(u.contains("ExecStart=/usr/lib/wolos/autoinstall"));
    assert!(repo("image/arch/airootfs/etc/systemd/system/multi-user.target.wants/wolos-autoinstall.service")
        .symlink_metadata()
        .is_ok());
    let d = scratch("auto");
    fake(&d, "systemctl", "");
    fake(&d, "install-run", "");
    fs::write(
        d.join("cmdline"),
        "quiet splash wolos.autoinstall=/dev/vda console=ttyS0\n",
    )
    .unwrap();
    let st = Command::new("sh")
        .arg(repo("image/files/usr/lib/wolos/autoinstall"))
        .env("PATH", path(&d))
        .env("WOLOS_CMDLINE", d.join("cmdline"))
        .env("WOLOS_INSTALL_RUN", d.join("bin/install-run"))
        .status()
        .unwrap();
    assert!(st.success());
    assert_eq!(log(&d), "install-run /dev/vda\nsystemctl poweroff\n");
}

/// wolos-update-slot: a thin wrapper; the update itself (snapshot the running root into the other
/// slot, repoint pacman, pacman -Syu, initramfs + counted entry, next boot) is `wol update apply`.
#[test]
fn update_slot_is_wol_update_apply() {
    let d = scratch("upd");
    fake(&d, "wol", "");
    fake(&d, "findmnt", "echo rw,relatime,subvol=/@wolos-a");
    let st = Command::new("sh")
        .arg(repo("image/arch/wolos-update-slot"))
        .arg("2026-11-20")
        .env("PATH", path(&d))
        .status()
        .unwrap();
    assert!(st.success());
    assert_eq!(
        log(&d),
        "findmnt -no OPTIONS /\nwol update apply @wolos-b 2026/11/20 --yes\n"
    );
    let w = read("image/repo/wolos-system/PKGBUILD");
    assert!(w.contains("image/arch/wolos-update-slot"));
}

#[test]
fn installer_scripts_parse() {
    for s in [
        "install-run",
        "install-check",
        "install-disks",
        "esp-sync",
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
    let u = read("image/files/usr/lib/systemd/system/wolos-boot-ok.service");
    for want in [
        "Requires=wold.service",
        "After=wold.service",
        "Before=boot-complete.target",
        "RequiredBy=boot-complete.target",
    ] {
        assert!(u.lines().any(|l| l == want), "wolos-boot-ok lacks {want}");
    }
    let preset = read("image/files/usr/lib/systemd/system-preset/50-wolos.preset");
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
    let crypt = read("image/files/usr/lib/wolos/repart.d-crypt/20-root.conf");
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
