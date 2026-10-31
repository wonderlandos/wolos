//! One app's Memory Saver life: in use -> background grace period -> saving -> (App Nap) ->
//! restored the moment it is used again. Pure: facts and a clock in, an action out.
use shared::memsaver::AppState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Background this long before memory is given back.
    pub delay_ms: u64,
    /// App Nap: hidden this long (and idle) before the app is paused.
    pub nap_after_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            delay_ms: 30_000,
            nap_after_ms: 10 * 60_000,
        }
    }
}

/// What the session and wold know about the app right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub enabled: bool,
    pub nap: bool,
    /// Frontmost, or a window of it is visible.
    pub in_use: bool,
    pub excluded: bool,
    pub playing: bool,
    pub game: bool,
    /// Has a window that is mapped but hidden (minimized, other workspace). App Nap needs it.
    pub hidden: bool,
    /// No CPU or I/O worth mentioning over the last minute.
    pub idle: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Active,
    Waiting {
        since: u64,
    },
    Saving {
        since: u64,
    },
    Napping {
        since: u64,
    },
    /// Saving gave up under pressure; nothing more until the app is used again.
    Released {
        since: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// Push cold pages to zram (memory.reclaim) and start the role ceilings.
    Save,
    /// Freeze the app (cgroup.freeze).
    Nap,
    /// Thaw, drop every ceiling; `prefetch`: bring compressed pages back ahead of use.
    Restore {
        prefetch: bool,
    },
}

/// Why the app is left alone, if it is.
pub fn guard(f: &Facts) -> Option<AppState> {
    if f.excluded {
        Some(AppState::Excluded)
    } else if f.game {
        Some(AppState::Game)
    } else if f.playing {
        Some(AppState::Playing)
    } else {
        None
    }
}

impl Phase {
    pub fn state(self) -> AppState {
        match self {
            Phase::Active => AppState::Active,
            Phase::Waiting { .. } => AppState::Waiting,
            Phase::Saving { .. } => AppState::Saving,
            Phase::Napping { .. } => AppState::Napping,
            Phase::Released { .. } => AppState::Released,
        }
    }

    /// Background start (`since` of every non-active phase).
    fn since(self) -> Option<u64> {
        match self {
            Phase::Active => None,
            Phase::Waiting { since }
            | Phase::Saving { since }
            | Phase::Napping { since }
            | Phase::Released { since } => Some(since),
        }
    }

    pub fn step(self, f: &Facts, now: u64, c: &Config) -> (Phase, Action) {
        let hands_off = !f.enabled || f.in_use || guard(f).is_some();
        if hands_off {
            let act = match self {
                Phase::Saving { .. } | Phase::Napping { .. } | Phase::Released { .. } => {
                    Action::Restore { prefetch: f.in_use }
                }
                Phase::Active | Phase::Waiting { .. } => Action::None,
            };
            return (Phase::Active, act);
        }
        let since = self.since().unwrap_or(now);
        let away = now.saturating_sub(since);
        match self {
            Phase::Active => (Phase::Waiting { since: now }, Action::None),
            Phase::Waiting { .. } if away >= c.delay_ms => (Phase::Saving { since }, Action::Save),
            Phase::Saving { .. } if f.nap && f.hidden && f.idle && away >= c.nap_after_ms => {
                (Phase::Napping { since }, Action::Nap)
            }
            p => (p, Action::None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bg() -> Facts {
        Facts {
            enabled: true,
            hidden: true,
            ..Facts::default()
        }
    }

    /// Replay (time, facts) pairs; returns the actions taken (None skipped) and the end phase.
    fn replay(steps: &[(u64, Facts)]) -> (Vec<(u64, Action)>, Phase) {
        let c = Config::default();
        let mut p = Phase::Active;
        let mut out = Vec::new();
        for (t, f) in steps {
            let (np, a) = p.step(f, *t, &c);
            if a != Action::None {
                out.push((*t, a));
            }
            p = np;
        }
        (out, p)
    }

    #[test]
    fn background_apps_give_memory_back_after_the_delay_and_get_it_back_on_focus() {
        let used = Facts {
            in_use: true,
            ..bg()
        };
        let (acts, end) = replay(&[
            (0, used),
            (1_000, bg()),
            (30_000, bg()),
            (31_000, bg()),
            (40_000, bg()),
            (41_000, used),
        ]);
        assert_eq!(
            acts,
            vec![
                (31_000, Action::Save),
                (41_000, Action::Restore { prefetch: true })
            ]
        );
        assert_eq!(end, Phase::Active);
    }

    #[test]
    fn coming_back_inside_the_delay_costs_nothing() {
        let used = Facts {
            in_use: true,
            ..bg()
        };
        let (acts, _) = replay(&[(0, bg()), (29_000, bg()), (29_500, used), (90_000, used)]);
        assert!(acts.is_empty(), "{acts:?}");
    }

    #[test]
    fn excluded_playing_and_games_are_never_touched_and_are_restored_if_saved() {
        for (guarded, state) in [
            (
                Facts {
                    excluded: true,
                    ..bg()
                },
                AppState::Excluded,
            ),
            (
                Facts {
                    playing: true,
                    ..bg()
                },
                AppState::Playing,
            ),
            (Facts { game: true, ..bg() }, AppState::Game),
        ] {
            assert_eq!(guard(&guarded), Some(state));
            let (acts, _) = replay(&[(0, guarded), (60_000, guarded), (600_000, guarded)]);
            assert!(acts.is_empty(), "{state:?}: {acts:?}");
            // Saving, then starts playing in the background: restored, but no prefetch needed.
            let (acts, end) = replay(&[(0, bg()), (31_000, bg()), (32_000, guarded)]);
            assert_eq!(
                acts,
                vec![
                    (31_000, Action::Save),
                    (32_000, Action::Restore { prefetch: false })
                ]
            );
            assert_eq!(end, Phase::Active);
        }
    }

    #[test]
    fn turning_memory_saver_off_restores_everything() {
        let off = Facts {
            enabled: false,
            ..bg()
        };
        let (acts, _) = replay(&[(0, bg()), (30_000, bg()), (35_000, off), (99_000, off)]);
        assert_eq!(
            acts,
            vec![
                (30_000, Action::Save),
                (35_000, Action::Restore { prefetch: false })
            ]
        );
    }

    #[test]
    fn app_nap_needs_opt_in_hidden_idle_and_ten_minutes() {
        let nap = Facts {
            nap: true,
            idle: true,
            ..bg()
        };
        let min = 60_000;
        let (acts, end) = replay(&[(0, nap), (30_000, nap), (9 * min, nap), (10 * min, nap)]);
        assert_eq!(acts, vec![(30_000, Action::Save), (10 * min, Action::Nap)]);
        assert_eq!(end, Phase::Napping { since: 0 });
        for no in [
            Facts { nap: false, ..nap },
            Facts { idle: false, ..nap },
            Facts {
                hidden: false,
                ..nap
            },
        ] {
            let (acts, _) = replay(&[(0, no), (30_000, no), (60 * min, no)]);
            assert_eq!(acts, vec![(30_000, Action::Save)], "{no:?}");
        }
        // A napping app is thawed the moment it is used.
        let (acts, _) = replay(&[
            (0, nap),
            (30_000, nap),
            (10 * min, nap),
            (
                10 * min + 1,
                Facts {
                    in_use: true,
                    ..nap
                },
            ),
        ]);
        assert_eq!(
            acts.last(),
            Some(&(10 * min + 1, Action::Restore { prefetch: true }))
        );
    }

    #[test]
    fn released_apps_wait_to_be_used() {
        let c = Config::default();
        let p = Phase::Released { since: 0 };
        assert_eq!(p.step(&bg(), 99_000, &c), (p, Action::None));
        assert_eq!(p.state(), AppState::Released);
        let used = Facts {
            in_use: true,
            ..bg()
        };
        assert_eq!(
            p.step(&used, 99_001, &c),
            (Phase::Active, Action::Restore { prefetch: true })
        );
    }
}
