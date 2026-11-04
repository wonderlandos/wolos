// SPDX-License-Identifier: BSD-2-Clause
//! The C ABI: the version script, the symbols file, the header and the exported functions name
//! the same set; the calls round-trip through raw pointers like a C caller's; and the built
//! library exports exactly the versioned symbols.
use std::collections::BTreeSet;
use wolcrypt_ffi::*;

fn here(p: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(p)).expect(p)
}

fn names(text: &str, pat: &str) -> BTreeSet<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| {
            w.starts_with(pat) && w.chars().any(|c| c.is_ascii_lowercase()) && *w != "wolcrypt_ffi"
        })
        .map(str::to_owned)
        .collect()
}

#[test]
fn map_symbols_header_and_source_agree() {
    let map = names(&here("abi/wolcrypt.map"), "wolcrypt_");
    let syms = names(&here("abi/wolcrypt-1.0.symbols"), "wolcrypt_");
    let header = names(&here("include/wolcrypt.h"), "wolcrypt_");
    let src: BTreeSet<String> = here("src/lib.rs")
        .lines()
        .filter_map(|l| l.split("extern \"C\" fn ").nth(1))
        .filter_map(|r| r.split('(').next())
        .map(str::to_owned)
        .collect();
    assert_eq!(map, syms);
    assert_eq!(map, header);
    assert_eq!(map, src);
    assert_eq!(map.len(), 13);
}

#[test]
fn calls_round_trip_through_raw_pointers() {
    assert_eq!(wolcrypt_version(), 1 << 16);
    let mut h = [0u8; 64];
    // SAFETY: buffers sized as documented.
    unsafe {
        assert_eq!(wolcrypt_sha512(b"abc".as_ptr(), 3, h.as_mut_ptr()), 0);
        assert_eq!(h, wolcrypt::hash::sha512(b"abc"));
        assert_eq!(
            wolcrypt_sha512(std::ptr::null(), 3, h.as_mut_ptr()),
            WOLCRYPT_ENULL
        );
        let seed = [3u8; 32];
        let mut pk = vec![0u8; 4096];
        let mut n = 0usize;
        assert_eq!(
            wolcrypt_public(seed.as_ptr(), 0, pk.as_mut_ptr(), pk.len(), &mut n),
            0
        );
        pk.truncate(n);
        let mut sig = vec![0u8; 8192];
        let mut sn = 0usize;
        let label = c"wolos/package/1";
        assert_eq!(
            wolcrypt_sign(
                seed.as_ptr(),
                0,
                label.as_ptr(),
                b"x".as_ptr(),
                1,
                sig.as_mut_ptr(),
                sig.len(),
                &mut sn
            ),
            0
        );
        sig.truncate(sn);
        assert_eq!(
            wolcrypt_verify(
                pk.as_ptr(),
                pk.len(),
                label.as_ptr(),
                b"x".as_ptr(),
                1,
                sig.as_ptr(),
                sig.len()
            ),
            0
        );
        assert_eq!(
            wolcrypt_verify(
                pk.as_ptr(),
                pk.len(),
                c"wolos/plugin/1".as_ptr(),
                b"x".as_ptr(),
                1,
                sig.as_ptr(),
                sig.len()
            ),
            1
        );
        let mut small = [0u8; 4];
        assert_eq!(
            wolcrypt_sign(
                seed.as_ptr(),
                0,
                label.as_ptr(),
                b"x".as_ptr(),
                1,
                small.as_mut_ptr(),
                4,
                &mut sn
            ),
            WOLCRYPT_ESPACE
        );
        let mut id = [0u8; 64];
        assert_eq!(wolcrypt_id(pk.as_ptr(), pk.len(), id.as_mut_ptr()), 0);
        assert_eq!(
            id,
            wolcrypt::id::of_keys(
                wolcrypt::sign::SigningKey::from_seed(&seed, wolcrypt::sign::Scheme::Signing)
                    .public(),
                None
            )
        );
        let mut to = [0u8; 1216];
        assert_eq!(wolcrypt_kem_public(seed.as_ptr(), to.as_mut_ptr()), 0);
        let rnd = [8u8; 64];
        let mut sealed = vec![0u8; 2048];
        let fmt = c"wolos/test/1";
        assert_eq!(
            wolcrypt_kem_seal(
                fmt.as_ptr(),
                to.as_ptr(),
                b"P".as_ptr(),
                1,
                b"hi".as_ptr(),
                2,
                rnd.as_ptr(),
                sealed.as_mut_ptr(),
                sealed.len(),
                &mut n
            ),
            0
        );
        sealed.truncate(n);
        let mut back = [0u8; 16];
        let mut bn = 0usize;
        assert_eq!(
            wolcrypt_kem_open(
                fmt.as_ptr(),
                seed.as_ptr(),
                sealed.as_ptr(),
                sealed.len(),
                1,
                back.as_mut_ptr(),
                16,
                &mut bn
            ),
            0
        );
        assert_eq!(&back[..bn], b"hi");
        assert_eq!(wolcrypt_ct_eq(b"ab".as_ptr(), b"ab".as_ptr(), 2), 0);
        assert_eq!(wolcrypt_ct_eq(b"ab".as_ptr(), b"ac".as_ptr(), 2), 1);
        let mut k = [0u8; 32];
        assert_eq!(
            wolcrypt_pbkdf2_sha512(b"pw".as_ptr(), 2, b"salt".as_ptr(), 4, 1, k.as_mut_ptr()),
            WOLCRYPT_ELEN
        );
    }
}

#[test]
fn the_linked_library_exports_exactly_the_versioned_symbols() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let target = std::env::var("CARGO_TARGET_DIR")
        .map_or_else(|_| dir.join("../../target"), std::path::PathBuf::from);
    let lib = target
        .join(if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        })
        .join("libwolcrypt_ffi.a");
    if !lib.exists()
        || std::process::Command::new("cc")
            .arg("--version")
            .output()
            .is_err()
    {
        eprintln!("skipped: no staticlib or no cc");
        return;
    }
    let out = std::env::temp_dir().join(format!("wolcrypt-abi-{}", std::process::id()));
    let st = std::process::Command::new(dir.join("link.sh"))
        .arg(&lib)
        .arg(&out)
        .status()
        .expect("link.sh runs");
    assert!(st.success());
    let nm = std::process::Command::new("nm")
        .args(["-D", "--defined-only"])
        .arg(out.join("libwolcrypt.so.1"))
        .output()
        .expect("nm");
    let text = String::from_utf8_lossy(&nm.stdout);
    let got: BTreeSet<String> = text
        .lines()
        .filter_map(|l| l.split_whitespace().nth(2))
        .filter(|s| s.starts_with("wolcrypt_"))
        .map(|s| s.replace("@@", "@"))
        .collect();
    let want: BTreeSet<String> = here("abi/wolcrypt-1.0.symbols")
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(got, want);
}
