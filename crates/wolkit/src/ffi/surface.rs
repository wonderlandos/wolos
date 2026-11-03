// SPDX-License-Identifier: BSD-2-Clause
//! `wolkit_surface_*`: an external GPU image placed by layout (a Wayland subsurface on the native
//! backend, plan section 5.6). The instance-extension table is real (`gpu::surface`), and so is
//! the Vulkan side of `create_vk` (`gpu::surface::AppSurface`, a `VkSurfaceKHR` on the app's own
//! instance). Making the subsurface is the native backend's (WK-W4); until it does, creation says
//! the window needs the native backend, so no handle exists to call `create_vk` on.

use super::window::wolkit_window;
use super::{borrow, guard, handle, need, release, retain, wolkit_error, wolkit_status};
use crate::error::Error;
use std::ffi::{c_char, c_void};

#[repr(C)]
#[allow(non_camel_case_types)] // the C name
pub struct wolkit_surface {
    tag: handle::Tag<wolkit_surface>,
}

handle::tagged!(wolkit_surface, 0x57_4f_4c_4b_49_54_55_01);

/// # Safety
/// `window` is live; `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_surface_create(
    window: *mut wolkit_window,
    node: u32,
    width: u32,
    height: u32,
    out: *mut *mut wolkit_surface,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        need(out, "out")?;
        // SAFETY: live (contract).
        let _ = unsafe { borrow(window) }?;
        let _ = (node, width, height);
        Err(Error::unavailable(
            "This window needs the native backend to show a GPU surface.",
        ))
    })
}

/// # Safety
/// `surface` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_surface_retain(
    surface: *mut wolkit_surface,
) -> *mut wolkit_surface {
    // SAFETY: NULL or live (contract).
    unsafe { retain(surface) }
}

/// # Safety
/// `surface` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_surface_release(surface: *mut wolkit_surface) {
    // SAFETY: NULL or a reference the caller owns (contract).
    unsafe { release(surface) }
}

/// # Safety
/// `surface` is live; the out-pointers are writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_surface_get_size(
    surface: *const wolkit_surface,
    w: *mut u32,
    h: *mut u32,
    scale: *mut f32,
) -> wolkit_status {
    super::guard_status(|| {
        // SAFETY: live (contract).
        let _ = unsafe { borrow(surface) }?;
        let _ = (w, h, scale);
        Err(Error::unavailable(
            "This window needs the native backend to show a GPU surface.",
        ))
    })
}

/// The Vulkan instance extensions an app enables to make a surface with
/// `wolkit_surface_create_vk`: `VK_KHR_surface` and `VK_KHR_wayland_surface`, as a
/// NULL-terminated table of static strings (`*count` gets 2).
///
/// # Safety
/// `count` is writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_surface_get_vk_instance_extensions(
    count: *mut u32,
) -> *const *const c_char {
    let table = &crate::gpu::surface::INSTANCE_EXTENSIONS.0;
    super::put(count, 2);
    table.as_ptr()
}

/// # Safety
/// `surface` is live; `vk_instance` is a VkInstance; `vk_surface` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_surface_create_vk(
    surface: *mut wolkit_surface,
    vk_instance: *mut c_void,
    vk_surface: *mut u64,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        // SAFETY: live (contract).
        let _ = unsafe { borrow(surface) }?;
        let _ = (vk_instance, vk_surface);
        Err(Error::unavailable(
            "This window needs the native backend to show a GPU surface.",
        ))
    })
}
