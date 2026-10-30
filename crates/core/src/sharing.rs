//! Settings > General > Sharing and Network > Firewall: computer name, Remote Login (sshd) and
//! the firewall (nftables). Reads go through [`Host`]; changes through wol-admind.
use crate::system::Host;

/// The ruleset nftables.service loads (block in, pass out; desktop/overlay).
pub const FIREWALL_RULES: &str = "/etc/wolos/nftables.conf";

/// One RFC 1123 label, lowercase: 1..=63 of `[a-z0-9-]`, no `-` at either end.
pub fn is_valid_hostname(s: &str) -> bool {
    (1..=63).contains(&s.len())
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Unit file state (`systemctl show -p UnitFileState --value <unit>`): enabled at boot or not;
/// None when unknown.
pub fn parse_enabled(text: &str) -> Option<bool> {
    match text.trim() {
        "enabled" | "enabled-runtime" | "alias" => Some(true),
        "disabled" | "masked" | "masked-runtime" | "static" | "indirect" | "linked" | "" => {
            Some(false)
        }
        _ => None,
    }
}

/// Unit enabled at boot (`systemctl show` exits 0 for disabled and missing units too).
pub fn enabled(h: &dyn Host, unit: &str) -> Option<bool> {
    parse_enabled(&h.run(&["systemctl", "show", "-p", "UnitFileState", "--value", unit])?)
}

/// Computer name now.
pub fn hostname(h: &dyn Host) -> Option<String> {
    let name = h.read_text("/proc/sys/kernel/hostname")?;
    Some(name.trim().to_owned()).filter(|n| !n.is_empty())
}

/// Remote Login (sshd) on at boot.
pub fn remote_login(h: &dyn Host) -> Option<bool> {
    enabled(h, "sshd.service")
}

/// The AirPlay receiver service (shairport-sync, shipped with the audio stack).
pub const AIRPLAY_UNIT: &str = "shairport-sync.service";

/// AirPlay Receiver on at boot.
pub fn airplay_receiver(h: &dyn Host) -> Option<bool> {
    enabled(h, AIRPLAY_UNIT)
}

/// Firewall (nftables) on at boot.
pub fn firewall(h: &dyn Host) -> Option<bool> {
    enabled(h, "nftables.service")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

    #[test]
    fn airplay_receiver_reads_the_unit_state() {
        let h = FakeHost::new(false);
        assert_eq!(airplay_receiver(&h), None, "no systemd: unknown");
        h.command(
            "systemctl show -p UnitFileState --value shairport-sync.service",
            "enabled\n",
        );
        assert_eq!(airplay_receiver(&h), Some(true));
        assert_eq!(parse_enabled("disabled"), Some(false));
    }

    #[test]
    fn hostname_is_one_rfc1123_label() {
        let long = "a".repeat(63);
        let too_long = "a".repeat(64);
        for (s, ok) in [
            ("wolos", true),
            ("julias-laptop", true),
            ("9lives", true),
            ("a", true),
            (long.as_str(), true),
            (too_long.as_str(), false),
            ("", false),
            ("-wolos", false),
            ("wolos-", false),
            ("Wolos", false),
            ("wol os", false),
            ("wol.os", false),
            ("wol_os", false),
            ("wolos;reboot", false),
            ("wölos", false),
        ] {
            assert_eq!(is_valid_hostname(s), ok, "{s:?}");
        }
    }

    #[test]
    fn systemctl_is_enabled_values() {
        for (t, want) in [
            ("enabled\n", Some(true)),
            ("disabled\n", Some(false)),
            ("masked\n", Some(false)),
            ("static\n", Some(false)),
            ("", Some(false)),
            ("Failed to get unit file state", None),
        ] {
            assert_eq!(parse_enabled(t), want, "{t:?}");
        }
    }

    #[test]
    fn reads() {
        let h = FakeHost::new(false);
        assert_eq!(
            (hostname(&h), remote_login(&h), firewall(&h)),
            (None, None, None),
            "no systemd: unknown"
        );
        h.file("/proc/sys/kernel/hostname", "wolos\n")
            .command(
                "systemctl show -p UnitFileState --value nftables.service",
                "enabled\n",
            )
            .command("systemctl show -p UnitFileState --value sshd.service", "\n");
        assert_eq!(hostname(&h).as_deref(), Some("wolos"));
        assert_eq!(firewall(&h), Some(true));
        assert_eq!(remote_login(&h), Some(false), "not installed: off");
    }
}
