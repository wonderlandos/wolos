// SPDX-License-Identifier: BSD-2-Clause
//! The glyph atlas, CPU side: R8 coverage pages with a shelf packer.
//!
//! * Pages are `page_size x page_size` single-channel images (2048 by default), allocated when
//!   first needed and capped at `cap_bytes` in total (32 MiB by default, so eight pages).
//! * Eviction is by page: when no page has room, the least recently used page is cleared and its
//!   generation bumped, which invalidates every slot that pointed into it in one step.
//! * Each page tracks a dirty rectangle so the GPU path uploads only what changed.
//! * Every lookup counts a hit or a miss; steady-state text must show a 100% hit rate.

use std::collections::HashMap;

use crate::paint::geom::PixelRect;
use crate::text::glyph::{GlyphBitmap, GlyphKey};

pub const DEFAULT_PAGE_SIZE: u32 = 2048;
pub const DEFAULT_CAP_BYTES: usize = 32 * 1024 * 1024;
/// Gutter around every glyph so sampling never bleeds into a neighbour.
const PAD: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub page: u16,
    generation: u32,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub left: i16,
    pub top: i16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AtlasStats {
    pub hits: u64,
    pub misses: u64,
    /// Pages cleared to make room.
    pub evictions: u64,
    /// Glyphs that did not fit any page even after eviction.
    pub rejected: u64,
}

impl AtlasStats {
    pub fn hit_rate(&self) -> f64 {
        let n = self.hits + self.misses;
        if n == 0 {
            1.0
        } else {
            self.hits as f64 / n as f64
        }
    }
}

struct Shelf {
    y: u32,
    h: u32,
    x: u32,
}

struct Page {
    pixels: Vec<u8>,
    generation: u32,
    shelves: Vec<Shelf>,
    next_y: u32,
    last_used: u64,
    dirty: Option<PixelRect>,
}

pub struct Atlas {
    size: u32,
    max_pages: usize,
    pages: Vec<Page>,
    map: HashMap<GlyphKey, Slot>,
    epoch: u64,
    pub stats: AtlasStats,
}

impl Default for Atlas {
    fn default() -> Self {
        Atlas::new(DEFAULT_PAGE_SIZE, DEFAULT_CAP_BYTES)
    }
}

impl Atlas {
    /// An atlas of `page_size`-square pages using at most `cap_bytes` (at least one page).
    pub fn new(page_size: u32, cap_bytes: usize) -> Atlas {
        let size = page_size.clamp(16, 8192);
        let page_bytes = (size as usize) * (size as usize);
        Atlas {
            size,
            max_pages: (cap_bytes / page_bytes).max(1),
            pages: Vec::new(),
            map: HashMap::new(),
            epoch: 1,
            stats: AtlasStats::default(),
        }
    }

    pub fn page_size(&self) -> u32 {
        self.size
    }

    pub fn max_pages(&self) -> usize {
        self.max_pages
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Bytes currently allocated for page pixels.
    pub fn allocated_bytes(&self) -> usize {
        self.pages.len() * (self.size as usize) * (self.size as usize)
    }

    /// Marks the start of a frame; pages touched since are protected from eviction in favour of
    /// older ones.
    pub fn begin_frame(&mut self) {
        self.epoch += 1;
    }

    /// Looks a glyph up; a stale slot (its page was cleared) is a miss.
    pub fn get(&mut self, key: &GlyphKey) -> Option<Slot> {
        if let Some(s) = self.map.get(key).copied() {
            if let Some(p) = self.pages.get_mut(s.page as usize) {
                if p.generation == s.generation {
                    p.last_used = self.epoch;
                    self.stats.hits += 1;
                    return Some(s);
                }
            }
        }
        self.stats.misses += 1;
        None
    }

    /// Records that `key` has no image (an empty glyph) is the caller's concern; this stores a
    /// rendered bitmap and returns its slot, or `None` when it cannot fit at all.
    pub fn insert(&mut self, key: GlyphKey, bm: &GlyphBitmap) -> Option<Slot> {
        let (w, h) = (u32::from(bm.width), u32::from(bm.height));
        if w + 2 * PAD > self.size || h + 2 * PAD > self.size {
            self.stats.rejected += 1;
            return None;
        }
        let (page, x, y) = self.allocate(w + 2 * PAD, h + 2 * PAD)?;
        let size = self.size as usize;
        let p = &mut self.pages[page];
        for row in 0..h as usize {
            let dst = (y as usize + PAD as usize + row) * size + x as usize + PAD as usize;
            let src = row * w as usize;
            p.pixels[dst..dst + w as usize].copy_from_slice(&bm.data[src..src + w as usize]);
        }
        let r = PixelRect { x0: x as i32, y0: y as i32, x1: (x + w + 2 * PAD) as i32, y1: (y + h + 2 * PAD) as i32 };
        p.dirty = Some(match p.dirty {
            None => r,
            Some(d) => PixelRect { x0: d.x0.min(r.x0), y0: d.y0.min(r.y0), x1: d.x1.max(r.x1), y1: d.y1.max(r.y1) },
        });
        p.last_used = self.epoch;
        let slot = Slot {
            page: page as u16,
            generation: p.generation,
            x: (x + PAD) as u16,
            y: (y + PAD) as u16,
            w: bm.width,
            h: bm.height,
            left: bm.left,
            top: bm.top,
        };
        self.map.insert(key, slot);
        Some(slot)
    }

    fn allocate(&mut self, w: u32, h: u32) -> Option<(usize, u32, u32)> {
        // 1. Existing pages, newest first (the page being filled is most likely to have room).
        for pi in (0..self.pages.len()).rev() {
            if let Some((x, y)) = Self::fit(&mut self.pages[pi], self.size, w, h) {
                return Some((pi, x, y));
            }
        }
        // 2. A fresh page.
        if self.pages.len() < self.max_pages {
            let n = (self.size as usize) * (self.size as usize);
            self.pages.push(Page {
                pixels: vec![0; n],
                generation: 1,
                shelves: Vec::new(),
                next_y: 0,
                last_used: self.epoch,
                dirty: None,
            });
            let pi = self.pages.len() - 1;
            let (x, y) = Self::fit(&mut self.pages[pi], self.size, w, h)?;
            return Some((pi, x, y));
        }
        // 3. Clear the least recently used page.
        let victim = (0..self.pages.len()).min_by_key(|&i| self.pages[i].last_used)?;
        self.clear_page(victim);
        let (x, y) = Self::fit(&mut self.pages[victim], self.size, w, h)?;
        Some((victim, x, y))
    }

    fn clear_page(&mut self, pi: usize) {
        self.stats.evictions += 1;
        let p = &mut self.pages[pi];
        p.generation = p.generation.wrapping_add(1);
        p.shelves.clear();
        p.next_y = 0;
        p.pixels.iter_mut().for_each(|b| *b = 0);
        p.dirty = Some(PixelRect { x0: 0, y0: 0, x1: self.size as i32, y1: self.size as i32 });
        self.map.retain(|_, s| s.page as usize != pi);
    }

    fn fit(p: &mut Page, size: u32, w: u32, h: u32) -> Option<(u32, u32)> {
        // Reuse a shelf at most 1.5x taller than the glyph.
        for s in &mut p.shelves {
            if s.h >= h && s.h * 2 <= h * 3 && s.x + w <= size {
                let x = s.x;
                s.x += w;
                return Some((x, s.y));
            }
        }
        // Round the new shelf's height up so similar glyphs share it.
        let sh = (h + 3) & !3;
        if p.next_y + sh <= size {
            let y = p.next_y;
            p.next_y += sh;
            p.shelves.push(Shelf { y, h: sh, x: w });
            return Some((0, y));
        }
        None
    }

    /// The pixels of a page (`page_size` bytes per row).
    pub fn page_pixels(&self, page: usize) -> Option<&[u8]> {
        self.pages.get(page).map(|p| p.pixels.as_slice())
    }

    /// Takes the dirty rectangle of a page for upload.
    pub fn take_dirty(&mut self, page: usize) -> Option<PixelRect> {
        self.pages.get_mut(page)?.dirty.take()
    }

    /// Drops every page and entry (Memory Saver trim).
    pub fn trim(&mut self) {
        self.pages.clear();
        self.pages.shrink_to_fit();
        self.map.clear();
    }

    /// The coverage bytes of a slot as `(row stride, data starting at the slot's top-left)`.
    pub fn slot_pixels(&self, s: &Slot) -> Option<(usize, &[u8])> {
        let p = self.pages.get(s.page as usize)?;
        if p.generation != s.generation {
            return None;
        }
        let size = self.size as usize;
        Some((size, &p.pixels[s.y as usize * size + s.x as usize..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::fontdb::FontId;

    fn bm(w: u16, h: u16, v: u8) -> GlyphBitmap {
        GlyphBitmap { width: w, height: h, left: 0, top: h as i16, data: vec![v; w as usize * h as usize] }
    }

    fn key(g: u32) -> GlyphKey {
        GlyphKey::new(FontId(0), g, 16.0, 0)
    }

    #[test]
    fn insert_then_hit() {
        let mut a = Atlas::new(64, 64 * 64 * 2);
        assert!(a.get(&key(1)).is_none());
        let s = a.insert(key(1), &bm(8, 10, 7)).unwrap();
        let got = a.get(&key(1)).unwrap();
        assert_eq!(got, s);
        let (stride, px) = a.slot_pixels(&got).unwrap();
        assert_eq!(stride, 64);
        assert_eq!(px[0], 7);
        assert_eq!(a.stats.hits, 1);
        assert_eq!(a.stats.misses, 1);
    }

    #[test]
    fn page_eviction_invalidates_slots_and_respects_cap() {
        let mut a = Atlas::new(64, 64 * 64 * 2);
        assert_eq!(a.max_pages(), 2);
        // 24x24 glyphs: 4 per page (26x26 with padding; two per shelf row, two rows).
        for g in 0..40u32 {
            a.begin_frame();
            if a.get(&key(g)).is_none() {
                a.insert(key(g), &bm(24, 24, g as u8)).unwrap();
            }
        }
        assert!(a.page_count() <= 2);
        assert!(a.allocated_bytes() <= 64 * 64 * 2);
        assert!(a.stats.evictions > 0);
        // Early glyphs were evicted.
        assert!(a.get(&key(0)).is_none());
        // The most recent one is still there with the right pixels.
        let s = a.get(&key(39)).unwrap();
        assert_eq!(a.slot_pixels(&s).unwrap().1[0], 39);
    }

    #[test]
    fn oversized_glyph_is_rejected() {
        let mut a = Atlas::new(32, 32 * 32);
        assert!(a.insert(key(1), &bm(31, 8, 1)).is_none());
        assert_eq!(a.stats.rejected, 1);
    }

    #[test]
    fn dirty_rect_covers_inserts_and_resets() {
        let mut a = Atlas::new(64, 64 * 64);
        a.insert(key(1), &bm(8, 8, 1)).unwrap();
        let d = a.take_dirty(0).unwrap();
        assert!(d.x1 >= 10 && d.y1 >= 10);
        assert!(a.take_dirty(0).is_none());
    }
}
