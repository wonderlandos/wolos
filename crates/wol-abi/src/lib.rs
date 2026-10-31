//! libwol (ADR 0020, archicture/modules/abi.md): the WolOS app API. One C ABI (`include/wol.h`,
//! `libwol.so.1`, symbols versioned by `abi/wol.map`) over a Rust implementation; `tests/abi.rs`
//! fails on any removed or changed symbol or declaration.
//!
//! Rules at the boundary: opaque handles only; `_create`/`_copy` return a reference the caller
//! releases, `_get_` borrows from its argument; UTF-8 strings with lengths; a status code and an
//! optional error object from every call that can fail; callbacks take `user_data` and run only
//! inside `wol_context_dispatch`. The library holds no privilege: services go through wold.
#![allow(non_camel_case_types)]
// The C boundary takes a pointer and a length per string.
#![allow(clippy::too_many_arguments)]

pub mod client;
pub mod clipboard;
pub mod context;
pub mod error;
pub mod prefs;
pub mod info;
pub mod kit;
pub mod menu;
pub mod notify;
pub mod string;
pub mod system;

/// libwol's own version (not the WolOS version): major.minor.patch, packed 0xMMmmpp.
pub const VERSION: (u32, u32, u32) = (1, 4, 0);

/// Every `#define` in `wol.h` (test-checked against the header).
pub mod abi {
    use crate::error::Status;
    pub const CONSTANTS: &[(&str, i64)] = &[
        ("WOL_VERSION_MAJOR", crate::VERSION.0 as i64),
        ("WOL_VERSION_MINOR", crate::VERSION.1 as i64),
        ("WOL_VERSION_PATCH", crate::VERSION.2 as i64),
        ("WOL_OK", Status::Ok as i64),
        ("WOL_ERR_INVALID_ARGUMENT", Status::InvalidArgument as i64),
        ("WOL_ERR_PERMISSION_DENIED", Status::PermissionDenied as i64),
        ("WOL_ERR_UNAVAILABLE", Status::Unavailable as i64),
        ("WOL_ERR_INTERNAL", Status::Internal as i64),
        (
            "WOL_APPEARANCE_LIGHT",
            crate::system::Appearance::Light as i64,
        ),
        (
            "WOL_APPEARANCE_DARK",
            crate::system::Appearance::Dark as i64,
        ),
    ];
}

/// Runtime version, packed `major << 16 | minor << 8 | patch`.
#[unsafe(no_mangle)]
pub extern "C" fn wol_version() -> u32 {
    (VERSION.0 << 16) | (VERSION.1 << 8) | VERSION.2
}

/// The version as text, static.
#[unsafe(no_mangle)]
pub extern "C" fn wol_version_string() -> *const std::ffi::c_char {
    c"1.4.0".as_ptr()
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_agrees() {
        let s = unsafe { std::ffi::CStr::from_ptr(super::wol_version_string()) };
        let v = super::VERSION;
        assert_eq!(s.to_str().unwrap(), format!("{}.{}.{}", v.0, v.1, v.2));
        assert_eq!(super::wol_version(), (v.0 << 16) | (v.1 << 8) | v.2);
    }
}
