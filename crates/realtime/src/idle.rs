//! Idle lock policy. Pure: activity and time in, "lock now" out.
//! Config: /etc/wolos/idle.json `{ "lock_after_secs": 300 }` (0 = never).

pub const DEFAULT_FILE: &str = "/etc/wolos/idle.json";
pub const DEFAULT_LOCK_AFTER_MS: u64 = 300_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    lock_after_ms: u64,
    last_activity: u64,
    fired: bool,
}

impl Policy {
    /// `lock_after_ms == 0` disables idle lock.
    pub fn new(lock_after_ms: u64, now: u64) -> Self {
        Self {
            lock_after_ms,
            last_activity: now,
            fired: false,
        }
    }
    pub fn lock_after_ms(&self) -> u64 {
        self.lock_after_ms
    }
    /// Input happened: restart the idle period.
    pub fn activity(&mut self, now: u64) {
        self.last_activity = now;
        self.fired = false;
    }
    /// Periodic check. True exactly once per idle period, when the lock command must run.
    /// An inhibitor or an existing lock restarts the period instead.
    pub fn tick(&mut self, now: u64, locked: bool, inhibited: bool) -> bool {
        if self.lock_after_ms == 0 {
            return false;
        }
        if locked || inhibited {
            self.activity(now);
            return false;
        }
        if self.fired || now.saturating_sub(self.last_activity) < self.lock_after_ms {
            return false;
        }
        self.fired = true;
        true
    }
}

/// Timeout from the config file contents; missing or bad -> default (with one log line if bad).
pub fn parse_lock_after_ms(json: Option<&str>) -> u64 {
    #[derive(serde::Deserialize)]
    struct File {
        lock_after_secs: Option<u64>,
    }
    let Some(json) = json else {
        return DEFAULT_LOCK_AFTER_MS;
    };
    match serde_json::from_str::<File>(json) {
        Ok(f) => f
            .lock_after_secs
            .map_or(DEFAULT_LOCK_AFTER_MS, |s| s.saturating_mul(1000)),
        Err(e) => {
            eprintln!("wol-compositor: idle config ignored: {e}");
            DEFAULT_LOCK_AFTER_MS
        }
    }
}

pub fn load_lock_after_ms() -> u64 {
    parse_lock_after_ms(std::fs::read_to_string(DEFAULT_FILE).ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_once_after_timeout() {
        let mut p = Policy::new(1000, 0);
        assert!(!p.tick(999, false, false));
        assert!(p.tick(1000, false, false));
        assert!(!p.tick(5000, false, false), "only once per idle period");
    }

    #[test]
    fn activity_restarts_period() {
        let mut p = Policy::new(1000, 0);
        p.activity(800);
        assert!(!p.tick(1500, false, false));
        assert!(p.tick(1800, false, false));
        p.activity(2000);
        assert!(p.tick(3000, false, false), "fires again after new activity");
    }

    #[test]
    fn inhibitor_or_lock_restarts_period() {
        for (locked, inhibited) in [(true, false), (false, true)] {
            let mut p = Policy::new(1000, 0);
            assert!(!p.tick(2000, locked, inhibited));
            assert!(
                !p.tick(2999, false, false),
                "period counts from when the block ended"
            );
            assert!(p.tick(3000, false, false));
        }
    }

    #[test]
    fn zero_disables() {
        let mut p = Policy::new(0, 0);
        assert!(!p.tick(u64::MAX / 2, false, false));
    }

    #[test]
    fn config_parsing() {
        let cases = [
            (None, DEFAULT_LOCK_AFTER_MS),
            (Some(r#"{"lock_after_secs": 60}"#), 60_000),
            (Some(r#"{"lock_after_secs": 0}"#), 0),
            (Some(r#"{}"#), DEFAULT_LOCK_AFTER_MS),
            (Some("{nope"), DEFAULT_LOCK_AFTER_MS),
        ];
        for (json, want) in cases {
            assert_eq!(parse_lock_after_ms(json), want, "{json:?}");
        }
    }
}
