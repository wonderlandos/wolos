// SPDX-License-Identifier: GPL-2.0-or-later
//! A/B slot builds (ADR 0016) as they run, not only as planned (FIX-slot, 0.12.1):
//! - the `/etc` overlay line: install-run writes `wol_core::update::ETC_FSTAB`, and the sed every
//!   slot build runs turns an installed system's older line into it (after a slot switch the old
//!   `index=on` line refused the upper with ESTALE and the system stopped in emergency mode);
//! - `etc-settle` copies metacopy-only files up through the live overlay before the new slot,
//!   which mounts with `metacopy=off`, has to read them;
//! - `pacman -U FILE` in the pacman wrapper: the file is copied into the copy (with a signature
//!   next to it) by the plan's own `mkdir`/`cp`/`find` steps, run here for real under a scratch
//!   folder, and the copy is a mount while pacman runs in it.
#[path = "support/repo.rs"]
mod repo;
use repo::{read, repo};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};
use wol_core::update::{self as up, Base, ETC_FSTAB, Step, TOP};

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-update-slot-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// The fstab of a two-slot install made by 0.12.0's install-run.
const FSTAB_012: &str = "\
# WolOS: / is the slot the boot entry names (rootflags=subvol=@wolos-a or @wolos-b).
UUID=1111 /home btrfs subvol=/@home,compress=zstd:1,noatime 0 0
UUID=1111 /var btrfs subvol=/@var,compress=zstd:1,noatime 0 0
UUID=1111 /run/wolos/top btrfs subvolid=5,noauto,x-systemd.automount,x-systemd.idle-timeout=60,compress=zstd:1,noatime 0 0
UUID=2222 /efi vfat umask=0077,shortname=winnt 0 2
# The slot is read-only: /etc stays writable, its changes live in the shared @var (they survive updates).
overlay /etc overlay lowerdir=/etc,upperdir=/var/lib/wolos/etc/upper,workdir=/var/lib/wolos/etc/work,x-systemd.requires-mounts-for=/var 0 0
";

/// A step as argv; the one file write (the new slot's `/etc/wolos/slot`) as `write PATH`.
fn argv(s: &Step) -> Vec<String> {
    match s {
        Step::Run(a) => a.clone(),
        Step::Write { path, .. } => vec!["write".into(), path.clone()],
    }
}

#[test]
fn the_installer_writes_the_etc_line_the_updates_keep() {
    let run = read("files/usr/lib/wolos/install-run");
    let line = run
        .lines()
        .find(|l| l.trim_start().starts_with("echo \"overlay /etc overlay "))
        .expect("install-run writes the /etc overlay line");
    let written = line
        .trim()
        .trim_start_matches("echo \"")
        .trim_end_matches('"');
    assert_eq!(written, ETC_FSTAB);
}

#[test]
fn a_slot_build_turns_the_old_etc_line_into_the_new_one() {
    let d = scratch("fstab");
    let steps = up::plan_from("wolos-b", Base::Running { replace: true }, None, None, &[]).unwrap();
    let sed = steps
        .iter()
        .map(argv)
        .find(|a| a[0] == "sed" && a.last().unwrap().ends_with("/etc/fstab"))
        .expect("the plan rewrites the new slot's fstab");
    // Built in the scratch copy (QA-STORE-3), which becomes wolos-b at the swap.
    assert_eq!(sed.last().unwrap(), &format!("{TOP}/@wolos-tx/etc/fstab"));
    let fstab = d.join("fstab");
    fs::write(&fstab, FSTAB_012).unwrap();
    let mut a = sed.clone();
    *a.last_mut().unwrap() = fstab.display().to_string();
    for round in 0..2 {
        let st = Command::new(&a[0]).args(&a[1..]).status().unwrap();
        assert!(st.success(), "round {round}");
        let got = fs::read_to_string(&fstab).unwrap();
        let want: String = FSTAB_012
            .lines()
            .map(|l| {
                if l.starts_with("overlay /etc ") {
                    ETC_FSTAB
                } else {
                    l
                }
            })
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(
            got, want,
            "round {round}: only the overlay line changes, once"
        );
    }
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn etc_settle_copies_metacopy_files_up_through_the_overlay_and_changes_nothing_in_them() {
    let d = scratch("settle");
    let (up_dir, etc, bin) = (d.join("upper"), d.join("etc"), d.join("bin"));
    for p in [&up_dir.join("ssh"), &etc.join("ssh"), &bin] {
        fs::create_dir_all(p).unwrap();
    }
    fs::write(etc.join("ssh/sshd_config"), "Port 22\n").unwrap();
    fs::write(etc.join("hosts"), "127.0.0.1 localhost\n").unwrap();
    std::os::unix::fs::symlink("hosts", etc.join("link")).unwrap();
    // What getfattr -R prints for the two metacopy files (and one the merged /etc no longer has).
    let fake = format!(
        "#!/bin/sh\necho \"getfattr $*\" >> {log}\nprintf '# file: %s\\ntrusted.overlay.metacopy\\n\\n' \
{u}/ssh/sshd_config {u}/hosts {u}/gone {u}/link\n",
        log = d.join("log").display(),
        u = up_dir.display()
    );
    let g = bin.join("getfattr");
    fs::write(&g, fake).unwrap();
    fs::set_permissions(&g, fs::Permissions::from_mode(0o755)).unwrap();
    let out = Command::new(repo("files/usr/lib/wolos/etc-settle"))
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("WOLOS_ETC_UPPER", &up_dir)
        .env("WOLOS_ETC", &etc)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    let log = fs::read_to_string(d.join("log")).unwrap();
    assert!(
        log.contains("-R -P --absolute-names -m ^trusted\\.overlay\\.metacopy$ --"),
        "{log}"
    );
    for f in ["ssh/sshd_config", "hosts"] {
        assert!(
            err.contains(&format!("copied up {}", etc.join(f).display())),
            "{f}: {err}"
        );
    }
    assert!(!err.contains("gone") && !err.contains("link"), "{err}");
    assert_eq!(
        fs::read_to_string(etc.join("ssh/sshd_config")).unwrap(),
        "Port 22\n"
    );
    assert_eq!(
        fs::read_to_string(etc.join("hosts")).unwrap(),
        "127.0.0.1 localhost\n"
    );
    // No upper (one copy, a chroot): nothing to do.
    let none = Command::new(repo("files/usr/lib/wolos/etc-settle"))
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("WOLOS_ETC_UPPER", d.join("nope"))
        .env("WOLOS_ETC", &etc)
        .status()
        .unwrap();
    assert!(none.success());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn pacman_u_files_reach_the_copy_with_their_signature() {
    let d = scratch("stage");
    let (home, var) = (d.join("home/a"), d.join("var/cache"));
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&var).unwrap();
    let k = var.join("linux-wolos-7.3.1-1-x86_64.pkg.tar.zst");
    let h = home.join("hello[1].pkg.tar.zst");
    fs::write(&k, "kernel").unwrap();
    fs::write(
        var.join("linux-wolos-7.3.1-1-x86_64.pkg.tar.zst.sig"),
        "sig",
    )
    .unwrap();
    fs::write(&h, "hello").unwrap();
    fs::write(home.join("hello1.pkg.tar.zst.sig"), "not this one").unwrap();
    let args: Vec<String> = [
        "-U",
        "--noconfirm",
        &k.display().to_string(),
        &h.display().to_string(),
    ]
    .map(String::from)
    .to_vec();
    let steps = up::pkg::plan("wolos-b", Base::Pending, &args, "current", None).unwrap();
    let work = format!("{TOP}/@wolos-tx");
    let root = d.join("tx");
    // Run the staging steps (everything before the bind) under the scratch folder.
    let moved = |a: &str| a.replace(&work, &root.display().to_string());
    // Up to pacman, which runs in the copy under `unshare` (RH-192).
    let bind = steps
        .iter()
        .position(|s| {
            s.to_string()
                .starts_with("unshare --mount --propagation private --fork --kill-child=SIGKILL wol update chroot ")
        })
        .unwrap();
    let mut ran = 0;
    for s in &steps[..bind] {
        let a: Vec<String> = argv(s).iter().map(|x| moved(x)).collect();
        // Only the staging of the files: the other steps before the bind act on the real
        // /run/wolos/top (the scratch clean-up) or the copy's /etc.
        if !matches!(a[0].as_str(), "mkdir" | "cp" | "find") || !a.join(" ").contains("wolos-pkg") {
            continue;
        }
        let st = Command::new(&a[0]).args(&a[1..]).status().unwrap();
        assert!(st.success(), "{}", a.join(" "));
        ran += 1;
    }
    assert_eq!(ran, 5, "mkdir, then cp and find for each file");
    let st = root.join("var/tmp/wolos-pkg");
    assert_eq!(
        fs::read_to_string(st.join("0/linux-wolos-7.3.1-1-x86_64.pkg.tar.zst")).unwrap(),
        "kernel"
    );
    assert_eq!(
        fs::read_to_string(st.join("0/linux-wolos-7.3.1-1-x86_64.pkg.tar.zst.sig")).unwrap(),
        "sig"
    );
    assert_eq!(
        fs::read_to_string(st.join("1/hello[1].pkg.tar.zst")).unwrap(),
        "hello"
    );
    assert!(
        !st.join("1/hello1.pkg.tar.zst.sig").exists(),
        "a glob never matches another file"
    );
    let pacman = steps[bind].to_string();
    assert_eq!(
        pacman,
        format!(
            "unshare --mount --propagation private --fork --kill-child=SIGKILL wol update chroot {work} -- /usr/bin/pacman -U --noconfirm \
/var/tmp/wolos-pkg/0/linux-wolos-7.3.1-1-x86_64.pkg.tar.zst /var/tmp/wolos-pkg/1/hello[1].pkg.tar.zst"
        )
    );
    // pacman's paths are the staged files, seen from inside the copy.
    for p in pacman
        .split_whitespace()
        .filter(|p| p.starts_with("/var/tmp/"))
    {
        assert!(root.join(p.trim_start_matches('/')).is_file(), "{p}");
    }
    let _ = fs::remove_dir_all(&d);
}
