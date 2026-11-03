// SPDX-License-Identifier: BSD-2-Clause
//! `wolkit_window_*`: windows, the patch verb and the headless test hooks.

use super::engine::wolkit_engine;
use super::style::wolkit_style_table;
use super::{
    borrow, bytes_in, bytes_out, give, guard, guard_status, handle, into_raw, need, put, quiet,
    release, retain, text_in, wolkit_error, wolkit_status,
};
use crate::engine::Engine;
use crate::error::Error;
use crate::scene::patch::MAX_PATCH;
use crate::window::{KeyInput, PointerInput};
use std::ffi::c_char;
use std::sync::Arc;

#[repr(C)]
#[allow(non_camel_case_types)] // the C name
pub struct wolkit_window {
    tag: handle::Tag<wolkit_window>,
    engine: Arc<Engine>,
    id: u32,
}

handle::tagged!(wolkit_window, 0x57_4f_4c_4b_49_54_57_01);

// Closes the window in its engine when the last reference goes.
impl Drop for wolkit_window {
    fn drop(&mut self) {
        // The last reference to a window closes it.
        self.engine.close_window(self.id);
    }
}

impl wolkit_window {
    pub(crate) fn engine(&self) -> &Arc<Engine> {
        &self.engine
    }
    pub(crate) fn id(&self) -> u32 {
        self.id
    }
}

/// # Safety
/// `engine` is live; `title` points at `title_len` readable bytes; `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_create(
    engine: *mut wolkit_engine,
    title: *const c_char,
    title_len: usize,
    width: u32,
    height: u32,
    flags: u32,
    out: *mut *mut wolkit_window,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        need(out, "out")?;
        // SAFETY: live (contract).
        let e = unsafe { borrow(engine) }?;
        // SAFETY: pointer and length are the caller's (contract).
        let t = unsafe { text_in(title, title_len, 256, "The title") }?;
        let id = e.engine.create_window(t, width, height, flags)?;
        let w = Arc::new(wolkit_window {
            tag: handle::Tag::new(),
            engine: e.engine.clone(),
            id,
        });
        // SAFETY: checked non-NULL above.
        unsafe { *out = into_raw(w) };
        Ok(())
    })
}

/// # Safety
/// `window` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_retain(window: *mut wolkit_window) -> *mut wolkit_window {
    // SAFETY: NULL or live (contract).
    unsafe { retain(window) }
}

/// # Safety
/// `window` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_release(window: *mut wolkit_window) {
    // SAFETY: NULL or a reference the caller owns (contract).
    unsafe { release(window) }
}

/// # Safety
/// `window` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_get_id(window: *const wolkit_window) -> u32 {
    // SAFETY: NULL or live (contract).
    quiet(0, || unsafe { borrow(window) }.map_or(0, |w| w.id))
}

macro_rules! window_call {
    ($window:ident, $error:ident, |$w:ident| $body:expr) => {
        guard($error, || {
            // SAFETY: live (contract).
            let $w = unsafe { borrow($window) }?;
            $body
        })
    };
}

/// # Safety
/// `window` is live; `title` points at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_set_title(
    window: *mut wolkit_window,
    title: *const c_char,
    len: usize,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    window_call!(window, error, |w| {
        // SAFETY: pointer and length are the caller's (contract).
        let t = unsafe { text_in(title, len, 256, "The title") }?;
        w.engine.set_title(w.id, t)
    })
}

/// # Safety
/// `window` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_set_size_limits(
    window: *mut wolkit_window,
    min_w: u32,
    min_h: u32,
    max_w: u32,
    max_h: u32,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    window_call!(window, error, |w| w.engine.set_size_limits(
        w.id,
        (min_w, min_h),
        (max_w, max_h)
    ))
}

/// # Safety
/// `window` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_show(
    window: *mut wolkit_window,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    window_call!(window, error, |w| w.engine.show(w.id))
}

/// # Safety
/// `window` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_close(
    window: *mut wolkit_window,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    window_call!(window, error, |w| {
        w.engine.close_window(w.id);
        Ok(())
    })
}

/// # Safety
/// `window` is live; `patch` points at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_apply(
    window: *mut wolkit_window,
    patch: *const u8,
    len: usize,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    window_call!(window, error, |w| {
        // SAFETY: pointer and length are the caller's (contract).
        let bytes = unsafe { bytes_in(patch, len, MAX_PATCH, "A patch") }?;
        w.engine.apply(w.id, bytes)
    })
}

/// # Safety
/// `window` is live; `buf` has `cap` writable bytes; `needed` is writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_read_text(
    window: *const wolkit_window,
    node: u32,
    buf: *mut u8,
    cap: usize,
    needed: *mut usize,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    window_call!(window, error, |w| {
        // SAFETY: the buffer is the caller's (contract).
        let out = unsafe { bytes_out(buf, cap, "The text buffer") }?;
        let n = w.engine.read_text(w.id, node, out)?;
        put(needed, n);
        if n > cap {
            return Err(Error::limit(format!(
                "The text needs {n} bytes; the buffer has {cap}."
            )));
        }
        Ok(())
    })
}

/// # Safety
/// `window` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_set_focus(
    window: *mut wolkit_window,
    node: u32,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    window_call!(window, error, |w| w.engine.set_focus(w.id, node))
}

/// # Safety
/// `window` is live; the out-pointers are writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_get_size(
    window: *const wolkit_window,
    w: *mut u32,
    h: *mut u32,
    scale: *mut f32,
) -> wolkit_status {
    guard_status(|| {
        // SAFETY: live (contract).
        let win = unsafe { borrow(window) }?;
        let (ww, hh, s) = win.engine.size(win.id)?;
        put(w, ww);
        put(h, hh);
        put(scale, s);
        Ok(())
    })
}

/// # Safety
/// `window` is live; `xywh` has 4 writable floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_node_rect(
    window: *const wolkit_window,
    node: u32,
    xywh: *mut f32,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    window_call!(window, error, |w| {
        need(xywh, "xywh")?;
        let r = w.engine.node_rect(w.id, node)?;
        for (i, v) in r.iter().enumerate() {
            // SAFETY: 4 writable floats (contract).
            unsafe { xywh.add(i).write(*v) };
        }
        Ok(())
    })
}

/// # Safety
/// `window` is live; `node` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_hit_test(
    window: *const wolkit_window,
    x: f32,
    y: f32,
    node: *mut u32,
) -> wolkit_status {
    guard_status(|| {
        need(node, "node")?;
        // SAFETY: live (contract).
        let w = unsafe { borrow(window) }?;
        let n = w.engine.hit_test(w.id, x, y)?;
        put(node, n);
        Ok(())
    })
}

/// # Safety
/// `window` is live; `buf` has `cap` writable bytes; `needed` is writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_dump(
    window: *mut wolkit_window,
    what: u32,
    buf: *mut u8,
    cap: usize,
    needed: *mut usize,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    window_call!(window, error, |w| {
        let text = w.engine.dump(w.id, what)?;
        // SAFETY: the buffer is the caller's (contract).
        unsafe { give(buf, cap, needed, text.as_bytes(), "The dump") }
    })
}

/// # Safety
/// `window` is live; `pixels` has `cap` writable bytes; `w` and `h` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_render_rgba(
    window: *mut wolkit_window,
    pixels: *mut u8,
    cap: usize,
    w: *mut u32,
    h: *mut u32,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    window_call!(window, error, |win| {
        need(w, "w")?;
        need(h, "h")?;
        // SAFETY: the buffer is the caller's (contract).
        let out = unsafe { bytes_out(pixels, cap, "The pixel buffer") }?;
        let (pw, ph) = win.engine.render_rgba(win.id, out)?;
        put(w, pw);
        put(h, ph);
        Ok(())
    })
}

/// # Safety
/// `window` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_inject_pointer(
    window: *mut wolkit_window,
    kind: u32,
    x: f32,
    y: f32,
    button: u32,
    mods: u32,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    window_call!(window, error, |w| w.engine.inject_pointer(
        w.id,
        PointerInput {
            kind,
            x,
            y,
            button,
            mods
        }
    ))
}

/// # Safety
/// `window` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_inject_key(
    window: *mut wolkit_window,
    keysym: u32,
    pressed: u32,
    mods: u32,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    window_call!(window, error, |w| w.engine.inject_key(
        w.id,
        KeyInput {
            keysym,
            pressed: pressed != 0,
            mods
        }
    ))
}

/// # Safety
/// `window` is live; `text` points at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_inject_text(
    window: *mut wolkit_window,
    text: *const c_char,
    len: usize,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    window_call!(window, error, |w| {
        // SAFETY: pointer and length are the caller's (contract).
        let t = unsafe { text_in(text, len, 64 * 1024, "The text") }?;
        w.engine.inject_text(w.id, t)
    })
}

/// # Safety
/// `window` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_advance_time(
    window: *mut wolkit_window,
    ns: u64,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    window_call!(window, error, |w| w.engine.advance_time(w.id, ns))
}

/// # Safety
/// `window` and `table` are live (`table` may be NULL to go back to the default).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_window_set_style_table(
    window: *mut wolkit_window,
    table: *mut wolkit_style_table,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    window_call!(window, error, |w| {
        let t = if table.is_null() {
            None
        } else {
            // SAFETY: live (contract).
            Some(unsafe { borrow(table.cast_const()) }?.table())
        };
        w.engine.set_style_table(w.id, t)
    })
}
