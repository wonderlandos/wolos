// SPDX-License-Identifier: GPL-2.0-or-later
//! Asking the ClamAV process (ADR 0028). Nothing here links it: this side passes an open file over
//! a socket and reads one line, so the libraries built from this crate stay free of libclamav.
use crate::scan::{Finding, Report, Skipped, Verdict};
use std::io::{BufRead, BufReader};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Signature data freshclam keeps, present only after the person opted in.
pub const DB_DIR: &str = "/var/lib/wolos/xguard/clamav";
pub const SOCKET: &str = "/run/wolos/clam.sock";
/// Set in tests and measurements instead of the two above.
pub const SOCKET_ENV: &str = "XGUARD_CLAM_SOCKET";
/// A load of the full database takes seconds: wait for it, but never hold a file forever.
pub const WAIT: Duration = Duration::from_secs(30);

/// What the process said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Clean,
    Found(String),
    /// It could not answer: why.
    NotRun(String),
}

/// `found NAME`, `clean`, or anything else.
pub fn parse(line: &str) -> Answer {
    match line.trim_end().split_once(' ') {
        _ if line.trim_end() == "clean" => Answer::Clean,
        Some(("found", n)) if !n.is_empty() => Answer::Found(n.to_owned()),
        Some(("error", why)) => Answer::NotRun(why.to_owned()),
        _ => Answer::NotRun("no answer".into()),
    }
}

/// Pass this file and wait for the line.
pub fn ask(socket: &Path, f: &std::fs::File, wait: Duration) -> Answer {
    let run = || -> std::io::Result<String> {
        let s = UnixStream::connect(socket)?;
        s.set_read_timeout(Some(wait))?;
        crate::ctl::send_fd(&s, "scan", f.as_raw_fd())?;
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line)?;
        Ok(line)
    };
    match run() {
        Ok(l) => parse(&l),
        Err(e) => Answer::NotRun(e.to_string()),
    }
}

/// The socket to use: `env` when set, else the system's once there is data to scan with.
pub fn socket(env: Option<PathBuf>, db: &Path) -> Option<PathBuf> {
    let has_data = || {
        ["daily.cvd", "daily.cld"]
            .iter()
            .any(|f| db.join(f).exists())
    };
    env.or_else(|| has_data().then(|| SOCKET.into()))
}

/// Add the answer to a report: a finding wins, and no answer means the result is not complete.
pub fn merge(r: &mut Report, a: Answer, path: &str) {
    match a {
        Answer::Clean => {}
        Answer::Found(name) => {
            r.findings.push(Finding {
                name,
                how: "clamav".into(),
                path: path.to_owned(),
            });
            r.verdict = Verdict::Found;
        }
        Answer::NotRun(why) => {
            r.skipped.push(Skipped {
                path: path.to_owned(),
                why: format!("clamav: {why}"),
            });
            if r.verdict == Verdict::Clean {
                r.verdict = Verdict::Incomplete;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, Write};
    use std::os::unix::net::UnixListener;

    fn report() -> Report {
        crate::scan::Engine::default().blank(0)
    }

    #[test]
    fn the_three_answers() {
        assert_eq!(parse("clean\n"), Answer::Clean);
        assert_eq!(
            parse("found Win.Test-1\n"),
            Answer::Found("Win.Test-1".into())
        );
        assert_eq!(
            parse("error Can't map\n"),
            Answer::NotRun("Can't map".into())
        );
        assert_eq!(parse(""), Answer::NotRun("no answer".into()));
        assert_eq!(parse("found \n"), Answer::NotRun("no answer".into()));
    }

    #[test]
    fn the_file_itself_goes_over_the_socket_and_the_line_comes_back() {
        let d = std::env::temp_dir().join(format!("xg-c-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let sock = d.join("s");
        let l = UnixListener::bind(&sock).unwrap();
        let t = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let (_, fd) = crate::ctl::recv_fd(&s).unwrap();
            let mut got = String::new();
            let mut f = std::fs::File::from(fd.unwrap());
            f.rewind().unwrap();
            f.read_to_string(&mut got).unwrap();
            (&s).write_all(format!("found {got}\n").as_bytes()).unwrap();
        });
        std::fs::write(d.join("f"), "Sig-9").unwrap();
        let f = std::fs::File::open(d.join("f")).unwrap();
        assert_eq!(ask(&sock, &f, WAIT), Answer::Found("Sig-9".into()));
        t.join().unwrap();
    }

    #[test]
    fn nobody_there_is_not_a_clean_result() {
        let f = std::fs::File::open("/proc/self/exe").unwrap();
        let a = ask(Path::new("/nonexistent/s"), &f, WAIT);
        assert!(matches!(a, Answer::NotRun(_)), "{a:?}");
        let mut r = report();
        r.verdict = Verdict::Clean;
        merge(&mut r, a, "x");
        assert_eq!(r.verdict, Verdict::Incomplete);
        assert!(r.skipped[0].why.starts_with("clamav: "));
    }

    #[test]
    fn a_finding_wins() {
        let mut r = report();
        r.verdict = Verdict::Incomplete;
        merge(&mut r, Answer::Found("X".into()), "a.exe");
        assert_eq!(r.verdict, Verdict::Found);
        assert_eq!(r.findings[0].how, "clamav");
    }

    #[test]
    fn asked_only_when_opted_in() {
        let d = std::env::temp_dir().join(format!("xg-o-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(socket(None, &d), None, "no data, no question");
        std::fs::write(d.join("daily.cld"), "").unwrap();
        assert_eq!(socket(None, &d), Some(SOCKET.into()));
        assert_eq!(
            socket(Some("/s".into()), Path::new("/x")),
            Some("/s".into())
        );
    }
}
