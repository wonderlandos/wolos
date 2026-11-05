// SPDX-License-Identifier: BSD-2-Clause
//! Magnet links the helper accepts: `magnet:?` with exactly one `xt=urn:btih:` (40 hex or 32
//! base32 digits), at most [`MAX_MAGNET`] bytes, optional `dn` (display name) and `tr` (trackers,
//! http, https or udp only, at most [`MAX_TRACKERS`], never this machine or the LAN:
//! [`crate::trackers::allowed`]). Anything else in the link is dropped, so a link cannot point the
//! helper at a web address to fetch (`xs`, `as`), at a local file or at a local service.

/// Longest magnet link accepted.
pub const MAX_MAGNET: usize = 8192;
/// Most trackers kept from one link.
pub const MAX_TRACKERS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Magnet {
    /// Info hash, 40 lowercase hex digits.
    pub id: String,
    /// Display name, checked as one path component, or None.
    pub name: Option<String>,
    pub trackers: Vec<String>,
}

impl Magnet {
    /// The link the engine gets: only the fields checked here.
    pub fn canonical(&self) -> String {
        let mut s = format!("magnet:?xt=urn:btih:{}", self.id);
        for t in &self.trackers {
            s.push_str("&tr=");
            s.push_str(&escape(t));
        }
        s
    }
}

fn unhex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn unescape(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let h = unhex(*b.get(i + 1)?)?;
                let l = unhex(*b.get(i + 2)?)?;
                out.push((h << 4) | l);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Some(out)
}

fn escape(s: &str) -> String {
    let mut o = String::new();
    for &c in s.as_bytes() {
        if c.is_ascii_alphanumeric() || b"-._~:/".contains(&c) {
            o.push(c as char);
        } else {
            o.push_str(&format!("%{c:02X}"));
        }
    }
    o
}

fn base32_to_hex(s: &str) -> Option<String> {
    let mut bits: u64 = 0;
    let mut n = 0;
    let mut out = Vec::new();
    for c in s.bytes() {
        let v = u64::from(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a',
            b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        });
        bits = (bits << 5) | v;
        n += 5;
        if n >= 8 {
            n -= 8;
            out.push(u8::try_from((bits >> n) & 0xff).unwrap_or(0));
        }
    }
    (out.len() == 20).then(|| out.iter().map(|b| format!("{b:02x}")).collect())
}

/// Check a magnet link.
pub fn parse(link: &str) -> Option<Magnet> {
    parse_with(link, false)
}

/// Check a magnet link; `allow_local` keeps loopback and LAN trackers (the tests' `--local-peers`).
pub fn parse_with(link: &str, allow_local: bool) -> Option<Magnet> {
    if link.len() > MAX_MAGNET {
        return None;
    }
    let q = link.strip_prefix("magnet:?")?;
    let mut id = None;
    let mut name = None;
    let mut trackers = Vec::new();
    for pair in q.split('&') {
        let (k, v) = pair.split_once('=')?;
        let v = String::from_utf8(unescape(v)?).ok()?;
        match k {
            "xt" => {
                let h = v.strip_prefix("urn:btih:")?;
                let hex = match h.len() {
                    40 if h.bytes().all(|c| c.is_ascii_hexdigit()) => h.to_ascii_lowercase(),
                    32 => base32_to_hex(h)?,
                    _ => return None,
                };
                if id.replace(hex).is_some() {
                    return None; // one torrent per link
                }
            }
            "dn" => name = crate::confine::component(v.as_bytes()).ok(),
            "tr" => {
                let ok = crate::trackers::allowed(&v, allow_local) && !trackers.contains(&v);
                if ok && trackers.len() < MAX_TRACKERS {
                    trackers.push(v);
                }
            }
            _ => {} // xs, as, ws, x.pe ... dropped
        }
    }
    Some(Magnet {
        id: id?,
        name,
        trackers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const H: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";

    #[test]
    fn a_plain_link() {
        let m = parse_with(
            &format!(
                "magnet:?xt=urn:btih:{}&dn=test+file.bin&tr=http%3A%2F%2F127.0.0.1%3A6969%2Fannounce",
                H.to_uppercase()
            ),
            true,
        )
        .unwrap();
        assert_eq!(m.id, H);
        assert_eq!(m.name.as_deref(), Some("test file.bin"));
        assert_eq!(m.trackers, vec!["http://127.0.0.1:6969/announce"]);
        assert_eq!(
            m.canonical(),
            format!("magnet:?xt=urn:btih:{H}&tr=http://127.0.0.1:6969/announce")
        );
    }

    #[test]
    fn base32_hashes_are_read_as_hex() {
        // 20 zero bytes in base32
        let m = parse("magnet:?xt=urn:btih:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
        assert_eq!(m.id, "0".repeat(40));
    }

    #[test]
    fn links_the_helper_would_not_take_are_refused() {
        assert!(parse("http://x/y.torrent").is_none());
        assert!(parse("magnet:?dn=x").is_none());
        assert!(parse(&format!("magnet:?xt=urn:btih:{H}&xt=urn:btih:{H}")).is_none());
        assert!(parse("magnet:?xt=urn:btih:zz").is_none());
        assert!(parse(&format!("magnet:?xt=urn:btih:{H}&dn=%zz")).is_none());
        assert!(parse(&format!("magnet:?xt=urn:btih:{H}&{}", "a=b&".repeat(3000))).is_none());
    }

    #[test]
    fn local_trackers_are_dropped() {
        // DL-SEC-03: a link may not point the helper at this machine or the LAN
        let m = parse(&format!(
            "magnet:?xt=urn:btih:{H}&tr=http%3A%2F%2F127.0.0.1%3A631%2F&tr=http://192.168.1.1/a&tr=udp://tracker.example.org:1337"
        ))
        .unwrap();
        assert_eq!(m.trackers, vec!["udp://tracker.example.org:1337"]);
        assert_eq!(
            parse_with(
                &format!("magnet:?xt=urn:btih:{H}&tr=http://127.0.0.1:631/"),
                true
            )
            .unwrap()
            .trackers,
            vec!["http://127.0.0.1:631/"]
        );
    }

    #[test]
    fn unsafe_parts_are_dropped() {
        let m = parse(&format!("magnet:?xt=urn:btih:{H}&dn=..&tr=file:///etc/passwd&xs=http://evil/x.torrent&tr=udp://t.example.org:1")).unwrap();
        assert_eq!(m.name, None);
        assert_eq!(m.trackers, vec!["udp://t.example.org:1"]);
        assert!(!m.canonical().contains("evil"));
    }
}
