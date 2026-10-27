//! Event bus client: `subscribe` on the wold socket, then `shared::events::Event` per line.
use shared::events::Event;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

/// Parse a stream of event lines; bad lines are skipped.
pub fn parse_stream(r: impl BufRead) -> impl Iterator<Item = Event> {
    r.lines()
        .map_while(Result::ok)
        .filter_map(|l| Event::parse(&l))
}

/// Subscribe on `socket` (normally `shared::jobs::SOCKET`).
pub fn subscribe(socket: &Path) -> std::io::Result<impl Iterator<Item = Event>> {
    let mut s = UnixStream::connect(socket)?;
    s.write_all(b"subscribe\n")?;
    Ok(parse_stream(BufReader::new(s)))
}

/// Publisher side: write one event line.
pub fn publish(w: &mut impl Write, e: &Event) -> std::io::Result<()> {
    w.write_all(e.to_line().as_bytes())
}

/// Publishes events to wold (`publish <json>` request, one connection per event: events are rare).
/// Falls back to `fallback` (normally stdout) when wold is not reachable.
pub struct Publisher<W: Write> {
    socket: std::path::PathBuf,
    fallback: W,
}

impl<W: Write> Publisher<W> {
    pub fn new(socket: impl Into<std::path::PathBuf>, fallback: W) -> Self {
        Self {
            socket: socket.into(),
            fallback,
        }
    }

    /// True when wold received it; false when it went to the fallback.
    pub fn send(&mut self, e: &Event) -> bool {
        let line =
            shared::jobs::JobRequest::Publish(e.to_line().trim_end().to_owned()).line() + "\n";
        let sent = UnixStream::connect(&self.socket)
            .and_then(|mut s| s.write_all(line.as_bytes()))
            .is_ok();
        if !sent {
            let _ = publish(&mut self.fallback, e);
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_then_parse_skips_garbage() {
        let mut buf = Vec::new();
        publish(&mut buf, &Event::Workspace { name: "1".into() }).unwrap();
        buf.extend_from_slice(b"junk\n");
        publish(
            &mut buf,
            &Event::CloseWindow {
                address: "a".into(),
            },
        )
        .unwrap();
        let got: Vec<Event> = parse_stream(&buf[..]).collect();
        assert_eq!(
            got,
            vec![
                Event::Workspace { name: "1".into() },
                Event::CloseWindow {
                    address: "a".into()
                }
            ]
        );
    }

    #[test]
    fn subscribe_sends_request() {
        use std::io::Read;
        let dir = std::env::temp_dir().join(format!("wol-bus-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("b.sock");
        let _ = std::fs::remove_file(&path);
        let l = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let t = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut req = [0u8; 10];
            c.read_exact(&mut req).unwrap();
            assert_eq!(&req, b"subscribe\n");
            publish(
                &mut c,
                &Event::Theme {
                    id: "wol-dark".into(),
                },
            )
            .unwrap();
        });
        let got: Vec<Event> = subscribe(&path).unwrap().collect();
        t.join().unwrap();
        assert_eq!(
            got,
            vec![Event::Theme {
                id: "wol-dark".into()
            }]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn publisher_sends_request_or_falls_back() {
        use std::io::Read;
        let e = Event::Workspace { name: "2".into() };
        let mut p = Publisher::new("/nonexistent/wold.sock", Vec::new());
        assert!(!p.send(&e));
        assert_eq!(p.fallback, e.to_line().into_bytes());

        let dir = std::env::temp_dir().join(format!("wol-pub-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("w.sock");
        let _ = std::fs::remove_file(&path);
        let l = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let mut p = Publisher::new(&path, Vec::new());
        assert!(p.send(&e));
        let mut got = String::new();
        l.accept().unwrap().0.read_to_string(&mut got).unwrap();
        assert_eq!(
            shared::jobs::JobRequest::parse(&got),
            Some(shared::jobs::JobRequest::Publish(
                e.to_line().trim_end().into()
            ))
        );
        assert!(p.fallback.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
