// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! Clipboard (ADR 0020, 1.3): service calls. wold checks the grant and asks the person's desktop;
//! libwol never touches the session itself.
use crate::client;
use crate::context::wol_context;
use crate::error::{Error, Status, guard, input, need, wol_error, wol_status};
use crate::string::{borrow, into_raw, wol_string};
use shared::portal::{Call, MAX_CLIPBOARD};
use std::ffi::c_char;

/// # Safety
/// `context` is live; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_clipboard_copy_text(
    context: *mut wol_context,
    out: *mut *mut wol_string,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let data = client::data(ctx.wold(), &Call::ClipboardRead {})?;
        let text = data
            .get("text")
            .and_then(|t| t.as_str())
            .ok_or_else(|| Error::new(Status::Internal, "The clipboard answer was unreadable."))?;
        unsafe { *out = into_raw(wol_string::new(text)) };
        Ok(())
    })
}

/// # Safety
/// `context` is live; `text` points at `len` bytes; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_clipboard_set_text(
    context: *mut wol_context,
    text: *const c_char,
    len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let ctx = unsafe { borrow(context) }?;
        let text = input(text, len, MAX_CLIPBOARD, "The text")?.to_owned();
        client::run(ctx.wold(), &Call::ClipboardWrite { text })
    })
}
