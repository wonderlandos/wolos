// SPDX-License-Identifier: GPL-2.0-or-later
//! Small pure helpers: base64, the token estimate, glob match, a bounded ring, error type.
use std::collections::VecDeque;
use std::fmt;

/// A failure reported to the agent as ONE line: what went wrong, then the fix hint.
#[derive(Debug, Clone, PartialEq)]
pub struct DriveError {
    pub code: &'static str,
    pub message: String,
    pub hint: String,
}

impl DriveError {
    pub fn new(code: &'static str, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: hint.into(),
        }
    }
    /// `message; hint` on one line (no newline inside).
    pub fn line(&self) -> String {
        let one = |s: &str| s.replace(['\n', '\r'], " ");
        if self.hint.is_empty() {
            one(&self.message)
        } else {
            format!("{}; {}", one(&self.message), one(&self.hint))
        }
    }
}

impl fmt::Display for DriveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.line())
    }
}
impl std::error::Error for DriveError {}

pub type Res<T> = Result<T, DriveError>;

pub fn err<T>(code: &'static str, message: impl Into<String>, hint: impl Into<String>) -> Res<T> {
    Err(DriveError::new(code, message, hint))
}

/// The token estimate every result carries: bytes / 4, rounded up (cl100k-like for English and code).
pub fn tokens(text_bytes: usize) -> usize {
    text_bytes.div_ceil(4)
}

/// Cut `s` to at most `max` bytes on a char boundary; true when it was cut.
pub fn clip(s: &str, max: usize) -> (&str, bool) {
    if s.len() <= max {
        return (s, false);
    }
    let mut n = max;
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    (&s[..n], true)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Decode standard base64 (padding optional, whitespace ignored). None on any other byte.
pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3 + 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for b in s.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// `*` matches any run (including none); everything else is literal. `Network.*`, `*.requestWillBeSent`.
pub fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rest = text;
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            match rest.strip_prefix(p) {
                Some(r) => rest = r,
                None => return false,
            }
        } else if i == parts.len() - 1 {
            return rest.ends_with(p);
        } else {
            match rest.find(p) {
                Some(at) => rest = &rest[at + p.len()..],
                None => return false,
            }
        }
    }
    true
}

/// A queue that keeps the newest `cap` items: no unbounded Vec anywhere.
#[derive(Debug, Clone)]
pub struct Ring<T> {
    cap: usize,
    pub items: VecDeque<T>,
    pub dropped: u64,
}

impl<T> Ring<T> {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            items: VecDeque::new(),
            dropped: 0,
        }
    }
    pub fn push(&mut self, v: T) {
        if self.items.len() >= self.cap {
            self.items.pop_front();
            self.dropped += 1;
        }
        self.items.push_back(v);
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// FNV-1a 64: a stable hash for boilerplate detection and diffs (not a security hash).
pub fn fnv(data: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// Named waits (RH-222): every deadline a verb uses is one of these, so both ends of a wait read the same value.
pub const WAIT_100MS: std::time::Duration = std::time::Duration::from_millis(100);
pub const WAIT_10S: std::time::Duration = std::time::Duration::from_secs(10);
pub const WAIT_120S: std::time::Duration = std::time::Duration::from_secs(120);
pub const WAIT_1500MS: std::time::Duration = std::time::Duration::from_millis(1500);
pub const WAIT_15S: std::time::Duration = std::time::Duration::from_secs(15);
pub const WAIT_20MS: std::time::Duration = std::time::Duration::from_millis(20);
pub const WAIT_20S: std::time::Duration = std::time::Duration::from_secs(20);
pub const WAIT_2S: std::time::Duration = std::time::Duration::from_secs(2);
pub const WAIT_3S: std::time::Duration = std::time::Duration::from_secs(3);
pub const WAIT_40MS: std::time::Duration = std::time::Duration::from_millis(40);
pub const WAIT_500MS: std::time::Duration = std::time::Duration::from_millis(500);
pub const WAIT_50MS: std::time::Duration = std::time::Duration::from_millis(50);
pub const WAIT_5S: std::time::Duration = std::time::Duration::from_secs(5);
pub const WAIT_60MS: std::time::Duration = std::time::Duration::from_millis(60);
pub const WAIT_60S: std::time::Duration = std::time::Duration::from_secs(60);
pub const WAIT_80MS: std::time::Duration = std::time::Duration::from_millis(80);
pub const WAIT_8S: std::time::Duration = std::time::Duration::from_secs(8);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_every_length() {
        for n in 0..40usize {
            let data: Vec<u8> = (0..n as u8).map(|i| i.wrapping_mul(37)).collect();
            assert_eq!(b64_decode(&b64_encode(&data)).unwrap(), data, "len {n}");
        }
        assert_eq!(b64_encode(b"Man"), "TWFu");
        assert_eq!(b64_encode(b"Ma"), "TWE=");
        assert!(b64_decode("TW!u").is_none());
    }

    #[test]
    fn globs() {
        assert!(glob("Network.*", "Network.requestWillBeSent"));
        assert!(!glob("Network.*", "Page.load"));
        assert!(glob("*.load", "Page.load"));
        assert!(glob("*", ""));
        assert!(glob("Page.frame*Loading", "Page.frameStartedLoading"));
        assert!(glob("a", "a") && !glob("a", "ab"));
    }

    #[test]
    fn tokens_and_clip() {
        assert_eq!(tokens(0), 0);
        assert_eq!(tokens(1), 1);
        assert_eq!(tokens(9), 3);
        assert_eq!(clip("héllo", 2), ("h", true));
        assert_eq!(clip("abc", 9), ("abc", false));
    }

    #[test]
    fn ring_keeps_the_newest() {
        let mut r = Ring::new(3);
        for i in 0..5 {
            r.push(i);
        }
        assert_eq!(r.items.iter().copied().collect::<Vec<_>>(), vec![2, 3, 4]);
        assert_eq!(r.dropped, 2);
    }

    #[test]
    fn error_is_one_line() {
        let e = DriveError::new("x", "bad\nthing", "do this");
        assert_eq!(e.line(), "bad thing; do this");
    }
}
