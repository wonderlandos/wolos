//! Wi-Fi: wpa_supplicant.conf editing and input validation (to be replaced by NetworkManager, see archicture/modules/wifi.md).
pub const CONF_PATH: &str = "/etc/wpa_supplicant.conf";

/// dBm to 0..100 (-100 dBm = 0, -50 dBm and above = 100).
pub fn signal_percent(dbm: i32) -> i32 {
    (2 * (dbm + 100)).clamp(0, 100)
}

/// Error text, or None when wpa_supplicant can take these (quoted, or hex for names and passwords
/// with `"`, `\\`, `#` or `}`). None password = open network. 802.11: a name is 1 to 32 bytes.
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
    (!(8..=63).contains(&p.chars().count())).then_some("Password must be 8 to 63 characters")
}

/// Safe between wpa_supplicant.conf quotes: its reader has no escapes, ends a value at the last
/// `"`, strips `#` comments by counting quotes and the block parser here ends at `}`.
fn quotable(s: &str) -> bool {
    !s.contains(['"', '\\', '#', '}'])
}

/// `ssid=` value: `"Home"`, or hex bytes when the name is not [`quotable`].
pub fn ssid_value(ssid: &str) -> String {
    if quotable(ssid) {
        format!("\"{ssid}\"")
    } else {
        hex(ssid.as_bytes())
    }
}

/// `psk=` value: the quoted passphrase, or the 256-bit key wpa_supplicant derives from it
/// (PBKDF2-HMAC-SHA1, 4096 rounds, salt = name) when the passphrase is not [`quotable`].
pub fn psk_value(ssid: &str, password: &str) -> String {
    if quotable(password) {
        format!("\"{password}\"")
    } else {
        hex(&psk(password.as_bytes(), ssid.as_bytes()))
    }
}

/// A name as `wpa_cli` prints it (printf_encode): `\\"`, `\\\\`, `\\xNN` for bytes outside ASCII.
pub fn wpa_txt(ssid: &str) -> String {
    let mut s = String::new();
    for b in ssid.bytes() {
        match b {
            b'"' => s.push_str("\\\""),
            b'\\' => s.push_str("\\\\"),
            0x1b => s.push_str("\\e"),
            b'\n' => s.push_str("\\n"),
            b'\r' => s.push_str("\\r"),
            b'\t' => s.push_str("\\t"),
            32..=126 => s.push(b as char),
            _ => s.push_str(&format!("\\x{b:02x}")),
        }
    }
    s
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// SHA-1 (FIPS 180-4): only for the WPA passphrase-to-key step, which the standard fixes.
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    let mut m = data.to_vec();
    m.push(0x80);
    while m.len() % 64 != 56 {
        m.push(0);
    }
    m.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in m.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i / 20 {
                0 => ((b & c) | (!b & d), 0x5a82_7999),
                1 => (b ^ c ^ d, 0x6ed9_eba1),
                2 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            (e, d, c, b, a) = (d, c, b.rotate_left(30), a, t);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (o, x) in out.chunks_mut(4).zip(h) {
        o.copy_from_slice(&x.to_be_bytes());
    }
    out
}

fn hmac_sha1(key: &[u8], msg: &[u8]) -> [u8; 20] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..20].copy_from_slice(&sha1(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let pad = |x: u8| k.iter().map(|b| b ^ x).collect::<Vec<u8>>();
    let mut inner = pad(0x36);
    inner.extend_from_slice(msg);
    let mut outer = pad(0x5c);
    outer.extend_from_slice(&sha1(&inner));
    sha1(&outer)
}

/// IEEE 802.11i passphrase-to-PSK: PBKDF2-HMAC-SHA1(passphrase, ssid, 4096, 32 bytes).
fn psk(passphrase: &[u8], ssid: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (block, o) in out.chunks_mut(20).enumerate() {
        let mut salt = ssid.to_vec();
        salt.extend_from_slice(&(block as u32 + 1).to_be_bytes());
        let mut u = hmac_sha1(passphrase, &salt);
        let mut t = u;
        for _ in 1..4096 {
            u = hmac_sha1(passphrase, &u);
            for (x, y) in t.iter_mut().zip(u) {
                *x ^= y;
            }
        }
        o.copy_from_slice(&t[..o.len()]);
    }
    out
}

/// Where joining `ssid` stands after wpa_supplicant reloaded its networks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinState {
    Joined,
    /// wpa_supplicant gave up on the network for now (`[TEMP-DISABLED]`: wrong password, the
    /// access point refused us).
    Refused,
    /// Scanning, associating or in the handshake.
    Trying,
}

/// `wpa_cli status` + `wpa_cli list_networks` -> state of `ssid`.
pub fn join_state(status: &str, networks: &str, ssid: &str) -> JoinState {
    let shown = wpa_txt(ssid);
    if crate::controls::parse_wpa_status(status).as_deref() == Some(shown.as_str()) {
        return JoinState::Joined;
    }
    let refused = networks.lines().skip(1).any(|l| {
        let f: Vec<&str> = l.split('\t').collect();
        f.get(1) == Some(&shown.as_str()) && f.get(3).is_some_and(|fl| fl.contains("DISABLED]"))
    });
    if refused {
        JoinState::Refused
    } else {
        JoinState::Trying
    }
}

/// Replace (or add) the `network={}` block for `ssid`; header and other networks kept.
pub fn upsert_network(conf: Option<&str>, ssid: &str, password: Option<&str>) -> String {
    upsert_network_with(conf, ssid, password, false)
}

/// [`upsert_network`]; `hidden`: the network does not announce its name, so wpa_supplicant asks
/// for it by name (`scan_ssid=1`, only for networks joined as hidden: it tells nearby radios).
pub fn upsert_network_with(
    conf: Option<&str>,
    ssid: &str,
    password: Option<&str>,
    hidden: bool,
) -> String {
    let text = match conf {
        Some(c) if !c.trim().is_empty() => c,
        _ => "ctrl_interface=/var/run/wpa_supplicant\nctrl_interface_group=wheel\n",
    };
    let mut out = String::new();
    let mut i = 0;
    let needle = format!("ssid={}\n", ssid_value(ssid));
    while i < text.len() {
        let Some(rel) = text[i..].find("network={") else {
            out.push_str(&text[i..]);
            break;
        };
        let start = i + rel;
        let mut end = text[start..]
            .find('}')
            .map_or(text.len(), |e| start + e + 1);
        if text[end..].starts_with('\n') {
            end += 1;
        }
        out.push_str(&text[i..start]);
        let block = &text[start..end];
        if !block.contains(&needle) {
            out.push_str(block);
        }
        i = end;
    }
    let mut s = out.trim_end_matches('\n').to_owned();
    s.push_str("\n\n");
    s.push_str(&format!("network={{\n\tssid={}\n", ssid_value(ssid)));
    if hidden {
        s.push_str("\tscan_ssid=1\n");
    }
    match password {
        None => s.push_str("\tkey_mgmt=NONE\n"),
        Some(p) => s.push_str(&format!("\tpsk={}\n", psk_value(ssid, p))),
    }
    s.push_str("\tpriority=10\n}\n");
    s
}

/// Drop the `network={}` block for `ssid`; header and other networks kept byte for byte.
pub fn remove_network(conf: Option<&str>, ssid: &str) -> String {
    let text = conf.unwrap_or_default();
    let needle = format!("ssid={}\n", ssid_value(ssid));
    let mut out = String::new();
    let mut i = 0;
    while i < text.len() {
        let Some(rel) = text[i..].find("network={") else {
            out.push_str(&text[i..]);
            break;
        };
        let start = i + rel;
        let mut end = text[start..]
            .find('}')
            .map_or(text.len(), |e| start + e + 1);
        if text[end..].starts_with('\n') {
            end += 1;
        }
        out.push_str(&text[i..start]);
        if !text[start..end].contains(&needle) {
            out.push_str(&text[start..end]);
        }
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_table() {
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
        assert_eq!(
            (
                signal_percent(-100),
                signal_percent(-75),
                signal_percent(-40)
            ),
            (0, 50, 100)
        );
    }

    /// After `wpa_cli reconfigure`: joined, refused (wrong password) or still trying.
    #[test]
    fn join_state_table() {
        const NETS: &str = "network id / ssid / bssid / flags\n0\tHome\tany\t[CURRENT]\n1\tCafe\tany\t[TEMP-DISABLED]\n2\tWork\tany\t[DISABLED]\n3\tLib\tany\t\n";
        for (status, ssid, want) in [
            (
                "wpa_state=COMPLETED\nssid=Home\n",
                "Home",
                JoinState::Joined,
            ),
            ("wpa_state=COMPLETED\nssid=Home\n", "Lib", JoinState::Trying),
            ("wpa_state=SCANNING\n", "Cafe", JoinState::Refused),
            ("wpa_state=DISCONNECTED\n", "Work", JoinState::Refused),
            (
                "wpa_state=4WAY_HANDSHAKE\nssid=Lib\n",
                "Lib",
                JoinState::Trying,
            ),
            ("wpa_state=SCANNING\n", "Gone", JoinState::Trying),
        ] {
            assert_eq!(join_state(status, NETS, ssid), want, "{status} {ssid}");
        }
    }

    #[test]
    fn upsert_replaces_only_that_network() {
        let conf = "ctrl_interface=/var/run/wpa_supplicant\n\nnetwork={\n\tssid=\"Home\"\n\tpsk=\"old\"\n}\nnetwork={\n\tssid=\"Work\"\n\tkey_mgmt=NONE\n}\n";
        let s = upsert_network(Some(conf), "Home", Some("newpassword"));
        assert!(s.starts_with("ctrl_interface"));
        assert!(!s.contains("old"));
        assert!(s.contains("ssid=\"Work\""));
        assert_eq!(s.matches("ssid=\"Home\"").count(), 1);
        assert!(s.ends_with("\tpsk=\"newpassword\"\n\tpriority=10\n}\n"));
        assert!(upsert_network(None, "Cafe", None).contains(
            "ctrl_interface_group=wheel\n\nnetwork={\n\tssid=\"Cafe\"\n\tkey_mgmt=NONE\n"
        ));
    }

    /// Names and passwords with quotes, backslashes, `#` or `}` (wpa_supplicant.conf has no
    /// escapes): hex name, derived key. Vectors: FIPS 180 SHA-1, IEEE 802.11i-2004 H.4.
    #[test]
    fn awkward_names_and_passwords_are_written_safely() {
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&sha1(&[b'a'; 1000])),
            "291e9a6c66994949b57ba5e650361e98fc36b1ba"
        );
        assert_eq!(
            psk_value("IEEE", "pass\"word"),
            hex(&psk(b"pass\"word", b"IEEE"))
        );
        assert_eq!(
            hex(&psk(b"password", b"IEEE")),
            "f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e"
        );
        assert_eq!(
            hex(&psk(b"ThisIsAPassword", b"ThisIsASSID")),
            "0dc0d6eb90555ed6419756b9a15ec3e3209b63df707dd508d14581f8982721af"
        );
        assert_eq!(psk_value("Home", "password1"), "\"password1\"");
        assert_eq!(ssid_value("Home"), "\"Home\"");
        assert_eq!(ssid_value("Café"), "\"Café\"");
        assert_eq!(ssid_value("Bob\"s"), "426f622273");
        let s = upsert_network(None, "Bob\"s", Some("pa\"ss#}word"));
        assert!(s.contains("\tssid=426f622273\n\tpsk="), "{s}");
        let psk_line = s.lines().find(|l| l.starts_with("\tpsk=")).unwrap();
        assert_eq!(psk_line.len(), "\tpsk=".len() + 64, "{psk_line}");
        assert!(
            !s.contains("pa\"ss"),
            "no passphrase in the file when it is not quotable"
        );
        assert_eq!(
            remove_network(Some(&s), "Bob\"s")
                .matches("network={")
                .count(),
            0
        );
        assert_eq!(
            upsert_network(Some(&s), "Bob\"s", None)
                .matches("network={")
                .count(),
            1
        );
        assert_eq!(wpa_txt("Bob\"s \\ Café"), "Bob\\\"s \\\\ Caf\\xc3\\xa9");
        let nets = "network id / ssid / bssid / flags\n0\tCaf\\xc3\\xa9\tany\t[TEMP-DISABLED]\n";
        assert_eq!(
            join_state("wpa_state=SCANNING\n", nets, "Café"),
            JoinState::Refused
        );
        assert_eq!(
            join_state("wpa_state=COMPLETED\nssid=Bob\\\"s\n", "", "Bob\"s"),
            JoinState::Joined
        );
    }

    #[test]
    fn hidden_networks_are_asked_for_by_name() {
        let s = upsert_network_with(None, "Attic", Some("password1"), true);
        assert!(s.ends_with("network={\n\tssid=\"Attic\"\n\tscan_ssid=1\n\tpsk=\"password1\"\n\tpriority=10\n}\n"), "{s}");
        assert!(!upsert_network(None, "Attic", None).contains("scan_ssid"));
    }

    #[test]
    fn remove_drops_only_that_network() {
        let conf = "ctrl_interface=/var/run/wpa_supplicant\nctrl_interface_group=wheel\n\nnetwork={\n\tssid=\"Home\"\n\tpsk=\"secret\"\n}\nnetwork={\n\tssid=\"Work\"\n\tkey_mgmt=NONE\n}\n";
        assert_eq!(
            remove_network(Some(conf), "Home"),
            "ctrl_interface=/var/run/wpa_supplicant\nctrl_interface_group=wheel\n\nnetwork={\n\tssid=\"Work\"\n\tkey_mgmt=NONE\n}\n"
        );
        assert_eq!(remove_network(Some(conf), "Nope"), conf);
        assert_eq!(remove_network(Some(conf), "Hom"), conf, "whole names only");
        assert_eq!(remove_network(None, "Home"), "");
    }
}
