// SPDX-License-Identifier: BSD-2-Clause
//! Base64url without padding (RFC 4648 section 5), canonical only: the form every WolOS file uses.
use std::vec::Vec;

fn val(c: u8) -> Option<u32> {
    Some(u32::from(match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'-' => 62,
        b'_' => 63,
        _ => return None,
    }))
}

/// Decodes at most `max` bytes; None for padding, other characters, a non-canonical last
/// character or a longer result.
pub fn decode(s: &str, max: usize) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.len() % 4 == 1 || b.len() / 4 * 3 > max + 3 {
        return None;
    }
    let mut out = Vec::with_capacity(b.len() / 4 * 3 + 2);
    for chunk in b.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= val(*c)? << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        let take = chunk.len() - 1;
        // Canonical: the bits below the last full byte are zero.
        let rest = match take {
            1 => n & 0xffff,
            2 => n & 0xff,
            _ => 0,
        };
        if rest != 0 {
            return None;
        }
        out.extend_from_slice(&bytes[1..1 + take]);
    }
    (out.len() <= max).then_some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_canonical_base64url_decodes() {
        assert_eq!(super::decode("AAECAw", 16), Some(vec![0, 1, 2, 3]));
        assert_eq!(super::decode("AAECAx", 16), None, "non-canonical tail");
        assert_eq!(super::decode("AAECAw==", 16), None, "padding");
        assert_eq!(super::decode("AAECAw", 3), None, "too long");
    }
}
