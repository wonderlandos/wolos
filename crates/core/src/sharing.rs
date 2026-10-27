//! Settings > General > Sharing and Network > Firewall: computer name, Remote Login (sshd) and
//! the firewall (pf). Reads go through [`Host`]; changes through wol-admind.
use crate::system::Host;

/// The ruleset `firewall on` loads (block in, pass out; desktop/overlay).
pub const PF_RULES: &str = "/usr/local/share/wolos/pf.conf";

/// One RFC 1123 label, lowercase: 1..=63 of `[a-z0-9-]`, no `-` at either end.
pub fn is_valid_hostname(s: &str) -> bool {
    (1..=63).contains(&s.len())
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// rc.conf boolean as rc.subr's checkyesno reads it (`sysrc -n <var>` output); `NONE` is off.
pub fn parse_rc_yes(text: &str) -> Option<bool> {
    match text.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "on" | "1" => Some(true),
        "no" | "false" | "off" | "0" | "none" => Some(false),
        _ => None,
    }
}

fn rc_yes(h: &dyn Host, var: &str) -> Option<bool> {
    if !h.is_freebsd() {
        return None;
    }
    parse_rc_yes(&h.run(&["sysrc", "-n", var])?)
}

/// Computer name now (`kern.hostname`; Linux dev host: /proc).
pub fn hostname(h: &dyn Host) -> Option<String> {
    let name = if h.is_freebsd() {
        h.sysctl_string("kern.hostname")?
    } else {
        h.read_text("/proc/sys/kernel/hostname")?
    };
    Some(name.trim().to_owned()).filter(|n| !n.is_empty())
}

/// Remote Login (sshd) on at boot; None off FreeBSD.
pub fn remote_login(h: &dyn Host) -> Option<bool> {
    rc_yes(h, "sshd_enable")
}

/// Firewall (pf) on at boot; None off FreeBSD.
pub fn firewall(h: &dyn Host) -> Option<bool> {
    rc_yes(h, "pf_enable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

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
    fn rc_yes_values() {
        for (t, want) in [
            ("YES\n", Some(true)),
            ("yes", Some(true)),
            ("TRUE\n", Some(true)),
            ("on", Some(true)),
            ("1", Some(true)),
            ("NO\n", Some(false)),
            ("false", Some(false)),
            ("off", Some(false)),
            ("0", Some(false)),
            ("NONE", Some(false)),
            ("", None),
            ("maybe", None),
        ] {
            assert_eq!(parse_rc_yes(t), want, "{t:?}");
        }
    }

    #[test]
    fn freebsd_reads() {
        let h = FakeHost::new(true);
        assert_eq!(
            (hostname(&h), remote_login(&h), firewall(&h)),
            (None, None, None)
        );
        h.string("kern.hostname", "wolos\n")
            .command("sysrc -n sshd_enable", "NO\n")
            .command("sysrc -n pf_enable", "YES\n");
        assert_eq!(hostname(&h).as_deref(), Some("wolos"));
        assert_eq!(remote_login(&h), Some(false));
        assert_eq!(firewall(&h), Some(true));
        let l = FakeHost::new(false);
        assert_eq!((remote_login(&l), firewall(&l)), (None, None));
        l.file("/proc/sys/kernel/hostname", "devbox\n");
        assert_eq!(hostname(&l).as_deref(), Some("devbox"));
    }
}
