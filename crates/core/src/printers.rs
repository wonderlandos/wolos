// SPDX-License-Identifier: BSD-2-Clause
//! Printers for Settings: CUPS queues (lpstat), discovery (lpinfo) and lpadmin argv for wol-admind.
use crate::system::Host;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Printer {
    pub name: String,
    pub uri: String,
    pub default: bool,
    /// "Idle", "Printing" or "Stopped".
    pub state: String,
    /// What CUPS says about the queue now ("Connecting to printer.", "The printer is not
    /// responding."), empty when it says nothing (QA-DEV-3: a printer that went away stayed
    /// "Printing" with no reason).
    #[serde(default)]
    pub message: String,
    /// Jobs waiting or printing (`lpstat -o`).
    #[serde(default)]
    pub jobs: usize,
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

/// `lpstat -p`: (queue, the queue's message), from the indented line under each printer line;
/// CUPS's placeholder "reason unknown" is no message.
pub fn parse_lpstat_p_messages(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut queue: Option<String> = None;
    for l in text.lines() {
        if let Some(rest) = l.strip_prefix("printer ") {
            queue = rest.split_once(' ').map(|(n, _)| n.to_string());
        } else if l.starts_with([' ', '\t'])
            && let Some(q) = &queue
        {
            let m = l.trim();
            if !m.is_empty() && m != "reason unknown" && !out.iter().any(|(n, _)| n == q) {
                out.push((q.clone(), m.to_string()));
            }
        }
    }
    out
}

/// `lpstat -o`: jobs per queue ("Office-12 ana 1024 Mon 26 Oct ..." is job 12 of Office).
pub fn parse_lpstat_o(text: &str, queue: &str) -> usize {
    text.lines()
        .filter_map(|l| l.split_whitespace().next()?.rsplit_once('-'))
        .filter(|(q, n)| *q == queue && n.chars().all(|c| c.is_ascii_digit()))
        .count()
}

/// CUPS's own test page, printed on `name` (QA-DEV-5). Any user may print; no admin.
pub const TEST_PAGE: &str = "/usr/share/cups/data/testprint";

pub fn test_page_argv(name: &str) -> Option<Vec<String>> {
    is_valid_name(name).then(|| argv(&["lp", "-d", name, "--", TEST_PAGE]))
}

/// Cancels every job of `name` the person may cancel (their own; no admin, QA-DEV-3).
pub fn cancel_argv(name: &str) -> Option<Vec<String>> {
    is_valid_name(name).then(|| argv(&["cancel", "-a", "--", name]))
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

/// [`queue_name`] of `display` that no name in `taken` has: "HP_LaserJet" taken -> "HP_LaserJet_2"
/// (QA-SET-14: two printers of one model both came out "HP_LaserJet", the second `lpadmin -p`
/// replaced the first). A name that cleans down to nothing ("日本語") is "Printer".
pub fn unique_queue_name(display: &str, taken: &[String]) -> String {
    let mut base = queue_name(display);
    if base.is_empty() {
        base = "Printer".into();
    }
    let free = |n: &str| !taken.iter().any(|t| t.eq_ignore_ascii_case(n));
    if free(&base) {
        return base;
    }
    (2..)
        .map(|i| {
            let tail = format!("_{i}");
            let keep: String = base.chars().take(127 - tail.len()).collect();
            keep + &tail
        })
        .find(|n| free(n))
        .unwrap_or(base)
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
    let p = h.run(&["lpstat", "-p"]).unwrap_or_default();
    let (states, messages) = (parse_lpstat_p(&p), parse_lpstat_p_messages(&p));
    let jobs = h.run(&["lpstat", "-o"]).unwrap_or_default();
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
                message: messages
                    .iter()
                    .find(|s| s.0 == name)
                    .map(|s| s.1.clone())
                    .unwrap_or_default(),
                jobs: parse_lpstat_o(&jobs, &name),
                name,
                uri,
            })
            .collect(),
    )
}

/// Devices CUPS can see (slow: backends probe the network; call off the UI thread). `None` when
/// `lpinfo` itself failed: CUPS answers `lpinfo -v` only to its administrators, so for a plain
/// account that is "not allowed", which is not the same as "no printer found" (QA-FDAY-15).
pub fn discover(h: &dyn Host) -> Option<Vec<String>> {
    h.run_slow(&["lpinfo", "-v"]).map(|t| parse_lpinfo_v(&t))
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

    /// QA-DEV-3 / QA-DEV-5: a printer that went away says why and how many jobs wait; the
    /// test page and Cancel run as the person (no admin).
    #[test]
    fn messages_jobs_and_user_argv() {
        let p = "printer Office now printing Office-12.  enabled since Mon 26 Oct 2026\n\tConnecting to printer.\nprinter HP_LaserJet is idle.  enabled since Mon\nprinter Old disabled since Mon -\n\treason unknown\n";
        assert_eq!(
            parse_lpstat_p_messages(p),
            [("Office".to_string(), "Connecting to printer.".to_string())]
        );
        let o = "Office-12   ana   1024   Mon 26 Oct 2026 02:06:00 PM\nOffice-13   ana   2048   Mon\nHP_LaserJet-3  bo  1  Mon\n";
        assert_eq!(parse_lpstat_o(o, "Office"), 2);
        assert_eq!(parse_lpstat_o(o, "HP_LaserJet"), 1);
        assert_eq!(parse_lpstat_o(o, "Old"), 0);
        assert_eq!(
            test_page_argv("Office").unwrap(),
            ["lp", "-d", "Office", "--", TEST_PAGE]
        );
        assert_eq!(
            cancel_argv("Office").unwrap(),
            ["cancel", "-a", "--", "Office"]
        );
        assert!(test_page_argv("-x").is_none() && cancel_argv("a b").is_none());
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
        // QA-SET-14: a second printer of one model gets its own queue, never the first one's.
        let taken = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(unique_queue_name("HP LaserJet", &[]), "HP_LaserJet");
        assert_eq!(
            unique_queue_name("HP LaserJet", &taken(&["HP_LaserJet"])),
            "HP_LaserJet_2"
        );
        assert_eq!(
            unique_queue_name("HP LaserJet", &taken(&["hp_laserjet", "HP_LaserJet_2"])),
            "HP_LaserJet_3"
        );
        assert_eq!(unique_queue_name("日本語", &[]), "Printer");
        assert_eq!(
            unique_queue_name("日本語", &taken(&["Printer"])),
            "Printer_2"
        );
        let long = "x".repeat(127);
        let n = unique_queue_name(&long, &taken(&[&long]));
        assert!(
            n.len() <= 127 && n.ends_with("_2") && is_valid_name(&n),
            "{n}"
        );
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
        assert_eq!(discover(&h), Some(vec!["ipp://10.0.0.9/ipp/print".into()]));
    }

    /// QA-FDAY-15: lpinfo failing (a plain account is refused) is not "no printers found".
    #[test]
    fn a_refused_lpinfo_is_not_an_empty_answer() {
        let h = FakeHost::new(true);
        assert_eq!(discover(&h), None, "lpinfo failed");
        h.slow("lpinfo -v", "network ipp\n");
        assert_eq!(discover(&h), Some(vec![]), "answered, nothing found");
    }
}
