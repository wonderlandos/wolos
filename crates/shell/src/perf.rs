//! Hot-path helpers: change detection (no widget update when equal) and TTL snapshots per topic.
use std::time::{Duration, Instant};

/// Last value pushed to a widget. `set` returns true only when the value changed.
#[derive(Debug, Default)]
pub struct Changed<T> {
    last: Option<T>,
}

impl<T: PartialEq> Changed<T> {
    pub const fn new() -> Self {
        Changed { last: None }
    }
    pub fn set(&mut self, v: T) -> bool {
        if self.last.as_ref() == Some(&v) {
            return false;
        }
        self.last = Some(v);
        true
    }
}

/// Snapshot valid for `ttl`; `get` refreshes only when stale.
pub struct Cached<T> {
    ttl: Duration,
    at: Option<Instant>,
    value: Option<T>,
}

impl<T: Clone> Cached<T> {
    pub const fn new(ttl: Duration) -> Self {
        Cached {
            ttl,
            at: None,
            value: None,
        }
    }
    pub fn get_at(&mut self, now: Instant, refresh: impl FnOnce() -> T) -> T {
        match (self.at, &self.value) {
            (Some(at), Some(v)) if now.duration_since(at) < self.ttl => v.clone(),
            _ => {
                let v = refresh();
                self.at = Some(now);
                self.value = Some(v.clone());
                v
            }
        }
    }
    pub fn get(&mut self, refresh: impl FnOnce() -> T) -> T {
        self.get_at(Instant::now(), refresh)
    }
    pub fn invalidate(&mut self) {
        self.at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_only_on_new_value() {
        let mut c = Changed::new();
        assert!(c.set("a"));
        assert!(!c.set("a"));
        assert!(c.set("b"));
    }

    #[test]
    fn cached_refreshes_after_ttl() {
        let mut c = Cached::new(Duration::from_secs(2));
        let t0 = Instant::now();
        let mut calls = 0;
        for dt in [0, 1, 1999] {
            c.get_at(t0 + Duration::from_millis(dt), || {
                calls += 1;
                7
            });
        }
        assert_eq!(calls, 1);
        c.get_at(t0 + Duration::from_secs(2), || {
            calls += 1;
            8
        });
        assert_eq!(calls, 2);
        c.invalidate();
        assert_eq!(c.get_at(t0 + Duration::from_secs(2), || 9), 9);
    }
}
