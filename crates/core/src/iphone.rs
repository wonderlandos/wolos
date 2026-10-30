//! iPhone attach: the usbmuxd `ListDevices` exchange and what a newly plugged phone shows.
//! Pure: callers own the socket and the notification. Wire: 16-byte little-endian header
//! (length, version 1, message type 8 = plist, tag) then an XML plist body.

/// usbmuxd's socket.
pub const SOCKET: &str = "/var/run/usbmuxd";

/// What the notice for a newly connected phone says.
pub const NOTICE_TITLE: &str = "iPhone connected";
pub const NOTICE_BODY: &str = "Trust on the phone to see photos";

/// A phone usbmuxd lists.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Phone {
    pub serial: String,
}

/// The `ListDevices` request, framed with `tag`.
pub fn list_request(tag: u32) -> Vec<u8> {
    let body = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<plist version=\"1.0\"><dict>",
        "<key>MessageType</key><string>ListDevices</string>",
        "<key>ClientVersionString</key><string>wolos</string>",
        "<key>ProgName</key><string>wolos</string>",
        "</dict></plist>\n"
    );
    let mut out = Vec::with_capacity(16 + body.len());
    for v in [16 + body.len() as u32, 1, 8, tag] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(body.as_bytes());
    out
}

/// Total length a reply announces (its first header word), once 4 bytes are in.
pub fn reply_len(head: &[u8]) -> Option<usize> {
    let n = u32::from_le_bytes(head.get(..4)?.try_into().ok()?) as usize;
    (16..=1 << 20).contains(&n).then_some(n)
}

/// Serial numbers in a `ListDevices` reply (header included). USB devices only; a malformed or
/// short reply lists nothing.
pub fn parse_devices(reply: &[u8]) -> Vec<Phone> {
    let Some(body) = reply.get(16..).and_then(|b| std::str::from_utf8(b).ok()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dev in body.split("<key>DeviceID</key>").skip(1) {
        let dev = dev.split("</dict>").next().unwrap_or("");
        let usb = dev.contains("<string>USB</string>");
        let serial = dev
            .split("<key>SerialNumber</key>")
            .nth(1)
            .and_then(|s| s.split("<string>").nth(1))
            .and_then(|s| s.split("</string>").next())
            .map(str::trim)
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 64
                    && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            });
        if let (true, Some(s)) = (usb, serial) {
            out.push(Phone { serial: s.into() });
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Phones in `now` that were not in `before`: each shows the notice once.
pub fn arrived(before: &[Phone], now: &[Phone]) -> Vec<Phone> {
    now.iter()
        .filter(|p| !before.contains(p))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(devs: &str) -> Vec<u8> {
        let body = format!(
            "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>DeviceList</key><array>{devs}</array></dict></plist>"
        );
        let mut v = vec![0u8; 16];
        v.extend_from_slice(body.as_bytes());
        v
    }

    fn dev(id: u32, conn: &str, serial: &str) -> String {
        format!(
            "<dict><key>DeviceID</key><integer>{id}</integer><key>MessageType</key><string>Attached</string><key>Properties</key><dict><key>ConnectionType</key><string>{conn}</string><key>SerialNumber</key><string>{serial}</string></dict></dict>"
        )
    }

    #[test]
    fn request_is_framed_for_usbmuxd() {
        let r = list_request(7);
        assert_eq!(
            u32::from_le_bytes(r[..4].try_into().unwrap()) as usize,
            r.len()
        );
        assert_eq!(u32::from_le_bytes(r[4..8].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(r[8..12].try_into().unwrap()), 8);
        assert_eq!(u32::from_le_bytes(r[12..16].try_into().unwrap()), 7);
        assert!(String::from_utf8_lossy(&r[16..]).contains("ListDevices"));
        assert_eq!(reply_len(&r), Some(r.len()));
        assert_eq!(reply_len(&[1, 0]), None);
        assert_eq!(reply_len(&[0xff, 0xff, 0xff, 0xff]), None);
    }

    #[test]
    fn lists_usb_phones_only() {
        let r = reply(
            &(dev(1, "USB", "00008110-001A")
                + &dev(2, "Network", "AAAA")
                + &dev(3, "USB", "bad<serial")),
        );
        assert_eq!(
            parse_devices(&r),
            [Phone {
                serial: "00008110-001A".into()
            }]
        );
        assert!(parse_devices(&reply("")).is_empty());
        assert!(parse_devices(b"short").is_empty());
    }

    #[test]
    fn the_notice_shows_once_per_plug() {
        let a = Phone { serial: "A".into() };
        let b = Phone { serial: "B".into() };
        assert_eq!(arrived(&[], &[a.clone()]), [a.clone()]);
        assert!(
            arrived(&[a.clone()], &[a.clone()]).is_empty(),
            "still plugged"
        );
        assert_eq!(arrived(&[a.clone()], &[a.clone(), b.clone()]), [b]);
        assert!(arrived(&[a], &[]).is_empty(), "unplugging says nothing");
        assert_eq!(NOTICE_TITLE, "iPhone connected");
        assert!(NOTICE_BODY.contains("Trust"));
    }
}
