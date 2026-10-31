//! Adaptive memory ceiling for one role group of a background app (memory.high). The hysteresis
//! follows memory-sentry (+8 MiB after 6 s busy at the ceiling, -4 MiB after 10 s quiet with
//! headroom, 8 s between changes); on Linux the group's memory pressure (PSI) also counts, so a
//! saved app can never thrash: pressure raises the ceiling, high pressure lets go of it.

const MIB: u64 = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tune {
    /// Per process, bytes.
    pub raise: u64,
    pub lower: u64,
    /// Group CPU, percent of one core.
    pub busy_cpu: f32,
    pub busy_ms: u64,
    pub quiet_cpu: f32,
    pub quiet_ms: u64,
    /// Per process: lower only with at least this much room under the ceiling.
    pub headroom: u64,
    pub cooldown_ms: u64,
    /// memory.pressure `some avg10` (percent): raise at once (cooldown still applies).
    pub psi_raise: f32,
    /// ... and let go of the ceiling entirely.
    pub psi_release: f32,
}

impl Default for Tune {
    fn default() -> Self {
        Tune {
            raise: 8 * MIB,
            lower: 4 * MIB,
            busy_cpu: 10.0,
            busy_ms: 6_000,
            quiet_cpu: 3.0,
            quiet_ms: 10_000,
            headroom: 6 * MIB,
            cooldown_ms: 8_000,
            psi_raise: 5.0,
            psi_release: 25.0,
        }
    }
}

/// One measurement of the group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub now_ms: u64,
    pub cpu: f32,
    /// memory.current
    pub current: u64,
    /// Processes in the group (>= 1 when sampled).
    pub procs: u32,
    pub psi_some10: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Hold,
    /// New group ceiling in bytes (write memory.high).
    Set(u64),
    /// Pressure: remove the ceiling (memory.high = max) until the app is used again.
    Release,
}

/// Per-process ceiling band and the timers of the hysteresis.
#[derive(Clone, Debug, PartialEq)]
pub struct Ceiling {
    pub per_proc: u64,
    pub min: u64,
    pub max: u64,
    busy_since: Option<u64>,
    quiet_since: Option<u64>,
    last_change: Option<u64>,
}

impl Ceiling {
    pub fn new(start: u64, max: u64) -> Ceiling {
        Ceiling {
            per_proc: start,
            min: start,
            max: max.max(start),
            busy_since: None,
            quiet_since: None,
            last_change: None,
        }
    }

    /// Group limit for `procs` processes.
    pub fn limit(&self, procs: u32) -> u64 {
        self.per_proc.saturating_mul(u64::from(procs.max(1)))
    }

    pub fn tick(&mut self, s: Sample, t: &Tune) -> Step {
        if s.psi_some10 >= t.psi_release {
            return Step::Release;
        }
        let n = u64::from(s.procs.max(1));
        let limit = self.limit(s.procs);
        let at_ceiling = s.current.saturating_add(n * MIB) >= limit;
        let busy = s.cpu >= t.busy_cpu && at_ceiling;
        let quiet = s.cpu <= t.quiet_cpu
            && s.psi_some10 < 1.0
            && limit.saturating_sub(s.current) >= t.headroom * n;
        self.busy_since = if busy {
            self.busy_since.or(Some(s.now_ms))
        } else {
            None
        };
        self.quiet_since = if quiet {
            self.quiet_since.or(Some(s.now_ms))
        } else {
            None
        };
        let cooled = self
            .last_change
            .is_none_or(|t0| s.now_ms.saturating_sub(t0) >= t.cooldown_ms);
        if !cooled {
            return Step::Hold;
        }
        let held = |since: Option<u64>, ms: u64| since.is_some_and(|t0| s.now_ms - t0 >= ms);
        let next = if s.psi_some10 >= t.psi_raise || held(self.busy_since, t.busy_ms) {
            (self.per_proc + t.raise).min(self.max)
        } else if held(self.quiet_since, t.quiet_ms) {
            self.per_proc.saturating_sub(t.lower).max(self.min)
        } else {
            return Step::Hold;
        };
        if next == self.per_proc {
            return Step::Hold;
        }
        self.per_proc = next;
        self.last_change = Some(s.now_ms);
        self.busy_since = None;
        self.quiet_since = None;
        Step::Set(self.limit(s.procs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(now_ms: u64, cpu: f32, current_mib: u64, psi: f32) -> Sample {
        Sample {
            now_ms,
            cpu,
            current: current_mib * MIB,
            procs: 1,
            psi_some10: psi,
        }
    }

    /// Drive a ceiling through samples every second; returns the steps that were not Hold.
    fn run(c: &mut Ceiling, samples: impl Iterator<Item = Sample>) -> Vec<(u64, Step)> {
        let t = Tune::default();
        samples
            .map(|x| (x.now_ms, c.tick(x, &t)))
            .filter(|(_, st)| *st != Step::Hold)
            .collect()
    }

    #[test]
    fn busy_at_the_ceiling_for_six_seconds_raises_by_eight() {
        let mut c = Ceiling::new(40 * MIB, 80 * MIB);
        let steps = run(&mut c, (0..=6).map(|i| s(i * 1000, 15.0, 40, 0.0)));
        assert_eq!(steps, vec![(6000, Step::Set(48 * MIB))]);
    }

    #[test]
    fn busy_below_the_ceiling_changes_nothing() {
        let mut c = Ceiling::new(40 * MIB, 80 * MIB);
        assert!(run(&mut c, (0..20).map(|i| s(i * 1000, 50.0, 20, 0.0))).is_empty());
    }

    #[test]
    fn cooldown_spaces_raises_eight_seconds_apart_up_to_the_max() {
        let mut c = Ceiling::new(40 * MIB, 56 * MIB);
        let steps = run(&mut c, (0..60).map(|i| s(i * 1000, 15.0, 100, 0.0)));
        // 6 s busy -> 48; busy again from 7 s, cooldown ends at 14 s -> 56; the max holds.
        assert_eq!(
            steps,
            vec![(6000, Step::Set(48 * MIB)), (14000, Step::Set(56 * MIB))]
        );
    }

    #[test]
    fn quiet_with_headroom_lowers_by_four_but_never_under_the_floor() {
        let mut c = Ceiling::new(40 * MIB, 80 * MIB);
        c.per_proc = 48 * MIB;
        let steps = run(&mut c, (0..60).map(|i| s(i * 1000, 1.0, 20, 0.0)));
        assert_eq!(
            steps,
            vec![(10000, Step::Set(44 * MIB)), (21000, Step::Set(40 * MIB))]
        );
        // Too little headroom (limit 40, using 36: 4 < 6) holds.
        let mut c = Ceiling::new(32 * MIB, 80 * MIB);
        c.per_proc = 40 * MIB;
        assert!(run(&mut c, (0..30).map(|i| s(i * 1000, 0.0, 36, 0.0))).is_empty());
    }

    #[test]
    fn memory_pressure_raises_at_once_and_high_pressure_lets_go() {
        let mut c = Ceiling::new(40 * MIB, 80 * MIB);
        let t = Tune::default();
        assert_eq!(c.tick(s(0, 0.0, 39, 6.0), &t), Step::Set(48 * MIB));
        assert_eq!(c.tick(s(1000, 0.0, 47, 6.0), &t), Step::Hold, "cooldown");
        assert_eq!(
            c.tick(s(2000, 0.0, 47, 30.0), &t),
            Step::Release,
            "never thrash"
        );
    }

    #[test]
    fn group_limits_scale_with_the_process_count() {
        let mut c = Ceiling::new(40 * MIB, 80 * MIB);
        let t = Tune::default();
        let mut x = s(0, 20.0, 200, 0.0);
        x.procs = 5;
        assert_eq!(c.limit(5), 200 * MIB);
        assert_eq!(c.tick(x, &t), Step::Hold);
        x.now_ms = 6000;
        assert_eq!(c.tick(x, &t), Step::Set(240 * MIB));
    }
}
