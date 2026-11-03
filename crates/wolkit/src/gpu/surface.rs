// SPDX-License-Identifier: BSD-2-Clause
//! Vulkan surfaces for apps (`wolkit_surface_*`, plan section 5.6): the instance extensions an app
//! must enable, and a `VkSurfaceKHR` made on the app's own `VkInstance` for a Wayland surface the
//! native backend owns (a subsurface placed by layout).
//!
//! The app's instance comes from the same system loader, so the surface functions are looked up
//! through `vkGetInstanceProcAddr` on that instance. The surface belongs to the `wolkit_surface`
//! and is destroyed with it, on the app's instance.

use std::ffi::{CStr, c_char, c_void};
use std::sync::OnceLock;

use ash::vk;
use ash::vk::Handle;

use super::copy;
use super::device::vk_err;
use crate::error::{Error, Result};

/// The extensions, as a NULL-terminated table of static C strings (the ABI hands it out).
pub struct ExtensionTable(pub [*const c_char; 3]);

// SAFETY: the table holds pointers to `'static` C string literals, never written.
unsafe impl Sync for ExtensionTable {}

pub static INSTANCE_EXTENSIONS: ExtensionTable = ExtensionTable([
    c"VK_KHR_surface".as_ptr(),
    c"VK_KHR_wayland_surface".as_ptr(),
    std::ptr::null(),
]);

/// The names, for Rust callers and tests.
pub fn instance_extensions() -> [&'static CStr; 2] {
    [c"VK_KHR_surface", c"VK_KHR_wayland_surface"]
}

fn entry() -> Option<&'static ash::Entry> {
    static ENTRY: OnceLock<Option<ash::Entry>> = OnceLock::new();
    // SAFETY: loading the system loader runs its initialisers once; it is the documented entry.
    ENTRY
        .get_or_init(|| unsafe { ash::Entry::load() }.ok())
        .as_ref()
}

/// A Vulkan surface on an app's instance.
pub struct AppSurface {
    surface: vk::SurfaceKHR,
    surface_fn: ash::khr::surface::Instance,
}

// SAFETY: plain Vulkan handles and function tables; destruction is externally synchronised by
// the engine's one-thread-at-a-time rule.
unsafe impl Send for AppSurface {}

impl AppSurface {
    /// Makes a `VkSurfaceKHR` for `wl_surface` on the app's `vk_instance`.
    ///
    /// # Safety
    /// `vk_instance` is a live `VkInstance` with [`INSTANCE_EXTENSIONS`] enabled, made by the
    /// system loader; `display` and `wl_surface` are live Wayland objects that outlive the
    /// returned surface.
    pub unsafe fn create(
        vk_instance: *mut c_void,
        display: *mut c_void,
        wl_surface: *mut c_void,
    ) -> Result<AppSurface> {
        if vk_instance.is_null() || display.is_null() || wl_surface.is_null() {
            return Err(Error::invalid(copy::NO_SURFACE));
        }
        let Some(e) = entry() else {
            return Err(Error::unavailable(copy::NO_VULKAN));
        };
        let handle = vk::Instance::from_raw(u64::try_from(vk_instance.addr()).unwrap_or(0));
        // SAFETY: `handle` is a live instance of the system loader (contract), so its function
        // table loads through `vkGetInstanceProcAddr`; the Wayland pointers are live (contract).
        unsafe {
            let instance = ash::Instance::load(e.static_fn(), handle);
            let wl = ash::khr::wayland_surface::Instance::new(e, &instance);
            let info = vk::WaylandSurfaceCreateInfoKHR::default()
                .display(display)
                .surface(wl_surface);
            let surface = wl.create_wayland_surface(&info, None).map_err(vk_err)?;
            Ok(AppSurface {
                surface,
                surface_fn: ash::khr::surface::Instance::new(e, &instance),
            })
        }
    }

    /// The handle as the `uint64_t` the ABI returns.
    pub fn raw(&self) -> u64 {
        self.surface.as_raw()
    }
}

impl Drop for AppSurface {
    // Destroys the VkSurfaceKHR on the app's instance (the app must keep that instance alive
    // until it released the wolkit_surface, as wolkit.h says).
    fn drop(&mut self) {
        // SAFETY: the surface was made on this instance by `create` and is destroyed once.
        unsafe { self.surface_fn.destroy_surface(self.surface, None) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_table_is_null_terminated_and_matches_the_names() {
        let names = instance_extensions();
        for (i, n) in names.iter().enumerate() {
            assert_eq!(INSTANCE_EXTENSIONS.0[i], n.as_ptr());
        }
        assert!(INSTANCE_EXTENSIONS.0[2].is_null());
        assert_eq!(names, super::super::device::SURFACE_INSTANCE_EXTENSIONS);
    }

    #[test]
    fn null_handles_are_refused_with_a_sentence() {
        // SAFETY: null pointers are checked before any use.
        let r = unsafe {
            AppSurface::create(
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert!(r.is_err_and(|e| !e.message.is_empty()));
    }
}
