// SPDX-License-Identifier: BSD-2-Clause
//! A wolkit page in the browser (ADR 0060). The site's loader (`site/public/kit/kit.js`)
//! fetches this module and the fonts, then [`mount`]s a view tree into a `<canvas>`: the same
//! scene, kit look, tokens, text shaping and display list as the native engine
//! (`wolkit_render::page::Page`), drawn by `wolkit_render::wgpu_backend::WgpuRenderer` on the
//! browser's WebGPU. Browser events arrive as wolkit input records (`pointer`, `key`, `text`).
//!
//! Only compiled for `wasm32`; natively the crate is empty (the workspace still builds it).

// The wasm-bindgen macros expand to unsafe glue on wasm32; the module itself has none.
#![cfg_attr(not(target_arch = "wasm32"), forbid(unsafe_code))]

#[cfg(target_arch = "wasm32")]
mod web;

/// X11 keysyms (the `wolkit.h` key records) for the DOM `KeyboardEvent.key` names the loader
/// forwards; printable characters go through `text` instead.
pub fn keysym(key: &str) -> Option<u32> {
    Some(match key {
        "Tab" => 0xff09,
        "Enter" => 0xff0d,
        "Escape" => 0xff1b,
        "Backspace" => 0xff08,
        "Delete" => 0xffff,
        "Home" => 0xff50,
        "End" => 0xff57,
        "ArrowLeft" => 0xff51,
        "ArrowUp" => 0xff52,
        "ArrowRight" => 0xff53,
        "ArrowDown" => 0xff54,
        " " => 0x0020,
        _ => return None,
    })
}

/// `KeyboardEvent` modifier flags to `WOLKIT_MOD_*`.
pub fn mods(shift: bool, ctrl: bool, alt: bool, meta: bool) -> u32 {
    use wolkit::window::{MOD_ALT, MOD_CTRL, MOD_SHIFT};
    // Command reaches apps as CTRL, like the rest of WolOS.
    (if shift { MOD_SHIFT } else { 0 })
        | (if ctrl || meta { MOD_CTRL } else { 0 })
        | (if alt { MOD_ALT } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom_keys_map_to_keysyms() {
        assert_eq!(keysym("Tab"), Some(0xff09));
        assert_eq!(keysym("ArrowRight"), Some(0xff53));
        assert_eq!(keysym("a"), None, "printable keys go through text");
        assert_eq!(
            mods(true, false, false, true),
            wolkit::window::MOD_SHIFT | wolkit::window::MOD_CTRL
        );
    }
}
