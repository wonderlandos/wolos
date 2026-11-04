// SPDX-License-Identifier: GPL-2.0-or-later
//! The fail-closed check that the browser's network namespace has no way out (ADR 0053 D2).
//!
//! systemd honours `PrivateNetwork=` in user units only where unprivileged user namespaces
//! work and skips it silently elsewhere (measured 2026-11-04 on Ubuntu 24.04 with the AppArmor
//! user-namespace restriction: the unit saw the host's interfaces and reached 1.1.1.1:443). So
//! the host checks what the kernel gives it before it starts Atlas: the only interface is `lo`,
//! the IPv4 main table is empty, and every IPv6 route is on `lo`. Anything else is refused.
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::Path;

/// Most bytes read from one `/proc` table (a namespace with no routes has a few hundred).
pub const MAX_TABLE: u64 = 1 << 20;

/// The namespace has a way out; Tor tabs are not started.
#[derive(Debug, PartialEq, Eq)]
pub struct NotIsolated(pub String);

impl fmt::Display for NotIsolated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Tor tabs need an isolated network and this system did not provide one ({})",
            self.0
        )
    }
}

impl std::error::Error for NotIsolated {}

/// Checks the namespace from its interface names and its two route tables (the text of
/// `/proc/net/route` and `/proc/net/ipv6_route`).
pub fn check(interfaces: &[String], route4: &str, route6: &str) -> Result<(), NotIsolated> {
    if let Some(i) = interfaces.iter().find(|i| i.as_str() != "lo") {
        return Err(NotIsolated(format!("interface {i} is present")));
    }
    if !interfaces.iter().any(|i| i == "lo") {
        return Err(NotIsolated(
            "there is no loopback interface for the relay".into(),
        ));
    }
    if let Some(row) = route4.lines().skip(1).find(|l| !l.trim().is_empty()) {
        let dev = row.split_whitespace().next().unwrap_or("?");
        return Err(NotIsolated(format!("an IPv4 route via {dev} exists")));
    }
    for row in route6.lines().filter(|l| !l.trim().is_empty()) {
        let dev = row.split_whitespace().last().unwrap_or("?");
        if dev != "lo" {
            return Err(NotIsolated(format!("an IPv6 route via {dev} exists")));
        }
    }
    Ok(())
}

fn table(path: &Path) -> Result<String, NotIsolated> {
    let mut s = String::new();
    fs::File::open(path)
        .and_then(|f| f.take(MAX_TABLE).read_to_string(&mut s))
        .map_err(|e| NotIsolated(format!("{} could not be read: {e}", path.display())))?;
    Ok(s)
}

/// Checks the namespace this process runs in, from `root` (`/` in use; a fixture in tests).
pub fn probe(root: &Path) -> Result<(), NotIsolated> {
    let net = root.join("sys/class/net");
    let mut interfaces: Vec<String> = fs::read_dir(&net)
        .map_err(|e| NotIsolated(format!("{} could not be read: {e}", net.display())))?
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect();
    interfaces.sort();
    let r4 = table(&root.join("proc/net/route"))?;
    // A kernel without IPv6 has no table: nothing to route.
    let r6 = table(&root.join("proc/net/ipv6_route")).unwrap_or_default();
    check(&interfaces, &r4, &r6)
}

#[cfg(test)]
mod tests {
    use super::*;

    const R4_HEAD: &str =
        "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n";
    const R6_LO: &str = concat!(
        "00000000000000000000000000000001 80 00000000000000000000000000000000 00 ",
        "00000000000000000000000000000000 00000000 00000001 00000000 80200001       lo\n"
    );

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn loopback_only_namespace_passes() {
        assert_eq!(check(&names(&["lo"]), R4_HEAD, R6_LO), Ok(()));
        assert_eq!(check(&names(&["lo"]), R4_HEAD, ""), Ok(()));
    }

    #[test]
    fn host_interfaces_or_routes_fail_closed() {
        assert!(
            check(&names(&["enp39s0", "lo"]), R4_HEAD, "")
                .unwrap_err()
                .0
                .contains("enp39s0")
        );
        let r4 = format!("{R4_HEAD}eth0\t00000000\t0102A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n");
        assert!(
            check(&names(&["lo"]), &r4, "")
                .unwrap_err()
                .0
                .contains("eth0")
        );
        let r6 = format!(
            "{R6_LO}20010db8000000000000000000000000 40 00000000000000000000000000000000 00 \
             00000000000000000000000000000000 00000100 00000001 00000000 00000001     wlan0\n"
        );
        assert!(
            check(&names(&["lo"]), R4_HEAD, &r6)
                .unwrap_err()
                .0
                .contains("wlan0")
        );
        assert!(check(&[], R4_HEAD, "").is_err(), "no loopback, no relay");
    }

    #[test]
    fn probe_reads_a_fixture_root() {
        let root = std::env::temp_dir().join(format!("wol-tor-netns-{}", std::process::id()));
        fs::create_dir_all(root.join("sys/class/net/lo")).unwrap();
        fs::create_dir_all(root.join("proc/net")).unwrap();
        fs::write(root.join("proc/net/route"), R4_HEAD).unwrap();
        fs::write(root.join("proc/net/ipv6_route"), R6_LO).unwrap();
        assert_eq!(probe(&root), Ok(()));
        fs::create_dir_all(root.join("sys/class/net/eth0")).unwrap();
        assert!(probe(&root).is_err());
        for d in [
            "sys/class/net/lo",
            "sys/class/net/eth0",
            "sys/class/net",
            "sys/class",
            "sys",
        ] {
            let _ = fs::remove_dir(root.join(d));
        }
        for f in ["proc/net/route", "proc/net/ipv6_route"] {
            let _ = fs::remove_file(root.join(f));
        }
        let _ = fs::remove_dir(root.join("proc/net"));
        let _ = fs::remove_dir(root.join("proc"));
        let _ = fs::remove_dir(&root);
    }
}
