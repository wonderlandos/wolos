// SPDX-License-Identifier: GPL-2.0-or-later
//! The administrator password on a terminal (`wol install`, `wol remove` and every verb that asks
//! wol-admind): "Password: " on stderr, the terminal's echo off while it is typed, a newline after.
//! [`NoEcho`] puts the terminal back when it is dropped: after the line, on an error, on a panic,
//! and on Ctrl-C (a SIGINT handler restores it before the process ends). Input that is not a
//! terminal (a pipe, a script) is read as one line and the terminal settings are never touched.
use std::io::{BufRead, Read, Write};
use std::sync::OnceLock;

/// The settings the terminal had before echo went off, for the SIGINT handler (it cannot reach the
/// guard). Every prompt of one process starts from the same settings, so the first is kept.
static SAVED: OnceLock<(i32, libc::termios)> = OnceLock::new();

/// Bytes of one password line read at most (a pasted file is not a password).
const MAX_PASSWORD: u64 = 4096;

/// `fd`'s terminal settings; None when `fd` is not a terminal.
fn get_attr(fd: i32) -> Option<libc::termios> {
    let mut t = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: tcgetattr writes a whole termios into `t` and returns 0, or fails (ENOTTY, EBADF)
    // and `t` is never read.
    unsafe { (libc::tcgetattr(fd, t.as_mut_ptr()) == 0).then(|| t.assume_init()) }
}

/// Applies `t` to `fd` at once; async-signal-safe (the SIGINT handler calls it).
fn set_attr(fd: i32, t: &libc::termios) -> bool {
    // SAFETY: `t` is a valid termios read by tcgetattr (and possibly changed in plain fields).
    unsafe { libc::tcsetattr(fd, libc::TCSANOW, t) == 0 }
}

/// Ctrl-C while echo is off: the terminal gets its settings back, then the process ends as an
/// interrupted one (130), like before the prompt existed.
extern "C" fn on_interrupt(_: libc::c_int) {
    if let Some((fd, t)) = SAVED.get() {
        set_attr(*fd, t);
    }
    // SAFETY: _exit is async-signal-safe and does not return; no Rust state is used after it.
    unsafe { libc::_exit(130) }
}

/// SIGINT goes to [`on_interrupt`] (`back` None) or back to the action `back`; the action it
/// replaced, None when sigaction failed.
fn swap_sigint(back: Option<&libc::sigaction>) -> Option<libc::sigaction> {
    // SAFETY: both structs are zeroed C structs filled before use; sigemptyset fills the mask;
    // sigaction reads `new` and writes `old` only.
    unsafe {
        let new = match back {
            Some(b) => *b,
            None => {
                let mut n: libc::sigaction = std::mem::zeroed();
                n.sa_sigaction = on_interrupt as extern "C" fn(libc::c_int) as libc::sighandler_t;
                libc::sigemptyset(&mut n.sa_mask);
                n
            }
        };
        let mut old: libc::sigaction = std::mem::zeroed();
        (libc::sigaction(libc::SIGINT, &new, &mut old) == 0).then_some(old)
    }
}

/// Echo off on the terminal `fd` for as long as this lives.
pub struct NoEcho {
    fd: i32,
    saved: libc::termios,
    old_sigint: Option<libc::sigaction>,
}

impl NoEcho {
    /// None when `fd` is not a terminal: then nothing was changed.
    pub fn new(fd: i32) -> Option<Self> {
        let saved = get_attr(fd)?;
        let _ = SAVED.set((fd, saved));
        let old_sigint = swap_sigint(None);
        let mut quiet = saved;
        quiet.c_lflag &= !libc::ECHO;
        set_attr(fd, &quiet);
        Some(Self {
            fd,
            saved,
            old_sigint,
        })
    }
}

// Drop releases the terminal: its settings from before the prompt, and the old SIGINT action.
impl Drop for NoEcho {
    fn drop(&mut self) {
        set_attr(self.fd, &self.saved);
        if let Some(old) = &self.old_sigint {
            swap_sigint(Some(old));
        }
    }
}

/// One password line from `input` without its line end; None when it could not be read. On the
/// terminal `fd`: "Password: " on `prompt` first, no echo while typing, a newline after. Not a
/// terminal: just the line, nothing written, the settings untouched.
pub fn read_password<R: BufRead>(input: &mut R, prompt: &mut dyn Write, fd: i32) -> Option<String> {
    let quiet = get_attr(fd).is_some().then(|| {
        let _ = prompt.write_all(b"Password: ");
        let _ = prompt.flush();
        NoEcho::new(fd)
    });
    let mut line = String::new();
    let read = input.by_ref().take(MAX_PASSWORD).read_line(&mut line);
    if let Some(guard) = quiet {
        drop(guard);
        let _ = prompt.write_all(b"\n");
        let _ = prompt.flush();
    }
    read.ok()?;
    Some(line.trim_end_matches(['\n', '\r']).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    /// A script or a pipe feeds the password: read as one line, no prompt, termios untouched.
    #[test]
    fn piped_input_is_one_line_and_no_prompt() {
        let devnull = std::fs::File::open("/dev/null").unwrap();
        let fd = devnull.as_raw_fd();
        assert!(get_attr(fd).is_none(), "/dev/null is no terminal");
        assert!(NoEcho::new(fd).is_none(), "nothing to turn off");
        let mut said = Vec::new();
        let mut input = std::io::Cursor::new(b"s3cret pass\r\nnext\n".to_vec());
        assert_eq!(
            read_password(&mut input, &mut said, fd).as_deref(),
            Some("s3cret pass")
        );
        assert!(said.is_empty(), "no prompt for a pipe: {said:?}");
        assert_eq!(
            read_password(&mut input, &mut said, fd).as_deref(),
            Some("next")
        );
        // End of input: an empty line, as before (the caller then gives up).
        assert_eq!(
            read_password(&mut input, &mut said, fd).as_deref(),
            Some("")
        );
        assert!(
            SAVED.get().is_none(),
            "the terminal settings were never read"
        );
    }
}
