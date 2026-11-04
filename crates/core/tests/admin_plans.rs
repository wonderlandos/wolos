// SPDX-License-Identifier: BSD-2-Clause
//! QA-SHADOW-42: no admin plan may run a FreeBSD tool. `AdminAction::Bluetooth` planned
//! `service bluetooth onestart ubt0` (rc.d), dead on Linux: `wol admin bluetooth on` ended
//! `Failed` with no sentence. Every program a plan in `core::admin` starts must be one the image
//! ships (systemd, NetworkManager, kmod, flatpak, pacman, the shell's own `wol`).
use shared::admin::{AdminAction, AdminRequest};

/// Programs of the installed image that `core::admin` may name in an `argv(&[..])` plan.
const IMAGE_PROGRAMS: [&str; 8] = [
    "flatpak",
    "hostnamectl",
    "modprobe",
    "nmcli",
    "pacman",
    "systemctl",
    "timedatectl",
    "wol",
];

/// The program each `argv(&["prog", ..` in the plan builders starts.
fn plan_programs(source: &str) -> Vec<String> {
    let production = source
        .split("#[cfg(test)]\nmod tests")
        .next()
        .unwrap_or(source);
    let mut out = Vec::new();
    for (i, _) in production.match_indices("argv(&[") {
        let rest = production[i + "argv(&[".len()..].trim_start();
        if let Some(lit) = rest.strip_prefix('"')
            && let Some(end) = lit.find('"')
        {
            out.push(lit[..end].to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn every_plan_binary_exists_in_image() {
    let source = include_str!("../src/admin.rs");
    let programs = plan_programs(source);
    assert!(!programs.is_empty(), "the scan found no plan");
    for p in &programs {
        assert!(
            IMAGE_PROGRAMS.contains(&p.as_str()),
            "a plan starts `{p}`, which the image does not ship (FreeBSD rc/pw/ifconfig?)"
        );
    }
    // The bluetooth plan itself, through the public planner.
    for (arg, verb) in [("on", "start"), ("off", "stop")] {
        let r = AdminRequest {
            action: AdminAction::Bluetooth,
            args: vec![arg.to_string()],
        };
        let steps = wol_core::admin::plan_with(&r, &|_| None).expect("a valid request plans");
        let shown = format!("{steps:?}");
        assert_eq!(
            shown.contains("reset-failed"),
            arg == "on",
            "Turn On clears a hit start limit first: {shown}"
        );
        assert!(
            shown.contains("systemctl")
                && shown.contains(verb)
                && shown.contains("bluetooth.service"),
            "{shown}"
        );
        assert!(
            !shown.contains("onestart") && !shown.contains("ubt0"),
            "{shown}"
        );
    }
}

fn plan_text(action: AdminAction, args: &[&str], firewall: &str) -> String {
    let r = AdminRequest {
        action,
        args: args.iter().map(|s| s.to_string()).collect(),
    };
    let f = firewall.to_string();
    let steps = wol_core::admin::plan_with(&r, &move |a: &[&str]| {
        (a.join(" ") == wol_core::sharing::FIREWALL_ENABLED_ARGV.join(" ")).then(|| f.clone())
    })
    .expect("a valid request plans");
    format!("{steps:?}")
}

/// QA-NET-5 (B7): AirPlay Receiver on writes its rule file with every port shairport-sync
/// listens on, loads the ruleset again (only while the firewall is on) and starts the service
/// after the ports are open; off stops it first and closes them. The shipped file is the closed one.
#[test]
fn airplay_opens_its_ports() {
    use wol_core::sharing::{AIRPLAY_RULE_FILE, airplay_rule};
    let on = plan_text(AdminAction::AirPlayReceiver, &["on"], "enabled\n");
    let (w, nft, start) = (
        on.find(AIRPLAY_RULE_FILE).expect(&on),
        on.find("\"nft\", \"-f\"").expect(&on),
        on.find("shairport-sync.service").expect(&on),
    );
    assert!(
        w < nft && nft < start,
        "rule, reload, then the service: {on}"
    );
    let rule = airplay_rule(true);
    for port in ["5000", "7000", "319", "320", "6000-6009"] {
        assert!(rule.contains(port), "{port} missing: {rule}");
    }
    assert!(rule.contains("tcp dport { 5000, 7000 }") && rule.contains("udp dport"));
    assert!(
        on.contains(&format!("{:?}", rule)),
        "the plan writes airplay_rule(true): {on}"
    );
    // Firewall off: the file is written, the ruleset is NOT loaded (that would turn it on).
    let off_fw = plan_text(AdminAction::AirPlayReceiver, &["on"], "disabled\n");
    assert!(
        off_fw.contains(AIRPLAY_RULE_FILE) && !off_fw.contains("\"nft\""),
        "{off_fw}"
    );
    let off = plan_text(AdminAction::AirPlayReceiver, &["off"], "enabled\n");
    assert!(
        off.find("disable").unwrap() < off.find(AIRPLAY_RULE_FILE).unwrap(),
        "{off}"
    );
    assert!(!airplay_rule(false).contains("dport"));
    assert_eq!(
        include_str!("../../../desktop/overlay/etc/wolos/nftables.d/airplay.conf"),
        airplay_rule(false),
        "shipped closed"
    );
    // The rule parses as nft input-chain rules (in a throw-away namespace, when root is
    // available without a password and nft is installed).
    let sudo = std::process::Command::new("sudo")
        .args(["-n", "true"])
        .status();
    let nft = std::process::Command::new("nft").arg("--version").output();
    if sudo.is_ok_and(|s| s.success()) && nft.is_ok_and(|o| o.status.success()) {
        let d = std::env::temp_dir().join(format!("b7-airplay-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let conf = d.join("n.conf");
        std::fs::write(
            &conf,
            format!(
                "table inet t {{\n chain input {{\n  type filter hook input priority filter; policy drop;\n{rule} }}\n}}\n"
            ),
        )
        .unwrap();
        let o = std::process::Command::new("sudo")
            .args(["-n", "unshare", "-n", "nft", "-c", "-f"])
            .arg(&conf)
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&d);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
}

/// QA-NET-17 (B7): Forget removes every saved profile that joins the SSID ("Home", "Home 1"),
/// by UUID, and never the profile of another network that happens to be named so.
#[test]
fn forget_by_ssid() {
    let r = AdminRequest {
        action: AdminAction::WifiForget,
        args: vec!["wlan0".into(), "Home".into()],
    };
    let probe = |a: &[&str]| -> Option<String> {
        Some(match a.join(" ").as_str() {
            "nmcli -t -f UUID,TYPE connection show" => "11111111-aaaa:802-11-wireless\n22222222-bbbb:802-11-wireless\n33333333-cccc:802-11-wireless\n44444444-dddd:802-3-ethernet\n".into(),
            "nmcli -g 802-11-wireless.ssid connection show uuid 11111111-aaaa" => "Home\n".into(),
            "nmcli -g 802-11-wireless.ssid connection show uuid 22222222-bbbb" => "Home\n".into(),
            // A profile NAMED "Home" that joins the neighbour's network.
            "nmcli -g 802-11-wireless.ssid connection show uuid 33333333-cccc" => "Neighbour\n".into(),
            _ => return None,
        })
    };
    let steps = format!("{:?}", wol_core::admin::plan_with(&r, &probe).unwrap());
    assert!(
        steps.contains("11111111-aaaa") && steps.contains("22222222-bbbb"),
        "{steps}"
    );
    assert!(
        !steps.contains("33333333") && !steps.contains("44444444"),
        "{steps}"
    );
    assert!(!steps.contains("\"id\""), "by uuid, not by name: {steps}");
    // Nothing readable: the name, as before.
    let steps = format!("{:?}", wol_core::admin::plan(&r).unwrap());
    assert!(steps.contains("\"delete\", \"id\", \"Home\""), "{steps}");
}

/// QA-NET-17 (B7): Disconnect is `nmcli dev disconnect <iface>` (the profile stays); never an
/// option-looking or path-looking name.
#[test]
fn disconnect_argv() {
    assert_eq!(
        wol_core::wifi::nm_disconnect_argv("wlan0").unwrap(),
        ["nmcli", "dev", "disconnect", "wlan0"]
    );
    for bad in ["", "-h", "a/b", "wl an0", "0123456789abcdef"] {
        assert_eq!(wol_core::wifi::nm_disconnect_argv(bad), None, "{bad}");
    }
}

/// QA-DATA-8 (B3): `wol admin user-remove bob` deleted /home/bob at once (`userdel -r`). The home
/// folder stays unless `with-files` is asked for, and a signed-in account is never removed.
#[test]
fn user_remove_keeps_home_by_default() {
    let req = |args: &[&str]| AdminRequest {
        action: AdminAction::UserRemove,
        args: args.iter().map(|s| s.to_string()).collect(),
    };
    let shown = |r: &AdminRequest| format!("{:?}", wol_core::admin::plan(r).unwrap());
    let keep = shown(&req(&["bob"]));
    assert!(keep.contains("[\"userdel\", \"bob\"]"), "{keep}");
    assert!(!keep.contains("-r"), "{keep}");
    let delete = shown(&req(&["bob", wol_core::admin::USER_REMOVE_WITH_FILES]));
    assert!(
        delete.contains("[\"userdel\", \"-r\", \"bob\"]"),
        "{delete}"
    );
    // bob is signed in (logind says a State): refused with the reason, nothing planned.
    let signed_in = |a: &[&str]| {
        (a.join(" ") == "loginctl show-user bob --property=State --value")
            .then(|| "active\n".into())
    };
    let r = wol_core::admin::plan_with(&req(&["bob"]), &signed_in);
    assert!(r.as_ref().is_err_and(|e| e.contains("signed in")), "{r:?}");
}
