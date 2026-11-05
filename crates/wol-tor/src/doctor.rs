// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol doctor tor` (ADR 0056): what is observed about the Tor layer, and what it means.
//!
//! [`Observations`] holds what could be measured (each `None` = not measured, never guessed);
//! [`evaluate`] turns it into findings with a fix sentence. The measuring is in the binary; the
//! judging is here so it is tested. The leak test contacts a check service, and it does so only
//! over tor's own SOCKS socket: it can never be the first clear-net contact.
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How bad a finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Fine.
    Ok,
    /// Not measured.
    Unknown,
    /// Works, but worth fixing.
    Warn,
    /// The layer does not do what it promises.
    Fail,
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ok => "ok",
            Self::Unknown => "not checked",
            Self::Warn => "warning",
            Self::Fail => "FAIL",
        })
    }
}

/// One line of the report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Check name.
    pub check: &'static str,
    /// Result.
    pub level: Level,
    /// What was found, with the fix when there is one.
    pub text: String,
}

/// What was measured.
#[derive(Clone, Debug, Default)]
pub struct Observations {
    /// Bootstrap percentage of the gateway or group tor.
    pub bootstrap: Option<u8>,
    /// The check service's answer over tor: `Some(true)` = it saw a Tor exit.
    pub is_tor: Option<bool>,
    /// The address a canary name resolved to through the layer; `Some` means a name was resolved.
    pub canary_answer: Option<String>,
    /// valid-after and valid-until of the cached consensus.
    pub consensus: Option<(SystemTime, SystemTime)>,
    /// Now (a parameter so tests do not read the clock).
    pub now: Option<SystemTime>,
    /// The packet-drop rules are loaded (system mode): `Some(false)` is a fail-open.
    pub kill_switch_loaded: Option<bool>,
    /// A socket from outside tor could connect (the kill-switch probe): `Some(true)` is a leak.
    pub probe_escaped: Option<bool>,
}

/// Skew beyond which tor refuses a consensus.
pub const MAX_SKEW: Duration = Duration::from_secs(3600);

fn f(check: &'static str, level: Level, text: impl Into<String>) -> Finding {
    Finding {
        check,
        level,
        text: text.into(),
    }
}

/// Whether an address is in tor's virtual range (`10.192.0.0/10`): the DNSPort's answers.
pub fn is_virtual_addr(a: &str) -> bool {
    let mut p = a.split('.').map(str::parse::<u8>);
    matches!((p.next(), p.next()), (Some(Ok(10)), Some(Ok(b))) if (192..=255).contains(&b))
}

/// Judges the observations.
pub fn evaluate(o: &Observations) -> Vec<Finding> {
    let mut out = Vec::new();
    out.push(match o.bootstrap {
        None => f(
            "bootstrap",
            Level::Unknown,
            "tor is not running or its control socket cannot be read",
        ),
        Some(100) => f("bootstrap", Level::Ok, "tor is bootstrapped"),
        Some(n) => f(
            "bootstrap",
            Level::Warn,
            format!("tor is at {n}%: wait, or check the network and the bridges (wol-tor bridges)"),
        ),
    });
    out.push(match o.is_tor {
        None => f(
            "tor-exit",
            Level::Unknown,
            "the check service was not asked (tor is not up)",
        ),
        Some(true) => f("tor-exit", Level::Ok, "the check service sees a Tor exit"),
        Some(false) => f(
            "tor-exit",
            Level::Fail,
            "the check service does NOT see a Tor exit: traffic is not leaving through tor",
        ),
    });
    out.push(match o.canary_answer.as_deref() {
        None => f("dns", Level::Unknown, "no canary name was resolved"),
        Some(a) if is_virtual_addr(a) => f(
            "dns",
            Level::Ok,
            "names resolve through tor (virtual address answer)",
        ),
        Some(a) => f(
            "dns",
            Level::Fail,
            format!(
                "a canary name resolved to {a}, not a tor virtual address: DNS leaves outside tor"
            ),
        ),
    });
    out.push(match (o.consensus, o.now) {
        (Some((after, until)), Some(now)) => {
            let early = after.duration_since(now).unwrap_or_default();
            let late = now.duration_since(until).unwrap_or_default();
            let secs = |t: SystemTime| t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
            if early > MAX_SKEW {
                f(
                    "clock",
                    Level::Fail,
                    format!(
                        "the clock is {} s behind the Tor consensus (valid-after {}): fix the time (timedatectl)",
                        early.as_secs(),
                        secs(after)
                    ),
                )
            } else if late > MAX_SKEW * 3 {
                f("clock", Level::Warn, "the cached consensus is old, or the clock is far ahead: tor will fetch a new one")
            } else {
                f("clock", Level::Ok, "the clock agrees with the consensus")
            }
        }
        _ => f("clock", Level::Unknown, "no consensus to compare the clock with"),
    });
    out.push(match o.kill_switch_loaded {
        None => f(
            "kill-switch",
            Level::Unknown,
            "the rules were not read (needs administrator rights)",
        ),
        Some(true) => f("kill-switch", Level::Ok, "the drop rules are loaded"),
        Some(false) => f(
            "kill-switch",
            Level::Fail,
            "the drop rules are NOT loaded: turn Tor for the machine off and on again",
        ),
    });
    out.push(match o.probe_escaped {
        None => f("probe", Level::Unknown, "no outside connection was attempted"),
        Some(false) => f("probe", Level::Ok, "a connection that does not go through tor was refused"),
        Some(true) => f("probe", Level::Fail, "a connection that does not go through tor SUCCEEDED: this is a leak, stop using the layer and report it"),
    });
    out
}

/// The worst level, for the exit code (`Unknown` does not make a run fail).
pub fn worst(findings: &[Finding]) -> Level {
    findings
        .iter()
        .map(|x| x.level)
        .filter(|l| *l != Level::Unknown)
        .max()
        .unwrap_or(Level::Unknown)
}

/// Reads the check service's JSON reply (`{"IsTor":true,"IP":"..."}`) out of an HTTP response.
/// `None` for anything else, a redirect page included: not measured, not a pass.
pub fn parse_is_tor(response: &str) -> Option<bool> {
    let body = response.split_once("\r\n\r\n").map_or(response, |(_, b)| b);
    let v: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    v.get("IsTor")?.as_bool()
}

/// The name the canary asks for: only a tor DNSPort answers it with a virtual address.
pub const CANARY_NAME: &str = "wol-doctor-canary.onion";

/// Where the canary query is sent: the local resolver address. The system rules redirect every
/// port-53 packet before the "leave loopback alone" rule, so the DNSPort answers; if the rules are
/// missing, nothing answers and the canary reads "not checked", not a pass. (A routable test
/// address would fail with "network unreachable" on a machine with no default route.)
pub const CANARY_SERVER: &str = "127.0.0.1:53";

/// A DNS query for the A record of `name` (id 0x574f, recursion desired).
pub fn dns_query(name: &str) -> Option<Vec<u8>> {
    let mut q = vec![0x57, 0x4f, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.trim_end_matches('.').split('.') {
        let n = u8::try_from(label.len())
            .ok()
            .filter(|n| (1..=63).contains(n))?;
        q.push(n);
        q.extend_from_slice(label.as_bytes());
    }
    q.extend_from_slice(&[0, 0, 1, 0, 1]);
    Some(q)
}

fn skip_name(r: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let b = *r.get(i)?;
        if b & 0xC0 == 0xC0 {
            return Some(i + 2);
        }
        if b == 0 {
            return Some(i + 1);
        }
        i += 1 + usize::from(b);
    }
}

/// The first A record of a DNS response, as dotted text.
pub fn dns_first_a(r: &[u8]) -> Option<String> {
    if r.len() < 12 || r[0..2] != [0x57, 0x4f] || r[3] & 0x0f != 0 {
        return None;
    }
    let (qd, an) = (
        u16::from_be_bytes([r[4], r[5]]),
        u16::from_be_bytes([r[6], r[7]]),
    );
    let mut i = 12;
    for _ in 0..qd {
        i = skip_name(r, i)? + 4;
    }
    for _ in 0..an {
        i = skip_name(r, i)?;
        let h = r.get(i..i + 10)?;
        let (ty, len) = (
            u16::from_be_bytes([h[0], h[1]]),
            usize::from(u16::from_be_bytes([h[8], h[9]])),
        );
        i += 10;
        if ty == 1 && len == 4 {
            let a = r.get(i..i + 4)?;
            return Some(format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3]));
        }
        i += len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn healthy() -> Observations {
        Observations {
            bootstrap: Some(100),
            is_tor: Some(true),
            canary_answer: Some("10.193.4.5".into()),
            consensus: Some((at(1000), at(1000 + 10_800))),
            now: Some(at(2000)),
            kill_switch_loaded: Some(true),
            probe_escaped: Some(false),
        }
    }

    fn level(fs: &[Finding], check: &str) -> Level {
        fs.iter().find(|x| x.check == check).unwrap().level
    }

    #[test]
    fn a_healthy_layer_passes_every_check() {
        let fs = evaluate(&healthy());
        assert!(fs.iter().all(|x| x.level == Level::Ok), "{fs:?}");
        assert_eq!(worst(&fs), Level::Ok);
    }

    #[test]
    fn nothing_measured_is_not_a_pass_and_not_a_failure() {
        let fs = evaluate(&Observations::default());
        assert!(fs.iter().all(|x| x.level == Level::Unknown));
        assert_eq!(worst(&fs), Level::Unknown);
    }

    #[test]
    fn a_dns_answer_outside_the_virtual_range_is_a_leak() {
        let mut o = healthy();
        o.canary_answer = Some("93.184.216.34".into());
        assert_eq!(level(&evaluate(&o), "dns"), Level::Fail);
        o.canary_answer = Some("10.10.0.1".into());
        assert_eq!(
            level(&evaluate(&o), "dns"),
            Level::Fail,
            "10/8 outside 10.192/10 is not tor's"
        );
        assert!(is_virtual_addr("10.192.0.1") && is_virtual_addr("10.255.255.254"));
        assert!(
            !is_virtual_addr("10.191.255.255")
                && !is_virtual_addr("11.192.0.1")
                && !is_virtual_addr("x")
        );
    }

    #[test]
    fn a_non_tor_exit_and_an_escaped_probe_fail() {
        let mut o = healthy();
        o.is_tor = Some(false);
        o.probe_escaped = Some(true);
        let fs = evaluate(&o);
        assert_eq!(level(&fs, "tor-exit"), Level::Fail);
        assert_eq!(level(&fs, "probe"), Level::Fail);
        assert_eq!(worst(&fs), Level::Fail);
    }

    #[test]
    fn a_missing_kill_switch_fails() {
        let mut o = healthy();
        o.kill_switch_loaded = Some(false);
        assert_eq!(level(&evaluate(&o), "kill-switch"), Level::Fail);
    }

    #[test]
    fn clock_skew_is_judged_against_the_consensus() {
        let mut o = healthy();
        o.now = Some(at(10_000 - 3601));
        // The consensus starts in the future by more than an hour: tor would refuse it.
        o.consensus = Some((at(10_000), at(20_800)));
        let fs = evaluate(&o);
        assert_eq!(level(&fs, "clock"), Level::Fail);
        assert!(
            fs.iter()
                .find(|x| x.check == "clock")
                .unwrap()
                .text
                .contains("timedatectl")
        );
        o.now = Some(at(20_800 + 3 * 3600 + 1));
        assert_eq!(level(&evaluate(&o), "clock"), Level::Warn);
    }

    #[test]
    fn partial_bootstrap_warns_and_names_the_percentage() {
        let mut o = healthy();
        o.bootstrap = Some(45);
        let fs = evaluate(&o);
        assert_eq!(level(&fs, "bootstrap"), Level::Warn);
        assert!(fs[0].text.contains("45%"));
    }

    #[test]
    fn the_canary_goes_to_loopback_where_the_nat_chain_redirects_it() {
        let addr: std::net::SocketAddr = CANARY_SERVER.parse().unwrap();
        assert!(addr.ip().is_loopback() && addr.port() == 53);
        // The same packet through the real system rules lands on the DNSPort, as an ordinary user.
        use crate::transparent::{DNS_PORT, Packet, Proto, SystemGateway, Verdict, system_ruleset};
        let rules = system_ruleset(&SystemGateway {
            tor_uid: 105,
            lan: vec![],
        });
        let p = Packet {
            uid: Some(1000),
            iif: None,
            dst: addr.ip(),
            proto: Proto::Udp,
            dport: addr.port(),
            local: true,
        };
        assert_eq!(rules.verdict(&p), Verdict::Redirected(DNS_PORT));
    }

    #[test]
    fn a_dns_query_and_an_answer_round_trip() {
        let q = dns_query("canary.onion").unwrap();
        assert_eq!(&q[..4], [0x57, 0x4f, 1, 0]);
        assert!(q.ends_with(&[0, 0, 1, 0, 1]));
        assert!(dns_query("a..b").is_none() && dns_query(&"x".repeat(64)).is_none());
        // Response: header (1 question, 1 answer), the question again, a compressed-name A answer.
        let mut r = vec![0x57, 0x4f, 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0];
        r.extend_from_slice(&q[12..]);
        r.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 10, 193, 4, 5]);
        assert_eq!(dns_first_a(&r), Some("10.193.4.5".into()));
        r[3] = 0x83; // NXDOMAIN
        assert_eq!(dns_first_a(&r), None);
        assert_eq!(dns_first_a(&[1, 2, 3]), None);
    }

    #[test]
    fn the_check_service_reply_is_read_strictly() {
        let ok = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"IsTor\":true,\"IP\":\"192.0.2.1\"}";
        assert_eq!(parse_is_tor(ok), Some(true));
        assert_eq!(
            parse_is_tor("HTTP/1.1 200 OK\r\n\r\n{\"IsTor\":false}"),
            Some(false)
        );
        assert_eq!(
            parse_is_tor("HTTP/1.1 301 Moved\r\nLocation: https://x\r\n\r\n"),
            None
        );
        assert_eq!(parse_is_tor("{\"IsTor\":\"yes\"}"), None);
        assert_eq!(parse_is_tor(""), None);
    }
}
