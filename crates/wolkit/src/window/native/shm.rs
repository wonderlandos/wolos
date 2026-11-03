// SPDX-License-Identifier: BSD-2-Clause
//! Shared-memory buffers for the CPU path: one anonymous file, up to three buffers in it (double
//! buffering, a third only while the compositor holds both), written with positioned writes so
//! the engine maps nothing. Pixels are opaque: the format is the RGBX order when the compositor
//! offers it (the rasteriser's byte order, no conversion), else the mandatory BGRX with red and
//! blue swapped on the way.

use super::client::Client;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use wayland_client::QueueHandle;
use wayland_client::protocol::{wl_buffer, wl_shm, wl_shm_pool};

/// Buffers kept at most.
pub const MAX_BUFFERS: usize = 3;
/// `wl_shm` format codes.
pub const XRGB8888: u32 = 1;
pub const XBGR8888: u32 = 0x3432_4258;

struct Slot {
    buffer: wl_buffer::WlBuffer,
    offset: usize,
    busy: bool,
    id: u32,
}

pub struct Buffers {
    file: Option<std::fs::File>,
    pool: Option<wl_shm_pool::WlShmPool>,
    slots: Vec<Slot>,
    width: u32,
    height: u32,
    pub format: u32,
    next_id: u32,
    /// Scratch for the format swap.
    swap: Vec<u8>,
}

impl Default for Buffers {
    fn default() -> Self {
        Buffers {
            file: None,
            pool: None,
            slots: Vec::new(),
            width: 0,
            height: 0,
            format: XRGB8888,
            next_id: 1,
            swap: Vec::new(),
        }
    }
}

fn stride(w: u32) -> usize {
    usize::try_from(w).unwrap_or(0) * 4
}

impl Buffers {
    /// Pick the format from what the compositor offers.
    pub fn choose_format(&mut self, offered: &[u32]) {
        self.format = if offered.contains(&XBGR8888) {
            XBGR8888
        } else {
            XRGB8888
        };
    }

    fn frame_bytes(&self) -> usize {
        stride(self.width) * usize::try_from(self.height).unwrap_or(0)
    }

    /// Drop every buffer (size change, hidden window, Memory Saver).
    pub fn clear(&mut self) {
        for s in self.slots.drain(..) {
            s.buffer.destroy();
        }
        if let Some(p) = self.pool.take() {
            p.destroy();
        }
        self.file = None;
        self.width = 0;
        self.height = 0;
    }

    /// A free buffer of `w` by `h` device pixels, made if needed; `None` when all are busy and
    /// no more may be made (skip this frame) or memory could not be had.
    fn acquire(
        &mut self,
        shm: &wl_shm::WlShm,
        qh: &QueueHandle<Client>,
        w: u32,
        h: u32,
    ) -> Option<usize> {
        if (w, h) != (self.width, self.height) {
            self.clear();
            self.width = w;
            self.height = h;
        }
        if let Some(i) = self.slots.iter().position(|s| !s.busy) {
            return Some(i);
        }
        if self.slots.len() >= MAX_BUFFERS {
            return None;
        }
        let one = self.frame_bytes();
        let total = one * (self.slots.len() + 1);
        if self.file.is_none() {
            self.file = Some(super::sys::memfd(c"wolkit-shm").ok()?);
        }
        let file = self.file.as_ref()?;
        file.set_len(u64::try_from(total).ok()?).ok()?;
        let size = i32::try_from(total).ok()?;
        match &self.pool {
            Some(p) => p.resize(size),
            None => self.pool = Some(shm.create_pool(file.as_fd(), size, qh, ())),
        }
        let pool = self.pool.as_ref()?;
        let offset = one * self.slots.len();
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let format = if self.format == XBGR8888 {
            wl_shm::Format::Xbgr8888
        } else {
            wl_shm::Format::Xrgb8888
        };
        let buffer = pool.create_buffer(
            i32::try_from(offset).ok()?,
            i32::try_from(w).ok()?,
            i32::try_from(h).ok()?,
            i32::try_from(stride(w)).ok()?,
            format,
            qh,
            id,
        );
        self.slots.push(Slot {
            buffer,
            offset,
            busy: false,
            id,
        });
        Some(self.slots.len() - 1)
    }

    /// Copy an RGBA frame into a free buffer and hand the buffer out (now busy until released).
    pub fn put(
        &mut self,
        shm: &wl_shm::WlShm,
        qh: &QueueHandle<Client>,
        w: u32,
        h: u32,
        rgba: &[u8],
    ) -> Option<wl_buffer::WlBuffer> {
        let i = self.acquire(shm, qh, w, h)?;
        let n = self.frame_bytes();
        let px = rgba.get(..n)?;
        let bytes: &[u8] = if self.format == XBGR8888 {
            px
        } else {
            self.swap.clear();
            self.swap.extend_from_slice(px);
            for p in self.swap.as_chunks_mut::<4>().0 {
                p.swap(0, 2);
            }
            &self.swap
        };
        self.file
            .as_ref()?
            .write_all_at(bytes, u64::try_from(self.slots[i].offset).ok()?)
            .ok()?;
        self.slots[i].busy = true;
        Some(self.slots[i].buffer.clone())
    }

    pub fn released(&mut self, id: u32) {
        if let Some(s) = self.slots.iter_mut().find(|s| s.id == id) {
            s.busy = false;
        }
    }
}
