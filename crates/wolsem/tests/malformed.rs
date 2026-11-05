// SPDX-License-Identifier: BSD-2-Clause
//! A table from a damaged or hostile file is refused with an error, never a panic or a huge
//! allocation: random bytes in the header, truncations, flips in the fold and token sections.

use std::path::PathBuf;

use wolsem::Table;

fn good() -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/mini.wst");
    std::fs::read(p).unwrap_or_else(|e| panic!("{e}"))
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n.max(1)).unwrap_or(1)).unwrap_or(0)
    }
}

#[test]
fn mutated_headers_and_truncations_are_errors_not_panics() {
    let base = good();
    assert!(Table::from_bytes(base.clone()).is_ok());
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let (mut refused, mut accepted) = (0, 0);
    for round in 0..3000 {
        let mut b = base.clone();
        match round % 4 {
            0 => {
                // random bytes in the 40-byte header
                for _ in 0..1 + rng.below(4) {
                    let at = rng.below(40);
                    b[at] = u8::try_from(rng.next() & 0xff).unwrap_or(0);
                }
            }
            1 => {
                // a whole offset set to a random or extreme value
                let at = 16 + 4 * rng.below(6);
                let v: u32 = [0, 1, u32::MAX, u32::MAX - 3, 0x8000_0000][rng.below(5)];
                let v = if rng.below(2) == 0 {
                    v
                } else {
                    u32::try_from(rng.next() & 0xffff_ffff).unwrap_or(0)
                };
                b[at..at + 4].copy_from_slice(&v.to_le_bytes());
            }
            2 => b.truncate(rng.below(base.len())),
            _ => {
                // flips in the fold section and the token offsets (the first 60 KB)
                for _ in 0..1 + rng.below(8) {
                    let at = rng.below(60_000.min(b.len()));
                    b[at] ^= 1 << rng.below(8);
                }
            }
        }
        match Table::from_bytes(b) {
            Ok(t) => {
                accepted += 1;
                // whatever it accepted must also be usable without a panic
                let _ = t.embed("dark mode 文字 한 é", 256);
            }
            Err(_) => refused += 1,
        }
    }
    assert!(
        refused > 1500,
        "only {refused} refused ({accepted} accepted)"
    );
}

#[test]
fn a_file_claiming_four_gigabytes_is_refused_before_it_is_allocated() {
    let base = good();
    let mut b = base[..40].to_vec();
    for k in 0..6 {
        let v: u32 = if k == 5 { u32::MAX } else { 40 };
        b[16 + 4 * k..20 + 4 * k].copy_from_slice(&v.to_le_bytes());
    }
    b[16 + 16..16 + 20].copy_from_slice(&(u32::MAX - 1).to_le_bytes()); // vectors start near 4 GiB
    let d = std::env::temp_dir().join(format!("wolsem-malformed-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap_or_else(|e| panic!("{e}"));
    let f = d.join("huge.wst");
    std::fs::write(&f, &b).unwrap_or_else(|e| panic!("{e}"));
    assert!(Table::open(&f).is_err());
    assert!(Table::from_bytes(b).is_err());
    // and a truncated real file
    std::fs::write(&f, &base[..base.len() / 2]).unwrap_or_else(|e| panic!("{e}"));
    assert!(Table::open(&f).is_err());
    let _ = std::fs::remove_dir_all(&d);
}
