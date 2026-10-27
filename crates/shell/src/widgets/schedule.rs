//! One timer for every widget on the desktop. Deadlines sit on wall-clock multiples of each period
//! (a 60 s and a 300 s widget wake together on the 5-minute mark), entries due within `SLACK`
//! run in the same wakeup, and nothing is armed while the desktop is hidden.
use std::collections::BTreeMap;

/// Seconds: entries due this close to a wakeup run in it.
pub const SLACK: u64 = 2;

#[derive(Clone, Debug, Default)]
pub struct Schedule {
    /// id -> (period s, next due unix s)
    entries: BTreeMap<String, (u64, u64)>,
}

/// Next multiple of `period` after `now`.
pub fn aligned_next(now: u64, period: u64) -> u64 {
    let p = period.max(1);
    (now / p + 1) * p
}

impl Schedule {
    /// Adds or re-times `id`; period 0 removes it (load once).
    pub fn set(&mut self, id: &str, period: u64, now: u64) {
        if period == 0 {
            self.entries.remove(id);
        } else {
            self.entries
                .insert(id.to_string(), (period, aligned_next(now, period)));
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.entries.remove(id);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Seconds from `now` to the earliest deadline (0 = something is overdue).
    pub fn wait(&self, now: u64) -> Option<u64> {
        self.entries
            .values()
            .map(|(_, due)| due.saturating_sub(now))
            .min()
    }

    /// Ids due at `now` (within `SLACK`), each moved to its next deadline.
    pub fn take_due(&mut self, now: u64) -> Vec<String> {
        let mut out = Vec::new();
        for (id, (period, due)) in self.entries.iter_mut() {
            if *due <= now + SLACK {
                out.push(id.clone());
                *due = aligned_next(now.max(*due), *period);
            }
        }
        out
    }

    /// Wakeups an hour of this schedule costs (perf budget check).
    pub fn wakeups_per_hour(&self, start: u64) -> usize {
        let mut s = self.clone();
        let (mut now, end, mut n) = (start, start + 3600, 0);
        while let Some(w) = s.wait(now) {
            now += w;
            if now >= end {
                break;
            }
            s.take_due(now);
            n += 1;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadlines_align_to_the_wall_clock() {
        for (now, p, want) in [
            (0, 60, 60),
            (59, 60, 60),
            (60, 60, 120),
            (1001, 5, 1005),
            (7, 0, 8),
        ] {
            assert_eq!(aligned_next(now, p), want, "{now} {p}");
        }
    }

    #[test]
    fn widgets_due_together_share_one_wakeup() {
        let mut s = Schedule::default();
        // 20 s past a minute, 140 s past a 5-minute mark.
        let t0 = 1_000_000_040;
        s.set("clock", 60, t0);
        s.set("weather", 1800, t0);
        s.set("friends", 300, t0);
        assert_eq!(s.wait(t0), Some(40));
        assert_eq!(s.take_due(t0 + 40), vec!["clock"]);
        assert_eq!(s.take_due(t0 + 100), vec!["clock"]);
        // On the 5-minute mark the clock and the 300 s widget run in one wakeup.
        assert_eq!(s.take_due(t0 + 160), vec!["clock", "friends"]);
        assert_eq!(s.wait(t0 + 160), Some(60));
    }

    #[test]
    fn overdue_entries_run_once_then_realign() {
        let mut s = Schedule::default();
        s.set("system", 5, 100);
        assert_eq!(s.wait(100), Some(5));
        // Paused for a minute (fullscreen): one run on resume, not twelve.
        assert_eq!(s.wait(170), Some(0));
        assert_eq!(s.take_due(170), vec!["system"]);
        assert_eq!(s.wait(170), Some(5));
        assert!(s.take_due(170).is_empty());
    }

    #[test]
    fn load_once_widgets_are_never_scheduled() {
        let mut s = Schedule::default();
        s.set("static", 0, 0);
        assert!(s.is_empty());
        assert_eq!(s.wait(0), None);
        s.set("x", 60, 0);
        s.remove("x");
        assert!(s.is_empty());
    }

    /// Budget: a clock, a 5 s System widget and a 30 min Weather widget wake 719 times in the hour
    /// after `start` (0.2/s), the System cadence alone; the others ride along.
    #[test]
    fn coalesced_wakeups_stay_within_budget() {
        let mut s = Schedule::default();
        s.set("clock", 60, 0);
        s.set("system", 5, 0);
        s.set("weather", 1800, 0);
        s.set("friends", 300, 0);
        assert_eq!(s.wakeups_per_hour(0), 719);
        s.remove("system");
        assert_eq!(s.wakeups_per_hour(0), 59);
    }
}
