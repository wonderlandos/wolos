// SPDX-License-Identifier: BSD-2-Clause
//! The image atlas: where picture levels sit in the GPU's RGBA8 texture array (binding 4,
//! [`PAGE`] pixels a side, up to [`MAX_PAGES`] pages, premultiplied). Levels are packed on
//! shelves; a level is uploaded once, when it is first placed (only those rectangles go to the
//! GPU). When the pages are full the atlas starts over and the frame is built again with only
//! what it shows; pictures beyond that are skipped for the frame. A new device or a texture that
//! had to grow uploads every placed level again.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use crate::image::{MAX_DRAW_SIDE, Pixels};
use crate::paint::batch::{ImagePlace, ImageSource};
use crate::scene::ids::to_u16;

/// Side of a page, pixels (64 MiB of RGBA8 for four pages).
pub const PAGE: u32 = MAX_DRAW_SIDE;
pub const MAX_PAGES: u32 = 4;
/// Shelf heights are rounded up to this, so similar thumbnails share shelves.
const SHELF_STEP: u32 = 8;

struct Shelf {
    y: u32,
    h: u32,
    x: u32,
}

#[derive(Default)]
struct Page {
    shelves: Vec<Shelf>,
    top: u32,
}

/// One level to copy into the texture.
pub struct Upload {
    pub page: u32,
    pub x: u32,
    pub y: u32,
    pub px: Arc<Pixels>,
    pub level: usize,
}

#[derive(Default)]
pub struct ImageAtlas {
    pages: Vec<Page>,
    placed: HashMap<(u64, usize), (ImagePlace, Weak<Pixels>)>,
    /// Levels placed since the last upload.
    pub pending: Vec<Upload>,
    /// Changes whenever placed levels may have moved (instances naming them must be rebuilt).
    pub generation: u64,
    /// A level did not fit this frame.
    overflow: bool,
}

impl ImageAtlas {
    pub fn new() -> ImageAtlas {
        ImageAtlas::default()
    }

    pub fn page_count(&self) -> u32 {
        u32::try_from(self.pages.len()).unwrap_or(MAX_PAGES)
    }

    /// Forget every placement (a full atlas, a new device, Memory Saver).
    pub fn reset(&mut self) {
        self.pages.clear();
        self.placed.clear();
        self.pending.clear();
        self.overflow = false;
        self.generation += 1;
    }

    /// Something did not fit since the last call (and clears the flag).
    pub fn take_overflow(&mut self) -> bool {
        std::mem::take(&mut self.overflow)
    }

    /// The texture was made again: every placed level that is still alive is uploaded again.
    pub fn reupload_all(&mut self) {
        self.pending.clear();
        for ((_, level), (at, px)) in &self.placed {
            if let Some(px) = px.upgrade() {
                self.pending.push(Upload {
                    page: at.page,
                    x: u32::from(at.x),
                    y: u32::from(at.y),
                    px,
                    level: *level,
                });
            }
        }
    }

    /// Bytes the pending uploads need in staging.
    pub fn pending_bytes(&self) -> u64 {
        self.pending
            .iter()
            .filter_map(|u| u.px.levels.get(u.level))
            .map(|l| u64::try_from(l.px.len()).unwrap_or(0))
            .sum()
    }

    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32, u32)> {
        if w > PAGE || h > PAGE {
            return None;
        }
        let sh = h.div_ceil(SHELF_STEP) * SHELF_STEP;
        for (pi, p) in self.pages.iter_mut().enumerate() {
            if let Some(s) = p
                .shelves
                .iter_mut()
                .find(|s| s.h >= sh && s.h <= sh * 2 && PAGE - s.x >= w)
            {
                let at = (u32::try_from(pi).ok()?, s.x, s.y);
                s.x += w;
                return Some(at);
            }
            if PAGE - p.top >= sh {
                let y = p.top;
                p.top += sh;
                p.shelves.push(Shelf { y, h: sh, x: w });
                return Some((u32::try_from(pi).ok()?, 0, y));
            }
        }
        if self.page_count() >= MAX_PAGES {
            return None;
        }
        self.pages.push(Page {
            shelves: vec![Shelf { y: 0, h: sh, x: w }],
            top: sh,
        });
        Some((self.page_count() - 1, 0, 0))
    }
}

impl ImageSource for ImageAtlas {
    fn place(&mut self, px: &Arc<Pixels>, level: usize) -> Option<ImagePlace> {
        if let Some((at, _)) = self.placed.get(&(px.key, level)) {
            return Some(*at);
        }
        let l = px.levels.get(level)?;
        let Some((page, x, y)) = self.alloc(l.w, l.h) else {
            self.overflow = true;
            return None;
        };
        let at = ImagePlace {
            page,
            x: to_u16(x),
            y: to_u16(y),
        };
        self.placed
            .insert((px.key, level), (at, Arc::downgrade(px)));
        self.pending.push(Upload {
            page,
            x,
            y,
            px: Arc::clone(px),
            level,
        });
        Some(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::Level;

    fn pic(w: u32, h: u32) -> Arc<Pixels> {
        Arc::new(Pixels::from_level(
            w,
            h,
            Level {
                w,
                h,
                px: vec![255; usize::try_from(w * h * 4).unwrap()],
            },
        ))
    }

    #[test]
    fn levels_are_placed_once_and_uploaded_once() {
        let mut a = ImageAtlas::new();
        let p = pic(160, 120);
        let first = a.place(&p, 0).unwrap();
        assert_eq!(a.place(&p, 0), Some(first));
        assert_eq!(a.pending.len(), 1);
        let q = pic(160, 120);
        let second = a.place(&q, 0).unwrap();
        assert_eq!((second.page, second.x, second.y), (0, 160, 0));
        assert_eq!(a.pending_bytes(), 2 * 160 * 120 * 4);
    }

    #[test]
    fn a_full_atlas_overflows_and_starts_over() {
        let mut a = ImageAtlas::new();
        let big = pic(2048, 2048);
        let more: Vec<_> = (0..MAX_PAGES + 1).map(|_| pic(2048, 2048)).collect();
        assert!(a.place(&big, 0).is_some());
        let placed = more.iter().filter(|p| a.place(p, 0).is_some()).count();
        assert_eq!(placed, usize::try_from(MAX_PAGES - 1).unwrap());
        assert!(a.take_overflow());
        let g = a.generation;
        a.reset();
        assert!(a.generation > g && a.page_count() == 0 && a.pending.is_empty());
    }

    #[test]
    fn reupload_lists_only_living_pictures() {
        let mut a = ImageAtlas::new();
        let p = pic(10, 10);
        a.place(&p, 0);
        {
            let q = pic(10, 10);
            a.place(&q, 0);
        }
        a.reupload_all();
        assert_eq!(a.pending.len(), 1);
    }
}
