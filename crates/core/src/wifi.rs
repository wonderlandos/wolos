// SPDX-License-Identifier: BSD-2-Clause
//! Wi-Fi through NetworkManager (`nmcli`): input validation, the join steps, how a failed join
//! reads, the kind of security a network asks for, and nmcli's terse escapes. See
//! docs/modules/wifi.md. (The wpa_supplicant.conf writer this file once held is gone, QA-NET-20.)

/// Error text, or None when NetworkManager can take these. None password = open network.
/// 802.11: a name is 1 to 32 bytes. A WPA passphrase is 8 to 63 characters, or the 256-bit key
/// itself as 64 hex digits (what many routers print on their label, QA-NET-18).
pub fn validate(ssid: &str, password: Option<&str>) -> Option<&'static str> {
    let typed = |s: &str| s.chars().all(|c| c >= ' ' && c != '\x7f');
    if ssid.is_empty() {
        return Some("Network name is empty");
    }
    if ssid.len() > 32 || !typed(ssid) {
        return Some("Network name has unsupported characters");
    }
    let p = password?;
    if !typed(p) {
        return Some("Password has unsupported characters");
    }
    let n = p.chars().count();
    let hex_key = n == 64 && p.bytes().all(|b| b.is_ascii_hexdigit());
    (!(8..=63).contains(&n) && !hex_key).then_some("Password must be 8 to 63 characters")
}

/// One nmcli terse value as typed: `\:` is `:` and `\\` is `\` (nmcli escapes both, QA-NET-19).
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some(n) => out.push(n),
                None => out.push(c),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// What a network asks for, from nmcli's SECURITY column ("WPA2", "WPA1 WPA2 802.1X", "OWE").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Security {
    #[default]
    Open,
    /// A password (WPA/WPA2/WPA3 Personal, WEP).
    Psk,
    /// 802.1X: a user name and password or a certificate (WPA2/WPA3 Enterprise). A join with a
    /// PSK can never succeed (QA-NET-9).
    Enterprise,
    /// Opportunistic Wireless Encryption: no password, encrypted.
    Owe,
}

/// [`Security`] from the SECURITY field (empty or `--` = open).
pub fn security_kind(sec: &str) -> Security {
    let w: Vec<&str> = sec.split_whitespace().collect();
    if w.contains(&"802.1X") {
        Security::Enterprise
    } else if w.is_empty() || w == ["--"] {
        Security::Open
    } else if w == ["OWE"] {
        Security::Owe
    } else {
        Security::Psk
    }
}

/// Why `nmcli` turned a join down, from its stderr (QA-NET-8: every failure used to read as a
/// wrong password).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinFailure {
    /// The network asked for the secret again: it refused the one given.
    WrongPassword,
    /// No Wi-Fi device, the radio is off, or NetworkManager is not running.
    NoDevice,
    /// No network of that name in range.
    NotFound,
    /// Joined the radio, got no address (DHCP gave up), or the join ran out of time.
    NoAddress,
    /// Anything else.
    Other,
}

/// [`JoinFailure`] for nmcli's error text. `secret`: a password was sent (a "secrets" error
/// without one means the network needs one).
pub fn classify_join_error(stderr: &str, secret: bool) -> JoinFailure {
    let e = stderr.to_ascii_lowercase();
    let has = |k: &[&str]| k.iter().any(|w| e.contains(w));
    if has(&[
        "no wi-fi device",
        "no suitable device",
        "wi-fi is disabled",
        "radio is disabled",
        "networkmanager is not running",
        "could not create nmclient",
    ]) {
        JoinFailure::NoDevice
    } else if has(&["no network with ssid", "network could not be found"]) {
        JoinFailure::NotFound
    } else if has(&[
        "secrets were required",
        "supplicant disconnected",
        "802.1x supplicant",
        "psk: property is invalid",
    ]) {
        if secret {
            JoinFailure::WrongPassword
        } else {
            JoinFailure::Other
        }
    } else if has(&[
        "ip configuration could not be reserved",
        "timeout",
        "timed out",
        "dhcp",
    ]) {
        JoinFailure::NoAddress
    } else {
        JoinFailure::Other
    }
}

/// Saved connections with their type, terse (`NAME:TYPE`); [`nm_has_profile`] reads it.
pub const NM_PROFILES_ARGV: [&str; 6] = ["nmcli", "-t", "-f", "NAME,TYPE", "connection", "show"];

/// Is there a saved Wi-Fi profile named `ssid` in [`NM_PROFILES_ARGV`]'s output? Only Wi-Fi ones
/// count: an Ethernet profile that happens to share the name is not the network.
pub fn nm_has_profile(listing: &str, ssid: &str) -> bool {
    crate::netinfo::parse_nmcli_connections(listing)
        .iter()
        .any(|n| n == ssid)
}

/// The scan, terse, SSID last (it may contain `\:`): what [`nm_key_mgmt`] reads.
pub const NM_SCAN_ARGV: [&str; 7] = ["nmcli", "-t", "-f", "SECURITY,SSID", "dev", "wifi", "list"];

/// `FIELD:rest` at the first colon that is not escaped (`\\:`) in nmcli's terse output.
fn split_unescaped(l: &str) -> Option<(&str, &str)> {
    let mut esc = false;
    for (i, c) in l.char_indices() {
        if c == ':' && !esc {
            return Some((&l[..i], &l[i + 1..]));
        }
        esc = c == '\\' && !esc;
    }
    None
}

/// `wifi-sec.key-mgmt` for a password join: `sae` when every access point of that name in the
/// scan (`scan` is [`NM_SCAN_ARGV`]'s output) offers WPA3 and nothing older, else `wpa-psk`
/// (WPA2, a WPA2/WPA3 mix, or a network not in range, e.g. a hidden one). A WPA3-only network
/// cannot be joined with `wpa-psk`.
pub fn nm_key_mgmt(scan: &str, ssid: &str) -> &'static str {
    let mut seen = false;
    for l in scan.lines() {
        let Some((sec, name)) = split_unescaped(l) else {
            continue;
        };
        if unescape(name) != ssid {
            continue;
        }
        let words: Vec<&str> = sec.split_whitespace().collect();
        let sae_only =
            words.contains(&"WPA3") && !words.iter().any(|w| matches!(*w, "WPA1" | "WPA2" | "WPA"));
        if !sae_only {
            return "wpa-psk";
        }
        seen = true;
    }
    if seen { "sae" } else { "wpa-psk" }
}

/// The steps of a password join through NetworkManager (NET2-1). `nmcli dev wifi connect ... passwd-file`
/// is not valid nmcli (rc 2, "invalid extra argument"): `passwd-file` belongs to `connection up`.
/// So the profile is made first (no secret in argv), then brought up with the secret on stdin
/// ([`nm_up_argv`], [`nm_secret_input`]). `exists`: a Wi-Fi profile of that name is saved; it is
/// changed in place (`connection modify`), so the person's autoconnect and address settings
/// survive and a second join never leaves two profiles for one network.
pub fn nm_profile_argvs(
    ssid: &str,
    hidden: bool,
    key_mgmt: &str,
    exists: bool,
) -> Vec<Vec<String>> {
    let s = |v: &[&str]| v.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>();
    let mut step = if exists {
        s(&["nmcli", "connection", "modify", "id", ssid])
    } else {
        s(&[
            "nmcli",
            "connection",
            "add",
            "type",
            "wifi",
            "con-name",
            ssid,
            "ifname",
            "*",
            "ssid",
            ssid,
        ])
    };
    step.extend(s(&["wifi-sec.key-mgmt", key_mgmt]));
    if hidden {
        step.extend(s(&["802-11-wireless.hidden", "yes"]));
    }
    vec![step]
}

/// Removes a profile a join made when bringing it up failed (a wrong password must not leave a
/// profile behind). Only for a join that found no profile of that name first.
pub fn nm_cleanup_argv(ssid: &str) -> Vec<String> {
    ["nmcli", "connection", "delete", "id", ssid]
        .map(str::to_owned)
        .to_vec()
}

/// `nmcli connection up <ssid> passwd-file /dev/stdin`: the secret comes on stdin as
/// [`nm_secret_input`].
pub fn nm_up_argv(ssid: &str) -> Vec<String> {
    [
        "nmcli",
        "connection",
        "up",
        "id",
        ssid,
        "passwd-file",
        "/dev/stdin",
    ]
    .map(str::to_owned)
    .to_vec()
}

/// The `passwd-file` text for a WPA-PSK profile.
pub fn nm_secret_input(password: &str) -> String {
    format!("802-11-wireless-security.psk:{password}\n")
}

/// The scan Settings and Control Center show: terse, SSID last, `--rescan auto` (NetworkManager
/// scans when its list is older than 30 s; QA-NET-16).
pub const NM_LIST_ARGV: [&str; 9] = [
    "nmcli",
    "-t",
    "-f",
    "IN-USE,SIGNAL,SECURITY,SSID",
    "dev",
    "wifi",
    "list",
    "--rescan",
    "auto",
];

/// [`Security`] of `ssid` in [`NM_SCAN_ARGV`]'s output; None when it is not in range. Any access
/// point of that name that asks for 802.1X makes it Enterprise.
pub fn nm_security(scan: &str, ssid: &str) -> Option<Security> {
    let mut found = None;
    for l in scan.lines() {
        let Some((sec, name)) = split_unescaped(l) else {
            continue;
        };
        if unescape(name) != ssid {
            continue;
        }
        let k = security_kind(sec);
        if k == Security::Enterprise {
            return Some(k);
        }
        found = Some(k);
    }
    found
}

/// `nmcli dev disconnect <iface>`: leave the current network, keep its profile (QA-NET-17).
/// None for a name that is not an interface name.
pub fn nm_disconnect_argv(iface: &str) -> Option<Vec<String>> {
    let ok = (1..=15).contains(&iface.len())
        && !iface.starts_with('-')
        && iface
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'/' && b != b':');
    ok.then(|| {
        ["nmcli", "dev", "disconnect", iface]
            .map(str::to_owned)
            .to_vec()
    })
}

/// Saved Wi-Fi profiles with their SSID: `nmcli -t -f NAME,UUID,TYPE connection show`, then
/// [`nm_ssid_argv`] per profile. Forget goes by the SSID, not the profile name (NetworkManager
/// names a second profile "Home 1"; QA-NET-17).
pub const NM_UUIDS_ARGV: [&str; 6] = ["nmcli", "-t", "-f", "UUID,TYPE", "connection", "show"];

/// Wi-Fi profile UUIDs from [`NM_UUIDS_ARGV`]'s output.
pub fn parse_wifi_uuids(listing: &str) -> Vec<String> {
    listing
        .lines()
        .filter_map(|l| l.strip_suffix(":802-11-wireless"))
        .filter(|u| !u.is_empty() && u.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
        .map(str::to_owned)
        .collect()
}

/// The SSID a profile joins: `nmcli -g 802-11-wireless.ssid connection show uuid <uuid>`.
pub fn nm_ssid_argv(uuid: &str) -> Vec<String> {
    [
        "nmcli",
        "-g",
        "802-11-wireless.ssid",
        "connection",
        "show",
        "uuid",
        uuid,
    ]
    .map(str::to_owned)
    .to_vec()
}

/// The active profiles' names, terse: what [`HOTSPOT_PROFILE`] is looked for in.
pub const NM_ACTIVE_ARGV: [&str; 7] = [
    "nmcli",
    "-t",
    "-f",
    "NAME",
    "connection",
    "show",
    "--active",
];

/// The profile name of the Wi-Fi hotspot (one per computer; changed in place).
pub const HOTSPOT_PROFILE: &str = "WolOS Hotspot";

/// The hotspot profile (NET2-2): an access point on the Wi-Fi device, WPA2-PSK, NetworkManager's
/// shared mode (it runs dnsmasq for addresses and NAT to the link that has the internet). No
/// secret in argv: [`nm_up_argv`] with [`nm_secret_input`] brings it up. `exists`: the profile is
/// saved, so it is changed in place. Never connects by itself.
pub fn hotspot_argvs(ssid: &str, exists: bool) -> Vec<Vec<String>> {
    let s = |v: &[&str]| v.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>();
    let mut step = if exists {
        s(&["nmcli", "connection", "modify", "id", HOTSPOT_PROFILE])
    } else {
        s(&[
            "nmcli",
            "connection",
            "add",
            "type",
            "wifi",
            "con-name",
            HOTSPOT_PROFILE,
            "ifname",
            "*",
        ])
    };
    step.extend(s(&[
        "ssid",
        ssid,
        "802-11-wireless.mode",
        "ap",
        "802-11-wireless.band",
        "bg",
        "ipv4.method",
        "shared",
        "ipv6.method",
        "ignore",
        "wifi-sec.key-mgmt",
        "wpa-psk",
        "wifi-sec.psk-flags",
        "0",
        "connection.autoconnect",
        "no",
    ]));
    if exists {
        // `ssid` is the setting's alias only for `add`; `modify` takes the property name.
        step[5] = "802-11-wireless.ssid".into();
    }
    vec![step]
}

/// Where a Setup Assistant join stands (`controller::login::wifi_reply`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinState {
    Joined,
    /// The network refused us (wrong password, the access point said no).
    Refused,
    /// Scanning, associating or in the handshake.
    Trying,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// QA-NET-18: the 64-character hex key routers print is a valid PSK (nmcli takes it); 64
    /// characters that are not hex are not.
    #[test]
    fn hex64_psk_ok() {
        let hex = "0123456789abcdefABCDEF0123456789abcdef0123456789abcdef0123456789";
        assert_eq!(hex.len(), 64);
        assert_eq!(validate("Home", Some(hex)), None);
        assert!(validate("Home", Some(&"g".repeat(64))).is_some());
        assert!(validate("Home", Some(&"a".repeat(65))).is_some());
    }

    #[test]
    fn ssid_and_password_validation_table() {
        for (ssid, pw, ok) in [
            ("Home", None, true),
            ("Home", Some("password1"), true),
            ("", None, false),
            ("Ho\"me", None, true),
            ("Home", Some("short"), false),
            ("Home", Some("pass\\word1"), true),
            ("Home", Some("pa\"ss#}word"), true),
            ("Home", Some("pass\nword1"), false),
            ("Ho\tme", None, false),
            (&"é".repeat(16), None, true),
            (&"é".repeat(17), None, false),
        ] {
            assert_eq!(validate(ssid, pw).is_none(), ok, "{ssid} {pw:?}");
        }
    }

    #[test]
    fn nmcli_escapes_are_undone() {
        assert_eq!(unescape("Cafe\\\\Bar"), "Cafe\\Bar");
        assert_eq!(unescape("A\\:B"), "A:B");
        assert_eq!(unescape("plain"), "plain");
        assert_eq!(unescape("end\\"), "end\\");
    }

    /// QA-NET-9: the kind of security, so an 802.1X network is never offered a password field.
    #[test]
    fn security_kinds_are_read_from_nmcli() {
        for (sec, want) in [
            ("", Security::Open),
            ("--", Security::Open),
            ("WPA2", Security::Psk),
            ("WPA1 WPA2", Security::Psk),
            ("WPA3", Security::Psk),
            ("WEP", Security::Psk),
            ("WPA2 802.1X", Security::Enterprise),
            ("WPA3 802.1X", Security::Enterprise),
            ("OWE", Security::Owe),
        ] {
            assert_eq!(security_kind(sec), want, "{sec}");
        }
    }
}
