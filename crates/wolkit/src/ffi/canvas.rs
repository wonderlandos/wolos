// SPDX-License-Identifier: BSD-2-Clause
//! `wolkit_window_canvas_submit`: a canvas node's drawing for the next frame, as a `WKC1` op
//! stream (format and limits in `gpu::canvas`). The stream is decoded and validated here, whole;
//! a bad stream is refused with one sentence and changes nothing. The decoded items are drawn by
//! the CPU rasteriser and the GPU renderer alike (one decoder, `gpu::canvas::Canvas::emit`).
//! Storing the canvas on its node is the window's job: until the native backend (WK-W4) takes
//! it, every backend answers that the window needs the native backend, after validating.

use super::window::wolkit_window;
use super::{borrow, bytes_in, guard, wolkit_error, wolkit_status};
use crate::error::Error;
use crate::gpu::canvas::Canvas;

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
        let bytes = unsafe { bytes_in(ops, len, MAX_OPS, "The canvas ops") }?;
        let canvas = Canvas::decode(bytes)?;
        let _ = (w.engine(), w.id(), node, canvas);
        Err(Error::unavailable(crate::copy::KIT_NO_CANVAS))
    })
}
