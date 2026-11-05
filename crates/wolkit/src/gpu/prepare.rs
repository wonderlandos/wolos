// SPDX-License-Identifier: BSD-2-Clause
//! The device-independent half of a GPU frame, shared by every GPU backend (the Vulkan renderer
//! in [`super::frame`] and the wgpu one in `crates/wolkit-render`): shadows pointed at their
//! prototypes in the shadow atlas, layer composites appended, and the byte images of the instance
//! ring and the clip table. A backend only uploads these and records its passes.

use super::shadow::ShadowAtlas;
use super::{ABSOLUTE, K_LAYER};
use crate::paint::batch::{ClipEntry, Draw, Instance, Pass, kind};
use crate::paint::geom::{PixelRect, Rect};
use crate::scene::ids::Ix;

/// Deepest nesting of overlapping-opacity layers in a frame.
pub fn max_depth(draws: &[Draw]) -> usize {
    let (mut d, mut m) = (0usize, 0usize);
    for x in draws {
        match x.pass {
            Pass::LayerBegin => {
                d += 1;
                m = m.max(d);
            }
            Pass::LayerEnd { .. } => d = d.saturating_sub(1),
            Pass::Draw => {}
        }
    }
    m
}

/// Rewrites the shadow instances of one draw: the quad becomes the mask's pixel rectangle in
/// window space (`ABSOLUTE`), `uv` the prototype in the shadow atlas, `extra` its slicing.
pub fn place_shadows(atlas: &mut ShadowAtlas, inst: &mut [Instance], d: &Draw) {
    let (a, b) = (d.first.ix(), (d.first + d.count).ix());
    let Some(range) = inst.get_mut(a..b) else {
        return;
    };
    for i in range.iter_mut().filter(|i| i.kind() == kind::SHADOW) {
        let cast = Rect::new(
            i.rect[0] + d.offset.0,
            i.rect[1] + d.offset.1,
            i.rect[2],
            i.rect[3],
        );
        let radii = i.radii.map(|q| f32::from(q) / 16.0);
        match atlas.get(&cast, &radii, i.extra[0]) {
            Some((p, ext)) => {
                i.rect = [
                    ext.x0 as f32,
                    ext.y0 as f32,
                    ext.width() as f32,
                    ext.height() as f32,
                ];
                i.extra = [
                    p.slicing.split_x as f32,
                    p.slicing.split_y as f32,
                    p.slicing.stretch_x as f32,
                    p.slicing.stretch_y as f32,
                ];
                i.uv = [p.x, p.y, p.w, p.h];
                i.params = kind::SHADOW | ABSOLUTE;
            }
            None => i.rect = [0.0; 4],
        }
    }
}

/// Appends one composite instance per layer end.
pub fn composite_instances(draws: &[Draw], scissor: PixelRect, out: &mut Vec<Instance>) {
    for d in draws {
        if let Pass::LayerEnd { alpha } = d.pass {
            out.push(Instance {
                rect: [
                    scissor.x0 as f32,
                    scissor.y0 as f32,
                    scissor.width() as f32,
                    scissor.height() as f32,
                ],
                color0: u32::from(alpha) << 24,
                params: K_LAYER | ABSOLUTE,
                ..Instance::default()
            });
        }
    }
}

/// The clip table as the shader reads it: three `vec4` rows per entry (rectangle, rounded
/// rectangle, radius), little-endian `f32`.
pub fn clip_bytes(clips: &[ClipEntry], out: &mut Vec<u8>) {
    out.clear();
    for c in clips {
        for v in c
            .rect
            .iter()
            .chain(c.rounded.iter())
            .chain([c.radius, 0.0, 0.0, 0.0].iter())
        {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
}

/// The instances of a built frame as the ring holds them: the builder's, with shadows placed
/// in `shadows`, then one composite per layer end over `scissor`.
pub fn frame_instances(
    built: &[Instance],
    draws: &[Draw],
    shadows: &mut ShadowAtlas,
    scissor: PixelRect,
    out: &mut Vec<Instance>,
) {
    out.clear();
    out.extend_from_slice(built);
    for d in draws {
        if d.pass == Pass::Draw {
            place_shadows(shadows, out, d);
        }
    }
    composite_instances(draws, scissor, out);
}
