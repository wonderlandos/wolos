// SPDX-License-Identifier: BSD-2-Clause
//! Every sentence the GPU renderer returns or shows. One sentence: what happened, and what to do
//! (RU-003). The engine falls back to the CPU rasteriser on each of these, so none is fatal.

pub const NO_VULKAN: &str =
    "Vulkan is not installed, so windows are drawn on the processor instead.";
pub const NO_DEVICE: &str =
    "No usable graphics device was found, so windows are drawn on the processor instead.";
pub const BLOCKED: &str =
    "This graphics device is on the blocklist, so windows are drawn on the processor instead.";
pub const DEVICE_LOST: &str = "The graphics device stopped responding; the window is drawn on the processor until it is back.";
pub const GAVE_UP: &str = "The graphics device was lost three times in a minute, so this app draws on the processor from now on.";
pub const OUT_OF_MEMORY: &str =
    "The graphics device ran out of memory; close other apps or windows and try again.";
pub const DRIVER: &str =
    "The graphics driver refused a request; windows are drawn on the processor instead.";
pub const TOO_LARGE: &str =
    "The window is larger than the graphics device can draw; make it smaller.";
pub const NO_SURFACE: &str =
    "The display does not offer a Vulkan surface for this window, so it is drawn on the processor.";
pub const BAD_CANVAS: &str =
    "The drawing commands are not in the canvas format; check the encoder.";
pub const CANVAS_LIMIT: &str =
    "A canvas holds at most 65,536 drawing commands; draw less per frame.";
pub const CANVAS_STACK: &str =
    "A canvas nests at most 64 clips and opacities, and every push needs its pop.";
