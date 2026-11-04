// SPDX-License-Identifier: GPL-2.0-or-later
//! The hardened torrc of a Tor group (ADR 0053 D2, D4), rendered from typed values.
//!
//! The file is generated, never edited: every value that comes from outside (a bridge line the
//! person pasted, a path) is checked first, so nothing can add a line to the config. tor reads it
//! with `--defaults-torrc /dev/null -f <file>` so no system torrc adds ports.
//! What is on, and why:
//! - SOCKS and control on Unix sockets only, no TCP port at all (`SocksPort 0` is not needed: a
//!   `SocksPort unix:` line replaces the default 9050).
//! - `IsolateSOCKSAuth` explicit: the relay's credentials pick the circuit ([`crate::isolation`]).
//!   No `IsolateDestAddr`: a circuit per third-party host would make pages slow and adds nothing
//!   that the per-site credential does not already give.
//! - `SafeSocks 1`: a SOCKS request with a bare IP from an app that resolved the name itself is
//!   refused, so a leak in the resolver order fails loudly.
//! - `ClientRejectInternalAddresses 1`, `ClientOnly 1`, `AvoidDiskWrites 1`, `SafeLogging 1`.
//! - Speed: conflux with the latency profile, circuit padding, learned circuit-build timeout.
//! - `Sandbox 1` only without pluggable transports [V: tor 0.4.8/0.4.9 with PT exec].
use std::fmt;
use std::path::{Path, PathBuf};

/// Longest bridge line accepted (a Snowflake line with its front domains is about 500 bytes).
pub const MAX_BRIDGE_LINE: usize = 2048;

/// A pluggable transport a bridge line can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Transport {
    /// obfs4 (lyrebird).
    Obfs4,
    /// WebTunnel (lyrebird).
    Webtunnel,
    /// meek over a CDN (lyrebird's `meek_lite`).
    MeekLite,
    /// Snowflake (snowflake-client).
    Snowflake,
}

impl Transport {
    /// The transport name as tor writes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Obfs4 => "obfs4",
            Self::Webtunnel => "webtunnel",
            Self::MeekLite => "meek_lite",
            Self::Snowflake => "snowflake",
        }
    }

    fn from_name(s: &str) -> Option<Self> {
        match s {
            "obfs4" => Some(Self::Obfs4),
            "webtunnel" => Some(Self::Webtunnel),
            "meek_lite" => Some(Self::MeekLite),
            "snowflake" => Some(Self::Snowflake),
            _ => None,
        }
    }
}

/// One checked `Bridge` line (the value after the `Bridge` keyword).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bridge {
    /// `None` for a plain bridge (`IP:port [fingerprint]`).
    pub transport: Option<Transport>,
    line: String,
}

/// Why a config was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum TorrcError {
    /// A bridge line that is empty, too long, has a control character or an unknown transport.
    BadBridge(String),
    /// A path with whitespace, a quote, a control character, or not absolute.
    BadPath(PathBuf),
    /// A bridge needs a transport program that is not configured.
    MissingTransport(&'static str),
}

impl fmt::Display for TorrcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadBridge(l) => write!(
                f,
                "this bridge line is not one Tor understands: {l:?}; copy it again from bridges.torproject.org"
            ),
            Self::BadPath(p) => write!(
                f,
                "the path {} cannot be used in the Tor config",
                p.display()
            ),
            Self::MissingTransport(t) => {
                write!(
                    f,
                    "the {t} bridge needs its transport program, which is not installed"
                )
            }
        }
    }
}

impl std::error::Error for TorrcError {}

impl Bridge {
    /// Checks one line as found on bridges.torproject.org, with or without a leading `Bridge `.
    pub fn parse(raw: &str) -> Result<Self, TorrcError> {
        let bad = || TorrcError::BadBridge(raw.to_string());
        let line = raw.trim();
        let line = line.strip_prefix("Bridge ").unwrap_or(line).trim();
        if line.is_empty()
            || line.len() > MAX_BRIDGE_LINE
            || !line.bytes().all(|b| (0x20..0x7f).contains(&b))
        {
            return Err(bad());
        }
        let mut words = line.split(' ').filter(|w| !w.is_empty());
        let first = words.next().ok_or_else(bad)?;
        let (transport, addr) = if looks_like_addr(first) {
            (None, first)
        } else {
            (
                Some(Transport::from_name(first).ok_or_else(bad)?),
                words.next().ok_or_else(bad)?,
            )
        };
        if !looks_like_addr(addr) {
            return Err(bad());
        }
        let line = line
            .split(' ')
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        Ok(Self { transport, line })
    }

    /// The line after the `Bridge` keyword.
    pub fn line(&self) -> &str {
        &self.line
    }
}

/// `1.2.3.4:443` or `[2001:db8::1]:443`: what tor accepts as a bridge address.
fn looks_like_addr(s: &str) -> bool {
    s.parse::<std::net::SocketAddr>().is_ok()
}

/// The transport programs installed (packages `lyrebird`, `snowflake-client`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Transports {
    /// lyrebird: obfs4, webtunnel, meek_lite.
    pub lyrebird: Option<PathBuf>,
    /// snowflake-client.
    pub snowflake: Option<PathBuf>,
}

/// What a group's tor needs to know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TorConfig {
    /// DataDirectory (the unit's tmpfs runtime directory).
    pub data_dir: PathBuf,
    /// SOCKS5 Unix socket.
    pub socks: PathBuf,
    /// Control Unix socket.
    pub control: PathBuf,
    /// Control cookie file.
    pub cookie: PathBuf,
    /// Bridges; empty = connect to the Tor network directly.
    pub bridges: Vec<Bridge>,
    /// Installed transport programs.
    pub transports: Transports,
    /// The machine-wide gateway's listeners (`None` for an Atlas group): (system, apps).
    pub gateway: Option<(bool, bool)>,
}

fn path(p: &Path) -> Result<&str, TorrcError> {
    let s = p
        .to_str()
        .ok_or_else(|| TorrcError::BadPath(p.to_path_buf()))?;
    let ok = p.is_absolute()
        && s.bytes()
            .all(|b| b > 0x20 && b < 0x7f && b != b'"' && b != b'\\' && b != b'#');
    if ok {
        Ok(s)
    } else {
        Err(TorrcError::BadPath(p.to_path_buf()))
    }
}

impl TorConfig {
    /// The torrc text. Lines only from this function; every outside value was checked.
    pub fn render(&self) -> Result<String, TorrcError> {
        let (data, socks, control, cookie) = (
            path(&self.data_dir)?,
            path(&self.socks)?,
            path(&self.control)?,
            path(&self.cookie)?,
        );
        let mut out = String::from(
            "# Generated by wol-tor (ADR 0053). Do not edit: the next start rewrites it.\n",
        );
        let mut line = |s: &str| {
            out.push_str(s);
            out.push('\n');
        };
        line(&format!("DataDirectory {data}"));
        // With the machine scope on, Atlas tab groups use this tor through the socket (instead of a
        // second tor of their own), so every local user's browser may connect; each connection is
        // isolated by its credential. Anything on the machine already reaches tor via the redirect.
        let world = if matches!(self.gateway, Some((true, _))) {
            " WorldWritable"
        } else {
            ""
        };
        line(&format!("SocksPort unix:{socks} IsolateSOCKSAuth{world}"));
        line(&format!("ControlPort unix:{control}"));
        line("CookieAuthentication 1");
        line(&format!("CookieAuthFile {cookie}"));
        for fixed in [
            "ClientOnly 1",
            "SafeSocks 1",
            "TestSocks 0",
            "ClientRejectInternalAddresses 1",
            "AvoidDiskWrites 1",
            "SafeLogging 1",
            "Log notice stdout",
            "ConfluxEnabled 1",
            "ConfluxClientUX latency",
            "CircuitPadding 1",
            "LearnCircuitBuildTimeout 1",
            "ClientUseIPv6 1",
            "ClientPreferIPv6ORPort auto",
            "FetchDirInfoEarly 1",
            "FetchUselessDescriptors 0",
        ] {
            line(fixed);
        }
        if let Some((system, apps)) = self.gateway {
            for l in crate::transparent::torrc_lines(system, apps) {
                line(&l);
            }
        }
        let mut used: Vec<Transport> = self.bridges.iter().filter_map(|b| b.transport).collect();
        used.sort();
        used.dedup();
        if self.bridges.is_empty() {
            line("Sandbox 1");
        } else {
            line("UseBridges 1");
            let lyre: Vec<&str> = used
                .iter()
                .filter(|t| **t != Transport::Snowflake)
                .map(|t| t.name())
                .collect();
            if !lyre.is_empty() {
                let p = self
                    .transports
                    .lyrebird
                    .as_deref()
                    .ok_or(TorrcError::MissingTransport("obfs4"))?;
                line(&format!(
                    "ClientTransportPlugin {} exec {}",
                    lyre.join(","),
                    path(p)?
                ));
            }
            if used.contains(&Transport::Snowflake) {
                let p = self
                    .transports
                    .snowflake
                    .as_deref()
                    .ok_or(TorrcError::MissingTransport("snowflake"))?;
                line(&format!(
                    "ClientTransportPlugin snowflake exec {}",
                    path(p)?
                ));
            }
            for b in &self.bridges {
                line(&format!("Bridge {}", b.line()));
            }
        }
        Ok(out)
    }
}

/// The argv tor is started with: no system defaults, this file only.
pub fn tor_argv(tor: &Path, torrc: &Path) -> Vec<String> {
    let mut v = Vec::new();
    for s in [
        tor.display().to_string(),
        "--defaults-torrc".into(),
        "/dev/null".into(),
        "-f".into(),
    ] {
        v.push(s);
    }
    v.push(torrc.display().to_string());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> TorConfig {
        TorConfig {
            data_dir: "/r/wol-tor/g/data".into(),
            socks: "/r/wol-tor/g/socks".into(),
            control: "/r/wol-tor/g/control".into(),
            cookie: "/r/wol-tor/g/control.cookie".into(),
            bridges: vec![],
            transports: Transports::default(),
            gateway: None,
        }
    }

    #[test]
    fn gateway_config_adds_the_transparent_ports_and_dns_without_a_fallback() {
        let plain = cfg().render().unwrap();
        assert!(!plain.contains("TransPort") && !plain.contains("DNSPort"));
        let mut c = cfg();
        c.gateway = Some((true, true));
        let t = c.render().unwrap();
        assert!(t.contains("TransPort 127.0.0.1:9040 IsolateClientAddr IsolateClientProtocol\n"));
        assert!(t.contains("TransPort 10.191.0.1:9040 IsolateClientAddr IsolateClientProtocol\n"));
        assert!(t.contains("DNSPort 127.0.0.1:5353\n") && t.contains("DNSPort 10.191.0.1:5353\n"));
        assert!(t.contains("AutomapHostsOnResolve 1\n"));
        assert!(t.contains("VirtualAddrNetworkIPv4 10.192.0.0/10\n"));
        assert!(t.contains("ClientOnly 1\n") && t.contains("SafeLogging 1\n"));
        assert!(
            t.contains("IsolateSOCKSAuth WorldWritable\n"),
            "machine scope: groups use this tor"
        );
        assert!(
            !plain.contains("WorldWritable"),
            "an Atlas group's socket stays private"
        );
        c.gateway = Some((false, true));
        assert!(
            !c.render().unwrap().contains("WorldWritable"),
            "apps only: no shared SOCKS socket"
        );
    }

    #[test]
    fn direct_config_has_unix_ports_and_sandbox() {
        let t = cfg().render().unwrap();
        assert!(t.contains("SocksPort unix:/r/wol-tor/g/socks IsolateSOCKSAuth"));
        assert!(t.contains("ControlPort unix:/r/wol-tor/g/control\n"));
        assert!(t.contains("Sandbox 1\n"));
        assert!(t.contains("ConfluxEnabled 1\n"));
        assert!(t.contains("SafeSocks 1\n"));
        assert!(!t.contains("UseBridges"));
        assert!(
            !t.lines()
                .any(|l| l.starts_with("SocksPort ") && !l.contains("unix:")),
            "no TCP SOCKS port"
        );
        assert!(!t.contains("ControlPort 9"), "no TCP control port");
    }

    #[test]
    fn bridge_lines_from_the_bridges_page_parse() {
        let o = Bridge::parse("Bridge obfs4 192.0.2.10:443 0123456789ABCDEF0123456789ABCDEF01234567 cert=abc iat-mode=0").unwrap();
        assert_eq!(o.transport, Some(Transport::Obfs4));
        assert_eq!(
            o.line(),
            "obfs4 192.0.2.10:443 0123456789ABCDEF0123456789ABCDEF01234567 cert=abc iat-mode=0"
        );
        let s = Bridge::parse("snowflake 192.0.2.3:80 2B280B23E1107BB62ABFC40DDCC8824814F80A72 fingerprint=2B28 url=https://x.example/").unwrap();
        assert_eq!(s.transport, Some(Transport::Snowflake));
        let plain =
            Bridge::parse("[2001:db8::1]:9001 0123456789ABCDEF0123456789ABCDEF01234567").unwrap();
        assert_eq!(plain.transport, None);
    }

    #[test]
    fn bridge_lines_cannot_inject_config() {
        for bad in [
            "",
            "obfs4",
            "obfs4 notanaddr:1",
            "vmess 192.0.2.1:443",
            "obfs4 192.0.2.1:443 x\nSocksPort 9050",
            "obfs4 192.0.2.1:443 x\rControlPort 9051",
            "obfs4 192.0.2.1:443 \u{7f}",
        ] {
            assert!(Bridge::parse(bad).is_err(), "{bad:?}");
        }
        assert!(
            Bridge::parse(&format!(
                "obfs4 192.0.2.1:443 {}",
                "a".repeat(MAX_BRIDGE_LINE)
            ))
            .is_err()
        );
    }

    #[test]
    fn bridges_need_their_transport_program() {
        let mut c = cfg();
        c.bridges = vec![
            Bridge::parse("obfs4 192.0.2.1:443 F cert=x").unwrap(),
            Bridge::parse("snowflake 192.0.2.3:80 F").unwrap(),
        ];
        assert_eq!(c.render(), Err(TorrcError::MissingTransport("obfs4")));
        c.transports.lyrebird = Some("/usr/bin/lyrebird".into());
        assert_eq!(c.render(), Err(TorrcError::MissingTransport("snowflake")));
        c.transports.snowflake = Some("/usr/bin/snowflake-client".into());
        let t = c.render().unwrap();
        assert!(t.contains("UseBridges 1\n"));
        assert!(t.contains("ClientTransportPlugin obfs4 exec /usr/bin/lyrebird\n"));
        assert!(t.contains("ClientTransportPlugin snowflake exec /usr/bin/snowflake-client\n"));
        assert!(t.contains("Bridge obfs4 192.0.2.1:443 F cert=x\n"));
        assert!(
            !t.contains("Sandbox 1"),
            "PT exec and the seccomp sandbox are not combined"
        );
    }

    #[test]
    fn paths_with_spaces_or_quotes_are_refused() {
        for p in ["/a b/data", "/a\"b", "relative/data", "/a#b"] {
            let mut c = cfg();
            c.data_dir = p.into();
            assert_eq!(c.render(), Err(TorrcError::BadPath(p.into())), "{p}");
        }
    }

    #[test]
    fn tor_runs_without_system_defaults() {
        assert_eq!(
            tor_argv(Path::new("/usr/bin/tor"), Path::new("/r/torrc")),
            [
                "/usr/bin/tor",
                "--defaults-torrc",
                "/dev/null",
                "-f",
                "/r/torrc"
            ]
        );
    }
}
