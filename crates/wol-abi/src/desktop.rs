// SPDX-License-Identifier: BSD-2-Clause
//! Things an app asks the desktop to do as the person (ADR 0020, 1.16): add a recent file, let the
//! person pick a colour on the screen, print a file. wold checks the grant, the desktop shows what
//! needs the person, and the app learns only what was done.
use crate::client;
use crate::context::wol_context;
use crate::error::{Error, Status, guard, input, need, wol_error, wol_status};
use crate::file::PANEL_WAIT;
use crate::kit::arc_of;
use crate::string::{into_raw, wol_string};
use shared::portal::{Call, MAX_PATH};
use std::ffi::c_char;

/// # Safety
/// `context` is live; `path` points at `len` bytes; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_recent_add(
    context: *mut wol_context,
    path: *const c_char,
    len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let ctx = arc_of(context)?;
        let path = input(path, len, MAX_PATH, "The path")?.to_owned();
        client::run(ctx.wold(), &Call::RecentAdd { path })
    })
}

fn waited(ctx: &wol_context, c: &Call) -> crate::error::Result<serde_json::Value> {
    let (a, _) = client::call_for(ctx.wold(), c, PANEL_WAIT);
    if !a.ok {
        return Err(Error::new(
            a.code.map_or(Status::Internal, Into::into),
            a.message.unwrap_or_default(),
        ));
    }
    Ok(a.data.unwrap_or_default())
}

/// `#rrggbb` -> 0xRRGGBB.
fn rgb_of(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#').filter(|h| h.len() == 6)?;
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

/// # Safety
/// `context` is live; `rgb` and `picked` writable; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_color_pick(
    context: *mut wol_context,
    rgb: *mut u32,
    picked: *mut i32,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        if rgb.is_null() || picked.is_null() {
            return Err(Error::invalid("rgb and picked must not be NULL."));
        }
        let ctx = arc_of(context)?;
        let data = waited(&ctx, &Call::ColorPick {})?;
        let color = data.get("color").and_then(|c| c.as_str()).and_then(rgb_of);
        // SAFETY: both were checked non-NULL above and are writable (header contract).
        unsafe {
            *picked = i32::from(color.is_some());
            *rgb = color.unwrap_or(0);
        }
        Ok(())
    })
}

/// # Safety
/// `context` is live; `path` points at `len` bytes; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_print_file(
    context: *mut wol_context,
    path: *const c_char,
    len: usize,
    out: *mut *mut wol_string,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = arc_of(context)?;
        let path = input(path, len, MAX_PATH, "The path")?.to_owned();
        let data = waited(&ctx, &Call::PrintFile { path })?;
        let done = data.get("done").and_then(|d| d.as_str());
        // SAFETY: `out` was checked non-NULL above and is writable (header contract).
        unsafe { *out = into_raw(wol_string::new(done.unwrap_or("cancelled"))) };
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::rgb_of;

    #[test]
    fn a_colour_is_six_hex_digits_after_a_hash() {
        assert_eq!(rgb_of("#336699"), Some(0x33_66_99));
        assert_eq!(rgb_of("#FFFFFF"), Some(0xff_ff_ff));
        for bad in [
            "336699", "#33669", "#3366999", "#33669g", "# 36699", "#+36699", "",
        ] {
            assert_eq!(rgb_of(bad), None, "{bad}");
        }
    }
}
