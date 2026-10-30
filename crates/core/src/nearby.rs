//! Nearby sharing discovery records: the mDNS instance name and the `n` TXT value a phone reads
//! to show this computer in its share sheet. Pure encode/decode; wold advertises them.

/// DNS-SD service type the share sheet browses for.
pub const SERVICE_TYPE: &str = "_FC9F5ED42C8A._tcp";
const PCP: u8 = 0x23;
const SERVICE_HASH: [u8; 3] = [0xFC, 0x9F, 0x5E];

/// What kind of device the record describes (drives the icon on the phone).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Unknown = 0,
    Phone = 1,
    Tablet = 2,
    Laptop = 3,
}

/// Endpoint ids are 4 ASCII alphanumerics.
pub fn is_valid_endpoint_id(id: &str) -> bool {
    id.len() == 4 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// mDNS instance name for `endpoint_id`; None when the id is invalid.
pub fn instance_name(endpoint_id: &str) -> Option<String> {
    if !is_valid_endpoint_id(endpoint_id) {
        return None;
    }
    let mut b = vec![PCP];
    b.extend_from_slice(endpoint_id.as_bytes());
    b.extend_from_slice(&SERVICE_HASH);
    b.extend_from_slice(&[0, 0]);
    Some(b64url_encode(&b))
}

/// Endpoint id carried by an instance name; None when it is not a share-sheet record.
pub fn parse_instance_name(name: &str) -> Option<String> {
    let b = b64url_decode(name)?;
    if b.len() < 8 || b[0] != PCP || b[5..8] != SERVICE_HASH {
        return None;
    }
    let id = std::str::from_utf8(&b[1..5]).ok()?;
    is_valid_endpoint_id(id).then(|| id.to_string())
}

/// Decoded endpoint info (the `n` TXT value).
#[derive(Debug, PartialEq, Eq)]
pub struct EndpointInfo {
    pub kind: DeviceKind,
    pub visible: bool,
    pub name: String,
}

/// `n` TXT value: flags byte, 16 salt bytes, name length, UTF-8 name (at most 255 bytes).
pub fn encode_endpoint_info(info: &EndpointInfo, salt: [u8; 16]) -> Option<String> {
    let name = info.name.as_bytes();
    if name.is_empty() || name.len() > 255 {
        return None;
    }
    let hidden = if info.visible { 0 } else { 1 };
    let mut b = vec![(hidden << 4) | ((info.kind as u8) << 1)];
    b.extend_from_slice(&salt);
    b.push(name.len() as u8);
    b.extend_from_slice(name);
    Some(b64url_encode(&b))
}

pub fn decode_endpoint_info(txt: &str) -> Option<EndpointInfo> {
    let b = b64url_decode(txt)?;
    let len = *b.get(17)? as usize;
    let name = std::str::from_utf8(b.get(18..18 + len)?).ok()?.to_string();
    let kind = match (b[0] >> 1) & 0x07 {
        1 => DeviceKind::Phone,
        2 => DeviceKind::Tablet,
        3 => DeviceKind::Laptop,
        _ => DeviceKind::Unknown,
    };
    Some(EndpointInfo {
        kind,
        visible: b[0] & 0x10 == 0,
        name,
    })
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// URL-safe base64 without padding.
fn b64url_encode(data: &[u8]) -> String {
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..=c.len() {
            out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_end_matches('=');
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for ch in s.bytes() {
        let v = ALPHABET.iter().position(|&a| a == ch)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_name_round_trips_endpoint_id() {
        let n = instance_name("Ab12").unwrap();
        assert_eq!(parse_instance_name(&n).as_deref(), Some("Ab12"));
        assert!(!n.contains(['+', '/', '=']));
    }

    #[test]
    fn instance_name_layout_is_fixed() {
        // 0x23 'W' 'O' 'L' '1' FC 9F 5E 00 00
        assert_eq!(instance_name("WOL1").unwrap(), "I1dPTDH8n14AAA");
    }

    #[test]
    fn rejects_bad_ids_and_foreign_names() {
        assert_eq!(instance_name("abc"), None);
        assert_eq!(instance_name("ab-1"), None);
        assert_eq!(parse_instance_name("aGVsbG8gd29ybGQ"), None);
        assert_eq!(parse_instance_name("not base64!"), None);
    }

    #[test]
    fn endpoint_info_round_trips() {
        let info = EndpointInfo {
            kind: DeviceKind::Laptop,
            visible: true,
            name: "Julia’s WolBook".into(),
        };
        let txt = encode_endpoint_info(&info, [7; 16]).unwrap();
        assert_eq!(decode_endpoint_info(&txt), Some(info));
    }

    #[test]
    fn endpoint_info_flags_byte() {
        let info = EndpointInfo {
            kind: DeviceKind::Laptop,
            visible: false,
            name: "x".into(),
        };
        let raw = b64url_decode(&encode_endpoint_info(&info, [0; 16]).unwrap()).unwrap();
        assert_eq!(raw[0], 0x16);
        assert_eq!(raw[17], 1);
    }

    #[test]
    fn endpoint_info_rejects_empty_long_and_truncated() {
        let mk = |name: String| EndpointInfo {
            kind: DeviceKind::Unknown,
            visible: true,
            name,
        };
        assert_eq!(encode_endpoint_info(&mk(String::new()), [0; 16]), None);
        assert_eq!(encode_endpoint_info(&mk("a".repeat(256)), [0; 16]), None);
        assert_eq!(decode_endpoint_info("AAAA"), None);
    }
}
