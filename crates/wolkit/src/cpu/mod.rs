// SPDX-License-Identifier: BSD-2-Clause
//! CPU reference rasteriser (owned by WK-W3): exact-area path coverage, the 64x64-tile display
//! list renderer, and `render_frame`, the scene drawn for `wolkit_window_render_rgba` (through
//! `window::pipeline::render_rgba`).

pub mod coverage;
pub mod headless;
pub mod raster;

pub use headless::{Frame, render_frame};
