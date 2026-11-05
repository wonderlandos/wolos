// SPDX-License-Identifier: BSD-2-Clause
//! Unit tests of core admin, split out of the parent file (`#[cfg(test)] mod tests;`).
use super::*;
use AdminAction::*;

/// Audit S4-4: a pacman request is waited for as long as wol-admind may run it; quick
/// actions keep a short wait.
#[test]
fn reply_waits_cover_the_daemon_steps() {
    let r = |action, args: &[&str]| AdminRequest {
        action,
        args: args.iter().map(|s| (*s).to_string()).collect(),
    };
    assert!(reply_secs(&r(AdminAction::PkgInstall, &["gimp"])) >= 1800);
    assert!(reply_secs(&r(AdminAction::PkgUpgrade, &[])) >= 1800);
    assert!(reply_secs(&r(AdminAction::WolInstall, &["/home/a/x.wol", "signed"])) >= 1800);
    assert!(reply_secs(&r(AdminAction::FlatpakInstall, &["org.gimp.GIMP"])) >= 1800);
    assert_eq!(step_secs(AdminAction::WifiJoin), 30);
    assert!(reply_secs(&r(AdminAction::Revoke, &[])) <= 120);
    for a in AdminAction::ALL {
        let want = if LONG_ACTIONS.contains(&a) { 1800 } else { 30 };
        assert_eq!(step_secs(a), want, "{}", a.name());
    }
}

fn r(a: AdminAction, args: &[&str]) -> AdminRequest {
    AdminRequest {
        action: a,
        args: args.iter().map(|s| (*s).to_owned()).collect(),
    }
}

#[test]
fn iface_follows_kernel_netdev_names() {
    for ok in [
        "wlan0",
        "wlp2s0",
        "wlx00c0ca123456",
        "wlp0s20f3",
        "a",
        "x".repeat(15).as_str(),
    ] {
        assert!(iface(ok), "{ok}");
    }
    for bad in [
        "",
        ".",
        "..",
        "wl/an0",
        "wl an0",
        "wlan0\n",
        "wl:an0",
        "x".repeat(16).as_str(),
        "-wlan0",
        "wlan0\t",
    ] {
        assert!(!iface(bad), "{bad:?}");
    }
}

#[test]
fn allow_list_table() {
    let long = "x".repeat(200);
    let cases: &[(AdminAction, &[&str], bool)] = &[
        (WifiPower, &["wlan0", "on"], true),
        (WifiPower, &["wlan0", "off"], true),
        (WifiPower, &["em/0", "off"], false),
        (WifiPower, &["wlan0", "destroy"], false),
        (WifiPower, &["wlan0 down", "on"], false),
        (WifiJoin, &["wlan0", "Home"], true),
        (WifiJoin, &["wlan0", "Home", "password1"], true),
        (WifiJoin, &["wlan0", "Ho\"me"], true),
        (WifiJoin, &["wlan0", "Ho\tme"], false),
        (WifiJoin, &["wlan0", "Home", "short"], false),
        (WifiJoin, &["wlan0", "Attic", "", "hidden"], true),
        (WifiJoin, &["wlan0", "Attic", "password1", "hidden"], true),
        (WifiJoin, &["wlan0", "Attic", "short", "hidden"], false),
        (WifiJoin, &["wlan0", "Attic", "password1", "visible"], false),
        (Bluetooth, &["on"], true),
        (Bluetooth, &["maybe"], false),
        (Appearance, &["dark"], true),
        (Appearance, &["light"], true),
        (Appearance, &["../x"], false),
        (Idle, &["300"], true),
        (Idle, &["0"], true),
        (Idle, &["-1"], false),
        (Idle, &["1e3"], false),
        (Idle, &["99999999"], false),
        (Keybinds, &[r#"{"super+t":["spawn","foot"]}"#], true),
        (Keybinds, &[r#"{"hyper+x":["x"]}"#], false),
        (Keybinds, &["not json"], false),
        (UserAdd, &["bob", "Bob Builder", "yes", "s3cret-pw"], true),
        (UserAdd, &["bob", "", "no", "s3cret-pw"], true),
        (UserAdd, &["bob", "Bob", "no"], false), // no password: refused
        (UserAdd, &["bob", "Bob", "no", "short"], false),
        (UserAdd, &["-G", "x", "no", "s3cret-pw"], false),
        (UserAdd, &["bob", "Bob", "maybe", "s3cret-pw"], false),
        (UserAdd, &["bob", &long, "no", "s3cret-pw"], false),
        (UserAdd, &["live", "Live", "no", "password1"], false),
        (UserAdd, &["systemd-x", "X", "no", "password1"], false),
        (UserAdd, &["git", "Git", "yes", "password1"], false),
        (UserRemove, &["bob"], true),
        (UserRemove, &["root"], false),
        (UserRemove, &["bob;id"], false),
        (UserRemove, &["bob", "with-files"], true),
        (UserRemove, &["bob", "-r"], false),
        (UserRemove, &["bob", "with-files", "x"], false),
        (TimeZone, &["America/Sao_Paulo"], true),
        (TimeZone, &["UTC"], true),
        (TimeZone, &["../../etc/passwd"], false),
        (TimeZone, &["/etc/passwd"], false),
        (TimeZone, &["-r"], false),
        (PkgUpgrade, &[], true),
        (PkgUpgrade, &["-f"], false),
        (PkgUpgrade, &["firefox", "gimp"], true),
        (PkgUpgrade, &["firefox", "--root=/x"], false),
        (PkgUpgrade, &["a,b"], false),
        (PkgRefresh, &[], true),
        (PkgRefresh, &["x"], false),
        (Revoke, &[], true),
        (Revoke, &["x"], false),
        (PkgInstall, &["steam"], true),
        (PkgInstall, &["htop"], true),
        (PkgInstall, &["-y"], false),
        (PkgInstall, &["a", "b"], false),
        (PkgRemove, &["gimp"], true),
        (
            FlatpakOverride,
            &["org.gimp.GIMP", "filesystems=home", "deny"],
            true,
        ),
        (
            FlatpakOverride,
            &["org.gimp.GIMP", "filesystems=home", "allow"],
            true,
        ),
        (FlatpakOverride, &["org.gimp.GIMP", "reset"], true),
        (
            FlatpakOverride,
            &["org.gimp.GIMP", "--filesystem=/", "allow"],
            false,
        ),
        (
            FlatpakOverride,
            &["org.gimp.GIMP", "filesystems=home", "maybe"],
            false,
        ),
        (FlatpakOverride, &["--user", "reset"], false),
        (FlatpakOverride, &[], false),
        (FlatpakInstall, &["org.gimp.GIMP"], true),
        (FlatpakInstall, &["gimp"], false),
        (FlatpakRemove, &["-y"], false),
        (PkgRemove, &["-dd"], false),
        (PkgRemove, &[], false),
        (BootEnvironment, &["wolos-a"], true),
        (BootEnvironment, &["default"], false),
        (BootEnvironment, &["-F"], false),
        (BootEnvironment, &["a;b"], false),
        (BootEnvironment, &[], false),
        (HwLoad, &["iwlwifi"], true),
        (HwLoad, &["i915"], true),
        (HwLoad, &["evil"], false),
        (HwLoad, &["/tmp/evil.ko"], false),
        (HwLoad, &["iwlwifi", "x"], false),
        (PowerMode, &["low"], true),
        (PowerMode, &["auto"], true),
        (PowerMode, &["high"], true),
        (PowerMode, &["max"], false),
        (PowerMode, &[], false),
        (SessionPower, &["sleep"], true),
        (SessionPower, &["restart"], true),
        (SessionPower, &["shutdown"], true),
        (SessionPower, &["logout"], false),
        (SessionPower, &["sleep", "now"], false),
    ];
    for (a, args, ok) in cases {
        assert_eq!(validate(&r(*a, args)).is_none(), *ok, "{a:?} {args:?}");
    }
}

/// NET2-1: `nmcli dev wifi connect <ssid> passwd-file` is not nmcli grammar (rc 2). A password
/// join is a profile plus `connection up ... passwd-file /dev/stdin`, secret on stdin only.
#[test]
fn wifi_join_uses_networkmanager_with_password_on_stdin() {
    let p = plan(&r(WifiJoin, &["wlan0", "Home", "password1"])).unwrap();
    let lines: Vec<String> = p
        .iter()
        .map(|s| match s {
            AdminStep::Run(v) => v.join(" "),
            AdminStep::RunInput { argv, .. } => format!("{} <stdin", argv.join(" ")),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        lines,
        [
            "nmcli connection add type wifi con-name Home ifname * ssid Home wifi-sec.key-mgmt wpa-psk",
            "nmcli connection up id Home passwd-file /dev/stdin <stdin",
        ]
    );
    let AdminStep::RunInput { input, .. } = &p[1] else {
        panic!("{p:?}")
    };
    assert_eq!(input, "802-11-wireless-security.psk:password1\n");
    assert!(
        !format!("{:?}", &p[..1]).contains("password1"),
        "secret never in argv"
    );
    assert!(
        !lines.iter().any(|l| l.contains("dev wifi connect")),
        "{lines:?}"
    );
    // A Wi-Fi profile of that name is saved: changed in place (autoconnect and address
    // settings survive), not deleted and not doubled. An Ethernet profile of the same name is
    // not the network.
    let join = r(WifiJoin, &["wlan0", "Home", "password1"]);
    let with = plan_with(
        &join,
        &probe(&[(
            "nmcli -t -f NAME,TYPE connection show",
            "Wired connection 1:802-3-ethernet\nHome:802-11-wireless\n",
        )]),
    )
    .unwrap();
    assert_eq!(
        shown(&with),
        "nmcli connection modify id Home wifi-sec.key-mgmt wpa-psk; \
             nmcli connection up id Home passwd-file /dev/stdin <stdin"
    );
    let eth = plan_with(
        &join,
        &probe(&[(
            "nmcli -t -f NAME,TYPE connection show",
            "Home:802-3-ethernet\n",
        )]),
    )
    .unwrap();
    assert!(shown(&eth).starts_with("nmcli connection add type wifi con-name Home"));
    // WPA3-only networks (the scan's SECURITY column) need sae; WPA2 and WPA2/WPA3 do not.
    let sae = |scan: &'static str| {
        shown(
            &plan_with(&join, &|argv: &[&str]| {
                (argv.join(" ") == "nmcli -t -f SECURITY,SSID dev wifi list")
                    .then(|| scan.to_string())
            })
            .unwrap(),
        )
    };
    assert!(sae("WPA3:Home\nWPA2:Other\n").contains("wifi-sec.key-mgmt sae"));
    assert!(sae("WPA2:Home\n").contains("wifi-sec.key-mgmt wpa-psk"));
    assert!(sae("WPA2 WPA3:Home\n").contains("wifi-sec.key-mgmt wpa-psk"));
    assert!(sae("WPA3:Home\nWPA2:Home\n").contains("wifi-sec.key-mgmt wpa-psk"));
    assert!(sae("WPA3:Other\n").contains("wifi-sec.key-mgmt wpa-psk"));
    assert_eq!(crate::wifi::nm_key_mgmt("WPA3:A\\:B\n", "A:B"), "sae");
    let p = plan(&r(WifiJoin, &["wlan0", "Attic", "", "hidden"])).unwrap();
    assert!(
        matches!(&p[..], [AdminStep::Run(v)] if v.join(" ") == "nmcli dev wifi connect Attic hidden yes")
    );
    let p = plan(&r(WifiJoin, &["wlan0", "Attic", "password1", "hidden"])).unwrap();
    assert!(
        matches!(&p[0], AdminStep::Run(v) if v.join(" ").ends_with("wpa-psk 802-11-wireless.hidden yes"))
    );
    assert!(
        !format!("{p:?}").contains("wpa_supplicant"),
        "no wpa_supplicant left"
    );
}

/// A wrong password must not leave a profile behind: the cleanup argv exists only for a
/// password join that found no saved Wi-Fi profile of that name.
#[test]
fn failed_join_removes_the_profile_it_made() {
    let join = r(WifiJoin, &["wlan0", "Home", "password1"]);
    let none = probe(&[(
        "nmcli -t -f NAME,TYPE connection show",
        "Wired connection 1:802-3-ethernet\n",
    )]);
    assert_eq!(
        wifi_join_cleanup(&join, &none)
            .map(|v| v.join(" "))
            .as_deref(),
        Some("nmcli connection delete id Home")
    );
    let saved = probe(&[(
        "nmcli -t -f NAME,TYPE connection show",
        "Home:802-11-wireless\n",
    )]);
    assert_eq!(
        wifi_join_cleanup(&join, &saved),
        None,
        "the person's own profile stays"
    );
    assert_eq!(
        wifi_join_cleanup(&r(WifiJoin, &["wlan0", "Home"]), &none),
        None,
        "open network"
    );
    assert_eq!(
        wifi_join_cleanup(&r(WifiPower, &["wlan0", "on"]), &none),
        None
    );
}

/// RC-01: the argv is checked against the real nmcli when the tool is installed. The old form
/// is refused as a usage error (rc 2); the new `up` form is understood (NetworkManager then
/// answers "unknown connection" or "not running", never a usage error), and the profile argv
/// parses (`--offline` prints the keyfile and touches nothing).
#[test]
fn wifi_join_argv_is_valid_nmcli_grammar() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    if Command::new("nmcli").arg("--version").output().is_err() {
        return; // not installed here: the journey qa100-net2-wifi-join-argv runs it on the image
    }
    let run = |argv: &[String], offline: bool, input: &str| {
        let mut c = Command::new(&argv[0]);
        if offline {
            c.arg("--offline");
        }
        let mut child = c
            .args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input.as_bytes()).ok();
        let o = child.wait_with_output().unwrap();
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        )
    };
    let old: Vec<String> = [
        "nmcli",
        "dev",
        "wifi",
        "connect",
        "QaNoSuchNet",
        "passwd-file",
        "/dev/stdin",
    ]
    .map(str::to_owned)
    .to_vec();
    let (rc, err, _) = run(&old, false, "802-11-wireless-security.psk:password1\n");
    assert_eq!(rc, Some(2), "{err}");
    assert!(err.contains("invalid extra argument"), "{err}");
    for hidden in [false, true] {
        for km in ["wpa-psk", "sae"] {
            for step in crate::wifi::nm_profile_argvs("Qa No Such Net", hidden, km, false) {
                let (rc, err, out) = run(&step, true, "");
                assert_eq!(rc, Some(0), "{step:?}: {err}");
                assert!(out.contains(&format!("key-mgmt={km}")), "{out}");
                assert_eq!(out.contains("hidden=true"), hidden, "{out}");
            }
        }
        // The in-place change of a saved profile and the cleanup parse as well (NetworkManager
        // answers "unknown connection" or "not running", never a usage error).
        for step in [
            crate::wifi::nm_profile_argvs("QaNoSuchNet", hidden, "sae", true).remove(0),
            crate::wifi::nm_cleanup_argv("QaNoSuchNet"),
        ] {
            let (rc, err, _) = run(&step, false, "");
            assert_ne!(rc, Some(2), "usage error {step:?}: {err}");
            assert!(
                !err.contains("invalid") && !err.contains("Usage"),
                "{step:?}: {err}"
            );
        }
    }
    let (rc, err, _) = run(&crate::wifi::nm_up_argv("QaNoSuchNet"), false, "x\n");
    assert_ne!(rc, Some(2), "usage error: {err}");
    assert!(
        !err.contains("invalid extra argument") && !err.contains("Usage"),
        "{err}"
    );
}

#[test]
fn plan_maps_to_fixed_argv() {
    let show = |s: &AdminStep| match s {
        AdminStep::Run(v) => v.join(" "),
        AdminStep::RunInput { argv, .. } => format!("{} <stdin", argv.join(" ")),
        AdminStep::Write { path, .. } => format!("write {path}"),
    };
    let cases: &[(AdminAction, &[&str], &str)] = &[
        (SessionPower, &["sleep"], "systemctl suspend"),
        (SessionPower, &["restart"], "systemctl reboot"),
        (SessionPower, &["shutdown"], "systemctl poweroff"),
        (WifiPower, &["wlan0", "off"], "nmcli radio wifi off"),
        (
            Bluetooth,
            &["on"],
            "systemctl reset-failed bluetooth.service; systemctl start bluetooth.service",
        ),
        (Bluetooth, &["off"], "systemctl stop bluetooth.service"),
        (
            UserAdd,
            &["bob", "Bob B", "yes", "s3cret-pw"],
            "useradd -m -c Bob B -s /usr/bin/zsh -G wheel,seat,video bob; chpasswd <stdin; /usr/lib/wolos/home-quota",
        ),
        (UserRemove, &["bob"], "userdel bob"),
        (UserRemove, &["bob", "with-files"], "userdel -r bob"),
        (
            TimeZone,
            &["Europe/Lisbon"],
            "timedatectl set-timezone Europe/Lisbon",
        ),
        (PkgRefresh, &[], "/usr/lib/wolos/update-check now"),
        (
            BootEnvironment,
            &["wolos-b"],
            "bootctl set-default wolos-b.conf",
        ),
        (
            PkgInstall,
            &["steam"],
            "wol pkg -- -S --needed --noprogressbar --noconfirm steam",
        ),
        (PkgRemove, &["gimp"], "wol pkg -- -Rns --noconfirm gimp"),
        (
            FlatpakOverride,
            &["org.gimp.GIMP", "shared=network", "deny"],
            "flatpak override --system --unshare=network org.gimp.GIMP",
        ),
        (
            FlatpakOverride,
            &["org.gimp.GIMP", "reset"],
            "flatpak override --system --reset org.gimp.GIMP",
        ),
        (
            FlatpakInstall,
            &["org.gimp.GIMP"],
            "flatpak install --system --noninteractive -y flathub org.gimp.GIMP",
        ),
        (
            FlatpakRemove,
            &["org.gimp.GIMP"],
            "flatpak uninstall --system --noninteractive -y org.gimp.GIMP",
        ),
        (WifiJoin, &["wlan0", "Home"], "nmcli dev wifi connect Home"),
        (Appearance, &["dark"], "write /etc/wolos/appearance"),
        (Idle, &["60"], "write /etc/wolos/idle.json"),
        (Revoke, &[], ""),
        (HwLoad, &["i2c_hid_acpi"], "modprobe i2c_hid_acpi"),
        (BootEnroll, &[], "wol boot enroll"),
    ];
    for (a, args, want) in cases {
        let steps = plan(&r(*a, args)).unwrap();
        assert_eq!(steps.iter().map(show).collect::<Vec<_>>().join("; "), *want);
    }
    assert!(plan(&r(PkgUpgrade, &["-f"])).is_err());
    assert!(plan(&r(BootEnroll, &["--force"])).is_err());
    assert!(
        !is_user_level(BootEnroll),
        "Secure Boot setup is an administrator's change"
    );
}

fn shown(steps: &[AdminStep]) -> String {
    steps
        .iter()
        .map(|s| match s {
            AdminStep::Run(v) => v.join(" "),
            AdminStep::RunInput { argv, .. } => format!("{} <stdin", argv.join(" ")),
            AdminStep::Write { path, .. } => format!("write {path}"),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// (probe command line, stdout) answers.
type Facts = &'static [(&'static str, &'static str)];

/// Probe that answers `facts`; anything else failed.
fn probe(facts: Facts) -> impl Fn(&[&str]) -> Option<String> {
    move |argv| {
        let line = argv.join(" ");
        facts.iter().find(|f| f.0 == line).map(|f| f.1.to_string())
    }
}

const SPEED_SHIFT: &[(&str, &str)] = &[("nproc", "4\n"), ("powerprofilesctl get", "balanced\n")];

/// Update Now installs the `[wolos]` fixes inside the pinned release in the other root slot;
/// wol-admind plans it itself from the running slot (the restore point), so the shared plan
/// has no steps and a client waits for the whole slot change.
#[test]
fn package_upgrade_is_planned_by_the_daemon() {
    assert_eq!(plan_with(&r(PkgUpgrade, &[]), &probe(&[])), Ok(vec![]));
    let most = crate::update::pkg::update_now_most_steps(&[]);
    assert!(most >= 9, "{most}");
    assert_eq!(
        reply_secs(&r(PkgUpgrade, &[])),
        u32::try_from(most).unwrap() * LONG_STEP_SECS + REPLY_MARGIN_SECS
    );
}

/// The mode is saved for `wol power apply` (AC changes, boot, resume); the plan switches the
/// power-profiles-daemon profile only when it differs.
#[test]
fn power_mode_saves_the_mode_and_runs_the_policy() {
    let p = plan_with(&r(PowerMode, &["low"]), &probe(SPEED_SHIFT)).unwrap();
    assert_eq!(
        shown(&p),
        "write /etc/wolos/power.json; powerprofilesctl set power-saver"
    );
    let AdminStep::Write { content, mode, .. } = &p[0] else {
        panic!()
    };
    assert_eq!(
        (content.render(Some("{\"mode\":\"auto\"}")).as_str(), *mode),
        ("{\"mode\":\"low\"}\n", 0o644)
    );
    for (m, want) in [
        ("auto", "write /etc/wolos/power.json"),
        (
            "high",
            "write /etc/wolos/power.json; powerprofilesctl set performance",
        ),
    ] {
        let p = plan_with(&r(PowerMode, &[m]), &probe(SPEED_SHIFT)).unwrap();
        assert_eq!(shown(&p), want, "{m}");
    }
}

#[test]
fn power_mode_steers_what_the_machine_has() {
    assert!(
        plan_with(&r(PowerMode, &["low"]), &probe(&[("nproc", "4\n")])).is_err(),
        "no power-profiles-daemon: nothing to control"
    );
    assert!(plan(&r(PowerMode, &["low"])).is_err(), "no facts");
}

#[test]
fn settings_verbs_allow_list() {
    let app = "/usr/bin/wayvnc";
    let cases: &[(AdminAction, &[&str], bool)] = &[
        (Ntp, &["on"], true),
        (Ntp, &["off"], true),
        (Ntp, &["yes"], false),
        (Ntp, &[], false),
        (Hostname, &["julias-laptop"], true),
        (Hostname, &["-n"], false),
        (Hostname, &["a.b"], false),
        (Hostname, &["x", "y"], false),
        (RemoteLogin, &["on"], true),
        (RemoteLogin, &["start"], false),
        (Firewall, &["off"], true),
        (Firewall, &["on", "x"], false),
        (MalwareSignatures, &["on"], true),
        (MalwareSignatures, &["maybe"], false),
        (AirPlayReceiver, &["on"], true),
        (AirPlayReceiver, &["off"], true),
        (AirPlayReceiver, &["start"], false),
        (AirPlayReceiver, &[], false),
        (Lid, &["sleep"], true),
        (Lid, &["nothing"], true),
        (Lid, &["S5"], false),
        (AutoUpdate, &["on"], true),
        (AutoUpdate, &["off"], true),
        (AutoUpdate, &["yes"], false),
        (AutoUpdate, &[], false),
        (AutoUpdate, &["metered", "on"], true),
        (AutoUpdate, &["metered", "off"], true),
        (AutoUpdate, &["metered", "maybe"], false),
        (AutoUpdate, &["roaming", "on"], false),
        (AutoUpdate, &["on", "off"], false),
        (AutoUpdate, &["metered"], false),
        (MirrorGate, &[], true),
        (MirrorGate, &["--yes"], false),
        (MirrorGate, &["on"], false),
        (CoolingHelper, &["on"], true),
        (CoolingHelper, &["off"], true),
        (CoolingHelper, &["maybe"], false),
        (CoolingHelper, &[], false),
        (Reports, &["on"], true),
        (Reports, &["off"], true),
        (Reports, &["yes"], false),
        (Reports, &[], false),
        (Reports, &["on", "off"], false),
        (
            PrinterAdd,
            &["HP_LaserJet", "ipp://10.0.0.5/ipp/print"],
            true,
        ),
        (
            PrinterAdd,
            &["HP LaserJet", "ipp://10.0.0.5/ipp/print"],
            false,
        ),
        (PrinterAdd, &["HP", "file:///etc/shadow"], false),
        (PrinterAdd, &["HP", "-E"], false),
        (PrinterAdd, &["HP"], false),
        (PrinterRemove, &["HP_LaserJet"], true),
        (PrinterRemove, &["-x"], false),
        (PrinterDefault, &["HP_LaserJet"], true),
        (PrinterDefault, &["a/b"], false),
        (Consent, &[app, "window-list", "allow"], true),
        (Consent, &[app, "input-injection", "deny"], true),
        (Consent, &[app, "shell-layer", "allow"], false),
        (Consent, &[app, "screen-capture", "allow"], false),
        (Consent, &[app, "screen-capture", "deny"], false),
        (Consent, &[app, "window-list", "maybe"], false),
        (Consent, &["wayvnc", "window-list", "allow"], false),
        (Consent, &["/usr/../etc/x", "window-list", "allow"], false),
        (Consent, &[app, "window-list"], false),
        (BtPair, &["00:11:22:aa:bb:cc", "AirPods Pro"], true),
        (BtPair, &["00:11:22:AA:BB:CC", "MX Keys"], true),
        (BtPair, &["00:11:22:aa:bb", "x"], false),
        (BtPair, &["00:11:22:aa:bb:cc", ""], false),
        (BtPair, &["00:11:22:aa:bb:cc", "a\"b"], false),
        (BtPair, &["00:11:22:aa:bb:cc", "a}b"], false),
        (BtPair, &["00:11:22:aa:bb:cc", &"x".repeat(65)], false),
        (BtForget, &["00:11:22:aa:bb:cc"], true),
        (BtForget, &["all"], false),
        (WifiForget, &["wlan0", "Home"], true),
        (WifiForget, &["wl an0", "Home"], false),
        (WifiForget, &["wlan0", "Ho\"me"], true),
        (WifiForget, &["wlan0", "Ho\nme"], false),
        (WifiForget, &["wlan0"], false),
    ];
    for (a, args, ok) in cases {
        assert_eq!(validate(&r(*a, args)).is_none(), *ok, "{a:?} {args:?}");
    }
}

/// Screen recording is no consent grant: refused with why, not as a malformed request.
#[test]
fn consent_refuses_screen_capture_with_why() {
    for verdict in ["allow", "deny"] {
        let why = validate(&r(
            AdminAction::Consent,
            &["/usr/bin/obs", "screen-capture", verdict],
        ))
        .unwrap();
        assert!(
            why.contains("not a grant") && why.contains("ask each time"),
            "{why}"
        );
    }
    assert_eq!(
        validate(&r(
            AdminAction::Consent,
            &["/usr/bin/obs", "teleport", "allow"]
        ))
        .as_deref(),
        Some("invalid consent request")
    );
}

/// privilege-cve M4: Remote Login opens port 22 through the firewall's include file while it
/// is on (rate-limited new connections) and closes it again when it is off.
#[test]
fn remote_login_opens_and_closes_port_22_in_the_firewall() {
    let rule = |on: &str| {
        plan(&r(RemoteLogin, &[on]))
            .unwrap()
            .into_iter()
            .find_map(|s| match s {
                AdminStep::Write {
                    path,
                    mode,
                    content: WriteContent::Fixed(t),
                } => Some((path, mode, t)),
                _ => None,
            })
            .unwrap()
    };
    let (path, mode, on) = rule("on");
    assert_eq!(path, crate::sharing::SSH_RULE_FILE);
    assert!(path.starts_with(crate::sharing::FIREWALL_INCLUDE_DIR));
    assert_eq!(mode, 0o644);
    assert!(
        on.lines()
            .any(|l| l == "tcp dport 22 ct state new limit rate 10/minute accept"),
        "{on}"
    );
    let (_, _, off) = rule("off");
    assert!(
        off.lines()
            .all(|l| l.trim().is_empty() || l.starts_with('#')),
        "{off}"
    );
}

/// QA-NET-3/PRIV-5: the rule file is only a file until the ruleset is loaded again. The reload
/// is `nft -f` (the unit's `reload` is a no-op when the oneshot is inactive), and it is
/// skipped when the firewall is off: reloading would turn it on.
#[test]
fn remote_login_reloads_ruleset() {
    let enabled = |state| plan_with(&r(RemoteLogin, &["on"]), &probe(state)).unwrap();
    let on = shown(&enabled(&[(
        "systemctl show -p UnitFileState --value nftables.service",
        "enabled\n",
    )]));
    assert_eq!(
        on,
        "write /etc/wolos/nftables.d/ssh.conf; nft -f /etc/wolos/nftables.conf; systemctl enable --now sshd.service"
    );
    let off = shown(&enabled(&[(
        "systemctl show -p UnitFileState --value nftables.service",
        "disabled\n",
    )]));
    assert_eq!(
        off,
        "write /etc/wolos/nftables.d/ssh.conf; systemctl enable --now sshd.service"
    );
    let closing = plan_with(
        &r(RemoteLogin, &["off"]),
        &probe(&[(
            "systemctl show -p UnitFileState --value nftables.service",
            "enabled\n",
        )]),
    )
    .unwrap();
    assert_eq!(
        shown(&closing),
        "systemctl disable --now sshd.service; write /etc/wolos/nftables.d/ssh.conf; nft -f /etc/wolos/nftables.conf"
    );
    assert!(!on.contains("systemctl reload") && !on.contains("try-reload"));
}

/// QA-NET-1: Firewall Off unloads the rules even when the unit reads "inactive" (a oneshot
/// that already ran), and Firewall On is only `enable --now`.
#[test]
fn firewall_off_deletes_table() {
    assert_eq!(
        shown(&plan(&r(Firewall, &["off"])).unwrap()),
        "systemctl disable --now nftables.service; nft destroy table inet wolos"
    );
    assert_eq!(
        shown(&plan(&r(Firewall, &["on"])).unwrap()),
        "systemctl enable --now nftables.service"
    );
}

#[test]
fn settings_verbs_plan() {
    let printer = ["HP", "ipp://10.0.0.5/ipp/print"];
    let app = "/usr/bin/wayvnc";
    let cases: &[(AdminAction, &[&str], Facts, &str)] = &[
        (Ntp, &["on"], &[], "timedatectl set-ntp true"),
        (Ntp, &["off"], &[], "timedatectl set-ntp false"),
        (
            RemoteLogin,
            &["on"],
            &[],
            "write /etc/wolos/nftables.d/ssh.conf; nft -f /etc/wolos/nftables.conf; systemctl enable --now sshd.service",
        ),
        (
            RemoteLogin,
            &["off"],
            &[],
            "systemctl disable --now sshd.service; write /etc/wolos/nftables.d/ssh.conf; nft -f /etc/wolos/nftables.conf",
        ),
        (
            AirPlayReceiver,
            &["on"],
            &[],
            "write /etc/wolos/nftables.d/airplay.conf; nft -f /etc/wolos/nftables.conf; systemctl enable --now shairport-sync.service",
        ),
        (
            AirPlayReceiver,
            &["off"],
            &[],
            "systemctl disable --now shairport-sync.service; write /etc/wolos/nftables.d/airplay.conf; nft -f /etc/wolos/nftables.conf",
        ),
        (
            Hostname,
            &["julias-laptop"],
            &[],
            "hostnamectl set-hostname julias-laptop",
        ),
        (
            Firewall,
            &["on"],
            &[],
            "systemctl enable --now nftables.service",
        ),
        (
            Firewall,
            &["off"],
            &[],
            "systemctl disable --now nftables.service; nft destroy table inet wolos",
        ),
        (
            Lid,
            &["nothing"],
            &[],
            "write /etc/systemd/logind.conf.d/50-wolos-lid.conf; systemctl kill -s HUP systemd-logind",
        ),
        (
            PrinterAdd,
            &printer,
            &[],
            "lpadmin -p HP -E -v ipp://10.0.0.5/ipp/print -m everywhere",
        ),
        (PrinterRemove, &["HP"], &[], "lpadmin -x HP"),
        (PrinterDefault, &["HP"], &[], "lpadmin -d HP"),
        (
            Consent,
            &[app, "window-list", "allow"],
            &[],
            "write /etc/wolos/consent.json",
        ),
        (
            BtPair,
            &["00:11:22:aa:bb:cc", "MX Keys"],
            &[],
            "bluetoothctl pair 00:11:22:aa:bb:cc; bluetoothctl trust 00:11:22:aa:bb:cc; bluetoothctl connect 00:11:22:aa:bb:cc",
        ),
        (
            BtForget,
            &["00:11:22:aa:bb:cc"],
            &[],
            "bluetoothctl remove 00:11:22:aa:bb:cc",
        ),
        (
            WifiForget,
            &["wlan0", "Home"],
            &[],
            "nmcli connection delete id Home",
        ),
    ];
    for (a, args, facts, want) in cases {
        let p = plan_with(&r(*a, args), &probe(facts)).unwrap();
        assert_eq!(shown(&p), *want, "{a:?} {args:?} {facts:?}");
    }
}

/// Merging writes keep everything else in the file.
#[test]
fn settings_verbs_merge_files() {
    let content = |a: AdminAction, args: &[&str]| {
        plan(&r(a, args))
            .unwrap()
            .into_iter()
            .find_map(|s| match s {
                AdminStep::Write { content, mode, .. } => Some((content, mode)),
                _ => None,
            })
            .unwrap()
    };
    let (c, mode) = content(AutoUpdate, &["off"]);
    assert_eq!(
        (c.render(None), mode),
        (crate::update::auto::conf(false), 0o644)
    );
    // QA-100 metered download: each switch keeps the other's line.
    let (c, mode) = content(AutoUpdate, &["metered", "on"]);
    assert_eq!(mode, 0o644);
    let both = c.render(Some(&crate::update::auto::conf(false)));
    assert!(
        both.contains("download=off") && both.contains("metered=on"),
        "{both}"
    );
    assert!(crate::update::auto::metered_allowed(Some(&both)));
    assert!(!crate::update::auto::downloads(Some(&both)));
    let (c, _) = content(AutoUpdate, &["on"]);
    let back = c.render(Some(&both));
    assert!(
        back.contains("download=on") && back.contains("metered=on"),
        "{back}"
    );
    assert_eq!(
        back.matches("download=").count(),
        1,
        "replaced, not added: {back}"
    );
    let (c, _) = content(AutoUpdate, &["metered", "off"]);
    assert!(!crate::update::auto::metered_allowed(Some(
        &c.render(Some(&back))
    )));
    let (c, mode) = content(Lid, &["sleep"]);
    assert_eq!(
        (c.render(Some("[Login]\nHandleLidSwitch=ignore\n")), mode),
        (power::lid_conf(power::Lid::Sleep), 0o644)
    );
    let (c, mode) = content(Consent, &["/usr/bin/wayvnc", "window-list", "allow"]);
    let cur = r#"{"grants":[{"app":"/usr/bin/wlrctl","capability":"window-list"}]}"#;
    assert_eq!(crate::consent::parse(&c.render(Some(cur))).len(), 2);
    assert_eq!(mode, 0o644);
    let (c, _) = content(Consent, &["/usr/bin/wlrctl", "window-list", "deny"]);
    assert!(crate::consent::parse(&c.render(Some(cur))).is_empty());
    // An old screen-capture entry is dropped by the next write.
    let old = r#"{"grants":[{"app":"/usr/bin/grim","capability":"screen-capture"}]}"#;
    assert!(!c.render(Some(old)).contains("screen-capture"));
}

/// NET-3: the proxy writes its own file and the login environment; DNS over TLS writes the
/// resolved drop-in and restarts resolved. Both ask for the password every time, and a
/// value that could break out of the shell file is refused before any step.
#[test]
fn proxy_and_dns_over_tls_plans() {
    let steps = plan(&r(Proxy, &["manual", "proxy.lan:3128", "-", "-", "-"])).unwrap();
    let paths: Vec<&str> = steps
        .iter()
        .filter_map(|s| match s {
            AdminStep::Write { path, .. } => Some(path.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        paths,
        [crate::netconf::PROXY_CONF, crate::netconf::PROXY_PROFILE]
    );
    let AdminStep::Write { content, mode, .. } = &steps[1] else {
        panic!("{steps:?}")
    };
    assert_eq!(*mode, 0o644);
    assert!(
        content
            .render(None)
            .contains("export http_proxy='http://proxy.lan:3128/'")
    );
    for bad in [
        &["manual", "proxy.lan:3128';id;'", "-", "-", "-"][..],
        &["auto", "http://x/$(id)", "-"],
        &[],
    ] {
        assert!(validate(&r(Proxy, bad)).is_some(), "{bad:?}");
    }
    let steps = plan(&r(DnsOverTls, &["opportunistic", "9.9.9.9#dns.quad9.net"])).unwrap();
    assert_eq!(steps.len(), 3);
    assert_eq!(
        steps[2],
        argv(&["systemctl", "restart", "systemd-resolved.service"])
    );
    let AdminStep::Write { path, content, .. } = &steps[1] else {
        panic!("{steps:?}")
    };
    assert_eq!(path, crate::netconf::DOT_DROPIN);
    assert!(
        content
            .render(None)
            .contains("DNSOverTLS=opportunistic\nDNS=9.9.9.9#dns.quad9.net\n")
    );
    assert!(
        validate(&r(DnsOverTls, &["strict"])).is_some(),
        "strict needs a server"
    );
    assert!(validate(&r(DnsOverTls, &["off", "x;y"])).is_some());
    assert!(needs_fresh_password(&r(Proxy, &["off"])));
    assert!(needs_fresh_password(&r(DnsOverTls, &["off"])));
}

/// A printer URI may carry credentials (`ipp://user:pass@host`): never logged.
#[test]
fn describe_hides_printer_credentials() {
    assert_eq!(
        describe(&r(
            PrinterAdd,
            &["HP", "ipps://bob:hunter22@print.example/ipp"]
        )),
        "printer-add HP ipps://print.example/ipp"
    );
    assert_eq!(
        describe(&r(PrinterAdd, &["HP", "ipp://10.0.0.5/a@b"])),
        "printer-add HP ipp://10.0.0.5/a@b"
    );
    assert_eq!(describe(&r(Hostname, &["wolos"])), "hostname wolos");
}

/// QA-100 error reports switch: the sender's own flag, then the timer follows.
#[test]
fn reports_switch_writes_the_flag_and_the_timer() {
    for (arg, flag, verb) in [("on", "send=1\n", "enable"), ("off", "send=0\n", "disable")] {
        let steps = plan(&r(Reports, &[arg])).unwrap();
        assert_eq!(steps.len(), 2, "{steps:?}");
        let AdminStep::Write {
            path,
            mode,
            content,
        } = &steps[0]
        else {
            panic!("{steps:?}")
        };
        assert_eq!((path.as_str(), *mode), (REPORTS_CONF, 0o644));
        assert_eq!(content.render(None), flag);
        assert_eq!(
            steps[1],
            AdminStep::Run(
                ["systemctl", verb, "--now", REPORTS_TIMER]
                    .map(String::from)
                    .to_vec()
            )
        );
    }
    assert_eq!(REPORTS_CONF, "/etc/wolos/reports.conf");
    assert!(!needs_fresh_password(&r(Reports, &["on"])));
    assert_eq!(describe(&r(Reports, &["on"])), "reports on");
}

#[test]
fn write_contents() {
    let p = plan(&r(Idle, &["60"])).unwrap();
    let AdminStep::Write { content, mode, .. } = &p[0] else {
        panic!()
    };
    assert_eq!(
        (content.render(None).as_str(), *mode),
        (r#"{"lock_after_secs":60}"#, 0o644)
    );
}

/// QA-LIVE-17: a new account sets (again) the cap on the homes' subvolume, last, after the
/// account and its password exist; the helper takes no argument that could carry a secret.
#[test]
fn a_new_account_sets_the_home_quota() {
    let p = plan(&r(UserAdd, &["bob", "Bob", "no", "s3cret-pw"])).unwrap();
    assert_eq!(p.len(), 3);
    assert!(matches!(&p[2], AdminStep::Run(v) if v == &["/usr/lib/wolos/home-quota"]));
    assert_eq!(users::HOME_QUOTA, "/usr/lib/wolos/home-quota");
}

#[test]
fn user_password_goes_to_stdin_only() {
    let p = plan(&r(UserAdd, &["bob", "Bob", "no", "s3cret-pw"])).unwrap();
    let AdminStep::RunInput { argv, input } = &p[1] else {
        panic!()
    };
    assert_eq!(input, "bob:s3cret-pw\n");
    assert!(argv.iter().all(|a| !a.contains("s3cret")));
    assert!(
        !p.iter()
            .any(|s| matches!(s, AdminStep::Run(v) if v.iter().any(|a| a.contains("s3cret"))))
    );
    assert_eq!(
        describe(&r(UserAdd, &["bob", "Bob", "no", "s3cret-pw"])),
        "user-add bob"
    );
}

/// One path changes the account password and the disk's password slot with it.
#[test]
fn a_new_password_reaches_the_account_and_the_disk_on_stdin_only() {
    assert!(validate(&r(UserPassword, &["ann", "new-s3cret"])).is_none());
    assert!(validate(&r(UserPassword, &["ann", "short"])).is_some());
    assert!(validate(&r(UserPassword, &["root", "new-s3cret"])).is_some());
    assert!(validate(&r(UserPassword, &["-G", "new-s3cret"])).is_some());
    let p = plan(&r(UserPassword, &["ann", "new-s3cret"])).unwrap();
    let steps: Vec<(String, String)> = p
        .iter()
        .map(|s| match s {
            AdminStep::RunInput { argv, input } => (argv.join(" "), input.clone()),
            _ => panic!("secrets only on stdin"),
        })
        .collect();
    assert_eq!(
        steps,
        [
            ("chpasswd".into(), "ann:new-s3cret\n".into()),
            ("wol encrypt password ann".into(), "new-s3cret\n".into())
        ]
    );
    assert_eq!(
        describe(&r(UserPassword, &["ann", "new-s3cret"])),
        "user-password ann"
    );
    assert!(
        needs_fresh_password(&r(UserPassword, &["ann", "new-s3cret"])),
        "an open session alone never changes the password"
    );
}

#[test]
fn every_action_is_classified() {
    for a in AdminAction::ALL {
        let rows = FRESH_PASSWORD.iter().filter(|(x, _)| *x == a).count();
        assert_eq!(rows, 1, "{a:?} needs exactly one row in FRESH_PASSWORD");
    }
    assert_eq!(FRESH_PASSWORD.len(), AdminAction::ALL.len());
    for a in [
        UserAdd,
        UserRemove,
        UserPassword,
        RemoteLogin,
        Firewall,
        Consent,
        FlatpakOverride,
        BootEnvironment,
        AurOpen,
        BootEnroll,
        PkgInstallFile,
        SystemRollback,
        SystemChannel,
        Proxy,
        DnsOverTls,
    ] {
        assert_eq!(fresh_rule(a), Fresh::Always, "{a:?}");
    }
    assert!(!needs_fresh_password(&r(WolInstall, &["/a.wol", "signed"])));
    assert!(needs_fresh_password(&r(
        WolInstall,
        &["/a.wol", "unsigned"]
    )));
    assert!(!needs_fresh_password(&r(PkgUpgrade, &[])));
}

/// Audit C7-1 (ADR 0034): what wold used to run as root after a plain Allow is an
/// administrator action. A local package installs only wold's own staged copy, through the
/// sealed-system pacman with `--` before the path; never a path the caller names.
#[test]
fn root_package_changes_are_admin_actions_on_the_staged_copy() {
    let staged = format!(
        "{}/0123456789abcdef0123456789ABCDEF/x-1-1-x86_64.pkg.tar.zst",
        shared::package::STAGING_DIR
    );
    let sha = "0123456789abcdef".repeat(4);
    let steps = plan(&r(PkgInstallFile, &[&staged, &sha])).unwrap();
    assert_eq!(
        shown(&steps),
        format!("wol pkg -- -U --noconfirm -- {staged}")
    );
    let s = shared::package::STAGING_DIR;
    for bad in [
        "/home/ann/x-1-1-x86_64.pkg.tar.zst".to_owned(),
        format!("{s}/x-1-1-x86_64.pkg.tar.zst"),
        format!("{s}/1234-5678-abc/x-1-1-x86_64.pkg.tar.zst"),
        format!("{s}/0123456789abcdef0123456789abcdef/../x.pkg.tar.zst"),
        format!("{s}/0123456789abcdef0123456789abcdef/a/x.pkg.tar.zst"),
        format!("{s}/0123456789abcdef0123456789abcdef/-x.pkg.tar.zst"),
        format!("{s}/0123456789abcdef0123456789abcdef/x.tar"),
        format!("{s}/0123456789abcdef0123456789abcdeg/x.pkg.tar.zst"),
        format!("{s}/0123456789abcdef0123456789abcdef/x\n.pkg.tar.zst"),
    ] {
        assert!(
            validate(&r(PkgInstallFile, &[&bad, &sha])).is_some(),
            "{bad}"
        );
    }
    assert!(validate(&r(PkgInstallFile, &[])).is_some());
    // ADR 0034: the SHA-256 the sheet showed comes along (wol-admind checks it) and is logged.
    assert!(
        validate(&r(PkgInstallFile, &[&staged])).is_some(),
        "no digest"
    );
    let upper = sha.to_uppercase();
    let short = &sha[..63];
    for bad in [upper.as_str(), short, "", &format!("{sha}0")] {
        assert!(
            validate(&r(PkgInstallFile, &[&staged, bad])).is_some(),
            "{bad}"
        );
    }
    assert_eq!(
        describe(&r(PkgInstallFile, &[&staged, &sha])),
        format!("pkg-install-file {staged} {sha}")
    );
    assert_eq!(
        shown(&plan(&r(SystemRollback, &[])).unwrap()),
        "wol update rollback"
    );
    assert!(validate(&r(SystemRollback, &["x"])).is_some());
    let ch = plan(&r(SystemChannel, &["stable"])).unwrap();
    assert!(matches!(
        &ch[..],
        [AdminStep::Write { path, mode: 0o644, content: WriteContent::Fixed(t) }]
            if path == CHANNEL_FILE && t == "stable\n"
    ));
    // QA-STORE-68: 0.13 offers stable only; a testing channel is never written.
    for bad in ["alpha", "", "--x", "beta\nnightly", "beta", "nightly"] {
        assert!(validate(&r(SystemChannel, &[bad])).is_some(), "{bad:?}");
    }
    for a in [PkgInstallFile, SystemRollback, SystemChannel] {
        assert!(!is_user_level(a), "{a:?} is an administrator's change");
        assert!(needs_fresh_password(&r(a, &[])), "{a:?}");
    }
    assert!(LONG_ACTIONS.contains(&PkgInstallFile));
}

#[test]
fn describe_hides_secrets() {
    assert_eq!(
        describe(&r(WifiJoin, &["wlan0", "Home", "password1"])),
        "wifi-join wlan0 ssid=Home"
    );
    assert_eq!(describe(&r(Keybinds, &["{}"])), "keybinds");
    assert_eq!(describe(&r(Idle, &["60"])), "idle 60");
    assert_eq!(describe(&r(Revoke, &[])), "revoke");
}

#[test]
fn policy_table() {
    let group = "wheel:*:0:root,klee\nstaff:*:20:bob\n";
    assert!(is_admin("klee", 1001, group));
    assert!(!is_admin("bob", 1002, group));
    assert!(is_admin("bob", 0, group));
    use AdminDecision as D;
    for (uid, admin, cached, pw, want) in [
        (Some(0), false, false, false, D::Allow),
        (None, true, true, true, D::NotAdmin),
        (Some(1001), false, true, true, D::NotAdmin),
        (Some(1001), true, true, false, D::Allow),
        (Some(1001), true, false, true, D::Authenticate),
        (Some(1001), true, false, false, D::NeedPassword),
    ] {
        assert_eq!(decide(uid, admin, cached, pw), want);
    }
}

/// The person at the computer joins Wi-Fi, pairs a keyboard, sleeps or shuts down without a
/// password. Everything system-wide still needs an administrator; restart and shut
/// down too while another account has a desktop session (it would lose its work).
#[test]
fn console_user_needs_no_password_for_everyday_changes() {
    for (a, user_level) in [
        (PowerMode, true),
        (WifiJoin, true),
        (WifiPower, true),
        (WifiForget, true),
        (Bluetooth, true),
        (BtPair, true),
        (BtForget, true),
        (SessionPower, true),
        (PkgRefresh, true),
        (PkgUpgrade, false),
        (UserAdd, false),
        (Firewall, false),
        (MalwareSignatures, false),
        (AirPlayReceiver, false),
        (Consent, false),
        (HwLoad, false),
        (RemoteLogin, false),
    ] {
        assert_eq!(is_user_level(a), user_level, "{a:?}");
    }
    let join = r(WifiJoin, &["wlan0", "Home", "psk"]);
    let off = r(SessionPower, &["shutdown"]);
    let sleep = r(SessionPower, &["sleep"]);
    // The lock screen's Restart (audit6 S6-5): refused while another account is signed in,
    // taken when the session is the only one; its caption says so.
    let restart = r(SessionPower, &["restart"]);
    for (req, uid, sessions, want) in [
        (&join, 1002, &[1002][..], true),
        (&join, 1002, &[1001, 1002][..], true),
        (&join, 1002, &[][..], false),
        (&join, 1002, &[1001][..], false),
        (&off, 1002, &[1002][..], true),
        (&off, 1002, &[1001, 1002][..], false),
        (&restart, 1002, &[1002][..], true),
        (&restart, 1002, &[1001, 1002][..], false),
        (&sleep, 1002, &[1001, 1002][..], true),
        (&r(PkgUpgrade, &[]), 1002, &[1002][..], false),
    ] {
        assert_eq!(
            user_level_ok(req, uid, sessions),
            want,
            "{:?} {uid} {sessions:?}",
            req.action
        );
    }
    // Who is in the way of a restart or shut down, for the refusal: each other account once.
    for (req, sessions, want) in [
        (&restart, &[1001, 1002, 1001, 1003][..], &[1001, 1003][..]),
        (&off, &[1002][..], &[][..]),
        (&sleep, &[1001, 1002][..], &[][..]),
        (&join, &[1001, 1002][..], &[][..]),
    ] {
        assert_eq!(signed_in_others(req, 1002, sessions), want, "{sessions:?}");
    }
}

#[test]
fn auth_cache_expires_and_revokes() {
    let mut c = AuthCache::default();
    c.grant(1001, 7, 100);
    assert!(c.valid(1001, 7, 399));
    assert!(!c.valid(1001, 7, 400));
    assert!(!c.valid(1001, 8, 101));
    c.revoke(1001, 7);
    assert!(!c.valid(1001, 7, 101));
    assert_eq!(
        requests::wifi_join("wlan0", "H", None).args,
        vec!["wlan0", "H"]
    );
}

/// `.wol` installs (ADR 0011): a local file path and the trust the caller saw; the daemon
/// verifies again and runs the steps itself, after a restore point.
#[test]
fn wol_package_actions() {
    let cases: &[(AdminAction, &[&str], bool)] = &[
        (
            WolInstall,
            &["/home/ann/Downloads/raider-3.1.0.wol", "signed"],
            true,
        ),
        (
            WolInstall,
            &["/home/ann/.cache/wolos/packages/a.wol", "unsigned"],
            true,
        ),
        (WolInstall, &["/home/ann/a.wol", "trusted"], false),
        (WolInstall, &["a.wol", "signed"], false),
        (WolInstall, &["/home/ann/../../etc/a.wol", "signed"], false),
        (WolInstall, &["/home/ann/a.pkg", "signed"], false),
        (WolInstall, &["/home/ann/a\nb.wol", "signed"], false),
        (WolInstall, &["/home/ann/a.wol"], false),
        (WolRemove, &["io.github.adbeveridge.Raider"], true),
        (WolRemove, &["raider"], false),
        (WolRemove, &["../x.y"], false),
    ];
    for (a, args, ok) in cases {
        assert_eq!(validate(&r(*a, args)).is_none(), *ok, "{a:?} {args:?}");
    }
    assert!(
        plan(&r(WolInstall, &["/h/a.wol", "signed"]))
            .unwrap()
            .is_empty()
    );
    assert!(needs_fresh_password(&r(
        WolInstall,
        &["/h/a.wol", "unsigned"]
    )));
    assert!(!needs_fresh_password(&r(
        WolInstall,
        &["/h/a.wol", "signed"]
    )));
    assert!(!needs_fresh_password(&r(PkgUpgrade, &[])));
}

/// IMAGE-F: the two changes `wol doctor hardware` offers a button for. Mirror Gate runs the
/// repo gate as root (CPU level against the optimized sections of pacman.conf); the cooling
/// helper turns thermald on or off with its unit, and nothing else.
#[test]
fn the_doctor_hardware_actions_plan_exactly_one_command_each() {
    assert_eq!(
        plan(&r(MirrorGate, &[])).unwrap(),
        [AdminStep::Run(
            ["wol", "mirror", "gate", "--yes"]
                .map(String::from)
                .to_vec()
        )]
    );
    assert_eq!(
        plan(&r(CoolingHelper, &["on"])).unwrap(),
        [AdminStep::Run(
            ["systemctl", "enable", "--now", "thermald.service"]
                .map(String::from)
                .to_vec()
        )]
    );
    assert_eq!(
        plan(&r(CoolingHelper, &["off"])).unwrap(),
        [AdminStep::Run(
            ["systemctl", "disable", "--now", "thermald.service"]
                .map(String::from)
                .to_vec()
        )]
    );
    // Repo changes need the password every time; the cooling switch is an everyday change.
    assert_eq!(fresh_rule(MirrorGate), Fresh::Always);
    assert_eq!(fresh_rule(CoolingHelper), Fresh::Cached);
    assert!(!is_user_level(MirrorGate) && !is_user_level(CoolingHelper));
    assert_eq!(MirrorGate.name(), "mirror-gate");
    assert_eq!(CoolingHelper.name(), "cooling-helper");
}

/// QA-A7-17: a process the compositor started has no `$USER`; the name then follows the uid into
/// `passwd`, so an administrator is not read as a standard account.
#[test]
fn the_admin_check_works_without_user_in_the_environment() {
    let passwd = "root:x:0:0::/root:/bin/sh\nada:x:1000:1000::/home/ada:/bin/bash\nbob:x:1001:1001::/home/bob:/bin/bash\n";
    let group = "wheel:x:998:ada\nusers:x:100:bob\n";
    assert!(account_is_admin(None, 1000, passwd, group));
    assert!(account_is_admin(Some(""), 1000, passwd, group));
    assert!(!account_is_admin(None, 1001, passwd, group));
    assert!(account_is_admin(Some("ada"), 1001, passwd, group));
    assert!(!account_is_admin(None, 4242, passwd, group), "unknown uid");
}
