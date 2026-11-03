// SPDX-License-Identifier: BSD-2-Clause
//! The C ABI of libWolkit: the only place with `extern "C"`. Every export checks its handles
//! (`handle::live`), converts pointer and length arguments with the checked helpers here, catches
//! panics (`WOLKIT_ERR_INTERNAL`) and writes out-pointers only on success. The engine behind it is
//! plain Rust (`crate::engine`).
//!
//! Files: `error` (the error object), `engine`, `window`, `style`, `image`, `canvas` and `surface`
//! (stubs W5 fills), `diag`.
#![allow(non_camel_case_types)]
// The C boundary takes a pointer and a length per buffer.
#![allow(clippy::too_many_arguments)]

pub mod canvas;
pub mod diag;
pub mod engine;
pub mod error;
pub mod handle;
pub mod image;
pub mod style;
pub mod surface;
pub mod window;

use crate::error::{Error, Result, Status};
use std::ffi::c_char;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

pub type wolkit_status = i32;
pub use error::wolkit_error;

/// Run an export body: panics become `Internal`, errors fill `*error` when it is given.
pub(crate) fn guard(error: *mut *mut wolkit_error, f: impl FnOnce() -> Result<()>) -> wolkit_status {
    let r = catch_unwind(AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(Error::new(Status::Internal, "libwolkit hit an internal error.")));
    match r {
        Ok(()) => Status::Ok as wolkit_status,
        Err(e) => {
            error::fill(error, &e);
            e.status as wolkit_status
        }
    }
}

/// Like [`guard`] for exports with no error out-pointer.
pub(crate) fn guard_status(f: impl FnOnce() -> Result<()>) -> wolkit_status {
    guard(std::ptr::null_mut(), f)
}

/// For exports that cannot report an error: panics give `fallback`.
pub(crate) fn quiet<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

#[allow(clippy::disallowed_methods)] // the C boundary wrapper (clippy.toml)
/// Bytes from C: a pointer and a length. NULL is the empty slice only with length 0.
///
/// # Safety
/// `ptr` has `len` readable bytes (header contract).
pub(crate) unsafe fn bytes_in<'a>(ptr: *const u8, len: usize, max: usize, what: &str) -> Result<&'a [u8]> {
    if len > max {
        return Err(Error::limit(format!("{what} is longer than {max} bytes.")));
    }
    if ptr.is_null() {
        return if len == 0 {
            Ok(&[])
        } else {
            Err(Error::invalid(format!("{what} is NULL but has a length.")))
        };
    }
    // SAFETY: the caller guarantees `len` readable bytes at `ptr` (header contract).
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

/// UTF-8 text from C.
///
/// # Safety
/// As [`bytes_in`].
pub(crate) unsafe fn text_in<'a>(ptr: *const c_char, len: usize, max: usize, what: &str) -> Result<&'a str> {
    // SAFETY: forwarded contract.
    let b = unsafe { bytes_in(ptr.cast(), len, max, what) }?;
    std::str::from_utf8(b).map_err(|_| Error::invalid(format!("{what} is not UTF-8.")))
}

#[allow(clippy::disallowed_methods)] // the C boundary wrapper (clippy.toml)
/// A writable buffer from C. NULL is the empty slice only with capacity 0.
///
/// # Safety
/// `ptr` has `cap` writable bytes (header contract).
pub(crate) unsafe fn bytes_out<'a>(ptr: *mut u8, cap: usize, what: &str) -> Result<&'a mut [u8]> {
    if ptr.is_null() {
        return if cap == 0 {
            Ok(&mut [])
        } else {
            Err(Error::invalid(format!("{what} is NULL but has a capacity.")))
        };
    }
    // SAFETY: the caller guarantees `cap` writable bytes at `ptr` (header contract).
    Ok(unsafe { std::slice::from_raw_parts_mut(ptr, cap) })
}

/// Write an out-pointer when it is given.
pub(crate) fn put<T>(out: *mut T, v: T) {
    if !out.is_null() {
        // SAFETY: a non-NULL out-pointer is writable (header contract).
        unsafe { out.write(v) };
    }
}

/// A required out-pointer.
pub(crate) fn need<T>(out: *mut T, what: &str) -> Result<()> {
    if out.is_null() {
        return Err(Error::invalid(format!("{what} must not be NULL.")));
    }
    Ok(())
}

/// Copy `data` into the caller's buffer with the "needed" convention: `*needed` is always set; a
/// buffer that is too small gets `WOLKIT_ERR_LIMIT` and is left alone.
///
/// # Safety
/// As [`bytes_out`]; `needed` is NULL or writable.
pub(crate) unsafe fn give(buf: *mut u8, cap: usize, needed: *mut usize, data: &[u8], what: &str) -> Result<()> {
    put(needed, data.len());
    // SAFETY: forwarded contract.
    let out = unsafe { bytes_out(buf, cap, what) }?;
    if data.len() > out.len() {
        return Err(Error::limit(format!("{what} needs {} bytes; the buffer has {}.", data.len(), out.len())));
    }
    out[..data.len()].copy_from_slice(data);
    Ok(())
}

/// Hand out a +1 reference.
pub(crate) fn into_raw<T>(a: Arc<T>) -> *mut T {
    Arc::into_raw(a).cast_mut()
}

/// # Safety
/// `p` is NULL or a live handle of type `T` from libwolkit.
pub(crate) unsafe fn retain<T: handle::Tagged>(p: *mut T) -> *mut T {
    // SAFETY: NULL or a libwolkit handle (contract); the tag is checked before anything else.
    if unsafe { handle::live(p.cast_const()) }.is_some() {
        // SAFETY: `p` came from Arc::into_raw and is live (contract).
        unsafe { Arc::increment_strong_count(p.cast_const()) };
    }
    p
}

/// # Safety
/// `p` is NULL or a handle the caller owns a reference to.
pub(crate) unsafe fn release<T: handle::Tagged>(p: *mut T) {
    // SAFETY: NULL or a libwolkit handle (contract); a released or foreign pointer is ignored.
    if unsafe { handle::live(p.cast_const()) }.is_some() {
        // SAFETY: the caller gives up one reference it owns (contract).
        drop(unsafe { Arc::from_raw(p.cast_const()) });
    }
}

/// # Safety
/// `p` is NULL or a live handle of type `T`.
pub(crate) unsafe fn borrow<'a, T: handle::Tagged>(p: *const T) -> Result<&'a T> {
    if p.is_null() {
        return Err(Error::invalid("A handle is NULL."));
    }
    // SAFETY: a libwolkit handle per the contract; its tag is checked before use.
    unsafe { handle::live(p) }.ok_or_else(|| Error::invalid("A handle is not a live object of the expected type."))
}
