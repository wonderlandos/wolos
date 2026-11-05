// SPDX-License-Identifier: BSD-2-Clause
//! Standard base64 (RFC 4648, padding optional, whitespace ignored) for the vectors in an index JSON.

/// Bytes of `s`, or None for a character outside the alphabet.
pub fn decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3 + 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn base64_decodes_with_and_without_padding() {
        assert_eq!(decode("aGVsbG8=").as_deref(), Some(&b"hello"[..]));
        assert_eq!(decode("aGVsbG8").as_deref(), Some(&b"hello"[..]));
        assert_eq!(decode("").as_deref(), Some(&b""[..]));
        assert_eq!(decode("a$b"), None);
    }
}
