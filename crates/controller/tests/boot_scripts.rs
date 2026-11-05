// SPDX-License-Identifier: GPL-2.0-or-later
//! The boot shell scripts with their tools faked and a fake ESP / extensions directory on disk:
//! boot-ok (what blesses a boot), esp-sync (the boot images and entries) and xguard-update.
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn lib(n: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../files/usr/lib/wolos")
        .join(n)
}

/// A scratch directory under the temp dir, removed when the test ends (passed or not).
struct Scratch(PathBuf);

impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

// The scratch dir of one test goes with it, passed or failed.
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn dir(name: &str) -> Scratch {
    let d = std::env::temp_dir().join(format!("wol-b5-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("bin")).unwrap();
    Scratch(d)
}

/// A fake command in `d/bin` that logs `name args` to `d/log`, then runs `body`.
fn fake(d: &Path, name: &str, body: &str) {
    let p = d.join("bin").join(name);
    fs::write(
        &p,
        format!(
            "#!/bin/sh\necho \"{name} $*\" >> {}\n{body}\n",
            d.join("log").display()
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

/// boot-ok in `d`: `procs` are the program names running (their /proc/<pid>/comm), `cmdline`
/// the kernel's; wold is active, sleep, wol and systemd-sysext are fakes.
fn boot_ok(d: &Path, procs: &[&str], cmdline: &str) -> Output {
    let proc_dir = d.join("proc");
    let _ = fs::remove_dir_all(&proc_dir);
    for (i, p) in procs.iter().enumerate() {
        let pid = proc_dir.join(format!("{}", 100 + i));
        fs::create_dir_all(&pid).unwrap();
        fs::write(pid.join("comm"), format!("{p}\n")).unwrap();
    }
    fs::create_dir_all(&proc_dir).unwrap();
    fs::write(d.join("cmdline"), cmdline).unwrap();
    for sub in ["marks", "ext", "work"] {
        fs::create_dir_all(d.join(sub)).unwrap();
    }
    if !d.join("bin/systemctl").exists() {
        fake(
            d,
            "systemctl",
            "case \"$*\" in *ActiveState*) echo inactive ;; esac",
        );
    }
    fake(d, "sleep", "");
    fake(d, "wol", "");
    fake(d, "systemd-sysext", "");
    Command::new("sh")
        .arg(lib("boot-ok"))
        .env("PATH", path(d))
        .env("WOLOS_PROC", &proc_dir)
        .env("WOLOS_CMDLINE_FILE", d.join("cmdline"))
        .env("WOLOS_MARKS", d.join("marks"))
        .env("WOLOS_ESP", d.join("esp"))
        .env("WOLOS_SLOTS", d.join("run/slots.json"))
        .output()
        .unwrap()
}

/// A bootctl that lists one slot (what `bootctl list --json=short` prints), logging its args.
const SLOTS: &str = r#"[{"id":"wolos-a.conf","title":"WolOS","version":"1-2026.11.02","isDefault":true,"isSelected":true}]"#;

/// A4 F5/F6 (QA-LIVE4-5): Settings > Restore and `wol restore` run as the person, who cannot read
/// the ESP (fmask 0077), so `bootctl list` fails for them. A good boot and every esp-sync leave
/// bootctl's list world-readable in /run/wolos/slots.json, replaced whole, for the ESP they used.
#[test]
fn boot_ok_and_esp_sync_leave_a_readable_slot_list() {
    use std::os::unix::fs::PermissionsExt;
    let d = dir("slots");
    fake(&d, "bootctl", &format!("echo '{SLOTS}'"));
    let o = boot_ok(&d, &["wol-loginwindow"], "rootflags=subvol=@wolos-a ro");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let f = d.join("run/slots.json");
    assert_eq!(fs::read_to_string(&f).unwrap().trim(), SLOTS);
    assert_eq!(
        fs::metadata(&f).unwrap().permissions().mode() & 0o777,
        0o644
    );
    assert!(
        log(&d).contains(&format!(
            "bootctl --esp-path={} list --json=short",
            d.join("esp").display()
        )),
        "{}",
        log(&d)
    );
    assert!(!d.join("run/slots.json.new").exists());
    // bootctl failing leaves the last good list in place.
    fake(&d, "bootctl", "exit 1");
    assert!(
        boot_ok(&d, &["wol-loginwindow"], "rootflags=subvol=@wolos-a ro")
            .status
            .success()
    );
    assert_eq!(fs::read_to_string(&f).unwrap().trim(), SLOTS);

    let e = esp("slots-esp");
    fake(&e, "bootctl", &format!("echo '{SLOTS}'"));
    let o = esp_sync(&e, true);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        fs::read_to_string(e.join("run/slots.json")).unwrap().trim(),
        SLOTS
    );
}

/// QA-SYS-3: wold up is not a good boot; the login window (or Setup, or a session's shell or lock
/// screen) must be on the screen, seen on two looks in a row. Nothing for 90 s: not blessed.
#[test]
fn waits_for_greeter_mark() {
    let d = dir("greeter");
    let o = boot_ok(
        &d,
        &["systemd", "wold", "Hyprland"],
        "rootflags=subvol=@wolos-b",
    );
    assert!(!o.status.success(), "no login window: {}", log(&d));
    assert!(!log(&d).contains("wol boot bless"), "{}", log(&d));
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("nothing reached the screen"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    for ui in ["wol-loginwindow", "wol-setup", "wol-shell", "wol-lock"] {
        let d = dir(&format!("greeter-{ui}"));
        let o = boot_ok(&d, &["wold", "Hyprland", ui], "rootflags=subvol=@wolos-b");
        assert!(
            o.status.success(),
            "{ui}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(log(&d).contains("wol boot bless"), "{ui}: {}", log(&d));
    }
    // wold not active: the screen alone does not count either.
    let d = dir("greeter-nowold");
    fake(
        &d,
        "systemctl",
        "case \"$*\" in *is-active*) exit 3 ;; esac",
    );
    let o = boot_ok(&d, &["wol-loginwindow"], "rootflags=subvol=@wolos-b");
    assert!(!o.status.success(), "{}", log(&d));
}

/// QA-LIVE3-36: a slot whose seal or floor check failed this boot is never blessed, even when
/// the login window comes up after it.
#[test]
fn boot_ok_requires_seal_check() {
    for mark in ["verity-failed-wolos-b", "epoch-failed-wolos-b"] {
        let d = dir(mark);
        fs::create_dir_all(d.join("marks")).unwrap();
        fs::write(d.join("marks").join(mark), "failed\n").unwrap();
        let o = boot_ok(&d, &["wol-loginwindow"], "rootflags=subvol=@wolos-b ro");
        assert!(!o.status.success(), "{mark}");
        assert!(!log(&d).contains("wol boot bless"), "{mark}: {}", log(&d));
    }
    // The other slot's mark is not this boot's business.
    let d = dir("mark-other");
    fs::create_dir_all(d.join("marks")).unwrap();
    fs::write(d.join("marks/verity-failed-wolos-a"), "failed\n").unwrap();
    let o = boot_ok(&d, &["wol-loginwindow"], "rootflags=subvol=@wolos-b ro");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

/// QA-A9-CORE-2 (RC-13, RC-02): the cleanup of a layer pacman never finished is `wol layers clear`'s
/// alone, under the update lock (its tests in tests/pkg.rs). boot-ok used to `rm -rf` a layer
/// without its record behind a `mountpoint -q` guard that is false for ever (the layer chroot lives
/// in a private mount namespace): a `wol pkg` running at that moment lost its layer. boot-ok
/// touches no layer, whatever its state, and leaves the one call.
#[test]
fn boot_ok_leaves_every_layer_to_wol_layers_clear() {
    let d = dir("layers");
    let ext = d.join("ext");
    for n in ["wolos-layer-0001", "wolos-layer-0002", "wolos-layer-0003"] {
        fs::create_dir_all(ext.join(n).join("usr/bin")).unwrap();
        fs::create_dir_all(d.join("work").join(n)).unwrap();
    }
    // 0002: no record (a change may be running in it). 0003: record, no extension-release.
    fs::write(ext.join("wolos-layer-0001/wolos-layer.json"), "{}\n").unwrap();
    fs::write(ext.join("wolos-layer-0003/wolos-layer.json"), "{}\n").unwrap();
    let o = boot_ok(&d, &["wol-loginwindow"], "rootflags=subvol=@wolos-a");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    for n in ["wolos-layer-0001", "wolos-layer-0002", "wolos-layer-0003"] {
        assert!(ext.join(n).is_dir(), "{n} removed by boot-ok");
        assert!(
            d.join("work").join(n).is_dir(),
            "{n} work removed by boot-ok"
        );
    }
    assert!(
        !ext.join("wolos-layer-0003/usr/lib/extension-release.d")
            .exists(),
        "boot-ok wrote a release itself"
    );
    let l = log(&d);
    assert_eq!(l.matches("wol layers clear").count(), 1, "{l}");
    assert!(!l.contains("systemctl reload-or-restart"), "{l}");
    // The script has no `mountpoint` guard to be blind with.
    let t = fs::read_to_string(lib("boot-ok")).unwrap();
    assert!(
        !t.contains("mountpoint -q"),
        "boot-ok still guards with mountpoint"
    );
}

/// QA-A9-CORE-4: power cut after the new entry was written and before `update-building` went. The
/// new slot boots and is blessed, so its mark goes; the mark of a build of the other slot stays.
#[test]
fn boot_ok_clears_the_mark_of_the_slot_it_blessed() {
    let d = dir("build-mark");
    fs::create_dir_all(d.join("marks")).unwrap();
    fs::write(d.join("marks/update-building"), "wolos-b 2026/11/04\n").unwrap();
    let o = boot_ok(&d, &["wol-loginwindow"], "rootflags=subvol=@wolos-b ro");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!d.join("marks/update-building").exists());
    let d2 = dir("build-mark-other");
    fs::create_dir_all(d2.join("marks")).unwrap();
    fs::write(d2.join("marks/update-building"), "wolos-a 2026/11/04\n").unwrap();
    let o = boot_ok(&d2, &["wol-loginwindow"], "rootflags=subvol=@wolos-b ro");
    assert!(o.status.success());
    assert!(
        d2.join("marks/update-building").exists(),
        "a build of the other slot lost its mark"
    );
}

/// A slot root with a kernel and initramfs, and an ESP, for esp-sync; ukify is a fake that
/// writes its --output.
fn esp(name: &str) -> Scratch {
    let d = dir(name);
    for sub in ["root/boot", "root/etc/kernel", "root/usr/lib", "esp", "sb"] {
        fs::create_dir_all(d.join(sub)).unwrap();
    }
    fs::write(d.join("root/boot/vmlinuz-linux-wolos"), "kernel 1").unwrap();
    fs::write(d.join("root/boot/initramfs-linux-wolos.img"), "initrd 1").unwrap();
    fs::write(
        d.join("root/etc/kernel/cmdline"),
        "root=x rootflags=subvol=@wolos-a ro",
    )
    .unwrap();
    fs::write(d.join("root/usr/lib/os-release"), "NAME=WolOS").unwrap();
    fake(
        &d,
        "ukify",
        "for a; do case $a in --output=*) echo image > \"${a#--output=}\" ;; esac; done",
    );
    fake(&d, "sync", "");
    d
}

fn esp_sync(d: &Path, new: bool) -> Output {
    let mut c = Command::new("sh");
    c.arg(lib("esp-sync"));
    if new {
        c.arg("--new");
    }
    c.env("PATH", path(d))
        .env("WOLOS_ROOT", d.join("root"))
        .env("WOLOS_ESP", d.join("esp"))
        .env("WOLOS_SLOT", "wolos-a")
        .env("WOLOS_SB_DIR", d.join("sb"))
        .env("WOLOS_SLOTS", d.join("run/slots.json"))
        .output()
        .unwrap()
}

fn entries(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(d.join("esp/loader/entries"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// QA-SYS-11: a kernel (or initramfs) change inside a slot gives the new image three tries
/// again, whatever the old entry had left; the same image rebuilt keeps the entry as it is.
#[test]
fn kernel_change_resets_tries() {
    let d = esp("tries");
    let o = esp_sync(&d, true);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(entries(&d), ["wolos-a+3.conf"]);
    // Two boots used two tries.
    fs::rename(
        d.join("esp/loader/entries/wolos-a+3.conf"),
        d.join("esp/loader/entries/wolos-a+1-2.conf"),
    )
    .unwrap();
    // Nothing changed: the entry keeps its count.
    assert!(esp_sync(&d, false).status.success());
    assert_eq!(entries(&d), ["wolos-a+1-2.conf"]);
    // A new kernel: three tries again, the old entry gone, the generation kept.
    fs::write(d.join("root/boot/vmlinuz-linux-wolos"), "kernel 2").unwrap();
    let o = esp_sync(&d, false);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(entries(&d), ["wolos-a+3.conf"]);
    let e = fs::read_to_string(d.join("esp/loader/entries/wolos-a+3.conf")).unwrap();
    assert!(e.contains("version 1-"), "{e}");
    // A blessed entry gets counted again after a new initramfs.
    fs::rename(
        d.join("esp/loader/entries/wolos-a+3.conf"),
        d.join("esp/loader/entries/wolos-a.conf"),
    )
    .unwrap();
    fs::write(d.join("root/boot/initramfs-linux-wolos.img"), "initrd 2").unwrap();
    assert!(esp_sync(&d, false).status.success());
    assert_eq!(entries(&d), ["wolos-a+3.conf"]);
}

/// QA-F7 / BOOT2-7: every image and entry is synced before its name moves, the ESP at the end,
/// leftovers of a cut-off run go, and the image's digest is kept for the other slot's check.
#[test]
fn esp_files_are_durable_before_done() {
    let d = esp("durable");
    fs::create_dir_all(d.join("esp/EFI/wolos")).unwrap();
    fs::create_dir_all(d.join("esp/loader/entries")).unwrap();
    fs::write(d.join("esp/EFI/wolos/wolos-a.efi.new"), "").unwrap();
    fs::write(d.join("esp/loader/entries/wolos-a+3.conf.new"), "").unwrap();
    fs::write(d.join("esp/EFI/wolos/wolos-b.efi.new"), "other slot's").unwrap();
    let o = esp_sync(&d, true);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let l = log(&d);
    let esp = d.join("esp").display().to_string();
    let img = l
        .find(&format!("sync {esp}/EFI/wolos/wolos-a.efi.new"))
        .expect(&l);
    let ent = l
        .find(&format!("sync {esp}/loader/entries/wolos-a+3.conf.new"))
        .expect(&l);
    let all = l.rfind(&format!("sync -f {esp}/")).expect(&l);
    assert!(img < ent && ent < all, "{l}");
    assert!(!d.join("esp/EFI/wolos/wolos-a.efi.new").exists());
    assert!(!d.join("esp/loader/entries/wolos-a+3.conf.new").exists());
    assert!(
        d.join("esp/EFI/wolos/wolos-b.efi.new").exists(),
        "not this slot's"
    );
    let sum = fs::read_to_string(d.join("esp/wolos-a/image.sha256")).unwrap();
    assert_eq!(sum.trim().len(), 64, "{sum}");
}

/// QA-LIVE4-7: a cut after "updated" left `wolos-b.efi` and its `+3` entry 0 bytes. FAT renames
/// are directory writes a file's own sync does not cover: the image's name is on the disk (the
/// whole ESP synced) before the entry naming it is written, and the image's digest record is
/// written aside and renamed whole (a cut-off one is removed at the next run).
#[test]
fn fsync_before_entry() {
    let d = esp("fsync-entry");
    fs::create_dir_all(d.join("esp/wolos-a")).unwrap();
    fs::write(d.join("esp/wolos-a/image.sha256.new"), "").unwrap();
    let o = esp_sync(&d, true);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let l = log(&d);
    let esp = d.join("esp").display().to_string();
    let img = l
        .find(&format!("sync {esp}/EFI/wolos/wolos-a.efi.new"))
        .expect(&l);
    let ent = l
        .find(&format!("sync {esp}/loader/entries/wolos-a+3.conf.new"))
        .expect(&l);
    assert!(
        l[img..ent].contains(&format!("sync -f {esp}/")),
        "the ESP is synced between the image's rename and its entry: {l}"
    );
    assert!(
        l[img..ent].contains(&format!("sync {esp}/wolos-a/image.sha256.new")),
        "{l}"
    );
    assert!(!d.join("esp/wolos-a/image.sha256.new").exists());
    let sum = fs::read_to_string(d.join("esp/wolos-a/image.sha256")).unwrap();
    assert_eq!(sum.trim().len(), 64, "{sum}");
}

/// QA-STORE-54: offline is not an error; a bundle that is here and refused is (exit 1, said).
#[test]
fn rules_failure_exits_1() {
    let d = dir("xguard");
    fake(
        &d,
        "curl",
        "while [ $# -gt 0 ]; do [ \"$1\" = -o ] && { echo '{\"files\":[]}' > \"$2\"; shift; }; shift; done",
    );
    // The signed manifest passes `rules check`; the bundle then fails `rules install` (a data
    // file that does not match its signed hash): a failure the unit shows. A refused check (not
    // signed, not newer) is the normal no-news day and stays exit 0 (rel/0.12.1).
    fake(&d, "wol-xguard", "[ \"$2\" = check ] && exit 0; exit 1");
    fake(&d, "systemctl", "");
    let run = |d: &Path| {
        Command::new("sh")
            .arg(lib("xguard-update"))
            .env("PATH", path(d))
            .output()
            .unwrap()
    };
    let o = run(&d);
    assert_eq!(o.status.code(), Some(1), "{}", log(&d));
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("refused"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!log(&d).contains("systemctl try-restart"), "{}", log(&d));
    // Offline: exit 0, nothing installed.
    let d = dir("xguard-offline");
    fake(&d, "curl", "exit 6");
    fake(&d, "wol-xguard", "");
    let o = run(&d);
    assert_eq!(o.status.code(), Some(0));
    assert!(!log(&d).contains("wol-xguard"), "{}", log(&d));
    // Installed: the scanner looks again.
    let d = dir("xguard-ok");
    fake(
        &d,
        "curl",
        "while [ $# -gt 0 ]; do [ \"$1\" = -o ] && { echo '{}' > \"$2\"; shift; }; shift; done",
    );
    fake(&d, "wol-xguard", "");
    fake(&d, "systemctl", "");
    let o = run(&d);
    assert_eq!(o.status.code(), Some(0), "{}", log(&d));
    assert!(log(&d).contains("systemctl try-restart wolos-xguard.service"));
}

/// QA-SYS-4: the splash draws the disk passphrase prompt (and any question or message) itself;
/// with no handlers plymouth showed nothing on a real screen. The screenshot golden is the
/// wolvm journey (unrun here); this is the static half.
#[test]
fn plymouth_shows_the_passphrase_prompt() {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../files/usr/share/plymouth/themes/wolos/wolos.script");
    let t = fs::read_to_string(p).unwrap();
    for f in [
        "Plymouth.SetDisplayPasswordFunction(display_password);",
        "Plymouth.SetDisplayQuestionFunction(display_question);",
        "Plymouth.SetDisplayNormalFunction(display_normal);",
        "Plymouth.SetMessageFunction(display_message);",
        "Plymouth.SetHideMessageFunction(hide_message);",
    ] {
        assert!(t.contains(f), "{f}");
    }
    for fun in [
        "fun display_password(text, bullets)",
        "fun display_question(text, typed)",
        "fun display_normal()",
        "fun display_message(text)",
        "fun hide_message(text)",
    ] {
        assert!(t.contains(fun), "{fun}");
    }
    let open = t.matches('{').count();
    assert_eq!(open, t.matches('}').count(), "balanced braces");
}

/// QA-DOC-22: install-run re-runs itself under systemd-inhibit before it starts the results
/// folder and the log; the run under the hold starts both afresh, so a line said before the
/// re-run was lost. Run for real: what the first pass would print reaches the final log.
#[test]
fn install_run_holds_sleep_before_the_log_starts() {
    let t = fs::read_to_string(lib("install-run")).unwrap();
    let hold = t.find("exec systemd-inhibit").unwrap();
    for later in ["rm -rf \"${R:?}\"", "exec 2>>\"$LOG\"", "take info <<EOF"] {
        let at = t.find(later).unwrap_or_else(|| panic!("{later}"));
        assert!(hold < at, "the hold comes before {later}");
    }
    // systemd-inhibit (fake) runs the script again; the log of that run holds the refusal.
    let d = dir("install-log");
    fake(
        &d,
        "systemd-inhibit",
        "while [ \"${1#--}\" != \"$1\" ]; do shift; done; exec \"$@\"",
    );
    let res = d.join("res");
    let o = Command::new("sh")
        .arg(lib("install-run"))
        .arg("/dev/nonexistent-b5")
        .env("PATH", path(&d))
        .env("WOLOS_INSTALL_PATH", path(&d))
        .env("WOLOS_RESULT_DIR", &res)
        .env("WOLOS_INSTALL_ROOT", d.join("t"))
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert_eq!(log(&d).matches("systemd-inhibit").count(), 1, "{}", log(&d));
    let l = fs::read_to_string(res.join("log")).unwrap_or_default();
    assert!(l.contains("not a disk the installer may erase"), "{l}");
}

/// live-4 F3 with the D5 rule: blessed stays blessed unless the image is damaged (rebuilt, then
/// counted), is not the image blessed last, or this boot's check failed; then the next start is
/// counted again. A counted entry is left alone. The new entry is on the disk (written, synced,
/// renamed) before the blessed one goes (D4).
#[test]
fn shutdown_rearms_the_try_counter() {
    let sha = |p: &Path| {
        let o = Command::new("sha256sum").arg(p).output().unwrap();
        String::from_utf8_lossy(&o.stdout)
            .split(' ')
            .next()
            .unwrap()
            .to_owned()
    };
    // `stamp`: the digest esp-sync kept matches the image; `blessed`: so does boot-ok's record.
    let setup = |name: &str, entry: &str, stamp: bool, blessed: bool, mark: bool| {
        let d = dir(name);
        for sub in [
            "esp/loader/entries",
            "esp/EFI/wolos",
            "esp/wolos-a",
            "lib",
            "marks",
        ] {
            fs::create_dir_all(d.join(sub)).unwrap();
        }
        fs::write(
            d.join("esp/loader/entries").join(entry),
            "uki /EFI/wolos/wolos-a.efi\n",
        )
        .unwrap();
        let img = d.join("esp/EFI/wolos/wolos-a.efi");
        fs::write(&img, "image").unwrap();
        let sum = sha(&img);
        let other = "0".repeat(64);
        let pick = |b: bool| if b { sum.clone() } else { other.clone() };
        fs::write(d.join("esp/wolos-a/image.sha256"), pick(stamp) + "\n").unwrap();
        fs::write(d.join("esp/wolos-a/blessed.sha256"), pick(blessed) + "\n").unwrap();
        if mark {
            fs::write(d.join("marks/verity-failed-wolos-a"), "failed\n").unwrap();
        }
        fs::write(d.join("cmdline"), "root=x rootflags=subvol=@wolos-a ro").unwrap();
        fs::write(
            d.join("lib/esp-sync"),
            format!(
                "echo \"esp-sync $WOLOS_SLOT\" >> {}\n",
                d.join("log").display()
            ),
        )
        .unwrap();
        fake(&d, "sync", "");
        d
    };
    let run = |d: &Path, mode: &str| {
        Command::new("sh")
            .arg(lib("boot-rearm"))
            .arg(mode)
            .env("PATH", path(d))
            .env("WOLOS_ESP", d.join("esp"))
            .env("WOLOS_CMDLINE_FILE", d.join("cmdline"))
            .env("WOLOS_LIB", d.join("lib"))
            .env("WOLOS_MARKS", d.join("marks"))
            .output()
            .unwrap()
    };
    let entries = |d: &Path| {
        let mut v: Vec<String> = fs::read_dir(d.join("esp/loader/entries"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    };
    // QA-LIVE4-3: intact and the image blessed last: the next start is still counted, once, so
    // an image damaged while the computer is off falls back (the blessed entry never did);
    // `start` is a no-op.
    let d = setup("rearm-keep", "wolos-a.conf", true, true, false);
    assert!(run(&d, "start").status.success());
    assert_eq!(entries(&d), ["wolos-a.conf"]);
    assert!(run(&d, "stop").status.success());
    assert_eq!(entries(&d), ["wolos-a+1.conf"]);
    assert!(!log(&d).contains("esp-sync"), "{}", log(&d));
    // Damaged: rebuilt from the slot, then counted.
    let d = setup("rearm-damaged", "wolos-a.conf", false, true, false);
    assert!(run(&d, "stop").status.success());
    assert!(log(&d).contains("esp-sync wolos-a"), "{}", log(&d));
    assert_eq!(entries(&d), ["wolos-a+3.conf"]);
    // Not the image blessed last (rebuilt or re-signed since): counted.
    let d = setup("rearm-changed", "wolos-a.conf", true, false, false);
    assert!(run(&d, "stop").status.success());
    assert_eq!(entries(&d), ["wolos-a+3.conf"]);
    assert!(!log(&d).contains("esp-sync"), "{}", log(&d));
    // This boot's seal check failed: counted.
    let d = setup("rearm-mark", "wolos-a.conf", true, true, true);
    assert!(run(&d, "stop").status.success());
    assert_eq!(entries(&d), ["wolos-a+3.conf"]);
    // D4: the new entry is synced before its rename, the ESP again before the old one goes.
    let l = log(&d);
    let esp = d.join("esp").display().to_string();
    let new = l
        .find(&format!("sync {esp}/loader/entries/wolos-a+3.conf.new"))
        .expect(&l);
    assert!(l[new..].contains(&format!("sync -f {esp}/")), "{l}");
    let t = fs::read_to_string(lib("boot-rearm")).unwrap();
    let mv = t
        .find("mv -f \"$E/$slot+$tries.conf.new\" \"$E/$slot+$tries.conf\"")
        .unwrap();
    let rm = t.find("rm -f \"$E/$slot.conf\"").unwrap();
    assert!(mv < rm, "rename before the old entry goes");
    assert!(
        !t.contains("mv -f \"$E/$slot.conf\""),
        "no rename of the only entry"
    );
    // Counted already (this boot was not good): left as it is.
    let d = setup("rearm-counted", "wolos-a+1-2.conf", false, false, true);
    assert!(run(&d, "stop").status.success());
    assert_eq!(entries(&d), ["wolos-a+1-2.conf"]);
}

/// D5: a good boot records the image it started, which boot-rearm compares against.
#[test]
fn boot_ok_records_the_blessed_image() {
    let d = dir("blessed-record");
    fs::create_dir_all(d.join("esp/EFI/wolos")).unwrap();
    fs::create_dir_all(d.join("esp/wolos-a")).unwrap();
    fs::write(d.join("esp/EFI/wolos/wolos-a.efi"), "image").unwrap();
    fake(&d, "sync", "");
    let o = boot_ok(&d, &["wol-loginwindow"], "rootflags=subvol=@wolos-a ro");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let rec = fs::read_to_string(d.join("esp/wolos-a/blessed.sha256")).unwrap();
    let o = Command::new("sha256sum")
        .arg(d.join("esp/EFI/wolos/wolos-a.efi"))
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&o.stdout).starts_with(rec.trim()),
        "{rec}"
    );
}
