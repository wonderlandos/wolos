// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! Audio (ADR 0020, 1.8): service calls. wold checks the grant and the desktop acts: sound belongs
//! to the person's session, not to the system.
use crate::client;
use crate::context::wol_context;
use crate::error::{Error, guard, wol_error, wol_status};
use crate::info::{copy_info, wol_info};
use crate::string::borrow;
use shared::portal::Call;

/// # Safety
/// `context` is live; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_audio_copy_info(
    context: *mut wol_context,
    out: *mut *mut wol_info,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || unsafe {
        copy_info(context, &Call::AudioInfo {}, out)
    })
}

/// # Safety
/// `context` is live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_audio_set_volume(
    context: *mut wol_context,
    percent: u32,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let ctx = unsafe { borrow(context) }?;
        let percent = u8::try_from(percent)
            .ok()
            .filter(|p| *p <= 100)
            .ok_or_else(|| Error::invalid("A volume is 0 to 100."))?;
        client::run(ctx.wold(), &Call::AudioSetVolume { percent })
    })
}

/// # Safety
/// `context` is live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_audio_set_muted(
    context: *mut wol_context,
    muted: i32,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let ctx = unsafe { borrow(context) }?;
        let muted = muted != 0;
        client::run(ctx.wold(), &Call::AudioSetMuted { muted })
    })
}
