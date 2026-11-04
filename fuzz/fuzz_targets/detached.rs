// SPDX-License-Identifier: GPL-2.0-or-later
//! The witness's detached check (`wolcrypt::detached::verify`, behind wolos.org/verify and the C
//! ABI) over any bytes (audit 2 L5). Input: `u16 wsig length || wsig || file`, checked against
//! the TEST roots A and B and the TEST witness, and with the witness and roots taken from the
//! bytes too. Never panic; a signature that verifies gives a witness line naming the file's hash.
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;
use wol_core::woltrust::pki::Pki;
use wol_core::woltrust::vectors_ids::Ids;
use wol_core::woltrust::wolcrypt::detached;

/// Root A, root B and witness A of the TEST authority (made once: SLH-DSA key generation is slow).
fn fixed() -> &'static (Vec<u8>, Vec<u8>, Vec<u8>) {
    static F: OnceLock<(Vec<u8>, Vec<u8>, Vec<u8>)> = OnceLock::new();
    F.get_or_init(|| {
        let p = Pki::new();
        let ids = Ids::new(&p);
        (p.root_a.file.clone(), p.root_b.file.clone(), ids.witness.file.clone())
    })
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 {
        return;
    }
    let n = usize::from(u16::from_be_bytes([data[0], data[1]])).min(data.len() - 2);
    let (wsig, file) = data[2..].split_at(n);
    let (a, b, w) = fixed();
    if let Ok(line) = detached::verify(file, wsig, w, [a, b]) {
        let hash = wol_core::woltrust::wolcrypt::hash::hex(&wol_core::woltrust::wolcrypt::hash::sha512(file));
        assert!(line.contains(&hash), "the line names the file that was checked");
    }
    // Witness and roots from the input: any bytes, never a panic.
    let third = file.len() / 3;
    let (x, rest) = file.split_at(third);
    let (y, z) = rest.split_at(rest.len() / 2);
    let _ = detached::verify(z, wsig, x, [y, a]);
});
