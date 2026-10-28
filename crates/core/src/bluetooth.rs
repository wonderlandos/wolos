//! Bluetooth for Settings: BlueZ through bluetoothctl (no root: the person at the computer may
//! pair, connect and forget). The hcsecd.conf helpers stay until wol-admind drops `bt-pair`.
use crate::system::Host;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BtKind {
    #[default]
    Other,
    Keyboard,
    Mouse,
    Audio,
    Phone,
    Computer,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BtDevice {
    /// "00:11:22:33:44:55", lowercase.
    pub addr: String,
    pub name: String,
    pub kind: BtKind,
    pub paired: bool,
    pub connected: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BtState {
    pub present: bool,
    pub powered: bool,
    pub devices: Vec<BtDevice>,
}

/// Transitional (ADR 0016): the FreeBSD pairing file wol-admind `bt-pair`/`bt-forget` still write.
pub const HCSECD_CONF: &str = "/etc/bluetooth/hcsecd.conf";

/// Six hex pairs separated by ':'.
pub fn is_valid_addr(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// (start, end) of each `device { ... }` block, end past the closing brace and its newline.
fn blocks(text: &str) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let mut i = 0;
    while let Some(rel) = text[i..].find("device") {
        let start = i + rel;
        let line_start = text[..start].rfind('\n').map_or(0, |n| n + 1);
        let Some(open) = text[start..].find('{').map(|o| start + o) else {
            break;
        };
        let commented = text[line_start..start].contains('#');
        let Some(close) = text[open..].find('}').map(|c| open + c + 1) else {
            break;
        };
        let end = if text[close..].starts_with('\n') {
            close + 1
        } else {
            close
        };
        if !commented && text[start + 6..open].trim().is_empty() {
            v.push((line_start, end));
        }
        i = end;
    }
    v
}

/// One `key value;` entry of a device block.
fn field(block: &str, key: &str) -> Option<String> {
    block.lines().find_map(|l| {
        let l = l.trim().trim_end_matches(';');
        let (k, v) = l.split_once(char::is_whitespace)?;
        (k == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

/// hcsecd.conf: every `device {}` block is a paired device.
pub fn parse_hcsecd(text: &str) -> Vec<BtDevice> {
    blocks(text)
        .into_iter()
        .filter_map(|(s, e)| {
            let b = &text[s..e];
            let addr = field(b, "bdaddr")?.to_ascii_lowercase();
            is_valid_addr(&addr).then(|| BtDevice {
                name: field(b, "name").unwrap_or_else(|| addr.clone()),
                addr,
                paired: true,
                ..Default::default()
            })
        })
        .collect()
}

/// `conf` without the block for `addr` (case-insensitive); everything else byte for byte.
pub fn remove_hcsecd(conf: Option<&str>, addr: &str) -> String {
    let text = conf.unwrap_or_default();
    let mut out = String::new();
    let mut i = 0;
    for (s, e) in blocks(text) {
        let mine = field(&text[s..e], "bdaddr").is_some_and(|a| a.eq_ignore_ascii_case(addr));
        if mine {
            out.push_str(&text[i..s]);
            i = e;
        }
    }
    out.push_str(&text[i..]);
    out
}

/// Add or replace the block for `addr`: no stored link key, no PIN (Secure Simple Pairing).
pub fn upsert_hcsecd(conf: Option<&str>, addr: &str, name: &str) -> String {
    let mut s = remove_hcsecd(conf, addr).trim_end_matches('\n').to_string();
    if !s.is_empty() {
        s.push_str("\n\n");
    }
    let name: String = name
        .chars()
        .filter(|c| *c != '"' && !c.is_control())
        .collect();
    s.push_str(&format!(
        "device {{\n\tbdaddr\t{};\n\tname\t\"{name}\";\n\tkey\tnokey;\n\tpin\tnopin;\n}}\n",
        addr.to_ascii_lowercase()
    ));
    s
}

/// bluetoothctl `devices [Paired|Connected]`: "Device AA:BB:.. Name".
pub fn parse_bluetoothctl_devices(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Device ")?;
            let (addr, name) = rest.split_once(' ').unwrap_or((rest, ""));
            let addr = addr.to_ascii_lowercase();
            is_valid_addr(&addr).then(|| {
                let name = if name.trim().is_empty() {
                    addr.clone()
                } else {
                    name.trim().to_string()
                };
                (addr, name)
            })
        })
        .collect()
}

/// Adapter, power and paired devices (fast; no scan).
pub fn read(h: &dyn Host) -> BtState {
    let Some(show) = h.run(&["bluetoothctl", "show"]) else {
        return BtState::default();
    };
    let connected: Vec<String> = h
        .run(&["bluetoothctl", "devices", "Connected"])
        .map(|t| {
            parse_bluetoothctl_devices(&t)
                .into_iter()
                .map(|d| d.0)
                .collect()
        })
        .unwrap_or_default();
    BtState {
        present: show.contains("Controller"),
        powered: crate::controls::parse_bluetoothctl(&show).unwrap_or(false),
        devices: h
            .run(&["bluetoothctl", "devices", "Paired"])
            .map(|t| parse_bluetoothctl_devices(&t))
            .unwrap_or_default()
            .into_iter()
            .map(|(addr, name)| BtDevice {
                connected: connected.contains(&addr),
                addr,
                name,
                paired: true,
                ..Default::default()
            })
            .collect(),
    }
}

/// Nearby devices (slow: the scan takes ~8 s; call off the UI thread). Nameless devices show their address.
pub fn scan(h: &dyn Host) -> Vec<BtDevice> {
    let _ = h.run_slow(&["bluetoothctl", "--timeout", "8", "scan", "on"]);
    h.run(&["bluetoothctl", "devices"])
        .map(|t| parse_bluetoothctl_devices(&t))
        .unwrap_or_default()
        .into_iter()
        .map(|(addr, name)| BtDevice {
            addr,
            name,
            ..Default::default()
        })
        .collect()
}

/// Pair, trust (reconnects by itself) and connect; or remove. Fixed argv, address checked.
pub fn pair_argvs(addr: &str) -> Option<[[&str; 3]; 3]> {
    is_valid_addr(addr).then_some([
        ["bluetoothctl", "pair", addr],
        ["bluetoothctl", "trust", addr],
        ["bluetoothctl", "connect", addr],
    ])
}

pub fn forget_argv(addr: &str) -> Option<[&str; 3]> {
    is_valid_addr(addr).then_some(["bluetoothctl", "remove", addr])
}

/// Pair a device found by [`scan`] (slow: call off the UI thread).
pub fn pair(h: &dyn Host, addr: &str) -> bool {
    pair_argvs(addr).is_some_and(|steps| steps.iter().all(|a| h.run_slow(a).is_some()))
}

pub fn forget(h: &dyn Host, addr: &str) -> bool {
    forget_argv(addr).is_some_and(|a| h.run(&a).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

    const HCSECD: &str = "# $FreeBSD$\n#\n# hcsecd.conf\n#\ndevice {\n\tbdaddr\t00:1e:52:7a:2b:3c;\n\tname\t\"Magic Keyboard\";\n\tkey\tnokey;\n\tpin\t\"0000\";\n}\n\ndevice {\n\tbdaddr\t00:11:22:33:44:55;\n\tname\t\"Headphones\";\n\tkey\tnokey;\n\tpin\tnopin;\n}\n";

    #[test]
    fn addresses_and_classes() {
        for (a, ok) in [
            ("00:1e:52:7a:2b:3c", true),
            ("00:1E:52:7A:2B:3C", true),
            ("00:1e:52:7a:2b", false),
            ("00:1e:52:7a:2b:3g", false),
            ("00-1e-52-7a-2b-3c", false),
            ("00:1e:52:7a:2b:3c;", false),
            ("", false),
        ] {
            assert_eq!(is_valid_addr(a), ok, "{a}");
        }
    }

    #[test]
    fn hcsecd_parse_upsert_remove() {
        let d = parse_hcsecd(HCSECD);
        assert_eq!(
            d.iter()
                .map(|x| (x.addr.as_str(), x.name.as_str(), x.paired))
                .collect::<Vec<_>>(),
            [
                ("00:1e:52:7a:2b:3c", "Magic Keyboard", true),
                ("00:11:22:33:44:55", "Headphones", true)
            ]
        );
        let up = upsert_hcsecd(Some(HCSECD), "00:1E:52:7A:2B:3C", "Keyboard");
        assert!(up.starts_with("# $FreeBSD$\n"));
        assert_eq!(up.matches("bdaddr").count(), 2);
        assert!(up.contains("name\t\"Headphones\""), "other block untouched");
        let p = parse_hcsecd(&up);
        assert_eq!(p.iter().filter(|x| x.name == "Keyboard").count(), 1);
        assert!(!up.contains("Magic Keyboard"));
        assert!(up.contains("\tkey\tnokey;\n\tpin\tnopin;\n"));
        let rm = remove_hcsecd(Some(HCSECD), "00:11:22:33:44:55");
        assert_eq!(parse_hcsecd(&rm).len(), 1);
        assert!(rm.contains("Magic Keyboard") && !rm.contains("Headphones"));
        assert_eq!(remove_hcsecd(Some(HCSECD), "aa:aa:aa:aa:aa:aa"), HCSECD);
        let fresh = upsert_hcsecd(None, "00:11:22:33:44:55", "Mouse");
        assert_eq!(parse_hcsecd(&fresh)[0].name, "Mouse");
    }

    #[test]
    fn bluetoothctl_devices() {
        assert_eq!(
            parse_bluetoothctl_devices(
                "Device AA:BB:CC:DD:EE:FF Magic Mouse\nDevice 11:22:33:44:55:66 Buds\nController x\n"
            ),
            [
                ("aa:bb:cc:dd:ee:ff".to_string(), "Magic Mouse".to_string()),
                ("11:22:33:44:55:66".to_string(), "Buds".to_string())
            ]
        );
    }

    #[test]
    fn reads_bluetoothctl() {
        let h = FakeHost::linux();
        h.command(
            "bluetoothctl show",
            "Controller 00:1A:7D:DA:71:13 (public)\n\tPowered: no\n",
        )
        .command(
            "bluetoothctl devices Paired",
            "Device AA:BB:CC:DD:EE:FF Magic Mouse\n",
        )
        .command("bluetoothctl devices Connected", "")
        .slow("bluetoothctl --timeout 8 scan on", "Discovery started\n")
        .command("bluetoothctl devices", "Device 11:22:33:44:55:66 Buds\n");
        let s = read(&h);
        assert!(s.present && !s.powered);
        assert_eq!(s.devices[0].name, "Magic Mouse");
        assert!(s.devices[0].paired && !s.devices[0].connected);
        assert_eq!(scan(&h)[0].addr, "11:22:33:44:55:66");
        assert_eq!(read(&FakeHost::linux()), BtState::default(), "no BlueZ");
    }

    #[test]
    fn pair_and_forget() {
        let h = FakeHost::linux();
        h.slow(
            "bluetoothctl pair 11:22:33:44:55:66",
            "Pairing successful\n",
        )
        .slow("bluetoothctl trust 11:22:33:44:55:66", "trust succeeded\n")
        .command(
            "bluetoothctl remove 11:22:33:44:55:66",
            "Device has been removed\n",
        );
        assert!(!pair(&h, "11:22:33:44:55:66"), "connect failed");
        h.slow(
            "bluetoothctl connect 11:22:33:44:55:66",
            "Connection successful\n",
        );
        assert!(pair(&h, "11:22:33:44:55:66"));
        assert!(forget(&h, "11:22:33:44:55:66"));
        assert!(!pair(&h, "11:22:33:44:55:66; reboot"));
        assert_eq!(forget_argv("--help"), None);
    }
}
