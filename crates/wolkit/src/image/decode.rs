// SPDX-License-Identifier: BSD-2-Clause
//! Decoding: PNG and JPEG bytes to a premultiplied RGBA8 [`Level`], with the EXIF orientation
//! applied and the longer side capped. The header is read first ([`probe`]) and its size checked
//! against the limits before any pixel memory is allocated. Bytes are untrusted: every failure is
//! an error with one sentence, never a panic (`fuzz/fuzz_targets/wolkit_image.rs`).

use std::io::Cursor;

use super::{Level, Pixels, check_size, exif};
use crate::error::{Error, Result};
use crate::num::round_u32;
use crate::scene::ids::{Ix, to_u8, to_u32};

pub const NOT_AN_IMAGE: &str = "An image is a PNG, a JPEG or an SVG file.";
pub const DAMAGED: &str = "The picture could not be read: the file is damaged or cut short.";
pub const NO_SVG: &str = "SVG pictures are kept but not drawn yet; use a PNG or a JPEG.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Svg,
}

/// The format, by the first bytes.
pub fn sniff(b: &[u8]) -> Option<Format> {
    let starts = |tag: &[u8]| {
        b.iter()
            .take(256)
            .skip_while(|c| c.is_ascii_whitespace())
            .take(tag.len())
            .copied()
            .eq(tag.iter().copied())
    };
    if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some(Format::Png)
    } else if b.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(Format::Jpeg)
    } else if starts(b"<?xml") || starts(b"<svg") {
        Some(Format::Svg)
    } else {
        None
    }
}

/// What the header says: the oriented size and the EXIF orientation (1 to 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub width: u32,
    pub height: u32,
    pub orientation: u8,
}

fn damaged<E>(_: E) -> Error {
    Error::invalid(DAMAGED)
}

fn png_decoder(b: &[u8]) -> png::Decoder<Cursor<&[u8]>> {
    let limits = png::Limits {
        bytes: usize::try_from(super::MAX_PIXELS * 8).unwrap_or(usize::MAX),
    };
    let mut d = png::Decoder::new_with_limits(Cursor::new(b), limits);
    d.set_transformations(png::Transformations::normalize_to_color8());
    d.set_ignore_text_chunk(true);
    d
}

fn jpeg_options() -> zune_core::options::DecoderOptions {
    let side = super::MAX_SIDE.ix();
    zune_core::options::DecoderOptions::default()
        .jpeg_set_out_colorspace(zune_core::colorspace::ColorSpace::RGBA)
        .set_max_width(side)
        .set_max_height(side)
}

fn oriented(w: u32, h: u32, o: u8) -> (u32, u32) {
    if exif::swaps_axes(o) { (h, w) } else { (w, h) }
}

/// Reads the header only and checks the size limits.
pub fn probe(b: &[u8]) -> Result<Header> {
    let (w, h, o) = match sniff(b) {
        Some(Format::Png) => {
            let r = png_decoder(b).read_info().map_err(damaged)?;
            let i = r.info();
            let o = i.exif_metadata.as_deref().map_or(1, exif::orientation);
            (i.width, i.height, o)
        }
        Some(Format::Jpeg) => {
            let mut d = zune_jpeg::JpegDecoder::new_with_options(
                zune_core::bytestream::ZCursor::new(b),
                jpeg_options(),
            );
            d.decode_headers().map_err(damaged)?;
            let i = d.info().ok_or_else(|| Error::invalid(DAMAGED))?;
            let o = d.exif().map_or(1, |e| exif::orientation(e));
            (u32::from(i.width), u32::from(i.height), o)
        }
        Some(Format::Svg) => return Err(Error::invalid(NO_SVG)),
        None => return Err(Error::invalid(NOT_AN_IMAGE)),
    };
    check_size(w, h)?;
    let (width, height) = oriented(w, h, o);
    Ok(Header {
        width,
        height,
        orientation: o,
    })
}

/// Decodes `b` into a mip chain; `max_side` caps the longer side (0: none).
pub fn decode(b: &[u8], max_side: u32) -> Result<Pixels> {
    let head = probe(b)?;
    let (mut top, o) = match sniff(b) {
        Some(Format::Png) => (png_rgba(b)?, head.orientation),
        Some(Format::Jpeg) => (jpeg_rgba(b)?, head.orientation),
        _ => return Err(Error::invalid(NOT_AN_IMAGE)),
    };
    premultiply(&mut top.px);
    let top = cap(top, max_side);
    let top = if (2..=8).contains(&o) {
        orient(&top, o)
    } else {
        top
    };
    Ok(Pixels::from_level(head.width, head.height, top))
}

fn png_rgba(b: &[u8]) -> Result<Level> {
    let mut r = png_decoder(b).read_info().map_err(damaged)?;
    let (w, h) = (r.info().width, r.info().height);
    check_size(w, h)?;
    let n = r
        .output_buffer_size()
        .ok_or_else(|| Error::invalid(DAMAGED))?;
    let mut buf = vec![0u8; n];
    let out = r.next_frame(&mut buf).map_err(damaged)?;
    let px = w.ix() * h.ix();
    let rgba = match out.color_type {
        png::ColorType::Rgba => {
            buf.truncate(px * 4);
            buf
        }
        png::ColorType::Rgb => expand(&buf, px, 3, |s| [s[0], s[1], s[2], 255]),
        png::ColorType::GrayscaleAlpha => expand(&buf, px, 2, |s| [s[0], s[0], s[0], s[1]]),
        png::ColorType::Grayscale => expand(&buf, px, 1, |s| [s[0], s[0], s[0], 255]),
        png::ColorType::Indexed => return Err(Error::invalid(DAMAGED)),
    };
    if rgba.len() < px * 4 {
        return Err(Error::invalid(DAMAGED));
    }
    Ok(Level { w, h, px: rgba })
}

fn expand(src: &[u8], px: usize, n: usize, f: impl Fn(&[u8]) -> [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(px * 4);
    for s in src.chunks_exact(n).take(px) {
        out.extend_from_slice(&f(s));
    }
    out
}

fn jpeg_rgba(b: &[u8]) -> Result<Level> {
    let mut d = zune_jpeg::JpegDecoder::new_with_options(
        zune_core::bytestream::ZCursor::new(b),
        jpeg_options(),
    );
    let px = d.decode().map_err(damaged)?;
    let i = d.info().ok_or_else(|| Error::invalid(DAMAGED))?;
    let (w, h) = (u32::from(i.width), u32::from(i.height));
    check_size(w, h)?;
    if px.len() < w.ix() * h.ix() * 4 {
        return Err(Error::invalid(DAMAGED));
    }
    Ok(Level { w, h, px })
}

/// Straight to premultiplied alpha, in place, with exact rounding.
pub fn premultiply(px: &mut [u8]) {
    for p in px.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        if a == 255 {
            continue;
        }
        for c in &mut p[..3] {
            *c = to_u8(crate::paint::geom::div255(u32::from(*c) * a));
        }
    }
}

/// Half the size (rounded up), each texel the mean of up to four (edges repeat).
pub fn halve(l: &Level) -> Level {
    let (w, h) = (l.w.max(1).ix(), l.h.max(1).ix());
    let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
    let mut px = vec![0u8; nw * nh * 4];
    if l.px.len() < w * h * 4 {
        return Level {
            w: to_u32(nw),
            h: to_u32(nh),
            px,
        };
    }
    for (y, out) in px.chunks_mut(nw * 4).enumerate() {
        let r0 = &l.px[2 * y * w * 4..][..w * 4];
        let r1 = &l.px[(2 * y + 1).min(h - 1) * w * 4..][..w * 4];
        for (x, o) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (a, b) = (2 * x * 4, (2 * x + 1).min(w - 1) * 4);
            for c in 0..4 {
                let s = u32::from(r0[a + c])
                    + u32::from(r0[b + c])
                    + u32::from(r1[a + c])
                    + u32::from(r1[b + c]);
                o[c] = to_u8((s + 2) / 4);
            }
        }
    }
    Level {
        w: to_u32(nw),
        h: to_u32(nh),
        px,
    }
}

/// Scales `l` down so its longer side is `max_side` (no-op for 0 or a smaller picture): halves
/// while that stays at or above the target, then a bilinear step to the exact size.
pub fn cap(mut l: Level, max_side: u32) -> Level {
    let long = l.w.max(l.h);
    if max_side == 0 || long <= max_side {
        return l;
    }
    let k = max_side as f32 / long as f32;
    let tw = round_u32(l.w as f32 * k).clamp(1, max_side);
    let th = round_u32(l.h as f32 * k).clamp(1, max_side);
    while l.w / 2 >= tw && l.h / 2 >= th {
        l = halve(&l);
    }
    if (l.w, l.h) == (tw, th) {
        return l;
    }
    resample(&l, tw, th)
}

/// Bilinear resample of `l` to `tw x th` (pixel centres mapped onto pixel centres).
fn resample(l: &Level, tw: u32, th: u32) -> Level {
    let (kx, ky) = (l.w as f32 / tw as f32, l.h as f32 / th as f32);
    let mut px = Vec::with_capacity(tw.ix() * th.ix() * 4);
    for y in 0..th {
        let v = (y as f32 + 0.5) * ky - 0.5;
        for x in 0..tw {
            let u = (x as f32 + 0.5) * kx - 0.5;
            px.extend_from_slice(&super::fit::bilinear(l, u, v));
        }
    }
    Level { w: tw, h: th, px }
}

/// The picture turned upright for EXIF orientation `o` (1 to 8; anything else is 1).
pub fn orient(l: &Level, o: u8) -> Level {
    if !(2..=8).contains(&o) {
        return l.clone();
    }
    let (w, h) = (l.w, l.h);
    let (dw, dh) = oriented(w, h, o);
    let mut px = vec![0u8; dw.ix() * dh.ix() * 4];
    for sy in 0..h {
        for sx in 0..w {
            let (dx, dy) = match o {
                2 => (w - 1 - sx, sy),
                3 => (w - 1 - sx, h - 1 - sy),
                4 => (sx, h - 1 - sy),
                5 => (sy, sx),
                6 => (h - 1 - sy, sx),
                7 => (h - 1 - sy, w - 1 - sx),
                _ => (sy, w - 1 - sx),
            };
            let s = (sy.ix() * w.ix() + sx.ix()) * 4;
            let d = (dy.ix() * dw.ix() + dx.ix()) * 4;
            if let (Some(src), Some(dst)) = (l.px.get(s..s + 4), px.get_mut(d..d + 4)) {
                dst.copy_from_slice(src);
            }
        }
    }
    Level { w: dw, h: dh, px }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3x2 picture whose texels are numbered: texel i is [i, 0, 0, 255].
    fn numbered() -> Level {
        let px = (0..6u8).flat_map(|i| [i, 0, 0, 255]).collect();
        Level { w: 3, h: 2, px }
    }

    fn ids(l: &Level) -> Vec<u8> {
        l.px.as_chunks::<4>().0.iter().map(|p| p[0]).collect()
    }

    #[test]
    fn orientation_turns_the_picture_upright() {
        // 0 1 2
        // 3 4 5
        let l = numbered();
        assert_eq!(ids(&orient(&l, 1)), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(ids(&orient(&l, 2)), vec![2, 1, 0, 5, 4, 3]);
        assert_eq!(ids(&orient(&l, 3)), vec![5, 4, 3, 2, 1, 0]);
        assert_eq!(ids(&orient(&l, 4)), vec![3, 4, 5, 0, 1, 2]);
        let t = orient(&l, 5);
        assert_eq!((t.w, t.h), (2, 3));
        assert_eq!(ids(&t), vec![0, 3, 1, 4, 2, 5]);
        assert_eq!(ids(&orient(&l, 6)), vec![3, 0, 4, 1, 5, 2]);
        assert_eq!(ids(&orient(&l, 7)), vec![5, 2, 4, 1, 3, 0]);
        assert_eq!(ids(&orient(&l, 8)), vec![2, 5, 1, 4, 0, 3]);
    }

    #[test]
    fn premultiply_rounds_exactly() {
        let mut p = [255, 128, 0, 128, 10, 20, 30, 255, 200, 200, 200, 0];
        premultiply(&mut p);
        assert_eq!(p, [128, 64, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn cap_shrinks_the_longer_side_to_the_limit() {
        let l = Level {
            w: 1000,
            h: 500,
            px: vec![200; 1000 * 500 * 4],
        };
        let c = cap(l.clone(), 256);
        assert_eq!((c.w, c.h), (256, 128));
        assert!(c.px.iter().all(|v| *v == 200));
        assert_eq!(cap(l.clone(), 0).w, 1000);
        assert_eq!(cap(l, 4000).w, 1000);
    }

    #[test]
    fn sniff_knows_the_formats_by_their_first_bytes() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nxx"), Some(Format::Png));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some(Format::Jpeg));
        assert_eq!(sniff(b"  <svg xmlns"), Some(Format::Svg));
        assert_eq!(sniff(b"GIF89a"), None);
    }

    #[test]
    fn damaged_bytes_are_an_error_not_a_panic() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0]);
        assert!(decode(&png, 0).is_err());
        assert!(decode(&[0xff, 0xd8, 0xff, 0xe0, 0, 2], 0).is_err());
        assert!(decode(b"nope", 0).is_err());
    }
}
