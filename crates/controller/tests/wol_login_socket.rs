// SPDX-License-Identifier: GPL-2.0-or-later
//! wol-login when the login window cannot start (QA-SHADOW-15, QA-SHADOW-38): its socket cannot
//! be bound or its program cannot be executed. The real failures (a bind in a missing directory,
//! an exec of a file without the execute bit) feed `greeter_start_failed`; a fake console VT
//! records what reaches the screen.
use controller::login::{
    GreeterRespawn, NOTICE_FILE, SeatSession, VT_UNLOCKSWITCH, VtStep, early_end_sentence,
    failed_early, greeter_start_failed, parse_df_avail, seat0_user_session,
};
use std::cell::RefCell;
use std::os::unix::fs::PermissionsExt;
use std::rc::Rc;

/// An ordered log of every ioctl and write on the fake console.
#[derive(Default, Clone)]
struct Vt(Rc<RefCell<Vec<String>>>);

impl std::io::Write for Vt {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0
            .borrow_mut()
            .push(format!("write {}", String::from_utf8_lossy(b)));
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn on_screen(respawn: &mut GreeterRespawn, why: &str) -> (std::time::Duration, Vec<String>) {
    let vt = Vt::default();
    let log = vt.0.clone();
    let ioctl = |d: &Vt, req: libc::c_ulong| {
        d.0.borrow_mut().push(format!("ioctl {req:#x}"));
        Ok(())
    };
    let (next, step) = greeter_start_failed(respawn, why, true, || Ok(vt), ioctl);
    assert_eq!(step, VtStep::Done);
    assert!(next.notice, "the notice from the first failure");
    let log = log.borrow().clone();
    (next.delay, log)
}

#[test]
fn greeter_spawn_err_writes_notice_unlocks_vt() {
    let dir = std::env::temp_dir().join(format!("wol-login-socket-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // The program exists but cannot be executed (chmod -x): the greeter's spawn fails for real.
    let greeter = dir.join("wol-greeter-session");
    std::fs::write(&greeter, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&greeter, std::fs::Permissions::from_mode(0o644)).unwrap();
    let spawn_err = std::process::Command::new(&greeter).spawn().unwrap_err();

    // The socket's directory is missing: the bind fails for real.
    let sock = dir.join("missing").join("wol-login.sock");
    let bind_err = std::os::unix::net::UnixListener::bind(&sock).unwrap_err();

    let mut respawn = GreeterRespawn::default();
    let mut delays = vec![];
    for why in [
        format!("{}: {spawn_err}", greeter.display()),
        format!("{}: {bind_err}", sock.display()),
        format!("{}: {spawn_err}", greeter.display()),
    ] {
        let (delay, log) = on_screen(&mut respawn, &why);
        delays.push(delay.as_secs());
        // VT switching is unlocked first, then the notice names the real error and the way out.
        assert_eq!(log[0], format!("ioctl {VT_UNLOCKSWITCH:#x}"), "{log:?}");
        assert_eq!(log.len(), 2, "{log:?}");
        assert!(log[1].contains(&format!("Reason: {why}")), "{log:?}");
        assert!(log[1].contains("Ctrl+Alt+Delete"), "{log:?}");
        assert!(log[1].contains("journalctl -b -u wold"), "{log:?}");
    }
    // 1, 2, 4 s: retried with back-off, never a fixed 5 s forever.
    assert_eq!(delays, [1, 2, 4]);
    std::fs::remove_dir_all(&dir).ok();
}

/// The same refusal on a machine with no console VT at all (a container): quiet, nothing drawn.
#[test]
fn no_vt_is_not_an_error_for_the_notice() {
    let mut respawn = GreeterRespawn::default();
    let (next, step) = greeter_start_failed(
        &mut respawn,
        "x",
        false,
        || -> std::io::Result<Vt> { Err(std::io::Error::from_raw_os_error(libc::ENOENT)) },
        |_, _| Ok(()),
    );
    assert_eq!(step, VtStep::NoVt);
    assert!(next.notice);
}

/// QA-LIVE-37: after `systemctl kill wold` the new wol-login finds the person's session on seat0
/// (loginctl) and waits for its leader instead of starting a login window the seat refuses.
#[test]
fn adopts_existing_seat_session() {
    let list = "c1 911 _wollogin seat0 tty1\n2 1000 ana seat0 tty1\n";
    let show = |id: &str| match id {
        "2" => "Class=user\nType=wayland\nLeader=55757\nState=active\n".to_owned(),
        _ => "Class=greeter\nType=wayland\nLeader=300\nState=active\n".to_owned(),
    };
    let s = seat0_user_session(list, 911, show).expect("the session is found");
    assert_eq!(
        s,
        SeatSession {
            id: "2".into(),
            user: "ana".into(),
            uid: 1000,
            leader: 55757
        }
    );
    // Only the login window on the seat: nothing to wait for, the window starts as before.
    assert_eq!(
        seat0_user_session("c1 911 _wollogin seat0 tty1\n", 911, show),
        None
    );
}

/// QA-LIVE-33: a session that ended within seconds is a failed start, whatever its exit status,
/// and the login window is told in a sentence (not a silent return).
#[test]
fn failed_early_shows_notice() {
    assert!(failed_early(std::time::Duration::from_millis(600)));
    assert!(!failed_early(std::time::Duration::from_secs(31)));
    let unwritable = early_end_sentence(Some(10 << 30), Some(false));
    assert!(
        unwritable.starts_with("Your session could not start"),
        "{unwritable}"
    );
    assert!(unwritable.contains("settings folder"), "{unwritable}");
    assert_eq!(NOTICE_FILE, "/run/wol-login-notice");
}

/// QA-DATA-3: on a full disk the sentence says so (the `df` figure of the home's disk decides).
#[test]
fn failed_early_names_full_disk() {
    let free = parse_df_avail("      Avail\n          0\n");
    assert_eq!(free, Some(0));
    assert!(early_end_sentence(free, Some(true)).contains("disk is full"));
    let roomy = parse_df_avail("      Avail\n 52428800000\n");
    assert!(!early_end_sentence(roomy, Some(true)).contains("disk is full"));
}

/// QA-SHADOW-39: the reply wol-login sends for a password that is not text is the one the login
/// window (shell login.rs from_reply) maps to its sentence (shell copy::LOGIN_TEXT_PASSWORD).
#[test]
fn text_password_reply_is_the_shared_word() {
    assert_eq!(
        controller::login::TEXT_PASSWORD_REPLY,
        shared::login::REPLY_TEXT_PASSWORD
    );
}
