// SPDX-License-Identifier: BSD-2-Clause
//! `wolkit_image_*`: picture resources (`crate::image`). A handle holds one picture; a node shows
//! it by id (`WOLKIT_PROP_IMAGE_ID`). Encoded pictures (PNG, JPEG) are decoded on the engine's
//! workers with the EXIF orientation applied; until then `wolkit_image_get_state` says PENDING
//! and the node shows a placeholder. Limits: 16384 px a side, 256 megapixels, 64 MiB encoded,
//! 512 MB of pictures per process.
//!
//! 1.0: `create` (encoded, no cap), `create_rgba`, `get_id`, `retain`, `release`.
//! 1.1: `create_encoded` and `create_file` (with a cap on the longer side), `get_size`,
//! `get_state`.

use super::engine::wolkit_engine;
use super::{
    borrow, bytes_in, guard, guard_status, handle, into_raw, need, put, quiet, release, retain,
    text_in, wolkit_error, wolkit_status,
};
use crate::error::{Error, Result};
use crate::image::{Entry, MAX_ENCODED, MAX_SIDE, STATE_FAILED, source};
use crate::scene::ids::Ix;
use std::ffi::c_char;
use std::sync::Arc;

#[repr(C)]
#[allow(non_camel_case_types)] // the C name
pub struct wolkit_image {
    tag: handle::Tag<wolkit_image>,
    entry: Arc<Entry>,
}

handle::tagged!(wolkit_image, 0x57_4f_4c_4b_49_54_49_01);

fn wrap(entry: Arc<Entry>, out: *mut *mut wolkit_image) {
    let img = Arc::new(wolkit_image {
        tag: handle::Tag::new(),
        entry,
    });
    put(out, into_raw(img));
}

fn cap(max_side: u32) -> Result<u32> {
    if max_side > MAX_SIDE {
        return Err(Error::invalid(
            "A size cap is at most 16384 pixels (0 means none).",
        ));
    }
    Ok(max_side)
}

/// # Safety
/// `engine` is live; `encoded` points at `len` readable bytes; `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_image_create(
    engine: *mut wolkit_engine,
    encoded: *const u8,
    len: usize,
    out: *mut *mut wolkit_image,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    // SAFETY: forwarded contract; 0 is "no cap".
    unsafe { wolkit_image_create_encoded(engine, encoded, len, 0, out, error) }
}

/// # Safety
/// `engine` is live; `encoded` points at `len` readable bytes; `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_image_create_encoded(
    engine: *mut wolkit_engine,
    encoded: *const u8,
    len: usize,
    max_side: u32,
    out: *mut *mut wolkit_image,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        need(out, "out")?;
        // SAFETY: live (contract).
        let _ = unsafe { borrow(engine) }?;
        let max_side = cap(max_side)?;
        // SAFETY: pointer and length are the caller's (contract).
        let b = unsafe { bytes_in(encoded, len, MAX_ENCODED, "An image") }?;
        wrap(Entry::from_encoded(b.to_vec(), max_side)?, out);
        Ok(())
    })
}

/// # Safety
/// `engine` is live; `path` points at `len` readable bytes; `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_image_create_file(
    engine: *mut wolkit_engine,
    path: *const c_char,
    len: usize,
    max_side: u32,
    out: *mut *mut wolkit_image,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        need(out, "out")?;
        // SAFETY: live (contract).
        let _ = unsafe { borrow(engine) }?;
        let max_side = cap(max_side)?;
        // SAFETY: pointer and length are the caller's (contract).
        let p = unsafe { text_in(path, len, 4096, "A picture path") }?;
        let Some(p) = source::path_of(p) else {
            return Err(Error::invalid(
                "A picture path is absolute, like /home/me/Pictures/a.jpg.",
            ));
        };
        wrap(Entry::from_file(p.into(), max_side), out);
        Ok(())
    })
}

/// # Safety
/// `engine` is live; `pixels` points at `len` readable bytes; `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_image_create_rgba(
    engine: *mut wolkit_engine,
    w: u32,
    h: u32,
    stride: u32,
    pixels: *const u8,
    len: usize,
    out: *mut *mut wolkit_image,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        need(out, "out")?;
        // SAFETY: live (contract).
        let _ = unsafe { borrow(engine) }?;
        crate::image::check_size(w, h)?;
        let row = w.ix() * 4;
        if (stride.ix()) < row {
            return Err(Error::invalid(
                "An image row is at least width times 4 bytes.",
            ));
        }
        let need_len = (stride.ix())
            .checked_mul(h.ix() - 1)
            .and_then(|n| n.checked_add(row));
        // SAFETY: pointer and length are the caller's (contract).
        let b = unsafe { bytes_in(pixels, len, usize::MAX, "The pixels") }?;
        if need_len.is_none_or(|n| b.len() < n) {
            return Err(Error::invalid(
                "The pixel buffer is shorter than stride times height.",
            ));
        }
        wrap(Entry::from_rgba(w, h, stride.ix(), b)?, out);
        Ok(())
    })
}

/// # Safety
/// `image` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_image_get_id(image: *const wolkit_image) -> u32 {
    // SAFETY: NULL or live (contract).
    quiet(0, || unsafe { borrow(image) }.map_or(0, |i| i.entry.id))
}

/// # Safety
/// `image` is live; `w` and `h` are NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_image_get_size(
    image: *const wolkit_image,
    w: *mut u32,
    h: *mut u32,
) -> wolkit_status {
    guard_status(|| {
        // SAFETY: live (contract).
        let i = unsafe { borrow(image) }?;
        let (ww, hh) = i.entry.natural().ok_or_else(|| {
            Error::unavailable("The picture's size is known once its file has been read.")
        })?;
        put(w, ww);
        put(h, hh);
        Ok(())
    })
}

/// # Safety
/// `image` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_image_get_state(image: *const wolkit_image) -> u32 {
    // SAFETY: NULL or live (contract).
    quiet(STATE_FAILED, || {
        unsafe { borrow(image) }.map_or(STATE_FAILED, |i| i.entry.state_code())
    })
}

/// # Safety
/// `image` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_image_retain(image: *mut wolkit_image) -> *mut wolkit_image {
    // SAFETY: NULL or live (contract).
    unsafe { retain(image) }
}

/// # Safety
/// `image` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_image_release(image: *mut wolkit_image) {
    // SAFETY: NULL or a reference the caller owns (contract).
    unsafe { release(image) }
}
