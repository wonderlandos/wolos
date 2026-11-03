// SPDX-License-Identifier: BSD-2-Clause
//! Clipboard and drag and drop over the Wayland data device (plan section 2.7). A copy or cut in
//! a text field offers the text as a data source; a paste reads the selection offer's text
//! through a socket pair without blocking (the descriptor joins the engine's wait set) and puts
//! it in the field. The compositor mediates both, so no consent grant is involved; programmatic
//! clipboard access stays a libwol verb with its grants.
//!
//! A drop on a part that asked for `DROP` delivers `u32 fd`, `f32 x`, `f32 y`, `u32 length` and
//! the chosen type: the app reads the data from `fd` and closes it. The type is the first of
//! `text/uri-list`, UTF-8 text, plain text that is offered, else the first offered.

use super::mime;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, IntoRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

/// A paste being read.
pub struct Reading {
    stream: UnixStream,
    buf: Vec<u8>,
}

/// A drag over the window.
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    pub offer: u32,
    pub serial: u32,
    pub x: f32,
    pub y: f32,
    /// The type accepted for the part under the pointer, if it takes drops.
    pub accepted: Option<String>,
    pub target: crate::scene::Idx,
}

#[derive(Default)]
pub struct Clipboard {
    /// What we offered last, while our source is the selection.
    pub ours: Option<String>,
    /// The selection offer, when another client owns the clipboard.
    pub selection: Option<u32>,
    pub reading: Vec<Reading>,
    pub drag: Option<Drag>,
}

/// The reader end of a paste or a drop, and the writer end to give the source.
pub fn pair() -> std::io::Result<(UnixStream, OwnedFd)> {
    let (r, w) = UnixStream::pair()?;
    r.set_nonblocking(true)?;
    Ok((r, OwnedFd::from(w)))
}

impl Clipboard {
    pub fn fds(&self, out: &mut Vec<RawFd>) {
        out.extend(self.reading.iter().map(|r| r.stream.as_raw_fd()));
    }

    pub fn start_read(&mut self, stream: UnixStream) {
        // At most a few pastes in flight; an older one is dropped (its text is stale anyway).
        if self.reading.len() >= 4 {
            self.reading.remove(0);
        }
        self.reading.push(Reading {
            stream,
            buf: Vec::new(),
        });
    }

    /// Read what arrived; the text of each paste that finished.
    pub fn poll(&mut self) -> Vec<String> {
        let mut done = Vec::new();
        let mut b = [0u8; 8192];
        self.reading.retain_mut(|r| {
            loop {
                match r.stream.read(&mut b) {
                    Ok(0) => {
                        done.push(mime::paste_text(&r.buf));
                        return false;
                    }
                    Ok(n) => {
                        r.buf.extend_from_slice(&b[..n]);
                        if r.buf.len() > mime::MAX_PASTE {
                            done.push(mime::paste_text(&r.buf));
                            return false;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return true,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => return false,
                }
            }
        });
        done
    }
}

/// Write `text` to a descriptor a pasting client gave us, off the engine's thread (the reader may
/// be slow; a stuck one only holds this short-lived thread).
pub fn send(text: String, fd: OwnedFd) {
    let spawned = std::thread::Builder::new()
        .name("wolkit-clip-tx".into())
        .spawn(move || {
            let mut f = std::fs::File::from(fd);
            // The reader went away: nothing to do.
            let _ = f.write_all(text.as_bytes());
        });
    // Without a thread the paste gets nothing, which the reader sees as an empty clipboard.
    let _ = spawned;
}

/// The payload of a `DROP` record; the descriptor is handed to the app (it closes it).
pub fn drop_payload(fd: OwnedFd, x: f32, y: f32, ty: &str) -> (Vec<u8>, Vec<u8>) {
    let raw = fd.into_raw_fd();
    let mut head = Vec::with_capacity(16);
    head.extend_from_slice(&raw.cast_unsigned().to_le_bytes());
    head.extend_from_slice(&x.to_le_bytes());
    head.extend_from_slice(&y.to_le_bytes());
    head.extend_from_slice(&crate::scene::ids::to_u32(ty.len()).to_le_bytes());
    (head, ty.as_bytes().to_vec())
}
