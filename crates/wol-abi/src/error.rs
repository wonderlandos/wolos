//! Status codes, the error object, and the guards every export goes through: input checks,
//! panics caught at the boundary, out-pointers written only on success.
use std::ffi::{c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};

pub type wol_status = i32;

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok = 0,
    InvalidArgument = 1,
    PermissionDenied = 2,
    Unavailable = 3,
    Internal = 4,
}

impl From<shared::portal::Code> for Status {
    fn from(c: shared::portal::Code) -> Self {
        use shared::portal::Code;
        match c {
            Code::InvalidArgument => Self::InvalidArgument,
            Code::PermissionDenied => Self::PermissionDenied,
            Code::Unavailable => Self::Unavailable,
            Code::Internal => Self::Internal,
        }
    }
}

/// A failed call: what to show the person, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub status: Status,
    pub message: String,
}

impl Error {
    pub fn new(status: Status, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(Status::InvalidArgument, message)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Opaque error object. Owned by the caller after a failed call; `wol_error_release` frees it.
pub struct wol_error {
    status: Status,
    /// Message bytes with a trailing NUL (not counted in its length).
    message: Vec<u8>,
}

/// Run an export body: panics become `Internal`, errors fill `*error` when it is given.
pub(crate) fn guard(error: *mut *mut wol_error, f: impl FnOnce() -> Result<()>) -> wol_status {
    let r = catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        Err(Error::new(
            Status::Internal,
            "libwol hit an internal error.",
        ))
    });
    match r {
        Ok(()) => Status::Ok as wol_status,
        Err(e) => {
            if !error.is_null() {
                let mut message = e.message.replace('\0', " ").into_bytes();
                message.push(0);
                let obj = Box::new(wol_error {
                    status: e.status,
                    message,
                });
                // SAFETY: the caller passed a writable `wol_error **` (header contract).
                unsafe { *error = Box::into_raw(obj) };
            }
            e.status as wol_status
        }
    }
}

/// Like [`guard`] for exports that cannot report an error: panics give `fallback`.
pub(crate) fn quiet<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

/// UTF-8 input with an explicit length. NULL is allowed only with length 0 (the empty string).
pub(crate) fn input<'a>(ptr: *const c_char, len: usize, max: usize, what: &str) -> Result<&'a str> {
    if ptr.is_null() {
        return if len == 0 {
            Ok("")
        } else {
            Err(Error::invalid(format!("{what} is NULL but has a length.")))
        };
    }
    if len > max {
        return Err(Error::invalid(format!(
            "{what} is longer than {max} bytes."
        )));
    }
    // SAFETY: the caller guarantees `len` readable bytes at `ptr` (header contract).
    let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) };
    std::str::from_utf8(bytes).map_err(|_| Error::invalid(format!("{what} is not UTF-8.")))
}

/// Write an out-pointer; NULL out-pointers are refused before any work.
pub(crate) fn need<T>(out: *mut *mut T, what: &str) -> Result<()> {
    if out.is_null() {
        return Err(Error::invalid(format!("{what} must not be NULL.")));
    }
    Ok(())
}

pub(crate) fn write_len(len: *mut usize, n: usize) {
    if !len.is_null() {
        // SAFETY: a non-NULL length out-pointer is writable (header contract).
        unsafe { *len = n };
    }
}

pub(crate) type UserData = *mut c_void;

/// # Safety
/// `error` is NULL or came from libwol.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_error_get_code(error: *const wol_error) -> wol_status {
    // SAFETY: NULL or a live error object (contract).
    unsafe { error.as_ref() }.map_or(Status::InvalidArgument as wol_status, |e| e.status as i32)
}

/// # Safety
/// `error` is NULL or came from libwol; `len` is NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_error_get_message(
    error: *const wol_error,
    len: *mut usize,
) -> *const c_char {
    // SAFETY: NULL or a live error object (contract).
    match unsafe { error.as_ref() } {
        Some(e) => {
            write_len(len, e.message.len() - 1);
            e.message.as_ptr().cast()
        }
        None => {
            write_len(len, 0);
            c"".as_ptr()
        }
    }
}

/// # Safety
/// `error` is NULL or an error libwol handed out, released once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_error_release(error: *mut wol_error) {
    if !error.is_null() {
        // SAFETY: made by Box::into_raw in `guard`, released once (contract).
        drop(unsafe { Box::from_raw(error) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_fills_error_only_on_failure_and_catches_panics() {
        let mut e: *mut wol_error = std::ptr::null_mut();
        assert_eq!(guard(&mut e, || Ok(())), 0);
        assert!(e.is_null());
        assert_eq!(guard(&mut e, || Err(Error::invalid("bad\0x"))), 1);
        let mut n = 0;
        let m = unsafe { wol_error_get_message(e, &mut n) };
        let text = unsafe { std::slice::from_raw_parts(m.cast::<u8>(), n + 1) };
        assert_eq!(text, b"bad x\0");
        assert_eq!(unsafe { wol_error_get_code(e) }, 1);
        unsafe { wol_error_release(e) };
        let code = guard(std::ptr::null_mut(), || panic!("boom"));
        assert_eq!(code, Status::Internal as i32);
    }

    #[test]
    fn input_table() {
        let s = "héllo";
        let p = s.as_ptr().cast();
        assert_eq!(input(p, s.len(), 64, "x"), Ok("héllo"));
        assert_eq!(input(std::ptr::null(), 0, 64, "x"), Ok(""));
        assert!(input(std::ptr::null(), 3, 64, "x").is_err());
        assert!(input(p, s.len(), 2, "x").is_err());
        assert!(input(p, 2, 64, "x").is_err(), "cut inside é");
    }
}
