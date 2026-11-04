// SPDX-License-Identifier: BSD-2-Clause
//! libwolcrypt: the C ABI of crate `wolcrypt` (include/wolcrypt.h, abi/wolcrypt.map). Every
//! function takes pointers and lengths, writes only into buffers the caller sized, never
//! allocates for the caller, and returns a status: 0 success, 1 does not verify / not equal,
//! negative a bad argument (-1 a NULL pointer, -2 a length, -3 a buffer too small, -4 a bad key).
//! The `unsafe` here is the pointer boundary only; every operation is the safe crate's.
use wolcrypt::Zeroizing;
use wolcrypt::sign::{PublicKeys, Scheme, Second, Sig, SigningKey};

/// Success.
pub const WOLCRYPT_OK: i32 = 0;
/// Does not verify, or not equal.
pub const WOLCRYPT_NO: i32 = 1;
/// A required pointer is NULL.
pub const WOLCRYPT_ENULL: i32 = -1;
/// A length is out of range.
pub const WOLCRYPT_ELEN: i32 = -2;
/// The output buffer is too small (the needed size is written to `*out_len`).
pub const WOLCRYPT_ESPACE: i32 = -3;
/// A key is not a valid key.
pub const WOLCRYPT_EKEY: i32 = -4;

const MAX_IN: usize = 1 << 30;

/// A 32-byte seed copied out of the caller's buffer into a wiped copy (audit 2 L3).
fn seed32(b: &[u8]) -> Result<Zeroizing<[u8; 32]>, i32> {
    Ok(Zeroizing::new(b.try_into().map_err(|_| WOLCRYPT_ELEN)?))
}

/// # Safety
/// `p` is NULL or points at `len` readable bytes.
unsafe fn input<'a>(p: *const u8, len: usize) -> Result<&'a [u8], i32> {
    if len == 0 {
        return Ok(&[]);
    }
    if p.is_null() {
        return Err(WOLCRYPT_ENULL);
    }
    if len > MAX_IN {
        return Err(WOLCRYPT_ELEN);
    }
    #[allow(clippy::disallowed_methods)] // The one place a slice is built from a C pointer.
    // SAFETY: non-NULL, `len` readable bytes (contract).
    Ok(unsafe { std::slice::from_raw_parts(p, len) })
}

/// # Safety
/// `p` is NULL or points at `N` writable bytes.
unsafe fn out_n<'a, const N: usize>(p: *mut u8) -> Result<&'a mut [u8; N], i32> {
    if p.is_null() {
        return Err(WOLCRYPT_ENULL);
    }
    // SAFETY: non-NULL, `N` writable bytes (contract).
    Ok(unsafe { &mut *p.cast::<[u8; N]>() })
}

/// Copies `src` into the caller's buffer `out` of `cap` bytes and writes the length.
/// # Safety
/// `out` points at `cap` writable bytes or is NULL with `cap == 0`; `out_len` is writable.
unsafe fn give(src: &[u8], out: *mut u8, cap: usize, out_len: *mut usize) -> i32 {
    if out_len.is_null() {
        return WOLCRYPT_ENULL;
    }
    // SAFETY: writable (contract).
    unsafe { *out_len = src.len() };
    if src.len() > cap {
        return WOLCRYPT_ESPACE;
    }
    if src.is_empty() {
        return WOLCRYPT_OK;
    }
    if out.is_null() {
        return WOLCRYPT_ENULL;
    }
    // SAFETY: `cap >= src.len()` writable bytes (contract), regions distinct.
    unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), out, src.len()) };
    WOLCRYPT_OK
}

/// # Safety
/// `s` is a NUL-terminated string or NULL.
unsafe fn text<'a>(s: *const std::ffi::c_char) -> Result<&'a str, i32> {
    if s.is_null() {
        return Err(WOLCRYPT_ENULL);
    }
    // SAFETY: NUL-terminated (contract).
    let c = unsafe { std::ffi::CStr::from_ptr(s) };
    c.to_str().map_err(|_| WOLCRYPT_ELEN)
}

fn status(r: Result<i32, i32>) -> i32 {
    r.unwrap_or_else(|e| e)
}

fn scheme(root: i32) -> Scheme {
    if root != 0 {
        Scheme::Root
    } else {
        Scheme::Signing
    }
}

/// `major << 16 | minor` of the library.
#[unsafe(no_mangle)]
pub extern "C" fn wolcrypt_version() -> u32 {
    (u32::from(wolcrypt::VERSION.0) << 16) | u32::from(wolcrypt::VERSION.1)
}

/// SHA-512 of `len` bytes at `data` into `out[64]`.
/// # Safety
/// `data` holds `len` readable bytes; `out` 64 writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolcrypt_sha512(data: *const u8, len: usize, out: *mut u8) -> i32 {
    status((|| {
        // SAFETY: contract.
        let d = unsafe { input(data, len) }?;
        // SAFETY: contract.
        *unsafe { out_n::<64>(out) }? = wolcrypt::hash::sha512(d);
        Ok(WOLCRYPT_OK)
    })())
}

/// SHA-512/256 of `len` bytes at `data` into `out[32]`.
/// # Safety
/// `data` holds `len` readable bytes; `out` 32 writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolcrypt_sha512_256(data: *const u8, len: usize, out: *mut u8) -> i32 {
    status((|| {
        // SAFETY: contract.
        let d = unsafe { input(data, len) }?;
        // SAFETY: contract.
        *unsafe { out_n::<32>(out) }? = wolcrypt::hash::sha512_256(d);
        Ok(WOLCRYPT_OK)
    })())
}

/// The id of a key set: SHA-512 over the public keys already concatenated in id order (32, 64,
/// 1984 or 3200 bytes), into `out[64]`.
/// # Safety
/// `keys` holds `len` readable bytes; `out` 64 writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolcrypt_id(keys: *const u8, len: usize, out: *mut u8) -> i32 {
    status((|| {
        if ![32, 64, 1984, 3200].contains(&len) {
            return Err(WOLCRYPT_ELEN);
        }
        // SAFETY: contract.
        let k = unsafe { input(keys, len) }?;
        // SAFETY: contract.
        *unsafe { out_n::<64>(out) }? = wolcrypt::hash::sha512(k);
        Ok(WOLCRYPT_OK)
    })())
}

/// The public signing keys of a 32-byte seed: Ed25519 (32) then ML-DSA-65 (1952) or, with
/// `root != 0`, SLH-DSA-SHA2-128s (32), into `out`.
/// # Safety
/// `seed` 32 readable bytes; `out` `cap` writable bytes; `out_len` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolcrypt_public(
    seed: *const u8,
    root: i32,
    out: *mut u8,
    cap: usize,
    out_len: *mut usize,
) -> i32 {
    status((|| {
        // SAFETY: contract.
        let s = seed32(unsafe { input(seed, 32) }?)?;
        let k = SigningKey::from_seed(&s, scheme(root));
        let mut v = k.public().ed25519.to_vec();
        v.extend_from_slice(k.public().second.as_bytes());
        // SAFETY: contract.
        Ok(unsafe { give(&v, out, cap, out_len) })
    })())
}

/// Signs `label || 0x00 || SHA-512(content)` with both halves of the keys of `seed`: Ed25519
/// (64) then the post-quantum half, into `out`.
/// # Safety
/// `seed` 32 readable bytes; `label` NUL-terminated; `content` `len` readable bytes; `out` `cap`
/// writable bytes; `out_len` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolcrypt_sign(
    seed: *const u8,
    root: i32,
    label: *const std::ffi::c_char,
    content: *const u8,
    len: usize,
    out: *mut u8,
    cap: usize,
    out_len: *mut usize,
) -> i32 {
    status((|| {
        // SAFETY: contract.
        let s = seed32(unsafe { input(seed, 32) }?)?;
        // SAFETY: contract.
        let l = unsafe { text(label) }?;
        // SAFETY: contract.
        let c = unsafe { input(content, len) }?;
        let sig = SigningKey::from_seed(&s, scheme(root))
            .sign(l, c)
            .map_err(|_| WOLCRYPT_EKEY)?;
        let mut v = sig.ed25519.to_vec();
        v.extend_from_slice(&sig.second);
        // SAFETY: contract.
        Ok(unsafe { give(&v, out, cap, out_len) })
    })())
}

/// Verifies both halves of `sig` (Ed25519 64 bytes, then the post-quantum half) over
/// `label || 0x00 || SHA-512(content)` with `keys` (as [`wolcrypt_public`] writes them). 0 when
/// both verify, 1 when not. There is no call that checks one half.
/// # Safety
/// Each pointer holds its length in readable bytes; `label` NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolcrypt_verify(
    keys: *const u8,
    keys_len: usize,
    label: *const std::ffi::c_char,
    content: *const u8,
    len: usize,
    sig: *const u8,
    sig_len: usize,
) -> i32 {
    status((|| {
        // SAFETY: contract.
        let k = unsafe { input(keys, keys_len) }?;
        // SAFETY: contract.
        let l = unsafe { text(label) }?;
        // SAFETY: contract.
        let c = unsafe { input(content, len) }?;
        // SAFETY: contract.
        let s = unsafe { input(sig, sig_len) }?;
        let ed: [u8; 32] = k
            .get(..32)
            .ok_or(WOLCRYPT_ELEN)?
            .try_into()
            .map_err(|_| WOLCRYPT_ELEN)?;
        let second = match k.len() - 32 {
            wolcrypt::sign::ML_PK => {
                Second::MlDsa65(Box::new(k[32..].try_into().map_err(|_| WOLCRYPT_ELEN)?))
            }
            wolcrypt::sign::SLH_PK => {
                Second::Slh(Box::new(k[32..].try_into().map_err(|_| WOLCRYPT_ELEN)?))
            }
            _ => return Err(WOLCRYPT_ELEN),
        };
        if s.len() < 64 {
            return Err(WOLCRYPT_ELEN);
        }
        let sig = Sig {
            ed25519: s[..64].try_into().map_err(|_| WOLCRYPT_ELEN)?,
            second: s[64..].to_vec(),
        };
        let pk = PublicKeys {
            ed25519: ed,
            second,
        };
        Ok(if wolcrypt::sign::verify(&pk, l, c, &sig).is_ok() {
            WOLCRYPT_OK
        } else {
            WOLCRYPT_NO
        })
    })())
}

/// The public encryption keys of a 32-byte seed: X25519 (32) then ML-KEM-768 (1184) into
/// `out[1216]`.
/// # Safety
/// `seed` 32 readable bytes; `out` 1216 writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolcrypt_kem_public(seed: *const u8, out: *mut u8) -> i32 {
    status((|| {
        // SAFETY: contract.
        let s = seed32(unsafe { input(seed, 32) }?)?;
        let e = wolcrypt::kem::EncSecret::from_seed(&s);
        // SAFETY: contract.
        let o = unsafe { out_n::<1216>(out) }?;
        o[..32].copy_from_slice(&e.public().x25519);
        o[32..].copy_from_slice(&e.public().ml_kem_768[..]);
        Ok(WOLCRYPT_OK)
    })())
}

/// Seals `plaintext` to `to[1216]` (X25519 then ML-KEM-768): `prefix || ephemeral ||
/// ciphertext || aead` into `out`; `format` names the key schedule; `rnd` is 64 fresh random
/// bytes.
/// # Safety
/// Each pointer holds its length in readable bytes (`to` 1216, `rnd` 64); `format`
/// NUL-terminated; `out` `cap` writable bytes; `out_len` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolcrypt_kem_seal(
    format: *const std::ffi::c_char,
    to: *const u8,
    prefix: *const u8,
    prefix_len: usize,
    plaintext: *const u8,
    len: usize,
    rnd: *const u8,
    out: *mut u8,
    cap: usize,
    out_len: *mut usize,
) -> i32 {
    status((|| {
        // SAFETY: contract.
        let f = unsafe { text(format) }?;
        // SAFETY: contract.
        let t = unsafe { input(to, 1216) }?;
        // SAFETY: contract.
        let p = unsafe { input(prefix, prefix_len) }?;
        // SAFETY: contract.
        let m = unsafe { input(plaintext, len) }?;
        // SAFETY: contract.
        let r: Zeroizing<[u8; 64]> = Zeroizing::new(
            unsafe { input(rnd, 64) }?
                .try_into()
                .map_err(|_| WOLCRYPT_ELEN)?,
        );
        let pk = wolcrypt::kem::EncPublic {
            x25519: t[..32].try_into().map_err(|_| WOLCRYPT_ELEN)?,
            ml_kem_768: Box::new(t[32..].try_into().map_err(|_| WOLCRYPT_ELEN)?),
        };
        let s = wolcrypt::kem::seal(f, &pk, p, m, &r).map_err(|_| WOLCRYPT_EKEY)?;
        // SAFETY: contract.
        Ok(unsafe { give(&s, out, cap, out_len) })
    })())
}

/// Opens a sealed message whose prefix is `prefix_len` bytes with the keys of `seed`; the
/// plaintext into `out`. 1 when it does not open (changed, or not this key's).
/// # Safety
/// `seed` 32 readable bytes; `format` NUL-terminated; `sealed` `len` readable bytes; `out` `cap`
/// writable bytes; `out_len` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolcrypt_kem_open(
    format: *const std::ffi::c_char,
    seed: *const u8,
    sealed: *const u8,
    len: usize,
    prefix_len: usize,
    out: *mut u8,
    cap: usize,
    out_len: *mut usize,
) -> i32 {
    status((|| {
        // SAFETY: contract.
        let f = unsafe { text(format) }?;
        // SAFETY: contract.
        let s = seed32(unsafe { input(seed, 32) }?)?;
        // SAFETY: contract.
        let m = unsafe { input(sealed, len) }?;
        let keys = wolcrypt::kem::EncSecret::from_seed(&s);
        match wolcrypt::kem::open(f, &keys, m, prefix_len) {
            // SAFETY: contract.
            Ok(p) => Ok(unsafe { give(&p, out, cap, out_len) }),
            Err(_) => Ok(WOLCRYPT_NO),
        }
    })())
}

/// PBKDF2-HMAC-SHA512 within wolcrypt's bounds (210 000 to 10 000 000 rounds) into `out[32]`.
/// # Safety
/// `pass` and `salt` hold their lengths in readable bytes; `out` 32 writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolcrypt_pbkdf2_sha512(
    pass: *const u8,
    pass_len: usize,
    salt: *const u8,
    salt_len: usize,
    rounds: u32,
    out: *mut u8,
) -> i32 {
    status((|| {
        // SAFETY: contract.
        let p = unsafe { input(pass, pass_len) }?;
        // SAFETY: contract.
        let s = unsafe { input(salt, salt_len) }?;
        let k = wolcrypt::kdf::pbkdf2_sha512(p, s, rounds).ok_or(WOLCRYPT_ELEN)?;
        // SAFETY: contract.
        *unsafe { out_n::<32>(out) }? = *k;
        Ok(WOLCRYPT_OK)
    })())
}

/// Constant-time comparison of `len` bytes: 0 equal, 1 not.
/// # Safety
/// `a` and `b` hold `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolcrypt_ct_eq(a: *const u8, b: *const u8, len: usize) -> i32 {
    status((|| {
        // SAFETY: contract.
        let x = unsafe { input(a, len) }?;
        // SAFETY: contract.
        let y = unsafe { input(b, len) }?;
        Ok(if wolcrypt::ct::eq(x, y) {
            WOLCRYPT_OK
        } else {
            WOLCRYPT_NO
        })
    })())
}

/// A witness's detached signature (`wolcrypt::detached`): 0 and the signature line (no NUL) in
/// `line` when it verifies, 1 when not.
/// # Safety
/// Each input pointer holds its length in readable bytes; `line` `cap` writable bytes;
/// `line_len` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolcrypt_verify_detached(
    file: *const u8,
    file_len: usize,
    wsig: *const u8,
    wsig_len: usize,
    witness: *const u8,
    witness_len: usize,
    root_a: *const u8,
    root_a_len: usize,
    root_b: *const u8,
    root_b_len: usize,
    line: *mut u8,
    cap: usize,
    line_len: *mut usize,
) -> i32 {
    status((|| {
        // SAFETY: contract (each pair).
        let f = unsafe { input(file, file_len) }?;
        // SAFETY: contract.
        let s = unsafe { input(wsig, wsig_len) }?;
        // SAFETY: contract.
        let w = unsafe { input(witness, witness_len) }?;
        // SAFETY: contract.
        let a = unsafe { input(root_a, root_a_len) }?;
        // SAFETY: contract.
        let b = unsafe { input(root_b, root_b_len) }?;
        match wolcrypt::detached::verify(f, s, w, [a, b]) {
            // SAFETY: contract.
            Ok(l) => Ok(unsafe { give(l.as_bytes(), line, cap, line_len) }),
            Err(_) => Ok(WOLCRYPT_NO),
        }
    })())
}
