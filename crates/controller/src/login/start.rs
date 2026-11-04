// SPDX-License-Identifier: GPL-2.0-or-later
//! What wol-login does when it cannot start the login window (QA-SHADOW-15/38) and when a session
//! is already on the seat (QA L2-39/39b).
use super::{GreeterRespawn, Respawn, VtStep, console_failure_notice, console_step};

/// The greeter could not be started because of `why`: step the backoff, draw the console notice
/// with VT switching unlocked (nothing owns the screen; the person can see what is wrong and the
/// Ctrl+Alt+Delete hint works), and return the wait before the next try with what the console
/// step did. `open` and `ioctl` are the device layer, as for [`console_step`].
pub fn greeter_start_failed<D: std::io::Write>(
    respawn: &mut GreeterRespawn,
    why: &str,
    dri_present: bool,
    open: impl FnOnce() -> std::io::Result<D>,
    ioctl: impl FnOnce(&D, libc::c_ulong) -> std::io::Result<()>,
) -> (Respawn, VtStep) {
    let next = respawn.start_failed();
    let text = console_failure_notice(dri_present, why, next.delay);
    let step = console_step(open, ioctl, false, &text);
    (next, step)
}

/// Where wol-login leaves the sentence for the login window when a person's session ended within
/// seconds of starting (QA-LIVE-33, QA-DATA-3: it came straight back to the login window without
/// a word). World readable, replaced atomically, removed when someone signs in and stays.
pub const NOTICE_FILE: &str = "/run/wol-login-notice";

/// Below this much free space a session cannot write its first files (`hyprland.lua`, the links,
/// `hypr-vm.lua`).
pub const DISK_FULL_BELOW: u64 = 1 << 20;

/// What `df -B1 --output=avail PATH` printed (a header line and the number).
pub fn parse_df_avail(out: &str) -> Option<u64> {
    out.lines().filter_map(|l| l.trim().parse().ok()).next()
}

/// The sentence for a session that ended too soon: `free` is the free space on the home's disk,
/// `config_writable` whether the account can write its `~/.config` (None: not known).
pub fn early_end_sentence(free: Option<u64>, config_writable: Option<bool>) -> &'static str {
    if free.is_some_and(|f| f < DISK_FULL_BELOW) {
        "Your session could not start because the disk is full. Free some space, then sign in again."
    } else if config_writable == Some(false) {
        "Your session could not start because your settings folder cannot be written to. Ask the person who looks after this computer."
    } else {
        "Your session could not start. Try again; if it keeps happening, restart the computer."
    }
}

/// The person (or logind, for `loginctl terminate-session`) asked the session to end: it left
/// with status 0, or by SIGTERM, SIGHUP or SIGINT. A person who logs out within seconds of
/// signing in is not told their session "could not start".
pub fn ended_on_request(code: Option<i32>, signal: Option<i32>) -> bool {
    code == Some(0) || matches!(signal, Some(libc::SIGTERM | libc::SIGHUP | libc::SIGINT))
}

/// A user session of ours already running on the seat: wol-login (or wold) was restarted under
/// it. Starting the login window then makes the seat daemon refuse the second compositor ("seat
/// is VT-bound"), the window aborts, and the back-off hides the real login window for up to five
/// minutes after the next Log Out (QA L2-39/39b).
#[derive(Debug, PartialEq, Eq)]
pub struct SeatSession {
    pub id: String,
    pub user: String,
    pub uid: u32,
    /// The session leader (`wol-session-pam`), whose end is the end of the session.
    pub leader: i32,
}

/// `loginctl list-sessions --no-legend` lines (`SESSION UID USER SEAT [TTY]`) on `seat0`, and for
/// each `show` (the `loginctl show-session` properties `Class`, `Type`, `Leader`, `State`): the
/// first session of class `user`, type `wayland`, state `active`/`online`/`opening`, whose user is
/// not the login window's own. None when there is none.
pub fn seat0_user_session(
    list: &str,
    ui_uid: u32,
    show: impl Fn(&str) -> String,
) -> Option<SeatSession> {
    list.lines().find_map(|l| {
        let mut f = l.split_whitespace();
        let (id, uid, user, seat) = (f.next()?, f.next()?, f.next()?, f.next()?);
        let uid: u32 = uid.parse().ok()?;
        if seat != "seat0" || uid == ui_uid {
            return None;
        }
        let text = show(id);
        let prop = |k: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(k)?.strip_prefix('='))
                .map(str::trim)
        };
        let live = matches!(prop("State")?, "active" | "online" | "opening");
        if prop("Class")? != "user" || prop("Type")? != "wayland" || !live {
            return None;
        }
        Some(SeatSession {
            id: id.into(),
            user: user.into(),
            uid,
            leader: prop("Leader")?.parse().ok().filter(|p| *p > 1)?,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(id: &str) -> String {
        match id {
            "c1" => "Class=greeter\nType=wayland\nLeader=300\nState=active\n",
            "4" => "Class=user\nType=wayland\nLeader=55757\nState=active\n",
            "5" => "Class=user\nType=tty\nLeader=900\nState=online\n",
            "6" => "Class=manager\nType=unspecified\nLeader=901\nState=active\n",
            "7" => "Class=user\nType=wayland\nLeader=902\nState=closing\n",
            _ => "",
        }
        .to_owned()
    }

    /// QA L2-39: a Wayland user session on seat0 is found, so wol-login waits for it instead of
    /// starting a login window on the busy seat.
    #[test]
    fn adopts_the_running_session_on_seat0() {
        let list = "c1 911 _wollogin seat0 tty1\n4 1000 ana seat0 tty1\n5 1000 ana seat0 -\n";
        assert_eq!(
            seat0_user_session(list, 911, show),
            Some(SeatSession {
                id: "4".into(),
                user: "ana".into(),
                uid: 1000,
                leader: 55757
            })
        );
        // The keyring's `su -l` session (no seat), a manager, a closing session, the greeter and
        // an empty answer are not a session to wait for.
        for list in [
            "5 1000 ana - -\n",
            "6 1000 ana seat0 -\n",
            "7 1000 ana seat0 tty1\n",
            "c1 911 _wollogin seat0 tty1\n",
            "8 1000 ana seat1 -\n",
            "",
            "garbage",
        ] {
            assert_eq!(seat0_user_session(list, 911, show), None, "{list:?}");
        }
    }

    /// QA-LIVE-33 / QA-DATA-3: the login window is told why the session ended at once.
    #[test]
    fn early_end_sentences_name_the_cause() {
        assert!(early_end_sentence(Some(0), Some(true)).contains("disk is full"));
        assert!(early_end_sentence(Some(DISK_FULL_BELOW - 1), None).contains("disk is full"));
        assert!(early_end_sentence(Some(1 << 30), Some(false)).contains("settings folder"));
        // A full disk wins: nothing can be written there either.
        assert!(early_end_sentence(Some(0), Some(false)).contains("disk is full"));
        for unknown in [
            early_end_sentence(None, None),
            early_end_sentence(Some(1 << 30), Some(true)),
        ] {
            assert!(unknown.contains("Try again"), "{unknown}");
        }
        assert_eq!(parse_df_avail("     Avail\n 123456\n"), Some(123456));
        assert_eq!(parse_df_avail("garbage"), None);
    }

    /// A logout (status 0 or a TERM/HUP from logind) is a request; a crash is not.
    #[test]
    fn a_logout_is_not_a_failed_start() {
        assert!(ended_on_request(Some(0), None));
        assert!(ended_on_request(None, Some(libc::SIGTERM)));
        assert!(ended_on_request(None, Some(libc::SIGHUP)));
        assert!(!ended_on_request(Some(1), None));
        assert!(!ended_on_request(Some(101), None));
        assert!(!ended_on_request(None, Some(libc::SIGSEGV)));
        assert!(!ended_on_request(None, Some(libc::SIGKILL)));
        assert!(!ended_on_request(None, None));
    }
}
