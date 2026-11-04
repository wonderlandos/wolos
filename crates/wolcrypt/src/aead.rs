// SPDX-License-Identifier: BSD-2-Clause
//! ChaCha20-Poly1305 (RFC 8439) with associated data: a 32-byte key, a 12-byte nonce, a 16-byte
//! tag appended. Callers use each key with one nonce only (the KEM derives a fresh key per
//! message; `.wolkey` files take a random salt and nonce).
use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, Tag};
use std::vec::Vec;

/// Key length.
pub const KEY: usize = 32;
/// Nonce length.
pub const NONCE: usize = 12;
/// Tag length.
pub const TAG: usize = 16;

/// Encrypts `buf` in place and appends the tag.
pub fn seal(key: &[u8; KEY], nonce: &[u8; NONCE], ad: &[u8], buf: &mut Vec<u8>) {
    let c = ChaCha20Poly1305::new(Key::from_slice(key));
    // Only fails above 2^38 bytes (256 GiB), far beyond every caller's cap.
    if let Ok(tag) = c.encrypt_in_place_detached(Nonce::from_slice(nonce), ad, buf) {
        buf.extend_from_slice(&tag);
    } else {
        buf.clear();
    }
}

/// Checks the tag at the end of `buf` and decrypts in place; on success `buf` holds the plaintext.
/// On failure `buf` is wiped and None returned: nothing unauthenticated is ever handed out.
pub fn open(key: &[u8; KEY], nonce: &[u8; NONCE], ad: &[u8], buf: &mut Vec<u8>) -> Option<()> {
    let Some(n) = buf.len().checked_sub(TAG) else {
        buf.clear();
        return None;
    };
    let tag = *Tag::from_slice(&buf[n..]);
    buf.truncate(n);
    let c = ChaCha20Poly1305::new(Key::from_slice(key));
    if c.decrypt_in_place_detached(Nonce::from_slice(nonce), ad, buf, &tag)
        .is_ok()
    {
        Some(())
    } else {
        zeroize::Zeroize::zeroize(buf);
        buf.clear();
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::{hex, unhex};

    #[test]
    fn rfc_8439_section_2_8_2() {
        let key: [u8; 32] = core::array::from_fn(|i| 0x80 + i as u8);
        let nonce = [
            0x07, 0, 0, 0, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47,
        ];
        let ad = unhex("50515253c0c1c2c3c4c5c6c7").unwrap();
        let pt = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let mut buf = pt.to_vec();
        seal(&key, &nonce, &ad, &mut buf);
        assert_eq!(
            hex(&buf[buf.len() - TAG..]),
            "1ae10b594f09e26a7e902ecbd0600691"
        );
        assert_eq!(hex(&buf[..16]), "d31a8d34648e60db7b86afbc53ef7ec2");
        assert!(open(&key, &nonce, &ad, &mut buf).is_some());
        assert_eq!(&buf[..], &pt[..]);
        let mut t = pt.to_vec();
        seal(&key, &nonce, &ad, &mut t);
        t[3] ^= 1;
        assert!(open(&key, &nonce, &ad, &mut t).is_none());
        assert!(t.is_empty());
        let mut short = vec![1u8; 5];
        assert!(open(&key, &nonce, &ad, &mut short).is_none());
    }
}
