//! Printers for Settings: CUPS queues (lpstat), discovery (lpinfo) and lpadmin argv for wol-admind.
use crate::system::Host;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Printer {
    pub name: String,
    pub uri: String,
    pub default: bool,
    /// "Idle", "Printing" or "Stopped".
    pub state: String,
}

/// `lpstat -v`: "device for HP_LaserJet: ipp://10.0.0.5/ipp/print".
pub fn parse_lpstat_v(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let (name, uri) = l.trim().strip_prefix("device for ")?.split_once(": ")?;
            Some((name.to_string(), uri.trim().to_string()))
        })
        .collect()
}

/// `lpstat -d`: "system default destination: HP_LaserJet".
pub fn parse_lpstat_d(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        l.trim()
            .strip_prefix("system default destination:")
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
    })
}

/// `lpstat -p`: (queue, "Idle" | "Printing" | "Stopped").
pub fn parse_lpstat_p(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("printer ")?;
            let (name, state) = rest.split_once(' ')?;
            let state = if state.starts_with("disabled") {
                "Stopped"
            } else if state.starts_with("now printing") {
                "Printing"
            } else {
                "Idle"
            };
            Some((name.to_string(), state.to_string()))
        })
        .collect()
}

/// `lpinfo -v`: device URIs found; bare backend names ("network ipp") dropped.
pub fn parse_lpinfo_v(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .filter(|u| u.contains("://"))
        .map(String::from)
        .collect()
}

/// CUPS queue name: printable, no space, '/', '#', no leading '-', at most 127 bytes.
pub fn is_valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 127
        && !s.starts_with('-')
        && s.chars().all(|c| {
            c.is_ascii_graphic() && c != '/' && c != '#' && c != '\\' && c != '\'' && c != '"'
        })
}

const SCHEMES: [&str; 8] = [
    "ipp", "ipps", "http", "https", "socket", "lpd", "usb", "dnssd",
];

/// Device URI lpadmin may take (network and USB backends only).
pub fn is_valid_uri(s: &str) -> bool {
    s.len() < 512
        && !s.starts_with('-')
        && !s.chars().any(|c| c.is_whitespace() || c.is_control())
        && s.split_once("://")
            .is_some_and(|(scheme, rest)| SCHEMES.contains(&scheme) && !rest.is_empty())
}

/// "HP LaserJet Pro" -> "HP_LaserJet_Pro".
pub fn queue_name(display: &str) -> String {
    let mut out = String::new();
    for w in display.split_whitespace() {
        let w: String = w
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
            .collect();
        if w.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('_');
        }
        out.push_str(&w);
    }
    out.trim_start_matches('-').chars().take(127).collect()
}

fn argv(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Driverless queue (IPP Everywhere) for a discovered device.
pub fn add_argv(name: &str, uri: &str) -> Option<Vec<String>> {
    (is_valid_name(name) && is_valid_uri(uri))
        .then(|| argv(&["lpadmin", "-p", name, "-E", "-v", uri, "-m", "everywhere"]))
}

pub fn remove_argv(name: &str) -> Option<Vec<String>> {
    is_valid_name(name).then(|| argv(&["lpadmin", "-x", name]))
}

pub fn default_argv(name: &str) -> Option<Vec<String>> {
    is_valid_name(name).then(|| argv(&["lpadmin", "-d", name]))
}

/// Queues with state and default; None when CUPS is not installed or not running.
pub fn read(h: &dyn Host) -> Option<Vec<Printer>> {
    let r = h.run(&["lpstat", "-r"])?;
    if r.contains("not running") {
        return None;
    }
    let default = h.run(&["lpstat", "-d"]).and_then(|t| parse_lpstat_d(&t));
    let states = h
        .run(&["lpstat", "-p"])
        .map(|t| parse_lpstat_p(&t))
        .unwrap_or_default();
    Some(
        h.run(&["lpstat", "-v"])
            .map(|t| parse_lpstat_v(&t))
            .unwrap_or_default()
            .into_iter()
            .map(|(name, uri)| Printer {
                default: default.as_deref() == Some(name.as_str()),
                state: states
                    .iter()
                    .find(|s| s.0 == name)
                    .map_or("Idle", |s| s.1.as_str())
                    .to_string(),
                name,
                uri,
            })
            .collect(),
    )
}

/// Devices CUPS can see (slow: backends probe the network; call off the UI thread).
pub fn discover(h: &dyn Host) -> Vec<String> {
    h.run_slow(&["lpinfo", "-v"])
        .map(|t| parse_lpinfo_v(&t))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

    const LPSTAT_V: &str = "device for HP_LaserJet: ipp://10.0.0.5/ipp/print\ndevice for Office: dnssd://Office%20Printer._ipp._tcp.local/\n";
    const LPSTAT_P: &str = "printer HP_LaserJet is idle.  enabled since Mon 26 Oct 2026 02:05:00 PM\nprinter Office now printing Office-12.  enabled since Mon 26 Oct 2026 02:06:00 PM\nprinter Old disabled since Mon 26 Oct 2026 02:07:00 PM -\n\treason unknown\n";

    #[test]
    fn lpstat_outputs() {
        assert_eq!(
            parse_lpstat_v(LPSTAT_V),
            [
                (
                    "HP_LaserJet".to_string(),
                    "ipp://10.0.0.5/ipp/print".to_string()
                ),
                (
                    "Office".to_string(),
                    "dnssd://Office%20Printer._ipp._tcp.local/".to_string()
                )
            ]
        );
        assert_eq!(
            parse_lpstat_d("system default destination: HP_LaserJet\n").as_deref(),
            Some("HP_LaserJet")
        );
        assert_eq!(parse_lpstat_d("no system default destination\n"), None);
        assert_eq!(
            parse_lpstat_p(LPSTAT_P),
            [
                ("HP_LaserJet".to_string(), "Idle".to_string()),
                ("Office".to_string(), "Printing".to_string()),
                ("Old".to_string(), "Stopped".to_string())
            ]
        );
    }

    #[test]
    fn lpinfo_keeps_full_uris() {
        let t = "network https\nnetwork ipp\nnetwork ipps\ndirect usb://HP/LaserJet%20Pro?serial=123\nnetwork dnssd://HP%20LaserJet._ipp._tcp.local/?uuid=1\nnetwork socket\nnetwork lpd\n";
        assert_eq!(
            parse_lpinfo_v(t),
            [
                "usb://HP/LaserJet%20Pro?serial=123",
                "dnssd://HP%20LaserJet._ipp._tcp.local/?uuid=1"
            ]
        );
    }

    #[test]
    fn names_and_uris() {
        for (n, ok) in [
            ("HP_LaserJet", true),
            ("Office-2", true),
            ("", false),
            ("-x", false),
            ("a b", false),
            ("a/b", false),
            ("a#b", false),
            ("a\tb", false),
        ] {
            assert_eq!(is_valid_name(n), ok, "{n:?}");
        }
        assert!(!is_valid_name(&"x".repeat(128)));
        for (u, ok) in [
            ("ipp://10.0.0.5/ipp/print", true),
            ("ipps://printer.local:631/ipp/print", true),
            ("socket://10.0.0.5:9100", true),
            ("usb://HP/LaserJet?serial=1", true),
            ("dnssd://Office%20Printer._ipp._tcp.local/", true),
            ("file:///etc/passwd", false),
            ("ipp://a b", false),
            ("-v", false),
            ("ipp", false),
        ] {
            assert_eq!(is_valid_uri(u), ok, "{u}");
        }
        assert_eq!(queue_name("HP LaserJet Pro"), "HP_LaserJet_Pro");
        assert_eq!(queue_name("  Café / #2 "), "Caf_2");
    }

    #[test]
    fn lpadmin_argv() {
        assert_eq!(
            add_argv("HP", "ipp://10.0.0.5/ipp/print")
                .unwrap()
                .join(" "),
            "lpadmin -p HP -E -v ipp://10.0.0.5/ipp/print -m everywhere"
        );
        assert_eq!(add_argv("-x", "ipp://h/p"), None);
        assert_eq!(add_argv("HP", "file:///x"), None);
        assert_eq!(remove_argv("HP").unwrap().join(" "), "lpadmin -x HP");
        assert_eq!(default_argv("HP").unwrap().join(" "), "lpadmin -d HP");
        assert_eq!(remove_argv("a b"), None);
    }

    #[test]
    fn reads_through_host() {
        let h = FakeHost::new(true);
        assert_eq!(read(&h), None, "no lpstat: no CUPS");
        h.command("lpstat -r", "scheduler is not running\n");
        assert_eq!(read(&h), None);
        h.command("lpstat -r", "scheduler is running\n")
            .command("lpstat -v", LPSTAT_V)
            .command("lpstat -d", "system default destination: Office\n")
            .command("lpstat -p", LPSTAT_P);
        let p = read(&h).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(
            (p[1].name.as_str(), p[1].default, p[1].state.as_str()),
            ("Office", true, "Printing")
        );
        assert!(!p[0].default);
        h.command("lpstat -v", "").command("lpstat -p", "");
        assert_eq!(read(&h), Some(vec![]), "CUPS up, no printers");
        h.slow(
            "lpinfo -v",
            "network ipp\nnetwork ipp://10.0.0.9/ipp/print\n",
        );
        assert_eq!(discover(&h), ["ipp://10.0.0.9/ipp/print"]);
    }
}
