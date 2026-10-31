//! The bridge to wold for system changes (ADR 0021): one `shared::packages::PackageCall` per
//! connection, one `shared::portal::Answer` back. libwopkg never runs anything as root.
use crate::context::Context;
use crate::ffi::{Error, Result, Status};
use shared::packages::PackageCall;
use shared::portal::{Answer, Code};
use std::io::{BufRead, BufReader, Write};

pub fn call(ctx: &Context, c: &PackageCall) -> Result<()> {
    c.validate().map_err(Error::invalid)?;
    let mut s = std::os::unix::net::UnixStream::connect(&ctx.wold).map_err(|_| {
        Error::unavailable("WolOS's system service is not running, so the system was not changed.")
    })?;
    s.write_all(format!("call {}\n", c.encode()).as_bytes())
        .map_err(|_| Error::unavailable("WolOS's system service did not take the request."))?;
    let mut line = String::new();
    BufReader::new(&s)
        .read_line(&mut line)
        .map_err(|_| Error::unavailable("WolOS's system service stopped answering."))?;
    let a = Answer::parse(&line);
    if a.ok {
        return Ok(());
    }
    let st = match a.code {
        Some(Code::PermissionDenied) => Status::PermissionDenied,
        Some(Code::InvalidArgument) => Status::InvalidArgument,
        Some(Code::Unavailable) => Status::Unavailable,
        _ => Status::Internal,
    };
    Err(Error::new(
        st,
        a.message
            .unwrap_or_else(|| "The system was not changed.".into()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::packages::Verb;

    #[test]
    fn sends_one_call_and_reads_the_answer() {
        let d = crate::testutil::temp("wold");
        let sock = d.join("wold.sock");
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let t = std::thread::spawn(move || {
            let mut got = Vec::new();
            for answer in [
                Answer::ok(),
                Answer::err(
                    Code::PermissionDenied,
                    "wopkg is not allowed to install system apps.",
                ),
            ] {
                let (c, _) = l.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&c).read_line(&mut line).unwrap();
                got.push(line);
                (&c).write_all(answer.to_line().as_bytes()).unwrap();
            }
            got
        });
        let mut ctx = crate::appimage::tests::ctx(&d);
        ctx.wold = sock;
        let c = PackageCall::Change {
            verb: Verb::Install,
            names: vec!["firefox".into()],
        };
        call(&ctx, &c).unwrap();
        let e = call(&ctx, &c).unwrap_err();
        assert_eq!(e.status, Status::PermissionDenied);
        assert!(e.message.contains("not allowed"));
        let got = t.join().unwrap();
        assert_eq!(got[0], format!("call {}\n", c.encode()));
        let bad = PackageCall::Change {
            verb: Verb::Install,
            names: vec!["-x".into()],
        };
        assert_eq!(
            call(&ctx, &bad).unwrap_err().status,
            Status::InvalidArgument,
            "checked before sending"
        );
    }
}
