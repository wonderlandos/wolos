// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! File panels (ADR 0020, 1.5): the person picks, the desktop opens the file as them, and the
//! descriptor comes back through wold. The app never gets a path.
use crate::client;
use crate::context::wol_context;
use crate::error::{Error, Result, Status, guard, input, need, wol_error, wol_status};
use crate::string::{borrow, into_raw, release, retain, wol_string};
use shared::portal::{Call, MAX_NAME, MAX_TITLE};
use std::ffi::{c_char, c_int};
use std::os::fd::{AsRawFd, OwnedFd};
use std::ptr::null_mut;
use std::sync::Arc;
use std::time::Duration;

/// A little over wold's own wait, so wold's answer arrives first.
pub(crate) const PANEL_WAIT: Duration = Duration::from_secs(610);

pub struct wol_file {
    fd: OwnedFd,
    name: Arc<wol_string>,
}

pub(crate) fn panel(ctx: &wol_context, c: &Call) -> Result<*mut wol_file> {
    let (a, fd) = client::call_for(ctx.wold(), c, PANEL_WAIT);
    if !a.ok {
        return Err(Error::new(
            a.code.map_or(Status::Internal, Into::into),
            a.message.unwrap_or_default(),
        ));
    }
    let data = a.data.unwrap_or_default();
    if data.get("cancelled").and_then(|v| v.as_bool()) == Some(true) {
        return Ok(null_mut());
    }
    match (fd, data.get("name").and_then(|n| n.as_str())) {
        (Some(fd), Some(name)) => Ok(into_raw(Arc::new(wol_file {
            fd,
            name: wol_string::new(name),
        }))),
        _ => Err(Error::new(
            Status::Internal,
            "The panel did not hand over a file.",
        )),
    }
}

/// # Safety
/// `context` is live; `title` points at `title_len` bytes; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_panel_open(
    context: *mut wol_context,
    title: *const c_char,
    title_len: usize,
    out: *mut *mut wol_file,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let title = input(title, title_len, MAX_TITLE, "The title")?.to_owned();
        let f = panel(ctx, &Call::PanelOpen { title })?;
        unsafe { *out = f };
        Ok(())
    })
}

/// # Safety
/// `context` is live; `title`, `name` point at their lengths; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_panel_save(
    context: *mut wol_context,
    title: *const c_char,
    title_len: usize,
    name: *const c_char,
    name_len: usize,
    out: *mut *mut wol_file,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let title = input(title, title_len, MAX_TITLE, "The title")?.to_owned();
        let name = input(name, name_len, MAX_NAME, "The name")?.to_owned();
        let f = panel(ctx, &Call::PanelSave { title, name })?;
        unsafe { *out = f };
        Ok(())
    })
}

/// # Safety
/// `file` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_file_get_fd(file: *const wol_file) -> c_int {
    unsafe { file.as_ref() }.map_or(-1, |f| f.fd.as_raw_fd())
}

/// # Safety
/// `file` is NULL or live. Borrowed from `file`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_file_get_name(file: *const wol_file) -> *const wol_string {
    unsafe { file.as_ref() }.map_or(std::ptr::null(), |f| Arc::as_ptr(&f.name))
}

/// # Safety
/// `file` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_file_retain(file: *mut wol_file) -> *mut wol_file {
    unsafe { retain(file) }
}

/// # Safety
/// `file` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_file_release(file: *mut wol_file) {
    unsafe { release(file) }
}
