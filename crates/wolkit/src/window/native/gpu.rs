// SPDX-License-Identifier: BSD-2-Clause
//! The GPU present path of a native window (plan section 2.6, v1): the scene lowered to the same
//! display list the CPU rasteriser draws, handed to WK-W5's renderer and presented through the
//! Vulkan Wayland surface extension on the window's own `wl_surface`. The window keeps pacing
//! (frame callback, viewport or buffer scale, presentation feedback are set before the present
//! commits). Any failure drops back to the CPU path for the rest of the window's life; the CPU
//! path is also what `WOLKIT_ENGINE_FORCE_CPU` asks for.

use crate::error::Result;
use crate::gpu::frame::{Frame, Renderer, RendererOptions};
use crate::gpu::present::{Presented, Presenter};
use crate::paint::list::DisplayList;
use crate::paint::palette::Palette;
use crate::paint::scene::{Lower, build_with};
use crate::window::{WindowCore, pipeline};
use std::ffi::c_void;

pub struct GpuPath {
    renderer: Renderer,
    presenter: Presenter,
    list: DisplayList,
}

impl GpuPath {
    /// Open a device and make a presenter for the surface. Fails when no device opens.
    ///
    /// # Safety
    /// `display` is the live `wl_display *` of the window's connection and `surface` the live
    /// `wl_surface *` of the window; both outlive the returned value (the window drops it before
    /// it destroys the surface).
    pub unsafe fn new(
        display: *mut c_void,
        surface: *mut c_void,
        width: u32,
        height: u32,
    ) -> Result<GpuPath> {
        let renderer = Renderer::new(RendererOptions::default())?;
        // SAFETY: the pointers are live and outlive the presenter (this function's contract).
        let presenter = unsafe { Presenter::new(display, surface, width, height) };
        Ok(GpuPath {
            renderer,
            presenter,
            list: DisplayList::new(),
        })
    }

    /// Draw and present the window at `width` by `height` device pixels.
    pub fn draw(
        &mut self,
        w: &mut WindowCore,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Presented> {
        pipeline::ensure_layout(w);
        let palette = Palette::for_env(&w.env);
        let text = w.text.clone();
        let mut sys = crate::text::measure::lock(&text);
        build_with(
            &mut Lower {
                scene: &w.scene,
                styles: &w.styles,
                palette: &palette,
                text: &mut sys,
                cache: &mut w.text_cache,
            },
            Some(&w.canvases),
            &mut self.list,
        );
        self.presenter.resize(width, height);
        let sys = &mut *sys;
        let frame = Frame {
            list: &self.list,
            text: Some((&sys.db, &mut sys.glyphs)),
            width,
            height,
            scale,
            clear: palette.background(),
            damage: None,
        };
        self.presenter.present(&mut self.renderer, frame)
    }

    /// Memory Saver: drop what the renderer can rebuild.
    pub fn trim(&mut self) {
        self.renderer.trim();
    }
}
