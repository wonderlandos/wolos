// SPDX-License-Identifier: BSD-2-Clause
//! CPU reference rasteriser (owned by WK-W3): exact-area path coverage, the 64x64-tile display
//! list renderer, and the headless entry point `render` that `wolkit_window_render_rgba` calls.

pub mod coverage;
pub mod headless;
pub mod raster;

pub use headless::{Frame, render};
