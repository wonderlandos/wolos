// SPDX-License-Identifier: GPL-2.0-or-later
//! Asking wold: the portal calls of libwol 1.15 (`crates/wol-abi/src/client.rs` sends the same
//! lines). The bridge is the app wold sees; wold decides from the person's grants for it.
use shared::portal::{Answer, Call, Code};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

/// How long a call may wait for the person: wold's own panel wait plus a margin (libwol's
/// `PANEL_WAIT`).
pub const WAIT: Duration = Duration::from_secs(610);
/// Writes never wait long.
const WRITE_WAIT: Duration = Duration::from_secs(5);
/// Longest answer line read.
const MAX_ANSWER: u64 = 64 * 1024;

/// The text wold answers to a call it does not know (`Call::decode` in `shared::portal`): a wold
/// older than libwol 1.15 answers this to `file.trash`, `share.file` and `open.with`.
pub const UNKNOWN_CALL: &str = "The request is not a known call.";

/// The text when wold cannot be reached at all.
pub const NOT_REACHED: &str = "The WolOS system service is not running.";

/// Something that answers portal calls: wold's socket, or a fake in tests.
pub trait Wold: Send + Sync {
    /// Ask wold; blocks while the person chooses.
    fn call(&self, call: &Call) -> Answer;
}

/// wold's socket (`$WOL_SOCKET`, as libwol reads it, or `/var/run/wold.sock`).
pub struct Socket {
    pub path: PathBuf,
}

impl Socket {
    /// wold's socket at `path`.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The socket libwol would use: `socket` (the value of `$WOL_SOCKET`, which the binary reads)
    /// or the default.
    pub fn from_setting(socket: Option<std::ffi::OsString>) -> Self {
        Self::at(socket.map_or_else(|| shared::jobs::SOCKET.into(), PathBuf::from))
    }
}

fn not_reached() -> Answer {
    Answer::err(Code::Unavailable, NOT_REACHED)
}

impl Wold for Socket {
    fn call(&self, call: &Call) -> Answer {
        if let Err(m) = call.validate() {
            return Answer::err(Code::InvalidArgument, m);
        }
        let Ok(mut s) = shared::jobs::connect(&self.path) else {
            return not_reached();
        };
        let _ = s.set_read_timeout(Some(WAIT));
        let _ = s.set_write_timeout(Some(WRITE_WAIT));
        if s.write_all(format!("call {}\n", call.encode()).as_bytes())
            .is_err()
        {
            return not_reached();
        }
        let mut line = Vec::new();
        match BufReader::new(s)
            .take(MAX_ANSWER)
            .read_until(b'\n', &mut line)
        {
            Ok(n) if n > 0 => Answer::parse(&String::from_utf8_lossy(&line)),
            _ => not_reached(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text the bridge keys the fallback on is the one wold's decoder writes.
    #[test]
    fn unknown_call_is_what_wold_answers() {
        let e = Call::decode(br#"{"call":"file.shred","path":"/a"}"#).unwrap_err();
        assert_eq!(e, UNKNOWN_CALL);
    }

    #[test]
    fn no_wold_is_unavailable() {
        let s = Socket::at("/nonexistent/wold.sock");
        let a = s.call(&Call::FileTrash { path: "/a".into() });
        assert_eq!(a.code, Some(Code::Unavailable));
    }
}
