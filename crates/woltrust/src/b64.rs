// SPDX-License-Identifier: BSD-2-Clause
//! Base64url without padding (RFC 4648 section 5), strict: one canonical text per byte string, so a
//! signature field cannot be written two ways. Decoding is bounded by the caller's `max`.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Encodes `data` as base64url, no padding.
pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..=chunk.len() {
            let idx = usize::try_from((n >> (18 - 6 * i)) & 63).unwrap_or(0);
            out.push(char::from(ALPHABET[idx]));
        }
    }
    out
}

fn value(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some(u32::from(c - b'A')),
        b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    }
}

/// Decodes canonical base64url; None for padding, other characters, a length that no byte string
/// encodes to, non-zero spare bits, or more than `max` decoded bytes.
pub fn decode(text: &str, max: usize) -> Option<Vec<u8>> {
    let t = text.as_bytes();
    if t.len() % 4 == 1 || t.len() / 4 * 3 > max.saturating_add(3) {
        return None;
    }
    let mut out = Vec::with_capacity(t.len() / 4 * 3 + 2);
    for chunk in t.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= value(*c)? << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        let keep = chunk.len() * 6 / 8;
        out.extend_from_slice(&bytes[1..=keep]);
        // The unused low bits of the last group must be zero.
        let spare = chunk.len() * 6 - keep * 8;
        if spare > 0 && (n >> (24 - chunk.len() * 6)) & ((1 << spare) - 1) != 0 {
            return None;
        }
    }
    (out.len() <= max).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_strings_round_trip() {
        for (raw, text) in [
            (&b""[..], ""),
            (b"f", "Zg"),
            (b"fo", "Zm8"),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg"),
            (&[0xfb, 0xff], "-_8"),
        ] {
            assert_eq!(encode(raw), text);
            assert_eq!(decode(text, 100).as_deref(), Some(raw));
        }
    }

    #[test]
    fn non_canonical_text_is_refused() {
        for bad in ["Zg==", "Zh", "Z", "Zm 9", "Zm9v!", "Zm+/"] {
            assert!(decode(bad, 100).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_cap_is_enforced() {
        assert!(decode("Zm9vYg", 3).is_none());
        assert!(decode("Zm9vYg", 4).is_some());
    }
}
