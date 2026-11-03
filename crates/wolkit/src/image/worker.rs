// SPDX-License-Identifier: BSD-2-Clause
//! The decode workers: a small pool of named threads (`wolkit-decode`, at most four, made on
//! first use) that read files and decode pictures off the app's thread. A job holds only a weak
//! reference, so a picture released before its turn is skipped. Each finished job bumps the
//! image generation and wakes the subscribed windows (`image::subscribe`).

use std::collections::VecDeque;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, LazyLock, Mutex, Weak};

use super::{Entry, MAX_ENCODED, decode, lock, pending_add};
use crate::error::{Error, Result};

pub enum Input {
    Bytes(Vec<u8>),
    File(PathBuf),
    /// The size only: the first [`HEADER_BYTES`] of the file.
    Header(PathBuf),
    /// An icon of the theme, `px` square, symbolic ones in `color`.
    Icon {
        name: String,
        px: u32,
        color: crate::paint::geom::Color,
    },
}

/// Enough of a file for its header and camera block (APP segments are at most 64 KiB each).
const HEADER_BYTES: u64 = 1 << 20;

struct Job {
    entry: Weak<Entry>,
    input: Input,
    max_side: u32,
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    threads: usize,
    idle: usize,
}

static QUEUE: LazyLock<(Mutex<Queue>, Condvar)> = LazyLock::new(Default::default);

fn pool_size() -> usize {
    std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 4))
}

pub fn submit(e: &Arc<Entry>, input: Input, max_side: u32) {
    pending_add(1);
    let (m, cv) = &*QUEUE;
    let mut q = lock(m);
    q.jobs.push_back(Job {
        entry: Arc::downgrade(e),
        input,
        max_side,
    });
    if q.idle == 0 && q.threads < pool_size() {
        let spawned = std::thread::Builder::new()
            .name("wolkit-decode".into())
            .spawn(run);
        if spawned.is_ok() {
            q.threads += 1;
        }
    }
    let alone = q.threads == 0;
    drop(q);
    cv.notify_one();
    if alone {
        // No thread could be made: decode here rather than never.
        run_one();
    }
}

fn next(block: bool) -> Option<Job> {
    let (m, cv) = &*QUEUE;
    let mut q = lock(m);
    loop {
        if let Some(j) = q.jobs.pop_front() {
            return Some(j);
        }
        if !block {
            return None;
        }
        q.idle += 1;
        q = cv.wait(q).unwrap_or_else(|e| e.into_inner());
        q.idle -= 1;
    }
}

fn run() {
    while let Some(j) = next(true) {
        work(j);
    }
}

fn run_one() {
    if let Some(j) = next(false) {
        work(j);
    }
}

fn read_file(p: &std::path::Path) -> Result<Vec<u8>> {
    read_head(p, u64::try_from(MAX_ENCODED).unwrap_or(u64::MAX) + 1)
}

fn read_head(p: &std::path::Path, cap: u64) -> Result<Vec<u8>> {
    let f = std::fs::File::open(p).map_err(|_| Error::invalid(super::source::UNREADABLE))?;
    let mut b = Vec::new();
    f.take(cap)
        .read_to_end(&mut b)
        .map_err(|_| Error::invalid(super::source::UNREADABLE))?;
    if b.len() > MAX_ENCODED {
        return Err(Error::limit("A picture file is at most 64 MiB."));
    }
    Ok(b)
}

fn work(j: Job) {
    if let (Some(e), Input::Header(p)) = (j.entry.upgrade(), &j.input) {
        let wh = std::panic::catch_unwind(|| {
            let b = read_head(p, HEADER_BYTES).ok()?;
            decode::probe(&b).ok().map(|h| (h.width, h.height))
        });
        e.finish_header(wh.ok().flatten());
    } else if let Some(e) = j.entry.upgrade() {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let bytes = match j.input {
                Input::Bytes(b) => b,
                // Handled above; a size-only job never decodes.
                Input::Header(_) => return Err(Error::invalid(decode::DAMAGED)),
                Input::Icon { name, px, color } => return super::icon::load(&name, px, color),
                Input::File(p) => {
                    let b = read_file(&p)?;
                    e.set_natural({
                        let h = decode::probe(&b)?;
                        (h.width, h.height)
                    });
                    b
                }
            };
            decode::decode(&bytes, j.max_side)
        }));
        e.finish(r.unwrap_or_else(|_| Err(Error::invalid(decode::DAMAGED))));
    }
    pending_add(-1);
}
