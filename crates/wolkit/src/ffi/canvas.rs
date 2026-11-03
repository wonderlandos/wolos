// SPDX-License-Identifier: BSD-2-Clause
//! `wolkit_window_canvas_submit`: the canvas op stream. W1 made this stub; WK-W5 fills it (the op
//! decoder is shared with the CPU rasteriser, so the function belongs to whoever owns that format).

use super::window::wolkit_window;
use super::{borrow, bytes_in, guard, wolkit_error, wolkit_status};
use crate::error::Error;

/// Longest canvas op stream per call.
pub const MAX_OPS: usize = 4 << 20;

/// # Safety
/// `window` is live; `ops` points at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_canvas_submit(
    window: *mut wolkit_window,
    node: u32,
    ops: *const u8,
    len: usize,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        // SAFETY: live (contract).
        let w = unsafe { borrow(window) }?;
        // SAFETY: pointer and length are the caller's (contract).
        let _ = unsafe { bytes_in(ops, len, MAX_OPS, "The canvas ops") }?;
        let _ = (w.engine(), w.id(), node);
        Err(Error::unavailable("This window needs the native backend to draw on a canvas."))
    })
}
