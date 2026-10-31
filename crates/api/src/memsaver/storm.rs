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

#[cfg(test)]
mod tests {
    use super::*;

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
