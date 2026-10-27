//! Event bus: `subscribe` connections receive every published `shared::events::Event` line.
use shared::events::Event;
use std::io::Write;
use std::sync::Mutex;

pub struct Bus<W: Write + Send> {
    subscribers: Mutex<Vec<W>>,
}

impl<W: Write + Send> Default for Bus<W> {
    fn default() -> Self {
        Self {
            subscribers: Mutex::new(Vec::new()),
        }
    }
}

impl<W: Write + Send> Bus<W> {
    pub fn subscribe(&self, w: W) {
        self.lock().push(w);
    }

    /// Fan out to every subscriber; dead ones are dropped. Returns live subscriber count.
    pub fn publish(&self, e: &Event) -> usize {
        let line = e.to_line();
        let mut subs = self.lock();
        subs.retain_mut(|w| {
            w.write_all(line.as_bytes())
                .and_then(|()| w.flush())
                .is_ok()
        });
        subs.len()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<W>> {
        self.subscribers.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;

    #[test]
    fn fans_out_and_drops_dead_subscribers() {
        let bus = Bus::default();
        let (a, a_peer) = UnixStream::pair().unwrap();
        let (b, b_peer) = UnixStream::pair().unwrap();
        bus.subscribe(a);
        bus.subscribe(b);
        drop(b_peer);
        let e = Event::Workspace { name: "2".into() };
        assert_eq!(bus.publish(&e), 1);
        let mut line = String::new();
        BufReader::new(a_peer).read_line(&mut line).unwrap();
        assert_eq!(Event::parse(&line), Some(e));
    }
}
