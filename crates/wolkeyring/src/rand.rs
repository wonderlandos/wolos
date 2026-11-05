// SPDX-License-Identifier: BSD-2-Clause
//! Where fresh random bytes come from. The core takes a [`Random`]; the system uses [`OsRandom`]
//! (the kernel's `getrandom` pool through `/dev/urandom`, which blocks only before the pool is
//! seeded at boot); tests use [`Counter`] so their bytes are the same on every run.
use std::io::Read;

/// A source of random bytes.
pub trait Random {
    /// Fills `buf`; an error only when the system source cannot be read.
    fn fill(&mut self, buf: &mut [u8]) -> crate::Result<()>;
    /// `N` fresh bytes.
    fn bytes<const N: usize>(&mut self) -> crate::Result<[u8; N]>
    where
        Self: Sized,
    {
        let mut b = [0u8; N];
        self.fill(&mut b)?;
        Ok(b)
    }
}

/// The kernel's random pool.
pub struct OsRandom;

impl Random for OsRandom {
    fn fill(&mut self, buf: &mut [u8]) -> crate::Result<()> {
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(buf))
            .map_err(|e| crate::Error::Io(format!("the random source could not be read: {e}")))
    }
}

/// Deterministic bytes for tests and vectors: SHA-512 of a seed and a counter. Never used by the
/// binaries.
pub struct Counter {
    seed: [u8; 32],
    n: u64,
}

impl Counter {
    /// A stream named by `seed`.
    pub fn new(seed: u8) -> Self {
        Counter {
            seed: [seed; 32],
            n: 0,
        }
    }
}

impl Random for Counter {
    fn fill(&mut self, buf: &mut [u8]) -> crate::Result<()> {
        for chunk in buf.chunks_mut(64) {
            self.n += 1;
            let h = crate::wire::derive(&[b"test-random", &self.seed, &self.n.to_be_bytes()]);
            chunk.copy_from_slice(&h[..chunk.len()]);
        }
        Ok(())
    }
}
