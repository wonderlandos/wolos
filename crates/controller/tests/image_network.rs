// SPDX-License-Identifier: GPL-2.0-or-later
//! B7 network-stack image contract: `.local` names, the connectivity probe, the resolver's
//! fallback, the AirPlay rule file shipped closed. Reads the overlaid sibling root (wolos +
//! wolos-pkgbuilds + wolos-iso, `support/siblings.rs`).
#[path = "support/siblings.rs"]
mod siblings;

use std::fs;
use std::process::Command;

fn read(p: &str) -> String {
    let path = siblings::root().join(p);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{p}: {e}"))
}

/// QA-NET-4: `.local` names never resolved: Arch's hosts line `resolve [!UNAVAIL=return]` ends the
/// lookup at systemd-resolved, whose mDNS is off (avahi owns it). Either nss-mdns is asked first
/// or resolved does mDNS; WolOS does the first. The shipped script turns Arch's default hosts
/// line into one with `mdns4_minimal [NOTFOUND=return]` in front of `resolve`, once. IPv4 only
/// (dry run 0.13.0, net-firewall-state): with `mdns_minimal` every lookup of a name with no IPv6
/// address waited 5 s for avahi's AAAA answer (getaddrinfo too; measured 5.0 s against 0.01 s),
/// and a 0.12 line with `mdns_minimal` is moved to `mdns4_minimal`.
#[test]
fn nsswitch_has_mdns_or_resolved_off() {
    let resolved = read("files/usr/lib/systemd/resolved.conf.d/wolos.conf");
    let script = siblings::root().join("wolos-system/nsswitch-mdns.sh");
    assert!(script.is_file(), "{} missing", script.display());
    let d = std::env::temp_dir().join(format!("b7-nss-{}", std::process::id()));
    fs::create_dir_all(&d).unwrap();
    let ns = d.join("nsswitch.conf");
    let run = |hosts: &str| {
        fs::write(&ns, format!("passwd: files systemd\nhosts: {hosts}\n")).unwrap();
        for _ in 0..2 {
            let o = Command::new("sh").arg(&script).arg(&ns).output().unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        }
        let text = fs::read_to_string(&ns).unwrap();
        text.lines()
            .find(|l| l.starts_with("hosts:"))
            .unwrap()
            .to_owned()
    };
    let arch = "mymachines resolve [!UNAVAIL=return] files myhostname dns";
    let line = run(arch);
    // An installed 0.12 or 0.13 computer: its line moves to IPv4-only mDNS on the upgrade.
    let old = run(&arch.replace("resolve", "mdns_minimal [NOTFOUND=return] resolve"));
    assert_eq!(old, line, "the 0.12 line is moved");
    let _ = fs::remove_dir_all(&d);
    let line = line.as_str();
    assert!(!line.contains(" mdns_minimal"), "a 5 s AAAA wait: {line}");
    let mdns = line.find("mdns4_minimal [NOTFOUND=return]");
    let resolve = line.find(" resolve");
    let mdns_first = matches!((mdns, resolve), (Some(m), Some(r)) if m < r);
    let resolved_mdns =
        resolved.contains("MulticastDNS=yes") || resolved.contains("MulticastDNS=resolve");
    assert!(mdns_first || resolved_mdns, "{line}");
    assert_eq!(
        line.matches("mdns4_minimal").count(),
        1,
        "idempotent: {line}"
    );
    // One mDNS stack: not both.
    assert!(
        mdns_first != resolved_mdns,
        "two mDNS stacks: {line} / {resolved}"
    );
}

/// QA-NET-4: no compiled-in third-party fallback resolvers (Quad9, Cloudflare, Google).
#[test]
fn resolver_has_no_hidden_fallback() {
    let d = read("files/usr/lib/systemd/resolved.conf.d/wolos.conf");
    assert!(d.lines().any(|l| l.trim() == "FallbackDNS="), "{d}");
}

/// QA-NET-12 / NET2-3: NetworkManager's connectivity check is on (portal and limited exist only
/// with it), plain http (a portal intercepts http; https reads as limited), and its endpoint and
/// cadence are WolOS's choice, written down, not an inherited default.
#[test]
fn connectivity_check_is_configured() {
    let c = read("files/usr/lib/NetworkManager/conf.d/wolos-connectivity.conf");
    for want in [
        "[connectivity]",
        "enabled=true",
        "uri=http://",
        "response=",
        "interval=300",
    ] {
        assert!(c.contains(want), "{want} missing: {c}");
    }
    assert!(!c.contains("uri=https://"), "a portal cannot answer https");
}

/// QA-NET-5: AirPlay's rule file ships closed (the plan writes it open while the receiver is on).
#[test]
fn airplay_rule_file_ships_closed() {
    assert_eq!(
        read("desktop/overlay/etc/wolos/nftables.d/airplay.conf"),
        wol_core::sharing::airplay_rule(false)
    );
    let n = read("desktop/overlay/etc/wolos/nftables.conf");
    assert!(n.contains("include \"/etc/wolos/nftables.d/*.conf\""));
}
