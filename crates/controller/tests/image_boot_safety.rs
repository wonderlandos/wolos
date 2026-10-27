//! Boot chain and disk safety of the installer (image/files/usr/local/libexec/wolos):
//! install-check (refusals, other systems, Secure Boot), install-esp (\EFI\WolOS, firmware entry
//! "WolOS" first, other entries kept) and install-verify, run with fake FreeBSD tools. Every fake
//! logs its arguments: no tool may ever name a disk other than the picked one.
//! Plus the quiet loader (no menu unless a key is held) and the rEFInd icon in the image.
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
const ESP_UUID: &str = "f859c46d-19ee-4e40-8975-3ad1ab00ac09";

fn libexec(name: &str) -> PathBuf {
    PathBuf::from(ROOT)
        .join("image/files/usr/local/libexec/wolos")
        .join(name)
}

struct Fake {
    dir: PathBuf,
}

impl Fake {
    fn new(name: &str) -> Fake {
        let dir = std::env::temp_dir().join(format!("wol-boot-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::write(dir.join("log"), "").unwrap();
        let f = Fake { dir };
        // Anything that could reach a real disk or NVRAM is faked, failing unless a test says so.
        for t in [
            "efibootmgr",
            "efivar",
            "mount",
            "mount_msdosfs",
            "umount",
            "zpool",
            "gpart",
            "diskinfo",
            "sysctl",
            "glabel",
            "kenv",
            "dd",
            "newfs",
        ] {
            f.tool(t, "exit 1");
        }
        f
    }

    /// A fake tool: logs "<name> <args>" then runs `body`.
    fn tool(&self, name: &str, body: &str) -> &Fake {
        let p = self.dir.join("bin").join(name);
        let log = self.dir.join("log");
        fs::write(
            &p,
            format!(
                "#!/bin/sh\necho \"{name} $*\" >> '{}'\n{body}\n",
                log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        self
    }

    fn run(&self, script: &str, args: &[&str], env: &[(&str, &Path)]) -> (i32, String) {
        let mut c = Command::new("sh");
        c.arg(libexec(script)).args(args).env(
            "PATH",
            format!("{}:/usr/bin:/bin", self.dir.join("bin").display()),
        );
        for (k, v) in env {
            c.env(k, v);
        }
        let out = c.output().unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    }

    fn log(&self) -> String {
        fs::read_to_string(self.dir.join("log")).unwrap()
    }

    /// No logged tool call names `other` (a disk the user did not pick).
    fn never_touched(&self, other: &str) {
        let log = self.log();
        assert!(!log.contains(other), "{other} was touched:\n{log}");
    }
}

/// kern.disks = picked nda0, bystander ada0, boot stick da0 (ufs/FreeBSD_Install).
fn disks(f: &Fake, bytes: u64, gpart_list: &str, secureboot: &str, mounts: &str) {
    f.tool("sysctl", "echo 'nda0 ada0 da0'")
        .tool(
            "glabel",
            "printf '      Name  Status  Components\\nufs/FreeBSD_Install  N/A  da0s2a\\n'",
        )
        .tool(
            "diskinfo",
            &format!("echo \"$1\t512\t{bytes}\t1000\t0\t0\t0\""),
        )
        .tool("gpart", &format!("cat <<'G'\n{gpart_list}\nG"))
        .tool(
            "efivar",
            &if secureboot.is_empty() {
                "exit 1".to_string()
            } else {
                format!("echo '8be4df61-93ca-11d2-aa0d-00e098032b8c-SecureBoot'; echo '0000: {secureboot}'")
            },
        )
        .tool("mount", &format!("printf '{mounts}'"));
}

fn gpart(types: &[(&str, &str)]) -> String {
    let mut s = String::from("Geom name: nda0\nProviders:\n");
    for (i, (t, label)) in types.iter().enumerate() {
        s += &format!(
            "{n}. Name: nda0p{n}\n   label: {label}\n   type: {t}\n   rawuuid: 0000000{n}-0000-0000-0000-000000000000\n",
            n = i + 1
        );
    }
    s
}

#[test]
fn check_names_other_systems_and_refuses_unsafe_disks() {
    let big = 500_000_000_000;
    for (name, picked, bytes, parts, sb, mounts, rc, want, not) in [
        (
            "empty",
            "nda0",
            big,
            gpart(&[]),
            "",
            "",
            0,
            vec!["size 500000000000"],
            vec!["os ", "refuse", "secureboot"],
        ),
        (
            "windows",
            "nda0",
            big,
            gpart(&[
                ("efi", "EFI system partition"),
                ("ms-reserved", "Microsoft reserved partition"),
                ("ms-basic-data", "Basic data partition"),
                ("ms-recovery", ""),
            ]),
            "",
            "",
            0,
            vec!["os Windows"],
            vec!["os Linux", "refuse"],
        ),
        (
            "dual-boot",
            "nda0",
            big,
            gpart(&[
                ("efi", ""),
                ("!e3c9e316-0b5c-4db8-817d-f92df00215ae", ""),
                ("!4f68bce3-e8cd-4db1-96e7-fbcaf984b709", "root"),
                ("linux-swap", ""),
            ]),
            "",
            "",
            0,
            vec!["os Windows", "os Linux"],
            vec!["os macOS"],
        ),
        (
            "mac",
            "nda0",
            big,
            gpart(&[("efi", ""), ("apple-apfs", "")]),
            "",
            "",
            0,
            vec!["os macOS"],
            vec!["os Windows"],
        ),
        (
            "old-wolos",
            "nda0",
            big,
            gpart(&[
                ("freebsd-boot", "gptboot0"),
                ("efi", "efiboot0"),
                ("freebsd-zfs", "zfs0"),
            ]),
            "",
            "",
            0,
            vec!["os FreeBSD"],
            vec![],
        ),
        (
            "secureboot",
            "nda0",
            big,
            gpart(&[]),
            "01",
            "",
            0,
            vec!["secureboot on"],
            vec!["refuse"],
        ),
        (
            "secureboot-off",
            "nda0",
            big,
            gpart(&[]),
            "00",
            "",
            0,
            vec![],
            vec!["secureboot"],
        ),
        (
            "small",
            "nda0",
            8_000_000_000,
            gpart(&[]),
            "",
            "",
            2,
            vec!["refuse too-small"],
            vec![],
        ),
        (
            "boot-stick",
            "da0",
            big,
            gpart(&[]),
            "",
            "",
            2,
            vec!["refuse not-listed"],
            vec!["size"],
        ),
        (
            "missing",
            "nda9",
            big,
            gpart(&[]),
            "",
            "",
            2,
            vec!["refuse not-listed"],
            vec![],
        ),
        (
            "mounted",
            "nda0",
            big,
            gpart(&[]),
            "",
            "/dev/nda0p1 /media/usb msdosfs rw 0 0\\n",
            2,
            vec!["refuse in-use"],
            vec![],
        ),
        (
            "other-mounted",
            "nda0",
            big,
            gpart(&[]),
            "",
            "/dev/nda01p1 /media/x ufs rw 0 0\\n/dev/ada0p2 / ufs rw 1 1\\n",
            0,
            vec![],
            vec!["refuse"],
        ),
    ] {
        let f = Fake::new(&format!("check-{name}"));
        disks(&f, bytes, &parts, sb, mounts);
        let (code, out) = f.run("install-check", &[picked], &[]);
        assert_eq!(code, rc, "{name}: {out}");
        for w in want {
            assert!(out.lines().any(|l| l == w), "{name}: want {w:?} in {out}");
        }
        for n in not {
            assert!(!out.contains(n), "{name}: {n:?} in {out}");
        }
        f.never_touched("ada0 ");
        let log = f.log();
        for writer in [
            "gpart destroy",
            "gpart create",
            "gpart add",
            "zpool create",
            "newfs",
            "dd ",
        ] {
            assert!(!log.contains(writer), "{name}: install-check wrote: {log}");
        }
    }
}

/// Fake efibootmgr with NVRAM in files: entries "NUM LABEL PATH" (uuid = the ESP's when the
/// path is under $ESP_MNT), BootOrder appended on create like a firmware that doesn't reorder.
const EFIBOOTMGR: &str = r#"S=$FAKE_NVRAM
case "$1" in
  -v|"") echo "BootOrder   : $(cat $S/order)"
     while read -r n l p u; do echo "+Boot$n* $l HD(1,GPT,$u,0x28,0x82000)/File($p)"; done < $S/entries ;;
  -B) grep -v "^$3 " $S/entries > $S/e; mv $S/e $S/entries
      tr -d ' ' < $S/order | tr , '\n' | grep -vx "$3" | paste -sd, - | sed 's/,/, /g' > $S/o; mv $S/o $S/order ;;
  --create) n=$(printf "%04X" $(( $(wc -l < $S/entries) + 7 ))); p=${6#$ESP_MNT}
      echo "$n $4 $(echo $p | tr / '\\\\') $FAKE_UUID" >> $S/entries; echo "$(cat $S/order), $n" > $S/order ;;
  -o) echo "$2" | sed 's/,/, /g' > $S/order ;;
esac"#;

#[test]
fn esp_gets_wolos_loader_icon_and_first_firmware_entry_without_touching_others() {
    let f = Fake::new("esp");
    let esp = f.dir.join("esp");
    let root = f.dir.join("root");
    let nv = f.dir.join("nvram");
    for d in [
        &esp.join("EFI/BOOT"),
        &esp.join("EFI/freebsd"),
        &root.join("boot"),
        &root.join("usr/local/share/wolos/boot"),
        &nv,
    ] {
        fs::create_dir_all(d).unwrap();
    }
    let loader = b"\0FreeBSD/amd64 EFI loader, Revision 3.0\0".to_vec();
    fs::write(root.join("boot/loader.efi"), &loader).unwrap();
    fs::write(root.join("usr/local/share/wolos/boot/os_wolos.png"), b"PNG").unwrap();
    // What bsdinstall's bootconfig left: its copy, the fallback, an entry on this (new) ESP.
    fs::write(esp.join("EFI/freebsd/loader.efi"), &loader).unwrap();
    fs::write(esp.join("EFI/BOOT/BOOTX64.EFI"), &loader).unwrap();
    fs::write(
        nv.join("entries"),
        format!(
            "0000 Windows \\EFI\\Microsoft\\Boot\\bootmgfw.efi 11111111-0000-0000-0000-000000000000\n\
             0001 ubuntu \\EFI\\ubuntu\\shimx64.efi 22222222-0000-0000-0000-000000000000\n\
             0004 WolOS \\efi\\freebsd\\loader.efi {ESP_UUID}\n"
        ),
    )
    .unwrap();
    fs::write(nv.join("order"), "0000, 0001, 0004\n").unwrap();
    f.tool(
        "gpart",
        &format!(
            "case $1 in show) printf '=>  40  1000  nda0  GPT  (500G)\\n  40  1024  nda0p1  freebsd-boot  (512K)\\n  2048  532480  nda0p2  efi  (260M)\\n  534528 999 nda0p3 freebsd-swap (4G)\\n';;\n\
             list) printf '1. Name: nda0p1\\n   rawuuid: 99999999-0000-0000-0000-000000000000\\n2. Name: nda0p2\\n   rawuuid: {ESP_UUID}\\n';; esac"
        ),
    )
    .tool("efibootmgr", EFIBOOTMGR)
    .tool("mount_msdosfs", "exit 1");
    let (code, out) = f.run(
        "install-esp",
        &["nda0"],
        &[
            ("WOLOS_ROOT", &root),
            ("WOLOS_ESP_MNT", &esp),
            ("FAKE_NVRAM", &nv),
            ("FAKE_UUID", Path::new(ESP_UUID)),
            ("ESP_MNT", &esp),
        ],
    );
    assert_eq!(code, 0, "{out}\n{}", f.log());
    assert_eq!(fs::read(esp.join("EFI/WolOS/loader.efi")).unwrap(), loader);
    for icon in ["loader.png", "os_wolos.png"] {
        assert_eq!(
            fs::read(esp.join("EFI/WolOS").join(icon)).unwrap(),
            b"PNG",
            "rEFInd icon {icon}"
        );
    }
    assert!(
        !esp.join("EFI/freebsd").exists(),
        "one WolOS loader in boot menus"
    );
    let entries = fs::read_to_string(nv.join("entries")).unwrap();
    assert!(entries.contains("Windows \\EFI\\Microsoft"), "{entries}");
    assert!(entries.contains("ubuntu \\EFI\\ubuntu"), "{entries}");
    assert!(
        !entries.contains("freebsd"),
        "bsdinstall's entry replaced: {entries}"
    );
    let wolos: Vec<&str> = entries.lines().filter(|l| l.contains(" WolOS ")).collect();
    assert_eq!(wolos.len(), 1, "{entries}");
    assert!(wolos[0].contains("\\EFI\\WolOS\\loader.efi"), "{entries}");
    let new = &wolos[0][..4];
    let order = fs::read_to_string(nv.join("order")).unwrap();
    assert_eq!(
        order.trim(),
        format!("{new}, 0000, 0001"),
        "WolOS first, others kept in order"
    );
    let log = f.log();
    for other in ["-B -b 0000", "-B -b 0001", "nda0p1 ", "ada"] {
        assert!(!log.contains(other), "touched {other:?}:\n{log}");
    }
}

#[test]
fn esp_keeps_another_systems_fallback_loader() {
    let f = Fake::new("esp-fallback");
    let esp = f.dir.join("esp");
    let root = f.dir.join("root");
    fs::create_dir_all(esp.join("EFI/BOOT")).unwrap();
    fs::create_dir_all(root.join("boot")).unwrap();
    fs::write(root.join("boot/loader.efi"), b"FreeBSD/amd64 EFI loader").unwrap();
    fs::write(esp.join("EFI/BOOT/BOOTX64.EFI"), b"shim from someone else").unwrap();
    f.tool("gpart", "printf '  40  532480  nda0p1  efi  (260M)\\n'")
        .tool("efibootmgr", "exit 1");
    let (code, out) = f.run(
        "install-esp",
        &["nda0"],
        &[("WOLOS_ROOT", &root), ("WOLOS_ESP_MNT", &esp)],
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        fs::read(esp.join("EFI/BOOT/BOOTX64.EFI")).unwrap(),
        b"shim from someone else"
    );
    assert!(esp.join("EFI/WolOS/loader.efi").exists());
    // BIOS boot (no efibootmgr): files only, no NVRAM writes.
    assert!(!f.log().contains("--create"), "{}", f.log());
}

#[test]
fn esp_refuses_a_partition_of_another_disk() {
    let f = Fake::new("esp-other");
    f.tool("gpart", "printf '  40  532480  ada0p1  efi  (260M)\\n'");
    let (code, _) = f.run("install-esp", &["nda0"], &[]);
    assert_eq!(code, 1);
    assert!(!f.log().contains("mount_msdosfs"));
}

#[test]
fn verify_imports_only_from_the_picked_disk_and_reports_faults() {
    for (name, layout, import_ok, want_rc, want) in [
        (
            "whole",
            "freebsd-boot efi freebsd-swap freebsd-zfs",
            true,
            0,
            "ok",
        ),
        (
            "no-bios",
            "efi freebsd-swap freebsd-zfs",
            true,
            1,
            "problem bios-boot",
        ),
        (
            "pool-gone",
            "freebsd-boot efi freebsd-swap freebsd-zfs",
            false,
            1,
            "problem pool-import",
        ),
    ] {
        let f = Fake::new(&format!("verify-{name}"));
        let mut rows = String::new();
        for (i, t) in layout.split(' ').enumerate() {
            rows += &format!("  40  1000  nda0p{}  {t}  (1G)\\n", i + 1);
        }
        let esp = f.dir.join("esp");
        fs::create_dir_all(esp.join("EFI/WolOS")).unwrap();
        fs::create_dir_all(esp.join("EFI/BOOT")).unwrap();
        fs::write(esp.join("EFI/WolOS/loader.efi"), b"x").unwrap();
        fs::write(esp.join("EFI/BOOT/BOOTX64.EFI"), b"x").unwrap();
        f.tool("gpart", &format!("printf '{rows}'"))
            .tool(
                "zpool",
                &format!(
                    "case $1 in import) {};; get) echo zroot/ROOT/default;; list) [ \"$3\" = -v ] && exit 1; echo ONLINE;; esac",
                    if import_ok { "exit 0" } else { "exit 1" }
                ),
            )
            // mount_msdosfs -o ro /dev/X <dir>: point the temp dir at the fake ESP.
            .tool("mount_msdosfs", &format!("rmdir \"$4\" && ln -s '{}' \"$4\"", esp.display()))
            .tool("umount", "rm \"$1\" && mkdir \"$1\"");
        let (code, out) = f.run("install-verify", &["nda0"], &[]);
        assert_eq!(code, want_rc, "{name}: {out}");
        assert!(out.lines().any(|l| l == want), "{name}: {out}");
        let log = f.log();
        if log.contains("zpool import") {
            assert!(log.contains("zpool import -d /dev/nda0p"), "{name}: {log}");
        }
        assert!(
            !log.contains("zpool create") && !log.contains("gpart destroy"),
            "{log}"
        );
    }
}

#[test]
fn install_run_checks_before_writing_and_verifies_before_done() {
    let run = fs::read_to_string(libexec("install-run")).unwrap();
    let check = run
        .find("install-check")
        .expect("install-run calls install-check");
    let write = run.find("bsdinstall script").unwrap();
    assert!(check < write, "every check before the first write");
    let verify = run.find("install-verify").expect("install-run verifies");
    let done = run.find("say done 100").unwrap();
    assert!(verify < done, "done only after the check");
    let cfg = fs::read_to_string(PathBuf::from(ROOT).join("image/installerconfig")).unwrap();
    assert!(
        cfg.contains("install-esp \"$ZFSBOOT_DISKS\""),
        "ESP layout on the picked disk"
    );
    assert!(
        cfg.contains("ZFSBOOT_DISKS=$WOLOS_INSTALL_DISK"),
        "only the picked disk"
    );
}

#[test]
fn loader_boots_quietly_unless_a_key_is_held() {
    let lua =
        fs::read_to_string(PathBuf::from(ROOT).join("image/files/boot/lua/local.lua")).unwrap();
    for want in [
        "io.ischar()",
        "\"autoboot_delay\", \"-1\"",
        "\"autoboot_delay\", \"NO\"",
    ] {
        assert!(lua.contains(want), "local.lua: {want}");
    }
    let build = fs::read_to_string(PathBuf::from(ROOT).join("image/build.sh")).unwrap();
    let conf = &build[build.find("loader.conf.wolos\" <<'L'").unwrap()..];
    let conf = &conf[..conf.find("\nL\n").unwrap()];
    for want in [
        "boot_mute=\"YES\"",
        "module_verbose=\"0\"",
        "splash=\"/boot/images/wolos.png\"",
    ] {
        assert!(conf.contains(want), "loader.conf.wolos: {want}");
    }
    assert!(
        build.contains("share/wolos/boot/os_wolos.png"),
        "rEFInd icon shipped"
    );
    assert!(
        PathBuf::from(ROOT)
            .join("design/brand/os_wolos.png")
            .exists(),
        "brand icon"
    );
    // Live: no console login, no bsdinstall unless asked for.
    assert!(build.contains("ttyv0"), "ttyv0 getty off");
    assert!(
        build.contains("kenv -q wolos.install"),
        "bsdinstall only on request"
    );
}
