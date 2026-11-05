// SPDX-License-Identifier: BSD-2-Clause
//! launchd-style jobs: one JSON file per job in [`JOBS_DIR`], wold control socket wire.
use serde::{Deserialize, Serialize};

pub const JOBS_DIR: &str = "/etc/wolos/jobs";
pub const SOCKET: &str = "/var/run/wold.sock";

/// First word of the identity handshake (ADR 0033): wold sends `hello <nonce>` before it reads
/// anything, and the client's first line must be the same `hello <nonce>`. A request written before
/// wold accepted the connection therefore never counts, so a process cannot connect, exec() into
/// another binary and have its pre-written request run under that binary's identity.
pub const HELLO: &str = "hello";

/// How long either side waits for the other's hello.
pub const HELLO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Longest hello line (bytes, newline included): `hello ` plus 32 hex digits, with slack.
const MAX_HELLO: usize = 64;

/// A fresh 128-bit nonce as 32 lowercase hex digits. Err if the kernel's random source is unreadable.
pub fn new_nonce() -> std::io::Result<String> {
    use std::io::Read;
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// The hello line for `nonce`, newline included.
pub fn hello_line(nonce: &str) -> String {
    format!("{HELLO} {nonce}\n")
}

/// Server side: send a fresh hello on `s`; returns the line the client must echo first.
pub fn send_hello(s: &std::os::unix::net::UnixStream) -> std::io::Result<String> {
    use std::io::Write;
    let line = hello_line(&new_nonce()?);
    (&*s).write_all(line.as_bytes())?;
    Ok(line)
}

/// Client side: read wold's hello (byte by byte, so nothing after it is consumed) within
/// [`HELLO_TIMEOUT`] and echo it. Err if wold sent no well-formed hello: do not send the request.
pub fn answer_hello(s: &std::os::unix::net::UnixStream) -> std::io::Result<()> {
    use std::io::{Read, Write};
    let before = s.read_timeout()?;
    s.set_read_timeout(Some(HELLO_TIMEOUT))?;
    let mut line = Vec::with_capacity(MAX_HELLO);
    let mut byte = [0u8; 1];
    while line.last() != Some(&b'\n') {
        if line.len() >= MAX_HELLO || (&*s).read(&mut byte)? == 0 {
            return Err(std::io::Error::other("wold did not greet the connection"));
        }
        line.extend_from_slice(&byte);
    }
    s.set_read_timeout(before)?;
    let ok = std::str::from_utf8(&line).ok().and_then(|l| {
        let nonce = l
            .strip_prefix(HELLO)?
            .strip_prefix(' ')?
            .strip_suffix('\n')?;
        (!nonce.is_empty() && nonce.bytes().all(|b| b.is_ascii_alphanumeric())).then_some(())
    });
    if ok.is_none() {
        return Err(std::io::Error::other("wold's greeting was not a hello"));
    }
    (&*s).write_all(&line)
}

/// Server side in one step, for wold: send a hello and read the client's echo (byte by byte, so
/// the request after it stays unread). Err if the echo does not match, or if any byte of it was
/// written by another process than the one the kernel recorded at connect (SO_PEERCRED). The
/// writer comes from SCM_CREDENTIALS: SO_PASSCRED is on before the hello goes out, so the kernel
/// stamps every byte the client writes with the writing process (an unprivileged sender cannot
/// claim another pid). That closes the fork gap of ADR 0033: a process that connects, forks and
/// execs a trusted binary before accept cannot have the child answer for it. Clients need nothing
/// extra: [`answer_hello`] is a plain write.
pub fn greet(s: &std::os::unix::net::UnixStream) -> std::io::Result<()> {
    greet_by(s, std::time::Instant::now() + HELLO_TIMEOUT)
}

/// [`greet`] with the whole exchange (hello out, echo in) due at `deadline`: the wait is cut down
/// before each read, so a peer that sends a byte now and then does not extend it. The atlas bridge
/// uses this to count its hello wait from accept (QA-A9-SEC-1).
pub fn greet_by(
    s: &std::os::unix::net::UnixStream,
    deadline: std::time::Instant,
) -> std::io::Result<()> {
    let before = s.read_timeout()?;
    let peer = crate::sys::peer(s)
        .filter(|p| p.pid > 0)
        .ok_or_else(|| std::io::Error::other("the kernel did not say who connected"))?;
    crate::sys::set_passcred(s, true)?;
    let hello = send_hello(s)?;
    let mut line = Vec::with_capacity(MAX_HELLO);
    let mut byte = [0u8; 1];
    while line.last() != Some(&b'\n') {
        if line.len() >= MAX_HELLO {
            return Err(std::io::Error::other("the client did not answer the hello"));
        }
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err(std::io::Error::other(
                "the client did not answer the hello in time",
            ));
        }
        s.set_read_timeout(Some(left))?;
        let (n, pid) = crate::sys::recv_with_pid(s, &mut byte)?;
        if n == 0 {
            return Err(std::io::Error::other("the client did not answer the hello"));
        }
        if pid != Some(peer.pid) {
            return Err(std::io::Error::other(
                "the hello was answered by another process than the one that connected",
            ));
        }
        line.extend_from_slice(&byte);
    }
    // Off again: later reads (FdReader) expect only descriptors in the control data.
    crate::sys::set_passcred(s, false)?;
    if line != hello.as_bytes() {
        return Err(std::io::Error::other("the client did not answer the hello"));
    }
    s.set_read_timeout(before)
}

/// Connect to wold's control socket at `path` and complete the handshake; then write the request.
pub fn connect(
    path: impl AsRef<std::path::Path>,
) -> std::io::Result<std::os::unix::net::UnixStream> {
    let s = std::os::unix::net::UnixStream::connect(path)?;
    answer_hello(&s)?;
    Ok(s)
}

/// Job definition. `run_at_load = false` = on demand (`wol start <label>`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobSpec {
    pub label: String,
    pub program: Vec<String>,
    #[serde(default)]
    pub keep_alive: bool,
    #[serde(default = "ten")]
    pub throttle_seconds: u32,
    #[serde(default = "yes")]
    pub run_at_load: bool,
    /// QoS class name (`shared::qos::QosClass::name`); default "default".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qos: Option<String>,
    /// Run as this account, with a clean environment; default root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<u32>,
}

fn ten() -> u32 {
    10
}
fn yes() -> bool {
    true
}

impl JobSpec {
    pub fn parse(json: &str) -> Result<Self, String> {
        let j: JobSpec = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if j.label.is_empty() || j.program.is_empty() || j.program[0].is_empty() {
            return Err("job needs label and program".into());
        }
        if let Some(q) = &j.qos
            && crate::qos::QosClass::parse(q).is_none()
        {
            return Err(format!("unknown qos {q}"));
        }
        Ok(j)
    }
}

/// One job as reported by `list`. state: running | stopped | waiting.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub label: String,
    pub state: String,
    pub pid: Option<i32>,
    pub starts: u32,
    pub last_exit: Option<i32>,
}

/// Control request: one line in, one JSON line out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobRequest {
    List,
    Start(String),
    Stop(String),
    /// Stream events (`shared::events`) until the client disconnects.
    Subscribe,
    /// Also this account's events (notifications): only the session's notification shower.
    SubscribeAccount,
    /// Publish one `shared::events::Event` JSON line to subscribers.
    Publish(String),
    /// libwol service call (`shared::portal::Call` JSON, decoded and checked by wold).
    Call(String),
    /// A system web app's bridge call (ADR 0048): `<method> [<params JSON>]`, from the app's host
    /// process only; the app is the caller's scope, never a word here.
    WebappCall(String),
}

impl JobRequest {
    pub fn parse(line: &str) -> Option<Self> {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            ["list"] => Some(Self::List),
            ["subscribe"] => Some(Self::Subscribe),
            ["subscribe", "account"] => Some(Self::SubscribeAccount),
            ["publish", ..] => {
                let json = line.trim().strip_prefix("publish")?.trim();
                crate::events::Event::parse(json).map(|_| Self::Publish(json.into()))
            }
            ["call", ..] => Some(Self::Call(line.trim().strip_prefix("call")?.trim().into())),
            ["webapp-call", _, ..] => Some(Self::WebappCall(
                line.trim().strip_prefix("webapp-call")?.trim().into(),
            )),
            ["start", l] => Some(Self::Start((*l).into())),
            ["stop", l] => Some(Self::Stop((*l).into())),
            _ => None,
        }
    }

    pub fn line(&self) -> String {
        match self {
            Self::List => "list".into(),
            Self::Subscribe => "subscribe".into(),
            Self::SubscribeAccount => "subscribe account".into(),
            Self::Publish(j) => format!("publish {j}"),
            Self::Call(j) => format!("call {j}"),
            Self::WebappCall(r) => format!("webapp-call {r}"),
            Self::Start(l) => format!("start {l}"),
            Self::Stop(l) => format!("stop {l}"),
        }
    }

    /// Mutating requests need root (peer uid 0).
    pub fn needs_root(&self) -> bool {
        matches!(self, Self::Start(_) | Self::Stop(_))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub ok: bool,
    pub error: Option<String>,
}

impl Reply {
    pub fn of(error: Option<String>) -> String {
        serde_json::to_string(&Reply {
            ok: error.is_none(),
            error,
        })
        .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_hello_round_trips_and_a_bad_greeting_is_refused() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixStream;
        let (srv, cli) = UnixStream::pair().unwrap();
        let line = send_hello(&srv).unwrap();
        assert_eq!(line.len(), "hello ".len() + 32 + 1);
        answer_hello(&cli).unwrap();
        (&cli).write_all(b"list\n").unwrap();
        let mut r = BufReader::new(&srv);
        let mut got = String::new();
        r.read_line(&mut got).unwrap();
        assert_eq!(got, line, "the echo comes first");
        got.clear();
        r.read_line(&mut got).unwrap();
        assert_eq!(got, "list\n", "nothing after the hello was eaten");

        let (srv, cli) = UnixStream::pair().unwrap();
        (&srv).write_all(b"[]\n").unwrap();
        assert!(answer_hello(&cli).is_err());
        assert_ne!(new_nonce().unwrap(), new_nonce().unwrap());
    }

    /// The server side checks who wrote the echo: the process that connected (here this one,
    /// from another thread) passes, and the request after it stays unread.
    #[test]
    fn greet_accepts_the_echo_from_the_connecting_process() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;
        let (srv, cli) = UnixStream::pair().unwrap();
        let h = std::thread::spawn(move || {
            answer_hello(&cli).unwrap();
            (&cli).write_all(b"list\n").unwrap();
            cli
        });
        greet(&srv).unwrap();
        let _cli = h.join().unwrap();
        let mut rest = [0u8; 5];
        (&srv).read_exact(&mut rest).unwrap();
        assert_eq!(&rest, b"list\n");
    }

    /// ADR 0033 fork gap: the socket's other end is handed to another process (here `sh`, as its
    /// stdin and stdout), which answers the hello. SO_PEERCRED names this process,
    /// SCM_CREDENTIALS names the shell, so the connection is refused.
    #[test]
    fn greet_refuses_an_echo_from_another_process() {
        use std::os::unix::net::UnixStream;
        use std::process::{Command, Stdio};
        let (srv, cli) = UnixStream::pair().unwrap();
        let input = std::os::fd::OwnedFd::from(cli.try_clone().unwrap());
        let mut child = Command::new("/bin/sh")
            .args(["-c", r#"read l; printf '%s\n' "$l"; sleep 2"#])
            .stdin(Stdio::from(input))
            .stdout(Stdio::from(std::os::fd::OwnedFd::from(cli)))
            .spawn()
            .unwrap();
        let e = greet(&srv).unwrap_err();
        assert!(e.to_string().contains("another process"), "{e}");
        let _ = child.kill();
        let _ = child.wait();
    }

    use super::*;

    /// A peer that sends a byte now and then does not stretch the deadline (the atlas bridge counts
    /// its hello wait from accept).
    #[test]
    fn greet_by_cuts_a_trickling_peer_off_at_the_deadline() {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        let (srv, mut cli) = UnixStream::pair().unwrap();
        let t = std::time::Instant::now();
        let h = std::thread::spawn(move || {
            // A pause that is a wait on a channel nobody sends on (tests do not sleep).
            let (_keep, never) = std::sync::mpsc::channel::<()>();
            for _ in 0..40 {
                let _ = never.recv_timeout(std::time::Duration::from_millis(50));
                if cli.write_all(b" ").is_err() {
                    break;
                }
            }
        });
        let e = greet_by(&srv, t + std::time::Duration::from_millis(500)).unwrap_err();
        assert!(t.elapsed() < std::time::Duration::from_millis(1500), "{e}");
        drop(srv);
        let _ = h.join();
    }

    #[test]
    fn spec_defaults_and_validation() {
        let j = JobSpec::parse(r#"{"label":"a","program":["/bin/a"]}"#).unwrap();
        assert!(!j.keep_alive && j.run_at_load && j.throttle_seconds == 10 && j.qos.is_none());
        for bad in [
            "{}",
            r#"{"label":"","program":["x"]}"#,
            r#"{"label":"a","program":[]}"#,
            r#"{"label":"a","program":["x"],"qos":"turbo"}"#,
            "not json",
        ] {
            assert!(JobSpec::parse(bad).is_err(), "{bad}");
        }
        let j = JobSpec::parse(r#"{"label":"a","program":["x"],"keepAlive":true,"throttleSeconds":3,"runAtLoad":false,"qos":"utility"}"#).unwrap();
        assert!(j.keep_alive && !j.run_at_load && j.throttle_seconds == 3);
    }

    #[test]
    fn request_wire_table() {
        for (line, want) in [
            ("list", Some(JobRequest::List)),
            (" list \n", Some(JobRequest::List)),
            ("subscribe", Some(JobRequest::Subscribe)),
            ("subscribe account", Some(JobRequest::SubscribeAccount)),
            ("subscribe all", None),
            ("start a", Some(JobRequest::Start("a".into()))),
            ("stop a", Some(JobRequest::Stop("a".into()))),
            ("start", None),
            ("start a b", None),
            ("rm a", None),
            (
                "publish {\"event\":\"workspace\",\"name\":\"1\"}",
                Some(JobRequest::Publish(
                    "{\"event\":\"workspace\",\"name\":\"1\"}".into(),
                )),
            ),
            ("publish nope", None),
            (
                "call {\"call\":\"notification.post\",\"title\":\"a b\"}\n",
                Some(JobRequest::Call(
                    "{\"call\":\"notification.post\",\"title\":\"a b\"}".into(),
                )),
            ),
        ] {
            assert_eq!(JobRequest::parse(line), want, "{line}");
            if let Some(r) = want {
                assert_eq!(JobRequest::parse(&r.line()), Some(r));
            }
        }
        assert!(!JobRequest::List.needs_root() && JobRequest::Stop("a".into()).needs_root());
        assert!(!JobRequest::Call(String::new()).needs_root());
    }

    #[test]
    fn status_and_reply_json() {
        let s = JobStatus {
            label: "a".into(),
            state: "running".into(),
            pid: Some(3),
            starts: 1,
            last_exit: None,
        };
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(
            j,
            r#"{"label":"a","state":"running","pid":3,"starts":1,"lastExit":null}"#
        );
        assert_eq!(Reply::of(None), r#"{"ok":true,"error":null}"#);
        assert_eq!(Reply::of(Some("x".into())), r#"{"ok":false,"error":"x"}"#);
    }
}
