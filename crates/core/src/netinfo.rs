// SPDX-License-Identifier: BSD-2-Clause
//! Network facts for Settings > Network and Wi-Fi: interfaces, addresses, DNS, gateway, known networks.
use crate::system::Host;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IfKind {
    #[default]
    Other,
    Ethernet,
    WiFi,
    /// A tunnel (WireGuard, OpenVPN tun, PPP): `link_type` none (QA-NET-28).
    Vpn,
}

/// NetworkManager's own answer to "can this computer reach the internet" (`nmcli -t -g
/// CONNECTIVITY general`): one definition of online for the tray, Control Center, `wol network`
/// and wold's facts (QA-NET-12).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Connectivity {
    /// NetworkManager did not say (not running, check disabled).
    #[default]
    Unknown,
    /// No network at all.
    None,
    /// A network, but a sign-in page answers every web request (a café or hotel; NET2-3).
    Portal,
    /// A network, but the internet cannot be reached through it.
    Limited,
    Full,
}

/// [`Connectivity`] from `nmcli -t -g CONNECTIVITY general`.
pub fn parse_connectivity(text: &str) -> Connectivity {
    match text.trim() {
        "full" => Connectivity::Full,
        "limited" => Connectivity::Limited,
        "portal" => Connectivity::Portal,
        "none" => Connectivity::None,
        _ => Connectivity::Unknown,
    }
}

/// NetworkManager's connectivity state argv (answered from its cache: no probe is sent).
pub const CONNECTIVITY_ARGV: [&str; 6] = ["nmcli", "-t", "-g", "CONNECTIVITY", "general", "status"];

/// NetworkManager's Metered property (the same read as /usr/lib/wolos/update-download).
pub const METERED_ARGV: [&str; 6] = [
    "busctl",
    "get-property",
    "org.freedesktop.NetworkManager",
    "/org/freedesktop/NetworkManager",
    "org.freedesktop.NetworkManager",
    "Metered",
];

/// `u 1` (yes) or `u 3` (guess yes: a phone hotspot, a mobile modem) is metered.
pub fn parse_metered(text: &str) -> bool {
    matches!(text.split_whitespace().nth(1), Some("1" | "3"))
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
    /// This link's own DNS servers (NetworkManager), not the whole computer's list: a VPN's
    /// resolver shows under the VPN (QA-NET-28).
    #[serde(default)]
    pub dns: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NetInfo {
    pub interfaces: Vec<Interface>,
    pub dns: Vec<String>,
    pub search: Vec<String>,
    pub gateway: Option<String>,
    /// The link that carries the default route (IPv4 first, else IPv6).
    #[serde(default)]
    pub route_dev: Option<String>,
    #[serde(default)]
    pub connectivity: Connectivity,
    /// The connection in use is metered (NetworkManager's guess counts).
    #[serde(default)]
    pub metered: bool,
}

impl NetInfo {
    /// Online: a default route (IPv4 or IPv6), unless NetworkManager says there is no network.
    pub fn online(&self) -> bool {
        self.gateway.is_some() && self.connectivity != Connectivity::None
    }
    /// A network that does not reach the internet (NetworkManager: limited).
    pub fn limited(&self) -> bool {
        self.connectivity == Connectivity::Limited
    }
    /// A sign-in page holds the network (NetworkManager: portal).
    pub fn portal(&self) -> bool {
        self.connectivity == Connectivity::Portal
    }
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
            let flags: Vec<&str> = i
                .get("flags")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|f| f.as_str()).collect())
                .unwrap_or_default();
            let tunnel = s("link_type") == "none";
            let mut it = Interface {
                name: name.into(),
                kind: if tunnel {
                    IfKind::Vpn
                } else if name.starts_with("wl") {
                    IfKind::WiFi
                } else if name.starts_with("en") || name.starts_with("eth") {
                    IfKind::Ethernet
                } else {
                    IfKind::Other
                },
                // A tunnel has no carrier to report: its operstate is UNKNOWN while it runs.
                up: s("operstate") == "UP"
                    || (s("operstate") == "UNKNOWN" && tunnel && flags.contains(&"LOWER_UP")),
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
    word_after(text, "via")
}

/// The `dev` of the first default route.
pub fn parse_route_dev(text: &str) -> Option<String> {
    word_after(text, "dev")
}

fn word_after(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        f.windows(2).find(|w| w[0] == key).map(|w| w[1].to_string())
    })
}

/// `nmcli -t -f GENERAL.DEVICE,IP4.DNS,IP6.DNS device show`: DNS servers per device.
pub fn parse_device_dns(text: &str) -> Vec<(String, Vec<String>)> {
    let mut v: Vec<(String, Vec<String>)> = Vec::new();
    for l in text.lines() {
        let Some((k, val)) = l.split_once(':') else {
            continue;
        };
        let val = crate::wifi::unescape(val.trim());
        if k == "GENERAL.DEVICE" {
            v.push((val, Vec::new()));
        } else if (k.starts_with("IP4.DNS") || k.starts_with("IP6.DNS"))
            && !val.is_empty()
            && let Some(last) = v.last_mut()
        {
            push_unique(&mut last.1, val);
        }
    }
    v
}

/// Per-device DNS argv ([`parse_device_dns`]).
pub const DEVICE_DNS_ARGV: [&str; 6] = [
    "nmcli",
    "-t",
    "-f",
    "GENERAL.DEVICE,IP4.DNS,IP6.DNS",
    "device",
    "show",
];

/// Where NetworkManager's connectivity check is configured, in the order it reads them (a later
/// setting wins): the packaged drop-ins, its own file, the admin's drop-ins.
pub const NM_CONFS: [&str; 3] = [
    "/usr/lib/NetworkManager/conf.d",
    "/etc/NetworkManager/NetworkManager.conf",
    "/etc/NetworkManager/conf.d",
];

/// The `uri=` of the `[connectivity]` section in `text`: the plain-HTTP address NetworkManager
/// probes. A captive portal intercepts it and answers with its sign-in page, so opening it in the
/// browser lands there (QA-NET2-3). Plain HTTP is the point: a portal cannot answer for HTTPS.
pub fn connectivity_uri(text: &str) -> Option<String> {
    let mut inside = false;
    let mut uri = None;
    for l in text.lines().map(str::trim) {
        if l.starts_with('[') {
            inside = l == "[connectivity]";
        } else if inside && let Some(v) = l.strip_prefix("uri=") {
            uri = Some(v.trim().to_owned()).filter(|v| !v.is_empty());
        }
    }
    uri
}

/// The address NetworkManager's connectivity check uses, from its configuration ([`NM_CONFS`];
/// in a directory, the `.conf` files by name; the last that sets it wins). None when no check is
/// configured: then there is no portal state either.
pub fn nm_connectivity_uri(h: &dyn Host) -> Option<String> {
    let mut files = Vec::new();
    for p in NM_CONFS {
        let inside = h.list_dir(p);
        if inside.is_empty() {
            files.push(p.to_owned());
        } else {
            files.extend(inside.into_iter().filter(|f| f.ends_with(".conf")));
        }
    }
    files
        .iter()
        .rev()
        .find_map(|f| h.read_text(f).and_then(|t| connectivity_uri(&t)))
}

/// Interfaces, addresses, DNS (whole computer and per link), the default route, connectivity and
/// metered (short commands side by side; call off the UI thread).
pub fn read(h: &dyn Host) -> NetInfo {
    // The NetworkManager reads run beside the `ip` ones: `wol network` and Settings wait for the
    // slowest probe, not for the sum.
    let ((mut interfaces, route), (per, (connectivity, metered))) = std::thread::scope(|sc| {
        let nm = sc.spawn(|| {
            let per = h.run(&DEVICE_DNS_ARGV).map(|t| parse_device_dns(&t));
            let c = sc.spawn(|| {
                h.run(&CONNECTIVITY_ARGV)
                    .map(|t| parse_connectivity(&t))
                    .unwrap_or_default()
            });
            let m = h.run(&METERED_ARGV).is_some_and(|t| parse_metered(&t));
            (per, (c.join().unwrap_or_default(), m))
        });
        let route = sc.spawn(|| {
            // IPv4 first, else IPv6: an IPv6-only network is online (QA-NET-12a).
            h.run(&["ip", "route", "show", "default"])
                .filter(|t| parse_ip_route(t).is_some())
                .or_else(|| h.run(&["ip", "-6", "route", "show", "default"]))
        });
        let ifs = h
            .run(&["ip", "-j", "addr"])
            .map(|t| parse_ip_addr_json(&t))
            .unwrap_or_default();
        (
            (ifs, route.join().ok().flatten()),
            nm.join().unwrap_or_default(),
        )
    });
    for i in &mut interfaces {
        // A `wireless` directory makes it Wi-Fi whatever its name (iwd names it wlan0).
        if !h
            .list_dir(&format!("/sys/class/net/{}/wireless", i.name))
            .is_empty()
        {
            i.kind = IfKind::WiFi;
        }
        if let Some((_, d)) = per.iter().flatten().find(|(n, _)| *n == i.name) {
            i.dns = d.clone();
        }
    }
    let gateway = route.as_deref().and_then(parse_ip_route);
    let route_dev = route.as_deref().and_then(parse_route_dev);
    let (dns, search) = h
        .read_text("/etc/resolv.conf")
        .map(|t| parse_resolv_conf(&t))
        .unwrap_or_default();
    NetInfo {
        interfaces,
        dns,
        search,
        gateway,
        route_dev,
        connectivity,
        metered,
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
            push_unique(&mut v, crate::wifi::unescape(name));
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

/// The SSIDs of the saved Wi-Fi profiles (QA-SET-16: profiles named "Home 1" are still the
/// network "Home"); None when NetworkManager does not answer. One read per profile: off the UI thread.
pub fn known_ssids(h: &dyn Host) -> Option<Vec<String>> {
    let listing = h.run(&crate::wifi::NM_UUIDS_ARGV)?;
    let mut v = Vec::new();
    for u in crate::wifi::parse_wifi_uuids(&listing) {
        let a = crate::wifi::nm_ssid_argv(&u);
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        if let Some(ssid) = h.run(&a) {
            push_unique(&mut v, crate::wifi::unescape(ssid.trim()));
        }
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// QA-NET2-3: the sign-in page is the address NetworkManager probes, the last setting winning.
    #[test]
    fn the_portal_page_is_what_networkmanager_probes() {
        let conf = "[main]\nuri=http://wrong.example\n[connectivity]\nuri=http://ping.archlinux.org/nm-check.txt\n";
        assert_eq!(
            connectivity_uri(conf).as_deref(),
            Some("http://ping.archlinux.org/nm-check.txt")
        );
        assert_eq!(connectivity_uri("[connectivity]\nenabled=false\n"), None);
        let h = FakeHost::linux();
        assert_eq!(nm_connectivity_uri(&h), None, "no check configured");
        h.file("/usr/lib/NetworkManager/conf.d/20-connectivity.conf", conf)
            .file(
                "/etc/NetworkManager/conf.d/90-mine.conf",
                "[connectivity]\nuri=http://portal.test/\n",
            )
            .file(
                "/etc/NetworkManager/conf.d/91-other.conf",
                "[main]\nplugins=keyfile\n",
            );
        assert_eq!(
            nm_connectivity_uri(&h).as_deref(),
            Some("http://portal.test/")
        );
    }
    use crate::system::fake::FakeHost;

    #[test]
    fn linux_ip_json() {
        let j = r#"[{"ifindex":1,"ifname":"lo","operstate":"UNKNOWN","address":"00:00:00:00:00:00","addr_info":[{"family":"inet","local":"127.0.0.1",
"prefixlen":8}]},
{"ifindex":2,"ifname":"wlp2s0","operstate":"UP","address":"3c:58:c2:aa:bb:cc","addr_info":[{"family":"inet","local":"192.168.1.5",
"prefixlen":24},{"family":"inet6","local":"fe80::1","prefixlen":64,"scope":"link"},{"family":"inet6","local":"2001:db8::5","prefixlen":64,"scope":"global"}]},
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
                dns: vec![],
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

    /// QA-NET-12a: an IPv6-only link (no IPv4 default route) is online: its IPv6 default route
    /// is the gateway.
    #[test]
    fn ipv6_only_is_online() {
        let l = FakeHost::linux();
        l.command("ip route show default", "").command(
            "ip -6 route show default",
            "default via fe80::1 dev enp0s1 proto ra metric 100 pref medium\n",
        );
        assert_eq!(read(&l).gateway.as_deref(), Some("fe80::1"));
    }

    /// QA-NET-12 / NET2-3: NetworkManager's connectivity is the one "online"; portal and
    /// limited are their own states.
    #[test]
    fn limited_and_portal_states() {
        for (t, want) in [
            ("full\n", Connectivity::Full),
            ("portal\n", Connectivity::Portal),
            ("limited\n", Connectivity::Limited),
            ("none\n", Connectivity::None),
            ("unknown\n", Connectivity::Unknown),
            ("", Connectivity::Unknown),
        ] {
            assert_eq!(parse_connectivity(t), want, "{t}");
        }
        let l = FakeHost::linux();
        l.command(
            "ip route show default",
            "default via 10.99.0.1 dev enp0s2\n",
        )
        .command("nmcli -t -g CONNECTIVITY general status", "portal\n");
        let n = read(&l);
        assert!(n.online() && n.portal() && !n.limited(), "{n:?}");
        assert_eq!(n.route_dev.as_deref(), Some("enp0s2"));
        l.command("nmcli -t -g CONNECTIVITY general status", "none\n");
        assert!(!read(&l).online());
        assert!(parse_metered("u 1\n") && parse_metered("u 3") && !parse_metered("u 4"));
    }

    /// QA-NET-28: an active WireGuard tunnel is a VPN that is up, with its own DNS.
    #[test]
    fn wireguard_is_vpn_with_its_dns() {
        let j = r#"[{"ifname":"enp0s1","operstate":"UP","link_type":"ether","address":"52:54:00:12:34:56","addr_info":[{"family":"inet","local":"10.0.2.15",
"prefixlen":24}]},
{"ifname":"qa-wg0","flags":["POINTOPOINT","NOARP","UP","LOWER_UP"],"operstate":"UNKNOWN","link_type":"none","addr_info":[{"family":"inet",
"local":"10.77.0.2","prefixlen":32}]}]"#;
        let l = FakeHost::linux();
        l.command("ip -j addr", j).command(
            "nmcli -t -f GENERAL.DEVICE,IP4.DNS,IP6.DNS device show",
            "GENERAL.DEVICE:enp0s1\nIP4.DNS[1]:10.0.2.3\nGENERAL.DEVICE:qa-wg0\nIP4.DNS[1]:10.77.0.1\nIP6.DNS[1]:fd00\\:\\:1\nGENERAL.DEVICE:lo\n",
        );
        let n = read(&l);
        let wg = &n.interfaces[1];
        assert_eq!((wg.kind, wg.up), (IfKind::Vpn, true), "{wg:?}");
        assert_eq!(wg.dns, ["10.77.0.1", "fd00::1"]);
        assert_eq!(n.interfaces[0].dns, ["10.0.2.3"]);
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
        assert_eq!(known_ssids(&l), None, "NetworkManager did not answer");
        l.command(
            "nmcli -t -f UUID,TYPE connection show",
            "1111-aa:802-11-wireless\n2222-bb:802-11-wireless\n3333-cc:802-3-ethernet\n",
        )
        .command(
            "nmcli -g 802-11-wireless.ssid connection show uuid 1111-aa",
            "Home\n",
        )
        .command(
            "nmcli -g 802-11-wireless.ssid connection show uuid 2222-bb",
            "Home\n",
        );
        assert_eq!(
            known_ssids(&l),
            Some(vec!["Home".to_string()]),
            "Home and Home 1: one network"
        );
    }
}
