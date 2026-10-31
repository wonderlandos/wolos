// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! Network state (ADR 0020, 1.7): a service call.
use crate::context::wol_context;
use crate::error::{guard, wol_error, wol_status};
use crate::info::{copy_info, wol_info};
use shared::portal::Call;

/// # Safety
/// `context` is live; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_network_copy_info(
    context: *mut wol_context,
    out: *mut *mut wol_info,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || unsafe {
        copy_info(context, &Call::NetworkInfo {}, out)
    })
}
