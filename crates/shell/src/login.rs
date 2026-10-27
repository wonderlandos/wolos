//! Auth wire formats. Lock: `/usr/local/libexec/wol-authcheck`, password on stdin, exit 0 ok / 1 denied / 2 error (ADR 0023).
//! Login window: `/var/run/wol-login.sock`, `user\n` + password, write side closed, reply `ok|denied|error` (ADR 0024).
use std::io::{Read, Write};

pub const AUTHCHECK: &str = "/usr/local/libexec/wol-authcheck";
pub const LOGIN_SOCKET: &str = shared::login::SOCKET;
pub const MAX_PASSWORD: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    Denied,
    Error,
}

impl Verdict {
    pub fn message(self) -> &'static str {
        match self {
            Verdict::Ok => "",
            Verdict::Denied => "Incorrect password",
            Verdict::Error => "Your password can't be checked right now. Try again, or restart.",
        }
    }
}

pub fn from_exit(code: Option<i32>) -> Verdict {
    match code {
        Some(0) => Verdict::Ok,
        Some(1) => Verdict::Denied,
        _ => Verdict::Error,
    }
}

pub fn from_reply(reply: &str) -> Verdict {
    match reply.trim() {
        "ok" => Verdict::Ok,
        "denied" => Verdict::Denied,
        _ => Verdict::Error,
    }
}

/// None when the input can never be valid (too long, NUL, newline in user).
pub fn login_request(user: &str, password: &str) -> Option<Vec<u8>> {
    if user.is_empty()
        || user.contains(['\n', '\0'])
        || password.len() > MAX_PASSWORD
        || password.contains('\0')
    {
        return None;
    }
    let mut b = Vec::with_capacity(user.len() + 1 + password.len());
    b.extend_from_slice(user.as_bytes());
    b.push(b'\n');
    b.extend_from_slice(password.as_bytes());
    Some(b)
}

/// Blocking; run off the GTK thread.
pub fn authcheck(password: &str) -> Verdict {
    if password.len() > MAX_PASSWORD || password.contains('\0') {
        return Verdict::Denied;
    }
    let Ok(mut child) = std::process::Command::new(AUTHCHECK)
        .env_clear()
        .stdin(std::process::Stdio::piped())
        .spawn()
    else {
        return Verdict::Error;
    };
    if let Some(mut i) = child.stdin.take() {
        let _ = i.write_all(password.as_bytes());
    }
    from_exit(child.wait().ok().and_then(|s| s.code()))
}

/// Blocking; run off the GTK thread.
pub fn login(socket: &str, user: &str, password: &str) -> Verdict {
    let Some(req) = login_request(user, password) else {
        return Verdict::Denied;
    };
    exchange(socket, &req)
}

/// One request, write side closed, one-line reply. Blocking.
pub fn request(socket: &str, req: &[u8]) -> Verdict {
    exchange(socket, req)
}

fn exchange(socket: &str, req: &[u8]) -> Verdict {
    reply_to(socket, req).map_or(Verdict::Error, |r| from_reply(&r))
}

/// Whole reply to one request (write side closed first). Blocking.
fn reply_to(socket: &str, req: &[u8]) -> Option<String> {
    let mut s = std::os::unix::net::UnixStream::connect(socket).ok()?;
    s.write_all(req).ok()?;
    s.shutdown(std::net::Shutdown::Write).ok()?;
    let mut r = String::new();
    s.read_to_string(&mut r).ok()?;
    Some(r)
}

/// Setup's Wi-Fi list: wol-login scans and answers (ADR 0013). None: wol-login unreachable.
/// Blocking (a few seconds); run off the GTK thread.
pub fn wifi_scan(socket: &str) -> Option<Vec<shared::login::SeenNetwork>> {
    shared::login::decode_networks(&reply_to(socket, &shared::login::encode_wifi_scan())?)
}

/// Avatar initials: "Klee Lima" -> "KL", "root" -> "R".
pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

/// Login users: uid >= 1000, not nobody, shell listed in /etc/shells.
pub fn users(passwd: &str, shells: &str) -> Vec<String> {
    let ok: Vec<&str> = shells
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('/'))
        .collect();
    passwd
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            let uid: u32 = f.get(2)?.parse().ok()?;
            (uid >= 1000 && uid != 65534 && ok.contains(f.last()?)).then(|| f[0].to_string())
        })
        .collect()
}

/// Login users as (login name, name shown): the shown name is the full name when passwd has one.
pub fn login_users(passwd: &str, shells: &str) -> Vec<(String, String)> {
    users(passwd, shells)
        .into_iter()
        .map(|u| {
            let shown = display_name(&u, passwd);
            (u, shown)
        })
        .collect()
}

/// The account's full name (passwd GECOS, first field), else the login name.
pub fn display_name(user: &str, passwd: &str) -> String {
    passwd
        .lines()
        .map(|l| l.split(':').collect::<Vec<_>>())
        .find(|f| f.len() >= 5 && f[0] == user)
        .and_then(|f| {
            f[4].split(',')
                .next()
                .map(str::trim)
                .filter(|g| !g.is_empty())
                .map(String::from)
        })
        .unwrap_or_else(|| user.into())
}

pub use shared::login::Power;

/// Restart from the lock screen when passwords can't be checked (operator group, no root).
pub const RESTART_NOW: [&str; 3] = ["shutdown", "-r", "now"];

pub fn power_label(p: Power) -> &'static str {
    match p {
        Power::Sleep => "Sleep",
        Power::Restart => "Restart",
        Power::ShutDown => "Shut Down",
    }
}

pub fn power_icon(p: Power) -> &'static str {
    match p {
        Power::Sleep => "weather-clear-night-symbolic",
        Power::Restart => "view-refresh-symbolic",
        Power::ShutDown => "system-shutdown-symbolic",
    }
}

/// Blocking; run off the GTK thread. wol-login (root) runs it (ADR 0003).
pub fn power(socket: &str, p: Power) -> Verdict {
    exchange(socket, &shared::login::encode_power(p))
}

/// Wrong passwords before the login window shows the account's hint.
pub const HINT_AFTER: u32 = 3;

/// "Hint: …" once `failures` reaches [`HINT_AFTER`] and a non-blank hint exists.
pub fn hint_message(failures: u32, hint: Option<&str>) -> Option<String> {
    let h = hint?.trim();
    (failures >= HINT_AFTER && !h.is_empty()).then(|| format!("Hint: {h}"))
}

/// The hint file for `user` (written by wol-login at setup, ADR 0004).
pub fn read_hint(user: &str) -> Option<String> {
    if user.is_empty() || user.contains(['/', '\0']) || user.starts_with('.') {
        return None;
    }
    std::fs::read_to_string(std::path::Path::new(shared::login::HINTS_DIR).join(user)).ok()
}

/// Avatar colour chosen in Setup (ADR 0013): one allow-listed tone, else None (grey).
pub fn avatar_tone(text: &str) -> Option<&'static str> {
    let t = text.trim();
    shared::login::AVATAR_TONES
        .iter()
        .find(|a| **a == t)
        .copied()
}

/// The avatar file for `user` (`/var/db/wolos/avatars/<user>`, written by wol-login at setup).
pub fn read_avatar(user: &str) -> Option<&'static str> {
    if user.is_empty() || user.contains(['/', '\0']) || user.starts_with('.') {
        return None;
    }
    let p = std::path::Path::new(shared::login::AVATARS_DIR).join(user);
    avatar_tone(&std::fs::read_to_string(p).ok()?)
}

/// Owned-argument form for worker threads.
pub fn authcheck_owned(password: String) -> Verdict {
    authcheck(&password)
}

/// Printed by the lock screen on stdout right after a real unlock; nothing else ends the lock.
pub const UNLOCKED: &str = "wol-lock: unlocked";
/// Respawns before the supervisor gives up (the compositor still holds the session locked).
pub const MAX_LOCK_RESPAWNS: u32 = 30;

/// wol-lock supervisor (ADR 0012): what follows the end of a lock screen process.
#[derive(Debug, PartialEq, Eq)]
pub enum LockExit {
    Done,
    Respawn(std::time::Duration),
}

/// A lock screen that ended without [`UNLOCKED`] crashed: Hyprland keeps the session locked
/// (`misc:allow_session_lock_restore`) and a new one takes over, 100 ms .. 3 s apart.
pub fn lock_exit(unlocked: bool, compositor_alive: bool, attempt: u32) -> LockExit {
    if unlocked || !compositor_alive || attempt >= MAX_LOCK_RESPAWNS {
        return LockExit::Done;
    }
    let ms = (100u64 << attempt.min(5)).min(3000);
    LockExit::Respawn(std::time::Duration::from_millis(ms))
}

/// Set by the supervisor for the lock screen process it runs.
pub const LOCK_UI_ENV: &str = "WOL_LOCK_UI";
/// Held (flock) by the running supervisor, in `$XDG_RUNTIME_DIR`.
pub const LOCK_GUARD: &str = "wol-lock.lock";
/// `wol-lock --startup`: the session's first screen (locked start, crash restart, ADR 0012).
pub const STARTUP: &str = "--startup";

/// Arguments the supervisor passes on to the lock screen (only [`STARTUP`]).
pub fn lock_ui_args(args: &[String]) -> Vec<String> {
    args.iter().filter(|a| *a == STARTUP).cloned().collect()
}

/// `$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY` (absolute display paths as given; empty = `wayland-0`).
pub fn wayland_socket(runtime: &str, display: &str) -> std::path::PathBuf {
    let d = if display.is_empty() {
        "wayland-0"
    } else {
        display
    };
    std::path::Path::new(runtime).join(d)
}

pub fn reported_unlock(stdout: &[u8]) -> bool {
    String::from_utf8_lossy(stdout)
        .lines()
        .any(|l| l == UNLOCKED)
}

pub const SECRETS: &str = "org.freedesktop.secrets";
pub const LOGIN_COLLECTION: &str = "/org/freedesktop/secrets/collection/login";

/// Start-up lock (ADR 0012): nobody typed a password at boot, so the login keyring opens with the
/// one typed at the lock screen (gnome-keyring's master-password call; `plain` transport stays on
/// this private session bus connection). Blocking.
pub fn unlock_login_keyring(conn: &zbus::blocking::Connection, password: &str) -> zbus::Result<()> {
    use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
    let root = "/org/freedesktop/secrets";
    let reply = conn.call_method(
        Some(SECRETS),
        root,
        Some("org.freedesktop.Secret.Service"),
        "OpenSession",
        &("plain", Value::from("")),
    )?;
    let (_, session): (OwnedValue, OwnedObjectPath) = reply.body().deserialize()?;
    let secret = (
        session.as_ref(),
        Vec::<u8>::new(),
        password.as_bytes(),
        "text/plain",
    );
    conn.call_method(
        Some(SECRETS),
        root,
        Some("org.gnome.keyring.InternalUnsupportedGuiltRiddenInterface"),
        "UnlockWithMasterPassword",
        &(ObjectPath::try_from(LOGIN_COLLECTION)?, secret),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts() {
        for (c, v) in [
            (Some(0), Verdict::Ok),
            (Some(1), Verdict::Denied),
            (Some(2), Verdict::Error),
            (None, Verdict::Error),
        ] {
            assert_eq!(from_exit(c), v);
        }
        for (r, v) in [
            ("ok\n", Verdict::Ok),
            ("denied", Verdict::Denied),
            ("error", Verdict::Error),
            ("", Verdict::Error),
        ] {
            assert_eq!(from_reply(r), v);
        }
    }

    #[test]
    fn request_format() {
        assert_eq!(login_request("klee", "pw").unwrap(), b"klee\npw");
        assert!(login_request("", "pw").is_none());
        assert!(login_request("a\nb", "pw").is_none());
        assert!(login_request("a", &"x".repeat(513)).is_none());
    }

    #[test]
    fn hint_after_three_failures() {
        for (n, hint, want) in [
            (2, Some("white rabbit\n"), None),
            (3, Some("white rabbit\n"), Some("Hint: white rabbit")),
            (5, Some("white rabbit"), Some("Hint: white rabbit")),
            (3, Some("  \n"), None),
            (3, None, None),
        ] {
            assert_eq!(hint_message(n, hint).as_deref(), want, "{n} {hint:?}");
        }
    }

    #[test]
    fn power_goes_to_wol_login() {
        assert_eq!(
            Power::ALL.map(power_label),
            ["Sleep", "Restart", "Shut Down"]
        );
        let dir = std::env::temp_dir().join(format!("wol-power-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let sock = dir.join("s");
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let srv = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut req = Vec::new();
            c.read_to_end(&mut req).unwrap();
            c.write_all(b"ok\n").unwrap();
            req
        });
        assert_eq!(power(sock.to_str().unwrap(), Power::Restart), Verdict::Ok);
        assert_eq!(srv.join().unwrap(), b"!restart\n");
        assert_eq!(power("/nonexistent/sock", Power::Sleep), Verdict::Error);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn login_users_show_full_names() {
        // /etc/passwd (7 fields): the login window tiles and pill show GECOS, auth keeps the login.
        let pw = "root:*:0:0:Charlie &:/root:/bin/sh\nanalima:*:1001:1001:Ana Lima,,,:/home/analima:/bin/sh\nbo:*:1002:1002::/home/bo:/bin/sh\n";
        assert_eq!(
            login_users(pw, "/bin/sh\n"),
            [
                ("analima".to_string(), "Ana Lima".to_string()),
                ("bo".into(), "bo".into())
            ]
        );
    }

    #[test]
    fn a_crashed_lock_screen_comes_back_until_unlocked() {
        use std::time::Duration;
        // (unlocked, compositor alive, attempt) -> next
        assert_eq!(lock_exit(true, true, 0), LockExit::Done);
        assert_eq!(lock_exit(false, false, 0), LockExit::Done, "session over");
        let delays: Vec<LockExit> = (0..8).map(|n| lock_exit(false, true, n)).collect();
        assert_eq!(delays[0], LockExit::Respawn(Duration::from_millis(100)));
        assert_eq!(delays[1], LockExit::Respawn(Duration::from_millis(200)));
        assert_eq!(delays[7], LockExit::Respawn(Duration::from_secs(3)));
        // Giving up never unlocks: the compositor keeps the session locked.
        assert_eq!(lock_exit(false, true, MAX_LOCK_RESPAWNS), LockExit::Done);
        // Only the line the lock screen prints after a real unlock counts.
        assert!(reported_unlock(format!("x\n{UNLOCKED}\n").as_bytes()));
        assert!(!reported_unlock(b"wol-lock: unlocked-ish\n"));
        assert!(!reported_unlock(b""));
    }

    /// The supervisor respawns only while the compositor that holds the lock is alive.
    #[test]
    fn avatar_tone_is_allow_listed() {
        assert_eq!(avatar_tone("green\n"), Some("green"));
        assert_eq!(avatar_tone("neon"), None);
        assert_eq!(avatar_tone("../x"), None);
        assert_eq!(read_avatar("../etc/passwd"), None);
    }

    #[test]
    fn compositor_alive_means_its_wayland_socket_exists() {
        use std::path::Path;
        assert_eq!(
            wayland_socket("/var/run/user/1001", "wayland-1"),
            Path::new("/var/run/user/1001/wayland-1")
        );
        assert_eq!(
            wayland_socket("/var/run/user/1001", "/tmp/w-0"),
            Path::new("/tmp/w-0")
        );
        assert_eq!(wayland_socket("/r", ""), Path::new("/r/wayland-0"));
        // The lock UI runs under the supervisor; `--startup` passes through.
        assert_eq!(lock_ui_args(&["--startup".into()]), ["--startup"]);
        assert!(lock_ui_args(&["--evil".into(), "x".into()]).is_empty());
    }

    #[test]
    fn initials_and_users() {
        assert_eq!(initials("Klee Lima"), "KL");
        assert_eq!(initials("root"), "R");
        let pw = "root:*:0:0::0:0:Charlie:/root:/bin/sh\nklee:*:1001:1001::0:0:K:/home/klee:/bin/sh\nnobody:*:65534:65534::0:0::/:/bin/sh\nsvc:*:1002:1:::::/usr/sbin/nologin";
        assert_eq!(users(pw, "# c\n/bin/sh\n/bin/csh\n"), ["klee"]);
    }
}
