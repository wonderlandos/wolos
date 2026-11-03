// SPDX-License-Identifier: BSD-2-Clause
//! Pictures named by path: an `image` node whose `image-source` is an absolute path (or a
//! `file://` URL) is drawn from that file, read and decoded by the engine on a worker. The
//! process keeps one cache of them keyed by path and size bucket (the node's longer side times
//! two, rounded up to a power of two or a half step, at most `MAX_DRAW_SIDE`), so a thumbnail grid decodes
//! small. Least recently used entries go first past 512 entries or 256 MB.
//!
//! Other sources (`icon:` names) are not pictures this module draws.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use super::{Entry, MAX_DRAW_SIDE, State, lock};
use crate::num::trunc_u32;

pub const UNREADABLE: &str =
    "The picture file could not be opened; check that it exists and can be read.";

const MAX_ENTRIES: usize = 512;
const MAX_BYTES: u64 = 256 << 20;
const MIN_CAP: u32 = 64;

struct Cached {
    entry: Arc<Entry>,
    used: u64,
}

#[derive(Default)]
struct Cache {
    map: HashMap<(String, u32), Cached>,
    tick: u64,
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Default::default);

/// The file an `image-source` value names, if it names one.
pub fn path_of(src: &str) -> Option<&str> {
    let p = src.strip_prefix("file://").unwrap_or(src);
    (p.starts_with('/') && !p.contains('\0')).then_some(p)
}

/// Size buckets: powers of two and the half steps between them.
const BUCKETS: [u32; 10] = [64, 128, 192, 256, 384, 512, 768, 1024, 1536, MAX_DRAW_SIDE];

/// The size bucket for a node of `w x h` logical pixels: the smallest that holds its longer side
/// at twice the size (a high-density screen).
pub fn cap_for(w: f32, h: f32) -> u32 {
    let want = trunc_u32((w.max(h) * 2.0).ceil()).max(MIN_CAP);
    BUCKETS
        .iter()
        .copied()
        .find(|b| *b >= want)
        .unwrap_or(MAX_DRAW_SIDE)
}

/// The picture to draw for `src` in a node of `w x h`: a cached one of at least the bucket's
/// size when there is one; otherwise the bucket is requested, and until it is ready the largest
/// ready picture of the same file stands in (so a resize does not flash the placeholder).
pub fn get(src: &str, w: f32, h: f32) -> Option<Arc<Entry>> {
    let path = path_of(src)?;
    let cap = cap_for(w, h);
    let mut c = lock(&CACHE);
    c.tick += 1;
    let tick = c.tick;
    if let Some(hit) = c.map.get_mut(&(path.to_owned(), cap)) {
        hit.used = tick;
        let e = Arc::clone(&hit.entry);
        if !matches!(e.state(), State::Pending) {
            return Some(e);
        }
        drop(c);
        return Some(stand_in(path, cap).unwrap_or(e));
    }
    // A bigger one that is ready serves a smaller node too.
    let bigger = c
        .map
        .iter()
        .filter(|((p, k), x)| p == path && *k > cap && x.entry.pixels().is_some())
        .min_by_key(|((_, k), _)| *k)
        .map(|(_, x)| Arc::clone(&x.entry));
    if let Some(e) = bigger {
        return Some(e);
    }
    let e = Entry::from_file(path.into(), cap);
    c.map.insert(
        (path.to_owned(), cap),
        Cached {
            entry: Arc::clone(&e),
            used: tick,
        },
    );
    evict(&mut c, tick);
    drop(c);
    Some(stand_in(path, cap).unwrap_or(e))
}

/// The largest ready picture of `path` below `cap`.
fn stand_in(path: &str, cap: u32) -> Option<Arc<Entry>> {
    let c = lock(&CACHE);
    c.map
        .iter()
        .filter(|((p, k), x)| p == path && *k < cap && x.entry.pixels().is_some())
        .max_by_key(|((_, k), _)| *k)
        .map(|(_, x)| Arc::clone(&x.entry))
}

fn evict(c: &mut Cache, keep: u64) {
    let bytes = |c: &Cache| -> u64 {
        c.map
            .values()
            .filter_map(|x| x.entry.pixels())
            .map(|p| p.bytes())
            .sum()
    };
    while c.map.len() > MAX_ENTRIES || (c.map.len() > 1 && bytes(c) > MAX_BYTES) {
        let oldest = c
            .map
            .iter()
            .filter(|(_, x)| x.used != keep)
            .min_by_key(|(_, x)| x.used)
            .map(|(k, _)| k.clone());
        match oldest {
            Some(k) => {
                c.map.remove(&k);
            }
            None => break,
        }
    }
}

/// Memory Saver: forget every cached picture (the next frame asks again).
pub fn trim() {
    lock(&CACHE).map.clear();
}

/// Entries in the cache (tests and stats).
pub fn len() -> usize {
    lock(&CACHE).map.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_absolute_paths_name_files() {
        assert_eq!(path_of("/a/b.jpg"), Some("/a/b.jpg"));
        assert_eq!(path_of("file:///a/b.jpg"), Some("/a/b.jpg"));
        assert_eq!(path_of("icon:folder"), None);
        assert_eq!(path_of("b.jpg"), None);
    }

    #[test]
    fn size_buckets_hold_twice_the_longer_side() {
        assert_eq!(cap_for(160.0, 120.0), 384);
        assert_eq!(cap_for(64.0, 64.0), 128);
        assert_eq!(cap_for(10.0, 4.0), 64);
        assert_eq!(cap_for(5000.0, 10.0), MAX_DRAW_SIDE);
        assert_eq!(cap_for(f32::NAN, 0.0), 64);
    }

    #[test]
    fn a_missing_file_fails_and_is_cached_once() {
        let a = get("/nonexistent/wolkit-image-test.png", 50.0, 50.0).unwrap();
        assert!(super::super::settle(std::time::Duration::from_secs(10)));
        assert!(matches!(a.state(), State::Failed(_)));
        let b = get("/nonexistent/wolkit-image-test.png", 50.0, 50.0).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
    }
}
