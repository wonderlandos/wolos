//! Date & time (zone, network time) and language choices for Settings > General.
use crate::system::Host;

pub const ZONE_TABS: [&str; 2] = [
    "/usr/share/zoneinfo/zone1970.tab",
    "/usr/share/zoneinfo/zone.tab",
];

/// zone1970.tab / zone.tab: the TZ column, sorted, unique.
pub fn parse_zone_tab(text: &str) -> Vec<String> {
    let mut v: Vec<String> = text
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split('\t').nth(2))
        .map(str::trim)
        .filter(|z| !z.is_empty())
        .map(String::from)
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Every zone the system knows; empty without tzdata.
pub fn zones(h: &dyn Host) -> Vec<String> {
    ZONE_TABS
        .iter()
        .filter_map(|p| h.read_text(p))
        .map(|t| parse_zone_tab(&t))
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

/// `/etc/localtime` (timedatectl's symlink); "UTC" when missing or odd.
pub fn current_zone(h: &dyn Host) -> String {
    let link = h.read_link("/etc/localtime");
    let z = crate::settings::timezones::current(link.as_deref());
    if crate::settings::timezones::is_valid(z) {
        z.into()
    } else {
        "UTC".into()
    }
}

/// Network time on (systemd-timesyncd through timedatectl)? None when unknown.
pub fn ntp_enabled(h: &dyn Host) -> Option<bool> {
    match h
        .run(&["timedatectl", "show", "-p", "NTP", "--value"])?
        .trim()
    {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

/// "America/Sao_Paulo" -> "Sao Paulo (America)".
pub fn zone_label(zone: &str) -> String {
    let city = zone.rsplit('/').next().unwrap_or(zone).replace('_', " ");
    match zone.split_once('/') {
        Some((region, _)) => format!("{city} ({region})"),
        None => city,
    }
}

/// (locale, label) shipped choices, label in its own language.
pub const LANGUAGES: [(&str, &str); 12] = [
    ("en_US.UTF-8", "English (US)"),
    ("en_GB.UTF-8", "English (UK)"),
    ("pt_BR.UTF-8", "Português (Brasil)"),
    ("pt_PT.UTF-8", "Português (Portugal)"),
    ("es_ES.UTF-8", "Español"),
    ("fr_FR.UTF-8", "Français"),
    ("de_DE.UTF-8", "Deutsch"),
    ("it_IT.UTF-8", "Italiano"),
    ("nl_NL.UTF-8", "Nederlands"),
    ("pl_PL.UTF-8", "Polski"),
    ("ja_JP.UTF-8", "日本語"),
    ("zh_CN.UTF-8", "中文 (简体)"),
];

/// `ll_CC.UTF-8` (2-3 lowercase letters, 2 uppercase).
pub fn is_valid_locale(s: &str) -> bool {
    s.strip_suffix(".UTF-8")
        .and_then(|l| l.split_once('_'))
        .is_some_and(|(l, r)| {
            (2..=3).contains(&l.len())
                && l.bytes().all(|b| b.is_ascii_lowercase())
                && r.len() == 2
                && r.bytes().all(|b| b.is_ascii_uppercase())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

    const TAB: &str = "# tzdb timezone descriptions\n#\n#codes\tcoordinates\tTZ\tcomments\nBR\t-2332-04637\tAmerica/Sao_Paulo\tBrazil (southeast)\nPT\t+3843-00908\tEurope/Lisbon\tPortugal (mainland)\nCH,DE,LI\t+4723+00832\tEurope/Zurich\tBüsingen\nAR\t-3436-05827\tAmerica/Argentina/Buenos_Aires\tBuenos Aires (BA, CF)\nPT\t+3843-00908\tEurope/Lisbon\n\n";

    #[test]
    fn zone_tab() {
        assert_eq!(
            parse_zone_tab(TAB),
            [
                "America/Argentina/Buenos_Aires",
                "America/Sao_Paulo",
                "Europe/Lisbon",
                "Europe/Zurich"
            ]
        );
        assert!(parse_zone_tab("").is_empty());
        let h = FakeHost::linux();
        assert!(zones(&h).is_empty());
        h.file(
            "/usr/share/zoneinfo/zone.tab",
            "BR\t-2332-04637\tAmerica/Sao_Paulo\n",
        );
        assert_eq!(zones(&h), ["America/Sao_Paulo"]);
        h.file("/usr/share/zoneinfo/zone1970.tab", TAB);
        assert_eq!(zones(&h).len(), 4, "zone1970.tab wins");
    }

    #[test]
    fn current_zone_is_the_localtime_link() {
        let h = FakeHost::linux();
        assert_eq!(current_zone(&h), "UTC");
        h.link("/etc/localtime", "../usr/share/zoneinfo/America/Sao_Paulo");
        assert_eq!(current_zone(&h), "America/Sao_Paulo");
        h.link("/etc/localtime", "/usr/share/zoneinfo/Europe/Lisbon");
        assert_eq!(current_zone(&h), "Europe/Lisbon");
        h.link("/etc/localtime", "/usr/share/zoneinfo/../../etc/passwd");
        assert_eq!(current_zone(&h), "UTC");
        h.link("/etc/localtime", "/etc/passwd");
        assert_eq!(current_zone(&h), "UTC");
    }

    #[test]
    fn network_time() {
        let l = FakeHost::linux();
        assert_eq!(ntp_enabled(&l), None);
        l.command("timedatectl show -p NTP --value", "yes\n");
        assert_eq!(ntp_enabled(&l), Some(true));
        l.command("timedatectl show -p NTP --value", "no\n");
        assert_eq!(ntp_enabled(&l), Some(false));
    }

    #[test]
    fn real_host_zone_is_valid() {
        let z = current_zone(&crate::system::PosixHost);
        assert!(crate::settings::timezones::is_valid(&z), "{z}");
    }

    #[test]
    fn zone_labels() {
        for (z, want) in [
            ("America/Sao_Paulo", "Sao Paulo (America)"),
            ("America/Argentina/Buenos_Aires", "Buenos Aires (America)"),
            ("Europe/Lisbon", "Lisbon (Europe)"),
            ("UTC", "UTC"),
        ] {
            assert_eq!(zone_label(z), want, "{z}");
        }
    }

    #[test]
    fn locales() {
        for (l, ok) in [
            ("pt_BR.UTF-8", true),
            ("ast_ES.UTF-8", true),
            ("pt_BR", false),
            ("PT_br.UTF-8", false),
            ("pt_BR.UTF-8; rm", false),
            ("", false),
        ] {
            assert_eq!(is_valid_locale(l), ok, "{l}");
        }
        assert!(LANGUAGES.iter().all(|(l, _)| is_valid_locale(l)));
    }
}
