// SPDX-License-Identifier: BSD-2-Clause
//! `wolkit_engine_*`: the engine handle, dispatch and the event stream.

use super::{borrow, bytes_out, give, guard, guard_status, handle, into_raw, need, put, quiet, release, retain, text_in, wolkit_error, wolkit_status};
use crate::engine::Engine;
use crate::error::{Error, Status};
use std::ffi::{c_char, c_int};
use std::sync::Arc;

#[repr(C)]
pub struct wolkit_engine {
    tag: handle::Tag<wolkit_engine>,
    pub(crate) engine: Arc<Engine>,
}

handle::tagged!(wolkit_engine, 0x57_4f_4c_4b_49_54_45_01);

/// `WOLKIT_VERSION_MAJOR << 16 | MINOR << 8 | PATCH`.
#[unsafe(no_mangle)]
pub extern "C" fn wolkit_version() -> u32 {
    (crate::VERSION.0 << 16) | (crate::VERSION.1 << 8) | crate::VERSION.2
}

#[unsafe(no_mangle)]
pub extern "C" fn wolkit_version_string() -> *const c_char {
    c"1.0.0".as_ptr()
}

/// # Safety
/// `app_id` points at `len` readable bytes; `out` is writable; `error` is writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_create(
    app_id: *const c_char,
    len: usize,
    flags: u32,
    out: *mut *mut wolkit_engine,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        need(out, "out")?;
        // SAFETY: the pointer and length are the caller's (contract).
        let id = unsafe { text_in(app_id, len, 128, "The app id") }?;
        let engine = Engine::new(id, flags)?;
        let h = Arc::new(wolkit_engine { tag: handle::Tag::new(), engine });
        // SAFETY: checked non-NULL above.
        unsafe { *out = into_raw(h) };
        Ok(())
    })
}

/// # Safety
/// `engine` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_retain(engine: *mut wolkit_engine) -> *mut wolkit_engine {
    // SAFETY: NULL or a live handle (contract).
    unsafe { retain(engine) }
}

/// # Safety
/// `engine` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_release(engine: *mut wolkit_engine) {
    // SAFETY: NULL or a reference the caller owns (contract).
    unsafe { release(engine) }
}

/// # Safety
/// `engine` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_get_fd(engine: *const wolkit_engine) -> c_int {
    // SAFETY: NULL or live (contract).
    quiet(-1, || unsafe { borrow(engine) }.map_or(-1, |e| e.engine.fd()))
}

/// # Safety
/// `engine` is live; `flags` is writable or NULL; `error` is writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_dispatch(
    engine: *mut wolkit_engine,
    timeout_ms: i32,
    flags: *mut u32,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        // SAFETY: live (contract).
        let e = unsafe { borrow(engine) }?;
        let f = e.engine.dispatch(timeout_ms)?;
        put(flags, f);
        Ok(())
    })
}

/// # Safety
/// `engine` is live; `buf` has `cap` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_read_events(engine: *mut wolkit_engine, buf: *mut u8, cap: usize) -> usize {
    quiet(0, || {
        // SAFETY: live (contract).
        let Ok(e) = (unsafe { borrow(engine) }) else { return 0 };
        // SAFETY: `cap` writable bytes (contract).
        let Ok(out) = (unsafe { bytes_out(buf, cap, "The event buffer") }) else { return 0 };
        e.engine.read_events(out)
    })
}

/// # Safety
/// `engine` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_get_info(engine: *const wolkit_engine, key: u32) -> u64 {
    // SAFETY: NULL or live (contract).
    quiet(0, || unsafe { borrow(engine) }.map_or(0, |e| e.engine.info(key)))
}

/// # Safety
/// `engine` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_trim(engine: *mut wolkit_engine, level: u32) {
    // SAFETY: NULL or live (contract).
    quiet((), || {
        if let Ok(e) = unsafe { borrow(engine) } {
            e.engine.trim(level);
        }
    })
}

/// # Safety
/// `engine` is live; `buf` has `cap` writable bytes; `needed` and `error` are writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_stats_json(
    engine: *mut wolkit_engine,
    buf: *mut u8,
    cap: usize,
    needed: *mut usize,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        // SAFETY: live (contract).
        let e = unsafe { borrow(engine) }?;
        let json = e.engine.stats_json();
        // SAFETY: the buffer is the caller's (contract).
        unsafe { give(buf, cap, needed, json.as_bytes(), "The statistics") }
    })
}

/// # Safety
/// `engine` is live; `out16` has 16 writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_get_gpu_uuid(engine: *const wolkit_engine, out16: *mut u8) -> wolkit_status {
    guard_status(|| {
        // SAFETY: live (contract).
        let _ = unsafe { borrow(engine) }?;
        let _ = out16;
        Err(Error::new(Status::Unavailable, "This engine has no GPU; it draws on the CPU."))
    })
}

/// # Safety
/// `engine` is live; `error` is writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_engine_set_environment(
    engine: *mut wolkit_engine,
    dark: u32,
    accent_rgb: u32,
    flags: u32,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        // SAFETY: live (contract).
        let e = unsafe { borrow(engine) }?;
        e.engine.set_environment(dark != 0, accent_rgb, flags)
    })
}
