// SPDX-License-Identifier: BSD-2-Clause
//! image: the pictures `image` nodes draw. An [`Entry`] is one picture: registered by id (the
//! `wolkit_image_*` verbs and `WOLKIT_PROP_IMAGE_ID`) or found by path (`image-source`,
//! [`source`]). Encoded pictures are decoded on the engine's worker threads ([`worker`]): PNG and
//! JPEG, with the EXIF orientation applied ([`exif`]) and an optional cap on the longer side.
//! The result is a premultiplied RGBA8 mip chain ([`Pixels`]) whose largest level is at most
//! [`MAX_DRAW_SIDE`] a side; paint picks a level and both rasterisers sample it the same way
//! ([`fit`]). Until a picture is ready its node shows a placeholder.
//!
//! Limits (every one checked before memory is spent): 16384 pixels a side and 256 megapixels as
//! decoded, 64 MiB encoded, 512 MB of pictures per process (encoded bytes while pending, levels
//! once decoded).

pub mod decode;
pub mod exif;
pub mod fit;
pub mod icon;
pub mod source;
mod worker;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::loop_::Waker;

pub const MAX_SIDE: u32 = 16_384;
pub const MAX_PIXELS: u64 = 256 * 1024 * 1024;
pub const MAX_ENCODED: usize = 64 << 20;
pub const MAX_TOTAL: u64 = 512 << 20;
/// The largest level kept and drawn: the side of a GPU image atlas page. A bigger picture is
/// halved until it fits, so "actual size" of a larger photo is drawn from this level.
pub const MAX_DRAW_SIDE: u32 = 2048;
/// Levels stop halving at this longer side.
const MIN_LEVEL_SIDE: u32 = 16;

/// `wolkit_image_get_state`.
pub const STATE_PENDING: u32 = 0;
pub const STATE_READY: u32 = 1;
pub const STATE_FAILED: u32 = 2;

/// One level of a mip chain: premultiplied RGBA8, tightly packed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

impl Level {
    /// The premultiplied texel at `(x, y)`, clamped to the edges.
    #[inline]
    pub fn texel(&self, x: i32, y: i32) -> [u8; 4] {
        let cx = u32::try_from(x.max(0))
            .unwrap_or(0)
            .min(self.w.saturating_sub(1));
        let cy = u32::try_from(y.max(0))
            .unwrap_or(0)
            .min(self.h.saturating_sub(1));
        let i =
            usize::try_from((u64::from(cy) * u64::from(self.w) + u64::from(cx)) * 4).unwrap_or(0);
        match self.px.get(i..i + 4) {
            Some(p) => [p[0], p[1], p[2], p[3]],
            None => [0; 4],
        }
    }
}

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);

/// A decoded picture: its oriented natural size and the levels paint draws from, largest first.
#[derive(Debug)]
pub struct Pixels {
    /// Unique per decode (the GPU atlas key).
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Level>,
}

impl Pixels {
    /// A chain from premultiplied level 0 (halved first while it is over [`MAX_DRAW_SIDE`]).
    pub fn from_level(width: u32, height: u32, mut top: Level) -> Pixels {
        while top.w.max(top.h) > MAX_DRAW_SIDE {
            top = decode::halve(&top);
        }
        let mut levels = vec![top];
        while let Some(l) = levels.last()
            && l.w.max(l.h) > MIN_LEVEL_SIDE
        {
            let next = decode::halve(l);
            levels.push(next);
        }
        Pixels {
            key: NEXT_KEY.fetch_add(1, Ordering::Relaxed),
            width,
            height,
            levels,
        }
    }

    pub fn bytes(&self) -> u64 {
        self.levels
            .iter()
            .map(|l| u64::try_from(l.px.len()).unwrap_or(0))
            .sum()
    }

    /// The level to draw a destination of `dw x dh` device pixels from: the smallest that is at
    /// least that size, else the largest. Both rasterisers use this rule.
    pub fn level_for(&self, dw: f32, dh: f32) -> usize {
        let mut pick = 0;
        for (i, l) in self.levels.iter().enumerate() {
            if l.w as f32 + 0.5 >= dw && l.h as f32 + 0.5 >= dh {
                pick = i;
            } else {
                break;
            }
        }
        pick
    }
}

/// A picture as a display list holds it: compared by identity.
#[derive(Clone)]
pub struct ImageRef(pub Arc<Pixels>);

impl PartialEq for ImageRef {
    fn eq(&self, o: &ImageRef) -> bool {
        Arc::ptr_eq(&self.0, &o.0)
    }
}

impl std::fmt::Debug for ImageRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ImageRef({}x{} #{})",
            self.0.width, self.0.height, self.0.key
        )
    }
}

#[derive(Clone, Debug)]
pub enum State {
    Pending,
    Ready(Arc<Pixels>),
    Failed(String),
}

struct Inner {
    /// Oriented natural size, once the header was read.
    natural: Option<(u32, u32)>,
    state: State,
    /// Layout asked for the size before it was known: learning it moves [`size_generation`].
    size_wanted: bool,
}

impl Inner {
    fn learn(&mut self, wh: (u32, u32)) {
        self.natural = Some(wh);
        if std::mem::take(&mut self.size_wanted) {
            SIZE_GENERATION.fetch_add(1, Ordering::AcqRel);
        }
    }
}

/// One picture.
pub struct Entry {
    pub id: u32,
    inner: Mutex<Inner>,
    /// Bytes this entry holds against [`MAX_TOTAL`].
    held: AtomicU64,
}

// Removes the entry from the id registry and returns its bytes to the process budget.
impl Drop for Entry {
    fn drop(&mut self) {
        release(self.held.load(Ordering::Acquire));
        lock(&REGISTRY).remove(&self.id);
    }
}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);
static HELD: AtomicU64 = AtomicU64::new(0);
static GENERATION: AtomicU64 = AtomicU64::new(1);
static SIZE_GENERATION: AtomicU64 = AtomicU64::new(1);
static REGISTRY: LazyLock<Mutex<HashMap<u32, Weak<Entry>>>> = LazyLock::new(Default::default);
static PENDING: LazyLock<(Mutex<usize>, Condvar)> = LazyLock::new(Default::default);
static SUBSCRIBERS: LazyLock<Mutex<Vec<Weak<Waker>>>> = LazyLock::new(Default::default);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn reserve(n: u64) -> Result<()> {
    if HELD.fetch_add(n, Ordering::AcqRel) + n > MAX_TOTAL {
        HELD.fetch_sub(n, Ordering::AcqRel);
        return Err(Error::limit(
            "Images take at most 512 MB; release some first.",
        ));
    }
    Ok(())
}

fn release(n: u64) {
    if n > 0 {
        HELD.fetch_sub(n, Ordering::AcqRel);
    }
}

/// Bytes held by every picture in the process.
pub fn held_bytes() -> u64 {
    HELD.load(Ordering::Acquire)
}

/// Changes whenever any picture became ready or failed: a frame drawn at an older generation
/// may show a placeholder that can now be a picture.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

/// Changes whenever a natural size layout asked for became known (`Entry::natural_for_layout`);
/// part of the layout key (`window::pipeline::ensure_layout`).
pub fn size_generation() -> u64 {
    SIZE_GENERATION.load(Ordering::Acquire)
}

/// A wake-up the native backend polls: woken whenever [`generation`] moves.
pub fn subscribe() -> Option<Arc<Waker>> {
    let w = Arc::new(Waker::new().ok()?);
    let mut subs = lock(&SUBSCRIBERS);
    subs.retain(|s| s.strong_count() > 0);
    subs.push(Arc::downgrade(&w));
    Some(w)
}

fn changed() {
    GENERATION.fetch_add(1, Ordering::AcqRel);
    for s in lock(&SUBSCRIBERS).iter().filter_map(Weak::upgrade) {
        s.wake();
    }
}

/// Waits until no decode is queued or running, at most `timeout`. True when none is left. The
/// headless backend calls this before it draws, so its frames do not depend on timing.
pub fn settle(timeout: Duration) -> bool {
    let end = Instant::now() + timeout;
    let (m, cv) = &*PENDING;
    let mut n = lock(m);
    while *n > 0 {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        n = cv
            .wait_timeout(n, left)
            .map_or_else(|e| e.into_inner().0, |r| r.0);
    }
    true
}

fn pending_add(k: isize) {
    let (m, cv) = &*PENDING;
    let mut n = lock(m);
    *n = n.saturating_add_signed(k);
    if *n == 0 {
        cv.notify_all();
    }
}

impl Entry {
    fn new(natural: Option<(u32, u32)>, state: State, held: u64) -> Arc<Entry> {
        let e = Arc::new(Entry {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            inner: Mutex::new(Inner {
                natural,
                state,
                size_wanted: false,
            }),
            held: AtomicU64::new(held),
        });
        lock(&REGISTRY).insert(e.id, Arc::downgrade(&e));
        e
    }

    /// Straight RGBA8 pixels, `stride` bytes a row (ready at once).
    pub fn from_rgba(w: u32, h: u32, stride: usize, px: &[u8]) -> Result<Arc<Entry>> {
        check_size(w, h)?;
        let row = usize::try_from(w).unwrap_or(usize::MAX).saturating_mul(4);
        let mut packed = Vec::with_capacity(row.saturating_mul(usize::try_from(h).unwrap_or(0)));
        for y in 0..usize::try_from(h).unwrap_or(0) {
            let a = y * stride;
            packed.extend_from_slice(px.get(a..a + row).unwrap_or_default());
        }
        decode::premultiply(&mut packed);
        let pixels = Pixels::from_level(w, h, Level { w, h, px: packed });
        let n = pixels.bytes();
        reserve(n)?;
        Ok(Entry::new(Some((w, h)), State::Ready(Arc::new(pixels)), n))
    }

    /// Encoded bytes (PNG, JPEG; SVG is kept but not drawn): the header is read now, the pixels
    /// on a worker. `max_side` caps the longer side of the result (0: no cap).
    pub fn from_encoded(bytes: Vec<u8>, max_side: u32) -> Result<Arc<Entry>> {
        let n = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        match decode::sniff(&bytes) {
            // Accepted (the 1.0 contract) but not kept: nothing draws SVG yet.
            Some(decode::Format::Svg) => {
                Ok(Entry::new(None, State::Failed(decode::NO_SVG.into()), 0))
            }
            Some(_) => {
                let h = decode::probe(&bytes)?;
                reserve(n)?;
                let e = Entry::new(Some((h.width, h.height)), State::Pending, n);
                worker::submit(&e, worker::Input::Bytes(bytes), max_side);
                Ok(e)
            }
            None => Err(Error::invalid(decode::NOT_AN_IMAGE)),
        }
    }

    /// An icon of the theme (`icon:<name>`), drawn `px` pixels square in `color` on a worker.
    pub fn icon(name: String, px: u32, color: crate::paint::geom::Color) -> Arc<Entry> {
        let e = Entry::new(Some((px, px)), State::Pending, 0);
        worker::submit(&e, worker::Input::Icon { name, px, color }, 0);
        e
    }

    /// The size of a file only: its header is read on a worker (layout of a part with no size).
    pub fn header_of_file(path: std::path::PathBuf) -> Arc<Entry> {
        let e = Entry::new(None, State::Pending, 0);
        worker::submit(&e, worker::Input::Header(path), 0);
        e
    }

    /// A file, read and decoded on a worker.
    pub fn from_file(path: std::path::PathBuf, max_side: u32) -> Arc<Entry> {
        let e = Entry::new(None, State::Pending, 0);
        worker::submit(&e, worker::Input::File(path), max_side);
        e
    }

    pub fn state(&self) -> State {
        lock(&self.inner).state.clone()
    }

    pub fn state_code(&self) -> u32 {
        match lock(&self.inner).state {
            State::Pending => STATE_PENDING,
            State::Ready(_) => STATE_READY,
            State::Failed(_) => STATE_FAILED,
        }
    }

    /// The picture, when it is ready.
    pub fn pixels(&self) -> Option<Arc<Pixels>> {
        match &lock(&self.inner).state {
            State::Ready(p) => Some(Arc::clone(p)),
            _ => None,
        }
    }

    /// Oriented natural size, once known.
    pub fn natural(&self) -> Option<(u32, u32)> {
        lock(&self.inner).natural
    }

    fn set_natural(&self, wh: (u32, u32)) {
        lock(&self.inner).learn(wh);
    }

    /// The natural size for layout. When it is not known yet the entry remembers that layout
    /// asked, and learning it moves [`size_generation`] (so the window lays out again).
    pub fn natural_for_layout(&self) -> Option<(u32, u32)> {
        let mut g = lock(&self.inner);
        if g.natural.is_none() && matches!(g.state, State::Pending) {
            g.size_wanted = true;
        }
        g.natural
    }

    /// A size-only entry is done: its header was read (or could not be).
    fn finish_header(&self, wh: Option<(u32, u32)>) {
        let mut g = lock(&self.inner);
        match wh {
            Some(wh) => g.learn(wh),
            None => g.size_wanted = false,
        }
        g.state = State::Failed(String::new());
        drop(g);
        changed();
    }

    /// Worker side: the decode finished. The budget moves from the encoded bytes to the levels.
    fn finish(&self, r: Result<Pixels>) {
        let state = match r {
            Ok(p) => {
                let n = p.bytes();
                let before = self.held.load(Ordering::Acquire);
                release(before);
                match reserve(n) {
                    Ok(()) => {
                        self.held.store(n, Ordering::Release);
                        let mut g = lock(&self.inner);
                        g.learn((p.width, p.height));
                        g.state = State::Ready(Arc::new(p));
                        drop(g);
                        changed();
                        return;
                    }
                    Err(e) => {
                        self.held.store(0, Ordering::Release);
                        State::Failed(e.message)
                    }
                }
            }
            Err(e) => {
                release(self.held.swap(0, Ordering::AcqRel));
                State::Failed(e.message)
            }
        };
        lock(&self.inner).state = state;
        changed();
    }
}

/// The picture registered under `id`, if it is still alive.
pub fn lookup(id: u32) -> Option<Arc<Entry>> {
    let w = lock(&REGISTRY).get(&id).cloned();
    w.and_then(|w| w.upgrade())
}

pub fn check_size(w: u32, h: u32) -> Result<()> {
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || u64::from(w) * u64::from(h) > MAX_PIXELS
    {
        return Err(Error::limit(
            "An image is at most 16384 pixels a side and 256 megapixels.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
        c.iter()
            .copied()
            .cycle()
            .take(usize::try_from(w * h * 4).unwrap())
            .collect()
    }

    #[test]
    fn rgba_entries_are_ready_and_registered() {
        let e = Entry::from_rgba(8, 4, 32, &solid(8, 4, [255, 0, 0, 255])).unwrap();
        assert_eq!(e.state_code(), STATE_READY);
        assert_eq!(e.natural(), Some((8, 4)));
        let p = e.pixels().unwrap();
        assert_eq!(p.levels[0].texel(3, 2), [255, 0, 0, 255]);
        assert!(lookup(e.id).is_some());
        let id = e.id;
        drop(p);
        drop(e);
        assert!(lookup(id).is_none());
    }

    #[test]
    fn mip_chain_halves_down_to_the_smallest_level() {
        let e = Entry::from_rgba(100, 40, 400, &solid(100, 40, [10, 20, 30, 255])).unwrap();
        let p = e.pixels().unwrap();
        let sides: Vec<_> = p.levels.iter().map(|l| (l.w, l.h)).collect();
        assert_eq!(sides, vec![(100, 40), (50, 20), (25, 10), (13, 5)]);
        assert_eq!(p.level_for(100.0, 40.0), 0);
        assert_eq!(p.level_for(40.0, 16.0), 1);
        assert_eq!(p.level_for(5.0, 2.0), 3);
        assert_eq!(p.level_for(400.0, 400.0), 0);
    }

    #[test]
    fn large_pictures_are_capped_at_the_atlas_page() {
        let l = Level {
            w: 4100,
            h: 10,
            px: vec![0; 4100 * 10 * 4],
        };
        let p = Pixels::from_level(4100, 10, l);
        assert!(p.levels[0].w <= MAX_DRAW_SIDE, "{}", p.levels[0].w);
        assert_eq!((p.width, p.height), (4100, 10));
    }

    #[test]
    fn oversized_rgba_is_refused_before_allocating() {
        assert!(Entry::from_rgba(20_000, 1, 80_000, &[]).is_err());
        assert!(Entry::from_rgba(0, 1, 0, &[]).is_err());
    }
}
