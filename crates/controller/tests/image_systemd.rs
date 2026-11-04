// SPDX-License-Identifier: GPL-2.0-or-later
//! WolOS on systemd (ADR 0016): the rc.d scripts became units in files/usr/lib/systemd, devd
//! became udev, cron became a timer, dhclient hooks became a NetworkManager dispatcher script,
//! sysctl.conf.local became sysctl.d. Scripts run against fakes; units are checked key by key.
#[path = "support/siblings.rs"]
mod siblings;

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
fn repo(p: &str) -> PathBuf {
    siblings::root().join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-systemd-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

const UNITS: &str = "files/usr/lib/systemd/system";

/// Values of KEY in SECTION of a unit file.
fn values(unit: &str, section: &str, key: &str) -> Vec<String> {
    let mut sect = "";
    let mut v = vec![];
    for l in unit.lines() {
        let l = l.trim();
        if l.starts_with('[') {
            sect = l;
        } else if sect == format!("[{section}]")
            && let Some((k, val)) = l.split_once('=')
            && k == key
        {
            v.extend(val.split_whitespace().map(String::from));
        }
    }
    v
}

fn unit(name: &str) -> String {
    read(&format!("{UNITS}/{name}"))
}

/// Fake commands in DIR/bin that log "name args" to DIR/log; returns PATH.
fn fakes(d: &Path, names: &[(&str, &str)]) -> String {
    let bin = d.join("bin");
    fs::create_dir_all(&bin).unwrap();
    for (n, body) in names {
        let p = bin.join(n);
        fs::write(
            &p,
            format!(
                "#!/bin/sh\necho \"{n} $*\" >> {}\n{body}\n",
                d.join("log").display()
            ),
        )
        .unwrap();
        Command::new("chmod").arg("+x").arg(&p).status().unwrap();
    }
    format!("{}:/usr/bin:/bin", bin.display())
}

fn log(d: &Path) -> String {
    fs::read_to_string(d.join("log")).unwrap_or_default()
}

#[test]
fn no_rc_d_or_usr_local_left_in_the_image() {
    for gone in [
        "files/etc/rc.d",
        "files/usr/local",
        "files/boot",
        "crates/api/rc.d",
        "desktop/overlay/etc/rc.conf.d",
        "desktop/overlay/etc/sysctl.conf.local",
        "desktop/overlay/etc/sysctl.kld.d",
        "desktop/overlay/etc/autofs",
        "desktop/overlay/etc/devd",
        "desktop/overlay/etc/cron.d",
        "desktop/overlay/etc/dhclient-exit-hooks",
        "desktop/overlay/usr/share/wolos/pf.conf",
    ] {
        assert!(!repo(gone).exists(), "{gone} still present");
    }
}

/// From util-linux in the base system, not built here.
const BASE_PROGRAMS: [&str; 1] = ["mount"];

#[test]
fn every_unit_runs_a_shipped_program_and_is_enabled_by_the_preset() {
    let preset = read("files/usr/lib/systemd/system-preset/50-wolos.preset");
    let rust = read("tools/build-rust.sh");
    let mut n = 0;
    for f in fs::read_dir(repo(UNITS)).unwrap().flatten() {
        let name = f.file_name().to_string_lossy().into_owned();
        if name.ends_with(".d") {
            continue; // drop-ins for Arch's units (nftables.service.d)
        }
        let u = fs::read_to_string(f.path()).unwrap();
        assert!(u.contains("[Unit]\nDescription="), "{name}");
        if name.ends_with(".service") {
            n += 1;
            for exec in values(&u, "Service", "ExecStart").iter().take(1) {
                let exec = exec.trim_start_matches(['-', '@', '+']);
                let ok = if let Some(b) = exec.strip_prefix("/usr/bin/") {
                    rust.contains(&format!(" {b} "))
                        || rust.contains(&format!(" {b};"))
                        || BASE_PROGRAMS.contains(&b)
                } else {
                    repo(&format!("files{exec}")).exists()
                };
                assert!(ok, "{name}: ExecStart {exec} is not shipped");
            }
        }
        // Timers and services started by a timer are enabled through the timer.
        let wanted = !values(&u, "Install", "WantedBy").is_empty();
        // Off until the person opts in: the preset has to say so, or systemd's default applies.
        let opt_in = name == "wolos-clam-update.timer";
        if wanted {
            let word = if opt_in { "disable" } else { "enable" };
            assert!(
                preset.lines().any(|l| l == format!("{word} {name}")),
                "{name} not in the preset"
            );
        }
    }
    assert!(n >= 6, "{n} services");
    for arch in [
        "NetworkManager.service",
        "bluetooth.service",
        "nftables.service",
        "cups.socket",
        "avahi-daemon.service",
        "systemd-timesyncd.service",
        "udisks2.service",
    ] {
        assert!(
            preset.lines().any(|l| l == format!("enable {arch}")),
            "{arch}"
        );
    }
}

#[test]
fn boot_order_hw_first_then_gpu_firstboot_and_wold() {
    let hw = unit("wolos-hw.service");
    assert_eq!(
        values(&hw, "Service", "ExecStart"),
        ["/usr/bin/wol", "hw", "boot"]
    );
    for before in [
        "wolos-gpu.service",
        "wolos-firstboot.service",
        "wold.service",
        "NetworkManager.service",
    ] {
        assert!(
            values(&hw, "Unit", "Before").iter().any(|b| b == before),
            "hw before {before}"
        );
    }
    assert!(
        values(&hw, "Unit", "After")
            .iter()
            .any(|a| a == "systemd-modules-load.service")
    );
    let fb = unit("wolos-firstboot.service");
    assert_eq!(
        values(&fb, "Unit", "ConditionPathExists"),
        ["!/var/lib/wolos/firstboot-done"]
    );
    assert!(
        values(&fb, "Unit", "After")
            .iter()
            .any(|a| a == "network-online.target")
    );
    // The login window never waits for the network: first boot's drivers come in the background.
    assert!(
        !values(&fb, "Unit", "Before")
            .iter()
            .any(|a| a == "wold.service")
    );
    // NetworkManager alone: systemd-networkd's wait-online held boot for two minutes.
    let preset = read("files/usr/lib/systemd/system-preset/50-wolos.preset");
    for off in [
        "systemd-networkd.service",
        "systemd-networkd-wait-online.service",
    ] {
        assert!(preset.contains(&format!("\ndisable {off}\n")), "{off}");
    }
    let w = unit("wold.service");
    assert_eq!(values(&w, "Service", "Restart"), ["always"]);
    assert!(
        values(&w, "Unit", "After")
            .iter()
            .any(|a| a == "wolos-hw.service")
    );
    // SIGTERM stops every job tree before wold exits (not on a restart: the next wold adopts
    // them, QA-SYS-2, so systemd must not kill them either); its sockets go with it.
    assert_eq!(values(&w, "Service", "KillMode"), ["process"]);
    let fbs = read("files/usr/lib/wolos/firstboot");
    assert!(
        fbs.contains("wol hw boot --online")
            && fbs.contains("/var/lib/wolos}")
            && fbs.contains("firstboot-done")
    );
}

#[test]
fn login_window_user_comes_from_sysusers() {
    let s = read("files/usr/lib/sysusers.d/wolos.conf");
    assert!(s.lines().any(|l| l.starts_with("u _wollogin 911 ")), "{s}");
    assert!(s.lines().any(|l| l == "m _wollogin video"), "{s}");
    // seatd hands the display only to the seat group (Arch's seatd).
    assert!(s.lines().any(|l| l == "m _wollogin seat"), "{s}");
    for f in [
        "desktop/overlay/usr/bin/wol-greeter-session",
        "desktop/overlay/usr/bin/wol-session",
    ] {
        assert!(read(f).contains("export LIBSEAT_BACKEND=seatd"), "{f}");
    }
    assert!(
        wol_core::settings::users::ACCOUNT_GROUPS
            .split(',')
            .any(|g| g == "seat")
    );
}

/// No text console for people: no getty on the screen, no VT on Ctrl+Alt+F<n>.
#[test]
fn no_text_console_for_users() {
    let preset = read("files/usr/lib/systemd/system-preset/50-wolos.preset");
    assert!(preset.contains("\ndisable getty@.service\n"), "{preset}");
    let l = read("files/usr/lib/systemd/logind.conf.d/wolos.conf");
    assert!(l.contains("NAutoVTs=0") && l.contains("ReserveVT=0"), "{l}");
    assert!(controller::login::CONSOLE == "/dev/tty1");
    // Arch's systemd post_install enables getty@tty1 directly (no preset): a login prompt on the
    // session's VT read every key typed on the desktop, passwords included.
    let install = read("wolos-system/wolos-system.install");
    for hook in ["post_install() {", "post_upgrade() {"] {
        let body = install
            .split(hook)
            .nth(1)
            .and_then(|b| b.split("\n}").next());
        assert!(
            body.is_some_and(|b| b.contains("systemctl mask getty@tty1.service")),
            "{hook} must mask getty@tty1: {install}"
        );
    }
}

/// Hardening and no core files in home folders (a crashed compositor left a 27 MB core in ~).
#[test]
fn sysctl_hardening_and_no_core_files() {
    let s = read("files/usr/lib/sysctl.d/90-wolos.conf");
    for kv in [
        "kernel.dmesg_restrict = 1",
        "kernel.kptr_restrict = 2",
        "kernel.yama.ptrace_scope = 1",
        "kernel.sysrq = 0",
        "fs.protected_hardlinks = 1",
        "fs.protected_symlinks = 1",
        "fs.suid_dumpable = 0",
        "vm.max_map_count = 2147483642",
        // Hardening default (audit8 cve #2): admin-only; revisit when a shipped app needs io_uring.
        "kernel.io_uring_disabled = 1",
    ] {
        assert!(s.lines().any(|l| l == kv), "sysctl.d lacks {kv}");
    }
    let mut prev = "";
    for l in s.lines() {
        if l.contains(" = ") {
            assert!(
                prev.starts_with('#') || prev.contains(" = "),
                "{l}: no reason"
            );
        }
        prev = l;
    }
    coredump_keeps_crash_details_not_core_files();
}

/// QA-CRASH-4: no core file is kept (`Storage=none`: a crashed compositor once left 27 MB in a
/// home folder), but systemd-coredump still reads the core, so the journal gets the stack trace
/// (`coredumpctl info`) and `/usr/lib/wolos/report` sees the crash. `ProcessSizeMax=0` skipped
/// both: every crash read "terminated abnormally without generating a coredump".
fn coredump_keeps_crash_details_not_core_files() {
    let c = read("files/usr/lib/systemd/coredump.conf.d/wolos.conf");
    let key = |k: &str| {
        c.lines()
            .find_map(|l| l.strip_prefix(k)?.strip_prefix('='))
            .map(str::trim)
    };
    assert_eq!(key("Storage"), Some("none"), "no core files on disk");
    let max = key("ProcessSizeMax").expect("ProcessSizeMax");
    let mb: u64 = max
        .trim_end_matches('M')
        .parse()
        .expect("ProcessSizeMax in M");
    assert!(
        max.ends_with('M') && mb >= 64,
        "ProcessSizeMax={max}: the core is read for its stack trace"
    );
}

/// The power policy follows the AC adapter (udev) and locks sessions before sleep (system-sleep).
#[test]
fn power_follows_the_adapter_and_sleep() {
    let r = read("files/usr/lib/udev/rules.d/90-wolos-power.rules");
    assert!(
        r.contains(r#"SUBSYSTEM=="power_supply""#)
            && r.contains(r#"ATTR{type}=="Mains|USB|"#)
            && r.contains(r#"RUN+="/usr/bin/wol power apply""#),
        "{r}"
    );
    let d = scratch("sleep");
    let path = fakes(&d, &[("wol", "")]);
    for (phase, want) in [("pre", "wol power suspend"), ("post", "wol power apply")] {
        let st = Command::new("sh")
            .arg(repo("files/usr/lib/systemd/system-sleep/wolos-power"))
            .arg(phase)
            .arg("suspend")
            .env("PATH", &path)
            .status()
            .unwrap();
        assert!(st.success());
        assert!(log(&d).lines().last() == Some(want), "{}", log(&d));
    }
    let p = unit("wolos-power.service");
    assert_eq!(
        values(&p, "Service", "ExecStart"),
        ["/usr/bin/wol", "power", "apply"]
    );
}

/// Network up: a driver only the internet has (NVIDIA) that first boot could not install is
/// installed now, once at a time, in the background (was dhclient-exit-hooks).
#[test]
fn network_up_retries_a_pending_driver() {
    let f = "desktop/overlay/etc/NetworkManager/dispatcher.d/50-wolos-online";
    let d = scratch("nm");
    let path = fakes(
        &d,
        &[
            ("flock", "shift 2; exec \"$@\""),
            ("wol", ""),
            ("systemd-run", ""),
        ],
    );
    for (action, runs) in [
        ("up", true),
        ("down", false),
        ("connectivity-change", false),
    ] {
        let _ = fs::remove_file(d.join("log"));
        let st = Command::new("sh")
            .arg(repo(f))
            .arg("wlan0")
            .arg(action)
            .env("PATH", &path)
            .env("WOLOS_LOCK", d.join("lock"))
            .env("WOL_MODULES_PENDING", d.join("none"))
            .env("WOL_PACKAGES_PENDING", d.join("none"))
            .status()
            .unwrap();
        assert!(st.success());
        // `wol hw online` runs in the background: wait (bounded) for it when it is due. Not due,
        // the script has already exited before starting anything.
        let due = std::time::Instant::now() + std::time::Duration::from_secs(10);
        // The `wol` line itself (the `flock` line before it names the same words).
        let ran = || log(&d).lines().any(|l| l == "wol hw online");
        while runs && !ran() && std::time::Instant::now() < due {
            std::thread::yield_now();
        }
        assert_eq!(ran(), runs, "{action}: {}", log(&d));
        assert!(
            !log(&d).contains("systemd-run"),
            "nothing queued: {}",
            log(&d)
        );
    }
}

/// audit5 I6-5: modules or media formats the installer queued offline start installing when a
/// connection comes up (Wi-Fi set up after the first login), not only at the next start.
#[test]
fn network_up_drains_the_queued_modules() {
    let f = "desktop/overlay/etc/NetworkManager/dispatcher.d/50-wolos-online";
    let d = scratch("nm-modules");
    // A default route (the hook drains only on a connection that has one): this host's must not
    // decide the test.
    let route = "case \"$*\" in *-4*) echo 'default via 10.0.2.2 dev wlan0' ;; esac";
    let path = fakes(
        &d,
        &[
            ("flock", "exit 0"),
            ("wol", ""),
            ("systemd-run", ""),
            ("ip", route),
        ],
    );
    let (mods, pkgs, none) = (
        d.join("modules-pending"),
        d.join("packages-pending"),
        d.join("none"),
    );
    let run = |action: &str, m: &Path, p: &Path| {
        let _ = fs::remove_file(d.join("log"));
        let st = Command::new("sh")
            .arg(repo(f))
            .arg("wlan0")
            .arg(action)
            .env("PATH", &path)
            .env("WOLOS_LOCK", d.join("lock"))
            .env("WOL_MODULES_PENDING", m)
            .env("WOL_PACKAGES_PENDING", p)
            .status()
            .unwrap();
        assert!(st.success());
        // systemd-run is called before the script exits; only the flock job is in the background.
        log(&d)
    };
    fs::write(&mods, "x\n").unwrap();
    let l = run("up", &mods, &none);
    assert!(
        l.contains("systemd-run --no-block --unit=wolos-modules-pending"),
        "{l}"
    );
    assert!(l.contains("modules-install --drain"), "{l}");
    assert!(
        !run("down", &mods, &none).contains("systemd-run"),
        "only on up"
    );
    fs::write(&mods, "").unwrap();
    assert!(
        !run("up", &mods, &none).contains("systemd-run"),
        "an empty queue: nothing"
    );
    fs::write(&pkgs, "gst-libav\n").unwrap();
    assert!(
        run("up", &mods, &pkgs).contains("modules-install --drain"),
        "media formats alone are drained too"
    );
}

/// A curl that serves the release manifest (`-o FILE`) and a signature (stdout) from anywhere.
const CURL_OK: &str = "o=; p=; for u; do [ \"$p\" = -o ] && o=$u; p=$u; done\n\
case $u in *.sig) printf sig ;; *) printf '{}' > \"$o\" ;; esac";

/// A pacman database like an installed slot's (`local` records, `sync` lists) for update-check.
fn slot_db(d: &Path) -> PathBuf {
    let db = d.join("db");
    fs::create_dir_all(db.join("local")).unwrap();
    fs::create_dir_all(db.join("sync")).unwrap();
    fs::write(db.join("sync/wolos.db"), "old list").unwrap();
    db
}

/// Daily update check: a timer runs it hourly; the script checks at most once every 20 hours,
/// offline leaves no stamp, findings are readable by every account. e2e D3: the packages are
/// checked against a temporary copy of pacman's database (`pacman -Sy` cannot lock the sealed
/// one, checkupdates needs fakeroot, which the image lacks), never against the real one.
#[test]
fn update_check_runs_hourly_and_checks_once_a_day() {
    let t = unit("wolos-update-check.timer");
    assert_eq!(values(&t, "Timer", "OnCalendar"), ["hourly"]);
    assert_eq!(values(&t, "Timer", "Persistent"), ["true"]);
    let s = unit("wolos-update-check.service");
    assert_eq!(
        values(&s, "Service", "ExecStart"),
        ["/usr/lib/wolos/update-check"]
    );
    let d = scratch("updates");
    let out = d.join("out");
    let db = slot_db(&d);
    let tmp = d.join("tmp");
    fs::create_dir_all(&tmp).unwrap();
    let run = |online: bool, arg: Option<&str>| {
        // pacman -Qu exits 1 when nothing is pending; -Sy fails offline.
        let body = if online {
            "case \"$*\" in *-Qu*) echo 'wolos-system 0.11.1-1 -> 0.11.2-1' ;; esac"
        } else {
            "case \"$*\" in *-Sy*) exit 1 ;; *-Qu*) exit 1 ;; esac"
        };
        let path = fakes(&d, &[("pacman", body), ("curl", CURL_OK)]);
        let mut c = Command::new("sh");
        c.arg(repo("files/usr/lib/wolos/update-check"))
            .env("PATH", &path)
            .env("WOLOS_PACMAN_DB", &db)
            .env("TMPDIR", &tmp)
            .env("WOLOS_UPDATES", &out);
        if let Some(a) = arg {
            c.arg(a);
        }
        // QA-NET-26: offline is said (75, try again later), never the 0 of "checked".
        assert_eq!(
            c.status().unwrap().code(),
            Some(if online { 0 } else { 75 })
        );
    };
    run(false, None);
    assert!(!out.join("checked").exists(), "offline: no stamp");
    run(true, None);
    assert_eq!(
        fs::read_to_string(out.join("pkg")).unwrap(),
        "wolos-system 0.11.1-1 -> 0.11.2-1\n"
    );
    assert!(out.join("checked").exists());
    let refreshes = || {
        log(&d)
            .lines()
            .filter(|l| l.starts_with("pacman ") && l.contains(" -Sy"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let n = refreshes().len();
    // The refresh and the query use the temporary copy, which is gone afterwards; the real
    // database is never written.
    for l in log(&d)
        .lines()
        .filter(|l| l.contains(" -Sy") || l.contains(" -Qu"))
    {
        let tdb = l.split_whitespace().nth(2).unwrap();
        assert!(
            l.starts_with("pacman --dbpath ") && tdb.starts_with(tmp.to_str().unwrap()),
            "{l}"
        );
    }
    assert_eq!(
        fs::read_dir(&tmp).unwrap().count(),
        0,
        "temporary database removed"
    );
    assert_eq!(
        fs::read_to_string(db.join("sync/wolos.db")).unwrap(),
        "old list"
    );
    run(true, None);
    assert_eq!(refreshes().len(), n, "checked less than 20 h ago");
    run(true, Some("now"));
    assert_eq!(refreshes().len(), n + 1);
    // Nothing pending (-Qu exits 1, says nothing): an empty list, a stamp.
    fs::remove_file(out.join("checked")).unwrap();
    let path = fakes(
        &d,
        &[
            ("pacman", "case \"$*\" in *-Qu*) exit 1 ;; esac"),
            ("curl", CURL_OK),
        ],
    );
    let st = Command::new("sh")
        .arg(repo("files/usr/lib/wolos/update-check"))
        .env("PATH", &path)
        .env("WOLOS_PACMAN_DB", &db)
        .env("TMPDIR", &tmp)
        .env("WOLOS_UPDATES", &out)
        .status()
        .unwrap();
    assert!(st.success());
    assert_eq!(fs::read_to_string(out.join("pkg")).unwrap(), "");
    assert!(
        out.join("checked").exists(),
        "nothing pending is a finished check"
    );
    run(true, Some("now"));
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(out.join("pkg")).unwrap().permissions().mode();
    assert_eq!(mode & 0o044, 0o044, "every account reads the findings");
    // audit5 C9-5: a stamp from the future (the clock was ahead) is stale, not fresh.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fs::write(out.join("checked"), format!("{}\n", now + 1_000_000_000)).unwrap();
    fs::remove_file(out.join("pkg")).unwrap();
    run(true, None);
    assert!(
        out.join("pkg").exists(),
        "a future stamp does not stop the check"
    );
    let stamp: u64 = fs::read_to_string(out.join("checked"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(stamp <= now + 60, "the new stamp is today's: {stamp}");
}

/// The release manifest and its signature come from the CDN under cdn.wolos.org, else under the
/// server's own name, else GitHub's latest release (`wol_core::cdn::latest`); a place that answers
/// 404 (or cannot be reached) is skipped, and both files always come from one place.
#[test]
fn update_check_reads_the_release_from_the_cdn_then_github() {
    let d = scratch("release");
    let out = d.join("out");
    let db = slot_db(&d);
    let tmp = d.join("tmp");
    fs::create_dir_all(&tmp).unwrap();
    let [cdn, mirror, github] =
        <[String; 3]>::try_from(wol_core::cdn::latest("release.json")).unwrap();
    // curl [options] [-o FILE] -- URL (get()): each CDN name is down or serves its own name,
    // GitHub serves GH.
    let curl = |primary: &str, second: &str| {
        format!(
            "o=; p=; for u; do [ \"$p\" = -o ] && o=$u; p=$u; done\n\
             case $u in https://cdn.wolos.org/*) {primary} ;; https://wolos.squareweb.app/*) {second} ;; *) b=GH ;; esac\n\
             case $u in *.sig) printf '%s' \"$b-sig\" ;; *) printf '%s' \"$b\" > \"$o\" ;; esac"
        )
    };
    for (primary, second, want) in [
        ("exit 6", "exit 22", "GH"),
        ("exit 22", "b=MIRROR", "MIRROR"),
        ("b=CDN", "b=MIRROR", "CDN"),
    ] {
        let _ = fs::remove_dir_all(&out);
        let _ = fs::remove_file(d.join("log"));
        let path = fakes(&d, &[("pacman", ""), ("curl", &curl(primary, second))]);
        let st = Command::new("sh")
            .arg(repo("files/usr/lib/wolos/update-check"))
            .arg("now")
            .env("PATH", &path)
            .env("WOLOS_PACMAN_DB", &db)
            .env("TMPDIR", &tmp)
            .env("WOLOS_UPDATES", &out)
            .status()
            .unwrap();
        assert!(st.success());
        assert_eq!(fs::read_to_string(out.join("base")).unwrap(), want);
        let hex: String = format!("{want}-sig")
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(fs::read_to_string(out.join("base.sig")).unwrap(), hex);
        let gets: Vec<String> = log(&d)
            .lines()
            .filter(|l| l.starts_with("curl "))
            .map(|l| l.rsplit(' ').next().unwrap().to_owned())
            .collect();
        assert_eq!(gets[0], cdn, "{gets:?}");
        let asked = |u: &str| gets.iter().any(|g| g.starts_with(u));
        assert_eq!(asked(&mirror), want != "CDN", "{want}: {gets:?}");
        assert_eq!(asked(&github), want == "GH", "{want}: {gets:?}");
    }
}

/// SHELLS-check F2: the vulnerability list comes from arch-audit. Absent, the check says so in the
/// journal and leaves no list (an empty one would read "no known vulnerabilities"); present, a
/// finding is listed.
#[test]
fn update_check_without_arch_audit_says_so_and_lists_nothing() {
    let d = scratch("audit");
    let out = d.join("out");
    fs::create_dir_all(&out).unwrap();
    // PATH of exactly the tools the script uses, so a host arch-audit cannot leak in.
    let db = slot_db(&d);
    let tmp = d.join("tmp");
    fs::create_dir_all(&tmp).unwrap();
    let fake = fakes(
        &d,
        &[
            ("pacman", "case \"$*\" in *-Qq*) echo linux-wolos ;; esac"),
            ("curl", CURL_OK),
        ],
    );
    let bin = d.join("only");
    fs::create_dir_all(&bin).unwrap();
    for t in [
        "date", "cat", "mkdir", "mv", "rm", "chmod", "od", "tr", "mktemp", "ln", "cp", "wc",
        "head", "df", "awk",
    ] {
        let o = Command::new("sh")
            .args(["-c", &format!("command -v {t}")])
            .output()
            .unwrap();
        let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
        std::os::unix::fs::symlink(&p, bin.join(t)).unwrap();
    }
    for t in ["pacman", "curl"] {
        std::os::unix::fs::symlink(
            Path::new(fake.split(':').next().unwrap()).join(t),
            bin.join(t),
        )
        .unwrap();
    }
    let sh = Command::new("sh")
        .args(["-c", "command -v sh"])
        .output()
        .unwrap();
    let sh = String::from_utf8_lossy(&sh.stdout).trim().to_string();
    let run = |path: &Path| {
        fs::remove_file(out.join("checked")).ok();
        Command::new(&sh)
            .arg(repo("files/usr/lib/wolos/update-check"))
            .env("PATH", path)
            .env("WOLOS_PACMAN_DB", &db)
            .env("TMPDIR", &tmp)
            .env("WOLOS_UPDATES", &out)
            .output()
            .unwrap()
    };
    fs::write(out.join("audit"), "stale\n").unwrap();
    let o = run(&bin);
    assert!(o.status.success());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("arch-audit is not installed"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!out.join("audit").exists(), "no list without arch-audit");
    assert!(
        out.join("checked").exists(),
        "the rest of the check still ran"
    );
    // With it: exit 1 + names = vulnerable packages.
    fs::write(bin.join("arch-audit"), "#!/bin/sh\necho openssl\nexit 1\n").unwrap();
    Command::new("chmod")
        .arg("+x")
        .arg(bin.join("arch-audit"))
        .status()
        .unwrap();
    let o = run(&bin);
    assert!(
        o.status.success() && o.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(fs::read_to_string(out.join("audit")).unwrap(), "openssl\n");
}

/// A panel handed over at brightness 0 (passthrough, some firmware) is raised; never lowered.
#[test]
fn dark_panel_backlight_is_raised() {
    let d = scratch("backlight");
    let bl = d.join("backlight");
    for (n, cur) in [("intel_backlight", "0"), ("acpi_video0", "300")] {
        fs::create_dir_all(bl.join(n)).unwrap();
        fs::write(bl.join(n).join("brightness"), format!("{cur}\n")).unwrap();
        fs::write(bl.join(n).join("max_brightness"), "1000\n").unwrap();
    }
    let path = fakes(&d, &[("logger", "")]);
    let st = Command::new("sh")
        .arg(repo("files/usr/lib/wolos/gpu"))
        .env("PATH", &path)
        .env("WOLOS_BACKLIGHT_DIR", &bl)
        .status()
        .unwrap();
    assert!(st.success());
    let b = |n: &str| fs::read_to_string(bl.join(n).join("brightness")).unwrap();
    assert_eq!(b("intel_backlight").trim(), "800");
    assert_eq!(b("acpi_video0").trim(), "300");
}

/// Settings > Network > Firewall: nothing comes in unasked except what a desktop needs.
#[test]
fn firewall_blocks_unasked_traffic_in() {
    // Our rules, not Arch's /etc/nftables.conf (nftables owns it): the unit loads them by drop-in.
    let n = read("desktop/overlay/etc/wolos/nftables.conf");
    let unit = read("files/usr/lib/systemd/system/nftables.service.d/wolos.conf");
    for want in [
        "ExecStart=\n",
        "ExecStart=/usr/bin/nft -f /etc/wolos/nftables.conf",
        "ExecReload=\n",
        "ExecReload=/usr/bin/nft -f /etc/wolos/nftables.conf",
        // QA-NET-1/2/3: stays active after loading (disable --now, reload and "running now" all
        // need that) and stopping takes the table out.
        "Type=oneshot\n",
        "RemainAfterExit=yes\n",
        "ExecStop=\n",
        "ExecStop=-/usr/bin/nft delete table inet wolos",
    ] {
        assert!(unit.contains(want), "nftables drop-in lacks {want:?}");
    }
    assert!(
        wol_core::sharing::FIREWALL_DROP_IN.ends_with("nftables.service.d/wolos.conf"),
        "the constant names the drop-in"
    );
    assert_eq!(
        wol_core::sharing::FIREWALL_RULES,
        "/etc/wolos/nftables.conf"
    );
    for want in [
        "flush ruleset",
        "type filter hook input priority filter; policy drop;",
        "ct state established,related accept",
        "iif \"lo\" accept",
        "icmp type { echo-request, destination-unreachable } accept",
        "icmpv6 type { echo-request, destination-unreachable, packet-too-big, nd-neighbor-solicit, nd-neighbor-advert, nd-router-advert } accept",
        "udp sport 67 udp dport 68 accept",
        "type filter hook output priority filter; policy accept;",
        // What a setting opens while it is on is a file in nftables.d, included in the input chain.
        "include \"/etc/wolos/nftables.d/*.conf\"",
    ] {
        assert!(n.contains(want), "nftables.conf lacks {want}");
    }
    assert_eq!(
        wol_core::sharing::FIREWALL_INCLUDE_DIR,
        "/etc/wolos/nftables.d"
    );
    // Shipped closed: Remote Login is off until the person turns it on.
    assert_eq!(
        read("desktop/overlay/etc/wolos/nftables.d/ssh.conf"),
        wol_core::sharing::ssh_rule(false)
    );
    assert!(!n.contains("tcp dport"), "no open TCP port by default");
}

/// QA-PRIV-6: mDNS is answered only for the multicast groups; a unicast query from any host that
/// can route here (udp dport 5353 alone) gets nothing. The ruleset is also loaded for real (in a
/// throw-away network namespace) when root is available without a password and nft is installed.
#[test]
fn mdns_multicast_only() {
    let n = read("desktop/overlay/etc/wolos/nftables.conf");
    let rules: Vec<&str> = n
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && l.contains("5353"))
        .collect();
    assert_eq!(
        rules,
        [
            "ip daddr 224.0.0.251 udp dport 5353 accept",
            "ip6 daddr ff02::fb udp dport 5353 accept",
            // RFC 6762 5.4: unicast replies to QU questions, link-local only (TTL 255).
            "ip ttl 255 udp sport 5353 udp dport 5353 accept",
            "ip6 hoplimit 255 udp sport 5353 udp dport 5353 accept",
        ],
        "every 5353 rule names the mDNS group or is a link-local reply from port 5353"
    );
    assert!(
        !n.lines().any(|l| l.trim() == "udp dport 5353 accept"),
        "mDNS open to any address"
    );
    let sudo = Command::new("sudo").args(["-n", "true"]).status();
    let nft = Command::new("nft").arg("--version").output();
    if !(sudo.is_ok_and(|s| s.success()) && nft.is_ok()) {
        return;
    }
    let d = scratch("nftcheck");
    fs::create_dir_all(d.join("d")).unwrap();
    fs::write(d.join("d/ssh.conf"), wol_core::sharing::ssh_rule(true)).unwrap();
    let conf = d.join("n.conf");
    fs::write(
        &conf,
        n.replace("/etc/wolos/nftables.d", d.join("d").to_str().unwrap()),
    )
    .unwrap();
    let o = Command::new("sudo")
        .args(["-n", "unshare", "-n", "nft", "-c", "-f"])
        .arg(&conf)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

/// Automatic updates (Settings > Software Update): a daily system timer checks and downloads
/// into pacman's cache unless /etc/wolos/auto-update says `download=off`; it never installs.
/// App updates are a per-account user timer that Settings enables.
#[test]
fn automatic_updates_download_daily_and_apps_update_per_account() {
    let t = unit("wolos-update-download.timer");
    assert_eq!(values(&t, "Timer", "OnCalendar"), ["daily"]);
    assert_eq!(values(&t, "Timer", "Persistent"), ["true"]);
    let s = unit("wolos-update-download.service");
    assert_eq!(
        values(&s, "Service", "ExecStart"),
        ["/usr/lib/wolos/update-download"]
    );
    assert_eq!(values(&s, "Service", "Type"), ["oneshot"]);
    let d = scratch("autoupdate");
    let conf = d.join("auto-update");
    // pacman's database as on a sealed slot: never written by the download (audit5 C9-1).
    let db = d.join("db");
    fs::create_dir_all(db.join("local")).unwrap();
    fs::create_dir_all(db.join("sync")).unwrap();
    fs::write(db.join("sync/core.db"), "old").unwrap();
    let tmp = d.join("tmp");
    fs::create_dir_all(&tmp).unwrap();
    // A download-only pacman: takes its lock in the database it is given, refreshes sync there.
    let pacman = "db=$2; [ \"$1\" = --dbpath ] || exit 9; : > \"$db/db.lck\" || exit 1
[ -L \"$db/local\" ] || exit 8; echo new > \"$db/sync/core.db\"
[ ! -e \"$(dirname \"$0\")/../pacman-fails\" ] || exit 1; rm -f \"$db/db.lck\"";
    // An unmetered connection (NetworkManager Metered 2 = no); the metered, battery and
    // free-space conditions are units.rs `download_timer_conditions` (QA-ACC-18).
    let path = fakes(
        &d,
        &[("wol", ""), ("pacman", pacman), ("busctl", "echo 'u 2'")],
    );
    let run = || {
        Command::new("sh")
            .arg(repo("files/usr/lib/wolos/update-download"))
            .env("PATH", &path)
            .env("WOLOS_AUTO_UPDATE", &conf)
            .env("WOLOS_PACMAN_DB", &db)
            .env("WOLOS_PACMAN_CACHE", &tmp)
            .env("TMPDIR", &tmp)
            .output()
            .unwrap()
    };
    let o = run();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let l = log(&d);
    let mut lines = l.lines();
    assert!(
        lines
            .next()
            .is_some_and(|x| x.starts_with("busctl --system get-property")),
        "metered? {l}"
    );
    assert_eq!(
        lines.next(),
        Some("wol update check --json"),
        "no file: on: {l}"
    );
    // The size of the download first (twice of it must be free), then the download.
    let size = lines.next().unwrap_or_default();
    assert!(
        size.contains(" -Sp --refresh --sysupgrade --print-format %s "),
        "{l}"
    );
    let p = lines.next().unwrap_or_default();
    let want = format!("pacman --dbpath {}/wolos-download-db.", tmp.display());
    assert!(
        p.starts_with(&want),
        "a temporary database, not the slot's: {l}"
    );
    assert!(
        p.ends_with(" -Sw --refresh --sysupgrade --noconfirm --noprogressbar"),
        "download only: {l}"
    );
    assert_eq!(
        fs::read_to_string(db.join("sync/core.db")).unwrap(),
        "old",
        "the slot's database is never written"
    );
    assert!(!db.join("db.lck").exists());
    assert_eq!(
        fs::read_dir(&tmp).unwrap().count(),
        0,
        "the temporary database goes"
    );
    // A failed download is the unit's failure, not a silent success.
    fs::write(d.join("pacman-fails"), "").unwrap();
    let o = run();
    assert!(!o.status.success(), "pacman failed: the unit fails");
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("could not download"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(
        fs::read_dir(&tmp).unwrap().count(),
        0,
        "the temporary database goes"
    );
    fs::remove_file(d.join("pacman-fails")).unwrap();
    fs::write(&conf, wol_core::update::auto::conf(false)).unwrap();
    fs::remove_file(d.join("log")).unwrap();
    assert!(run().status.success());
    assert_eq!(log(&d), "", "download=off: nothing runs");
    assert!(!log(&d).contains("-Syu"), "never installs");

    let user = "files/usr/lib/systemd/user";
    let ut = read(&format!("{user}/wolos-app-update.timer"));
    assert_eq!(values(&ut, "Timer", "OnCalendar"), ["daily"]);
    assert_eq!(values(&ut, "Install", "WantedBy"), ["timers.target"]);
    let us = read(&format!("{user}/wolos-app-update.service"));
    assert_eq!(
        values(&us, "Service", "ExecStart"),
        [
            "/usr/bin/flatpak",
            "--user",
            "update",
            "-y",
            "--noninteractive"
        ]
    );
    assert_eq!(wol_core::update::auto::APPS_TIMER, "wolos-app-update.timer");
}

/// Error reports (Settings > Privacy, off until turned on): a failed unit or a crash is sent once,
/// as {kind, version, kernel, detail}; nothing else leaves the machine.
#[test]
fn reports_send_new_failures_once_and_only_when_on() {
    let d = scratch("reports");
    let conf = d.join("reports.conf");
    let osr = d.join("os-release");
    fs::write(&osr, "NAME=WolOS\nVERSION_ID=0.11.2\n").unwrap();
    let path = fakes(
        &d,
        &[
            (
                "systemctl",
                "echo 'wold.service loaded failed failed WolOS daemon'",
            ),
            (
                "coredumpctl",
                "echo 'Thu 2026-10-29 18:00:01 UTC 4242 1000 1000 SIGSEGV present /usr/bin/wol-shell 1.2M'",
            ),
            ("curl", "cat >> \"$(dirname \"$0\")/../sent\""),
            ("uname", "echo 7.3.0-1-wolos"),
        ],
    );
    let run = || {
        let st = Command::new("sh")
            .arg(repo("files/usr/lib/wolos/report"))
            .env("PATH", &path)
            .env("WOLOS_REPORTS_CONF", &conf)
            .env("WOLOS_REPORTS_STATE", d.join("state"))
            .env("WOLOS_OS_RELEASE", &osr)
            .status()
            .unwrap();
        assert!(st.success());
    };
    run();
    assert!(!log(&d).contains("curl"), "off by default");
    fs::write(&conf, "send=1\n").unwrap();
    run();
    let sent = fs::read_to_string(d.join("sent")).unwrap();
    let lines: Vec<&str> = sent.lines().collect();
    assert_eq!(
        lines,
        [
            r#"{"kind":"unit-failed","version":"0.11.2","kernel":"7.3.0-1-wolos","detail":"wold.service"}"#,
            r#"{"kind":"crash","version":"0.11.2","kernel":"7.3.0-1-wolos","detail":"wol-shell SIGSEGV"}"#,
        ]
    );
    assert!(log(&d).contains(wol_core::cdn::REPORT[0]));
    run();
    assert_eq!(
        fs::read_to_string(d.join("sent")).unwrap(),
        sent,
        "sent once"
    );
    let t = unit("wolos-report.timer");
    assert_eq!(values(&t, "Timer", "OnCalendar"), ["hourly"]);
    let s = unit("wolos-report.service");
    assert_eq!(
        values(&s, "Service", "ExecStart"),
        ["/usr/lib/wolos/report"]
    );
}

#[test]
fn every_pam_service_the_code_opens_has_its_file() {
    // Linux-PAM falls back to /etc/pam.d/other (deny) for a missing service: a lock screen that
    // rejects every password. Services: the `c"wol-..."` names wol-login and wol-authcheck open.
    let mut services = vec![];
    for src in [
        "crates/controller/src/bin/wol-login.rs",
        "crates/controller/src/bin/wol-authcheck.rs",
        "crates/controller/src/login.rs",
    ] {
        for part in read(src).split("c\"wol-").skip(1) {
            services.push(format!("wol-{}", part.split('"').next().unwrap()));
        }
    }
    for s in ["wol-login", "wol-lock", "wol-admin"] {
        assert!(services.iter().any(|x| x == s), "{s} not found in the code");
    }
    for s in &services {
        let pam = read(&format!("desktop/overlay/etc/pam.d/{s}"));
        let auth: Vec<&str> = pam.lines().filter(|l| l.starts_with("auth")).collect();
        assert!(
            auth.iter().any(|l| l.contains("pam_unix.so")),
            "{s}: no password check"
        );
        // Arch's system-auth locks for 10 minutes after 3 typos, and the correct password then
        // reads "Incorrect password". WolOS still stops guessing, but forgives typos: 10 failures
        // lock for 60 s. No system-auth include, so the Arch default never applies here.
        assert!(!auth.iter().any(|l| l.contains("system-")), "{s}: {pam}");
        for stage in ["preauth", "authfail"] {
            assert!(
                auth.iter().any(|l| l.contains("pam_faillock.so")
                    && l.contains(stage)
                    && l.contains("deny=10")
                    && l.contains("unlock_time=60")),
                "{s}: no forgiving pam_faillock {stage}: {pam}"
            );
        }
        // The account phase clears the count after a success, so typos never add up over days.
        assert!(
            pam.lines()
                .any(|l| l.starts_with("account") && l.contains("pam_faillock.so")),
            "{s}: failures never reset: {pam}"
        );
    }
}

#[test]
fn one_mdns_stack() {
    // avahi answers .local (printers, AirPlay, Bonjour); resolved doing it too makes both unreliable.
    let d = read("files/usr/lib/systemd/resolved.conf.d/wolos.conf");
    assert!(
        d.contains("[Resolve]\n") && d.contains("MulticastDNS=no"),
        "{d}"
    );
}

/// The live medium's mark (wolos-iso build.sh); install-run leaves it out of the installed system.
const LIVE_MARK: &str = "ConditionPathExists=!/etc/wolos-live";

/// Units 50-wolos.preset enables that may run on the live medium: (unit, the condition in the
/// unit that keeps it off there, or "" when running there is right, reason).
const LIVE_SAFE: [(&str, &str, &str); 21] = [
    (
        "wold.service",
        "",
        "the login window; airootfs's wold.service.d/wolos-live.conf starts it only for Try WolOS",
    ),
    (
        "wolos-tpm-bind.service",
        "ConditionPathExists=/var/lib/wolos/secureboot/rebind.key",
        "install-run writes the key into the installed system",
    ),
    (
        "wolos-tpm-reseal.service",
        "ConditionKernelCommandLine=rd.luks.name",
        "the live medium has no encrypted root",
    ),
    (
        "wolos-mok-enroll.service",
        "ConditionPathExists=/var/lib/wolos/secureboot/db.der",
        "the computer's keys are made by install-run",
    ),
    (
        "systemd-sysext.service",
        "",
        "no system extension exists on the live medium; nothing to merge",
    ),
    (
        "wolos-hw.service",
        "",
        "drivers and settings for the devices present: the live session needs them",
    ),
    (
        "wolos-shm-limit.service",
        "",
        "the same /dev/shm limit for the live session",
    ),
    (
        "wolos-gpu.service",
        "",
        "panel brightness once the display driver is up, live too",
    ),
    (
        "wolos-power.service",
        "",
        "power policy for the adapter and battery, live too",
    ),
    (
        "NetworkManager.service",
        "",
        "the installer's online step and Try WolOS need the network",
    ),
    ("iwd.service", "", "Wi-Fi under NetworkManager"),
    (
        "bluetooth.service",
        "",
        "Bluetooth keyboards and mice in the live session",
    ),
    (
        "nftables.service",
        "",
        "the firewall protects the live session as well",
    ),
    (
        "cups.socket",
        "",
        "socket-activated: idle until Try WolOS prints",
    ),
    ("avahi-daemon.service", "", "finds printers for Try WolOS"),
    (
        "systemd-timesyncd.service",
        "",
        "a right clock for TLS downloads and the installed system's files",
    ),
    ("udisks2.service", "", "Files mounts disks in Try WolOS"),
    (
        "systemd-oomd.service",
        "",
        "a live session on 4 GB needs the same out-of-memory policy as the installed one (QA-ACC-22)",
    ),
    ("seatd.service", "", "the live session's seat"),
    (
        "vboxservice.service",
        "",
        "Arch's unit: ConditionVirtualization=oracle; guest tools in a live VM too",
    ),
    (
        "vmtoolsd.service",
        "",
        "Arch's unit: ConditionVirtualization=vmware; guest tools in a live VM too",
    ),
];

/// A unit shipped in files/ plus its drop-ins there.
fn unit_and_dropins(name: &str) -> String {
    let mut s = fs::read_to_string(repo(&format!("{UNITS}/{name}"))).unwrap_or_default();
    if let Ok(d) = fs::read_dir(repo(&format!("{UNITS}/{name}.d"))) {
        let mut ds: Vec<_> = d.flatten().map(|e| e.path()).collect();
        ds.sort();
        for p in ds {
            s.push('\n');
            s.push_str(&fs::read_to_string(p).unwrap());
        }
    }
    s
}

#[test]
fn preset_units_stay_off_on_the_live_medium() {
    // Audit CA-2: firstboot, updates, reports, xguard, clam and boot-ok must not run on the ISO.
    let preset = read("files/usr/lib/systemd/system-preset/50-wolos.preset");
    let enabled: Vec<&str> = preset
        .lines()
        .filter_map(|l| l.strip_prefix("enable "))
        .collect();
    assert!(enabled.len() >= 20, "{enabled:?}");
    let mut guarded = 0;
    for u in &enabled {
        let text = unit_and_dropins(u);
        let live_off = text.lines().any(|l| l.trim() == LIVE_MARK);
        let safe = LIVE_SAFE.iter().find(|(n, _, _)| n == u);
        match (live_off, safe) {
            (true, None) => guarded += 1,
            (true, Some(_)) => panic!("{u}: live-safe and guarded; drop one"),
            (false, None) => panic!(
                "{u}: enabled by the preset, runs on the live medium: add {UNITS}/{u}.d/no-live.conf \
                 ({LIVE_MARK}) or list it in LIVE_SAFE with a reason"
            ),
            (false, Some((_, cond, why))) => {
                assert!(!why.is_empty(), "{u}: a reason");
                assert!(
                    cond.is_empty() || text.lines().any(|l| l.trim() == *cond),
                    "{u}: LIVE_SAFE says {cond} keeps it off the live medium; the unit lacks it"
                );
            }
        }
    }
    for (n, _, _) in LIVE_SAFE {
        assert!(
            enabled.contains(&n),
            "LIVE_SAFE names {n}, which the preset does not enable"
        );
    }
    for must in [
        "wolos-firstboot.service",
        "wolos-update-check.timer",
        "wolos-update-download.timer",
        "wolos-report.timer",
        "wolos-xguard.service",
        "wolos-xguard-update.timer",
        "wolos-xguard-sweep.timer",
        "wolos-clam.socket",
        "wolos-boot-ok.service",
    ] {
        assert!(
            unit_and_dropins(must)
                .lines()
                .any(|l| l.trim() == LIVE_MARK),
            "{must} runs on the live medium"
        );
    }
    assert!(guarded >= 9, "{guarded} guarded");
    // The mark exists only on the live medium: the image build makes it, install-run drops it.
    let run = read("files/usr/lib/wolos/install-run");
    assert!(
        run.contains("\"$T/etc/wolos-live\""),
        "install-run copies /etc/wolos-live into the installed system"
    );
    if let Ok(b) = fs::read_to_string(repo("build.sh")) {
        assert!(
            b.contains("touch \"$A/etc/wolos-live\""),
            "build.sh no longer marks the live medium"
        );
    }
}

#[test]
fn wifi_is_iwd_under_networkmanager() {
    // Audit CA-11: one Wi-Fi stack. networkmanager depends on wpa_supplicant in Arch, so it is on
    // the disk; NetworkManager never starts it with the iwd backend, and the preset keeps it off.
    let conf = read("files/etc/NetworkManager/conf.d/wolos-iwd.conf");
    assert!(conf.contains("[device]\nwifi.backend=iwd\n"), "{conf}");
    let preset = read("files/usr/lib/systemd/system-preset/50-wolos.preset");
    for l in [
        "enable NetworkManager.service",
        "enable iwd.service",
        "disable wpa_supplicant.service",
    ] {
        assert!(preset.lines().any(|p| p == l), "preset lacks {l}");
    }
    // The live medium enables units from the package scriptlet, not this preset: NetworkManager
    // pulls iwd in itself.
    let nm = unit_and_dropins("NetworkManager.service");
    assert!(
        nm.contains("[Unit]\nWants=iwd.service\nAfter=iwd.service\n"),
        "{nm}"
    );
    if let Ok(p) = fs::read_to_string(repo("profile/packages.x86_64")) {
        let names: Vec<&str> = p
            .lines()
            .map(|l| l.split('#').next().unwrap().trim())
            .collect();
        assert!(
            names.contains(&"networkmanager") && names.contains(&"iwd"),
            "image lacks NetworkManager or iwd"
        );
        assert!(
            !names.contains(&"wpa_supplicant"),
            "wpa_supplicant listed: one Wi-Fi stack"
        );
    }
}

#[test]
fn mok_enroll_is_skipped_without_secure_boot_firmware() {
    // wolvm e2e (BW8): on firmware without Secure Boot (plain OVMF) mokutil fails; the unit must be
    // skipped there, not failed. The SecureBoot variable exists whenever the firmware has Secure
    // Boot, on or off (the person may turn it on later).
    let u = unit("wolos-mok-enroll.service");
    let conds = values(&u, "Unit", "ConditionPathExists");
    assert!(
        conds.contains(
            &"/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c".to_owned()
        ),
        "{conds:?}"
    );
    // Still only where the key screen is the way in.
    for c in [
        "/var/lib/wolos/secureboot/db.der",
        "/var/lib/wolos/secureboot/key-screen",
        "!/var/lib/wolos/secureboot/enrolled",
    ] {
        assert!(conds.contains(&c.to_owned()), "{c}: {conds:?}");
    }
}

/// QA-FILES-22: a stick mounted by one account stayed mounted after its logout, invisible and
/// unreachable for the next. The session ends it: a user unit that unmounts /run/media/<user> when
/// the user manager stops, pulled in by the session target but never stopped with it (a compositor
/// restart restarts the target and must leave the sticks mounted).
#[test]
fn removable_disks_are_unmounted_at_the_end_of_the_session() {
    let user = "files/usr/lib/systemd/user";
    let u = read(&format!("{user}/wolos-media-unmount.service"));
    assert_eq!(values(&u, "Service", "Type"), ["oneshot"]);
    assert_eq!(values(&u, "Service", "RemainAfterExit"), ["yes"]);
    assert_eq!(
        values(&u, "Service", "ExecStart"),
        ["/usr/lib/wolos/media-unmount", "--start"]
    );
    assert_eq!(
        values(&u, "Service", "ExecStop"),
        ["-/usr/lib/wolos/media-unmount", "%u"]
    );
    assert!(
        values(&u, "Unit", "PartOf").is_empty() && values(&u, "Unit", "BindsTo").is_empty(),
        "stopped with the session target, a compositor restart would unmount the sticks"
    );
    let t = read(&format!("{user}/wolos-session.target"));
    assert!(
        values(&t, "Unit", "Wants")
            .join(" ")
            .contains("wolos-media-unmount.service"),
        "the session target pulls it in"
    );
    // The script, against a folder of mounts and fake findmnt/udisksctl: only the account's own.
    let d = scratch("media-unmount");
    let media = d.join("media");
    for m in ["alice/STICK", "alice/CARD", "bob/OTHER"] {
        fs::create_dir_all(media.join(m)).unwrap();
    }
    let path = fakes(
        &d,
        &[
            ("findmnt", "echo /dev/sdX-$(basename \"$5\")"),
            ("udisksctl", ""),
        ],
    );
    let run = |who: &str| {
        Command::new("sh")
            .arg(repo("files/usr/lib/wolos/media-unmount"))
            .arg(who)
            .env("PATH", &path)
            .env("WOLOS_MEDIA", &media)
            .output()
            .unwrap()
    };
    let o = run("alice");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let l = log(&d);
    assert!(
        l.contains("udisksctl unmount --no-user-interaction -b /dev/sdX-STICK"),
        "{l}"
    );
    assert!(l.contains("-b /dev/sdX-CARD"), "{l}");
    assert!(!l.contains("OTHER"), "another account's stick stays: {l}");
    // A name that is a path is refused, an account without mounts is fine.
    assert!(!run("../etc").status.success());
    // The start does nothing (the unit only works when it stops).
    let before = log(&d);
    assert!(run("--start").status.success());
    assert_eq!(log(&d), before);
    assert!(run("carol").status.success());
}
