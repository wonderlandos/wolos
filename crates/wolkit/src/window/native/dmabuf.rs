// SPDX-License-Identifier: BSD-2-Clause
//! linux-dmabuf feedback: the compositor's main device and the formats it can take, read from the
//! format table it shares (16-byte entries: u32 format, 4 bytes padding, u64 modifier). The CPU
//! path presents through `wl_shm`; this is what the GPU path (WK-W5) chooses buffers from.

use std::os::fd::OwnedFd;
use std::os::unix::fs::FileExt;

/// Entries read at most (real tables have a few hundred).
pub const MAX_FORMATS: usize = 16_384;

/// Read the format table without mapping it.
pub fn read_table(fd: OwnedFd, size: u32) -> Vec<(u32, u64)> {
    let f = std::fs::File::from(fd);
    let n = usize::try_from(size).unwrap_or(0) / 16;
    let n = n.min(MAX_FORMATS);
    let mut buf = vec![0u8; n * 16];
    if f.read_exact_at(&mut buf, 0).is_err() {
        return Vec::new();
    }
    buf.as_chunks::<16>()
        .0
        .iter()
        .map(|e| {
            let fmt = u32::from_ne_bytes([e[0], e[1], e[2], e[3]]);
            let mut m = [0u8; 8];
            m.copy_from_slice(&e[8..16]);
            (fmt, u64::from_ne_bytes(m))
        })
        .collect()
}

/// A `dev_t` sent as an array of bytes (native endian).
pub fn dev_t(bytes: &[u8]) -> Option<u64> {
    match bytes.len() {
        8 => {
            let mut b = [0u8; 8];
            b.copy_from_slice(bytes);
            Some(u64::from_ne_bytes(b))
        }
        4 => Some(u64::from(u32::from_ne_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
        ]))),
        _ => None,
    }
}
