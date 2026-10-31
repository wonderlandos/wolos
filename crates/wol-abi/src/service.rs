// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! An app's background jobs (ADR 0020, 1.14). wold runs the app's own executable as the account,
//! so a name is all an app gives.
use crate::client;
use crate::context::wol_context;
use crate::error::{guard, input, wol_error, wol_status};
use crate::info::{copy_info, wol_info};
use crate::kit::arc_of;
use shared::portal::{Call, MAX_ENDPOINT};
use std::ffi::c_char;

/// # Safety
/// `context` is live; `name` points at `len` bytes; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_service_start(
    context: *mut wol_context,
    name: *const c_char,
    len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let context = arc_of(context)?;
        let name = input(name, len, MAX_ENDPOINT, "The name")?.to_owned();
        client::run(context.wold(), &Call::ServiceStart { name })
    })
}

/// # Safety
/// `context` is live; `name` points at `len` bytes; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_service_stop(
    context: *mut wol_context,
    name: *const c_char,
    len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let context = arc_of(context)?;
        let name = input(name, len, MAX_ENDPOINT, "The name")?.to_owned();
        client::run(context.wold(), &Call::ServiceStop { name })
    })
}

/// # Safety
/// `context` is live; `name` points at `len` bytes; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_service_copy_status(
    context: *mut wol_context,
    name: *const c_char,
    len: usize,
    out: *mut *mut wol_info,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let name = input(name, len, MAX_ENDPOINT, "The name")?.to_owned();
        unsafe { copy_info(context, &Call::ServiceStatus { name }, out) }
    })
}
