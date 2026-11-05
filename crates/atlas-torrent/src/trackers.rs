// SPDX-License-Identifier: BSD-2-Clause
//! Which trackers and peers the helper talks to (DL-SEC-03). A torrent or a magnet link names its
//! trackers, and a tracker names the peers: both come from strangers. The user service manager does
//! not enforce `IPAddressDeny=` for user units, so the fence is here:
//! - at most [`MAX_TRACKERS`] trackers per job, `http`, `https` or `udp` only;
//! - a tracker host may not be `localhost`, a LAN name, or a literal address in the ranges of
//!   [`BOGONS_V4`] / [`BOGONS_V6`] (loopback, private, link-local, carrier NAT, multicast,
//!   documentation, reserved); numeric host spellings a URL parser would read as an address
//!   (`127.1`, `0x7f.1`, `2130706433`), percent signs and non-ASCII hosts are refused too;
//! - every peer address goes through the engine's blocklist, written from the same ranges
//!   ([`blocklist_text`]), for outgoing and incoming peers.
//!
//! A host NAME that resolves to a LAN address is not caught here (the engine resolves it); its
//! peers are still blocked. Tests and the e2e use `--local-peers` (builds with the `tools` feature
//! only), which allows loopback trackers and peers.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Most trackers kept for one job (the rest are dropped).
pub const MAX_TRACKERS: usize = 32;
/// Longest tracker address kept.
pub const MAX_URL: usize = 512;

/// IPv4 networks no tracker or peer of a public swarm lives in.
pub const BOGONS_V4: &[&str] = &[
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.0.0.0/24",
    "192.0.2.0/24",
    "192.168.0.0/16",
    "198.18.0.0/15",
    "198.51.100.0/24",
    "203.0.113.0/24",
    "224.0.0.0/3",
];

/// IPv6 networks likewise (IPv4-mapped and NAT64 addresses are refused as a whole).
pub const BOGONS_V6: &[&str] = &[
    "::/127",
    "::ffff:0:0/96",
    "64:ff9b::/96",
    "100::/64",
    "2001:db8::/32",
    "fc00::/7",
    "fe80::/10",
    "ff00::/8",
];

/// The first and last address of a `net/len` network.
fn range_v4(net: &str) -> Option<(Ipv4Addr, Ipv4Addr)> {
    let (a, l) = net.split_once('/')?;
    let (a, l): (Ipv4Addr, u32) = (a.parse().ok()?, l.parse().ok()?);
    let mask = u32::MAX.checked_shl(32 - l).unwrap_or(0);
    let first = a.to_bits() & mask;
    Some((
        Ipv4Addr::from_bits(first),
        Ipv4Addr::from_bits(first | !mask),
    ))
}

fn range_v6(net: &str) -> Option<(Ipv6Addr, Ipv6Addr)> {
    let (a, l) = net.split_once('/')?;
    let (a, l): (Ipv6Addr, u32) = (a.parse().ok()?, l.parse().ok()?);
    let mask = u128::MAX.checked_shl(128 - l).unwrap_or(0);
    let first = a.to_bits() & mask;
    Some((
        Ipv6Addr::from_bits(first),
        Ipv6Addr::from_bits(first | !mask),
    ))
}

/// The schemes a tracker may use.
const SCHEMES: &[&str] = &["http", "https", "udp"];

/// Host names of this machine or its LAN.
const LOCAL_NAMES: &[&str] = &[
    "localhost",
    "localhost.localdomain",
    "ip6-localhost",
    "ip6-loopback",
];
const LOCAL_SUFFIXES: &[&str] = &[
    ".localhost",
    ".local",
    ".lan",
    ".home",
    ".internal",
    ".intranet",
    ".corp",
    ".home.arpa",
];

/// True when `ip` is in one of the bogon ranges.
pub fn bogon(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => BOGONS_V4
            .iter()
            .filter_map(|n| range_v4(n))
            .any(|(s, e)| (s..=e).contains(&a)),
        IpAddr::V6(a) => BOGONS_V6
            .iter()
            .filter_map(|n| range_v6(n))
            .any(|(s, e)| (s..=e).contains(&a)),
    }
}

/// The host of a tracker address, lowercased; None when the address has no plain host.
fn host_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !SCHEMES.iter().any(|x| x.eq_ignore_ascii_case(scheme)) {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') || authority.contains('%') || !authority.is_ascii() {
        return None;
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        let (h, after) = v6.split_once(']')?;
        if !(after.is_empty() || after.starts_with(':')) {
            return None;
        }
        format!("[{h}]")
    } else {
        authority.split(':').next()?.to_string()
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// True when a URL parser would read `host` as an IPv4 address (its last label is a number).
fn looks_numeric(host: &str) -> bool {
    let last = host.rsplit('.').next().unwrap_or("");
    !last.is_empty()
        && (last.bytes().all(|c| c.is_ascii_digit())
            || (last.starts_with("0x") && last[2..].bytes().all(|c| c.is_ascii_hexdigit())))
}

/// Whether the helper may announce to this tracker (`allow_local`: the tests' loopback tracker).
pub fn allowed(url: &str, allow_local: bool) -> bool {
    if url.len() > MAX_URL || url.bytes().any(|c| c.is_ascii_control() || c == b' ') {
        return false;
    }
    let Some(host) = host_of(url) else {
        return false;
    };
    if allow_local {
        return true;
    }
    if let Some(v6) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return v6.parse::<Ipv6Addr>().is_ok_and(|a| !bogon(IpAddr::V6(a)));
    }
    if looks_numeric(&host) {
        // only the plain dotted form, and not in a bogon range
        return host
            .parse::<Ipv4Addr>()
            .is_ok_and(|a| !bogon(IpAddr::V4(a)));
    }
    if !host.contains('.') || LOCAL_NAMES.contains(&host.as_str()) {
        return false; // a single-label name is a LAN name
    }
    !LOCAL_SUFFIXES.iter().any(|s| host.ends_with(s))
}

/// The trackers kept from `urls`: allowed, without repeats, at most [`MAX_TRACKERS`].
pub fn keep<'a>(urls: impl IntoIterator<Item = &'a str>, allow_local: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for u in urls {
        if out.len() >= MAX_TRACKERS {
            break;
        }
        if allowed(u, allow_local) && !out.iter().any(|o| o == u) {
            out.push(u.to_string());
        }
    }
    out
}

/// The engine's blocklist file (librqbit's `name:first-last` lines) for the bogon ranges.
pub fn blocklist_text() -> String {
    let mut s = String::from("# atlas-torrent: peers never connected to (DL-SEC-03)\n");
    for (a, b) in BOGONS_V4.iter().filter_map(|n| range_v4(n)) {
        s.push_str(&format!("bogon:{a}-{b}\n"));
    }
    for (a, b) in BOGONS_V6.iter().filter_map(|n| range_v6(n)) {
        s.push_str(&format!("bogon:{a}-{b}\n"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_trackers_pass() {
        for u in [
            "udp://tracker.opentrackr.org:1337/announce",
            "https://tracker.example.org/announce",
            "http://93.184.216.34:6969/announce",
            "udp://[2606:4700::1111]:80",
        ] {
            assert!(allowed(u, false), "{u}");
        }
    }

    #[test]
    fn local_and_lan_trackers_are_refused() {
        for u in [
            "http://127.0.0.1:631/",
            "http://localhost/announce",
            "http://LOCALHOST./announce",
            "http://printer.local/announce",
            "http://router/announce",
            "http://192.168.1.1/",
            "http://10.0.0.8:80/",
            "http://169.254.169.254/latest/meta-data",
            "http://100.64.0.1/",
            "udp://[::1]:6969",
            "udp://[fe80::1]:6969",
            "udp://[::ffff:127.0.0.1]:6969",
            "http://127.1/",
            "http://0x7f.1/",
            "http://2130706433/",
            "http://0177.0.0.1/",
            "http://%31%32%37.0.0.1/",
            "http://user@tracker.example.org/",
            "http://１２７.0.0.1/",
            "http://224.0.0.1/",
            "http://0.0.0.0:80/",
            "ftp://tracker.example.org/",
            "file:///etc/passwd",
        ] {
            assert!(!allowed(u, false), "{u}");
        }
        assert!(allowed("http://127.0.0.1:8132/announce", true));
    }

    #[test]
    fn at_most_32_trackers_are_kept() {
        let many: Vec<String> = (0..5000)
            .map(|i| format!("udp://t{i}.example.org:80/announce"))
            .collect();
        let k = keep(many.iter().map(String::as_str), false);
        assert_eq!(k.len(), MAX_TRACKERS);
        assert_eq!(
            keep(["udp://a.example.org:1", "udp://a.example.org:1"], false).len(),
            1
        );
    }

    #[test]
    fn the_blocklist_covers_the_bogons() {
        let t = blocklist_text();
        assert!(t.contains("bogon:127.0.0.0-127.255.255.255"));
        assert!(t.contains("bogon:fe80::-febf:"));
        assert!(t.contains("bogon:224.0.0.0-255.255.255.255"));
        assert!(t.contains("bogon:::-::1\n"));
        assert_eq!(t.lines().count(), 1 + BOGONS_V4.len() + BOGONS_V6.len());
        for ip in [
            "127.0.0.1",
            "192.168.0.10",
            "169.254.169.254",
            "::1",
            "fd00::1",
        ] {
            assert!(bogon(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["93.184.216.34", "2606:4700::1111"] {
            assert!(!bogon(ip.parse().unwrap()), "{ip}");
        }
    }
}
