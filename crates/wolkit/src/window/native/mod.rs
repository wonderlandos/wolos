// SPDX-License-Identifier: BSD-2-Clause
//! Native Wayland window backend: owned by WK-W4. This file is W1's placeholder so the crate
//! builds. `available` says whether the native backend can open a window here; `window::choose`
//! falls back to the Kit backend while it says no. W4 adds `pub fn create(...) -> Result<Box<dyn
//! Backend>>` and the call site in `Engine::create_window` (`src/engine.rs`).

/// Whether this build and this session can open native windows.
pub fn available() -> bool {
    false
}
