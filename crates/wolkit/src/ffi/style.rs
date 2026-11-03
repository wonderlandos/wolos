// SPDX-License-Identifier: BSD-2-Clause
//! `wolkit_style_table_*` and `wolkit_style_compile`: the `WSB1` loader and the `.wolcss`
//! compiler (parsing is `crate::style`, owned by WK-W2).

use super::engine::wolkit_engine;
use super::{
    borrow, bytes_in, give, guard, handle, into_raw, need, release, retain, text_in, wolkit_error,
    wolkit_status,
};
use std::ffi::c_char;
use std::sync::Arc;

#[repr(C)]
#[allow(non_camel_case_types)] // the C name
pub struct wolkit_style_table {
    tag: handle::Tag<wolkit_style_table>,
    table: Arc<crate::style::Table>,
}

handle::tagged!(wolkit_style_table, 0x57_4f_4c_4b_49_54_53_01);

impl wolkit_style_table {
    pub(crate) fn table(&self) -> Arc<crate::style::Table> {
        self.table.clone()
    }
}

/// # Safety
/// `engine` is live; `bytes` points at `len` readable bytes; `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_style_table_load(
    engine: *mut wolkit_engine,
    bytes: *const u8,
    len: usize,
    out: *mut *mut wolkit_style_table,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        need(out, "out")?;
        // SAFETY: live (contract).
        let _ = unsafe { borrow(engine) }?;
        // SAFETY: pointer and length are the caller's (contract).
        let b = unsafe { bytes_in(bytes, len, crate::style::MAX_TABLE, "A style table") }?;
        let table = crate::style::load(b)?;
        let h = Arc::new(wolkit_style_table {
            tag: handle::Tag::new(),
            table: Arc::new(table),
        });
        // SAFETY: checked non-NULL above.
        unsafe { *out = into_raw(h) };
        Ok(())
    })
}

/// # Safety
/// `table` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_style_table_retain(
    table: *mut wolkit_style_table,
) -> *mut wolkit_style_table {
    // SAFETY: NULL or live (contract).
    unsafe { retain(table) }
}

/// # Safety
/// `table` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_style_table_release(table: *mut wolkit_style_table) {
    // SAFETY: NULL or a reference the caller owns (contract).
    unsafe { release(table) }
}

/// # Safety
/// `wolcss` points at `len` readable bytes; `out` has `cap` writable bytes; `needed` is writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_style_compile(
    wolcss: *const c_char,
    len: usize,
    out: *mut u8,
    cap: usize,
    needed: *mut usize,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        // SAFETY: pointer and length are the caller's (contract).
        let src = unsafe { text_in(wolcss, len, crate::style::MAX_TABLE, "The style sheet") }?;
        let bytes = crate::style::compile(src)?;
        // SAFETY: the buffer is the caller's (contract).
        unsafe { give(out, cap, needed, &bytes, "The style table") }
    })
}
