// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! Power (ADR 0020, 1.6): service calls. A keep-awake lasts as long as its connection to wold, so
//! a process that dies gives it up without any cleanup.
use crate::client;
use crate::context::wol_context;
use crate::error::{guard, input, need, wol_error, wol_status};
use crate::info::{copy_info, wol_info};
use crate::string::{borrow, into_raw, release, retain};
use shared::portal::{Call, MAX_TITLE};
use std::ffi::c_char;
use std::os::unix::net::UnixStream;
use std::sync::Arc;

pub struct wol_assertion {
    _held: UnixStream,
}

/// # Safety
/// `context` is live; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_power_copy_info(
    context: *mut wol_context,
    out: *mut *mut wol_info,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || unsafe {
        copy_info(context, &Call::PowerInfo {}, out)
    })
}

/// # Safety
/// `context` is live; `reason` points at `reason_len` bytes; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_power_keep_awake(
    context: *mut wol_context,
    reason: *const c_char,
    reason_len: usize,
    out: *mut *mut wol_assertion,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let reason = input(reason, reason_len, MAX_TITLE, "The reason")?.to_owned();
        let held = client::hold(ctx.wold(), &Call::PowerKeepAwake { reason })?;
        unsafe { *out = into_raw(Arc::new(wol_assertion { _held: held })) };
        Ok(())
    })
}

/// # Safety
/// `assertion` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_assertion_retain(assertion: *mut wol_assertion) -> *mut wol_assertion {
    unsafe { retain(assertion) }
}

/// # Safety
/// `assertion` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_assertion_release(assertion: *mut wol_assertion) {
    unsafe { release(assertion) }
}
