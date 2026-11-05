// SPDX-License-Identifier: GPL-2.0-or-later
//! The console VT wol-login draws on: blank before each compositor, VT switching locked, and the
//! notice when the login window cannot start (ADR 0012).

/// The console VT. The login window and every session run on it: the seat daemon takes the VT
/// that is active at boot (tty1; no getty there or on any other VT, logind.conf.d/wolos.conf), and
/// the notice is drawn on it when the login window cannot start.
pub const CONSOLE: &str = "/dev/tty1";

/// Written to [`CONSOLE`] before each compositor starts (ADR 0012): when a compositor exits, the
/// seat daemon puts the VT back in text mode and the kernel redraws its text, so it must be plain
/// black, with no text and no cursor. Black on black too: later kernel console text stays
/// invisible. The kernel command line (`quiet`, `vt.global_cursor_default=0`) keeps the VT so from
/// the start of boot.
pub const BLANK_CONSOLE: &str = "\x1b[30;40m\x1b[?25l\x1b[H\x1b[2J";
pub const SHOW_CURSOR: &str = "\x1b[?25h";
/// Default colours again (the console notice must be readable).
pub const TEXT_VISIBLE: &str = "\x1b[0m";

/// Linux `VT_LOCKSWITCH` (`linux/vt.h`, `0x560B`): no argument, needs `CAP_SYS_TTY_CONFIG`, works
/// on any VT descriptor. The lock is global: while it is set the kernel ignores every VT switch,
/// Alt+Fn on a text VT and the seat daemon's `VT_ACTIVATE` for a compositor's Ctrl+Alt+Fn alike,
/// so nothing but WolOS is ever on screen (ADR 0012). Graphics mode (`KDSETMODE`), keyboard mode
/// and `VT_SETMODE` are the seat daemon's job when a compositor opens the seat; wol-login draws
/// nothing on the VT but escape sequences, so it never sets them.
pub const VT_LOCKSWITCH: libc::c_ulong = 0x560B;
/// Linux `VT_UNLOCKSWITCH` (`linux/vt.h`, `0x560C`): VT switching works again.
pub const VT_UNLOCKSWITCH: libc::c_ulong = 0x560C;

/// The ioctl that locks (true) or unlocks (false) VT switching.
pub fn vt_switch_request(lock: bool) -> libc::c_ulong {
    if lock { VT_LOCKSWITCH } else { VT_UNLOCKSWITCH }
}

/// Errors that mean there is no VT to manage: no such device (kernel without VTs, a container) or
/// a descriptor that is no VT (`ENOTTY`: a pty or a file, e.g. under a nested compositor).
pub fn no_vt(e: &std::io::Error) -> bool {
    matches!(
        e.raw_os_error(),
        Some(libc::ENOENT | libc::ENXIO | libc::ENODEV | libc::ENOTTY)
    )
}

/// What [`console_step`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum VtStep {
    /// Switching set and the text written.
    Done,
    /// No VT here: nothing to do, not an error.
    NoVt,
    /// Something to log (the reasons, `; ` between them).
    Failed(String),
}

/// One console step (ADR 0012): open the console VT, lock (before a compositor) or unlock (for the
/// notice) VT switching, then write `text`. `open` and `ioctl` are the device layer, so tests run
/// it on fakes. No VT at all is [`VtStep::NoVt`] and nothing is written; a failed ioctl (e.g.
/// `EPERM` without `CAP_SYS_TTY_CONFIG`) still writes the text, the screen matters more.
pub fn console_step<D: std::io::Write>(
    open: impl FnOnce() -> std::io::Result<D>,
    ioctl: impl FnOnce(&D, libc::c_ulong) -> std::io::Result<()>,
    lock: bool,
    text: &str,
) -> VtStep {
    let mut tty = match open() {
        Ok(t) => t,
        Err(e) if no_vt(&e) => return VtStep::NoVt,
        Err(e) => return VtStep::Failed(format!("open: {e}")),
    };
    let mut failed = Vec::new();
    let name = if lock {
        "VT_LOCKSWITCH"
    } else {
        "VT_UNLOCKSWITCH"
    };
    match ioctl(&tty, vt_switch_request(lock)) {
        Ok(()) => {}
        Err(e) if e.raw_os_error() == Some(libc::ENOTTY) => return VtStep::NoVt,
        Err(e) => failed.push(format!("{name}: {e}")),
    }
    if let Err(e) = tty.write_all(text.as_bytes()).and_then(|()| tty.flush()) {
        failed.push(format!("write: {e}"));
    }
    if failed.is_empty() {
        VtStep::Done
    } else {
        VtStep::Failed(failed.join("; "))
    }
}

/// Full-screen text for the console when the greeter keeps failing: what is wrong, the raw reason
/// from the greeter exit, when the next try is, where the details are, and how to restart (no VT
/// has a getty, so there is no console login to point to). wol-login draws it after every exit:
/// the next try blanks the VT (ADR 0012), so a notice drawn once was gone after one second.
pub fn console_notice(dri_present: bool, status: &str, retry_in: std::time::Duration) -> String {
    console_failure_notice(
        dri_present,
        &format!("login window exited, {status}"),
        retry_in,
    )
}

/// [`console_notice`] for any reason: "login window exited, exit status: 1", or why it never
/// started ("/usr/bin/wol-greeter-session: Permission denied (os error 13)").
pub fn console_failure_notice(
    dri_present: bool,
    reason: &str,
    retry_in: std::time::Duration,
) -> String {
    let headline = if dri_present {
        "The WolOS login window could not start."
    } else {
        "WolOS needs a supported graphics card (no /dev/dri: no GPU driver loaded)."
    };
    let retry = retry_words(retry_in);
    format!(
        "{TEXT_VISIBLE}{SHOW_CURSOR}\x1b[2J\x1b[H\r\n  WolOS\r\n\r\n  {headline}\r\n\r\n  Reason: {reason}.\r\n  Trying again in {retry}.\r\n  \
            Details: journalctl -b -u wold\r\n\r\n  Restart: press Ctrl+Alt+Delete, or hold the power button.\r\n"
    )
}

/// A wait in the words of the console notice: whole seconds under a minute, else whole minutes
/// (the backoff waits are 1 s .. 5 min).
pub fn retry_words(d: std::time::Duration) -> String {
    let s = d.as_secs().max(1);
    let (n, unit) = if s < 60 {
        (s, "second")
    } else {
        ((s + 30) / 60, "minute")
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::login::GreeterRespawn;

    /// Fake console VT: every ioctl and write lands in one ordered log; `write_err` makes writes
    /// fail with that errno ([`step_on_fake`] fakes open and ioctl errors).
    #[derive(Default)]
    struct FakeVt {
        log: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
        write_err: Option<i32>,
    }

    impl std::io::Write for FakeVt {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if let Some(e) = self.write_err {
                return Err(std::io::Error::from_raw_os_error(e));
            }
            let t = String::from_utf8_lossy(b).into_owned();
            self.log.borrow_mut().push(format!("write {t:?}"));
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Runs [`console_step`] on a [`FakeVt`]: (result, log).
    fn step_on_fake(
        open_err: Option<i32>,
        ioctl_err: Option<i32>,
        write_err: Option<i32>,
        lock: bool,
        text: &str,
    ) -> (VtStep, Vec<String>) {
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let vt = FakeVt {
            log: log.clone(),
            write_err,
        };
        let open = || match open_err {
            Some(e) => Err(std::io::Error::from_raw_os_error(e)),
            None => Ok(vt),
        };
        let ioctl = |d: &FakeVt, req: libc::c_ulong| {
            d.log.borrow_mut().push(format!("ioctl {req:#x}"));
            ioctl_err.map_or(Ok(()), |e| Err(std::io::Error::from_raw_os_error(e)))
        };
        let got = console_step(open, ioctl, lock, text);
        let log = log.borrow().clone();
        (got, log)
    }

    #[test]
    fn console_notice_names_the_reason() {
        let cases = [
            (
                false,
                "signal: 6 (SIGABRT)",
                "WolOS needs a supported graphics card",
            ),
            (
                true,
                "exit status: 1",
                "The WolOS login window could not start",
            ),
        ];
        for (dri, status, headline) in cases {
            let m = console_notice(dri, status, std::time::Duration::from_secs(4));
            assert!(m.contains("Trying again in 4 seconds."), "{m}");
            assert!(m.contains(headline), "{m}");
            assert!(m.contains(status), "{m}");
            // No VT has a getty (logind.conf.d/wolos.conf): no console login to point to, and
            // wold logs to the journal, not to a file.
            assert!(!m.contains("Alt+F"), "no console login exists: {m}");
            assert!(!m.contains("/var/log/"), "{m}");
            assert!(m.contains("journalctl -b -u wold"), "{m}");
            assert!(m.contains("Ctrl+Alt+Delete"), "how to restart: {m}");
            assert!(!m.contains("FreeBSD"), "{m}");
            assert!(
                m.starts_with(&format!("{TEXT_VISIBLE}{SHOW_CURSOR}")),
                "notice is readable over black-on-black boot text, with a cursor: {m:?}"
            );
        }
    }

    /// The notice says when the next try is, for every wait of the backoff.
    #[test]
    fn console_notice_says_when_it_tries_again() {
        let mut r = GreeterRespawn::default();
        let waits: Vec<String> = (0..10)
            .map(|_| retry_words(r.exited(std::time::Duration::ZERO).delay))
            .collect();
        assert_eq!(
            waits,
            [
                "1 second",
                "2 seconds",
                "4 seconds",
                "8 seconds",
                "16 seconds",
                "32 seconds",
                "1 minute",
                "2 minutes",
                "4 minutes",
                "5 minutes"
            ]
        );
    }

    /// ADR 0012: between two compositors the kernel redraws the console VT's text; it must be black
    /// with no cursor, and any later kernel console text black on black.
    #[test]
    fn console_blank_clears_and_hides_the_cursor() {
        assert_eq!(BLANK_CONSOLE, "\x1b[30;40m\x1b[?25l\x1b[H\x1b[2J");
        assert_eq!(SHOW_CURSOR, "\x1b[?25h");
        assert_eq!(TEXT_VISIBLE, "\x1b[0m");
    }

    /// linux/vt.h: `#define VT_LOCKSWITCH 0x560B`, `#define VT_UNLOCKSWITCH 0x560C` ('V' << 8 | n).
    /// The old base's `_IOW('v', 9, int)` (0x80047609) is no VT ioctl on Linux.
    #[test]
    fn vt_lock_requests_match_linux_vt_h() {
        let v = |n: libc::c_ulong| libc::c_ulong::from(b'V') << 8 | n;
        assert_eq!((VT_LOCKSWITCH, VT_UNLOCKSWITCH), (v(0x0B), v(0x0C)));
        assert_eq!(vt_switch_request(true), 0x560B);
        assert_eq!(vt_switch_request(false), 0x560C);
        assert_ne!(VT_LOCKSWITCH, 0x8004_7609);
    }

    /// ADR 0012: before a compositor, switching is locked and the VT blanked; for the notice it is
    /// unlocked first, then the notice drawn.
    #[test]
    fn console_step_locks_before_a_compositor_and_unlocks_for_the_notice() {
        let (got, log) = step_on_fake(None, None, None, true, BLANK_CONSOLE);
        assert_eq!(got, VtStep::Done);
        assert_eq!(
            log,
            [
                "ioctl 0x560b".to_owned(),
                format!("write {BLANK_CONSOLE:?}")
            ]
        );
        let notice = console_notice(true, "exit status: 1", std::time::Duration::from_secs(1));
        let (got, log) = step_on_fake(None, None, None, false, &notice);
        assert_eq!(got, VtStep::Done);
        assert_eq!(
            log,
            ["ioctl 0x560c".to_owned(), format!("write {notice:?}")]
        );
    }

    /// No VT at all (kernel without VTs, a container, a nested compositor's pty): a quiet no-op.
    #[test]
    fn console_step_without_a_vt_is_a_no_op() {
        for e in [libc::ENOENT, libc::ENXIO, libc::ENODEV, libc::ENOTTY] {
            for lock in [true, false] {
                let (got, log) = step_on_fake(Some(e), None, None, lock, BLANK_CONSOLE);
                assert_eq!(got, VtStep::NoVt, "open errno {e}");
                assert!(log.is_empty(), "nothing touched: {log:?}");
            }
        }
        // The device opens but is no VT: nothing written to it either.
        let (got, log) = step_on_fake(None, Some(libc::ENOTTY), None, true, BLANK_CONSOLE);
        assert_eq!(got, VtStep::NoVt);
        assert_eq!(log, ["ioctl 0x560b"]);
    }

    /// Real failures are reported; the text still reaches the screen when only the ioctl failed.
    #[test]
    fn console_step_reports_real_failures() {
        let (got, log) = step_on_fake(None, Some(libc::EPERM), None, true, BLANK_CONSOLE);
        let VtStep::Failed(why) = got else {
            panic!("EPERM is a failure: {got:?}")
        };
        assert!(why.starts_with("VT_LOCKSWITCH: "), "{why}");
        assert_eq!(log.len(), 2, "blank written anyway: {log:?}");
        let (got, _) = step_on_fake(None, Some(libc::EPERM), None, false, "x");
        assert!(
            matches!(&got, VtStep::Failed(w) if w.starts_with("VT_UNLOCKSWITCH: ")),
            "{got:?}"
        );
        let (got, log) = step_on_fake(Some(libc::EACCES), None, None, true, BLANK_CONSOLE);
        assert!(
            matches!(&got, VtStep::Failed(w) if w.starts_with("open: ")),
            "{got:?}"
        );
        assert!(log.is_empty());
        let (got, log) = step_on_fake(None, None, Some(libc::EIO), true, BLANK_CONSOLE);
        assert!(
            matches!(&got, VtStep::Failed(w) if w.starts_with("write: ")),
            "{got:?}"
        );
        assert_eq!(log, ["ioctl 0x560b"], "locked even when the write fails");
    }
}
