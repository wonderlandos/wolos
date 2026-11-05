// SPDX-License-Identifier: BSD-2-Clause
//! What the shim does when the engine says a new frame is ready (`notify_new_frame_ready`). Only
//! std: `tests/engine_frame.rs` includes this file and tests the order without the engine.
//!
//! The engine draws a frame into the rendering context's back buffer (`paint`); nothing reaches the
//! Wayland surface until the context swaps it (`present`, `eglSwapBuffers`). An embedder does both
//! for every frame (servoshell: `ServoShellWindow::repaint_webviews`); a shim that only paints
//! leaves the window blank while the engine reports `load` and `frames` as if all was well.
use std::cell::Cell;

/// The view and its rendering context, as a frame needs them.
pub trait Frame {
    /// Makes the context's GL context current on this thread.
    fn make_current(&self);
    /// Draws the engine's newest frame into the context's back buffer.
    fn paint(&self);
    /// Swaps the back buffer onto the target (a Wayland surface commit with a new buffer).
    fn present(&self);
}

/// A new frame is ready: count it, draw it and put it on screen.
pub fn ready(frames: &Cell<u64>, f: &impl Frame) {
    frames.set(frames.get() + 1);
    f.make_current();
    f.paint();
    f.present();
}
