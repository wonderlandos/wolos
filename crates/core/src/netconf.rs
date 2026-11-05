// SPDX-License-Identifier: BSD-2-Clause
//! Network settings the person makes (NET-3, docs/modules/wifi.md): a link's IPv4 address by hand
//! or automatic (QA-LIVE3-14), the system proxy, DNS over TLS and airplane mode. Pure parsers,
//! validators and fixed argv here; [`crate::controls`] runs them (`controls/net.rs`) and
//! wol-admind writes the root files ([`crate::admin`] `proxy`, `dns-over-tls`).
use std::net::{IpAddr, Ipv4Addr};

// ---- IPv4 address of one link (QA-LIVE3-14) ----

/// How a link gets its IPv4 address: Automatic (DHCP) or typed in (Manual).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Ipv4Config {
    /// false: Automatic (DHCP); the other fields are ignored.
    pub manual: bool,
    /// "192.168.1.20/24".
    pub address: String,
    /// "192.168.1.1"; empty: no router (a network with no way out).
    pub router: String,
    /// "1.1.1.1", "2606:4700:4700::1111".
    pub dns: Vec<String>,
}

/// What is wrong with a typed address, for the one sentence that says so.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpProblem {
    /// Not four numbers 0-255 with a prefix ("192.168.1.20/24").
    Address,
    /// The address is the subnet's own number or its broadcast one.
    Reserved,
    /// The router is not an IPv4 address.
    Router,
    /// The router is outside the address's subnet (or is the address itself).
    RouterOutside,
    /// A DNS server is not an IP address.
    Dns,
}

impl IpProblem {
    /// One plain sentence (`wol network ip`, Settings).
    pub fn sentence(self) -> &'static str {
        match self {
            Self::Address => {
                "Type the address with its prefix, like 192.168.1.20/24 (the prefix is 1 to 32)."
            }
            Self::Reserved => {
                "That address is the network's own number or its broadcast address; pick another one."
            }
            Self::Router => "Type the router as an IPv4 address, like 192.168.1.1.",
            Self::RouterOutside => {
                "The router must be on the same network as the address (inside its prefix)."
            }
            Self::Dns => {
                "Type each DNS server as an IP address, separated by commas, like 1.1.1.1, 9.9.9.9."
            }
        }
    }
}

/// "a.b.c.d/p" -> (address, prefix 1..=32).
pub fn parse_cidr(s: &str) -> Option<(Ipv4Addr, u8)> {
    let (a, p) = s.trim().split_once('/')?;
    let p: u8 = p.parse().ok().filter(|p| (1..=32).contains(p))?;
    // `Ipv4Addr` parsing refuses leading zeros and out-of-range parts.
    Some((a.parse().ok()?, p))
}

fn mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    }
}

/// A DNS list as typed: commas or spaces between addresses. None when one is not an address.
pub fn parse_dns_list(s: &str) -> Option<Vec<String>> {
    let mut v = Vec::new();
    for t in s.split([',', ' ', ';']).filter(|t| !t.is_empty()) {
        let ip: IpAddr = t.parse().ok()?;
        let ip = ip.to_string();
        if !v.contains(&ip) {
            v.push(ip);
        }
    }
    Some(v)
}

/// The most DNS servers a link takes here (resolved uses the first three anyway).
pub const MAX_DNS: usize = 8;

/// Checks a Manual configuration (Automatic always passes): CIDR address that is not the subnet's
/// own number or broadcast (prefixes 31 and 32 have none), a router inside the subnet that is not
/// the address, DNS servers that are IP addresses.
pub fn validate_ipv4(c: &Ipv4Config) -> Result<(), IpProblem> {
    if !c.manual {
        return Ok(());
    }
    let (ip, p) = parse_cidr(&c.address).ok_or(IpProblem::Address)?;
    let (n, m) = (u32::from(ip), mask(p));
    if p <= 30 && (n & !m == 0 || n & !m == !m) {
        return Err(IpProblem::Reserved);
    }
    if !c.router.trim().is_empty() {
        let r: Ipv4Addr = c.router.trim().parse().map_err(|_| IpProblem::Router)?;
        let r = u32::from(r);
        if r & m != n & m || r == n || (p <= 30 && (r & !m == 0 || r & !m == !m)) {
            return Err(IpProblem::RouterOutside);
        }
    }
    if c.dns.len() > MAX_DNS || c.dns.iter().any(|d| d.parse::<IpAddr>().is_err()) {
        return Err(IpProblem::Dns);
    }
    Ok(())
}

/// The profile (UUID) active on `iface`: `nmcli -g GENERAL.CON-UUID device show <iface>`.
pub fn device_uuid_argv(iface: &str) -> [&str; 6] {
    ["nmcli", "-g", "GENERAL.CON-UUID", "device", "show", iface]
}

/// `nmcli -g GENERAL.STATE device show <iface>`: "100 (connected)" once it is on the network.
pub fn device_state_argv(iface: &str) -> [&str; 6] {
    ["nmcli", "-g", "GENERAL.STATE", "device", "show", iface]
}

/// NetworkManager's device state 100 is "connected" (IP configured).
pub fn device_connected(state: &str) -> bool {
    state.split_whitespace().next() == Some("100")
}

/// A UUID as nmcli prints it (hex and dashes), so it can go into an argv.
pub fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// The IPv4 fields of a profile: `nmcli -t -f ipv4.method,... connection show uuid <uuid>`.
pub fn ipv4_read_argv(uuid: &str) -> Vec<String> {
    sv(&[
        "nmcli",
        "-t",
        "-f",
        "ipv4.method,ipv4.addresses,ipv4.gateway,ipv4.dns",
        "connection",
        "show",
        "uuid",
        uuid,
    ])
}

/// [`ipv4_read_argv`]'s output -> the configuration (`manual` only for method manual).
pub fn parse_ipv4(text: &str) -> Ipv4Config {
    let mut c = Ipv4Config::default();
    for l in text.lines() {
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        let v = v.trim();
        let list = || {
            v.split(',')
                .map(str::trim)
                .filter(|x| !x.is_empty() && *x != "--")
                .map(String::from)
                .collect::<Vec<_>>()
        };
        match k {
            "ipv4.method" => c.manual = v == "manual",
            "ipv4.addresses" => c.address = list().into_iter().next().unwrap_or_default(),
            "ipv4.gateway" if v != "--" => c.router = v.into(),
            "ipv4.dns" => c.dns = list(),
            _ => {}
        }
    }
    c
}

fn sv(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

/// `nmcli connection modify uuid <uuid> ipv4.method …`: Manual sets the address, router and DNS;
/// Automatic clears all three (DHCP gives them).
pub fn ipv4_modify_argv(uuid: &str, c: &Ipv4Config) -> Vec<String> {
    let mut v = sv(&["nmcli", "connection", "modify", "uuid", uuid]);
    if c.manual {
        v.extend(sv(&[
            "ipv4.method",
            "manual",
            "ipv4.addresses",
            c.address.trim(),
            "ipv4.gateway",
            c.router.trim(),
            "ipv4.dns",
            &c.dns.join(","),
        ]));
    } else {
        v.extend(sv(&[
            "ipv4.method",
            "auto",
            "ipv4.addresses",
            "",
            "ipv4.gateway",
            "",
            "ipv4.dns",
            "",
        ]));
    }
    v
}

/// Brings the profile up again so the change applies now.
pub fn up_uuid_argv(uuid: &str) -> Vec<String> {
    sv(&["nmcli", "connection", "up", "uuid", uuid])
}

/// Asks NetworkManager to check the internet now (not its 5-minute cache).
pub const CONNECTIVITY_CHECK_ARGV: [&str; 4] = ["nmcli", "networking", "connectivity", "check"];

/// How long a new address gets to reach the network before it is undone.
pub const IP_TRIAL_SECS: u64 = 15;
/// Time between two checks of the trial.
pub const IP_TRIAL_STEP: std::time::Duration = std::time::Duration::from_secs(1);

/// What a Manual or Automatic change ended as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum IpApplied {
    /// The link is on the network as before: the change stays.
    Kept,
    /// The link lost the network within [`IP_TRIAL_SECS`]: it went back to Automatic (DHCP).
    RolledBack,
}

/// Higher is more reachable: none or unknown < limited < portal < full.
pub fn reach(c: crate::netinfo::Connectivity) -> u8 {
    use crate::netinfo::Connectivity as C;
    match c {
        C::Unknown | C::None => 0,
        C::Limited => 1,
        C::Portal => 2,
        C::Full => 3,
    }
}

// ---- Proxy ----

/// The proxy every program is asked to use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    #[default]
    Off,
    /// A PAC file (`pac_url`) decides per address.
    Auto,
    /// Fixed servers per protocol.
    Manual,
}

/// The system proxy as Settings and `wol proxy` show it ([`PROXY_CONF`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ProxyConfig {
    pub mode: ProxyMode,
    /// Auto: "http://wpad.example/proxy.pac".
    pub pac_url: String,
    /// Manual: "host:port" each, empty = none for that protocol.
    pub http: String,
    pub https: String,
    pub socks: String,
    /// Hosts and networks reached directly ("localhost", ".example.org", "10.0.0.0/8").
    pub no_proxy: Vec<String>,
}

/// What Settings and `wol proxy` read back (written by wol-admind `proxy`).
pub const PROXY_CONF: &str = "/etc/wolos/proxy.conf";
/// The environment of new login sessions (shells and the programs they start).
pub const PROXY_PROFILE: &str = "/etc/profile.d/wolos-proxy.sh";
/// The most "no proxy for" entries.
pub const MAX_NO_PROXY: usize = 64;

/// A host name or address with a port: "proxy.example:3128", "10.0.0.1:8080", "[2001:db8::1]:3128".
pub fn is_host_port(s: &str) -> bool {
    let Some((host, port)) = s.rsplit_once(':') else {
        return false;
    };
    if !port.parse::<u16>().is_ok_and(|p| p > 0) {
        return false;
    }
    if let Some(v6) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return v6.parse::<std::net::Ipv6Addr>().is_ok();
    }
    is_host(host)
}

fn is_host(h: &str) -> bool {
    (1..=253).contains(&h.len())
        && h.split('.').all(|l| {
            (1..=63).contains(&l.len())
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !l.starts_with('-')
                && !l.ends_with('-')
        })
}

/// A PAC address: http or https, no spaces, quotes or control characters (it is written into a
/// shell file and a settings key), and no `user:password@` in front of the host: a credential in a
/// URL ends up in `/etc/wolos/proxy.conf` and in logs (QA-A9-SEC-10).
pub fn is_pac_url(s: &str) -> bool {
    s.split_once("://").is_some_and(|(scheme, rest)| {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        matches!(scheme, "http" | "https") && !rest.is_empty() && !authority.contains('@')
    }) && s.len() <= 2048
        && s.bytes()
            .all(|b| b.is_ascii_graphic() && !b"'\"\\`$".contains(&b))
}

/// One "no proxy for" entry: a host, `.domain`, `*.domain`, an address or a CIDR network.
pub fn is_no_proxy_entry(s: &str) -> bool {
    (1..=253).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-*:/[]".contains(&b))
}

/// A "no proxy for" list as typed: commas or spaces between entries.
pub fn parse_no_proxy(s: &str) -> Option<Vec<String>> {
    let v: Vec<String> = s
        .split([',', ' ', ';'])
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect();
    (v.len() <= MAX_NO_PROXY && v.iter().all(|e| is_no_proxy_entry(e))).then_some(v)
}

/// What is wrong with a proxy setting, in one sentence; None when it can be saved.
pub fn proxy_problem(c: &ProxyConfig) -> Option<&'static str> {
    let manual = [&c.http, &c.https, &c.socks];
    match c.mode {
        ProxyMode::Off => None,
        ProxyMode::Auto if !is_pac_url(&c.pac_url) => Some(
            "Type the web address of the PAC file, like https://wpad.example.com/proxy.pac, without a user name or password in it.",
        ),
        ProxyMode::Manual if manual.iter().all(|s| s.is_empty()) => {
            Some("Type at least one proxy server, like proxy.example.com:3128.")
        }
        ProxyMode::Manual if manual.iter().any(|s| !s.is_empty() && !is_host_port(s)) => {
            Some("Type each server as a name or address and a port, like proxy.example.com:3128.")
        }
        _ if c.no_proxy.len() > MAX_NO_PROXY
            || c.no_proxy.iter().any(|e| !is_no_proxy_entry(e)) =>
        {
            Some(
                "Type the addresses to reach directly as names, domains or networks, separated by commas.",
            )
        }
        _ => None,
    }
}

fn dash(s: &str) -> &str {
    if s.is_empty() { "-" } else { s }
}

fn undash(s: &str) -> String {
    if s == "-" { String::new() } else { s.into() }
}

/// wol-admind `proxy` arguments: `off`, `auto <pac url> <no proxy|->`, `manual <http|-> <https|->
/// <socks|-> <no proxy|->` (lists comma-joined).
pub fn proxy_args(c: &ProxyConfig) -> Vec<String> {
    let np = c.no_proxy.join(",");
    match c.mode {
        ProxyMode::Off => sv(&["off"]),
        ProxyMode::Auto => sv(&["auto", &c.pac_url, dash(&np)]),
        ProxyMode::Manual => sv(&[
            "manual",
            dash(&c.http),
            dash(&c.https),
            dash(&c.socks),
            dash(&np),
        ]),
    }
}

/// [`proxy_args`] back to a configuration; None when malformed or [`proxy_problem`] refuses it.
pub fn proxy_from_args(a: &[&str]) -> Option<ProxyConfig> {
    let list = |s: &str| {
        if s == "-" {
            Some(Vec::new())
        } else {
            parse_no_proxy(s)
        }
    };
    let c = match a {
        ["off"] => ProxyConfig::default(),
        ["auto", url, np] => ProxyConfig {
            mode: ProxyMode::Auto,
            pac_url: (*url).into(),
            no_proxy: list(np)?,
            ..Default::default()
        },
        ["manual", http, https, socks, np] => ProxyConfig {
            mode: ProxyMode::Manual,
            http: undash(http),
            https: undash(https),
            socks: undash(socks),
            no_proxy: list(np)?,
            ..Default::default()
        },
        _ => return None,
    };
    proxy_problem(&c).is_none().then_some(c)
}

/// [`PROXY_CONF`]: `key=value` lines.
pub fn render_proxy_conf(c: &ProxyConfig) -> String {
    let mode = match c.mode {
        ProxyMode::Off => "off",
        ProxyMode::Auto => "auto",
        ProxyMode::Manual => "manual",
    };
    format!(
        concat!(
            "# Settings > Network > Proxy (wol-admind proxy). /etc/profile.d/wolos-proxy.sh follows it.\n",
            "mode={mode}\npac={}\nhttp={}\nhttps={}\nsocks={}\nno-proxy={}\n"
        ),
        c.pac_url,
        c.http,
        c.https,
        c.socks,
        c.no_proxy.join(","),
        mode = mode
    )
}

/// [`PROXY_CONF`] read back; a missing or odd file is Off.
pub fn parse_proxy_conf(text: &str) -> ProxyConfig {
    let mut c = ProxyConfig::default();
    for l in text.lines() {
        let Some((k, v)) = l.split_once('=') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "mode" => {
                c.mode = match v {
                    "auto" => ProxyMode::Auto,
                    "manual" => ProxyMode::Manual,
                    _ => ProxyMode::Off,
                }
            }
            "pac" => c.pac_url = v.into(),
            "http" => c.http = v.into(),
            "https" => c.https = v.into(),
            "socks" => c.socks = v.into(),
            "no-proxy" => c.no_proxy = parse_no_proxy(v).unwrap_or_default(),
            _ => {}
        }
    }
    if proxy_problem(&c).is_some() {
        return ProxyConfig::default();
    }
    c
}

/// [`PROXY_PROFILE`]: the variables curl, git, pacman and most programs read. A PAC file has no
/// variable: only programs that read the desktop's proxy settings follow it. Every value passed
/// [`proxy_problem`] (no quotes), and is single-quoted anyway.
pub fn render_proxy_profile(c: &ProxyConfig) -> String {
    let mut s = String::from(
        "# Written by wol-admind from /etc/wolos/proxy.conf (Settings > Network > Proxy). Do not edit.\n",
    );
    let mut export = |names: [&str; 2], v: &str| {
        s.push_str(&format!("export {0}='{v}' {1}='{v}'\n", names[0], names[1]));
    };
    if c.mode == ProxyMode::Manual {
        for (names, scheme, host) in [
            (["http_proxy", "HTTP_PROXY"], "http", &c.http),
            (["https_proxy", "HTTPS_PROXY"], "http", &c.https),
            (["all_proxy", "ALL_PROXY"], "socks5h", &c.socks),
        ] {
            if !host.is_empty() {
                export(names, &format!("{scheme}://{host}/"));
            }
        }
        let mut np = vec!["localhost".to_string(), "127.0.0.1".into(), "::1".into()];
        for e in &c.no_proxy {
            if !np.contains(e) {
                np.push(e.clone());
            }
        }
        export(["no_proxy", "NO_PROXY"], &np.join(","));
    }
    s
}

/// The desktop's proxy keys (`org.gnome.system.proxy`, read by GTK, GLib and browsers), set as
/// the person: one `gsettings set` per key.
pub fn gsettings_argvs(c: &ProxyConfig) -> Vec<Vec<String>> {
    let schema = "org.gnome.system.proxy";
    let mode = match c.mode {
        ProxyMode::Off => "none",
        ProxyMode::Auto => "auto",
        ProxyMode::Manual => "manual",
    };
    let set = |schema: &str, k: &str, v: &str| sv(&["gsettings", "set", schema, k, v]);
    let mut v = vec![set(schema, "mode", mode)];
    v.push(set(schema, "autoconfig-url", &c.pac_url));
    let hosts: Vec<String> = c.no_proxy.iter().map(|h| format!("'{h}'")).collect();
    v.push(set(
        schema,
        "ignore-hosts",
        &format!("[{}]", hosts.join(", ")),
    ));
    for (sub, hp) in [("http", &c.http), ("https", &c.https), ("socks", &c.socks)] {
        let (host, port) = hp.rsplit_once(':').unwrap_or(("", "0"));
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let s = format!("{schema}.{sub}");
        v.push(set(&s, "host", host));
        v.push(set(&s, "port", port));
    }
    v
}

/// NetworkManager's per-profile proxy (`proxy.method auto` with the PAC file, else none) on the
/// profile `uuid`.
pub fn nm_proxy_argv(uuid: &str, c: &ProxyConfig) -> Vec<String> {
    let pac = c.mode == ProxyMode::Auto;
    sv(&[
        "nmcli",
        "connection",
        "modify",
        "uuid",
        uuid,
        "proxy.method",
        if pac { "auto" } else { "none" },
        "proxy.pac-url",
        if pac { &c.pac_url } else { "" },
    ])
}

// ---- DNS over TLS (systemd-resolved) ----

/// How systemd-resolved uses TLS for DNS (`DNSOverTLS=`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DotMode {
    /// Plain DNS (resolved's default here).
    #[default]
    Off,
    /// TLS when the server answers on port 853, plain DNS otherwise.
    Opportunistic,
    /// TLS or no answer at all.
    Strict,
}

impl DotMode {
    /// The word wol-admind and `wol dns tls` take.
    pub fn arg(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Opportunistic => "opportunistic",
            Self::Strict => "strict",
        }
    }
    /// [`DotMode::arg`] back.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "opportunistic" => Some(Self::Opportunistic),
            "strict" => Some(Self::Strict),
            _ => None,
        }
    }
}

/// DNS over TLS and the computer's own DNS servers ([`DOT_DROPIN`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct DotConfig {
    pub mode: DotMode,
    /// The computer's own DNS servers, used before the network's: "9.9.9.9#dns.quad9.net".
    pub servers: Vec<String>,
}

/// The drop-in wol-admind writes (`dns-over-tls`); resolved reads every `*.conf` there.
pub const DOT_DROPIN: &str = "/etc/systemd/resolved.conf.d/50-wolos-dns-over-tls.conf";
/// systemd-resolved reads its configuration again only when it starts.
pub const RESOLVED_RESTART_ARGV: [&str; 3] = ["systemctl", "restart", "systemd-resolved.service"];
/// The most servers in the list.
pub const MAX_DOT_SERVERS: usize = 8;

/// One server as resolved.conf takes it: an IP address, optionally `#name` (the name its
/// certificate has, checked in Strict mode).
pub fn is_dot_server(s: &str) -> bool {
    let (ip, name) = match s.split_once('#') {
        Some((ip, n)) => (ip, Some(n)),
        None => (s, None),
    };
    ip.parse::<IpAddr>().is_ok() && name.is_none_or(is_host)
}

/// A server list as typed: commas or spaces between servers. None when one is not a server.
pub fn parse_dot_servers(s: &str) -> Option<Vec<String>> {
    let v: Vec<String> = s
        .split([',', ' ', ';'])
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect();
    (v.len() <= MAX_DOT_SERVERS && v.iter().all(|x| is_dot_server(x))).then_some(v)
}

/// What is wrong, in one sentence; None when it can be saved. Strict needs servers of its own:
/// with only the network's resolver (a home router never speaks TLS) every lookup fails
/// (network-r2: 347 attempts on port 853, no name answered).
pub fn dot_problem(c: &DotConfig) -> Option<&'static str> {
    if c.servers.len() > MAX_DOT_SERVERS || c.servers.iter().any(|s| !is_dot_server(s)) {
        return Some(
            "Type each server as an IP address, optionally with its name after #, like 9.9.9.9#dns.quad9.net.",
        );
    }
    if c.mode == DotMode::Strict && c.servers.is_empty() {
        return Some(
            "Always needs at least one server that speaks DNS over TLS, like 9.9.9.9#dns.quad9.net.",
        );
    }
    None
}

/// wol-admind `dns-over-tls` arguments: `<off|opportunistic|strict> [server…]`.
pub fn dot_args(c: &DotConfig) -> Vec<String> {
    let mut v = vec![c.mode.arg().to_string()];
    v.extend(c.servers.iter().cloned());
    v
}

/// [`dot_args`] back; None when malformed or [`dot_problem`] refuses it.
pub fn dot_from_args(a: &[&str]) -> Option<DotConfig> {
    let (m, rest) = a.split_first()?;
    let c = DotConfig {
        mode: DotMode::parse(m)?,
        servers: rest.iter().map(|s| (*s).to_string()).collect(),
    };
    dot_problem(&c).is_none().then_some(c)
}

/// [`DOT_DROPIN`]'s text.
pub fn render_dot(c: &DotConfig) -> String {
    let mode = match c.mode {
        DotMode::Off => "no",
        DotMode::Opportunistic => "opportunistic",
        DotMode::Strict => "yes",
    };
    let mut s = format!(
        "# Settings > Network > DNS over TLS (wol-admind dns-over-tls). Do not edit.\n[Resolve]\nDNSOverTLS={mode}\n"
    );
    if !c.servers.is_empty() {
        s.push_str(&format!("DNS={}\n", c.servers.join(" ")));
    }
    s
}

/// [`DOT_DROPIN`] read back (missing: Off, no servers).
pub fn parse_dot(text: &str) -> DotConfig {
    let mut c = DotConfig::default();
    for l in text.lines() {
        match l.split_once('=') {
            Some(("DNSOverTLS", v)) => {
                c.mode = match v.trim() {
                    "opportunistic" => DotMode::Opportunistic,
                    "yes" | "true" | "1" => DotMode::Strict,
                    _ => DotMode::Off,
                }
            }
            Some(("DNS", v)) => {
                c.servers = v
                    .split_whitespace()
                    .filter(|s| is_dot_server(s))
                    .map(String::from)
                    .collect()
            }
            _ => {}
        }
    }
    c
}

// ---- Airplane mode (rfkill) ----

/// Every radio's soft switch, JSON (util-linux rfkill).
pub const RFKILL_LIST_ARGV: [&str; 4] = ["rfkill", "-J", "-o", "TYPE,SOFT"];

/// One radio as rfkill lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Radio {
    /// "wlan", "bluetooth", "wwan"…
    pub kind: String,
    /// Switched off in software (airplane mode, `nmcli radio wifi off`).
    pub blocked: bool,
}

/// [`RFKILL_LIST_ARGV`]'s output: `{"rfkilldevices": [{"type": "wlan", "soft": "blocked"}]}`.
pub fn parse_rfkill(json: &str) -> Option<Vec<Radio>> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let devs = v.get("rfkilldevices")?.as_array()?;
    Some(
        devs.iter()
            .filter_map(|d| {
                Some(Radio {
                    kind: d.get("type")?.as_str()?.to_string(),
                    blocked: d.get("soft")?.as_str()? == "blocked",
                })
            })
            .collect(),
    )
}

/// A radio type rfkill takes as an argument.
pub fn is_radio_kind(s: &str) -> bool {
    matches!(
        s,
        "wlan" | "bluetooth" | "uwb" | "wimax" | "wwan" | "gps" | "fm" | "nfc"
    )
}

/// Where the radios that were on before airplane mode are kept, under `$XDG_STATE_HOME`
/// (`<state home>/wolos/airplane`; [`airplane_path`] says where): one rfkill type per line.
pub const AIRPLANE_FILE: &str = "wolos/airplane";

/// The computer's own folder for that list, writable by every account (made by tmpfiles.d): the
/// radios are the computer's, not one account's, so a person who signs in after airplane mode was
/// turned on sees it on, and turning it off brings back the radios that were on before it (a
/// list per account saw "off" while every radio was blocked; QA-A7-13).
pub const AIRPLANE_SHARED_DIR: &str = "/var/lib/wolos/airplane";

/// The list's file: in [`AIRPLANE_SHARED_DIR`] where the computer has it, else (a machine set up
/// by hand) in the account's state home as [`AIRPLANE_FILE`].
pub fn airplane_path(state_home: &std::path::Path) -> std::path::PathBuf {
    airplane_path_in(std::path::Path::new(AIRPLANE_SHARED_DIR), state_home)
}

/// [`airplane_path`] with the shared folder given.
pub fn airplane_path_in(
    shared: &std::path::Path,
    state_home: &std::path::Path,
) -> std::path::PathBuf {
    if shared.is_dir() {
        shared.join("radios")
    } else {
        state_home.join(AIRPLANE_FILE)
    }
}

/// The radio types that were on (not blocked) before airplane mode, once each.
pub fn radios_on(radios: &[Radio]) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for r in radios.iter().filter(|r| !r.blocked) {
        if !v.contains(&r.kind) {
            v.push(r.kind.clone());
        }
    }
    v
}

/// The saved list read back (unknown words dropped).
pub fn parse_airplane_file(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|k| is_radio_kind(k))
        .map(String::from)
        .collect()
}

/// The saved list for airplane mode turned on now: one type per line.
pub fn airplane_file_text(radios: &[Radio]) -> String {
    let mut t = radios_on(radios).join("\n");
    t.push('\n');
    t
}

/// Airplane mode is on: it was turned on here (the saved list exists) and a radio is still
/// switched off. A list left behind by `rfkill unblock all` from a terminal reads as off.
pub fn airplane_on(saved: Option<&str>, radios: &[Radio]) -> bool {
    saved.is_some() && radios.iter().any(|r| r.blocked)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// QA-A7-13: the airplane list belongs to the computer where it has a folder for it, and to
    /// the account's state home on a machine set up by hand.
    #[test]
    fn the_airplane_list_is_the_computers_when_it_has_a_folder() {
        let d = std::env::temp_dir().join(format!("wol-airplane-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (shared, state) = (d.join("shared"), d.join("state"));
        assert_eq!(airplane_path_in(&shared, &state), state.join(AIRPLANE_FILE));
        std::fs::create_dir_all(&shared).unwrap();
        assert_eq!(airplane_path_in(&shared, &state), shared.join("radios"));
        // Every account reads and writes the same place.
        assert_eq!(
            airplane_path_in(&shared, &state),
            airplane_path_in(&shared, &d.join("another-account"))
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    fn manual(a: &str, r: &str, dns: &[&str]) -> Ipv4Config {
        Ipv4Config {
            manual: true,
            address: a.into(),
            router: r.into(),
            dns: dns.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// QA-LIVE3-14: what a typed address may be (CIDR, router in the subnet, DNS addresses).
    #[test]
    fn typed_ipv4_addresses_are_checked() {
        assert_eq!(
            validate_ipv4(&manual("192.168.1.20/24", "192.168.1.1", &["1.1.1.1"])),
            Ok(())
        );
        assert_eq!(validate_ipv4(&Ipv4Config::default()), Ok(()), "automatic");
        assert_eq!(validate_ipv4(&manual("10.0.0.2/8", "", &[])), Ok(()));
        assert_eq!(
            validate_ipv4(&manual("10.0.0.2/31", "10.0.0.3", &[])),
            Ok(()),
            "point-to-point /31"
        );
        for (a, why) in [
            ("192.168.1.20", IpProblem::Address),
            ("192.168.1.20/33", IpProblem::Address),
            ("192.168.1.20/0", IpProblem::Address),
            ("192.168.01.20/24", IpProblem::Address),
            ("300.1.1.1/24", IpProblem::Address),
            ("192.168.1.0/24", IpProblem::Reserved),
            ("192.168.1.255/24", IpProblem::Reserved),
        ] {
            assert_eq!(
                validate_ipv4(&manual(a, "192.168.1.1", &[])),
                Err(why),
                "{a}"
            );
        }
        for (r, why) in [
            ("10.0.0.1", IpProblem::RouterOutside),
            ("192.168.1.20", IpProblem::RouterOutside),
            ("192.168.1.255", IpProblem::RouterOutside),
            ("router", IpProblem::Router),
        ] {
            assert_eq!(
                validate_ipv4(&manual("192.168.1.20/24", r, &[])),
                Err(why),
                "{r}"
            );
        }
        assert_eq!(
            validate_ipv4(&manual("192.168.1.20/24", "192.168.1.1", &["dns.example"])),
            Err(IpProblem::Dns)
        );
        assert_eq!(
            parse_dns_list("1.1.1.1, 9.9.9.9 2606:4700:4700::1111,1.1.1.1"),
            Some(vec![
                "1.1.1.1".into(),
                "9.9.9.9".into(),
                "2606:4700:4700::1111".into()
            ])
        );
        assert_eq!(parse_dns_list("1.1.1.1, quad9"), None);
        assert_eq!(parse_dns_list(""), Some(vec![]));
        for p in [
            IpProblem::Address,
            IpProblem::Reserved,
            IpProblem::Router,
            IpProblem::RouterOutside,
            IpProblem::Dns,
        ] {
            assert!(p.sentence().ends_with('.'));
        }
    }

    #[test]
    fn ipv4_profile_read_and_write() {
        let t = "ipv4.method:manual\nipv4.addresses:192.168.1.20/24\nipv4.gateway:192.168.1.1\nipv4.dns:1.1.1.1,9.9.9.9\n";
        assert_eq!(
            parse_ipv4(t),
            manual("192.168.1.20/24", "192.168.1.1", &["1.1.1.1", "9.9.9.9"])
        );
        let dhcp = parse_ipv4("ipv4.method:auto\nipv4.addresses:\nipv4.gateway:--\nipv4.dns:\n");
        assert_eq!(dhcp, Ipv4Config::default());
        let u = "6a3f0c2e-1111-4a5b-9c8d-0123456789ab";
        assert!(is_uuid(u) && !is_uuid("x; rm") && !is_uuid(""));
        assert_eq!(
            ipv4_modify_argv(
                u,
                &manual("10.0.0.5/24", "10.0.0.1", &["1.1.1.1", "9.9.9.9"])
            )
            .join(" "),
            format!(
                "nmcli connection modify uuid {u} ipv4.method manual ipv4.addresses 10.0.0.5/24 ipv4.gateway 10.0.0.1 ipv4.dns 1.1.1.1,9.9.9.9"
            )
        );
        let auto = ipv4_modify_argv(u, &Ipv4Config::default());
        assert_eq!(&auto[5..7], ["ipv4.method", "auto"]);
        assert_eq!(auto.len(), 13, "addresses, router and DNS cleared");
        assert!(device_connected("100 (connected)\n") && !device_connected("70 (connecting)"));
    }

    #[test]
    fn proxy_args_round_trip_and_files() {
        let m = ProxyConfig {
            mode: ProxyMode::Manual,
            http: "proxy.example:3128".into(),
            https: "proxy.example:3128".into(),
            socks: String::new(),
            no_proxy: vec![".corp.example".into(), "10.0.0.0/8".into()],
            ..Default::default()
        };
        let a = proxy_args(&m);
        assert_eq!(
            a,
            [
                "manual",
                "proxy.example:3128",
                "proxy.example:3128",
                "-",
                ".corp.example,10.0.0.0/8"
            ]
        );
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        assert_eq!(proxy_from_args(&a), Some(m.clone()));
        assert_eq!(parse_proxy_conf(&render_proxy_conf(&m)), m);
        let p = render_proxy_profile(&m);
        assert!(p.contains("export http_proxy='http://proxy.example:3128/' HTTP_PROXY='http://proxy.example:3128/'"), "{p}");
        assert!(!p.contains("all_proxy"), "no SOCKS server, no variable");
        assert!(p.contains("no_proxy='localhost,127.0.0.1,::1,.corp.example,10.0.0.0/8'"));
        let auto = ProxyConfig {
            mode: ProxyMode::Auto,
            pac_url: "http://wpad.example/proxy.pac".into(),
            ..Default::default()
        };
        assert_eq!(
            proxy_args(&auto),
            ["auto", "http://wpad.example/proxy.pac", "-"]
        );
        assert_eq!(parse_proxy_conf(&render_proxy_conf(&auto)), auto);
        assert!(!render_proxy_profile(&auto).contains("export"));
        assert_eq!(proxy_from_args(&["off"]), Some(ProxyConfig::default()));
        for bad in [
            &["manual", "-", "-", "-", "-"][..],
            &["manual", "proxy:0", "-", "-", "-"],
            &["manual", "pro xy:80", "-", "-", "-"],
            &["manual", "proxy:80", "-", "-", "a'b"],
            &["auto", "ftp://x/p.pac", "-"],
            &["auto", "http://x/'$(id)'", "-"],
            &["auto"],
            &["sometimes"],
        ] {
            assert_eq!(proxy_from_args(bad), None, "{bad:?}");
        }
        assert!(is_host_port("[2001:db8::1]:3128") && is_host_port("10.0.0.1:8080"));
        assert_eq!(parse_proxy_conf("mode=manual\n"), ProxyConfig::default());
    }

    /// QA-A9-SEC-10: a PAC address with credentials is refused (they would be written to
    /// proxy.conf and logged); an `@` further on in the path is fine.
    #[test]
    fn a_pac_address_never_carries_credentials() {
        assert!(is_pac_url("https://wpad.example/proxy.pac"));
        assert!(is_pac_url("http://wpad.example/a@b.pac"));
        assert!(is_pac_url("http://wpad.example/p.pac?u=a@b"));
        assert!(!is_pac_url("https://user:secret@wpad.example/proxy.pac"));
        assert!(!is_pac_url("http://user@wpad.example/proxy.pac"));
        assert!(!is_pac_url("http://@wpad.example/proxy.pac"));
        assert!(proxy_from_args(&["auto", "https://u:p@wpad.example/x.pac", "-"]).is_none());
    }

    #[test]
    fn proxy_desktop_keys_and_nm() {
        let m = ProxyConfig {
            mode: ProxyMode::Manual,
            http: "proxy.example:3128".into(),
            no_proxy: vec!["localhost".into()],
            ..Default::default()
        };
        let g: Vec<String> = gsettings_argvs(&m).iter().map(|a| a.join(" ")).collect();
        assert_eq!(g[0], "gsettings set org.gnome.system.proxy mode manual");
        assert!(
            g.contains(&"gsettings set org.gnome.system.proxy ignore-hosts ['localhost']".into())
        );
        assert!(g.contains(&"gsettings set org.gnome.system.proxy.http host proxy.example".into()));
        assert!(g.contains(&"gsettings set org.gnome.system.proxy.http port 3128".into()));
        assert!(g.contains(&"gsettings set org.gnome.system.proxy.socks port 0".into()));
        let u = "6a3f0c2e-1111-4a5b-9c8d-0123456789ab";
        assert_eq!(
            nm_proxy_argv(u, &m)[5..9],
            ["proxy.method", "none", "proxy.pac-url", ""]
        );
        let auto = ProxyConfig {
            mode: ProxyMode::Auto,
            pac_url: "https://wpad/p.pac".into(),
            ..Default::default()
        };
        assert_eq!(
            nm_proxy_argv(u, &auto)[5..9],
            [
                "proxy.method",
                "auto",
                "proxy.pac-url",
                "https://wpad/p.pac"
            ]
        );
    }

    /// NET-21 DoT: the drop-in resolved reads, and Strict refused without servers of its own.
    #[test]
    fn dns_over_tls_dropin() {
        let c = DotConfig {
            mode: DotMode::Opportunistic,
            servers: vec!["9.9.9.9#dns.quad9.net".into(), "2620:fe::fe".into()],
        };
        let t = render_dot(&c);
        assert!(
            t.contains(
                "[Resolve]\nDNSOverTLS=opportunistic\nDNS=9.9.9.9#dns.quad9.net 2620:fe::fe\n"
            ),
            "{t}"
        );
        assert_eq!(parse_dot(&t), c);
        assert_eq!(parse_dot(""), DotConfig::default());
        assert!(render_dot(&DotConfig::default()).contains("DNSOverTLS=no\n"));
        assert!(!render_dot(&DotConfig::default()).contains("DNS="));
        let strict = DotConfig {
            mode: DotMode::Strict,
            servers: vec![],
        };
        assert!(dot_problem(&strict).is_some());
        assert_eq!(dot_from_args(&["strict"]), None);
        assert_eq!(
            dot_from_args(&["strict", "1.1.1.1#cloudflare-dns.com"]).map(|c| render_dot(&c)),
            Some(
                concat!(
                    "# Settings > Network > DNS over TLS (wol-admind dns-over-tls). Do not edit.\n",
                    "[Resolve]\nDNSOverTLS=yes\nDNS=1.1.1.1#cloudflare-dns.com\n"
                )
                .into()
            )
        );
        assert_eq!(dot_from_args(&["off"]), Some(DotConfig::default()));
        assert_eq!(dot_from_args(&["off", "dns.quad9.net"]), None);
        assert_eq!(dot_from_args(&["off", "9.9.9.9#bad name"]), None);
        assert_eq!(dot_from_args(&["maybe"]), None);
        assert_eq!(dot_from_args(&[]), None);
        assert_eq!(
            dot_args(&c),
            ["opportunistic", "9.9.9.9#dns.quad9.net", "2620:fe::fe"]
        );
    }

    #[test]
    fn rfkill_and_airplane_state() {
        let j = concat!(
            r#"{"rfkilldevices": [{"type": "bluetooth", "soft": "unblocked"}, {"type": "wlan", "soft": "unblocked"}, "#,
            r#"{"type": "wwan", "soft": "blocked"}, {"type": "wlan", "soft": "unblocked"}]}"#
        );
        let r = parse_rfkill(j).unwrap();
        assert_eq!(r.len(), 4);
        assert_eq!(radios_on(&r), ["bluetooth", "wlan"], "wwan was off already");
        assert_eq!(parse_rfkill("junk"), None);
        assert_eq!(
            parse_airplane_file("wlan\nbluetooth\n; rm\n"),
            ["wlan", "bluetooth"]
        );
        let all_off: Vec<Radio> = r
            .iter()
            .map(|x| Radio {
                blocked: true,
                ..x.clone()
            })
            .collect();
        assert!(airplane_on(Some("wlan\n"), &all_off));
        assert!(
            !airplane_on(None, &all_off),
            "blocked from a terminal: not ours"
        );
        let none_off: Vec<Radio> = r
            .iter()
            .map(|x| Radio {
                blocked: false,
                ..x.clone()
            })
            .collect();
        assert!(
            !airplane_on(Some("wlan\n"), &none_off),
            "unblocked from a terminal"
        );
    }
}
