//! Socket glue for wold: one request line per connection; `subscribe` keeps the connection.
use crate::bus::Bus;
use crate::control::{JobTable, handle};
use shared::events::Event;
use shared::jobs::JobRequest;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

/// Serve one control connection. `uid` comes from peer credentials.
pub fn serve(stream: UnixStream, uid: Option<u32>, jobs: &dyn JobTable, bus: &Bus<UnixStream>) {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let mut line = String::new();
    let mut reader = BufReader::new(stream.try_clone().expect("clone socket"));
    if reader
        .by_ref()
        .take(64 * 1024)
        .read_line(&mut line)
        .is_err()
    {
        return;
    }
    let req = JobRequest::parse(&line);
    match &req {
        Some(JobRequest::Subscribe) => {
            let _ = stream.set_read_timeout(None);
            bus.subscribe(stream);
        }
        Some(JobRequest::Publish(json)) => {
            if let Some(e) = Event::parse(json) {
                bus.publish(&e);
            }
        }
        _ => {
            let mut w = &stream;
            let _ = writeln!(w, "{}", handle(req.as_ref(), uid, jobs));
        }
    }
}

/// Bind a socket path (stale file removed) with mode `mode`.
pub fn bind(path: &Path, mode: u32) -> std::io::Result<UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::remove_file(path);
    let l = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(l)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::jobs::JobStatus;

    struct Empty;
    impl JobTable for Empty {
        fn list(&self) -> Vec<JobStatus> {
            vec![]
        }
        fn start(&self, _: &str) -> Option<Option<String>> {
            None
        }
        fn stop(&self, _: &str) -> Option<Option<String>> {
            None
        }
    }

    fn call(line: &str, bus: &Bus<UnixStream>) -> UnixStream {
        let (srv, mut cli) = UnixStream::pair().unwrap();
        cli.write_all(line.as_bytes()).unwrap();
        serve(srv, Some(1000), &Empty, bus);
        cli
    }

    #[test]
    fn list_subscribe_publish() {
        let bus = Bus::default();
        let mut out = String::new();
        call("list\n", &bus).read_to_string(&mut out).unwrap();
        assert_eq!(out, "[]\n");
        let sub = call("subscribe\n", &bus);
        assert_eq!(bus.len(), 1);
        let e = Event::Theme {
            id: "wol-dark".into(),
        };
        drop(call(&format!("publish {}", e.to_line()), &bus));
        let mut line = String::new();
        BufReader::new(sub).read_line(&mut line).unwrap();
        assert_eq!(Event::parse(&line), Some(e));
        drop(call("publish {bad}\n", &bus));
    }
}
