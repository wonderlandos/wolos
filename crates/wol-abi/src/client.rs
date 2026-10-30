//! Safe Rust side of libwol: the sockets it talks to, with timeouts and bounded reads. The library
//! has no privilege of its own; anything that acts for the app is a `shared::portal::Call` to wold.
use shared::portal::{Answer, Call, Code};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Overrides for tests and development sessions (the app controls its own environment; wold
/// still decides).
pub const WOLD_ENV: &str = "WOL_SOCKET";
pub const KIT_ENV: &str = "WOL_KIT_SOCKET";
/// Kit host socket under `$XDG_RUNTIME_DIR`.
pub const KIT_SOCKET: &str = "wol-kit.sock";
pub const TIMEOUT: Duration = Duration::from_secs(5);
/// Longest line libwol reads from any socket.
pub const MAX_LINE: usize = 64 * 1024;

pub fn wold_socket() -> PathBuf {
    std::env::var_os(WOLD_ENV).map_or_else(|| shared::jobs::SOCKET.into(), PathBuf::from)
}

pub fn kit_socket() -> Option<PathBuf> {
    std::env::var_os(KIT_ENV)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join(KIT_SOCKET)))
}

fn unavailable() -> Answer {
    Answer::err(
        Code::Unavailable,
        "The WolOS system service is not running.",
    )
}

/// One service call: validated here first, decided by wold.
pub fn call(socket: &Path, call: &Call) -> Answer {
    if let Err(m) = call.validate() {
        return Answer::err(Code::InvalidArgument, m);
    }
    let Ok(mut s) = UnixStream::connect(socket) else {
        return unavailable();
    };
    let _ = s.set_read_timeout(Some(TIMEOUT));
    let _ = s.set_write_timeout(Some(TIMEOUT));
    if s.write_all(format!("call {}\n", call.encode()).as_bytes())
        .is_err()
    {
        return unavailable();
    }
    match read_line(&mut BufReader::new(s)) {
        Some(line) => Answer::parse(&line),
        None => unavailable(),
    }
}

/// Event stream from wold (`shared::events` lines).
pub fn subscribe(socket: &Path) -> std::io::Result<UnixStream> {
    let mut s = UnixStream::connect(socket)?;
    s.set_write_timeout(Some(TIMEOUT))?;
    s.write_all(b"subscribe\n")?;
    Ok(s)
}

/// Next line, at most [`MAX_LINE`] bytes; None at end of stream, on error, or on a longer line
/// (the connection is then not trusted any more).
pub fn read_line(r: &mut impl BufRead) -> Option<String> {
    let mut buf = Vec::new();
    let n = r
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', &mut buf)
        .ok()?;
    if n == 0 || (n > MAX_LINE && buf.last() != Some(&b'\n')) {
        return None;
    }
    String::from_utf8(buf).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn read_line_is_bounded() {
        let mut r = BufReader::new(&b"one\ntwo"[..]);
        assert_eq!(read_line(&mut r).as_deref(), Some("one\n"));
        assert_eq!(read_line(&mut r).as_deref(), Some("two"));
        assert_eq!(read_line(&mut r), None);
        let long = vec![b'x'; MAX_LINE + 10];
        assert_eq!(read_line(&mut BufReader::new(&long[..])), None);
    }

    #[test]
    fn call_validates_first_and_reports_a_missing_service() {
        let bad = Call::NotificationPost {
            title: String::new(),
            body: String::new(),
        };
        let a = call(Path::new("/nonexistent/wold.sock"), &bad);
        assert_eq!(a.code, Some(Code::InvalidArgument));
        let ok = Call::NotificationPost {
            title: "t".into(),
            body: String::new(),
        };
        let a = call(Path::new("/nonexistent/wold.sock"), &ok);
        assert_eq!(a.code, Some(Code::Unavailable));
    }

    #[test]
    fn call_round_trip() {
        let p = std::env::temp_dir().join(format!("wol-client-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let l = UnixListener::bind(&p).unwrap();
        let t = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let line = read_line(&mut BufReader::new(&s)).unwrap();
            (&s).write_all(Answer::ok().to_line().as_bytes()).unwrap();
            line
        });
        let c = Call::NotificationPost {
            title: "t".into(),
            body: "b".into(),
        };
        assert!(call(&p, &c).ok);
        assert_eq!(t.join().unwrap(), format!("call {}\n", c.encode()));
        let _ = std::fs::remove_file(p);
    }
}
