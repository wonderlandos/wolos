// SPDX-License-Identifier: BSD-2-Clause
//! Constant-time comparison (subtle): the time taken depends only on the lengths, never on where
//! two equal-length inputs first differ.
use subtle::ConstantTimeEq;

/// True when `a == b`. Unequal lengths return false at once (lengths are not secret).
pub fn eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

#[cfg(test)]
mod tests {
    #[test]
    fn equal_and_unequal() {
        assert!(super::eq(b"abc", b"abc"));
        assert!(!super::eq(b"abc", b"abd"));
        assert!(!super::eq(b"abc", b"ab"));
        assert!(super::eq(b"", b""));
    }
}
