// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! Sharing (ADR 0020, 1.13): the person sees what an app wants to share and picks where it goes.
//! The app learns only what was done.
use crate::client;
use crate::context::wol_context;
use crate::error::{Error, Status, guard, input, need, wol_error, wol_status};
use crate::file::PANEL_WAIT;
use crate::kit::arc_of;
use crate::string::{into_raw, wol_string};
use shared::portal::{Call, MAX_MESSAGE, MAX_TITLE};
use std::ffi::c_char;

/// # Safety
/// `context` is live; each pointer has its length; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_share_text(
    context: *mut wol_context,
    title: *const c_char,
    title_len: usize,
    text: *const c_char,
    text_len: usize,
    out: *mut *mut wol_string,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let context = arc_of(context)?;
        let c = Call::ShareText {
            title: input(title, title_len, MAX_TITLE, "The title")?.to_owned(),
            text: input(text, text_len, MAX_MESSAGE, "The text")?.to_owned(),
        };
        let (a, _) = client::call_for(context.wold(), &c, PANEL_WAIT);
        if !a.ok {
            return Err(Error::new(
                a.code.map_or(Status::Internal, Into::into),
                a.message.unwrap_or_default(),
            ));
        }
        let data = a.data.unwrap_or_default();
        let done = data.get("done").and_then(|d| d.as_str());
        unsafe { *out = into_raw(wol_string::new(done.unwrap_or("cancelled"))) };
        Ok(())
    })
}
