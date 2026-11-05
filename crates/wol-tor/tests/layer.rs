// SPDX-License-Identifier: GPL-2.0-or-later
//! The command line of the OS Tor layer (ADR 0056): switches, plans, dry-run rulesets, bridges.
use std::path::PathBuf;
use std::process::{Command, Output};

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-tor-layer-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn wol_tor(cfg: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wol-tor"))
        .args(args)
        .env("XDG_CONFIG_HOME", cfg)
        .env("HOME", cfg)
        .env("WOL_TOR_ETC", cfg.join("etc"))
        .env("WOL_TOR_RUN", cfg.join("run"))
        .env_remove("WOL_TOR_KEEP_READY")
        .output()
        .unwrap()
}

/// What `wol-tor apply` records after it ran: the switch file as it was applied.
fn mark_applied(cfg: &std::path::Path) {
    std::fs::create_dir_all(cfg.join("run")).unwrap();
    std::fs::copy(cfg.join("etc/switches"), cfg.join("run/applied")).unwrap();
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

#[test]
fn defaults_are_atlas_tabs_on_and_nothing_else() {
    let cfg = scratch("defaults");
    let o = wol_tor(&cfg, &["switches", "show"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "atlas on\nsystem off\nlan off\nkeep-ready off\n");
    assert!(!cfg.join("wol-tor").exists(), "showing writes nothing");
    let _ = std::fs::remove_dir_all(&cfg);
}

#[test]
fn turning_the_machine_on_plans_rules_before_tor_and_off_the_reverse() {
    let cfg = scratch("system");
    let on = wol_tor(&cfg, &["switches", "set", "system", "on"]);
    assert_eq!(
        out(&on),
        "plan LoadSystemRules\nplan StartGateway\n",
        "{}",
        err(&on)
    );
    assert!(out(&wol_tor(&cfg, &["switches", "show"])).contains("system on"));
    mark_applied(&cfg);
    let off = wol_tor(&cfg, &["switches", "set", "system", "off"]);
    assert_eq!(out(&off), "plan StopGateway\nplan UnloadSystemRules\n");
    let _ = std::fs::remove_dir_all(&cfg);
}

#[test]
fn apps_are_added_with_checked_names() {
    let cfg = scratch("apps");
    let add = wol_tor(&cfg, &["switches", "app", "add", "discord"]);
    assert_eq!(
        out(&add),
        "plan GatewayUp\nplan StartGateway\nplan AddApp(\"discord\")\n",
        "{}",
        err(&add)
    );
    for bad in ["Bad Name", "../x", "a;b"] {
        let o = wol_tor(&cfg, &["switches", "app", "add", bad]);
        assert!(!o.status.success(), "{bad}");
    }
    assert!(out(&wol_tor(&cfg, &["switches", "show"])).contains("app discord 1\n"));
    mark_applied(&cfg);
    let rm = wol_tor(&cfg, &["switches", "app", "remove", "discord"]);
    assert!(out(&rm).contains("plan RemoveApp(\"discord\")\nplan StopGateway\nplan GatewayDown"));
    let _ = std::fs::remove_dir_all(&cfg);
}

#[test]
fn the_dry_run_rulesets_and_steps_are_printed() {
    let cfg = scratch("dry");
    let apps = wol_tor(&cfg, &["gateway", "rules", "apps"]);
    assert!(
        out(&apps)
            .contains("iifname \"wol-tor0\" meta nfproto ipv4 meta l4proto tcp redirect to :9040")
    );
    let ns = wol_tor(&cfg, &["gateway", "rules", "ns"]);
    assert!(out(&ns).contains("policy drop"));
    let steps = wol_tor(&cfg, &["gateway", "steps", "add", "discord", "1"]);
    let text = out(&steps);
    assert!(
        text.contains("run ip netns add wolt-discord"),
        "{text}{}",
        err(&steps)
    );
    assert!(text.contains("write /etc/netns/wolt-discord/resolv.conf: nameserver 10.191.0.1"));
    assert!(text.contains("write /etc/netns/wolt-discord/nsswitch.conf: passwd: files systemd"));
    assert!(text.contains("run bridge link set dev wtv1 isolated on"));
    assert!(
        !wol_tor(&cfg, &["gateway", "steps", "add", "Bad", "1"])
            .status
            .success()
    );
    assert!(
        !wol_tor(&cfg, &["gateway", "steps", "add", "ok", "0"])
            .status
            .success()
    );
    let join = wol_tor(
        &cfg,
        &[
            "gateway",
            "join",
            "discord",
            "1",
            "1000",
            "1000",
            "--",
            "/usr/bin/discord",
        ],
    );
    assert!(
        out(&join).starts_with("ip netns exec wolt-discord ")
            && out(&join).contains(" app-exec -- setpriv --reuid 1000"),
        "{}",
        err(&join)
    );
    assert!(out(&join).trim_end().ends_with("-- /usr/bin/discord"));
    let unit = wol_tor(&cfg, &["gateway", "unit"]);
    assert!(out(&unit).contains("tor-run --gateway"));
    let _ = std::fs::remove_dir_all(&cfg);
}

#[test]
fn bridges_are_checked_before_they_are_kept() {
    let cfg = scratch("bridges");
    let bad = wol_tor(&cfg, &["bridges", "set", "not a bridge\nSocksPort 1"]);
    assert!(!bad.status.success());
    assert!(!cfg.join("wol-tor/bridges").exists());
    let good = wol_tor(
        &cfg,
        &[
            "bridges",
            "set",
            "192.0.2.5:443 0123456789ABCDEF0123456789ABCDEF01234567",
        ],
    );
    assert!(good.status.success(), "{}", err(&good));
    assert!(out(&wol_tor(&cfg, &["bridges", "show"])).contains("192.0.2.5:443"));
    let obfs = wol_tor(
        &cfg,
        &[
            "bridges",
            "set",
            "obfs4 192.0.2.6:443 0123456789ABCDEF0123456789ABCDEF01234567 cert=x iat-mode=0",
        ],
    );
    assert!(obfs.status.success(), "{}", err(&obfs));
    assert!(wol_tor(&cfg, &["bridges", "clear"]).status.success());
    assert_eq!(out(&wol_tor(&cfg, &["bridges", "show"])), "");
    let _ = std::fs::remove_dir_all(&cfg);
}

#[test]
fn doctor_without_a_running_tor_says_not_checked_and_does_not_fail() {
    let cfg = scratch("doctor");
    let run = scratch("doctor-run");
    let o = Command::new(env!("CARGO_BIN_EXE_wol-tor"))
        .arg("doctor")
        .env("XDG_CONFIG_HOME", &cfg)
        .env("HOME", &cfg)
        .env("XDG_RUNTIME_DIR", &run)
        .output()
        .unwrap();
    let text = out(&o);
    assert!(o.status.success(), "{text}{}", err(&o));
    assert!(
        text.contains("bootstrap") && text.contains("not checked"),
        "{text}"
    );
    assert!(!text.contains("FAIL"));
    let _ = std::fs::remove_dir_all(&cfg);
    let _ = std::fs::remove_dir_all(&run);
}

#[test]
fn the_switch_file_is_the_root_owned_one_and_the_user_has_no_copy() {
    let cfg = scratch("rootfile");
    let on = wol_tor(&cfg, &["switches", "set", "system", "on"]);
    assert!(on.status.success(), "{}", err(&on));
    assert!(
        cfg.join("etc/switches").is_file(),
        "written where the boot unit reads it"
    );
    assert!(
        !cfg.join("wol-tor/switches").exists(),
        "no per-user copy of the scopes"
    );
    assert!(
        std::fs::read_to_string(cfg.join("etc/switches"))
            .unwrap()
            .contains("system on\n")
    );
    // Without rights to write it, the change is refused with the reason, not half done.
    let out = Command::new(env!("CARGO_BIN_EXE_wol-tor"))
        .args(["switches", "set", "system", "on"])
        .env("XDG_CONFIG_HOME", &cfg)
        .env("HOME", &cfg)
        .env("WOL_TOR_ETC", "/proc/wol-tor-nope")
        .env("WOL_TOR_RUN", cfg.join("run"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(err(&out).contains("administrator"), "{}", err(&out));
    let _ = std::fs::remove_dir_all(&cfg);
}

#[test]
fn apply_at_boot_is_the_plan_from_nothing_to_the_switch_file() {
    let cfg = scratch("apply");
    for a in [["set", "system", "on"], ["set", "lan", "on"]] {
        assert!(
            wol_tor(&cfg, &["switches", a[0], a[1], a[2]])
                .status
                .success()
        );
    }
    assert!(
        wol_tor(&cfg, &["switches", "app", "add", "discord"])
            .status
            .success()
    );
    let dry = wol_tor(&cfg, &["apply", "--dry-run", "--tor-uid", "105"]);
    let text = out(&dry);
    assert!(dry.status.success(), "{}", err(&dry));
    let pos = |needle: &str| {
        text.find(needle)
            .unwrap_or_else(|| panic!("{needle} in\n{text}"))
    };
    let rules = pos("meta skuid 105 return");
    let bridge = pos("run ip link add wol-tor0 type bridge");
    let tor = pos("run systemctl start wol-tor-gateway.service");
    let ns = pos("run ip netns add wolt-discord");
    assert!(
        rules < bridge && bridge < tor && tor < ns,
        "rules, bridge, tor, then the namespace\n{text}"
    );
    assert!(
        text.contains("allowed LAN"),
        "the lan option reaches the rules"
    );
    // Reloading the firewall re-applies the rule files and nothing else.
    let reload = out(&wol_tor(
        &cfg,
        &["apply", "--dry-run", "--reload-rules", "--tor-uid", "105"],
    ));
    assert!(reload.contains("table inet wol_tor") && reload.contains("table inet wol_tor_apps"));
    assert!(!reload.contains("systemctl") && !reload.contains("netns add"));
    // With nothing switched on there is nothing to do.
    let empty = scratch("apply-empty");
    assert_eq!(
        out(&wol_tor(
            &empty,
            &["apply", "--dry-run", "--tor-uid", "105"]
        )),
        ""
    );
    let _ = std::fs::remove_dir_all(&cfg);
    let _ = std::fs::remove_dir_all(&empty);
}

#[test]
fn units_for_boot_and_for_the_firewall_reload_are_printed() {
    let cfg = scratch("units");
    let rules = out(&wol_tor(&cfg, &["gateway", "rules-unit"]));
    assert!(
        rules.contains("After=nftables.service")
            && rules.contains("Before=network-pre.target network-online.target")
    );
    assert!(rules.contains(" apply\n"));
    assert!(out(&wol_tor(&cfg, &["gateway", "nft-dropin"])).contains("apply --reload-rules"));
    let gw = out(&wol_tor(&cfg, &["gateway", "unit"]));
    assert!(gw.contains("StateDirectory=wol-tor/gateway"));
    let _ = std::fs::remove_dir_all(&cfg);
}
