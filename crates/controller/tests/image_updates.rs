//! The daily update check the image ships (wol_core::update::check): cron runs the script hourly,
//! it checks at most once a day, and leaves findings every account can read.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use wol_core::update::check::{self, Updates};

fn repo(p: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(p)
}

const SCRIPT: &str = "image/files/usr/local/libexec/wolos/update-check";

/// Fake pkg, freebsd-update and sysctl: log their arguments, print canned output.
fn fakes(d: &Path, kernel: &str, online: bool) {
    let bin = d.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let pkg = format!(
        "#!/bin/sh\necho \"pkg $*\" >> \"$LOG\"\ncase $1 in\n\
         update) exit {} ;;\n\
         version) echo 'curl-8.1 <   needs updating (remote has 8.2)' ;;\n\
         audit) echo curl-8.1; exit 1 ;;\nesac\n",
        if online { 0 } else { 3 }
    );
    let fu = "#!/bin/sh\necho \"freebsd-update $*\" >> \"$LOG\"\n\
              case \"$*\" in *--not-running-from-cron*) [ \"$PAGER\" = cat ] || exit 9; \
              echo 'The following files will be updated as part of updating to 15.1-RELEASE-p3:' ;; \
              *updatesready) exit 0 ;; esac\n";
    let sysctl = format!("#!/bin/sh\necho {kernel}\n");
    for (n, body) in [
        ("pkg", pkg.as_str()),
        ("freebsd-update", fu),
        ("sysctl", &sysctl),
    ] {
        fs::write(bin.join(n), body).unwrap();
        fs::set_permissions(bin.join(n), fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(
        d.join("freebsd-update.conf"),
        "KeyPrint abc\nServerName update.FreeBSD.org\nComponents src world kernel\n",
    )
    .unwrap();
}

fn run(d: &Path, arg: &[&str]) -> bool {
    Command::new("sh")
        .arg(repo(SCRIPT))
        .args(arg)
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("LOG", d.join("log"))
        .env("WOLOS_UPDATES", d.join("out"))
        .env("WOLOS_FREEBSD_UPDATE_CONF", d.join("freebsd-update.conf"))
        .status()
        .unwrap()
        .success()
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-upd-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    d
}

#[test]
fn checks_once_a_day_and_leaves_readable_findings() {
    let d = tmp("day");
    fakes(&d, "WolKernel", true);
    assert!(run(&d, &[]));
    let u = Updates::read(&d.join("out")).expect("findings");
    assert_eq!(u.packages, ["curl"]);
    assert_eq!(u.security, ["curl"]);
    assert_eq!(u.base.as_deref(), Some("15.1-RELEASE-p3"));
    for f in ["checked", "pkg", "audit", "base", "freebsd-update.conf"] {
        let m = fs::metadata(d.join("out").join(f))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(m & 0o777, 0o644, "{f}: every account reads it");
    }
    let conf = fs::read_to_string(d.join("out/freebsd-update.conf")).unwrap();
    assert!(
        conf.contains("\nComponents world\n"),
        "WolKernel is never replaced:\n{conf}"
    );
    assert!(
        conf.contains("KeyPrint abc"),
        "rest of the stock configuration kept"
    );
    let log = fs::read_to_string(d.join("log")).unwrap();
    assert!(log.contains("pkg update -q"), "catalogue refreshed:\n{log}");
    assert!(log.contains("pkg audit -F -q"));
    assert!(
        !log.contains(" install"),
        "the check never installs:\n{log}"
    );

    // Again within the day: nothing runs.
    fs::remove_file(d.join("log")).unwrap();
    assert!(run(&d, &[]));
    assert!(!d.join("log").exists());
    // `now` forces it.
    assert!(run(&d, &["now"]));
    assert!(d.join("log").exists());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn generic_kernel_keeps_kernel_fixes_and_offline_tries_again() {
    let d = tmp("generic");
    fakes(&d, "GENERIC", false);
    assert!(run(&d, &[]), "offline is not a failure");
    assert_eq!(
        Updates::read(&d.join("out")),
        None,
        "not stamped: next hour retries"
    );
    fakes(&d, "GENERIC", true);
    assert!(run(&d, &[]));
    let conf = fs::read_to_string(d.join("out/freebsd-update.conf")).unwrap();
    assert!(conf.contains("Components src world kernel"));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn cron_runs_it_and_shell_scripts_parse() {
    let cron = fs::read_to_string(repo("desktop/overlay/usr/local/etc/cron.d/wolos-updates"))
        .expect("cron entry");
    let line = cron
        .lines()
        .find(|l| !l.starts_with('#') && !l.trim().is_empty())
        .unwrap();
    let f: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(f[..2], ["@hourly", "root"], "{line}");
    assert_eq!(f[2], check::SCRIPT);
    let st = Command::new("sh")
        .arg("-n")
        .arg(repo(SCRIPT))
        .status()
        .unwrap();
    assert!(st.success(), "sh -n {SCRIPT}");
    let m = fs::metadata(repo(SCRIPT)).unwrap().permissions().mode();
    assert!(m & 0o111 != 0, "{SCRIPT} executable");
    assert_eq!(check::EVERY_SECS, 72000, "script's EVERY");
}
