// SPDX-License-Identifier: BSD-2-Clause
//! GPU surfaces placed by layout (`wolkit_surface_*`, plan section 5.6): a `surface` node gets a
//! Wayland subsurface of the window, desynchronised (the app presents to it at its own pace with
//! Vulkan on its own instance), positioned at the node's drawn rectangle on every frame of the
//! window, with an empty input region so the pointer still reaches the window's routing. A size
//! change of the node is reported as `SURFACE_RESIZED` (`u32 width`, `u32 height` in device
//! pixels, `f32 scale`) so the app rebuilds its swapchain.

use super::client::Client;
use crate::event::kind as ev;
use crate::scene::{Idx, Kind, NodeId, SceneRead};
use crate::window::{Sink, WindowCore};
use wayland_client::protocol::{wl_subsurface, wl_surface};
use wayland_client::{Proxy, QueueHandle};

/// Subsurfaces per window.
pub const MAX: usize = 16;

pub struct Sub {
    pub id: u32,
    node: Idx,
    node_id: NodeId,
    pub surface: wl_surface::WlSurface,
    sub: wl_subsurface::WlSubsurface,
    /// Last position and device size sent.
    at: (i32, i32),
    pub size: (u32, u32),
}

#[derive(Default)]
pub struct Subs {
    pub list: Vec<Sub>,
    next: u32,
}

/// What `create` hands back.
pub struct Made {
    pub id: u32,
    pub surface: wl_surface::WlSurface,
    pub size: (u32, u32),
}

fn device_size(w: &WindowCore, node: Idx, scale: f32) -> (u32, u32) {
    let r = crate::input::hit::drawn_rect(&w.scene, node);
    (
        crate::num::round_u32(r.w * scale).max(1),
        crate::num::round_u32(r.h * scale).max(1),
    )
}

impl Subs {
    /// A subsurface for `node` (a live `surface` node) under `parent`.
    pub(super) fn create(
        &mut self,
        g: &super::Globals,
        parent: &wl_surface::WlSurface,
        qh: &QueueHandle<Client>,
        w: &WindowCore,
        node: Idx,
        scale: f32,
    ) -> Result<Made, &'static str> {
        let (compositor, Some(subcompositor)) = (&g.compositor, &g.subcompositor) else {
            return Err(
                "The compositor does not offer subsurfaces, so a GPU surface cannot be placed.",
            );
        };
        if !w.scene.is_live(node) || w.scene.kind(node) != Kind::Surface {
            return Err("A GPU surface goes on a surface part.");
        }
        if self.list.len() >= MAX {
            return Err("A window has at most 16 GPU surfaces.");
        }
        let surface = compositor.create_surface(qh, ());
        let sub = subcompositor.get_subsurface(&surface, parent, qh, ());
        sub.set_desync();
        let empty = compositor.create_region(qh, ());
        surface.set_input_region(Some(&empty));
        empty.destroy();
        let r = crate::input::hit::drawn_rect(&w.scene, node);
        let at = (crate::num::round_i32(r.x), crate::num::round_i32(r.y));
        sub.set_position(at.0, at.1);
        surface.set_buffer_scale(crate::num::round_i32(scale).max(1));
        surface.commit();
        self.next = self.next.wrapping_add(1);
        let id = self.next;
        let size = device_size(w, node, scale);
        self.list.push(Sub {
            id,
            node,
            node_id: w.scene.id(node),
            surface: surface.clone(),
            sub,
            at,
            size,
        });
        Ok(Made { id, surface, size })
    }

    pub fn destroy(&mut self, id: u32) {
        if let Some(i) = self.list.iter().position(|s| s.id == id) {
            let s = self.list.remove(i);
            s.sub.destroy();
            s.surface.destroy();
        }
    }

    pub fn clear(&mut self) {
        for s in self.list.drain(..) {
            s.sub.destroy();
            s.surface.destroy();
        }
    }

    /// Follow layout: move subsurfaces whose node moved (applied with the window's next commit)
    /// and report size changes. A node that went away leaves its subsurface where it was until
    /// the app releases the surface.
    pub fn follow(&mut self, w: &WindowCore, s: &mut Sink, scale: f32) {
        for sub in &mut self.list {
            let n = sub.node;
            if !w.scene.is_live(n) || w.scene.id(n) != sub.node_id {
                continue;
            }
            let r = crate::input::hit::drawn_rect(&w.scene, n);
            let at = (crate::num::round_i32(r.x), crate::num::round_i32(r.y));
            if at != sub.at {
                sub.sub.set_position(at.0, at.1);
                sub.at = at;
            }
            let size = device_size(w, n, scale);
            if size != sub.size {
                sub.size = size;
                // To the surface node itself: the app's surface handle follows it.
                let mut head = [0u8; 12];
                head[0..4].copy_from_slice(&size.0.to_le_bytes());
                head[4..8].copy_from_slice(&size.1.to_le_bytes());
                head[8..12].copy_from_slice(&scale.to_bits().to_le_bytes());
                s.emit(ev::SURFACE_RESIZED, sub.node_id.0, &head, &[]);
            }
        }
    }

    /// The `wl_surface *` of subsurface `id`, for Vulkan WSI.
    pub fn ptr(&self, id: u32) -> Option<*mut std::ffi::c_void> {
        let s = self.list.iter().find(|s| s.id == id)?;
        let p = s.surface.id().as_ptr().cast::<std::ffi::c_void>();
        (!p.is_null()).then_some(p)
    }
}
