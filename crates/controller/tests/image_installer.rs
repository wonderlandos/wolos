//! Install WolOS from the live desktop (ADR 0013): the app runs `install-run <disk>` as root, which
//! runs the same installer script as the boot menu, without its dialogs, printing progress lines.
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn sh_n(p: &str) {
    assert!(
        Command::new("sh")
            .arg("-n")
            .arg(repo(p))
            .status()
            .unwrap()
            .success(),
        "{p}: sh -n"
    );
}

#[test]
fn install_run_checks_the_disk_and_reports_progress() {
    let run = read("image/files/usr/local/libexec/wolos/install-run");
    sh_n("image/files/usr/local/libexec/wolos/install-run");
    for want in [
        "export WOLOS_INSTALL_DISK=",
        "script -q /dev/null bsdinstall script /etc/installerconfig",
        "say prepare 0 ",
        "say done 100 ",
        "say failed 0 ",
        // a stopped earlier attempt's pool is let go before a retry
        "zpool export -f zroot",
    ] {
        assert!(run.contains(want), "install-run lacks {want}");
    }
    // A disk that is not a candidate never reaches bsdinstall.
    let out = Command::new("sh")
        .arg(repo("image/files/usr/local/libexec/wolos/install-run"))
        .arg("sda")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "wolos-install: failed 0 not a disk the installer may erase: sda\n"
    );
}

/// install-run in a scratch libexec with stub commands; returns (exit, stdout, sysctl calls).
fn run_install(name: &str, install_ok: bool) -> (Option<i32>, String, String) {
    let script = if install_ok {
        "echo 'wolos-install: disk 2 Erasing'; exit 0"
    } else {
        "echo 'wolos-install: failed 0 disk: a step failed'; exit 1"
    };
    let (dir, path) = stubs(
        name,
        &[
            ("install-check", "exit 0"),
            ("install-verify", "exit 0"),
            (
                "sysctl",
                "echo \"$*\" >> \"$STUB_LOG\"; [ \"$1\" = -n ] && echo S3; exit 0",
            ),
            ("script", script),
            ("mount", "exit 0"),
            ("zpool", "exit 1"),
        ],
    );
    let lib = dir.join("bin");
    fs::copy(
        repo("image/files/usr/local/libexec/wolos/install-run"),
        lib.join("install-run"),
    )
    .unwrap();
    let log = dir.join("sysctl.log");
    let out = Command::new("sh")
        .arg(lib.join("install-run"))
        .arg("ada0")
        .env("WOLOS_INSTALL_PATH", path)
        .env("STUB_LOG", &log)
        .output()
        .unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        fs::read_to_string(&log).unwrap_or_default(),
    )
}

#[test]
fn closing_the_lid_never_sleeps_an_install() {
    for ok in [true, false] {
        let (code, out, calls) = run_install(&format!("installer-lid-{ok}"), ok);
        assert_eq!(code, Some(if ok { 0 } else { 1 }), "{out}");
        assert_eq!(
            calls,
            "-n hw.acpi.lid_switch_state\n\
             hw.acpi.lid_switch_state=NONE\n\
             hw.acpi.lid_switch_state=S3\n",
            "lid held during the install and given back after (ok={ok})"
        );
        let last = out.lines().last().unwrap_or_default();
        assert!(
            last.starts_with(if ok {
                "wolos-install: done 100"
            } else {
                "wolos-install: failed 0"
            }),
            "{out}"
        );
    }
}

/// A scratch dir with stub commands first in PATH; returns (dir, PATH).
fn stubs(name: &str, cmds: &[(&str, &str)]) -> (PathBuf, String) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("bin")).unwrap();
    for (n, body) in cmds {
        let p = dir.join("bin").join(n);
        fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        Command::new("chmod").arg("755").arg(&p).status().unwrap();
    }
    let path = format!(
        "{}:{}",
        dir.join("bin").display(),
        std::env::var("PATH").unwrap()
    );
    (dir, path)
}

/// The preamble as bsdinstall runs it (sourced under `set -e`, `trap error EXIT`), then `steps`.
fn run_preamble(name: &str, steps: &str, fail_at: &str, fstab: bool) -> (Option<i32>, String) {
    let cfg = read("image/installerconfig");
    let preamble = cfg.split("#!/bin/sh").next().unwrap();
    let (dir, path) = stubs(
        name,
        &[
            ("kldload", "exit 0"),
            ("disks", "echo ada0; echo nda0"),
            ("bsdinstall", "echo \"ran $*\"; [ \"$1\" != \"$FAIL_AT\" ]"),
            ("zpool", "echo \"zpool $*\""),
        ],
    );
    let preamble = preamble.replace(
        "/usr/local/libexec/wolos/install-disks",
        &dir.join("bin/disks").display().to_string(),
    );
    let fstab_path = dir.join("fstab");
    if fstab {
        fs::write(&fstab_path, "").unwrap();
    }
    fs::write(dir.join("preamble"), preamble).unwrap();
    let body = format!(
        "error() {{ echo upstream-error-box; exit 1; }}\nset -e\ntrap error EXIT\n. ./preamble\n{steps}\ntrap - EXIT\n"
    );
    let out = Command::new("sh")
        .arg("-c")
        .arg(body)
        .current_dir(&dir)
        .env("PATH", path)
        .env("WOLOS_INSTALL_DISK", "ada0")
        .env("FAIL_AT", fail_at)
        .env("PATH_FSTAB", &fstab_path)
        .output()
        .unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn installer_script_reports_each_step_for_the_app() {
    let cfg = read("image/installerconfig");
    sh_n("image/installerconfig");
    let preamble = cfg.split("#!/bin/sh").next().unwrap();
    let app = preamble
        .split("if [ -n \"${WOLOS_INSTALL_DISK:-}\" ]; then")
        .nth(1)
        .and_then(|b| b.split("\nelse\n").next())
        .expect("app branch in the preamble");
    assert!(!app.contains("bsddialog"), "{app}");
    // The same steps bsdinstall runs for the boot menu, each announced first.
    let (code, out) = run_preamble(
        "installer-steps",
        "bsdinstall zfsboot\nbsdinstall mount\nbsdinstall checksum\nbsdinstall distextract\n\
         bsdinstall bootconfig\nbsdinstall config\nbsdinstall entropy\nbsdinstall umount",
        "none",
        false,
    );
    assert_eq!(code, Some(0), "{out}");
    let marks: Vec<String> = out
        .lines()
        .filter_map(|l| l.strip_prefix("wolos-install: "))
        .map(|l| l.split(' ').take(2).collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(
        marks,
        [
            "disk 2",
            "files 5",
            "files 6",
            "boot 22",
            "settings 24",
            "finish 97"
        ],
        "{out}"
    );
    assert!(
        out.contains("ran zfsboot") && out.contains("ran umount"),
        "{out}"
    );
    assert!(
        out.contains("Erasing ada0: GPT, EFI and BIOS boot, ZFS pool zroot"),
        "{out}"
    );
}

#[test]
fn a_stopped_install_says_where_and_lets_the_pool_go() {
    // Partitioning fails before anything is mounted: one line, no dialog box.
    let (code, out) = run_preamble(
        "installer-stop-disk",
        "bsdinstall zfsboot",
        "zfsboot",
        false,
    );
    assert_eq!(code, Some(1), "{out}");
    assert!(
        out.contains("wolos-install: failed 0 disk: a step failed"),
        "{out}"
    );
    assert!(!out.contains("upstream-error-box"), "{out}");
    // Extraction fails with the pool mounted: unmount, export, Try Again starts clean.
    let (code, out) = run_preamble(
        "installer-stop-files",
        "bsdinstall zfsboot\nbsdinstall mount\nbsdinstall checksum\nbsdinstall distextract",
        "distextract",
        true,
    );
    assert_eq!(code, Some(1), "{out}");
    assert!(
        out.contains("wolos-install: failed 0 files: a step failed"),
        "{out}"
    );
    assert!(
        out.contains("ran umount") && out.contains("zpool export -f zroot"),
        "{out}"
    );
    // A disk that is not a candidate stops before anything is erased.
    let cfg = read("image/installerconfig");
    assert!(cfg.contains(
        "echo \"$disks\" | grep -qx -- \"$WOLOS_INSTALL_DISK\" || error \"not a disk the installer may erase\""
    ));
}

#[test]
fn package_progress_becomes_percent_lines() {
    let cfg = read("image/installerconfig");
    let setup = cfg.split("#!/bin/sh").nth(1).unwrap();
    let prog = setup
        .split("awk -v app=\"${WOLOS_INSTALL_DISK:-}\" '")
        .nth(1)
        .and_then(|p| p.split('\'').next())
        .expect("progress awk in the setup script");
    let run = |app: &str, input: &str| {
        let mut c = Command::new("awk")
            .arg("-v")
            .arg(format!("app={app}"))
            .arg(prog)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        c.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        String::from_utf8(c.wait_with_output().unwrap().stdout).unwrap()
    };
    let pkg = "Updating repository\n[1/74] Installing pkg-2.0.6...\n[37/74] Installing zsh-5.9...\n\
               [74/74] Installing wol-store-1.0...\ninstall-pkgs rc=0\n";
    assert_eq!(
        run("ada0", pkg),
        "wolos-install: packages 25 Installing apps (1 of 74)\n\
         wolos-install: packages 60 Installing apps (37 of 74)\n\
         wolos-install: packages 95 Installing apps (74 of 74)\n\
         wolos-install: finish 95 Setting up the new system\n"
    );
    // The boot menu installer keeps its one console line.
    assert!(run("", pkg).contains("\r  Installing WolOS... (37/74)"));
}

#[test]
fn live_launcher_opens_the_app_and_the_image_ships_it() {
    let l = read("image/files/usr/local/libexec/wolos/wol-install");
    sh_n("image/files/usr/local/libexec/wolos/wol-install");
    let app = l
        .find("exec /usr/local/bin/wol-installer")
        .expect("app first");
    let term = l.find("exec foot").expect("terminal fallback");
    assert!(app < term);
    assert!(read("image/build-rust.sh").contains(" wol-installer "));
    let b = read("image/build.sh");
    for a in ["setup-welcome", "pocket-watch", "keyhole", "white-rabbit"] {
        assert!(b.contains(a), "build.sh does not ship {a}");
        assert!(
            repo(&format!("design/illustrations/png/{a}@2x.png")).exists(),
            "{a}@2x.png missing"
        );
    }
    assert!(b.contains("/usr/local/share/wolos/illustrations"));
}
