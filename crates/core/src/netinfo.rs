//! Network facts for Settings > Network and Wi-Fi: interfaces, addresses, DNS, gateway, known networks.
use crate::system::Host;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IfKind {
    #[default]
    Other,
    Ethernet,
    WiFi,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Interface {
    pub name: String,
    pub kind: IfKind,
    pub up: bool,
    pub mac: String,
    /// "192.168.1.5/24".
    pub ipv4: Vec<String>,
    /// Global addresses only, "2001:db8::5/64".
    pub ipv6: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NetInfo {
    pub interfaces: Vec<Interface>,
    pub dns: Vec<String>,
    pub search: Vec<String>,
    pub gateway: Option<String>,
}

/// Interfaces with no user meaning: loopback, pf logging/sync, IPsec, tunnels.
const HIDDEN: [&str; 6] = ["lo", "pflog", "pfsync", "enc", "gif", "stf"];

fn hidden(name: &str) -> bool {
    HIDDEN.iter().any(|p| {
        name.strip_prefix(p)
            .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// "0xffffff00" -> 24.
fn prefix_of_netmask(mask: &str) -> Option<u32> {
    let m = u32::from_str_radix(mask.trim_start_matches("0x"), 16).ok()?;
    Some(m.count_ones())
}

/// FreeBSD `ifconfig` (all interfaces), user-visible ones in order.
pub fn parse_ifconfig(text: &str) -> Vec<Interface> {
    let mut v: Vec<Interface> = Vec::new();
    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) {
            let Some((name, _)) = line.split_once(':') else {
                continue;
            };
            v.push(Interface {
                name: name.trim().into(),
                kind: if name.starts_with("wlan") {
                    IfKind::WiFi
                } else {
                    IfKind::Other
                },
                ..Default::default()
            });
            continue;
        }
        let Some(cur) = v.last_mut() else { continue };
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            ["ether", mac, ..] => cur.mac = (*mac).into(),
            ["inet", addr, "netmask", mask, ..] => {
                if let Some(p) = prefix_of_netmask(mask) {
                    cur.ipv4.push(format!("{addr}/{p}"));
                }
            }
            ["inet6", addr, "prefixlen", p, ..] => {
                let a = addr.split('%').next().unwrap_or(addr);
                if !a.starts_with("fe80:") && a != "::1" {
                    cur.ipv6.push(format!("{a}/{p}"));
                }
            }
            ["media:", rest @ ..] => {
                let m = rest.join(" ");
                if m.starts_with("IEEE 802.11") {
                    cur.kind = IfKind::WiFi;
                } else if m.starts_with("Ethernet") && cur.kind == IfKind::Other {
                    cur.kind = IfKind::Ethernet;
                }
            }
            ["status:", s, ..] => cur.up = *s == "active" || *s == "associated",
            _ => {}
        }
    }
    // Unaddressed tunnels are idle VPN devices.
    let idle_tun =
        |i: &Interface| i.name.starts_with("tun") && i.ipv4.is_empty() && i.ipv6.is_empty();
    v.retain(|i| !hidden(&i.name) && !idle_tun(i));
    v
}

/// Linux dev host `ip -j addr`.
pub fn parse_ip_addr_json(json: &str) -> Vec<Interface> {
    let Ok(serde_json::Value::Array(a)) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    a.iter()
        .filter_map(|i| {
            let s = |k: &str| i.get(k).and_then(|x| x.as_str()).unwrap_or("");
            let name = s("ifname");
            if name.is_empty() || name == "lo" {
                return None;
            }
            let mut it = Interface {
                name: name.into(),
                kind: if name.starts_with("wl") {
                    IfKind::WiFi
                } else if name.starts_with("en") || name.starts_with("eth") {
                    IfKind::Ethernet
                } else {
                    IfKind::Other
                },
                up: s("operstate") == "UP",
                mac: s("address").into(),
                ..Default::default()
            };
            for a in i
                .get("addr_info")
                .and_then(|x| x.as_array())
                .into_iter()
                .flatten()
            {
                let (Some(fam), Some(local), Some(p)) = (
                    a.get("family").and_then(|x| x.as_str()),
                    a.get("local").and_then(|x| x.as_str()),
                    a.get("prefixlen").and_then(|x| x.as_u64()),
                ) else {
                    continue;
                };
                match fam {
                    "inet" => it.ipv4.push(format!("{local}/{p}")),
                    "inet6" if a.get("scope").and_then(|x| x.as_str()) == Some("global") => {
                        it.ipv6.push(format!("{local}/{p}"))
                    }
                    _ => {}
                }
            }
            Some(it)
        })
        .collect()
}

/// resolv.conf(5): (nameservers, search domains).
pub fn parse_resolv_conf(text: &str) -> (Vec<String>, Vec<String>) {
    let (mut dns, mut search) = (Vec::new(), Vec::new());
    for l in text.lines() {
        let mut f = l.split_whitespace();
        match f.next() {
            Some("nameserver") => dns.extend(f.next().map(String::from)),
            Some("search" | "domain") => search.extend(f.map(String::from)),
            _ => {}
        }
    }
    (dns, search)
}

/// FreeBSD `route -n get default`: "gateway: 192.168.1.1".
pub fn parse_route_get(text: &str) -> Option<String> {
    crate::system::pairs(text, ':')
        .get("gateway")
        .filter(|g| !g.is_empty())
        .map(|g| g.to_string())
}

/// Linux `ip route show default`: "default via 192.168.1.1 dev ...".
pub fn parse_ip_route(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        f.windows(2)
            .find(|w| w[0] == "via")
            .map(|w| w[1].to_string())
    })
}

/// Interfaces, DNS and default gateway (forks two short commands; call off the UI thread).
pub fn read(h: &dyn Host) -> NetInfo {
    let (interfaces, gateway) = if h.is_freebsd() {
        (
            h.run(&["ifconfig"]).map(|t| parse_ifconfig(&t)),
            h.run(&["route", "-n", "get", "default"])
                .and_then(|t| parse_route_get(&t)),
        )
    } else {
        (
            h.run(&["ip", "-j", "addr"]).map(|t| parse_ip_addr_json(&t)),
            h.run(&["ip", "route", "show", "default"])
                .and_then(|t| parse_ip_route(&t)),
        )
    };
    let (dns, search) = h
        .read_text("/etc/resolv.conf")
        .map(|t| parse_resolv_conf(&t))
        .unwrap_or_default();
    NetInfo {
        interfaces: interfaces.unwrap_or_default(),
        dns,
        search,
        gateway,
    }
}

fn push_unique(v: &mut Vec<String>, s: String) {
    if !s.is_empty() && !v.contains(&s) {
        v.push(s);
    }
}

/// `wpa_cli -i wlan0 list_networks`: SSIDs in configuration order.
pub fn parse_wpa_networks(text: &str) -> Vec<String> {
    let mut v = Vec::new();
    for l in text.lines().skip(1) {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() >= 2 && f[0].trim().parse::<u32>().is_ok() {
            push_unique(&mut v, f[1].to_string());
        }
    }
    v
}

/// `nmcli -t -f NAME,TYPE connection show`: Wi-Fi connection names.
pub fn parse_nmcli_connections(text: &str) -> Vec<String> {
    let mut v = Vec::new();
    for l in text.lines() {
        if let Some(name) = l.strip_suffix(":802-11-wireless") {
            push_unique(&mut v, name.replace("\\:", ":"));
        }
    }
    v
}

/// Saved Wi-Fi networks (the ones Forget can remove).
pub fn known_networks(h: &dyn Host, wlan: &str) -> Vec<String> {
    if h.is_freebsd() {
        h.run(&["wpa_cli", "-i", wlan, "list_networks"])
            .map(|t| parse_wpa_networks(&t))
            .unwrap_or_default()
    } else {
        h.run(&["nmcli", "-t", "-f", "NAME,TYPE", "connection", "show"])
            .map(|t| parse_nmcli_connections(&t))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

    const IFCONFIG: &str =
        "em0: flags=1008843<UP,BROADCAST,RUNNING,SIMPLEX,MULTICAST,LOWER_UP> metric 0 mtu 1500
\toptions=4e524bb<RXCSUM,TXCSUM,VLAN_MTU>
\tether 52:54:00:12:34:56
\tinet 10.0.2.15 netmask 0xffffff00 broadcast 10.0.2.255
\tinet6 fe80::5054:ff:fe12:3456%em0 prefixlen 64 scopeid 0x1
\tinet6 2001:db8::15 prefixlen 64 autoconf
\tmedia: Ethernet autoselect (1000baseT <full-duplex>)
\tstatus: active
\tnd6 options=23<PERFORMNUD,ACCEPT_RTADV,AUTO_LINKLOCAL>
lo0: flags=1008049<UP,LOOPBACK,RUNNING,MULTICAST,LOWER_UP> metric 0 mtu 16384
\tinet 127.0.0.1 netmask 0xff000000
\tinet6 ::1 prefixlen 128
pflog0: flags=0 metric 0 mtu 33152
wlan0: flags=8843<UP,BROADCAST,RUNNING,SIMPLEX,MULTICAST> metric 0 mtu 1500
\tether 3c:58:c2:aa:bb:cc
\tinet 192.168.1.5 netmask 0xffffff00 broadcast 192.168.1.255
\tgroups: wlan
\tssid Home channel 6 (2437 MHz 11g ht/20) bssid 00:11:22:33:44:55
\tmedia: IEEE 802.11 Wireless Ethernet MCS mode 11ng
\tstatus: associated
re0: flags=8802<BROADCAST,SIMPLEX,MULTICAST> metric 0 mtu 1500
\tether 00:e0:4c:00:00:01
\tmedia: Ethernet autoselect
\tstatus: no carrier
";

    #[test]
    fn freebsd_ifconfig() {
        let v = parse_ifconfig(IFCONFIG);
        assert_eq!(
            v.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["em0", "wlan0", "re0"]
        );
        assert_eq!(
            v[0],
            Interface {
                name: "em0".into(),
                kind: IfKind::Ethernet,
                up: true,
                mac: "52:54:00:12:34:56".into(),
                ipv4: vec!["10.0.2.15/24".into()],
                ipv6: vec!["2001:db8::15/64".into()],
            }
        );
        assert_eq!((v[1].kind, v[1].up), (IfKind::WiFi, true));
        assert_eq!(v[1].ipv4, ["192.168.1.5/24"]);
        assert_eq!(
            (v[2].kind, v[2].up, v[2].ipv4.len()),
            (IfKind::Ethernet, false, 0)
        );
        assert!(parse_ifconfig("").is_empty());
    }

    #[test]
    fn linux_ip_json() {
        let j = r#"[{"ifindex":1,"ifname":"lo","operstate":"UNKNOWN","address":"00:00:00:00:00:00","addr_info":[{"family":"inet","local":"127.0.0.1","prefixlen":8}]},
{"ifindex":2,"ifname":"wlp2s0","operstate":"UP","address":"3c:58:c2:aa:bb:cc","addr_info":[{"family":"inet","local":"192.168.1.5","prefixlen":24},{"family":"inet6","local":"fe80::1","prefixlen":64,"scope":"link"},{"family":"inet6","local":"2001:db8::5","prefixlen":64,"scope":"global"}]},
{"ifindex":3,"ifname":"enp0s31f6","operstate":"DOWN","address":"aa:bb:cc:dd:ee:ff","addr_info":[]}]"#;
        let v = parse_ip_addr_json(j);
        assert_eq!(v.len(), 2);
        assert_eq!(
            v[0],
            Interface {
                name: "wlp2s0".into(),
                kind: IfKind::WiFi,
                up: true,
                mac: "3c:58:c2:aa:bb:cc".into(),
                ipv4: vec!["192.168.1.5/24".into()],
                ipv6: vec!["2001:db8::5/64".into()],
            }
        );
        assert_eq!((v[1].kind, v[1].up), (IfKind::Ethernet, false));
        assert!(parse_ip_addr_json("junk").is_empty());
    }

    #[test]
    fn resolv_and_routes() {
        let r = "# Generated by resolvconf\nsearch lan example.org\nnameserver 192.168.1.1\nnameserver 2001:db8::1\n domain home\noptions edns0\n";
        assert_eq!(
            parse_resolv_conf(r),
            (
                vec!["192.168.1.1".into(), "2001:db8::1".into()],
                vec!["lan".into(), "example.org".into(), "home".into()]
            )
        );
        let rg = "   route to: default\ndestination: default\n       mask: default\n    gateway: 192.168.1.1\n        fib: 0\n  interface: wlan0\n";
        assert_eq!(parse_route_get(rg).as_deref(), Some("192.168.1.1"));
        assert_eq!(parse_route_get("route: route has not been found\n"), None);
        assert_eq!(
            parse_ip_route("default via 10.0.0.1 dev wlp2s0 proto dhcp metric 600\n").as_deref(),
            Some("10.0.0.1")
        );
        assert_eq!(parse_ip_route(""), None);
    }

    #[test]
    fn known_network_lists() {
        let w = "network id / ssid / bssid / flags\n0\tHome\tany\t[CURRENT]\n1\tCafe Bar\tany\t\n2\tHome\tany\t[DISABLED]\n";
        assert_eq!(parse_wpa_networks(w), ["Home", "Cafe Bar"]);
        let n = "Home:802-11-wireless\nWired connection 1:802-3-ethernet\nCafe\\:Bar:802-11-wireless\nlo:loopback\n";
        assert_eq!(parse_nmcli_connections(n), ["Home", "Cafe:Bar"]);
    }

    #[test]
    fn reads_through_host() {
        let h = FakeHost::new(true);
        h.command("ifconfig", IFCONFIG)
            .command("route -n get default", "    gateway: 10.0.2.2\n")
            .command(
                "wpa_cli -i wlan0 list_networks",
                "network id / ssid / bssid / flags\n0\tHome\tany\t[CURRENT]\n",
            )
            .file("/etc/resolv.conf", "nameserver 10.0.2.3\n");
        let n = read(&h);
        assert_eq!(n.interfaces.len(), 3);
        assert_eq!(n.gateway.as_deref(), Some("10.0.2.2"));
        assert_eq!(n.dns, ["10.0.2.3"]);
        assert_eq!(known_networks(&h, "wlan0"), ["Home"]);
        let l = FakeHost::new(false);
        l.command(
            "nmcli -t -f NAME,TYPE connection show",
            "Home:802-11-wireless\n",
        );
        assert_eq!(known_networks(&l, "wlan0"), ["Home"]);
        assert_eq!(read(&l), NetInfo::default(), "no tools, no facts");
    }
}
