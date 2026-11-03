// SPDX-License-Identifier: BSD-2-Clause
//! The headless backend: no display, no compositor. A window is a scene, a layout and a CPU
//! raster on demand, driven by injected input and a virtual clock. It backs the conformance
//! suite, the golden harness and `wolkit/test` in the Crystal shard.

mod inject;

pub use inject::{hit_test, set_focus};

use super::{Backend, BACKEND_HEADLESS, KeyInput, PointerInput, Sink, WindowCore, pipeline};
use crate::error::{Error, Result};

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
        pipeline::ensure_layout(w);
        let (pw, ph) = (
            (w.width as f32 * w.scale).round() as u32,
            (w.height as f32 * w.scale).round() as u32,
        );
        let need = pw as usize * ph as usize * 4;
        if out.len() < need {
            return Err(Error::invalid(format!(
                "The pixel buffer needs {need} bytes for a {pw} by {ph} window."
            )));
        }
        crate::cpu::render(&w.scene, pw, ph, w.scale, &w.env, &mut out[..need]);
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
        w.time_ns = w.time_ns.saturating_add(ns);
        let _ = s;
        pipeline::frame(w);
        Ok(())
    }
}
