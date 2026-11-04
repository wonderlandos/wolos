// SPDX-License-Identifier: GPL-2.0-or-later
//! tor's control port over its Unix socket (ADR 0053 D2, D4): cookie authentication, the
//! bootstrap phase (the relay holds connects until it is `done`), `TAKEOWNERSHIP` (tor exits
//! when the host's control connection closes, crash included), and health.
//!
//! Replies are read line by line with a bound on the line length and on the number of lines, so
//! a misbehaving peer cannot make the host buffer without limit. Every call has a timeout.
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// Longest reply line accepted.
pub const MAX_LINE: usize = 8192;
/// Most lines in one reply.
pub const MAX_LINES: usize = 512;
/// tor's control cookie is 32 bytes.
pub const COOKIE_LEN: usize = 32;

/// A control-port failure.
#[derive(Debug)]
pub enum ControlError {
    /// The socket or the cookie file could not be used.
    Io(io::Error),
    /// tor answered with this status and text.
    Status(u16, String),
    /// The reply was not in the control-spec format, or too large.
    Malformed(String),
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "Tor's control socket could not be used ({e})"),
            Self::Status(c, t) => write!(f, "Tor answered {c}: {t}"),
            Self::Malformed(t) => write!(f, "Tor's control reply was not understood: {t}"),
        }
    }
}

impl std::error::Error for ControlError {}

impl From<io::Error> for ControlError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// One reply: the status code and the text of each line (`250-`, `250 ` and data lines).
#[derive(Debug, PartialEq, Eq)]
pub struct ControlReply {
    /// Status (250 = OK).
    pub status: u16,
    /// Lines without the status prefix.
    pub lines: Vec<String>,
}

/// Where bootstrap is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bootstrap {
    /// 0..=100.
    pub progress: u8,
    /// `done` at 100.
    pub tag: String,
}

impl Bootstrap {
    /// Bootstrapped: circuits can be built.
    pub fn done(&self) -> bool {
        self.progress == 100
    }
}

/// Parses `NOTICE BOOTSTRAP PROGRESS=85 TAG=ap_conn_done SUMMARY="..."` (the value of
/// `status/bootstrap-phase` and of a `STATUS_CLIENT` event).
pub fn parse_bootstrap(s: &str) -> Option<Bootstrap> {
    let mut progress = None;
    let mut tag = None;
    for w in s.split(' ') {
        if let Some(p) = w.strip_prefix("PROGRESS=") {
            progress = p.parse::<u8>().ok().filter(|p| *p <= 100);
        } else if let Some(t) = w.strip_prefix("TAG=") {
            tag = Some(t.to_string());
        }
    }
    Some(Bootstrap {
        progress: progress?,
        tag: tag?,
    })
}

fn read_line(r: &mut impl Read) -> Result<String, ControlError> {
    let mut buf = Vec::new();
    let mut b = [0u8; 1];
    loop {
        if r.read(&mut b)? == 0 {
            return Err(ControlError::Malformed(
                "the connection closed inside a reply".into(),
            ));
        }
        if b[0] == b'\n' {
            break;
        }
        if buf.len() >= MAX_LINE {
            return Err(ControlError::Malformed("a reply line is too long".into()));
        }
        buf.push(b[0]);
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    String::from_utf8(buf).map_err(|_| ControlError::Malformed("a reply line is not text".into()))
}

/// Reads one reply (control-spec section 2.3: `NNN-` mid, `NNN+` data until `.`, `NNN ` end).
pub fn read_reply(r: &mut impl Read) -> Result<ControlReply, ControlError> {
    let mut lines = Vec::new();
    loop {
        if lines.len() >= MAX_LINES {
            return Err(ControlError::Malformed(
                "the reply has too many lines".into(),
            ));
        }
        let l = read_line(r)?;
        if l.len() < 4 || !l.is_char_boundary(3) || !l.is_char_boundary(4) {
            return Err(ControlError::Malformed(l));
        }
        let status: u16 = l[..3]
            .parse()
            .map_err(|_| ControlError::Malformed(l.clone()))?;
        let sep = l.as_bytes()[3];
        let text = l[4..].to_string();
        match sep {
            b' ' => {
                lines.push(text);
                return Ok(ControlReply { status, lines });
            }
            b'-' => lines.push(text),
            b'+' => {
                lines.push(text);
                loop {
                    if lines.len() >= MAX_LINES {
                        return Err(ControlError::Malformed(
                            "the reply has too many lines".into(),
                        ));
                    }
                    let d = read_line(r)?;
                    if d == "." {
                        break;
                    }
                    lines.push(d.strip_prefix('.').map(str::to_string).unwrap_or(d));
                }
            }
            _ => return Err(ControlError::Malformed(l)),
        }
    }
}

/// Reads the 32-byte cookie file.
pub fn read_cookie(path: &Path) -> Result<Vec<u8>, ControlError> {
    let mut v = Vec::with_capacity(COOKIE_LEN);
    File::open(path)?
        .take(u64::try_from(COOKIE_LEN + 1).unwrap_or(33))
        .read_to_end(&mut v)?;
    if v.len() != COOKIE_LEN {
        return Err(ControlError::Malformed(format!(
            "the control cookie has {} bytes, not {COOKIE_LEN}",
            v.len()
        )));
    }
    Ok(v)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

/// A control connection over any stream (a Unix socket in use, a buffer in tests).
pub struct Control<S: Read + Write> {
    s: BufReader<S>,
}

impl Control<UnixStream> {
    /// Connects to tor's control socket with a timeout on every read and write.
    pub fn connect(path: &Path, timeout: Duration) -> Result<Self, ControlError> {
        let s = UnixStream::connect(path)?;
        s.set_read_timeout(Some(timeout))?;
        s.set_write_timeout(Some(timeout))?;
        Ok(Self::new(s))
    }
}

/// The fingerprint (without `$`) of the last hop of the first built general-purpose circuit in
/// the text of `GETINFO circuit-status`.
pub fn exit_fingerprint(circuit_status: &str) -> Option<String> {
    circuit_status.lines().find_map(|l| {
        let mut f = l.split_whitespace();
        let (_id, state, path) = (f.next()?, f.next()?, f.next()?);
        let general = l.split_whitespace().any(|w| w == "PURPOSE=GENERAL");
        if state != "BUILT" || !general {
            return None;
        }
        let last = path.rsplit(',').next()?;
        let fp = last.strip_prefix('$')?.split('~').next()?;
        (fp.len() == 40 && fp.bytes().all(|b| b.is_ascii_hexdigit())).then(|| fp.to_string())
    })
}

/// The relay address in the `r` line of `GETINFO ns/id/<fp>`.
pub fn router_address(ns_entry: &str) -> Option<String> {
    let r = ns_entry.lines().find(|l| l.starts_with("r "))?;
    r.split_whitespace().nth(6).map(str::to_string)
}

impl<S: Read + Write> Control<S> {
    /// Where the current exit relay is, as a two-letter country code (lowercase), for the status
    /// row. `None` when no general circuit is built or tor has no geoip file.
    pub fn exit_country(&mut self) -> Result<Option<String>, ControlError> {
        let Some(fp) = exit_fingerprint(&self.getinfo("circuit-status")?) else {
            return Ok(None);
        };
        let Some(ip) = router_address(&self.getinfo(&format!("ns/id/{fp}"))?) else {
            return Ok(None);
        };
        let cc = self.getinfo(&format!("ip-to-country/{ip}"))?;
        Ok((cc.len() == 2 && cc.bytes().all(|b| b.is_ascii_lowercase())).then_some(cc))
    }

    /// Wraps a stream.
    pub fn new(s: S) -> Self {
        Self {
            s: BufReader::new(s),
        }
    }

    /// Sends one command line (no CR/LF allowed inside) and reads its reply; non-2xx is an error.
    pub fn command(&mut self, line: &str) -> Result<ControlReply, ControlError> {
        if line.contains(['\r', '\n']) {
            return Err(ControlError::Malformed("a command has a line break".into()));
        }
        let w = self.s.get_mut();
        w.write_all(line.as_bytes())?;
        w.write_all(b"\r\n")?;
        w.flush()?;
        let r = read_reply(&mut self.s)?;
        if !(200..300).contains(&r.status) {
            return Err(ControlError::Status(r.status, r.lines.join(" ")));
        }
        Ok(r)
    }

    /// `AUTHENTICATE <cookie hex>`.
    pub fn authenticate(&mut self, cookie: &[u8]) -> Result<(), ControlError> {
        self.command(&format!("AUTHENTICATE {}", hex(cookie)))
            .map(|_| ())
    }

    /// `GETINFO <key>`: the value of `key=value` (or the data block of `key=`).
    pub fn getinfo(&mut self, key: &str) -> Result<String, ControlError> {
        let r = self.command(&format!("GETINFO {key}"))?;
        let prefix = format!("{key}=");
        let mut it = r.lines.iter();
        while let Some(l) = it.next() {
            if let Some(v) = l.strip_prefix(&prefix) {
                if v.is_empty() {
                    let rest: Vec<&str> = it
                        .take_while(|x| x.as_str() != "OK")
                        .map(String::as_str)
                        .collect();
                    return Ok(rest.join("\n"));
                }
                return Ok(v.to_string());
            }
        }
        Err(ControlError::Malformed(format!("no {key} in the reply")))
    }

    /// Where bootstrap is now.
    pub fn bootstrap(&mut self) -> Result<Bootstrap, ControlError> {
        let v = self.getinfo("status/bootstrap-phase")?;
        parse_bootstrap(&v).ok_or(ControlError::Malformed(v))
    }

    /// tor exits when this connection closes (the host ended or crashed).
    pub fn take_ownership(&mut self) -> Result<(), ControlError> {
        self.command("TAKEOWNERSHIP")?;
        // Without this, a later SIGHUP-style reload would re-read the owning-PID option.
        self.command("RESETCONF __OwningControllerProcess")
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Fake {
        replies: Cursor<Vec<u8>>,
        sent: Vec<u8>,
    }

    impl Read for Fake {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.replies.read(b)
        }
    }

    impl Write for Fake {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.sent.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn ctl(replies: &str) -> Control<Fake> {
        Control::new(Fake {
            replies: Cursor::new(replies.as_bytes().to_vec()),
            sent: vec![],
        })
    }

    #[test]
    fn bootstrap_phase_is_parsed_from_getinfo() {
        let mut c = ctl(
            "250-status/bootstrap-phase=NOTICE BOOTSTRAP PROGRESS=100 TAG=done SUMMARY=\"Done\"\r\n250 OK\r\n",
        );
        let b = c.bootstrap().unwrap();
        assert!(b.done());
        assert_eq!(b.tag, "done");
        assert_eq!(c.s.get_ref().sent, b"GETINFO status/bootstrap-phase\r\n");
        assert_eq!(
            parse_bootstrap("NOTICE BOOTSTRAP PROGRESS=14 TAG=handshake")
                .unwrap()
                .progress,
            14
        );
        assert_eq!(parse_bootstrap("NOTICE BOOTSTRAP PROGRESS=140 TAG=x"), None);
    }

    #[test]
    fn cookie_auth_sends_hex_and_errors_carry_status() {
        let mut c = ctl("250 OK\r\n");
        c.authenticate(&[0xab, 0x01]).unwrap();
        assert_eq!(c.s.get_ref().sent, b"AUTHENTICATE AB01\r\n");
        let mut c = ctl("515 Authentication failed\r\n");
        assert!(matches!(
            c.authenticate(&[0]),
            Err(ControlError::Status(515, _))
        ));
    }

    #[test]
    fn data_replies_and_dot_stuffing_are_read() {
        let mut c = ctl("250+circuit-status=\r\n1 BUILT $A,$B\r\n..dot\r\n.\r\n250 OK\r\n");
        assert_eq!(c.getinfo("circuit-status").unwrap(), "1 BUILT $A,$B\n.dot");
    }

    #[test]
    fn oversized_and_broken_replies_are_refused() {
        let long = format!("250 {}\r\n", "x".repeat(MAX_LINE + 1));
        assert!(matches!(
            read_reply(&mut Cursor::new(long.into_bytes())),
            Err(ControlError::Malformed(_))
        ));
        assert!(matches!(
            read_reply(&mut Cursor::new(b"25".to_vec())),
            Err(ControlError::Malformed(_))
        ));
        assert!(matches!(
            read_reply(&mut Cursor::new(b"250*x\r\n".to_vec())),
            Err(ControlError::Malformed(_))
        ));
        let many = "250-x\r\n".repeat(MAX_LINES + 1);
        assert!(matches!(
            read_reply(&mut Cursor::new(many.into_bytes())),
            Err(ControlError::Malformed(_))
        ));
    }

    #[test]
    fn commands_cannot_smuggle_a_second_line() {
        let mut c = ctl("250 OK\r\n");
        assert!(matches!(
            c.command("GETINFO x\r\nSIGNAL HALT"),
            Err(ControlError::Malformed(_))
        ));
        assert!(c.s.get_ref().sent.is_empty());
    }

    #[test]
    fn ownership_is_taken_and_owning_pid_reset() {
        let mut c = ctl("250 OK\r\n250 OK\r\n");
        c.take_ownership().unwrap();
        assert_eq!(
            c.s.get_ref().sent,
            b"TAKEOWNERSHIP\r\nRESETCONF __OwningControllerProcess\r\n"
        );
    }

    #[test]
    fn the_exit_is_the_last_hop_of_a_built_general_circuit() {
        let fp = |c: char| c.to_string().repeat(40);
        let text = format!(
            concat!(
                "1 EXTENDED ${a}~g,${b}~m PURPOSE=GENERAL\n",
                "2 BUILT ${a}~g,${b}~m,${c}~exit BUILD_FLAGS=NEED_CAPACITY PURPOSE=HS_CLIENT_INTRO\n",
                "3 BUILT ${a}~g,${b}~m,${d}~exit2 PURPOSE=GENERAL TIME_CREATED=x"
            ),
            a = fp('A'),
            b = fp('B'),
            c = fp('C'),
            d = fp('D')
        );
        assert_eq!(exit_fingerprint(&text), Some(fp('D')));
        assert_eq!(exit_fingerprint("1 LAUNCHED $x~y PURPOSE=GENERAL"), None);
        assert_eq!(exit_fingerprint(""), None);
        assert_eq!(exit_fingerprint("3 BUILT $short~x PURPOSE=GENERAL"), None);
    }

    #[test]
    fn the_relay_address_is_field_six_of_the_r_line() {
        let ns = "r nick AAAA BBBB 2026-11-04 20:00:00 192.0.2.44 9001 0\ns Fast Running\n";
        assert_eq!(router_address(ns), Some("192.0.2.44".into()));
        assert_eq!(router_address("s Fast\n"), None);
    }
}
