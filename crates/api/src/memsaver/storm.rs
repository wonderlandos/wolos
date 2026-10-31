//! Fork storms: a scope that sits at its pids limit and keeps asking is not an app doing work.
//! pids.events `max` counts refused forks. A build or a browser at the limit backs off; only a
//! runaway loop is refused thousands of times a tick, tick after tick.

/// Refused forks per tick (2 s) that count as a storm.
pub const REFUSED_PER_TICK: u64 = 2000;
/// Ticks in a row before the scope is killed.
pub const HOT_TICKS: u8 = 2;

#[derive(Debug, Default)]
pub struct Storm {
    seen: Option<u64>,
    hot: u8,
}

impl Storm {
    /// Feed pids.events `max` once a tick; true when the scope should die.
    pub fn step(&mut self, refused: u64) -> bool {
        let delta = refused.saturating_sub(self.seen.unwrap_or(refused));
        self.seen = Some(refused);
        self.hot = if delta >= REFUSED_PER_TICK {
            self.hot + 1
        } else {
            0
        };
        self.hot >= HOT_TICKS
    }
}

/// One app's OOM kills that count as a loop, over this many ticks. A browser that loses a tab or
/// two at its memory limit stays; something that refills the limit as fast as it dies does not.
pub const OOM_KILLS: u64 = 4;
pub const OOM_TICKS: usize = 5;

#[derive(Debug, Default)]
pub struct OomLoop {
    seen: Option<u64>,
    recent: [u64; OOM_TICKS],
    at: usize,
}

impl OomLoop {
    /// Feed memory.events `oom_kill` once a tick; true when the scope should die.
    pub fn step(&mut self, kills: u64) -> bool {
        let delta = kills.saturating_sub(self.seen.unwrap_or(kills));
        self.seen = Some(kills);
        self.recent[self.at] = delta;
        self.at = (self.at + 1) % OOM_TICKS;
        self.recent.iter().sum::<u64>() >= OOM_KILLS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oom(steps: &[u64]) -> Vec<bool> {
        let mut s = OomLoop::default();
        steps.iter().map(|k| s.step(*k)).collect()
    }

    #[test]
    fn repeated_oom_kills_are_a_loop() {
        assert_eq!(oom(&[0, 2, 3, 5]), [false, false, false, true]);
    }

    #[test]
    fn one_lost_tab_is_not_a_loop() {
        assert!(oom(&[0, 1, 1, 2, 2, 3, 3, 3, 3, 3, 4]).iter().all(|k| !k));
    }

    #[test]
    fn kills_from_before_we_watched_do_not_count() {
        assert_eq!(oom(&[50, 50, 50]), [false, false, false]);
    }

    fn run(steps: &[u64]) -> Vec<bool> {
        let mut s = Storm::default();
        steps.iter().map(|r| s.step(*r)).collect()
    }

    #[test]
    fn two_hot_ticks_kill() {
        assert_eq!(run(&[0, 5000, 10000]), [false, false, true]);
    }

    #[test]
    fn the_count_at_adoption_is_not_a_storm() {
        assert_eq!(run(&[900_000, 900_000, 900_100]), [false, false, false]);
    }

    #[test]
    fn a_calm_tick_resets() {
        assert_eq!(
            run(&[0, 5000, 5000 + 10, 10000, 15000]),
            [false, false, false, false, true]
        );
    }

    #[test]
    fn a_slow_retry_loop_lives() {
        let steps: Vec<u64> = (0..1000).map(|i| i * (REFUSED_PER_TICK - 1)).collect();
        assert!(run(&steps).iter().all(|k| !k));
    }
}
