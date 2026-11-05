// SPDX-License-Identifier: BSD-2-Clause
//! The existing Vulkan renderer (`wolkit::gpu`, ash) as a [`Renderer`]. Nothing is copied: the
//! trait call is `wolkit::gpu::Renderer::render_image`.

use crate::{FrameIn, Image, RenderError, Renderer};
use wolkit::gpu::{Frame, RendererOptions};

pub struct VulkanRenderer {
    inner: wolkit::gpu::Renderer,
}

impl VulkanRenderer {
    /// A renderer on the device `opts` selects (software first in tests, like the GPU goldens).
    pub fn new(opts: RendererOptions) -> Result<VulkanRenderer, RenderError> {
        wolkit::gpu::Renderer::new(opts)
            .map(|inner| VulkanRenderer { inner })
            .map_err(|e| RenderError(e.to_string()))
    }
}

impl Renderer for VulkanRenderer {
    fn name(&self) -> String {
        "vulkan".into()
    }

    fn render_image(&mut self, frame: FrameIn<'_>) -> Result<Image, RenderError> {
        self.inner.invalidate();
        self.inner
            .render_image(Frame {
                list: frame.list,
                text: frame.text,
                width: frame.width,
                height: frame.height,
                scale: frame.scale,
                clear: frame.clear,
                damage: None,
            })
            .map_err(|e| RenderError(e.to_string()))
    }
}
