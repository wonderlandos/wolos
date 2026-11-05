// SPDX-License-Identifier: BSD-2-Clause
//! Network interfaces and their traffic: byte counters from `/proc/net/dev`, the kind and link
//! state from `/sys/class/net/<name>`. Loopback is never listed.
use super::Root;

/// Cumulative bytes of one interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counters {
    pub name: String,
    pub rx: u64,
    pub tx: u64,
}

/// What an interface is: wired, wireless or virtual.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Kind {
    Wired,
    Wireless,
    /// No device behind it: a bridge, a VPN tunnel, a container link.
    Virtual,
}

/// One network interface with its rates since the last sample.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Interface {
    pub name: String,
    pub kind: Kind,
    pub up: bool,
    /// Bytes per second since the previous sample.
    pub rx_rate: f64,
    pub tx_rate: f64,
    /// Bytes since the interface came up.
    pub rx_total: u64,
    pub tx_total: u64,
}

/// `name: rx_bytes rx_packets ... (8 receive fields) tx_bytes ...`, after two header lines.
pub fn parse_net_dev(text: &str) -> Vec<Counters> {
    text.lines()
        .filter_map(|l| {
            let (name, rest) = l.split_once(':')?;
            let name = name.trim();
            let f: Vec<u64> = rest
                .split_ascii_whitespace()
                .map(|v| v.parse().ok())
                .collect::<Option<_>>()?;
            (name != "lo" && !name.is_empty() && f.len() >= 9).then(|| Counters {
                name: name.to_owned(),
                rx: f[0],
                tx: f[8],
            })
        })
        .collect()
}

/// The kind of the interface `name` from its sysfs files.
pub fn kind(root: &Root, name: &str) -> Kind {
    let base = format!("/sys/class/net/{name}");
    if root.exists(&format!("{base}/wireless")) || root.exists(&format!("{base}/phy80211")) {
        Kind::Wireless
    } else if root.exists(&format!("{base}/device")) {
        Kind::Wired
    } else {
        Kind::Virtual
    }
}

/// The link is up (`operstate`); a tunnel that reports `unknown` counts as up while it moves data.
pub fn is_up(root: &Root, name: &str) -> bool {
    matches!(
        root.read(&format!("/sys/class/net/{name}/operstate"))
            .as_deref()
            .map(str::trim),
        Some("up")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_or_broken_lines_are_skipped() {
        let t =
            "h\nh\n eth0: 1 2 3\nwg0: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0\nbad: x 0 0 0 0 0 0 0 0\n";
        assert_eq!(
            parse_net_dev(t),
            [Counters {
                name: "wg0".into(),
                rx: 10,
                tx: 20
            }]
        );
    }
}
