// SPDX-License-Identifier: GPL-2.0-or-later
//! CDP framing. The browser's pipe frames messages with a NUL byte (fds 3 and 4 of the browser);
//! the Drive socket frames them with `\n` (CDP messages never contain a raw newline: JSON escapes
//! it). Both readers are bounded: a message longer than the maximum is an error, never buffered.
use std::io::{self, Read};

/// The longest single CDP message accepted from the browser (a full-page screenshot is base64).
pub const MAX_BROWSER_MSG: usize = 96 * 1024 * 1024;
/// The longest line accepted from a client.
pub const MAX_CLIENT_LINE: usize = 32 * 1024 * 1024;

/// Splits a byte stream on one delimiter with a hard cap on a single message.
pub struct Framer<R> {
    r: R,
    delim: u8,
    max: usize,
    buf: Vec<u8>,
    start: usize,
    end: usize,
}

impl<R: Read> Framer<R> {
    pub fn new(r: R, delim: u8, max: usize) -> Self {
        Self {
            r,
            delim,
            max,
            buf: vec![0; 64 * 1024],
            start: 0,
            end: 0,
        }
    }

    pub fn get_ref(&self) -> &R {
        &self.r
    }

    /// The next message without its delimiter; Ok(None) at end of stream (a partial last message
    /// is dropped). Empty messages are skipped.
    pub fn next_msg(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut msg: Vec<u8> = Vec::new();
        loop {
            if self.start < self.end {
                let chunk = &self.buf[self.start..self.end];
                if let Some(at) = chunk.iter().position(|b| *b == self.delim) {
                    if msg.len() + at > self.max {
                        return Err(too_long());
                    }
                    msg.extend_from_slice(&chunk[..at]);
                    self.start += at + 1;
                    if msg.is_empty() {
                        continue;
                    }
                    return Ok(Some(msg));
                }
                if msg.len() + chunk.len() > self.max {
                    return Err(too_long());
                }
                msg.extend_from_slice(chunk);
                self.start = self.end;
            }
            self.start = 0;
            self.end = match self.r.read(&mut self.buf) {
                Ok(0) => return Ok(None),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => 0,
                Err(e) => return Err(e),
            };
        }
    }
}

fn too_long() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "message longer than the limit")
}

/// A message ready to write to the browser: the JSON text plus the NUL.
pub fn to_pipe(json: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(json.len() + 1);
    v.extend_from_slice(json.as_bytes());
    v.push(0);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_nul_across_reads() {
        // a reader that returns one byte at a time exercises every boundary
        struct One<'a>(&'a [u8]);
        impl Read for One<'_> {
            fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                b[0] = self.0[0];
                self.0 = &self.0[1..];
                Ok(1)
            }
        }
        let mut f = Framer::new(One(b"{\"a\":1}\0\0{\"b\":2}\0tail"), 0, 100);
        assert_eq!(f.next_msg().unwrap().unwrap(), b"{\"a\":1}");
        assert_eq!(f.next_msg().unwrap().unwrap(), b"{\"b\":2}");
        assert!(f.next_msg().unwrap().is_none());
    }

    #[test]
    fn a_long_message_is_an_error_not_a_buffer() {
        let big = vec![b'x'; 1000];
        let mut f = Framer::new(&big[..], b'\n', 100);
        assert!(f.next_msg().is_err());
        let mut ok = Framer::new(&b"abc\n"[..], b'\n', 3);
        assert_eq!(ok.next_msg().unwrap().unwrap(), b"abc");
        let mut over = Framer::new(&b"abcd\n"[..], b'\n', 3);
        assert!(over.next_msg().is_err());
    }

    #[test]
    fn pipe_frames_with_nul() {
        assert_eq!(to_pipe("{}"), b"{}\0");
    }
}
