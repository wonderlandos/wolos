// SPDX-License-Identifier: BSD-2-Clause
//! `wolkit_image_*`: image resources. The engine keeps the pixels (or, for an encoded image, the
//! bytes until a worker decodes them); a node refers to an image by id (`WOLKIT_PROP_IMAGE_ID`).
//! Limits: 16384 px a side, 256 megapixels, 512 MB per engine. Decoding encoded formats lands
//! with the paint stage (WK-W3); until then an encoded image is checked and kept.

use super::engine::wolkit_engine;
use super::{
    borrow, bytes_in, guard, handle, into_raw, need, quiet, release, retain, wolkit_error,
    wolkit_status,
};
use crate::error::Error;
use crate::scene::ids::Ix;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub const MAX_SIDE: u32 = 16_384;
pub const MAX_PIXELS: u64 = 256 * 1024 * 1024;
pub const MAX_ENCODED: usize = 64 << 20;
pub const MAX_TOTAL: u64 = 512 << 20;

static NEXT: AtomicU32 = AtomicU32::new(1);

pub enum Data {
    Rgba { w: u32, h: u32, pixels: Vec<u8> },
    Encoded(Vec<u8>),
}

#[repr(C)]
#[allow(non_camel_case_types)] // the C name
pub struct wolkit_image {
    tag: handle::Tag<wolkit_image>,
    id: u32,
    data: Data,
    held: Arc<AtomicU64>,
}

handle::tagged!(wolkit_image, 0x57_4f_4c_4b_49_54_49_01);

// Releases the image's bytes from the engine-wide budget.
impl Drop for wolkit_image {
    fn drop(&mut self) {
        let n = match &self.data {
            Data::Rgba { pixels, .. } => pixels.len(),
            Data::Encoded(b) => b.len(),
        };
        self.held.fetch_sub(n as u64, Ordering::AcqRel);
    }
}

/// Encoded formats the engine accepts, by their first bytes.
fn sniff(b: &[u8]) -> bool {
    b.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a])
        || b.starts_with(&[0xff, 0xd8, 0xff])
        || b.iter()
            .take(256)
            .skip_while(|c| c.is_ascii_whitespace())
            .take(5)
            .copied()
            .eq(*b"<?xml")
        || b.iter()
            .take(256)
            .skip_while(|c| c.is_ascii_whitespace())
            .take(4)
            .copied()
            .eq(*b"<svg")
}

/// Bytes held by all images of every engine in the process, a coarse stand-in for the per-engine
/// budget until the engine owns the count.
static HELD: std::sync::OnceLock<Arc<AtomicU64>> = std::sync::OnceLock::new();

fn held() -> Arc<AtomicU64> {
    HELD.get_or_init(|| Arc::new(AtomicU64::new(0))).clone()
}

fn reserve(n: u64) -> Result<Arc<AtomicU64>, Error> {
    let h = held();
    if h.fetch_add(n, Ordering::AcqRel) + n > MAX_TOTAL {
        h.fetch_sub(n, Ordering::AcqRel);
        return Err(Error::limit(
            "Images take at most 512 MB; release some first.",
        ));
    }
    Ok(h)
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
    guard(error, || {
        need(out, "out")?;
        // SAFETY: live (contract).
        let _ = unsafe { borrow(engine) }?;
        // SAFETY: pointer and length are the caller's (contract).
        let b = unsafe { bytes_in(encoded, len, MAX_ENCODED, "An image") }?;
        if !sniff(b) {
            return Err(Error::invalid("An image is a PNG, a JPEG or an SVG file."));
        }
        let held = reserve(b.len() as u64)?;
        let img = Arc::new(wolkit_image {
            tag: handle::Tag::new(),
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            data: Data::Encoded(b.to_vec()),
            held,
        });
        // SAFETY: checked non-NULL above.
        unsafe { *out = into_raw(img) };
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
        if w == 0
            || h == 0
            || w > MAX_SIDE
            || h > MAX_SIDE
            || u64::from(w) * u64::from(h) > MAX_PIXELS
        {
            return Err(Error::limit(
                "An image is at most 16384 pixels a side and 256 megapixels.",
            ));
        }
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
        let held = reserve(u64::from(w) * u64::from(h) * 4)?;
        // Tightly packed copy.
        let mut packed = Vec::with_capacity(row * h.ix());
        for y in 0..h.ix() {
            packed.extend_from_slice(&b[y * stride.ix()..y * stride.ix() + row]);
        }
        let img = Arc::new(wolkit_image {
            tag: handle::Tag::new(),
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            data: Data::Rgba {
                w,
                h,
                pixels: packed,
            },
            held,
        });
        // SAFETY: checked non-NULL above.
        unsafe { *out = into_raw(img) };
        Ok(())
    })
}

/// # Safety
/// `image` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_image_get_id(image: *const wolkit_image) -> u32 {
    // SAFETY: NULL or live (contract).
    quiet(0, || unsafe { borrow(image) }.map_or(0, |i| i.id))
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
