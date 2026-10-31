// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! App preferences (ADR 0020, 1.4): service calls. wold keeps them per account and executable, so
//! an app sees only its own and cannot name another's.
use crate::client;
use crate::context::wol_context;
use crate::error::{guard, input, need, wol_error, wol_status};
use crate::info::wol_info;
use crate::string::{borrow, into_raw, wol_string};
use shared::portal::{Call, MAX_PREF_KEY, MAX_PREF_VALUE};
use std::ffi::c_char;
use std::ptr::null_mut;

/// # Safety
/// `context` is live; `key` points at `key_len` bytes; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_prefs_copy_string(
    context: *mut wol_context,
    key: *const c_char,
    key_len: usize,
    out: *mut *mut wol_string,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let key = input(key, key_len, MAX_PREF_KEY, "The key")?.to_owned();
        let data = client::data(ctx.wold(), &Call::PrefsGet { key })?;
        let found = data.get("value").and_then(|v| v.as_str());
        unsafe { *out = found.map_or(null_mut(), |v| into_raw(wol_string::new(v))) };
        Ok(())
    })
}

/// # Safety
/// `context` is live; `key`, `value` point at their lengths; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_prefs_set_string(
    context: *mut wol_context,
    key: *const c_char,
    key_len: usize,
    value: *const c_char,
    value_len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let ctx = unsafe { borrow(context) }?;
        let key = input(key, key_len, MAX_PREF_KEY, "The key")?.to_owned();
        let value = input(value, value_len, MAX_PREF_VALUE, "The value")?.to_owned();
        client::run(ctx.wold(), &Call::PrefsSet { key, value })
    })
}

/// # Safety
/// `context` is live; `key` points at `key_len` bytes; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_prefs_remove(
    context: *mut wol_context,
    key: *const c_char,
    key_len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let ctx = unsafe { borrow(context) }?;
        let key = input(key, key_len, MAX_PREF_KEY, "The key")?.to_owned();
        client::run(ctx.wold(), &Call::PrefsRemove { key })
    })
}

/// # Safety
/// `context` is live; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_prefs_copy_keys(
    context: *mut wol_context,
    out: *mut *mut wol_info,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let data = client::data(ctx.wold(), &Call::PrefsKeys {})?;
        unsafe { *out = wol_info::out(&data) };
        Ok(())
    })
}
