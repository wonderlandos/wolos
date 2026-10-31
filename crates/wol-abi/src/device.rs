// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! Devices (ADR 0020, 1.12): a list, and a USB device opened by wold under a grant for that
//! device. The descriptor comes back as a `wol_file`; the app never gets a path.
use crate::context::wol_context;
use crate::error::{Error, guard, need, wol_error, wol_status};
use crate::file::{panel, wol_file};
use crate::info::{copy_info, wol_info};
use crate::kit::arc_of;
use shared::portal::Call;

/// # Safety
/// `context` is live; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_device_copy_list(
    context: *mut wol_context,
    out: *mut *mut wol_info,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || unsafe {
        copy_info(context, &Call::DeviceList {}, out)
    })
}

/// # Safety
/// `context` is live; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_device_open(
    context: *mut wol_context,
    vendor: u32,
    product: u32,
    out: *mut *mut wol_file,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let context = arc_of(context)?;
        let c = Call::DeviceOpen { vendor, product };
        c.validate().map_err(Error::invalid)?;
        let f = panel(&context, &c)?;
        unsafe { *out = f };
        Ok(())
    })
}
