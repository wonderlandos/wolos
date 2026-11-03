// SPDX-License-Identifier: BSD-2-Clause
//! The error object (`wolkit_error`): a status and a sentence, owned by the caller after a failed call.

use super::{handle, wolkit_status};
use crate::error::{Error, Status};
use std::ffi::c_char;

#[repr(C)]
#[allow(non_camel_case_types)] // the C name
pub struct wolkit_error {
    tag: handle::Tag<wolkit_error>,
    status: Status,
    /// Message bytes with a trailing NUL (not counted in its length).
    message: Vec<u8>,
}

handle::tagged!(wolkit_error, 0x57_4f_4c_4b_49_54_45_52);

/// Store `e` in `*error` when the caller gave an out-pointer.
pub(crate) fn fill(error: *mut *mut wolkit_error, e: &Error) {
    if error.is_null() {
        return;
    }
    let mut message = e.message.replace('\0', " ").into_bytes();
    message.push(0);
    let obj = Box::new(wolkit_error {
        tag: handle::Tag::new(),
        status: e.status,
        message,
    });
    // SAFETY: the caller passed a writable `wolkit_error **` (header contract).
    unsafe { *error = Box::into_raw(obj) };
}

/// # Safety
/// `error` is NULL or came from libwolkit.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_error_get_code(error: *const wolkit_error) -> wolkit_status {
    // SAFETY: NULL or a live error object (contract).
    unsafe { handle::live(error) }.map_or(Status::InvalidArgument.code(), |e| e.status.code())
}

/// # Safety
/// `error` is NULL or came from libwolkit; `len` is NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_error_get_message(
    error: *const wolkit_error,
    len: *mut usize,
) -> *const c_char {
    // SAFETY: NULL or a live error object (contract).
    match unsafe { handle::live(error) } {
        Some(e) => {
            super::put(len, e.message.len() - 1);
            e.message.as_ptr().cast()
        }
        None => {
            super::put(len, 0);
            c"".as_ptr()
        }
    }
}

/// # Safety
/// `error` is NULL or an error libwolkit handed out, released once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_error_release(error: *mut wolkit_error) {
    // SAFETY: NULL or a libwolkit handle (contract); a released or foreign pointer is ignored.
    if unsafe { handle::live(error.cast_const()) }.is_some() {
        // SAFETY: made by Box::into_raw in `fill`, released once (contract).
        drop(unsafe { Box::from_raw(error) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::disallowed_methods)] // reading back the message the way a C caller would
    #[test]
    fn guard_fills_error_only_on_failure_and_catches_panics() {
        let mut e: *mut wolkit_error = std::ptr::null_mut();
        assert_eq!(super::super::guard(&mut e, || Ok(())), 0);
        assert!(e.is_null());
        assert_eq!(
            super::super::guard(&mut e, || Err(Error::invalid("bad\0x"))),
            1
        );
        let mut n = 0;
        // SAFETY: `e` is the live error just made.
        let m = unsafe { wolkit_error_get_message(e, &mut n) };
        // SAFETY: the message is `n + 1` bytes with its NUL.
        let text = unsafe { std::slice::from_raw_parts(m.cast::<u8>(), n + 1) };
        assert_eq!(text, b"bad x\0");
        // SAFETY: as above.
        assert_eq!(unsafe { wolkit_error_get_code(e) }, 1);
        // SAFETY: released once.
        unsafe { wolkit_error_release(e) };
        let code = super::super::guard(std::ptr::null_mut(), || panic!("boom"));
        assert_eq!(code, Status::Internal.code());
    }
}
