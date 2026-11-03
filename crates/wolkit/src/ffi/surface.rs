// SPDX-License-Identifier: BSD-2-Clause
//! `wolkit_surface_*`: an external GPU image placed by layout (a Wayland subsurface on the native
//! backend, plan section 5.6). The instance-extension table is real (`gpu::surface`), and so is
//! the Vulkan side of `create_vk` (`gpu::surface::AppSurface`, a `VkSurfaceKHR` on the app's own
//! instance). The native backend makes the subsurface (`Backend::create_subsurface`); the other
//! backends say the window needs the native backend. Releasing the last reference destroys the
//! Vulkan surface first, then the subsurface.

use super::window::wolkit_window;
use super::{
    borrow, guard, handle, into_raw, need, put, release, retain, wolkit_error, wolkit_status,
};
use crate::engine::Engine;
use crate::gpu::surface::AppSurface;
use std::ffi::{c_char, c_void};
use std::sync::{Arc, Mutex};

#[repr(C)]
#[allow(non_camel_case_types)] // the C name
pub struct wolkit_surface {
    tag: handle::Tag<wolkit_surface>,
    engine: Arc<Engine>,
    window: u32,
    sub: u32,
    /// The `wl_display *` and `wl_surface *`, kept as exposed addresses (the backend owns them).
    display: usize,
    surface: usize,
    size: (u32, u32, f32),
    vk: Mutex<Option<AppSurface>>,
}

// Destroys the app's VkSurfaceKHR, then the subsurface it was made on.
impl Drop for wolkit_surface {
    fn drop(&mut self) {
        let vk = self.vk.get_mut().map_or(None, Option::take);
        drop(vk);
        self.engine.destroy_subsurface(self.window, self.sub);
    }
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
        let w = unsafe { borrow(window) }?;
        let sub = w.engine().create_subsurface(w.id(), node, width, height)?;
        let h = Arc::new(wolkit_surface {
            tag: handle::Tag::new(),
            engine: w.engine().clone(),
            window: w.id(),
            sub: sub.id,
            display: sub.display.expose_provenance(),
            surface: sub.surface.expose_provenance(),
            size: (sub.width, sub.height, sub.scale),
            vk: Mutex::new(None),
        });
        // SAFETY: checked non-NULL above.
        unsafe { *out = into_raw(h) };
        Ok(())
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
        let s = unsafe { borrow(surface) }?;
        put(w, s.size.0);
        put(h, s.size.1);
        put(scale, s.size.2);
        Ok(())
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
        need(vk_surface, "vk_surface")?;
        // SAFETY: live (contract).
        let s = unsafe { borrow(surface) }?;
        let mut slot = s.vk.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            let display = std::ptr::with_exposed_provenance_mut::<c_void>(s.display);
            let wl = std::ptr::with_exposed_provenance_mut::<c_void>(s.surface);
            // SAFETY: the instance is the caller's (contract); the display and the subsurface
            // stay alive while this handle does (the backend destroys the subsurface only when
            // the handle's last reference goes, after this Vulkan surface).
            *slot = Some(unsafe { AppSurface::create(vk_instance, display, wl) }?);
        }
        let raw = slot.as_ref().map_or(0, AppSurface::raw);
        // SAFETY: checked non-NULL above.
        unsafe { *vk_surface = raw };
        Ok(())
    })
}
