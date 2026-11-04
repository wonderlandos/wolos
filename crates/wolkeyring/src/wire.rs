// SPDX-License-Identifier: BSD-2-Clause
//! Unambiguous derivations and the small binary reader of the keyring formats. Every derivation
//! hashes length-prefixed parts (`u32` big-endian length, then the bytes), so no two different
//! part lists hash the same input; [`wolcrypt::hash::sha512_parts`] concatenates and is used here
//! only on the prefixed form.
use zeroize::Zeroizing;

/// `SHA-512(len(p0) || p0 || len(p1) || p1 || ...)`: the one derivation of this crate.
pub fn derive(parts: &[&[u8]]) -> Zeroizing<[u8; 64]> {
    let mut buf: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::new());
    for p in parts {
        let n = u32::try_from(p.len()).unwrap_or(u32::MAX);
        buf.extend_from_slice(&n.to_be_bytes());
        buf.extend_from_slice(p);
    }
    Zeroizing::new(wolcrypt::hash::sha512(&buf))
}

/// The AEAD key (bytes 0..32) and nonce (32..44) of a derivation, as wiped copies.
pub fn key_nonce(okm: &[u8; 64]) -> (Zeroizing<[u8; 32]>, Zeroizing<[u8; 12]>) {
    let mut k = Zeroizing::new([0u8; 32]);
    let mut n = Zeroizing::new([0u8; 12]);
    k.copy_from_slice(&okm[..32]);
    n.copy_from_slice(&okm[32..44]);
    (k, n)
}

/// Seals `plain` under a key derived from `parts`; the derivation must include a fresh salt, so
/// every key is used once (ADR 0054 section 2).
pub fn seal(parts: &[&[u8]], ad: &[u8], plain: &[u8]) -> Vec<u8> {
    let okm = derive(parts);
    let (k, n) = key_nonce(&okm);
    let mut buf = plain.to_vec();
    wolcrypt::aead::seal(&k, &n, ad, &mut buf);
    buf
}

/// Opens what [`seal`] made; None when a byte, the key or the associated data differs.
pub fn open(parts: &[&[u8]], ad: &[u8], sealed: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    let okm = derive(parts);
    let (k, n) = key_nonce(&okm);
    let mut buf = Zeroizing::new(sealed.to_vec());
    wolcrypt::aead::open(&k, &n, ad, &mut buf)?;
    Some(buf)
}

/// Base64url without padding (the form every WolOS file uses).
pub fn b64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        let take = c.len() + 1;
        for i in 0..take {
            let v = (n >> (18 - 6 * i)) & 63;
            out.push(char::from(A[v as usize]));
        }
    }
    out
}

/// Decodes base64url of at most `max` bytes.
pub fn unb64(s: &str, max: usize) -> crate::Result<Vec<u8>> {
    wolcrypt::b64::decode(s, max).ok_or(crate::Error::Malformed("base64"))
}

/// Decodes base64url of exactly `N` bytes.
pub fn unb64_n<const N: usize>(s: &str) -> crate::Result<[u8; N]> {
    let v = unb64(s, N)?;
    v.try_into().map_err(|_| crate::Error::Malformed("length"))
}

/// Lowercase hex.
pub fn hex(b: &[u8]) -> String {
    wolcrypt::hash::hex(b)
}

/// A cursor over untrusted bytes: every read is bounds-checked.
pub struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    /// Starts at byte 0.
    pub fn new(b: &'a [u8]) -> Self {
        Reader { b, at: 0 }
    }
    /// The next `n` bytes, or Malformed when fewer are left.
    pub fn take(&mut self, n: usize) -> crate::Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(n)
            .ok_or(crate::Error::Malformed("length"))?;
        let s = self
            .b
            .get(self.at..end)
            .ok_or(crate::Error::Malformed("cut short"))?;
        self.at = end;
        Ok(s)
    }
    /// A big-endian u32.
    pub fn u32(&mut self) -> crate::Result<u32> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    /// A big-endian u64.
    pub fn u64(&mut self) -> crate::Result<u64> {
        let s = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Ok(u64::from_be_bytes(a))
    }
    /// Exactly `N` bytes.
    pub fn arr<const N: usize>(&mut self) -> crate::Result<[u8; N]> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }
    /// A u32 length, at most `max`, then that many bytes.
    pub fn chunk(&mut self, max: usize) -> crate::Result<&'a [u8]> {
        let n = usize::try_from(self.u32()?).map_err(|_| crate::Error::Malformed("length"))?;
        if n > max {
            return Err(crate::Error::Limit("record size"));
        }
        self.take(n)
    }
    /// Bytes not read yet.
    pub fn rest(&self) -> usize {
        self.b.len() - self.at
    }
    /// The read position.
    pub fn pos(&self) -> usize {
        self.at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_prefix_separates_parts() {
        assert_ne!(*derive(&[b"ab", b"c"]), *derive(&[b"a", b"bc"]));
        assert_ne!(*derive(&[b"abc"]), *derive(&[b"abc", b""]));
    }

    #[test]
    fn b64_round_trips_through_wolcrypt_decoder() {
        for n in 0..70usize {
            let v: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(unb64(&b64(&v), 100).unwrap(), v, "{n}");
        }
        assert_eq!(b64(b"foobar"), "Zm9vYmFy");
        assert_eq!(b64(b"fo"), "Zm8");
    }

    #[test]
    fn seal_open_and_tamper() {
        let s = seal(&[b"k", b"salt"], b"ad", b"secret");
        assert_eq!(&open(&[b"k", b"salt"], b"ad", &s).unwrap()[..], b"secret");
        assert!(open(&[b"k", b"salt2"], b"ad", &s).is_none());
        assert!(open(&[b"k", b"salt"], b"ad2", &s).is_none());
        let mut t = s.clone();
        t[0] ^= 1;
        assert!(open(&[b"k", b"salt"], b"ad", &t).is_none());
    }

    #[test]
    fn reader_refuses_overrun() {
        let mut r = Reader::new(&[0, 0, 0, 9, 1, 2]);
        assert!(r.chunk(100).is_err());
        let mut r = Reader::new(&[0, 0, 0, 2, 1, 2]);
        assert_eq!(r.chunk(100).unwrap(), &[1, 2]);
        let mut r = Reader::new(&[0, 0, 0, 200]);
        assert_eq!(r.chunk(10), Err(crate::Error::Limit("record size")));
    }
}
