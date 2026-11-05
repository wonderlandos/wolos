// SPDX-License-Identifier: GPL-2.0-or-later
//! NET-3 verbs: `wol network ip`, `wol vpn`, `wol proxy`, `wol dns tls`, `wol airplane`, against
//! the recording fake (QA-LIVE3-14, QA-NET-21).
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use shared::admin::AdminReply;
use std::sync::Arc;
use wol_core::system::fake::FakeHost;

const U: &str = "6a3f0c2e-1111-4a5b-9c8d-0123456789ab";

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

fn linked() -> Arc<FakeHost> {
    let h = Arc::new(FakeHost::linux());
    h.command(
        "nmcli -g GENERAL.CON-UUID device show enp0s1",
        &format!("{U}\n"),
    )
    .command(
        &format!(
            "nmcli -t -f ipv4.method,ipv4.addresses,ipv4.gateway,ipv4.dns connection show uuid {U}"
        ),
        "ipv4.method:auto\nipv4.addresses:\nipv4.gateway:--\nipv4.dns:\n",
    )
    .command("nmcli -t -g CONNECTIVITY general status", "full\n")
    .command(
        "nmcli -g GENERAL.STATE device show enp0s1",
        "100 (connected)\n",
    )
    .command("nmcli networking connectivity check", "full\n");
    h
}

/// QA-LIVE3-14: `wol network ip` reads a link's address, refuses a router outside the subnet
/// with a sentence, and sets a Manual address that keeps the computer online.
#[test]
fn network_ip_reads_checks_and_sets_an_address() {
    let mut env = FakeEnv {
        host: Some(linked()),
        ..Default::default()
    };
    assert_eq!(run("network ip enp0s1 --json", &mut env), 0, "{}", env.err);
    assert!(env.out.contains("\"automatic\": true"), "{}", env.out);
    let mut env = FakeEnv {
        host: Some(linked()),
        ..Default::default()
    };
    assert_ne!(
        run(
            "network ip enp0s1 manual 10.0.2.50/24 192.168.9.1",
            &mut env
        ),
        0
    );
    assert!(env.err.contains("same network"), "{}", env.err);
    let mut env = FakeEnv {
        host: Some(linked()),
        ..Default::default()
    };
    assert_eq!(
        run(
            "network ip enp0s1 manual 10.0.2.50/24 10.0.2.2 1.1.1.1 9.9.9.9 --json",
            &mut env
        ),
        0,
        "{}",
        env.err
    );
    let mut env = FakeEnv {
        host: Some(Arc::new(FakeHost::linux())),
        ..Default::default()
    };
    assert_ne!(run("network ip enp9s9", &mut env), 0);
    assert!(env.err.contains("not connected"), "{}", env.err);
}

/// NET-21: the proxy and DNS over TLS go through wol-admind with the password on stdin; a bad
/// value is refused before anything is sent.
#[test]
fn proxy_and_dns_need_the_administrator() {
    let mut env = FakeEnv {
        host: Some(Arc::new(FakeHost::linux())),
        stdin: vec!["pw".into()],
        ..Default::default()
    };
    env.admin.lock().unwrap().replies = vec![AdminReply::NeedsAuth, AdminReply::Ok];
    assert_eq!(
        run(
            "proxy manual http=proxy.lan:3128 no-proxy=.corp,10.0.0.0/8 --json",
            &mut env
        ),
        0,
        "{}",
        env.err
    );
    let sent = env.admin.lock().unwrap().sent.clone();
    assert_eq!(
        sent.last().unwrap(),
        "proxy manual proxy.lan:3128 - - .corp,10.0.0.0/8 pw=pw"
    );
    let mut env = FakeEnv::default();
    assert_ne!(run("proxy manual http=proxy.lan", &mut env), 0);
    assert!(env.admin.lock().unwrap().sent.is_empty());
    assert_ne!(run("proxy manual color=blue", &mut env), 0);
    let mut env = FakeEnv {
        stdin: vec!["pw".into()],
        ..Default::default()
    };
    env.admin.lock().unwrap().replies = vec![AdminReply::NeedsAuth, AdminReply::Ok];
    assert_eq!(
        run("dns tls strict 9.9.9.9#dns.quad9.net --json", &mut env),
        0,
        "{}",
        env.err
    );
    assert_eq!(
        env.admin.lock().unwrap().sent.last().unwrap(),
        "dns-over-tls strict 9.9.9.9#dns.quad9.net pw=pw"
    );
    let mut env = FakeEnv::default();
    assert_ne!(
        run("dns tls strict", &mut env),
        0,
        "strict without a server"
    );
    assert!(env.err.contains("at least one server"), "{}", env.err);
}

/// NET-21 airplane: `on` saves the radios that were on, `off` removes the list.
#[test]
fn airplane_saves_and_clears_the_radio_list() {
    let h = Arc::new(FakeHost::linux());
    h.command(
        "rfkill -J -o TYPE,SOFT",
        r#"{"rfkilldevices":[{"type":"wlan","soft":"unblocked"},{"type":"bluetooth","soft":"unblocked"}]}"#,
    );
    let mut env = FakeEnv {
        host: Some(h.clone()),
        ..Default::default()
    };
    assert_eq!(run("airplane on --json", &mut env), 0, "{}", env.err);
    let (path, text) = env.written.last().unwrap().clone();
    assert!(path.ends_with("wolos/airplane"), "{path:?}");
    assert_eq!(text, "wlan\nbluetooth\n");
    assert_eq!(run("airplane off --json", &mut env), 0, "{}", env.err);
    assert!(env.removed.iter().any(|p| p.ends_with("wolos/airplane")));
    let mut env = FakeEnv {
        host: Some(Arc::new(FakeHost::linux())),
        ..Default::default()
    };
    assert_ne!(run("airplane on", &mut env), 0);
    assert!(env.err.contains("no radio"), "{}", env.err);
}

/// NET-21 VPN: the list comes from NetworkManager; an OpenVPN file without the plugin says so.
#[test]
fn vpn_list_and_missing_openvpn_plugin() {
    let h = Arc::new(FakeHost::linux());
    h.command(
        "nmcli -t -f NAME,UUID,TYPE,ACTIVE connection show",
        &format!("Office:{U}:wireguard:yes\nHome:11111111-2222-3333-4444-555555555555:802-11-wireless:yes\n"),
    );
    let mut env = FakeEnv {
        host: Some(h.clone()),
        ..Default::default()
    };
    assert_eq!(run("vpn --json", &mut env), 0, "{}", env.err);
    assert!(
        env.out.contains("\"Office\"") && !env.out.contains("Home"),
        "{}",
        env.out
    );
    let mut env = FakeEnv {
        host: Some(h.clone()),
        ..Default::default()
    };
    assert_ne!(run("vpn import /tmp/office.ovpn", &mut env), 0);
    assert!(env.err.contains("networkmanager-openvpn"), "{}", env.err);
    let mut env = FakeEnv {
        host: Some(h),
        ..Default::default()
    };
    assert_ne!(run("vpn connect Nowhere", &mut env), 0);
    assert!(env.err.contains("no VPN of that name"), "{}", env.err);
}

/// NET-3 DoT: the shipped resolved drop-in sets neither DNSOverTLS nor DNS, so the one Settings
/// writes in /etc/systemd/resolved.conf.d is what resolved uses (a key set in the /usr/lib file of
/// a later name would win over it).
#[test]
fn shipped_resolved_dropin_leaves_dns_over_tls_to_settings() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../files/usr/lib/systemd/resolved.conf.d/wolos.conf");
    let t = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    for l in t.lines().filter(|l| !l.starts_with('#')) {
        assert!(
            !l.starts_with("DNSOverTLS=") && !l.starts_with("DNS="),
            "{l}"
        );
    }
    assert!(
        wol_core::netconf::DOT_DROPIN.starts_with("/etc/systemd/resolved.conf.d/"),
        "an /etc drop-in"
    );
}

/// QA-A7-13: the folder the airplane list lives in is made by tmpfiles.d, open to every account,
/// and is the one the code names. QA-A9-CORE-7: open to the accounts (the `seat` group every
/// account made by WolOS is in, `settings::users::ACCOUNT_GROUPS`), not to every uid with 0777: a
/// world-writable system directory without the sticky bit lets any account or service delete or
/// replace the list another wrote. Not sticky either: the list is one shared file the next person
/// replaces, which a sticky directory would refuse.
#[test]
fn the_airplane_list_folder_is_made_for_every_account() {
    let t = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../files/usr/lib/tmpfiles.d/wolos.conf"),
    )
    .unwrap();
    let want = [
        "d",
        wol_core::netconf::AIRPLANE_SHARED_DIR,
        "2775",
        "root",
        "seat",
        "-",
    ];
    assert!(
        t.lines()
            .any(|l| l.split_whitespace().collect::<Vec<_>>() == want),
        "tmpfiles.d/wolos.conf lacks `{}`",
        want.join(" ")
    );
    assert!(
        wol_core::settings::users::ACCOUNT_GROUPS
            .split(',')
            .any(|g| g == "seat"),
        "every account is in the group the folder is open to"
    );
    // No `d` line of the file is writable by everyone.
    for l in t.lines().filter(|l| !l.trim_start().starts_with('#')) {
        let f: Vec<&str> = l.split_whitespace().collect();
        if let ["d" | "D", path, mode, ..] = f[..] {
            let m = u32::from_str_radix(mode.trim_start_matches('0'), 8).unwrap_or(0);
            assert_eq!(m & 0o002, 0, "{path}: {mode} is writable by every uid");
        }
    }
}
