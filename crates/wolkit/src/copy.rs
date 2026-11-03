// SPDX-License-Identifier: BSD-2-Clause
//! Every sentence the engine itself shows or returns that is not tied to one failing call. One
//! sentence: what happened, what to do (RU-003).

pub const KIT_NO_CANVAS: &str = "This window needs the native backend to show a drawing.";
pub const KIT_NO_OVERLAY: &str = "The Kit host cannot show popovers; use the native backend.";
pub const KIT_TOO_MANY_PARTS: &str =
    "The Kit host shows at most 200 parts per window; use a list or the native backend.";
pub const KIT_TOO_DEEP: &str =
    "The Kit host shows at most 12 levels; flatten the window or use the native backend.";
pub const KIT_LABEL_NEEDED: &str = "A button, switch or field needs a label for the Kit host.";
pub const KIT_TEXT_LONG: &str = "The Kit host shows at most 1024 bytes of text per part.";
pub const KIT_GONE: &str = "The WolOS desktop is not running, so the window cannot open.";
pub const KIT_LOST: &str = "The WolOS desktop closed the connection.";
pub const NO_DISPLAY: &str = "There is no display to open a window on.";
pub const HEADLESS_ONLY: &str = "This call is for tests and needs a headless engine.";
pub const WINDOW_GONE: &str = "That window is closed.";
pub const NO_GPU_SURFACE: &str =
    "A graphics surface needs the native backend on a display; this window has neither.";
pub const TOO_MANY_WINDOWS: &str = "An app has at most 32 windows open; close one first.";
