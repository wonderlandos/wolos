// SPDX-License-Identifier: GPL-2.0-or-later
//! The Memory Saver loop: adopt app processes into scopes, run the policy per app and apply its
//! actions. One `tick` every couple of seconds and on every focus change; everything it touches
//! goes through [`Sys`].
use super::storm::{OomLoop, Storm};
use super::{Sys, procs};
use shared::memsaver::{AppState, AppStatus, Snapshot, Status};
use std::collections::{BTreeMap, HashMap};
use wol_core::memsaver::app::{self, Action, Config, Facts, Phase};

/// Ask systemd again after this long if the scope did not show up.
const ADOPT_RETRY_MS: u64 = 10_000;
/// App Nap: idle means under 1 % of a core for a minute.
const IDLE_CPU: f32 = 1.0;
const IDLE_MS: u64 = 60_000;

struct App {
    id: String,
    /// Has shown a window: only those are ever saved (helpers started at login are left alone).
    windowed: bool,
    phase: Phase,
    games: HashMap<i32, bool>,
    before: u64,
    cpu: Option<(u64, u64)>,
    last_busy: u64,
    storm: Storm,
    oom: OomLoop,
}

struct Session {
    uid: u32,
    scope: String,
}

pub struct Engine<S: Sys> {
    pub sys: S,
    cfg: Config,
    session: Option<Session>,
    snap: Snapshot,
    apps: BTreeMap<String, App>,
    /// pid -> (asked at, scope cgroup, app id)
    asked: HashMap<i32, (u64, String, String)>,
    slice_storm: Storm,
}

fn pct(prev: Option<(u64, u64)>, now_ms: u64, usec: u64) -> f32 {
    match prev {
        Some((t, u)) if now_ms > t => {
            (usec.saturating_sub(u) as f32) / ((now_ms - t) as f32) / 10.0
        }
        _ => 0.0,
    }
}

impl<S: Sys> Engine<S> {
    pub fn new(sys: S, cfg: Config) -> Self {
        Engine {
            sys,
            cfg,
            session: None,
            snap: Snapshot::default(),
            apps: BTreeMap::new(),
            asked: HashMap::new(),
            slice_storm: Storm::default(),
        }
    }

    /// The session agent connected: its login session is the one whose apps we adopt.
    pub fn connect(&mut self, uid: u32, pid: i32) -> bool {
        let Some(cg) = self.sys.cgroup(pid) else {
            return false;
        };
        let Some((u, scope)) = procs::session_scope(&cg) else {
            return false;
        };
        if u != uid {
            return false;
        }
        self.session = Some(Session {
            uid,
            scope: scope.to_owned(),
        });
        true
    }

    pub fn snapshot(&mut self, s: Snapshot) {
        self.snap = s;
    }

    /// Frontmost changed (wold's QoS path sees it first): the next tick restores it.
    pub fn focus(&mut self, pid: i32) {
        self.snap.front = pid;
    }

    /// The agent went away: give every app its memory back and forget the session.
    pub fn disconnect(&mut self, now: u64) -> Status {
        self.snap = Snapshot::default();
        let st = self.tick(now);
        self.session = None;
        st
    }

    pub fn tick(&mut self, now: u64) -> Status {
        if self.session.is_some() && self.snap.enabled {
            self.adopt(now);
        }
        self.discover();
        let keys: Vec<String> = self.apps.keys().cloned().collect();
        for scope in keys {
            if !self.sys.exists(&scope) {
                self.apps.remove(&scope);
                continue;
            }
            self.run_app(&scope, now);
        }
        self.run_slice();
        self.status()
    }

    /// Several apps can fill the user's slice with none at its own limit: the refused forks then
    /// count on the slice, and the session is refused too. The biggest app goes.
    fn run_slice(&mut self) {
        let Some(s) = &self.session else { return };
        let slice = format!("/user.slice/user-{}.slice", s.uid);
        let storm = self.slice_storm.step(self.sys.refused(&slice));
        if !storm || self.sys.tasks(&slice) < procs::USER_TASKS * 95 / 100 {
            return;
        }
        if let Some(big) = self.apps.keys().max_by_key(|a| self.sys.tasks(a)) {
            eprintln!("wold: user slice full, killing {big}");
            self.sys.kill(big);
        }
    }

    /// Windows not in a scope of ours yet: their whole process tree goes into a new one. The
    /// tree, not the cgroup: Chromium and Electron move their browser process into a user manager
    /// scope on their own and leave the zygote and renderers in the session.
    fn adopt(&mut self, now: u64) {
        let Some(s) = &self.session else { return };
        let (uid, scope) = (s.uid, s.scope.clone());
        self.asked
            .retain(|_, (t, _, _)| now.saturating_sub(*t) < 6 * ADOPT_RETRY_MS);
        let home = format!("/user.slice/user-{uid}.slice/");
        let sys = &self.sys;
        let movable = |pid: i32| {
            sys.cgroup(pid)
                .is_some_and(|c| c.starts_with(&home) && !c.contains("/app-wol-"))
                && sys.owner_and_parent(pid).map(|o| o.0) == Some(uid)
                && !procs::NEVER.contains(&sys.comm(pid).as_str())
        };
        let want: Vec<(i32, String)> = self
            .snap
            .windows
            .iter()
            .filter(|w| movable(w.pid))
            .map(|w| (w.pid, w.app.clone()))
            .collect();
        if want.is_empty() {
            return;
        }
        let parents = sys.parents();
        for (pid, app) in want {
            let recent = self
                .asked
                .get(&pid)
                .is_some_and(|(t, _, _)| now.saturating_sub(*t) < ADOPT_RETRY_MS);
            if recent {
                continue;
            }
            let pids: Vec<i32> = procs::tree(pid, &parents)
                .into_iter()
                .filter(|p| movable(*p))
                .collect();
            let name = procs::scope_name(&app, pid);
            let cg = format!("{home}{name}");
            let id = shared::memsaver::app_id(&app);
            if sys.adopt(&procs::adopt_args(&name, uid, &scope, &pids, &id)) {
                eprintln!("wold: memsaver adopted {id} ({} processes)", pids.len());
            }
            self.asked.insert(pid, (now, cg, id));
        }
    }

    /// App scopes we know of: windows' scopes and the ones we asked systemd for.
    fn discover(&mut self) {
        let mut found: Vec<(String, String, bool)> = Vec::new();
        for w in &self.snap.windows {
            if let Some(cg) = self.sys.cgroup(w.pid)
                && let Some(scope) = procs::app_scope(&cg)
            {
                found.push((scope.to_owned(), w.app.clone(), true));
            }
        }
        for (_, cg, id) in self.asked.values() {
            if self.sys.exists(cg) {
                found.push((cg.clone(), id.clone(), false));
            }
        }
        for (scope, id, windowed) in found {
            let app = self.apps.entry(scope.clone()).or_insert_with(|| {
                // Left over from an earlier wold: not frozen.
                let _ = self.sys.freeze(&scope, false);
                App {
                    id: shared::memsaver::app_id(&id),
                    windowed: false,
                    phase: Phase::Active,
                    games: HashMap::new(),
                    before: 0,
                    cpu: None,
                    last_busy: 0,
                    storm: Storm::default(),
                    oom: OomLoop::default(),
                }
            });
            if windowed && !app.windowed {
                app.windowed = true;
                app.id = shared::memsaver::app_id(&id);
            }
        }
    }

    fn run_app(&mut self, scope: &str, now: u64) {
        let Some(mut app) = self.apps.remove(scope) else {
            return;
        };
        let storm = app.storm.step(self.sys.refused(scope));
        let oom = app.oom.step(self.sys.oom_kills(scope));
        if storm || oom {
            let what = if storm { "fork storm" } else { "OOM loop" };
            eprintln!("wold: {what} in {}, killing its scope", app.id);
            self.sys.kill(scope);
            self.apps.insert(scope.to_owned(), app);
            return;
        }
        if app.windowed {
            let pids = self.sys.procs(scope);
            let usec = self.sys.cpu_usec(scope);
            if pct(app.cpu, now, usec) > IDLE_CPU || app.cpu.is_none() {
                app.last_busy = now;
            }
            app.cpu = Some((now, usec));
            let facts = self.facts(&mut app, &pids, now);
            let (phase, action) = app.phase.step(&facts, now, &self.cfg);
            app.phase = phase;
            self.apply(scope, &mut app, action, pids);
        }
        self.apps.insert(scope.to_owned(), app);
    }

    fn facts(&self, app: &mut App, pids: &[i32], now: u64) -> Facts {
        for p in pids {
            if !app.games.contains_key(p) {
                app.games.insert(*p, self.sys.is_game(*p));
            }
        }
        app.games.retain(|p, _| pids.contains(p));
        let s = &self.snap;
        let mine = |p: &i32| pids.contains(p);
        let visible = s.windows.iter().any(|w| w.visible && mine(&w.pid));
        Facts {
            enabled: s.enabled,
            nap: s.nap,
            in_use: mine(&s.front) || visible,
            excluded: s.exclude.contains(&app.id),
            playing: s.playing.iter().any(mine),
            game: app.games.values().any(|g| *g),
            hidden: !visible,
            idle: now.saturating_sub(app.last_busy) >= IDLE_MS,
            // No signal from the runtime yet (ADR 0042): the saver reclaims as before.
            runtime_discarded: false,
        }
    }

    fn apply(&mut self, scope: &str, app: &mut App, action: Action, pids: Vec<i32>) {
        match action {
            Action::None => {}
            Action::Save => {
                app.before = self.sys.current(scope);
                self.sys.give_back(scope, app.before, pids);
                eprintln!(
                    "wold: memsaver saving {} ({} MiB)",
                    app.id,
                    app.before >> 20
                );
            }
            Action::Nap => {
                self.sys.freeze(scope, true);
                let held = self.sys.current(scope);
                self.sys.give_back(scope, held, pids);
                eprintln!("wold: memsaver napping {}", app.id);
            }
            Action::Restore => {
                self.sys.cancel(scope);
                self.sys.freeze(scope, false);
            }
        }
    }

    fn status(&self) -> Status {
        let mut st = Status {
            enabled: self.snap.enabled,
            ..Status::default()
        };
        for (scope, app) in self.apps.iter().filter(|(_, a)| a.windowed) {
            let memory = self.sys.current(scope);
            let pids = self.sys.procs(scope);
            let state = if !self.snap.enabled {
                AppState::Active
            } else if self.snap.exclude.contains(&app.id) {
                AppState::Excluded
            } else if app.games.values().any(|g| *g) {
                AppState::Game
            } else if app.phase == Phase::Active
                && self.snap.playing.iter().any(|p| pids.contains(p))
            {
                AppState::Playing
            } else {
                app.phase.state()
            };
            let swap = self.sys.swapped(&pids);
            let saved = if matches!(app.phase, Phase::Saving { .. } | Phase::Napping { .. }) {
                app.before.saturating_sub(memory).max(swap)
            } else {
                0
            };
            st.saved += saved;
            st.apps.push(AppStatus {
                app: app.id.clone(),
                state,
                saved,
                memory,
                swap,
            });
        }
        st
    }
}

pub use app::Config as AppConfig;

#[cfg(test)]
mod tests;
