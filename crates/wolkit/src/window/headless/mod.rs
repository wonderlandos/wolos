// SPDX-License-Identifier: BSD-2-Clause
//! The headless backend: no display, no compositor. A window is a scene, a layout and a CPU
//! raster on demand, driven by injected input and a virtual clock. It backs the conformance
//! suite, the golden harness and `wolkit/test` in the Crystal shard.

mod inject;

pub use inject::{hit_test, set_focus};

use super::{BACKEND_HEADLESS, Backend, KeyInput, PointerInput, Sink, WindowCore, pipeline};
use crate::error::{Error, Result};
use crate::scene::ids::Ix;

#[derive(Default)]
pub struct Headless;

impl Backend for Headless {
    fn kind(&self) -> u32 {
        BACKEND_HEADLESS
    }

    fn show(&mut self, w: &mut WindowCore, s: &mut Sink) -> Result<()> {
        w.shown = true;
        pipeline::ensure_layout(w);
        let scale = w.scale.to_bits();
        let mut head = [0u8; 12];
        head[0..4].copy_from_slice(&w.width.to_le_bytes());
        head[4..8].copy_from_slice(&w.height.to_le_bytes());
        head[8..12].copy_from_slice(&scale.to_le_bytes());
        s.emit(crate::event::kind::RESIZE, 0, &head, &[]);
        Ok(())
    }

    fn close(&mut self, w: &mut WindowCore, _s: &mut Sink) {
        w.shown = false;
    }

    fn committed(&mut self, w: &mut WindowCore, _s: &mut Sink) -> Result<()> {
        if w.shown {
            pipeline::frame(w);
        }
        Ok(())
    }

    fn render_rgba(&mut self, w: &mut WindowCore, out: &mut [u8]) -> Result<(u32, u32)> {
        pipeline::settle_images(w);
        let (pw, ph) = (scaled(w.width, w.scale), scaled(w.height, w.scale));
        let need = pw.ix() * ph.ix() * 4;
        if out.len() < need {
            return Err(Error::invalid(format!(
                "The pixel buffer needs {need} bytes for a {pw} by {ph} window."
            )));
        }
        pipeline::render_rgba(w, pw, ph, w.scale, &mut out[..need]);
        Ok((pw, ph))
    }

    fn inject_pointer(&mut self, w: &mut WindowCore, s: &mut Sink, p: PointerInput) -> Result<()> {
        inject::pointer(w, s, p)
    }

    fn inject_key(&mut self, w: &mut WindowCore, s: &mut Sink, k: KeyInput) -> Result<()> {
        inject::key(w, s, k)
    }

    fn inject_text(&mut self, w: &mut WindowCore, s: &mut Sink, t: &str) -> Result<()> {
        inject::text(w, s, t)
    }

    fn advance_time(&mut self, w: &mut WindowCore, s: &mut Sink, ns: u64) -> Result<()> {
        w.step_anims(s, ns);
        pipeline::frame(w);
        Ok(())
    }
}

/// A size in logical pixels at `scale`, in device pixels (rounded, at most the largest window).
fn scaled(px: u32, scale: f32) -> u32 {
    let f = (px as f32 * scale).round().clamp(0.0, 65_536.0);
    // The clamp makes the conversion exact; `u64` is the one cast the rules allow here.
    u32::try_from(f as u64).unwrap_or(u32::MAX)
}
