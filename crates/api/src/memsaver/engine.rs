//! The Memory Saver loop: adopt app processes into scopes, keep each process in its role group,
//! run the policy per app and apply its actions. One `tick` every couple of seconds and on every
//! focus change; everything it touches goes through [`Sys`].
use super::{Sys, procs};
use shared::memsaver::{AppState, AppStatus, Snapshot, Status};
use std::collections::{BTreeMap, HashMap};
use wol_core::memsaver::app::{self, Action, Config, Facts, Phase};
use wol_core::memsaver::ceiling::{Ceiling, Sample, Step, Tune};
use wol_core::memsaver::role::{self, Role};

/// Ask systemd again after this long if the scope did not show up.
const ADOPT_RETRY_MS: u64 = 10_000;
/// App Nap: idle means under 1 % of a core for a minute.
const IDLE_CPU: f32 = 1.0;
const IDLE_MS: u64 = 60_000;

struct Group {
    role: Role,
    ceiling: Option<Ceiling>,
    cpu: Option<(u64, u64)>,
}

struct App {
    id: String,
    /// Our own `app-wol-*` scope (Delegate=yes): role groups inside. Otherwise one group.
    delegated: bool,
    /// Has shown a window: only those are ever saved (helpers started at login are left alone).
    windowed: bool,
    phase: Phase,
    groups: BTreeMap<String, Group>,
    roles: HashMap<i32, Role>,
    games: HashMap<i32, bool>,
    chromium: bool,
    before: u64,
    cpu: Option<(u64, u64)>,
    last_busy: u64,
}

struct Session {
    uid: u32,
    scope: String,
}

pub struct Engine<S: Sys> {
    pub sys: S,
    cfg: Config,
    tune: Tune,
    session: Option<Session>,
    snap: Snapshot,
    apps: BTreeMap<String, App>,
    /// pid -> (asked at, scope cgroup, app id)
    asked: HashMap<i32, (u64, String, String)>,
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
    pub fn new(sys: S, cfg: Config, tune: Tune) -> Self {
        Engine {
            sys,
            cfg,
            tune,
            session: None,
            snap: Snapshot::default(),
            apps: BTreeMap::new(),
            asked: HashMap::new(),
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
        self.status()
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
                let delegated = scope
                    .rsplit('/')
                    .next()
                    .is_some_and(|n| n.starts_with("app-wol-"));
                // Left over from an earlier wold: no ceiling, not frozen.
                let _ = self.sys.freeze(&scope, false);
                let mut groups = BTreeMap::new();
                for r in role::ALL {
                    let g = format!("{scope}/{}", r.name());
                    if delegated && self.sys.exists(&g) {
                        self.sys.set_high(&g, None);
                        groups.insert(g, Group::new(r));
                    }
                }
                self.sys.set_high(&scope, None);
                App {
                    id: shared::memsaver::app_id(&id),
                    delegated,
                    windowed: false,
                    phase: Phase::Active,
                    groups,
                    roles: HashMap::new(),
                    games: HashMap::new(),
                    chromium: false,
                    before: 0,
                    cpu: None,
                    last_busy: 0,
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
        let pids = self.arrange(scope, &mut app);
        if app.windowed {
            let usec = self.sys.cpu_usec(scope);
            if pct(app.cpu, now, usec) > IDLE_CPU || app.cpu.is_none() {
                app.last_busy = now;
            }
            app.cpu = Some((now, usec));
            let facts = self.facts(&mut app, &pids, now);
            let (phase, action) = app.phase.step(&facts, now, &self.cfg);
            app.phase = phase;
            self.apply(scope, &mut app, action, &pids, now);
            if matches!(app.phase, Phase::Saving { .. }) {
                self.tune_groups(scope, &mut app, now);
            }
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
        }
    }

    /// Put every process of a delegated scope into its role group; returns all its pids.
    fn arrange(&mut self, scope: &str, app: &mut App) -> Vec<i32> {
        let mut placed: Vec<(String, i32)> = self
            .sys
            .procs(scope)
            .into_iter()
            .map(|p| (scope.to_owned(), p))
            .collect();
        for g in app.groups.keys() {
            placed.extend(self.sys.procs(g).into_iter().map(|p| (g.clone(), p)));
        }
        let pids: Vec<i32> = placed.iter().map(|x| x.1).collect();
        app.roles.retain(|p, _| pids.contains(p));
        if !app.delegated {
            if app.groups.is_empty() {
                app.groups
                    .insert(scope.to_owned(), Group::new(Role::Native));
            }
            return pids;
        }
        let new: Vec<(i32, Vec<String>)> = pids
            .iter()
            .filter(|p| !app.roles.contains_key(p))
            .map(|p| (*p, self.sys.argv(*p)))
            .collect();
        if !app.chromium && new.iter().any(|(_, a)| role::is_chromium_child(a)) {
            app.chromium = true;
            // Classified before the first child showed up: the untyped one is the browser.
            app.roles.retain(|_, r| *r != Role::Native);
        }
        for (p, a) in new {
            app.roles.insert(p, role::role(&a, app.chromium));
        }
        for p in &pids {
            if !app.roles.contains_key(p) {
                let a = self.sys.argv(*p);
                app.roles.insert(*p, role::role(&a, app.chromium));
            }
        }
        for (dir, p) in &placed {
            let r = app.roles[p];
            let want = format!("{scope}/{}", r.name());
            if *dir == want {
                continue;
            }
            if !app.groups.contains_key(&want) {
                if !self.sys.mkdir(&want) {
                    continue;
                }
                app.groups.insert(want.clone(), Group::new(r));
            }
            self.sys.attach(&want, *p);
        }
        if self.sys.procs(scope).is_empty() {
            self.sys.enable_memory(scope);
        }
        pids
    }

    fn apply(&mut self, scope: &str, app: &mut App, action: Action, pids: &[i32], now: u64) {
        match action {
            Action::None => {}
            Action::Save => {
                app.before = self.sys.current(scope);
                let mut grouped = 0;
                for (g, grp) in app.groups.iter_mut() {
                    grp.start(&self.sys, g, true);
                    grouped += self.sys.current(g);
                }
                // Pages charged before the processes moved into their groups.
                if app.delegated {
                    let orphan = self.sys.current(scope).saturating_sub(grouped);
                    if orphan > 0 {
                        self.sys.reclaim(scope, orphan);
                    }
                }
                self.sys.pageout(scope, pids.to_vec());
                eprintln!(
                    "wold: memsaver saving {} ({} MiB)",
                    app.id,
                    app.before >> 20
                );
            }
            Action::Nap => {
                self.sys.freeze(scope, true);
            }
            Action::Restore { prefetch } => {
                self.sys.cancel(scope);
                self.sys.freeze(scope, false);
                for (g, grp) in app.groups.iter_mut() {
                    self.sys.set_high(g, None);
                    grp.ceiling = None;
                }
                if prefetch {
                    // Compositor-facing processes first: GPU, browser, then the rest.
                    let mut order: Vec<i32> = pids.to_vec();
                    order.sort_by_key(|p| match app.roles.get(p) {
                        Some(Role::Gpu) => 0,
                        Some(Role::Main | Role::Native) => 1,
                        Some(Role::Renderer) => 2,
                        _ => 3,
                    });
                    self.sys.prefetch(order);
                }
                let _ = now;
            }
        }
    }

    fn tune_groups(&mut self, scope: &str, app: &mut App, now: u64) {
        let mut release = false;
        for (g, grp) in app.groups.iter_mut() {
            let n = self.sys.procs(g).len() as u32;
            if n == 0 {
                continue;
            }
            if grp.ceiling.is_none() {
                grp.start(&self.sys, g, false); // a role that appeared while saving
            }
            let usec = self.sys.cpu_usec(g);
            let cpu = pct(grp.cpu, now, usec);
            grp.cpu = Some((now, usec));
            let procs = if grp.role == Role::Native { 1 } else { n };
            let sample = Sample {
                now_ms: now,
                cpu,
                current: self.sys.current(g),
                procs,
                psi_some10: self.sys.pressure(g),
            };
            let Some(c) = grp.ceiling.as_mut() else {
                continue;
            };
            match c.tick(sample, &self.tune) {
                Step::Hold => {}
                Step::Set(l) => {
                    self.sys.set_high(g, Some(l));
                }
                Step::Release => release = true,
            }
        }
        if release {
            for (g, grp) in app.groups.iter_mut() {
                self.sys.set_high(g, None);
                grp.ceiling = None;
            }
            if let Phase::Saving { since } = app.phase {
                app.phase = Phase::Released { since };
            }
            self.sys.cancel(scope);
            eprintln!("wold: memsaver released {} (memory pressure)", app.id);
        }
    }

    fn status(&self) -> Status {
        let mut st = Status {
            enabled: self.snap.enabled,
            ..Status::default()
        };
        for (scope, app) in self.apps.iter().filter(|(_, a)| a.windowed) {
            let memory = self.sys.current(scope);
            let state = if !self.snap.enabled {
                AppState::Active
            } else if self.snap.exclude.contains(&app.id) {
                AppState::Excluded
            } else if app.games.values().any(|g| *g) {
                AppState::Game
            } else if app.phase == Phase::Active
                && self
                    .snap
                    .playing
                    .iter()
                    .any(|p| app.roles.contains_key(p) || self.sys.procs(scope).contains(p))
            {
                AppState::Playing
            } else {
                app.phase.state()
            };
            let pids = self.pids(scope, app);
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

    /// Every process of the app, role groups included.
    fn pids(&self, scope: &str, app: &App) -> Vec<i32> {
        let groups = app.groups.keys().map(String::as_str);
        let mut pids: Vec<i32> = std::iter::once(scope)
            .chain(groups)
            .flat_map(|g| self.sys.procs(g))
            .collect();
        pids.sort_unstable();
        pids.dedup();
        pids
    }
}

impl Group {
    fn new(role: Role) -> Group {
        Group {
            role,
            ceiling: None,
            cpu: None,
        }
    }

    /// First ceiling for this group (its role's floor, or half of a native app); `reclaim`:
    /// push what is over it to zram now instead of waiting for allocations to hit it.
    fn start(&mut self, sys: &impl Sys, g: &str, reclaim: bool) {
        let n = sys.procs(g).len() as u32;
        let cur = sys.current(g);
        let (c, procs) = match self.role.band() {
            Some((start, max)) => (Ceiling::new(start, max), n),
            None => {
                let s = role::native_ceiling(cur);
                (Ceiling::new(s, s * 2), 1)
            }
        };
        if n == 0 && self.role != Role::Native {
            return;
        }
        let limit = c.limit(procs);
        if reclaim && cur > limit {
            sys.reclaim(g, cur - limit);
        }
        sys.set_high(g, Some(limit));
        self.ceiling = Some(c);
    }
}

pub use app::Config as AppConfig;

#[cfg(test)]
mod tests;
