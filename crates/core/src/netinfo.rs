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

/// Interfaces with no user meaning: loopback and the virtual links containers and VMs add.
fn hidden(name: &str) -> bool {
    name == "lo"
        || ["veth", "docker", "br-", "virbr", "vnet"]
            .iter()
            .any(|p| name.starts_with(p))
}

/// `ip -j addr`: user-visible interfaces in order. Kind from the name; [`read`] corrects it from sysfs.
pub fn parse_ip_addr_json(json: &str) -> Vec<Interface> {
    let Ok(serde_json::Value::Array(a)) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    a.iter()
        .filter_map(|i| {
            let s = |k: &str| i.get(k).and_then(|x| x.as_str()).unwrap_or("");
            let name = s("ifname");
            if name.is_empty() || hidden(name) {
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

/// `ip route show default`: "default via 192.168.1.1 dev ...".
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
    let mut interfaces = h
        .run(&["ip", "-j", "addr"])
        .map(|t| parse_ip_addr_json(&t))
        .unwrap_or_default();
    for i in &mut interfaces {
        // A `wireless` directory makes it Wi-Fi whatever its name (iwd names it wlan0).
        if !h
            .list_dir(&format!("/sys/class/net/{}/wireless", i.name))
            .is_empty()
        {
            i.kind = IfKind::WiFi;
        }
    }
    let gateway = h
        .run(&["ip", "route", "show", "default"])
        .and_then(|t| parse_ip_route(&t));
    let (dns, search) = h
        .read_text("/etc/resolv.conf")
        .map(|t| parse_resolv_conf(&t))
        .unwrap_or_default();
    NetInfo {
        interfaces,
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

/// Saved Wi-Fi networks (NetworkManager connections; the ones Forget can remove). `_wlan` is
/// unused: NetworkManager keeps connections per machine, not per interface.
pub fn known_networks(h: &dyn Host, _wlan: &str) -> Vec<String> {
    h.run(&["nmcli", "-t", "-f", "NAME,TYPE", "connection", "show"])
        .map(|t| parse_nmcli_connections(&t))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

    #[test]
    fn linux_ip_json() {
        let j = r#"[{"ifindex":1,"ifname":"lo","operstate":"UNKNOWN","address":"00:00:00:00:00:00","addr_info":[{"family":"inet","local":"127.0.0.1","prefixlen":8}]},
{"ifindex":2,"ifname":"wlp2s0","operstate":"UP","address":"3c:58:c2:aa:bb:cc","addr_info":[{"family":"inet","local":"192.168.1.5","prefixlen":24},{"family":"inet6","local":"fe80::1","prefixlen":64,"scope":"link"},{"family":"inet6","local":"2001:db8::5","prefixlen":64,"scope":"global"}]},
{"ifindex":3,"ifname":"enp0s31f6","operstate":"DOWN","address":"aa:bb:cc:dd:ee:ff","addr_info":[]},
{"ifindex":4,"ifname":"docker0","operstate":"DOWN","address":"02:42:00:00:00:01","addr_info":[]},
{"ifindex":5,"ifname":"veth12ab","operstate":"UP","address":"02:42:00:00:00:02","addr_info":[]}]"#;
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
        assert_eq!(
            parse_ip_route("default via 10.0.0.1 dev wlp2s0 proto dhcp metric 600\n").as_deref(),
            Some("10.0.0.1")
        );
        assert_eq!(parse_ip_route(""), None);
    }

    #[test]
    fn known_network_lists() {
        let n = "Home:802-11-wireless\nWired connection 1:802-3-ethernet\nCafe\\:Bar:802-11-wireless\nlo:loopback\n";
        assert_eq!(parse_nmcli_connections(n), ["Home", "Cafe:Bar"]);
    }

    #[test]
    fn reads_through_host() {
        let l = FakeHost::linux();
        assert_eq!(read(&l), NetInfo::default(), "no tools, no facts");
        l.command(
            "ip -j addr",
            r#"[{"ifname":"wlan0","operstate":"UP","address":"3c:58:c2:aa:bb:cc","addr_info":[{"family":"inet","local":"192.168.3.64","prefixlen":24}]}]"#,
        )
        .command(
            "ip route show default",
            "default via 192.168.3.1 dev wlan0 proto dhcp src 192.168.3.64 metric 600\n",
        )
        .command(
            "nmcli -t -f NAME,TYPE connection show",
            "Home:802-11-wireless\n",
        )
        .file("/sys/class/net/wlan0/wireless/uevent", "")
        .file("/etc/resolv.conf", "nameserver 192.168.3.1\n");
        let n = read(&l);
        assert_eq!(
            (n.interfaces[0].kind, n.gateway.as_deref(), n.dns.as_slice()),
            (
                IfKind::WiFi,
                Some("192.168.3.1"),
                &["192.168.3.1".to_string()][..]
            )
        );
        assert_eq!(known_networks(&l, "wlan0"), ["Home"]);
    }
}
