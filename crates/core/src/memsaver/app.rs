// SPDX-License-Identifier: BSD-2-Clause
//! One app's Memory Saver life: in use -> background grace period -> saving -> (App Nap) ->
//! back in use. Pure: facts and a clock in, an action out.
use shared::memsaver::AppState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Background this long before memory is given back.
    pub delay_ms: u64,
    /// App Nap: hidden this long (and idle) before the app is paused.
    pub nap_after_ms: u64,
}

/// ADR 0026: memory goes back 30 s after an app stops being in use.
pub const SAVE_AFTER_MS: u64 = 30_000;
/// ADR 0026: App Nap pauses an app hidden and idle for 10 minutes (only when switched on).
pub const NAP_AFTER_MS: u64 = 10 * 60_000;

impl Default for Config {
    fn default() -> Self {
        Config {
            delay_ms: SAVE_AFTER_MS,
            nap_after_ms: NAP_AFTER_MS,
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
    /// The app's own runtime already gave its background content back (the browser discarded its
    /// tabs, ADR 0042): the saver moves on to Saving without a reclaim pass of its own, which
    /// would only scan pages the app has freed. App Nap and Restore are unchanged.
    pub runtime_discarded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Active,
    Waiting { since: u64 },
    Saving { since: u64 },
    Napping { since: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// Give memory back, once: what the app faults back in is what it uses.
    Save,
    /// Freeze the app (cgroup.freeze) and give back what it refaulted.
    Nap,
    /// Thaw and stop giving back. Pages return as they are touched.
    Restore,
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
        }
    }

    /// Background start (`since` of every non-active phase).
    fn since(self) -> Option<u64> {
        match self {
            Phase::Active => None,
            Phase::Waiting { since } | Phase::Saving { since } | Phase::Napping { since } => {
                Some(since)
            }
        }
    }

    pub fn step(self, f: &Facts, now: u64, c: &Config) -> (Phase, Action) {
        let hands_off = !f.enabled || f.in_use || guard(f).is_some();
        if hands_off {
            let act = match self {
                Phase::Saving { .. } | Phase::Napping { .. } => Action::Restore,
                Phase::Active | Phase::Waiting { .. } => Action::None,
            };
            return (Phase::Active, act);
        }
        let since = self.since().unwrap_or(now);
        let away = now.saturating_sub(since);
        match self {
            Phase::Active => (Phase::Waiting { since: now }, Action::None),
            Phase::Waiting { .. } if away >= c.delay_ms && f.runtime_discarded => {
                (Phase::Saving { since }, Action::None)
            }
            Phase::Waiting { .. } if away >= c.delay_ms => (Phase::Saving { since }, Action::Save),
            Phase::Saving { .. } if f.nap && f.hidden && f.idle && away >= c.nap_after_ms => {
                (Phase::Napping { since }, Action::Nap)
            }
            // App Nap switched off, or a window of it shows again: thaw now (CORE2-1). Memory
            // Saver is still on, so the app waits the delay again before it gives memory back.
            Phase::Napping { .. } if !(f.nap && f.hidden) => {
                (Phase::Waiting { since: now }, Action::Restore)
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

    /// QA-ADR-19/QA-ADR-10: the phase table runs on the delays ADR 0026 publishes, and the record
    /// still says them: a change to either side fails here.
    #[test]
    fn phase_table_runs_on_the_adr_0026_delays() {
        assert_eq!((SAVE_AFTER_MS, NAP_AFTER_MS), (30 * 1000, 10 * 60 * 1000));
        let adr = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/adr/0026-memory-saver.md"
        ))
        .unwrap();
        assert!(
            adr.contains("30 s after an app stops being in use"),
            "ADR 0026 save delay"
        );
        assert!(
            adr.contains("freeze after 10 minutes hidden and idle"),
            "ADR 0026 nap delay"
        );
        assert!(adr.contains("off by default"), "ADR 0026 nap default");
        let nap = Facts {
            nap: true,
            idle: true,
            ..bg()
        };
        let (s, n) = (SAVE_AFTER_MS, NAP_AFTER_MS);
        let (acts, _) = replay(&[(0, nap), (s - 1, nap), (s, nap), (n - 1, nap), (n, nap)]);
        assert_eq!(acts, vec![(s, Action::Save), (n, Action::Nap)]);
        // Without the opt-in (`nap` false, as `memsaver::Prefs::default` has it) only Save happens.
        let (acts, _) = replay(&[(0, bg()), (s, bg()), (n, bg())]);
        assert_eq!(acts, vec![(s, Action::Save)]);
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
            vec![(31_000, Action::Save), (41_000, Action::Restore)]
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
            // Saving, then starts playing in the background: restored.
            let (acts, end) = replay(&[(0, bg()), (31_000, bg()), (32_000, guarded)]);
            assert_eq!(
                acts,
                vec![(31_000, Action::Save), (32_000, Action::Restore)]
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
            vec![(30_000, Action::Save), (35_000, Action::Restore)]
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
        assert_eq!(acts.last(), Some(&(10 * min + 1, Action::Restore)));
    }

    #[test]
    fn switching_app_nap_off_thaws_a_napping_app() {
        let nap = Facts {
            nap: true,
            idle: true,
            ..bg()
        };
        let off = Facts { nap: false, ..nap };
        let min = 60_000;
        let (acts, end) = replay(&[
            (0, nap),
            (30_000, nap),
            (10 * min, nap),
            (10 * min + 1, off),
            (10 * min + 2, off),
        ]);
        assert_eq!(
            acts,
            vec![
                (30_000, Action::Save),
                (10 * min, Action::Nap),
                (10 * min + 1, Action::Restore)
            ]
        );
        assert_eq!(
            end,
            Phase::Waiting {
                since: 10 * min + 1
            }
        );
        // Still in the background with Memory Saver on: memory is given back again after the delay.
        let (acts, _) = replay(&[
            (0, nap),
            (30_000, nap),
            (10 * min, nap),
            (10 * min + 1, off),
            (10 * min + 1 + 30_000, off),
        ]);
        assert_eq!(acts.last(), Some(&(10 * min + 1 + 30_000, Action::Save)));
    }
    #[test]
    fn runtime_discarded_skips_the_reclaim_but_not_nap_or_restore() {
        let gone = Facts {
            runtime_discarded: true,
            nap: true,
            idle: true,
            ..bg()
        };
        let used = Facts {
            in_use: true,
            ..gone
        };
        let min = 60_000;
        let (acts, end) = replay(&[(0, gone), (30_000, gone)]);
        assert!(acts.is_empty(), "no reclaim pass: {acts:?}");
        assert_eq!(end, Phase::Saving { since: 0 });
        assert_eq!(
            end.state(),
            AppState::Saving,
            "Settings still shows it saving"
        );
        let (acts, _) = replay(&[
            (0, gone),
            (30_000, gone),
            (10 * min, gone),
            (10 * min + 1, used),
        ]);
        assert_eq!(
            acts,
            vec![(10 * min, Action::Nap), (10 * min + 1, Action::Restore)]
        );
        // Default (no signal from the runtime): unchanged.
        assert!(!Facts::default().runtime_discarded);
    }
}
