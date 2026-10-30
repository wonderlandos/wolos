//! `wol_string`: immutable, reference-counted UTF-8 with a length (and a trailing NUL for C).
use crate::error::{Result, guard, input, need, quiet, wol_error, wol_status, write_len};
use std::ffi::c_char;
use std::sync::Arc;

/// Largest string an app may hand libwol.
pub const MAX: usize = 1 << 20;

pub struct wol_string {
    /// Text bytes and a NUL that is not counted.
    bytes: Vec<u8>,
}

impl wol_string {
    pub fn new(s: &str) -> Arc<Self> {
        let mut bytes = Vec::with_capacity(s.len() + 1);
        bytes.extend_from_slice(s.as_bytes());
        bytes.push(0);
        Arc::new(Self { bytes })
    }

    pub fn as_str(&self) -> &str {
        // SAFETY-free: built only from &str in `new`.
        std::str::from_utf8(&self.bytes[..self.bytes.len() - 1]).unwrap_or_default()
    }
}

/// Hand out a +1 reference.
pub(crate) fn into_raw<T>(a: Arc<T>) -> *mut T {
    Arc::into_raw(a).cast_mut()
}

/// # Safety
/// `p` is NULL or a live handle of type `T` from libwol.
pub(crate) unsafe fn retain<T>(p: *mut T) -> *mut T {
    if !p.is_null() {
        // SAFETY: `p` came from Arc::into_raw and is live (contract).
        unsafe { Arc::increment_strong_count(p.cast_const()) };
    }
    p
}

/// # Safety
/// `p` is NULL or a handle the caller owns a reference to.
pub(crate) unsafe fn release<T>(p: *mut T) {
    if !p.is_null() {
        // SAFETY: the caller gives up one reference it owns (contract).
        drop(unsafe { Arc::from_raw(p.cast_const()) });
    }
}

/// # Safety
/// `p` is NULL or a live handle of type `T`.
pub(crate) unsafe fn borrow<'a, T>(p: *const T) -> Result<&'a T> {
    // SAFETY: NULL or live (contract).
    unsafe { p.as_ref() }.ok_or_else(|| crate::error::Error::invalid("A handle is NULL."))
}

/// # Safety
/// `utf8` points at `len` readable bytes (or is NULL with `len` 0); `out` and `error` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_string_create(
    utf8: *const c_char,
    len: usize,
    out: *mut *mut wol_string,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let s = input(utf8, len, MAX, "The string")?;
        // SAFETY: checked non-NULL above.
        unsafe { *out = into_raw(wol_string::new(s)) };
        Ok(())
    })
}

/// # Safety
/// `string` is NULL or live; `len` is NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_string_get_bytes(
    string: *const wol_string,
    len: *mut usize,
) -> *const c_char {
    quiet(c"".as_ptr(), || match unsafe { string.as_ref() } {
        Some(s) => {
            write_len(len, s.bytes.len() - 1);
            s.bytes.as_ptr().cast()
        }
        None => {
            write_len(len, 0);
            c"".as_ptr()
        }
    })
}

/// # Safety
/// `string` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_string_retain(string: *mut wol_string) -> *mut wol_string {
    unsafe { retain(string) }
}

/// # Safety
/// `string` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_string_release(string: *mut wol_string) {
    unsafe { release(string) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_get_retain_release() {
        let mut s: *mut wol_string = std::ptr::null_mut();
        let text = "a\0b";
        assert_eq!(
            unsafe { wol_string_create(text.as_ptr().cast(), 3, &mut s, std::ptr::null_mut()) },
            0
        );
        let mut n = 0;
        let p = unsafe { wol_string_get_bytes(s, &mut n) };
        assert_eq!(
            unsafe { std::slice::from_raw_parts(p.cast::<u8>(), n + 1) },
            b"a\0b\0"
        );
        assert_eq!(unsafe { wol_string_retain(s) }, s);
        unsafe { wol_string_release(s) };
        unsafe { wol_string_release(s) };
        let rc = unsafe {
            wol_string_create(c"x".as_ptr(), 1, std::ptr::null_mut(), std::ptr::null_mut())
        };
        assert_eq!(rc, 1, "NULL out is refused");
        unsafe { wol_string_release(std::ptr::null_mut()) };
    }
}
