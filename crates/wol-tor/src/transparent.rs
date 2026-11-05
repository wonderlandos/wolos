// SPDX-License-Identifier: GPL-2.0-or-later
//! The transparent Tor layer (ADR 0056): nftables rulesets and namespace set-up that send an
//! app's (or the whole machine's) traffic through tor, with the kernel as the kill switch.
//!
//! A ruleset is data ([`Ruleset`]), not text: [`Ruleset::render`] writes the `nft` file and
//! [`Ruleset::verdict`] runs a packet through the very same rules, so the tests that say "no
//! packet of a non-tor user reaches the network except through tor" test what is loaded, not a
//! copy of it. This crate never runs `nft` or `ip` itself: it makes the text and the argv, and
//! the privileged daemon (`wol-admind`) runs them, because they need `CAP_NET_ADMIN`.
//!
//! Two shapes:
//! - **system** ([`system_ruleset`]): all traffic of the machine except tor's own is redirected on
//!   the output hook; the filter chain's policy is `drop`, so with the redirect rules gone nothing
//!   leaves either.
//! - **app** ([`app_host_ruleset`], [`app_netns_ruleset`], [`netns_steps`]): the app runs in its own
//!   network namespace whose only way out is a veth into the `wol-tor0` bridge; the host redirects
//!   what arrives there to tor and drops the rest, and the namespace's own output chain drops
//!   everything that is not for the gateway (two independent layers).
//!
//! Limits, stated: IPv4 TCP is redirected (tor's `TransPort`); every UDP packet except DNS, every
//! ICMP and all IPv6 to the internet is dropped (Tor carries TCP only); `SO_ORIGINAL_DST` is read
//! by tor itself, so no code here needs `unsafe`.
use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;

/// The bridge that joins the app namespaces and carries the gateway address.
pub const BRIDGE: &str = "wol-tor0";
/// The gateway's address on the bridge: tor's `TransPort` and `DNSPort` listen here.
pub const GATEWAY: Ipv4Addr = Ipv4Addr::new(10, 191, 0, 1);
/// tor's `TransPort`.
pub const TRANS_PORT: u16 = 9040;
/// tor's `DNSPort`.
pub const DNS_PORT: u16 = 5353;
/// The nft table of the system rules (family `inet`).
pub const SYSTEM_TABLE: &str = "wol_tor";
/// The nft table of the app gateway rules on the host (family `inet`).
pub const APP_TABLE: &str = "wol_tor_apps";
/// Highest app index (one /24 bridge: addresses .2 to .201).
pub const MAX_APPS: u8 = 200;

/// IP protocol of a packet or a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proto {
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
    /// ICMP or ICMPv6.
    Icmp,
}

impl Proto {
    fn nft(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
            Self::Icmp => "icmp",
        }
    }
}

/// An address range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cidr {
    /// Network address.
    pub addr: IpAddr,
    /// Prefix length.
    pub prefix: u8,
}

impl Cidr {
    /// Parses `192.168.0.0/16` or `fe80::/10`.
    pub fn parse(s: &str) -> Option<Self> {
        let (a, p) = s.split_once('/')?;
        let addr: IpAddr = a.parse().ok()?;
        let prefix: u8 = p.parse().ok()?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        (prefix <= max).then_some(Self { addr, prefix })
    }

    /// Whether `ip` is inside the range (an address of the other family never is).
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(n), IpAddr::V4(i)) => {
                let m = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(n) & m == u32::from(i) & m
            }
            (IpAddr::V6(n), IpAddr::V6(i)) => {
                let m = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(n) & m == u128::from(i) & m
            }
            _ => false,
        }
    }

    fn text(&self) -> String {
        format!("{}/{}", self.addr, self.prefix)
    }
}

/// tor's `VirtualAddrNetworkIPv4`: the addresses the DNSPort hands out for `.onion` names.
pub fn virtual_range() -> Cidr {
    Cidr {
        addr: IpAddr::V4(Ipv4Addr::new(10, 192, 0, 0)),
        prefix: 10,
    }
}

/// The private ranges a person may allow to stay direct (printers, the router page, NAS).
pub fn lan_ranges() -> Vec<Cidr> {
    [
        concat!("10.0", ".0.0/8"),
        "172.16.0.0/12",
        "192.168.0.0/16",
        "169.254.0.0/16",
        "fc00::/7",
        "fe80::/10",
    ]
    .iter()
    .filter_map(|s| Cidr::parse(s))
    .collect()
}

/// One condition of a rule; all conditions of a rule must hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Match {
    /// The packet is IPv4.
    Ipv4,
    /// The packet is IPv6.
    Ipv6,
    /// Sent by this user id (output hook only).
    Uid(u32),
    /// Arrived on this interface.
    Iif(String),
    /// Leaves on this interface (after the redirect: `lo` for a redirected packet).
    Oif(String),
    /// Protocol and destination port.
    Port(Proto, u16),
    /// Protocol.
    Proto(Proto),
    /// Destination in one of these ranges (all of one family).
    Dst(Vec<Cidr>),
    /// Destination is exactly this address.
    DstIs(Ipv4Addr),
}

/// What a rule does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Let the packet through this hook.
    Accept,
    /// Stop this chain and apply its policy.
    Return,
    /// Discard.
    Drop,
    /// Redirect to this port on the machine.
    Redirect(u16),
}

/// A rule with the reason it exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Conditions.
    pub when: Vec<Match>,
    /// Action.
    pub then: Action,
    /// Why (becomes the nft `comment`).
    pub why: &'static str,
}

/// Where a chain is attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hook {
    /// Locally generated packets.
    Output,
    /// Arriving packets, before routing.
    Prerouting,
    /// Arriving packets for this machine.
    Input,
    /// Packets being routed through this machine.
    Forward,
}

impl Hook {
    fn nft(self) -> &'static str {
        match self {
            Self::Output => "output",
            Self::Prerouting => "prerouting",
            Self::Input => "input",
            Self::Forward => "forward",
        }
    }
}

/// A base chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chain {
    /// Name.
    pub name: &'static str,
    /// Hook.
    pub hook: Hook,
    /// nat (redirect) or filter chain.
    pub nat: bool,
    /// Priority.
    pub priority: i32,
    /// Verdict when no rule decides: `true` = drop.
    pub drop_by_default: bool,
    /// Rules, first match wins.
    pub rules: Vec<Rule>,
}

/// A packet as the rules see it.
#[derive(Clone, Debug)]
pub struct Packet {
    /// Sending user (local packets).
    pub uid: Option<u32>,
    /// Arrival interface (arriving packets).
    pub iif: Option<String>,
    /// Destination.
    pub dst: IpAddr,
    /// Protocol.
    pub proto: Proto,
    /// Destination port.
    pub dport: u16,
    /// Whether the packet is generated on this machine.
    pub local: bool,
}

/// What happens to a packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Leaves or arrives untouched.
    Accept,
    /// Delivered to this machine's port (tor's) instead.
    Redirected(u16),
    /// Discarded.
    Dropped,
}

/// A table of chains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ruleset {
    /// Table name (family `inet`).
    pub table: &'static str,
    /// Chains.
    pub chains: Vec<Chain>,
}

fn render_match(m: &Match) -> String {
    match m {
        Match::Ipv4 => "meta nfproto ipv4".into(),
        Match::Ipv6 => "meta nfproto ipv6".into(),
        Match::Uid(u) => format!("meta skuid {u}"),
        Match::Iif(i) => format!("iifname \"{i}\""),
        Match::Oif(i) => format!("oifname \"{i}\""),
        Match::Port(p, n) => format!("meta l4proto {} {} dport {n}", p.nft(), p.nft()),
        Match::Proto(Proto::Icmp) => "meta l4proto { icmp, ipv6-icmp }".into(),
        Match::Proto(p) => format!("meta l4proto {}", p.nft()),
        Match::Dst(r) => {
            let v6 = r.first().is_some_and(|c| c.addr.is_ipv6());
            let list: Vec<String> = r.iter().map(Cidr::text).collect();
            format!(
                "{} daddr {{ {} }}",
                if v6 { "ip6" } else { "ip" },
                list.join(", ")
            )
        }
        Match::DstIs(a) => format!("ip daddr {a}"),
    }
}

fn match_holds(m: &Match, p: &Packet) -> bool {
    match m {
        Match::Ipv4 => p.dst.is_ipv4(),
        Match::Ipv6 => p.dst.is_ipv6(),
        Match::Uid(u) => p.uid == Some(*u),
        Match::Iif(i) => p.iif.as_deref() == Some(i.as_str()),
        // After a redirect the packet is for this machine, so it leaves on lo.
        Match::Oif(i) => i == "lo" && p.dst.is_loopback(),
        Match::Port(pr, n) => p.proto == *pr && p.dport == *n,
        Match::Proto(pr) => p.proto == *pr,
        Match::Dst(r) => r.iter().any(|c| c.contains(p.dst)),
        Match::DstIs(a) => p.dst == IpAddr::V4(*a),
    }
}

impl Ruleset {
    /// The `nft -f` text. It starts by deleting this table, so loading it twice replaces it
    /// rather than stacking (the delete is preceded by an `add` so it cannot fail on a clean boot).
    pub fn render(&self) -> String {
        let mut o = String::new();
        let t = self.table;
        let _ = write!(
            o,
            "# Generated by wol-tor (ADR 0056). Do not edit.\nadd table inet {t}\ndelete table inet {t}\n\ntable inet {t} {{\n"
        );
        for c in &self.chains {
            let kind = if c.nat { "nat" } else { "filter" };
            let policy = if c.drop_by_default { "drop" } else { "accept" };
            let _ = writeln!(
                o,
                "  chain {} {{\n    type {kind} hook {} priority {}; policy {policy};",
                c.name,
                c.hook.nft(),
                c.priority
            );
            for r in &c.rules {
                let mut line: Vec<String> = r.when.iter().map(render_match).collect();
                line.push(match &r.then {
                    Action::Accept => "accept".into(),
                    Action::Return => "return".into(),
                    Action::Drop => "drop".into(),
                    Action::Redirect(p) => format!("redirect to :{p}"),
                });
                let _ = writeln!(o, "    {} comment \"{}\"", line.join(" "), r.why);
            }
            o.push_str("  }\n");
        }
        o.push_str("}\n");
        o
    }

    /// Runs a packet through the chains of its path, in hook and priority order. A redirect
    /// rewrites the packet's destination for the chains after it (loopback for a local packet, the
    /// gateway address for one arriving on the bridge), as netfilter does.
    pub fn verdict(&self, p: &Packet) -> Verdict {
        let first = if p.local {
            Hook::Output
        } else {
            Hook::Prerouting
        };
        let mut eff = p.clone();
        let mut redirected = None;
        let mut chains: Vec<&Chain> = self.chains.iter().filter(|c| c.hook == first).collect();
        chains.sort_by_key(|c| c.priority);
        for c in chains {
            match self.run(c, &eff) {
                Step::Drop => return Verdict::Dropped,
                Step::Redirect(port) => {
                    redirected = Some(port);
                    eff.dport = port;
                    eff.dst = if c.hook == Hook::Output {
                        IpAddr::V4(Ipv4Addr::LOCALHOST)
                    } else {
                        IpAddr::V4(GATEWAY)
                    };
                }
                Step::Pass => {}
            }
        }
        // An arriving packet is then input (redirected: for this machine) or forward.
        if !p.local {
            let local = redirected.is_some() || eff.dst == IpAddr::V4(GATEWAY);
            let next = if local { Hook::Input } else { Hook::Forward };
            for c in self.chains.iter().filter(|c| c.hook == next) {
                if let Step::Drop = self.run(c, &eff) {
                    return Verdict::Dropped;
                }
            }
        }
        redirected.map_or(Verdict::Accept, Verdict::Redirected)
    }

    fn run(&self, c: &Chain, p: &Packet) -> Step {
        for r in &c.rules {
            if r.when.iter().all(|m| match_holds(m, p)) {
                return match &r.then {
                    Action::Accept => Step::Pass,
                    Action::Return => break,
                    Action::Drop => Step::Drop,
                    Action::Redirect(port) => Step::Redirect(*port),
                };
            }
        }
        if c.drop_by_default {
            Step::Drop
        } else {
            Step::Pass
        }
    }
}

enum Step {
    Pass,
    Drop,
    Redirect(u16),
}

fn rule(when: Vec<Match>, then: Action, why: &'static str) -> Rule {
    Rule { when, then, why }
}

fn split_lan(lan: &[Cidr]) -> (Vec<Cidr>, Vec<Cidr>) {
    lan.iter().copied().partition(|c| c.addr.is_ipv4())
}

/// What the system rules need to know.
#[derive(Clone, Debug)]
pub struct SystemGateway {
    /// The user id tor runs as (its own traffic is the only traffic that may leave).
    pub tor_uid: u32,
    /// Ranges that stay direct (empty by default: everything goes through tor).
    pub lan: Vec<Cidr>,
}

/// The rules of "Tor for the whole machine".
pub fn system_ruleset(g: &SystemGateway) -> Ruleset {
    let (lan4, lan6) = split_lan(&g.lan);
    let mut nat = vec![
        rule(
            vec![Match::Uid(g.tor_uid)],
            Action::Return,
            "tor itself talks to the network",
        ),
        rule(
            vec![Match::Ipv4, Match::Port(Proto::Udp, 53)],
            Action::Redirect(DNS_PORT),
            "DNS to tor's DNSPort, no clear-net DNS",
        ),
        rule(
            vec![Match::Ipv4, Match::Port(Proto::Tcp, 53)],
            Action::Redirect(DNS_PORT),
            "TCP DNS likewise",
        ),
    ];
    let mut filter = vec![
        rule(vec![Match::Uid(g.tor_uid)], Action::Accept, "tor itself"),
        rule(
            vec![Match::Oif("lo".into())],
            Action::Accept,
            "loopback and redirected packets",
        ),
    ];
    // Tor's virtual addresses (what the DNSPort hands out for .onion names) are not a LAN even
    // when the LAN list contains the whole of 10/8: they must reach the TransPort.
    nat.push(rule(
        vec![
            Match::Ipv4,
            Match::Proto(Proto::Tcp),
            Match::Dst(vec![virtual_range()]),
        ],
        Action::Redirect(TRANS_PORT),
        "tor's virtual addresses (.onion) go to the TransPort, never direct",
    ));
    if !lan4.is_empty() {
        nat.push(rule(
            vec![Match::Ipv4, Match::Dst(lan4.clone())],
            Action::Return,
            "allowed LAN stays direct",
        ));
        filter.push(rule(
            vec![Match::Ipv4, Match::Dst(lan4)],
            Action::Accept,
            "allowed LAN",
        ));
    }
    if !lan6.is_empty() {
        filter.push(rule(
            vec![Match::Ipv6, Match::Dst(lan6)],
            Action::Accept,
            "allowed LAN (IPv6)",
        ));
    }
    nat.push(rule(
        vec![Match::Oif("lo".into())],
        Action::Return,
        "do not loop local traffic",
    ));
    nat.push(rule(
        vec![Match::Ipv4, Match::Proto(Proto::Tcp)],
        Action::Redirect(TRANS_PORT),
        "TCP to tor's TransPort",
    ));
    Ruleset {
        table: SYSTEM_TABLE,
        chains: vec![
            Chain {
                name: "to_tor",
                hook: Hook::Output,
                nat: true,
                priority: -100,
                drop_by_default: false,
                rules: nat,
            },
            Chain {
                name: "killswitch",
                hook: Hook::Output,
                nat: false,
                priority: 0,
                drop_by_default: true,
                rules: filter,
            },
        ],
    }
}

/// The host side of the app gateway (traffic arriving from the app namespaces on [`BRIDGE`]).
pub fn app_host_ruleset() -> Ruleset {
    let br = || Match::Iif(BRIDGE.into());
    Ruleset {
        table: APP_TABLE,
        chains: vec![
            Chain {
                name: "to_tor",
                hook: Hook::Prerouting,
                nat: true,
                priority: -100,
                drop_by_default: false,
                rules: vec![
                    rule(
                        vec![br(), Match::Port(Proto::Udp, 53)],
                        Action::Redirect(DNS_PORT),
                        "app DNS to the DNSPort",
                    ),
                    rule(
                        vec![br(), Match::Port(Proto::Tcp, 53)],
                        Action::Redirect(DNS_PORT),
                        "app TCP DNS likewise",
                    ),
                    rule(
                        vec![br(), Match::Ipv4, Match::Proto(Proto::Tcp)],
                        Action::Redirect(TRANS_PORT),
                        "app TCP to the TransPort",
                    ),
                ],
            },
            Chain {
                name: "gate",
                hook: Hook::Input,
                nat: false,
                priority: -10,
                drop_by_default: false,
                rules: vec![
                    rule(
                        vec![
                            br(),
                            Match::DstIs(GATEWAY),
                            Match::Port(Proto::Tcp, TRANS_PORT),
                        ],
                        Action::Accept,
                        "the TransPort",
                    ),
                    rule(
                        vec![
                            br(),
                            Match::DstIs(GATEWAY),
                            Match::Port(Proto::Udp, DNS_PORT),
                        ],
                        Action::Accept,
                        "the DNSPort (UDP)",
                    ),
                    rule(
                        vec![
                            br(),
                            Match::DstIs(GATEWAY),
                            Match::Port(Proto::Tcp, DNS_PORT),
                        ],
                        Action::Accept,
                        "the DNSPort (TCP)",
                    ),
                    rule(
                        vec![br()],
                        Action::Drop,
                        "an app talks to tor and to nothing else on the host",
                    ),
                ],
            },
            Chain {
                name: "noforward",
                hook: Hook::Forward,
                nat: false,
                priority: -10,
                drop_by_default: false,
                rules: vec![rule(
                    vec![br()],
                    Action::Drop,
                    "never routed, even if ip_forward is on",
                )],
            },
        ],
    }
}

/// The rules inside an app's namespace: the second layer, independent of the host's.
pub fn app_netns_ruleset() -> Ruleset {
    Ruleset {
        table: "wol_tor_ns",
        chains: vec![Chain {
            name: "killswitch",
            hook: Hook::Output,
            nat: false,
            priority: 0,
            drop_by_default: true,
            rules: vec![
                rule(vec![Match::Oif("lo".into())], Action::Accept, "loopback"),
                rule(
                    vec![Match::DstIs(GATEWAY), Match::Port(Proto::Tcp, TRANS_PORT)],
                    Action::Accept,
                    "TransPort",
                ),
                rule(
                    vec![Match::DstIs(GATEWAY), Match::Port(Proto::Udp, DNS_PORT)],
                    Action::Accept,
                    "DNSPort",
                ),
                rule(
                    vec![Match::Proto(Proto::Tcp)],
                    Action::Accept,
                    "TCP to anywhere; the host redirects it to tor",
                ),
                rule(
                    vec![Match::DstIs(GATEWAY), Match::Port(Proto::Udp, 53)],
                    Action::Accept,
                    "DNS to the gateway",
                ),
            ],
        }],
    }
}

/// An app's namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppNs {
    /// App name (a label: lowercase letters, digits, `-`).
    pub app: String,
    /// 1 to [`MAX_APPS`].
    pub index: u8,
}

/// The app name or index is not usable.
#[derive(Debug, PartialEq, Eq)]
pub struct BadApp(pub String);

impl std::fmt::Display for BadApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for BadApp {}

impl AppNs {
    /// Checks the name and index.
    pub fn new(app: &str, index: u8) -> Result<Self, BadApp> {
        let ok = !app.is_empty()
            && app.len() <= 24
            && !app.starts_with('-')
            && !app.ends_with('-')
            && app
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !ok {
            return Err(BadApp(format!(
                "{app:?} is not an app name (lowercase letters, digits, -; at most 24)"
            )));
        }
        if index == 0 || index > MAX_APPS {
            return Err(BadApp(format!(
                "app index {index} is outside 1 to {MAX_APPS}"
            )));
        }
        Ok(Self {
            app: app.into(),
            index,
        })
    }

    /// `wolt-<app>`.
    pub fn netns(&self) -> String {
        format!("wolt-{}", self.app)
    }

    /// The host end of the veth.
    pub fn veth(&self) -> String {
        format!("wtv{}", self.index)
    }

    /// The app's address on the bridge.
    pub fn addr(&self) -> Ipv4Addr {
        Ipv4Addr::new(10, 191, 0, self.index + 1)
    }

    /// The file `ip netns exec` bind-mounts as the namespace's `/etc/resolv.conf`.
    pub fn resolv_conf_path(&self) -> String {
        format!("/etc/netns/{}/resolv.conf", self.netns())
    }

    /// The file `ip netns exec` bind-mounts as the namespace's `/etc/nsswitch.conf`.
    pub fn nsswitch_path(&self) -> String {
        format!("/etc/netns/{}/nsswitch.conf", self.netns())
    }
}

/// The namespace's `/etc/nsswitch.conf`: names come from `/etc/hosts` and from DNS (the gateway's
/// DNSPort) and from nowhere else. The system file says `resolve [!UNAVAIL=return]`, which would
/// ask systemd-resolved over its unix socket, and a unix socket crosses the network namespace:
/// the lookup would leave the machine through the host's resolver, outside tor. `.onion` names
/// resolve only through the DNSPort (`AutomapHostsOnResolve`), so this is also what makes them work.
pub const NSSWITCH_CONF: &str = "passwd: files systemd\ngroup: files systemd\nshadow: files systemd\n\
hosts: files dns\nnetworks: files\nprotocols: files\nservices: files\nethers: files\nrpc: files\n";

/// The namespace's `/etc/resolv.conf`: the gateway only, TCP-free, short timeouts.
pub fn resolv_conf() -> String {
    format!("nameserver {GATEWAY}\noptions timeout:5 attempts:2\n")
}

/// One privileged step for the daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivStep {
    /// The program and arguments.
    pub argv: Vec<String>,
    /// Text written to its standard input (an nft file).
    pub stdin: Option<String>,
    /// File to write instead of running anything: (path, content).
    pub write_file: Option<(String, String)>,
}

fn run(argv: &[&str]) -> PrivStep {
    PrivStep {
        argv: argv.iter().map(|s| s.to_string()).collect(),
        stdin: None,
        write_file: None,
    }
}

fn nft(text: String) -> PrivStep {
    PrivStep {
        argv: vec!["nft".into(), "-f".into(), "-".into()],
        stdin: Some(text),
        write_file: None,
    }
}

/// The bridge and the host rules, once per boot of the app gateway (idempotent order, kill switch
/// first: the rules that drop are loaded before anything can send).
pub fn gateway_up_steps() -> Vec<PrivStep> {
    let gw = format!("{GATEWAY}/24");
    vec![
        nft(app_host_ruleset().render()),
        run(&["ip", "link", "add", BRIDGE, "type", "bridge"]),
        run(&["ip", "addr", "add", &gw, "dev", BRIDGE]),
        run(&["ip", "link", "set", BRIDGE, "up"]),
        run(&[
            "sysctl",
            "-qw",
            &format!("net.ipv4.conf.{BRIDGE}.forwarding=0"),
        ]),
        run(&[
            "sysctl",
            "-qw",
            &format!("net.ipv6.conf.{BRIDGE}.disable_ipv6=1"),
        ]),
    ]
}

/// Removes the bridge and the host rules.
pub fn gateway_down_steps() -> Vec<PrivStep> {
    vec![
        run(&["ip", "link", "del", BRIDGE]),
        run(&["nft", "delete", "table", "inet", APP_TABLE]),
    ]
}

/// Creates an app's namespace: no route but the gateway, IPv6 off, the namespace's own drop rules
/// loaded before the interface comes up.
pub fn netns_steps(ns: &AppNs) -> Vec<PrivStep> {
    let n = ns.netns();
    let (veth, addr) = (ns.veth(), format!("{}/24", ns.addr()));
    vec![
        run(&["ip", "netns", "add", &n]),
        PrivStep {
            argv: vec![],
            stdin: None,
            write_file: Some((ns.resolv_conf_path(), resolv_conf())),
        },
        PrivStep {
            argv: vec![],
            stdin: None,
            write_file: Some((ns.nsswitch_path(), NSSWITCH_CONF.to_string())),
        },
        run(&[
            "ip",
            "netns",
            "exec",
            &n,
            "sysctl",
            "-qw",
            "net.ipv6.conf.all.disable_ipv6=1",
        ]),
        PrivStep {
            argv: ["ip", "netns", "exec", &n, "nft", "-f", "-"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            stdin: Some(app_netns_ruleset().render()),
            write_file: None,
        },
        run(&[
            "ip", "link", "add", &veth, "type", "veth", "peer", "name", "eth0", "netns", &n,
        ]),
        run(&["ip", "link", "set", &veth, "master", BRIDGE]),
        // Isolated ports cannot talk to each other: one app cannot reach another app's address.
        run(&["bridge", "link", "set", "dev", &veth, "isolated", "on"]),
        run(&["ip", "link", "set", &veth, "up"]),
        run(&["ip", "-n", &n, "addr", "add", &addr, "dev", "eth0"]),
        run(&["ip", "-n", &n, "link", "set", "lo", "up"]),
        run(&["ip", "-n", &n, "link", "set", "eth0", "up"]),
        run(&[
            "ip",
            "-n",
            &n,
            "route",
            "add",
            "default",
            "via",
            &GATEWAY.to_string(),
        ]),
    ]
}

/// Removes an app's namespace and veth.
pub fn netns_down_steps(ns: &AppNs) -> Vec<PrivStep> {
    vec![
        run(&["ip", "link", "del", &ns.veth()]),
        run(&["ip", "netns", "del", &ns.netns()]),
    ]
}

/// Directories of the host resolvers' unix sockets. In the app's private mount namespace they are
/// emptied (tmpfs over them), because a unix socket is not bound by the network namespace and
/// systemd-resolved or avahi would answer from outside tor.
pub const MASK_DIRS: [&str; 2] = ["/run/systemd/resolve", "/run/avahi-daemon"];

/// The `mount` command that empties one of [`MASK_DIRS`] (read-only, no exec, 4 KiB).
pub fn mask_argv(dir: &str) -> Vec<String> {
    [
        "mount",
        "-t",
        "tmpfs",
        "-o",
        "ro,nosuid,nodev,noexec,size=4k",
        "tmpfs",
        dir,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// The command that starts an app inside its namespace as `uid`:`gid`, with no capability and no
/// way to get one. Run by the privileged daemon; the app's systemd scope wraps this.
pub fn join_argv(ns: &AppNs, wol_tor: &Path, uid: u32, gid: u32, cmd: &[String]) -> Vec<String> {
    let mut v: Vec<String> = [
        "ip",
        "netns",
        "exec",
        &ns.netns(),
        &wol_tor.display().to_string(),
        "app-exec",
        "--",
        "setpriv",
        "--reuid",
        &uid.to_string(),
        "--regid",
        &gid.to_string(),
        "--init-groups",
        "--no-new-privs",
        "--inh-caps=-all",
        "--ambient-caps=-all",
        "--bounding-set=-all",
        "--",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    v.extend(cmd.iter().cloned());
    v
}

/// torrc lines for the gateway ports: 127.0.0.1 for the system rules, the bridge address for the
/// app namespaces, both when both are on.
pub fn torrc_lines(system: bool, apps: bool) -> Vec<String> {
    let mut v = Vec::new();
    for (on, listen) in [
        (system, "127.0.0.1".to_string()),
        (apps, GATEWAY.to_string()),
    ] {
        if on {
            v.push(format!(
                "TransPort {listen}:{TRANS_PORT} IsolateClientAddr IsolateClientProtocol"
            ));
            v.push(format!("DNSPort {listen}:{DNS_PORT}"));
        }
    }
    v.push("AutomapHostsOnResolve 1".into());
    v.push("VirtualAddrNetworkIPv4 10.192.0.0/10".into());
    v
}

/// A tor binary is trusted to run as the gateway only if root owns it and neither group nor
/// others can write it (the check on the packaged `/usr/bin/tor`).
pub fn tor_binary_trusted(owner_uid: u32, mode: u32) -> Result<(), String> {
    if owner_uid != 0 {
        return Err(format!(
            "the tor binary is owned by uid {owner_uid}, not root"
        ));
    }
    if mode & 0o022 != 0 {
        return Err(format!(
            "the tor binary is writable by group or others (mode {:o})",
            mode & 0o7777
        ));
    }
    Ok(())
}

/// Whether `path` is the packaged tor.
pub fn is_packaged_tor(path: &Path) -> bool {
    path == Path::new("/usr/bin/tor")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOR: u32 = 105;
    const USER: u32 = 1000;

    fn pk(uid: u32, proto: Proto, dst: &str, dport: u16) -> Packet {
        Packet {
            uid: Some(uid),
            iif: None,
            dst: dst.parse().unwrap(),
            proto,
            dport,
            local: true,
        }
    }

    fn from_app(proto: Proto, dst: &str, dport: u16) -> Packet {
        Packet {
            uid: None,
            iif: Some(BRIDGE.into()),
            dst: dst.parse().unwrap(),
            proto,
            dport,
            local: false,
        }
    }

    fn sys(lan: Vec<Cidr>) -> Ruleset {
        system_ruleset(&SystemGateway { tor_uid: TOR, lan })
    }

    #[test]
    fn system_mode_sends_tcp_and_dns_to_tor() {
        let r = sys(vec![]);
        assert_eq!(
            r.verdict(&pk(USER, Proto::Tcp, "93.184.216.34", 443)),
            Verdict::Redirected(TRANS_PORT)
        );
        assert_eq!(
            r.verdict(&pk(USER, Proto::Udp, "1.1.1.1", 53)),
            Verdict::Redirected(DNS_PORT)
        );
        assert_eq!(
            r.verdict(&pk(USER, Proto::Udp, "127.0.0.53", 53)),
            Verdict::Redirected(DNS_PORT)
        );
        assert_eq!(
            r.verdict(&pk(USER, Proto::Tcp, "1.1.1.1", 853)),
            Verdict::Redirected(TRANS_PORT),
            "DoT rides tor"
        );
    }

    #[test]
    fn system_mode_drops_everything_tor_cannot_carry() {
        let r = sys(vec![]);
        for p in [
            pk(USER, Proto::Udp, "93.184.216.34", 443),
            pk(USER, Proto::Udp, "8.8.8.8", 3478),
            pk(USER, Proto::Icmp, "8.8.8.8", 0),
            pk(USER, Proto::Tcp, "2606:4700::1111", 443),
            pk(USER, Proto::Udp, "2606:4700::1111", 53),
            pk(0, Proto::Tcp, "93.184.216.34", 443),
        ] {
            let v = r.verdict(&p);
            assert!(
                matches!(v, Verdict::Dropped | Verdict::Redirected(_)),
                "{p:?} must not leave directly, got {v:?}"
            );
            if !matches!(p.proto, Proto::Tcp) || p.dst.is_ipv6() {
                assert_eq!(v, Verdict::Dropped, "{p:?}");
            }
        }
    }

    #[test]
    fn only_tor_itself_may_use_the_network() {
        let r = sys(vec![]);
        assert_eq!(
            r.verdict(&pk(TOR, Proto::Tcp, "198.51.100.7", 9001)),
            Verdict::Accept
        );
        assert_eq!(
            r.verdict(&pk(TOR, Proto::Udp, "198.51.100.7", 53)),
            Verdict::Accept
        );
    }

    #[test]
    fn no_packet_of_another_user_leaves_unredirected_for_any_destination() {
        // The kill-switch property over a grid of users, protocols, destinations and ports.
        let r = sys(vec![]);
        let dsts = [
            "1.1.1.1",
            "10.1.2.3",
            "192.168.1.1",
            "169.254.0.1",
            "100.64.0.1",
            "8.8.8.8",
            "2001:db8::1",
            "fe80::1",
            "fc00::1",
        ];
        for uid in [0, 1, USER, 65534] {
            for proto in [Proto::Tcp, Proto::Udp, Proto::Icmp] {
                for d in dsts {
                    for port in [0, 22, 53, 80, 443, 853, 9040, 5353, 51820] {
                        let v = r.verdict(&pk(uid, proto, d, port));
                        assert!(
                            !matches!(v, Verdict::Accept),
                            "uid {uid} {proto:?} {d}:{port} left the machine directly"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn losing_the_redirect_rules_still_leaves_nothing_open() {
        // Fail closed: the filter chain alone (as if the nat chain were flushed or never loaded)
        // drops every non-tor packet to the internet.
        let mut r = sys(vec![]);
        r.chains.retain(|c| !c.nat);
        for (proto, d, port) in [
            (Proto::Tcp, "93.184.216.34", 443),
            (Proto::Udp, "1.1.1.1", 53),
            (Proto::Icmp, "9.9.9.9", 0),
        ] {
            assert_eq!(
                r.verdict(&pk(USER, proto, d, port)),
                Verdict::Dropped,
                "{d}"
            );
        }
    }

    #[test]
    fn lan_stays_direct_only_when_allowed() {
        let off = sys(vec![]);
        let on = sys(lan_ranges());
        let printer = pk(USER, Proto::Tcp, "192.168.1.20", 631);
        assert_eq!(off.verdict(&printer), Verdict::Redirected(TRANS_PORT));
        assert_eq!(on.verdict(&printer), Verdict::Accept);
        assert_eq!(
            on.verdict(&pk(USER, Proto::Tcp, "93.184.216.34", 443)),
            Verdict::Redirected(TRANS_PORT)
        );
        assert_eq!(
            on.verdict(&pk(USER, Proto::Udp, "224.0.0.251", 5353)),
            Verdict::Dropped,
            "mDNS to the world is not LAN"
        );
        assert_eq!(
            on.verdict(&pk(USER, Proto::Udp, "192.168.1.1", 53)),
            Verdict::Redirected(DNS_PORT),
            "even LAN DNS goes to tor"
        );
    }

    #[test]
    fn loopback_is_left_alone() {
        let r = sys(vec![]);
        assert_eq!(
            r.verdict(&pk(USER, Proto::Tcp, "127.0.0.1", 631)),
            Verdict::Accept
        );
        assert_eq!(
            r.verdict(&pk(USER, Proto::Tcp, "::1", 631)),
            Verdict::Accept
        );
    }

    #[test]
    fn app_traffic_is_redirected_and_the_host_is_not_reachable() {
        let r = app_host_ruleset();
        assert_eq!(
            r.verdict(&from_app(Proto::Tcp, "93.184.216.34", 443)),
            Verdict::Redirected(TRANS_PORT)
        );
        assert_eq!(
            r.verdict(&from_app(Proto::Udp, "10.191.0.1", 53)),
            Verdict::Redirected(DNS_PORT)
        );
        assert_eq!(
            r.verdict(&from_app(Proto::Udp, "93.184.216.34", 443)),
            Verdict::Dropped,
            "no QUIC"
        );
        assert_eq!(
            r.verdict(&from_app(Proto::Icmp, "93.184.216.34", 0)),
            Verdict::Dropped
        );
        assert_eq!(
            r.verdict(&from_app(Proto::Tcp, "2606:4700::1111", 443)),
            Verdict::Dropped,
            "no IPv6 route out"
        );
        // Traffic aimed at the host's own services (ssh on the gateway address) is dropped.
        assert_eq!(
            r.verdict(&from_app(Proto::Tcp, "10.191.0.1", 22)),
            Verdict::Redirected(TRANS_PORT),
            "redirected to tor, never to sshd"
        );
    }

    #[test]
    fn app_traffic_is_never_forwarded_even_when_the_redirect_is_gone() {
        let mut r = app_host_ruleset();
        r.chains.retain(|c| !c.nat);
        for (proto, d, port) in [
            (Proto::Tcp, "93.184.216.34", 443),
            (Proto::Udp, "1.1.1.1", 53),
            (Proto::Tcp, "192.168.1.1", 80),
        ] {
            assert_eq!(
                r.verdict(&from_app(proto, d, port)),
                Verdict::Dropped,
                "{d}"
            );
        }
    }

    #[test]
    fn the_namespace_has_its_own_drop_layer() {
        let r = app_netns_ruleset();
        let local = |proto, d: &str, port| pk(USER, proto, d, port);
        assert_eq!(
            r.verdict(&local(Proto::Udp, "93.184.216.34", 443)),
            Verdict::Dropped
        );
        assert_eq!(
            r.verdict(&local(Proto::Icmp, "93.184.216.34", 0)),
            Verdict::Dropped
        );
        assert_eq!(
            r.verdict(&local(Proto::Udp, "10.191.0.1", 53)),
            Verdict::Accept
        );
        assert_eq!(
            r.verdict(&local(Proto::Udp, "8.8.8.8", 53)),
            Verdict::Dropped,
            "only the gateway resolves"
        );
        assert_eq!(
            r.verdict(&local(Proto::Tcp, "93.184.216.34", 443)),
            Verdict::Accept
        );
    }

    #[test]
    fn rendered_text_is_a_table_with_policy_drop_and_redirects() {
        let t = sys(vec![]).render();
        assert!(t.starts_with("# Generated by wol-tor"));
        assert!(t.contains("add table inet wol_tor\ndelete table inet wol_tor\n"));
        assert!(t.contains("type nat hook output priority -100; policy accept;"));
        assert!(t.contains("type filter hook output priority 0; policy drop;"));
        assert!(t.contains("meta skuid 105 return"));
        assert!(t.contains("meta nfproto ipv4 meta l4proto udp udp dport 53 redirect to :5353"));
        assert!(t.contains("redirect to :9040"));
        let lan = sys(lan_ranges()).render();
        assert!(lan.contains("ip daddr { 10.0."));
        assert!(lan.contains("/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16 }"));
        assert!(lan.contains("ip6 daddr { fc00::/7, fe80::/10 }"));
        assert!(!t.contains("allowed LAN"), "no LAN rule unless allowed");
        let a = app_host_ruleset().render();
        assert!(
            a.contains("iifname \"wol-tor0\" meta nfproto ipv4 meta l4proto tcp redirect to :9040")
        );
        assert!(a.contains("type filter hook forward priority -10; policy accept;"));
    }

    #[test]
    fn rendered_rules_load_in_the_kernel_parser_when_nft_is_available() {
        // `nft -c` checks the syntax with the kernel's parser; skipped where it cannot run.
        for (name, text) in [
            ("system", sys(lan_ranges()).render()),
            ("apps", app_host_ruleset().render()),
            ("ns", app_netns_ruleset().render()),
        ] {
            let dir =
                std::env::temp_dir().join(format!("wol-tor-nft-{name}-{}", std::process::id()));
            std::fs::write(&dir, &text).unwrap();
            let nft_check = |sudo: bool| {
                let mut c = std::process::Command::new(if sudo { "sudo" } else { "nft" });
                if sudo {
                    c.args(["-n", "nft"]);
                }
                c.args(["-c", "-f"]).arg(&dir).output()
            };
            let mut out = nft_check(false);
            if out
                .as_ref()
                .is_ok_and(|o| String::from_utf8_lossy(&o.stderr).contains("not permitted"))
            {
                // `nft -c` needs netlink access; use passwordless sudo where the machine has it.
                out = nft_check(true);
            }
            let _ = std::fs::remove_file(&dir);
            let Ok(out) = out else { return };
            let err = String::from_utf8_lossy(&out.stderr).to_string();
            if err.contains("not permitted")
                || err.contains("password")
                || err.contains("not found")
            {
                return; // no way to run the parser here
            }
            assert!(out.status.success(), "{name}: {err}\n{text}");
        }
    }

    #[test]
    fn app_namespaces_have_checked_names_and_distinct_addresses() {
        let a = AppNs::new("discord", 1).unwrap();
        let b = AppNs::new("signal-desktop", 2).unwrap();
        assert_eq!(a.addr(), Ipv4Addr::new(10, 191, 0, 2));
        assert_ne!(
            a.addr(),
            b.addr(),
            "IsolateClientAddr gives each app its own circuits"
        );
        assert_eq!(a.netns(), "wolt-discord");
        assert_eq!(a.veth(), "wtv1");
        for bad in ["", "A", "a b", "a/b", "-a", "a-", &"x".repeat(25), "a;rm"] {
            assert!(AppNs::new(bad, 1).is_err(), "{bad:?}");
        }
        assert!(AppNs::new("a", 0).is_err() && AppNs::new("a", 201).is_err());
    }

    #[test]
    fn netns_setup_loads_the_drop_rules_before_the_interface_exists() {
        let steps = netns_steps(&AppNs::new("discord", 1).unwrap());
        let pos = |f: &dyn Fn(&PrivStep) -> bool| steps.iter().position(f).unwrap();
        let rules = pos(&|s| {
            s.stdin
                .as_deref()
                .is_some_and(|t| t.contains("policy drop"))
        });
        let veth = pos(&|s| s.argv.contains(&"veth".to_string()));
        let up = pos(&|s| {
            s.argv.windows(2).any(|w| w == ["set", "eth0"])
                || s.argv.contains(&"eth0".to_string()) && s.argv.contains(&"up".to_string())
        });
        assert!(rules < veth && veth < up, "kill switch first");
        let routes: Vec<_> = steps
            .iter()
            .filter(|s| s.argv.contains(&"route".to_string()))
            .collect();
        assert_eq!(routes.len(), 1, "one route, the default via the gateway");
        assert!(
            routes[0]
                .argv
                .ends_with(&["default".into(), "via".into(), "10.191.0.1".into()])
        );
        assert!(
            steps
                .iter()
                .any(|s| s.argv.iter().any(|a| a.contains("disable_ipv6=1")))
        );
        let rc = steps.iter().find_map(|s| s.write_file.clone()).unwrap();
        assert_eq!(
            rc,
            ("/etc/netns/wolt-discord/resolv.conf".into(), resolv_conf())
        );
    }

    #[test]
    fn with_a_lan_allowed_tors_virtual_addresses_still_go_to_the_transport() {
        // 10/8 is in the LAN list, and tor's virtual range 10.192.0.0/10 is inside 10/8.
        let r = sys(lan_ranges());
        assert_eq!(
            r.verdict(&pk(USER, Proto::Tcp, "10.193.4.5", 80)),
            Verdict::Redirected(TRANS_PORT)
        );
        assert_eq!(
            r.verdict(&pk(USER, Proto::Tcp, "10.191.255.1", 80)),
            Verdict::Accept,
            "just below the virtual range is an ordinary LAN address"
        );
        // The bridge (10.191.0.0/24) is outside the virtual range, so no name can map onto it.
        assert!(!virtual_range().contains(IpAddr::V4(GATEWAY)));
        assert!(virtual_range().contains("10.255.255.254".parse().unwrap()));
    }

    #[test]
    fn the_gate_accepts_the_dnsport_only_at_the_gateway_address() {
        let r = app_host_ruleset();
        let to = |dst: &str, proto, port| from_app(proto, dst, port);
        // Aimed straight at a port of the host's other addresses: dropped, not accepted.
        assert_eq!(
            r.verdict(&to("192.168.1.5", Proto::Udp, DNS_PORT)),
            Verdict::Dropped
        );
        assert_eq!(
            r.verdict(&to("10.191.0.1", Proto::Udp, 5000)),
            Verdict::Dropped
        );
        assert_eq!(
            r.verdict(&to("10.191.0.1", Proto::Udp, DNS_PORT)),
            Verdict::Accept
        );
    }

    #[test]
    fn an_app_resolves_names_through_the_dnsport_and_never_through_resolved() {
        let steps = netns_steps(&AppNs::new("discord", 1).unwrap());
        let file = |name: &str| {
            steps
                .iter()
                .filter_map(|s| s.write_file.clone())
                .find(|(p, _)| p == name)
                .map(|(_, t)| t)
                .unwrap_or_else(|| panic!("{name} is written"))
        };
        let ns = file("/etc/netns/wolt-discord/nsswitch.conf");
        let hosts: Vec<&str> = ns.lines().filter(|l| l.starts_with("hosts:")).collect();
        assert_eq!(
            hosts,
            ["hosts: files dns"],
            "no resolve, mdns or myhostname"
        );
        assert!(!ns.contains("resolve") && !ns.contains("mdns") && !ns.contains("myhostname"));
        assert!(
            ns.contains("passwd: files systemd"),
            "complete file: it replaces the system one"
        );
        let rc = file("/etc/netns/wolt-discord/resolv.conf");
        assert_eq!(
            rc.lines()
                .filter(|l| l.starts_with("nameserver"))
                .collect::<Vec<_>>(),
            ["nameserver 10.191.0.1"]
        );
        assert!(!rc.contains("search") && !rc.contains("127.0.0.53"));
        // The resolver sockets of the host are masked in the app's mount namespace.
        let j = join_argv(
            &AppNs::new("discord", 1).unwrap(),
            Path::new("/usr/lib/wol-tor"),
            1000,
            1000,
            &["/bin/true".into()],
        );
        assert_eq!(
            j[4..7],
            ["/usr/lib/wol-tor", "app-exec", "--"],
            "the mask step runs inside the namespace, before the app"
        );
        assert_eq!(MASK_DIRS, ["/run/systemd/resolve", "/run/avahi-daemon"]);
        assert_eq!(mask_argv(MASK_DIRS[0])[..4], ["mount", "-t", "tmpfs", "-o"]);
        assert!(mask_argv(MASK_DIRS[0]).last().unwrap() == "/run/systemd/resolve");
        // and the path .onion takes is the DNSPort: the name goes to the gateway's UDP 53, which the
        // host redirects to tor.
        assert_eq!(
            app_host_ruleset().verdict(&from_app(Proto::Udp, "10.191.0.1", 53)),
            Verdict::Redirected(DNS_PORT)
        );
    }

    #[test]
    fn app_ports_are_isolated_from_each_other() {
        let steps = netns_steps(&AppNs::new("discord", 3).unwrap());
        assert!(
            steps
                .iter()
                .any(|s| s.argv == ["bridge", "link", "set", "dev", "wtv3", "isolated", "on"])
        );
        let master = steps
            .iter()
            .position(|s| s.argv.contains(&"master".to_string()))
            .unwrap();
        let iso = steps
            .iter()
            .position(|s| s.argv.contains(&"isolated".to_string()))
            .unwrap();
        let up = steps
            .iter()
            .position(|s| {
                s.argv.contains(&"wtv3".to_string()) && s.argv.contains(&"up".to_string())
            })
            .unwrap();
        assert!(
            master < iso && iso < up,
            "isolated before the port comes up"
        );
    }

    #[test]
    fn the_gateway_never_forwards_and_loads_its_rules_first() {
        let up = gateway_up_steps();
        assert!(
            up[0]
                .stdin
                .as_deref()
                .is_some_and(|t| t.contains("wol_tor_apps")),
            "rules before the bridge exists"
        );
        assert!(
            up.iter()
                .any(|s| s.argv.iter().any(|a| a.ends_with(".forwarding=0")))
        );
        assert!(
            !up.iter()
                .any(|s| s.argv.iter().any(|a| a.contains("ip_forward=1")))
        );
        let down = gateway_down_steps();
        assert_eq!(down[0].argv[..3], ["ip", "link", "del"]);
    }

    #[test]
    fn join_drops_every_capability_and_keeps_the_command_after_the_separator() {
        let ns = AppNs::new("discord", 1).unwrap();
        let v = join_argv(
            &ns,
            Path::new("/usr/lib/wol-tor"),
            1000,
            1000,
            &["/usr/bin/discord".into(), "--x".into()],
        );
        let dd = v.iter().rposition(|a| a == "--").unwrap();
        assert_eq!(&v[dd + 1..], ["/usr/bin/discord", "--x"]);
        for want in [
            "--no-new-privs",
            "--inh-caps=-all",
            "--bounding-set=-all",
            "--reuid",
            "1000",
            "wolt-discord",
        ] {
            assert!(v.iter().any(|a| a == want), "{want}");
        }
        assert_eq!(v[..4], ["ip", "netns", "exec", "wolt-discord"]);
    }

    #[test]
    fn gateway_ports_are_bound_where_the_redirect_lands() {
        let sys_lines = torrc_lines(true, false);
        let app_lines = torrc_lines(false, true);
        let both = torrc_lines(true, true);
        assert!(
            both[0].starts_with("TransPort 127.0.0.1:")
                && both[2].starts_with("TransPort 10.191.0.1:")
        );
        assert!(!sys_lines.iter().any(|l| l.contains("10.191.0.1")));
        assert!(sys_lines[0].starts_with("TransPort 127.0.0.1:9040 "));
        assert!(app_lines[0].starts_with("TransPort 10.191.0.1:9040 "));
        assert!(app_lines[0].contains("IsolateClientAddr"));
        assert!(sys_lines.contains(&"DNSPort 127.0.0.1:5353".to_string()));
        assert!(app_lines.contains(&"AutomapHostsOnResolve 1".to_string()));
    }

    #[test]
    fn the_tor_binary_must_be_root_owned_and_not_group_or_world_writable() {
        assert!(tor_binary_trusted(0, 0o755).is_ok());
        assert!(tor_binary_trusted(1000, 0o755).is_err());
        assert!(tor_binary_trusted(0, 0o775).is_err());
        assert!(tor_binary_trusted(0, 0o757).is_err());
        assert!(
            is_packaged_tor(Path::new("/usr/bin/tor")) && !is_packaged_tor(Path::new("/tmp/tor"))
        );
    }
}
