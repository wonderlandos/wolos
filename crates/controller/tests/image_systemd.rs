//! WolOS on systemd (ADR 0016): the rc.d scripts became units in image/files/usr/lib/systemd, devd
//! became udev, cron became a timer, dhclient hooks became a NetworkManager dispatcher script,
//! sysctl.conf.local became sysctl.d. Scripts run against fakes; units are checked key by key.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
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
const UNITS: &str = "image/files/usr/lib/systemd/system";

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
        "image/files/etc/rc.d",
        "image/files/usr/local",
        "image/files/boot",
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

#[test]
fn every_unit_runs_a_shipped_program_and_is_enabled_by_the_preset() {
    let preset = read("image/files/usr/lib/systemd/system-preset/50-wolos.preset");
    let rust = read("image/build-rust.sh");
    let mut n = 0;
    for f in fs::read_dir(repo(UNITS)).unwrap().flatten() {
        let name = f.file_name().to_string_lossy().into_owned();
        let u = fs::read_to_string(f.path()).unwrap();
        assert!(u.contains("[Unit]\nDescription="), "{name}");
        if name.ends_with(".service") {
            n += 1;
            for exec in values(&u, "Service", "ExecStart").iter().take(1) {
                let exec = exec.trim_start_matches(['-', '@', '+']);
                let ok = if let Some(b) = exec.strip_prefix("/usr/bin/") {
                    rust.contains(&format!(" {b} ")) || rust.contains(&format!(" {b};"))
                } else {
                    repo(&format!("image/files{exec}")).exists()
                };
                assert!(ok, "{name}: ExecStart {exec} is not shipped");
            }
        }
        // Timers and services started by a timer are enabled through the timer.
        let wanted = !values(&u, "Install", "WantedBy").is_empty();
        if wanted {
            assert!(
                preset.lines().any(|l| l == format!("enable {name}")),
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
    assert!(
        values(&fb, "Unit", "Before")
            .iter()
            .any(|a| a == "wold.service")
    );
    let w = unit("wold.service");
    assert_eq!(values(&w, "Service", "Restart"), ["always"]);
    assert!(
        values(&w, "Unit", "After")
            .iter()
            .any(|a| a == "wolos-hw.service")
    );
    // SIGTERM stops every job tree before wold exits; its sockets go with it.
    assert_eq!(values(&w, "Service", "KillMode"), ["mixed"]);
    let fbs = read("image/files/usr/lib/wolos/firstboot");
    assert!(
        fbs.contains("wol hw boot --online")
            && fbs.contains("/var/lib/wolos}")
            && fbs.contains("firstboot-done")
    );
}

#[test]
fn login_window_user_comes_from_sysusers() {
    let s = read("image/files/usr/lib/sysusers.d/wolos.conf");
    assert!(s.lines().any(|l| l.starts_with("u _wollogin 911 ")), "{s}");
    assert!(s.lines().any(|l| l == "m _wollogin video"), "{s}");
}

/// Hardening and no core files in home folders (a crashed compositor left a 27 MB core in ~).
#[test]
fn sysctl_hardening_and_no_core_files() {
    let s = read("image/files/usr/lib/sysctl.d/90-wolos.conf");
    for kv in [
        "kernel.dmesg_restrict = 1",
        "kernel.kptr_restrict = 2",
        "kernel.yama.ptrace_scope = 1",
        "kernel.sysrq = 0",
        "fs.protected_hardlinks = 1",
        "fs.protected_symlinks = 1",
        "fs.suid_dumpable = 0",
        "vm.max_map_count = 2147483642",
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
    let c = read("image/files/usr/lib/systemd/coredump.conf.d/wolos.conf");
    assert!(c.contains("Storage=none") && c.contains("ProcessSizeMax=0"));
}

/// The power policy follows the AC adapter (udev) and locks sessions before sleep (system-sleep).
#[test]
fn power_follows_the_adapter_and_sleep() {
    let r = read("image/files/usr/lib/udev/rules.d/90-wolos-power.rules");
    assert!(
        r.contains(r#"SUBSYSTEM=="power_supply""#)
            && r.contains(r#"ATTR{type}=="Mains""#)
            && r.contains(r#"RUN+="/usr/bin/wol power apply""#),
        "{r}"
    );
    let d = scratch("sleep");
    let path = fakes(&d, &[("wol", "")]);
    for (phase, want) in [("pre", "wol power suspend"), ("post", "wol power apply")] {
        let st = Command::new("sh")
            .arg(repo("image/files/usr/lib/systemd/system-sleep/wolos-power"))
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
    let path = fakes(&d, &[("flock", "shift 2; exec \"$@\""), ("wol", "")]);
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
            .status()
            .unwrap();
        assert!(st.success());
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(
            log(&d).contains("wol hw online"),
            runs,
            "{action}: {}",
            log(&d)
        );
    }
}

/// Daily update check: a timer runs it hourly; the script checks at most once every 20 hours,
/// offline leaves no stamp, findings are readable by every account.
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
    let run = |online: bool, arg: Option<&str>| {
        let body = if online {
            "echo 'wolos-system 0.11.1-1 -> 0.11.2-1'"
        } else {
            "exit 1"
        };
        let path = fakes(&d, &[("checkupdates", body)]);
        let mut c = Command::new("sh");
        c.arg(repo("image/files/usr/lib/wolos/update-check"))
            .env("PATH", &path)
            .env("WOLOS_UPDATES", &out);
        if let Some(a) = arg {
            c.arg(a);
        }
        assert!(c.status().unwrap().success());
    };
    run(false, None);
    assert!(!out.join("checked").exists(), "offline: no stamp");
    run(true, None);
    assert_eq!(
        fs::read_to_string(out.join("pkg")).unwrap(),
        "wolos-system 0.11.1-1 -> 0.11.2-1\n"
    );
    assert!(out.join("checked").exists());
    let n = log(&d)
        .lines()
        .filter(|l| l.starts_with("checkupdates"))
        .count();
    run(true, None);
    assert_eq!(
        log(&d)
            .lines()
            .filter(|l| l.starts_with("checkupdates"))
            .count(),
        n,
        "checked less than 20 h ago"
    );
    run(true, Some("now"));
    assert_eq!(
        log(&d)
            .lines()
            .filter(|l| l.starts_with("checkupdates"))
            .count(),
        n + 1
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(out.join("pkg")).unwrap().permissions().mode();
    assert_eq!(mode & 0o044, 0o044, "every account reads the findings");
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
        .arg(repo("image/files/usr/lib/wolos/gpu"))
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
    let n = read("desktop/overlay/etc/nftables.conf");
    for want in [
        "flush ruleset",
        "type filter hook input priority filter; policy drop;",
        "ct state established,related accept",
        "iif \"lo\" accept",
        "icmp type { echo-request, destination-unreachable } accept",
        "icmpv6 type { echo-request, destination-unreachable, packet-too-big, nd-neighbor-solicit, nd-neighbor-advert, nd-router-advert } accept",
        "udp sport 67 udp dport 68 accept",
        "udp dport 5353 accept",
        "type filter hook output priority filter; policy accept;",
    ] {
        assert!(n.contains(want), "nftables.conf lacks {want}");
    }
    assert!(!n.contains("tcp dport"), "no open TCP port by default");
}

/// One picture from power-on to the desktop (ADR 0012): plymouth with the WolOS theme, quiet kernel.
#[test]
fn boot_shows_only_the_wolos_splash() {
    let p = read("image/files/usr/share/plymouth/themes/wolos/wolos.plymouth");
    assert!(
        p.contains("ModuleName=script")
            && p.contains("ScriptFile=/usr/share/plymouth/themes/wolos/wolos.script")
    );
    let s = read("image/files/usr/share/plymouth/themes/wolos/wolos.script");
    assert!(
        s.contains("Image(\"wolos.png\")") && s.contains("Window.SetBackgroundTopColor(0, 0, 0)")
    );
    let i = read("image/repo/wolos-system/wolos-system.install");
    assert!(i.contains("plymouth-set-default-theme wolos"));
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
    let path = fakes(&d, &[("wol", ""), ("pacman", "")]);
    let run = || {
        let st = Command::new("sh")
            .arg(repo("image/files/usr/lib/wolos/update-download"))
            .env("PATH", &path)
            .env("WOLOS_AUTO_UPDATE", &conf)
            .status()
            .unwrap();
        assert!(st.success());
    };
    run();
    assert_eq!(
        log(&d),
        "wol update check --json\npacman -Swu --noconfirm --noprogressbar\n",
        "no file: on"
    );
    fs::write(&conf, wol_core::update::auto::conf(false)).unwrap();
    fs::remove_file(d.join("log")).unwrap();
    run();
    assert_eq!(log(&d), "", "download=off: nothing runs");
    assert!(!log(&d).contains("-Syu"), "never installs");

    let user = "image/files/usr/lib/systemd/user";
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
