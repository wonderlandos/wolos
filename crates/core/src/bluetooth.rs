//! Bluetooth for Settings: FreeBSD netgraph stack (hccontrol, hcsecd.conf), bluetoothctl on the Linux dev host.
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

pub const NODE: &str = "ubt0hci";
pub const HCSECD_CONF: &str = "/etc/bluetooth/hcsecd.conf";

/// Six hex pairs separated by ':'.
pub fn is_valid_addr(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Class of Device: major class in bits 8..12; peripheral minor bit 6 keyboard, bit 7 pointing.
pub fn kind_of_class(cod: u32) -> BtKind {
    match (cod >> 8) & 0x1f {
        0x01 => BtKind::Computer,
        0x02 => BtKind::Phone,
        0x04 => BtKind::Audio,
        0x05 if cod & 0x40 != 0 => BtKind::Keyboard,
        0x05 if cod & 0x80 != 0 => BtKind::Mouse,
        _ => BtKind::Other,
    }
}

/// "Class: 00:25:40" (hccontrol) or "0x002540" -> 0x002540.
fn parse_class(v: &str) -> Option<u32> {
    let v = v.trim();
    match v.strip_prefix("0x") {
        Some(h) => u32::from_str_radix(h, 16).ok(),
        None => u32::from_str_radix(&v.replace(':', ""), 16).ok(),
    }
}

fn value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.trim().strip_prefix(key).map(str::trim)
}

/// `hccontrol -n ubt0hci inquiry`: (address, class of device), first sighting wins.
pub fn parse_inquiry(text: &str) -> Vec<(String, u32)> {
    let mut v: Vec<(String, u32)> = Vec::new();
    let mut addr: Option<String> = None;
    for l in text.lines() {
        if let Some(a) = value(l, "BD_ADDR:") {
            addr = Some(a.to_ascii_lowercase()).filter(|a| is_valid_addr(a));
        } else if let Some(c) = value(l, "Class:").and_then(parse_class)
            && let Some(a) = addr.take()
            && !v.iter().any(|x| x.0 == a)
        {
            v.push((a, c));
        }
    }
    v
}

/// `hccontrol -n ubt0hci remote_name_request <addr>`: "Name: Magic Keyboard".
pub fn parse_remote_name(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| value(l, "Name:"))
        .filter(|n| !n.is_empty())
        .map(String::from)
}

/// `hccontrol -n ubt0hci read_connection_list`: remote addresses of open links.
pub fn parse_connections(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_ascii_lowercase)
        .filter(|a| is_valid_addr(a))
        .collect()
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

/// Adapter, power and paired devices (fast; no inquiry).
pub fn read(h: &dyn Host) -> BtState {
    if h.is_freebsd() {
        if h.run(&["hccontrol", "-n", NODE, "read_bd_addr"]).is_none() {
            return BtState::default();
        }
        let conns = h
            .run(&["hccontrol", "-n", NODE, "read_connection_list"])
            .map(|t| parse_connections(&t))
            .unwrap_or_default();
        let devices = h
            .read_text(HCSECD_CONF)
            .map(|t| parse_hcsecd(&t))
            .unwrap_or_default()
            .into_iter()
            .map(|d| BtDevice {
                connected: conns.contains(&d.addr),
                ..d
            })
            .collect();
        return BtState {
            present: true,
            powered: true,
            devices,
        };
    }
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

/// Nearby devices (slow: inquiry takes ~10 s; call off the UI thread). Nameless devices show their address.
pub fn scan(h: &dyn Host) -> Vec<BtDevice> {
    if h.is_freebsd() {
        return h
            .run_slow(&["hccontrol", "-n", NODE, "inquiry"])
            .map(|t| parse_inquiry(&t))
            .unwrap_or_default()
            .into_iter()
            .map(|(addr, cod)| BtDevice {
                name: h
                    .run(&["hccontrol", "-n", NODE, "remote_name_request", &addr])
                    .and_then(|t| parse_remote_name(&t))
                    .unwrap_or_else(|| addr.clone()),
                kind: kind_of_class(cod),
                addr,
                ..Default::default()
            })
            .collect();
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

    const INQUIRY: &str = "Inquiry result, num_responses=2
Inquiry result #0
\tBD_ADDR: 00:1E:52:7A:2B:3C
\tPage Scan Rep. Mode: 0x1
\tPage Scan Period Mode: 00
\tPage Scan Mode: 00
\tClass: 00:25:40
\tClock offset: 0x78ef
Inquiry result #1
\tBD_ADDR: 3c:58:c2:aa:bb:cc
\tPage Scan Rep. Mode: 0x1
\tClass: 0x5a020c
\tClock offset: 0x1234
Inquiry result, num_responses=1
Inquiry result #0
\tBD_ADDR: 00:1e:52:7a:2b:3c
\tClass: 00:25:40
Inquiry complete. Status: No error [00]
";

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
        for (cod, k) in [
            (0x002540, BtKind::Keyboard),
            (0x002580, BtKind::Mouse),
            (0x240404, BtKind::Audio),
            (0x5a020c, BtKind::Phone),
            (0x7a010c, BtKind::Computer),
            (0x000000, BtKind::Other),
        ] {
            assert_eq!(kind_of_class(cod), k, "{cod:#x}");
        }
    }

    #[test]
    fn hccontrol_outputs() {
        assert_eq!(
            parse_inquiry(INQUIRY),
            [
                ("00:1e:52:7a:2b:3c".to_string(), 0x002540),
                ("3c:58:c2:aa:bb:cc".to_string(), 0x5a020c)
            ]
        );
        assert_eq!(
            parse_remote_name("BD_ADDR: 00:1e:52:7a:2b:3c\nName: Magic Keyboard\n").as_deref(),
            Some("Magic Keyboard")
        );
        assert_eq!(
            parse_remote_name("hccontrol: Could not execute command\n"),
            None
        );
        let conns = "Remote BD_ADDR    Handle Type Mode Role Encrypt Pending Queue State\n00:1E:52:7A:2B:3C     41  ACL    0 MAST    NONE       0     0 OPEN\n";
        assert_eq!(parse_connections(conns), ["00:1e:52:7a:2b:3c"]);
        assert!(parse_connections("Remote BD_ADDR    Handle Type\n").is_empty());
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
    fn reads_freebsd_stack() {
        let h = FakeHost::new(true);
        assert_eq!(read(&h), BtState::default(), "no adapter: nothing");
        h.command("hccontrol -n ubt0hci read_bd_addr", "BD_ADDR: 00:1a:7d:da:71:13\n")
            .command(
                "hccontrol -n ubt0hci read_connection_list",
                "Remote BD_ADDR    Handle Type Mode Role Encrypt Pending Queue State\n00:1e:52:7a:2b:3c     41  ACL    0 MAST    NONE       0     0 OPEN\n",
            )
            .file(HCSECD_CONF, HCSECD);
        let s = read(&h);
        assert!(s.present && s.powered);
        assert_eq!(s.devices.len(), 2);
        assert!(s.devices[0].connected && !s.devices[1].connected);
        h.slow("hccontrol -n ubt0hci inquiry", INQUIRY).command(
            "hccontrol -n ubt0hci remote_name_request 00:1e:52:7a:2b:3c",
            "Name: Magic Keyboard\n",
        );
        let found = scan(&h);
        assert_eq!(found.len(), 2);
        assert_eq!(
            (found[0].name.as_str(), found[0].kind, found[0].paired),
            ("Magic Keyboard", BtKind::Keyboard, false)
        );
        assert_eq!(found[1].name, "3c:58:c2:aa:bb:cc", "nameless: address");
    }

    #[test]
    fn reads_bluetoothctl() {
        let h = FakeHost::new(false);
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
    }
}
