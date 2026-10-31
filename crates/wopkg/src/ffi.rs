//! The C boundary (ADR 0021 on ADR 0020's rules): status codes, the error object, strings, and the
//! guards every export goes through: input checks, panics caught, out-pointers written only on success.
use std::ffi::{c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};

#[allow(non_camel_case_types)]
pub type wopkg_status = i32;

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok = 0,
    InvalidArgument = 1,
    PermissionDenied = 2,
    Unavailable = 3,
    Internal = 4,
    /// A hash, size or signature did not match: nothing was changed.
    Untrusted = 5,
    NotFound = 6,
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
    pub fn invalid(m: impl Into<String>) -> Self {
        Self::new(Status::InvalidArgument, m)
    }
    pub fn unavailable(m: impl Into<String>) -> Self {
        Self::new(Status::Unavailable, m)
    }
    pub fn untrusted(m: impl Into<String>) -> Self {
        Self::new(Status::Untrusted, m)
    }
    pub fn internal(m: impl Into<String>) -> Self {
        Self::new(Status::Internal, m)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Opaque error object; `wopkg_error_release` frees it.
#[allow(non_camel_case_types)]
pub struct wopkg_error {
    status: Status,
    message: Vec<u8>,
}

/// Opaque immutable UTF-8 string (a JSON document for most results) with a trailing NUL.
#[allow(non_camel_case_types)]
pub struct wopkg_string {
    bytes: Vec<u8>,
}

/// `void (*)(const char *json, size_t len, void *user_data)`: progress, on the calling thread.
#[allow(non_camel_case_types)]
pub type wopkg_progress_fn = Option<unsafe extern "C" fn(*const c_char, usize, *mut c_void)>;

/// A progress sink the library calls with one JSON event at a time.
pub type Progress<'a> = &'a mut dyn FnMut(&serde_json::Value);

pub(crate) fn progress_of(
    cb: wopkg_progress_fn,
    ud: *mut c_void,
) -> impl FnMut(&serde_json::Value) {
    let ud = ud as usize;
    move |v: &serde_json::Value| {
        if let Some(f) = cb {
            let s = v.to_string();
            // SAFETY: the caller's callback takes (bytes, len, user_data) (header contract).
            unsafe { f(s.as_ptr().cast(), s.len(), ud as *mut c_void) };
        }
    }
}

pub(crate) fn guard(error: *mut *mut wopkg_error, f: impl FnOnce() -> Result<()>) -> wopkg_status {
    let r = catch_unwind(AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(Error::internal("libwopkg hit an internal error.")));
    match r {
        Ok(()) => Status::Ok as wopkg_status,
        Err(e) => {
            if !error.is_null() {
                let mut message = e.message.replace('\0', " ").into_bytes();
                message.push(0);
                let obj = Box::new(wopkg_error {
                    status: e.status,
                    message,
                });
                // SAFETY: the caller passed a writable `wopkg_error **` (header contract).
                unsafe { *error = Box::into_raw(obj) };
            }
            e.status as wopkg_status
        }
    }
}

/// UTF-8 input with an explicit length; NULL only with length 0.
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
    // SAFETY: `len` readable bytes at `ptr` (header contract).
    let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) };
    std::str::from_utf8(bytes).map_err(|_| Error::invalid(format!("{what} is not UTF-8.")))
}

/// JSON request input (NULL/empty = `{}`).
pub(crate) fn request<T: serde::de::DeserializeOwned>(ptr: *const c_char, len: usize) -> Result<T> {
    let s = input(ptr, len, MAX_REQUEST, "The request")?;
    let s = if s.trim().is_empty() { "{}" } else { s };
    serde_json::from_str(s).map_err(|e| Error::invalid(format!("The request is not valid: {e}.")))
}

pub const MAX_REQUEST: usize = 4 << 20;

pub(crate) fn need<T>(out: *mut *mut T, what: &str) -> Result<()> {
    if out.is_null() {
        return Err(Error::invalid(format!("{what} must not be NULL.")));
    }
    Ok(())
}

/// Hand `text` out through `out` (checked non-NULL before).
pub(crate) fn give(out: *mut *mut wopkg_string, text: String) {
    let mut bytes = text.into_bytes();
    bytes.push(0);
    // SAFETY: checked with `need` by the caller.
    unsafe { *out = Box::into_raw(Box::new(wopkg_string { bytes })) };
}

pub(crate) fn give_json(out: *mut *mut wopkg_string, v: &impl serde::Serialize) -> Result<()> {
    give(
        out,
        serde_json::to_string(v).map_err(|_| Error::internal("A result could not be written."))?,
    );
    Ok(())
}

fn write_len(len: *mut usize, n: usize) {
    if !len.is_null() {
        // SAFETY: non-NULL length out-pointer is writable (contract).
        unsafe { *len = n };
    }
}

fn quiet<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

/// # Safety
/// `string` is NULL or live; `len` is NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_string_get_bytes(
    string: *const wopkg_string,
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
/// `string` is NULL or a string the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_string_release(string: *mut wopkg_string) {
    if !string.is_null() {
        // SAFETY: from Box::into_raw in `give` (contract).
        drop(unsafe { Box::from_raw(string) });
    }
}

/// # Safety
/// `error` is NULL or came from libwopkg.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_error_get_code(error: *const wopkg_error) -> wopkg_status {
    unsafe { error.as_ref() }.map_or(Status::InvalidArgument as wopkg_status, |e| e.status as i32)
}

/// # Safety
/// `error` is NULL or came from libwopkg; `len` is NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_error_get_message(
    error: *const wopkg_error,
    len: *mut usize,
) -> *const c_char {
    quiet(c"".as_ptr(), || match unsafe { error.as_ref() } {
        Some(e) => {
            write_len(len, e.message.len() - 1);
            e.message.as_ptr().cast()
        }
        None => {
            write_len(len, 0);
            c"".as_ptr()
        }
    })
}

/// # Safety
/// `error` is NULL or an error the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_error_release(error: *mut wopkg_error) {
    if !error.is_null() {
        // SAFETY: from Box::into_raw in `guard` (contract).
        drop(unsafe { Box::from_raw(error) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_and_strings_cross_the_boundary() {
        let mut e: *mut wopkg_error = std::ptr::null_mut();
        let rc = guard(&mut e, || Err(Error::untrusted("bad\0hash")));
        assert_eq!(rc, Status::Untrusted as i32);
        let mut n = 0;
        let m = unsafe { wopkg_error_get_message(e, &mut n) };
        let m = unsafe { std::slice::from_raw_parts(m.cast::<u8>(), n) };
        assert_eq!(m, b"bad hash");
        assert_eq!(unsafe { wopkg_error_get_code(e) }, 5);
        unsafe { wopkg_error_release(e) };
        let rc = guard(std::ptr::null_mut(), || panic!("boom"));
        assert_eq!(rc, Status::Internal as i32);
        let mut s: *mut wopkg_string = std::ptr::null_mut();
        give(&mut s, "{}".into());
        let p = unsafe { wopkg_string_get_bytes(s, &mut n) };
        assert_eq!(
            unsafe { std::slice::from_raw_parts(p.cast::<u8>(), n + 1) },
            b"{}\0"
        );
        unsafe { wopkg_string_release(s) };
        unsafe { wopkg_string_release(std::ptr::null_mut()) };
    }

    #[test]
    fn input_is_checked() {
        assert!(input(std::ptr::null(), 0, 4, "x").is_ok());
        assert!(input(std::ptr::null(), 1, 4, "x").is_err());
        assert!(input(c"hello".as_ptr(), 5, 4, "x").is_err());
        let bad = [0xffu8, 0];
        assert!(input(bad.as_ptr().cast(), 1, 4, "x").is_err());
    }
}
