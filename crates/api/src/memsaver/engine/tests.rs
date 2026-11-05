// SPDX-License-Identifier: GPL-2.0-or-later
//! Engine behaviour against an in-memory model of /proc, cgroupfs and systemd.
use super::*;
use shared::memsaver::Window;
use std::cell::RefCell;

const MIB: u64 = 1 << 20;
const SESSION: &str = "/user.slice/user-1000.slice/session-2.scope";

#[derive(Clone)]
struct P {
    uid: u32,
    ppid: i32,
    comm: String,
    cg: String,
    game: bool,
}

#[derive(Clone, Default)]
struct C {
    current: u64,
    cpu: u64,
    frozen: bool,
    refused: u64,
    killed: bool,
    oom_kills: u64,
    tasks: u64,
    /// memory.high written by a budget (None: `max`), and every freeze/thaw in order.
    high: Option<u64>,
    freezes: Vec<bool>,
}

#[derive(Default)]
struct M {
    procs: BTreeMap<i32, P>,
    cgs: BTreeMap<String, C>,
    adopted: Vec<Vec<String>>,
    given: Vec<(String, u64, Vec<i32>)>,
    /// Every `page_out` (scope, pids), in order.
    paged: Vec<(String, Vec<i32>)>,
    cancelled: Vec<String>,
    swapped: BTreeMap<i32, u64>,
}

#[derive(Default)]
struct Model(RefCell<M>);

impl Model {
    fn add(&self, pid: i32, ppid: i32, comm: &str) {
        let mut m = self.0.borrow_mut();
        m.cgs.entry(SESSION.into()).or_default();
        m.procs.insert(
            pid,
            P {
                uid: 1000,
                ppid,
                comm: comm.into(),
                cg: SESSION.into(),
                game: false,
            },
        );
    }
    fn cg(&self, cg: &str) -> C {
        self.0.borrow().cgs.get(cg).cloned().unwrap_or_default()
    }
    fn set(&self, cg: &str, f: impl FnOnce(&mut C)) {
        f(self.0.borrow_mut().cgs.entry(cg.into()).or_default())
    }
    fn where_is(&self, pid: i32) -> String {
        self.0.borrow().procs[&pid].cg.clone()
    }
}

impl Sys for Model {
    fn cgroup(&self, pid: i32) -> Option<String> {
        self.0.borrow().procs.get(&pid).map(|p| p.cg.clone())
    }
    fn owner_and_parent(&self, pid: i32) -> Option<(u32, i32)> {
        self.0.borrow().procs.get(&pid).map(|p| (p.uid, p.ppid))
    }
    fn comm(&self, pid: i32) -> String {
        self.0
            .borrow()
            .procs
            .get(&pid)
            .map(|p| p.comm.clone())
            .unwrap_or_default()
    }
    fn is_game(&self, pid: i32) -> bool {
        self.0.borrow().procs.get(&pid).is_some_and(|p| p.game)
    }
    fn parents(&self) -> Vec<(i32, i32)> {
        self.0
            .borrow()
            .procs
            .iter()
            .map(|(k, p)| (*k, p.ppid))
            .collect()
    }
    fn exists(&self, cg: &str) -> bool {
        self.0.borrow().cgs.contains_key(cg)
    }
    fn procs(&self, cg: &str) -> Vec<i32> {
        let m = self.0.borrow();
        m.procs
            .iter()
            .filter(|(_, p)| p.cg == cg)
            .map(|(k, _)| *k)
            .collect()
    }
    fn current(&self, cg: &str) -> u64 {
        // Hierarchical, as memory.current is.
        let m = self.0.borrow();
        m.cgs
            .iter()
            .filter(|(k, _)| *k == cg || k.starts_with(&format!("{cg}/")))
            .map(|(_, c)| c.current)
            .sum()
    }
    fn cpu_usec(&self, cg: &str) -> u64 {
        self.cg(cg).cpu
    }
    fn freeze(&self, cg: &str, frozen: bool) -> bool {
        self.set(cg, |c| {
            c.frozen = frozen;
            c.freezes.push(frozen);
        });
        true
    }
    fn refused(&self, cg: &str) -> u64 {
        self.cg(cg).refused
    }
    fn kill(&self, cg: &str) -> bool {
        self.set(cg, |c| c.killed = true);
        true
    }
    fn oom_kills(&self, cg: &str) -> u64 {
        self.cg(cg).oom_kills
    }
    fn tasks(&self, cg: &str) -> u64 {
        self.cg(cg).tasks
    }
    fn adopt(&self, args: &[String]) -> bool {
        let name = args[7].clone();
        let n: usize = args[12].parse().unwrap();
        let pids: Vec<i32> = args[13..13 + n]
            .iter()
            .map(|p| p.parse().unwrap())
            .collect();
        let cg = format!("/user.slice/user-1000.slice/{name}");
        let mut m = self.0.borrow_mut();
        m.adopted.push(args.to_vec());
        m.cgs.entry(cg.clone()).or_default();
        for p in pids {
            m.procs.get_mut(&p).unwrap().cg = cg.clone();
        }
        true
    }
    fn give_back(&self, scope: &str, bytes: u64, pids: Vec<i32>) {
        self.0.borrow_mut().given.push((scope.into(), bytes, pids));
    }
    fn page_out(&self, scope: &str, pids: Vec<i32>) {
        self.0.borrow_mut().paged.push((scope.into(), pids));
    }
    fn cancel(&self, scope: &str) {
        self.0.borrow_mut().cancelled.push(scope.into());
    }
    fn swapped(&self, pids: &[i32]) -> u64 {
        let m = self.0.borrow();
        pids.iter().filter_map(|p| m.swapped.get(p)).sum()
    }
    fn set_high(&self, cg: &str, bytes: Option<u64>) -> bool {
        self.set(cg, |c| c.high = bytes);
        true
    }
}

const SCOPE: &str = "/user.slice/user-1000.slice/app-wol-code-20.scope";

/// The compositor (10) runs wol-realtime (11) and Code: browser 20, zygote 21, renderers 22 23, gpu 24.
fn session() -> Engine<Model> {
    let m = Model::default();
    m.add(10, 1, "Hyprland");
    m.add(11, 10, "wol-realtime");
    m.add(20, 10, "code");
    m.add(21, 20, "code");
    m.add(22, 21, "code");
    m.add(23, 21, "code");
    m.add(24, 20, "code");
    let mut e = Engine::new(m, Config::default());
    assert!(e.connect(1000, 11));
    e
}

fn snap(front: i32, visible: bool) -> Snapshot {
    Snapshot {
        enabled: true,
        front,
        windows: vec![Window {
            pid: 20,
            app: "code".into(),
            visible,
        }],
        ..Snapshot::default()
    }
}

/// Code adopted, 1200 MiB in its scope.
fn adopted() -> Engine<Model> {
    adopted_with(|_| {})
}

fn adopted_with(f: impl FnOnce(&Model)) -> Engine<Model> {
    let mut e = session();
    f(&e.sys);
    e.snapshot(snap(20, true));
    e.tick(0);
    e.tick(1);
    e.sys.set(SCOPE, |c| c.current = 1200 * MIB);
    e
}

#[test]
fn a_window_moves_its_whole_app_into_its_own_scope() {
    let e = adopted();
    let m = e.sys.0.borrow();
    assert_eq!(m.adopted.len(), 1, "asked systemd once");
    let args = m.adopted[0].join(" ");
    assert!(
        args.contains("app-wol-code-20.scope fail 9 PIDs au 5 20 21 24 22 23"),
        "{args}"
    );
    assert!(args.contains("BindsTo as 1 session-2.scope"), "{args}");
    drop(m);
    for pid in 20..=24 {
        assert_eq!(e.sys.where_is(pid), SCOPE, "pid {pid}");
    }
    assert_eq!(e.sys.where_is(10), SESSION, "the compositor stays");
    assert_eq!(e.sys.where_is(11), SESSION, "wol-realtime stays");
}

/// Pages touched before adoption stay charged to the session: memory.reclaim on the scope cannot
/// see them, so give_back also pages out every process, and focus stops it at once.
#[test]
fn background_apps_give_memory_back_once_after_thirty_seconds_and_focus_stops_it() {
    let mut e = adopted();
    e.snapshot(snap(99, false));
    let st = e.tick(1_000);
    assert_eq!(st.apps[0].state, AppState::Waiting);
    assert!(
        e.sys.0.borrow().given.is_empty(),
        "nothing inside the delay"
    );
    let st = e.tick(31_000);
    assert_eq!(st.apps[0].state, AppState::Saving);
    e.tick(33_000);
    e.tick(300_000);
    {
        let m = e.sys.0.borrow();
        assert_eq!(m.given.len(), 1, "once: a page refaulted is in use");
        let (scope, bytes, pids) = &m.given[0];
        assert_eq!((scope.as_str(), *bytes), (SCOPE, 1200 * MIB));
        assert_eq!(pids, &[20, 21, 22, 23, 24]);
        assert!(m.cancelled.is_empty());
    }
    // Focus (wold's QoS path) stops whatever is still running.
    e.focus(20);
    let st = e.tick(301_000);
    assert_eq!(st.apps[0].state, AppState::Active);
    assert_eq!(e.sys.0.borrow().cancelled, [SCOPE]);
}

#[test]
fn apps_playing_sound_are_never_touched() {
    let mut e = adopted();
    let mut s = snap(99, false);
    s.playing = vec![23];
    e.snapshot(s);
    e.tick(1_000);
    let st = e.tick(120_000);
    assert_eq!(st.apps[0].state, AppState::Playing);
    assert!(e.sys.0.borrow().given.is_empty());
}

#[test]
fn exclusions_and_games_are_never_touched() {
    let mut e = adopted();
    let mut s = snap(99, false);
    s.exclude = vec!["code".into()];
    e.snapshot(s);
    e.tick(1_000);
    assert_eq!(e.tick(120_000).apps[0].state, AppState::Excluded);
    let mut e = adopted_with(|m| m.0.borrow_mut().procs.get_mut(&22).unwrap().game = true);
    e.snapshot(snap(99, false));
    e.tick(1_000);
    assert_eq!(e.tick(120_000).apps[0].state, AppState::Game);
    assert!(e.sys.0.borrow().given.is_empty());
}

/// Chromium and Electron move their browser process into a user manager scope at startup; the
/// zygote and renderers stay behind in the session. The whole tree belongs to the app.
#[test]
fn an_app_that_moved_itself_to_a_user_scope_is_adopted_with_the_children_it_left_behind() {
    let mut e = session();
    let own = "/user.slice/user-1000.slice/user@1000.service/app.slice/app-code-20.scope";
    e.sys.set(own, |_| {});
    e.sys.0.borrow_mut().procs.get_mut(&20).unwrap().cg = own.into();
    e.snapshot(snap(20, false));
    e.tick(0);
    e.tick(1);
    for pid in 20..=24 {
        assert_eq!(e.sys.where_is(pid), SCOPE, "pid {pid}");
    }
    assert_eq!(e.sys.where_is(10), SESSION, "the compositor stays");
    assert_eq!(e.sys.0.borrow().adopted.len(), 1, "asked systemd once");
}

/// IMAGE-F: a Flatpak app runs as `flatpak run` (12) -> bwrap (13) -> bwrap (14) -> the app (20, its
/// helpers 21-24), all in the user manager's `app-flatpak-<id>-<n>.scope`. The window's process and its
/// children (the sandboxed tree) are adopted into the saver's scope, like Electron's; the sandbox's
/// supervisors above it stay where they are and keep the Flatpak scope alive. The app is known by its
/// app id.
#[test]
fn a_flatpak_app_is_adopted_by_its_window_and_its_supervisors_stay() {
    let m = Model::default();
    m.add(10, 1, "Hyprland");
    m.add(11, 10, "wol-realtime");
    m.add(12, 10, "flatpak");
    m.add(13, 12, "bwrap");
    m.add(14, 13, "bwrap");
    m.add(20, 14, "firefox");
    m.add(21, 20, "firefox");
    m.add(22, 20, "firefox");
    m.add(23, 20, "firefox");
    m.add(24, 20, "firefox");
    let own = "/user.slice/user-1000.slice/user@1000.service/app.slice/app-flatpak-org.mozilla.firefox-4242.scope";
    m.set(own, |_| {});
    for pid in [12, 13, 14, 20, 21, 22, 23, 24] {
        m.0.borrow_mut().procs.get_mut(&pid).unwrap().cg = own.into();
    }
    let mut e = Engine::new(m, Config::default());
    assert!(e.connect(1000, 11));
    e.snapshot(Snapshot {
        enabled: true,
        front: 20,
        windows: vec![Window {
            pid: 20,
            app: "org.mozilla.Firefox".into(),
            visible: false,
        }],
        ..Snapshot::default()
    });
    e.tick(0);
    e.tick(1);
    let scope = "/user.slice/user-1000.slice/app-wol-org.mozilla.firefox-20.scope";
    for pid in [20, 21, 22, 23, 24] {
        assert_eq!(e.sys.where_is(pid), scope, "pid {pid}");
    }
    for pid in [12, 13, 14] {
        assert_eq!(e.sys.where_is(pid), own, "supervisor {pid} stays");
    }
    assert_eq!(e.sys.where_is(10), SESSION, "the compositor stays");
    assert_eq!(e.sys.0.borrow().adopted.len(), 1, "asked systemd once");
    let st = e.tick(2);
    assert_eq!(st.apps.len(), 1, "{:?}", st.apps);
    assert_eq!(st.apps[0].app, "org.mozilla.firefox");
}

/// QA-100 H9-01: a web app's host already sits in its own `app-wol-` scope of the user's
/// manager; the saver leaves it there (wold knows the app by that scope name) and still lists it.
#[test]
fn app_wol_scope_under_user_manager_not_movable() {
    let mut e = session();
    let host = "/user.slice/user-1000.slice/user@1000.service/app.slice/app-wol-org.wolos.webapp.doctor-20-1f.scope";
    e.sys.set(host, |_| {});
    for pid in 20..=24 {
        e.sys.0.borrow_mut().procs.get_mut(&pid).unwrap().cg = host.into();
    }
    e.snapshot(Snapshot {
        enabled: true,
        front: 20,
        windows: vec![Window {
            pid: 20,
            app: "org.wolos.webapp.doctor".into(),
            visible: true,
        }],
        ..Snapshot::default()
    });
    let mut st = e.tick(0);
    for t in [1, 2_000, 4_000, 10_000] {
        st = e.tick(t);
    }
    assert!(
        e.sys.0.borrow().adopted.is_empty(),
        "nothing asked of systemd"
    );
    for pid in 20..=24 {
        assert_eq!(e.sys.where_is(pid), host, "pid {pid}");
    }
    assert!(
        wol_core::sysapp::app_of_cgroup(&format!("0::{host}"), 20).is_some(),
        "wold still knows the app"
    );
    assert_eq!(st.apps.len(), 1, "{:?}", st.apps);
}

#[test]
fn processes_without_a_window_are_left_where_they_are() {
    let m = Model::default();
    m.add(10, 1, "Hyprland");
    m.add(11, 10, "wol-realtime");
    m.add(12, 10, "wol-shell");
    m.add(30, 12, "slack");
    m.add(40, 10, "pipewire");
    let mut e = Engine::new(m, Config::default());
    assert!(e.connect(1000, 11));
    e.snapshot(Snapshot {
        enabled: true,
        ..Snapshot::default()
    });
    let st = e.tick(0);
    for pid in [12, 30, 40, 11] {
        assert_eq!(e.sys.where_is(pid), SESSION);
    }
    assert!(st.apps.is_empty());
    assert!(e.sys.0.borrow().adopted.is_empty());
}

#[test]
fn only_the_users_own_slice_is_adopted_from() {
    let mut e = session();
    let system = "/system.slice/foo.service";
    e.sys.set(system, |_| {});
    e.sys.0.borrow_mut().procs.get_mut(&20).unwrap().cg = system.into();
    e.snapshot(snap(20, false));
    e.tick(0);
    assert_eq!(e.sys.where_is(20), system);
    assert!(e.sys.0.borrow().adopted.is_empty());
}

#[test]
fn saved_counts_what_was_compressed_away_wherever_it_was_charged() {
    let mut e = adopted();
    e.snapshot(snap(99, false));
    e.tick(1_000);
    e.sys
        .0
        .borrow_mut()
        .swapped
        .extend([(20, 300 * MIB), (22, 900 * MIB)]);
    let st = e.tick(40_000);
    let code = st.apps.iter().find(|a| a.app == "code").unwrap();
    assert_eq!(code.saved, 1200 * MIB);
    assert_eq!(code.swap, 1200 * MIB);
    assert_eq!(st.saved, 1200 * MIB);
    e.snapshot(snap(20, true));
    let st = e.tick(41_000);
    assert_eq!(st.saved, 0, "in use: nothing is being saved");
}

/// QA-100 PERF-15: forks that share pages each report them as swapped; the total saved is
/// still no more than the app held when it was saved.
#[test]
fn saved_never_exceeds_what_the_app_held() {
    let mut e = adopted();
    e.snapshot(snap(99, false));
    e.tick(1_000);
    e.sys
        .0
        .borrow_mut()
        .swapped
        .extend([(20, 900 * MIB), (22, 900 * MIB)]);
    let st = e.tick(40_000);
    let code = st.apps.iter().find(|a| a.app == "code").unwrap();
    assert_eq!(code.saved, 1200 * MIB, "what it held");
    assert_eq!(code.swap, 1800 * MIB, "what the kernel counts");
    assert_eq!(st.saved, 1200 * MIB);
    e.snapshot(snap(20, true));
    let st = e.tick(41_000);
    assert_eq!(st.saved, 0);
}

#[test]
fn other_users_processes_are_never_adopted() {
    let mut e = session();
    e.sys.0.borrow_mut().procs.get_mut(&20).unwrap().uid = 0;
    e.snapshot(snap(99, false));
    e.tick(0);
    assert_eq!(e.sys.where_is(20), SESSION);
    // A session agent cannot claim someone else's session.
    let mut e = session();
    assert!(!e.connect(1001, 11));
}

#[test]
fn losing_the_session_agent_restores_everything() {
    let mut e = adopted();
    let mut s = snap(99, false);
    s.nap = true;
    e.snapshot(s);
    e.tick(1_000);
    e.tick(31_000);
    e.tick(11 * 60_000);
    assert!(e.sys.cg(SCOPE).frozen);
    let st = e.disconnect(12 * 60_000);
    assert!(!st.enabled);
    assert!(!e.sys.cg(SCOPE).frozen);
    assert_eq!(e.sys.0.borrow().cancelled, [SCOPE]);
}

/// A frozen app cannot fault its memory back, so a nap gives back what it had refaulted.
#[test]
fn app_nap_freezes_hidden_idle_apps_gives_back_again_and_thaws_on_focus() {
    let mut e = adopted();
    let mut s = snap(99, false);
    s.nap = true;
    e.snapshot(s.clone());
    e.tick(1_000);
    e.tick(31_000);
    e.sys.set(SCOPE, |c| c.current = 700 * MIB);
    let st = e.tick(11 * 60_000);
    assert_eq!(st.apps[0].state, AppState::Napping);
    assert!(e.sys.cg(SCOPE).frozen);
    assert_eq!(e.sys.0.borrow().given.len(), 2);
    assert_eq!(e.sys.0.borrow().given[1].1, 700 * MIB);
    s.front = 20;
    e.snapshot(s);
    e.tick(11 * 60_000 + 10);
    assert!(!e.sys.cg(SCOPE).frozen);
}

#[test]
fn a_fork_storm_kills_its_own_scope_and_nothing_else() {
    let mut e = adopted();
    let refused = |e: &Engine<Model>, n| e.sys.set(SCOPE, |c| c.refused = n);
    e.tick(2000);
    refused(&e, 50_000);
    e.tick(4000);
    assert!(!e.sys.cg(SCOPE).killed, "one tick can be a burst");
    refused(&e, 100_000);
    e.tick(6000);
    assert!(e.sys.cg(SCOPE).killed);
    assert!(!e.sys.cg(SESSION).killed);
}

#[test]
fn a_quiet_app_at_its_limit_is_left_alone() {
    let mut e = adopted();
    for i in 1..20 {
        e.sys.set(SCOPE, |c| c.refused = i * 100);
        e.tick(i * 2000);
    }
    assert!(!e.sys.cg(SCOPE).killed);
}

#[test]
fn a_memory_bomb_that_keeps_refilling_is_killed() {
    let mut e = adopted();
    for (i, n) in [0, 2, 3, 5].into_iter().enumerate() {
        e.sys.set(SCOPE, |c| c.oom_kills = n);
        e.tick(2000 * (i as u64 + 1));
    }
    assert!(e.sys.cg(SCOPE).killed);
}

const SLICE: &str = "/user.slice/user-1000.slice";

fn slice_storm(e: &mut Engine<Model>, full: u64) {
    e.sys.set(SCOPE, |c| c.tasks = 5000);
    for i in 1..4 {
        e.sys.set(SLICE, |c| {
            c.tasks = full;
            c.refused = i * 50_000;
        });
        e.tick(2000 * i);
    }
}

#[test]
fn a_full_slice_loses_its_biggest_app() {
    let mut e = adopted();
    slice_storm(&mut e, procs::USER_TASKS);
    assert!(e.sys.cg(SCOPE).killed, "its scope is below its own limit");
    assert!(!e.sys.cg(SESSION).killed);
}

#[test]
fn one_app_at_its_own_limit_is_not_a_full_slice() {
    let mut e = adopted();
    slice_storm(&mut e, procs::APP_TASKS as u64);
    assert!(!e.sys.cg(SCOPE).killed);
}

/// A saved app that quits: its give-back is stopped and forgotten with its scope.
#[test]
fn a_scope_that_is_gone_has_its_give_back_cancelled() {
    let mut e = adopted();
    e.snapshot(snap(99, false));
    e.tick(1_000);
    e.tick(31_000);
    assert_eq!(e.sys.0.borrow().given.len(), 1);
    {
        let mut m = e.sys.0.borrow_mut();
        m.cgs.remove(SCOPE);
        m.procs.retain(|p, _| !(20..=24).contains(p));
    }
    e.snapshot(Snapshot {
        enabled: true,
        front: 99,
        ..Snapshot::default()
    });
    let st = e.tick(33_000);
    assert!(st.apps.is_empty());
    assert_eq!(e.sys.0.borrow().cancelled, [SCOPE]);
}

// ---- per-app budgets (wol_core::memsaver::budget, ADR 0027 amendment) ----

/// Discord as `budget::launch_argv` starts it: main 30 (renderer 31, gpu 32) in
/// `app-discord-30.scope` of the user's manager,
const DISCORD: &str =
    "/user.slice/user-1000.slice/user@1000.service/app.slice/app-discord-30.scope";

fn discord() -> Engine<Model> {
    let e = session();
    for (pid, ppid) in [(30, 10), (31, 30), (32, 30)] {
        e.sys.add(pid, ppid, "Discord");
        e.sys.0.borrow_mut().procs.get_mut(&pid).unwrap().cg = DISCORD.into();
    }
    e.sys.set(DISCORD, |_| {});
    e
}

/// Discord's row with the freeze cycle on: the engine holds the cycle for a row that earns it.
static DUTY_TABLE: [Budget; 1] = [Budget {
    duty: Some(budget::Duty {
        stop_ms: 4_000,
        run_ms: 1_000,
    }),
    ..budget::TABLE[0]
}];

fn discord_duty() -> Engine<Model> {
    let mut e = discord();
    e.table = &DUTY_TABLE;
    e
}

fn dsnap(front: i32, visible: bool) -> Snapshot {
    Snapshot {
        enabled: true,
        front,
        windows: vec![Window {
            pid: 30,
            app: "discord".into(),
            visible,
        }],
        ..Snapshot::default()
    }
}

#[test]
fn a_budgeted_app_started_in_its_own_scope_stays_there() {
    let mut e = discord();
    e.snapshot(dsnap(30, true));
    for t in [0, 1, 20_000] {
        e.tick(t);
    }
    assert!(
        e.sys.0.borrow().adopted.is_empty(),
        "nothing asked of systemd"
    );
    for pid in 30..=32 {
        assert_eq!(e.sys.where_is(pid), DISCORD, "pid {pid}");
    }
    // Started the usual way (in the session), Discord is adopted like any app.
    let mut e = discord();
    for pid in 30..=32 {
        e.sys.0.borrow_mut().procs.get_mut(&pid).unwrap().cg = SESSION.into();
    }
    e.snapshot(dsnap(30, true));
    e.tick(0);
    assert_eq!(e.sys.0.borrow().adopted.len(), 1);
}

#[test]
fn the_budget_waits_for_the_start_then_limits_front_and_back() {
    let b = budget::of("discord").unwrap();
    let mut e = discord();
    e.snapshot(dsnap(30, true));
    e.tick(0);
    assert_eq!(e.sys.cg(DISCORD).high, None, "no limit while it starts");
    e.tick(b.start_ms - 1);
    assert_eq!(e.sys.cg(DISCORD).high, None);
    e.tick(b.start_ms);
    assert_eq!(e.sys.cg(DISCORD).high, b.high(Place::Front));
    // Behind: the front limit through the saver's delay, then the background one.
    e.snapshot(dsnap(99, false));
    let away = b.start_ms + 1_000;
    e.tick(away);
    assert_eq!(e.sys.cg(DISCORD).high, b.high(Place::Front));
    e.tick(away + Config::default().delay_ms);
    assert_eq!(e.sys.cg(DISCORD).high, b.high(Place::Back));
    // Back in front: the front limit again.
    e.focus(30);
    e.tick(away + Config::default().delay_ms + 10);
    assert_eq!(e.sys.cg(DISCORD).high, b.high(Place::Front));
    // Memory Saver off: no limit at all.
    let mut s = dsnap(30, true);
    s.enabled = false;
    e.snapshot(s);
    e.tick(away + Config::default().delay_ms + 2_000);
    assert_eq!(e.sys.cg(DISCORD).high, None);
}

#[test]
fn behind_a_budgeted_app_runs_the_freeze_duty_cycle_and_is_thawed_when_used() {
    let d = DUTY_TABLE[0].duty.unwrap();
    let mut e = discord_duty();
    e.snapshot(dsnap(30, true));
    e.tick(0);
    e.sys.set(DISCORD, |c| c.freezes.clear());
    e.snapshot(dsnap(99, false));
    let away = 1_000;
    e.tick(away);
    let saving = away + Config::default().delay_ms;
    e.tick(saving);
    assert_eq!(
        e.sys.cg(DISCORD).freezes,
        [true],
        "frozen once Saving starts"
    );
    assert_eq!(e.due(), Some(saving + d.stop_ms));
    e.tick(saving + d.stop_ms);
    e.tick(saving + d.stop_ms + d.run_ms);
    assert_eq!(e.sys.cg(DISCORD).freezes, [true, false, true]);
    assert!(e.sys.cg(DISCORD).frozen);
    // Not started long enough for a limit (start_ms): frozen all the same.
    assert_eq!(e.sys.cg(DISCORD).high, None);
    // Used again: thawed at once, no cycle due.
    e.focus(30);
    e.tick(saving + d.stop_ms + d.run_ms + 10);
    assert!(!e.sys.cg(DISCORD).frozen);
    assert_eq!(e.due(), None);
}

#[test]
fn a_budgeted_app_in_a_call_is_never_frozen() {
    let mut e = discord_duty();
    e.snapshot(dsnap(30, true));
    e.tick(0);
    e.sys.set(DISCORD, |c| c.freezes.clear());
    let mut s = dsnap(99, false);
    s.playing = vec![31];
    e.snapshot(s);
    for t in [1_000, 40_000, 45_000, 50_000, 120_000] {
        e.tick(t);
    }
    assert!(
        e.sys.cg(DISCORD).freezes.is_empty(),
        "{:?}",
        e.sys.cg(DISCORD).freezes
    );
    assert_eq!(e.due(), None);
}

const ADOPTED: &str =
    "/user.slice/user-1000.slice/user@1000.service/app.slice/app-wol-discord-30.scope";

/// Discord adopted into a Memory Saver scope (started the usual way) gets no budget: its pages
/// were charged elsewhere before the move, a limit would only pin `memory.current` and stall it.
/// It is a plain Memory Saver app: no `memory.high`, no freeze cycle (even in a table that has one).
#[test]
fn an_adopted_discord_gets_no_limit_and_no_freeze() {
    let mut e = discord_duty();
    for pid in 30..=32 {
        e.sys.0.borrow_mut().procs.get_mut(&pid).unwrap().cg = ADOPTED.into();
    }
    e.sys.set(ADOPTED, |_| {});
    e.snapshot(dsnap(30, true));
    e.tick(0);
    e.snapshot(dsnap(99, false));
    let away = 70_000;
    e.tick(away);
    for t in [away + 30_000, away + 40_000, away + 50_000, away + 90_000] {
        e.tick(t);
    }
    let c = e.sys.cg(ADOPTED);
    assert_eq!(c.high, None, "no memory.high on an adopted scope");
    assert!(!c.freezes.contains(&true), "no freeze: {:?}", c.freezes);
    assert_eq!(e.due(), None);
}

/// The table's own Discord row has no freeze cycle: behind, the scope is limited and never frozen.
#[test]
fn behind_discord_is_limited_and_not_frozen_by_default() {
    let b = budget::of("discord").unwrap();
    let mut e = discord();
    e.snapshot(dsnap(30, true));
    e.tick(0);
    e.snapshot(dsnap(99, false));
    let away = b.start_ms + 1_000;
    e.tick(away);
    e.tick(away + Config::default().delay_ms);
    e.tick(away + Config::default().delay_ms + 20_000);
    assert_eq!(e.sys.cg(DISCORD).high, b.high(Place::Back));
    assert!(!e.sys.cg(DISCORD).freezes.contains(&true));
    assert_eq!(e.due(), None);
}

// ---- the budget's page-out behind (ADR 0061 §4) ----

/// Times (ms after the window first showed) at which Discord, behind from `away`, has passed its
/// start and the saver's delay: the tick that turns it to Saving with the background limit.
fn behind_from(e: &mut Engine<Model>, away: u64) -> u64 {
    let b = budget::of("discord").unwrap();
    assert!(away >= b.start_ms);
    e.snapshot(dsnap(30, true));
    e.tick(0);
    e.snapshot(dsnap(99, false));
    e.tick(away);
    let saving = away + Config::default().delay_ms;
    e.tick(saving);
    assert_eq!(e.sys.cg(DISCORD).high, b.high(Place::Back));
    saving
}

fn paged(e: &Engine<Model>) -> Vec<(String, Vec<i32>)> {
    e.sys.0.borrow().paged.clone()
}

#[test]
fn the_page_out_runs_every_thirty_seconds() {
    assert_eq!(PAGEOUT_EVERY_MS, 30_000);
}

/// Behind, Discord gives back once (the saver's Save) and is then paged out every 30 s, each time
/// the scope's processes as they are now; in front again nothing more is asked and a page-out
/// still running is stopped.
#[test]
fn behind_a_budgeted_app_is_paged_out_every_thirty_seconds_until_it_is_used() {
    let mut e = discord();
    let saving = behind_from(&mut e, 61_000);
    assert_eq!(
        e.sys.0.borrow().given.len(),
        1,
        "the Save is the first page-out"
    );
    assert!(paged(&e).is_empty());
    e.tick(saving + PAGEOUT_EVERY_MS - 1);
    assert!(paged(&e).is_empty(), "not before 30 s");
    e.tick(saving + PAGEOUT_EVERY_MS);
    assert_eq!(paged(&e), [(DISCORD.to_owned(), vec![30, 31, 32])]);
    // A renderer that started meanwhile is in the next one.
    e.sys.add(33, 30, "Discord");
    e.sys.0.borrow_mut().procs.get_mut(&33).unwrap().cg = DISCORD.into();
    e.tick(saving + PAGEOUT_EVERY_MS + 2_000);
    assert_eq!(paged(&e).len(), 1, "one per 30 s, not per tick");
    e.tick(saving + 2 * PAGEOUT_EVERY_MS);
    assert_eq!(paged(&e)[1], (DISCORD.to_owned(), vec![30, 31, 32, 33]));
    let st = e.tick(saving + 2 * PAGEOUT_EVERY_MS + 10);
    assert_eq!(st.apps[0].pageouts, 2, "{st:?}");
    // Used again: no page-out, the running one is stopped.
    e.focus(30);
    e.tick(saving + 2 * PAGEOUT_EVERY_MS + 100);
    assert!(e.sys.0.borrow().cancelled.contains(&DISCORD.to_owned()));
    for t in [3, 4, 5, 10] {
        e.tick(saving + t * PAGEOUT_EVERY_MS);
    }
    assert_eq!(paged(&e).len(), 2, "never in front");
}

/// A call (a capture stream) or sound: never paged out, however long it is behind.
#[test]
fn a_budgeted_app_playing_or_in_a_call_is_never_paged_out() {
    let mut e = discord();
    e.snapshot(dsnap(30, true));
    e.tick(0);
    let mut s = dsnap(99, false);
    s.playing = vec![31];
    e.snapshot(s);
    for t in (61..=900).step_by(2).map(|s| s * 1_000) {
        e.tick(t);
    }
    assert!(paged(&e).is_empty(), "{:?}", paged(&e));
    // The call ends behind: the saver's delay, then the page-outs.
    e.snapshot(dsnap(99, false));
    let quiet = 902_000;
    e.tick(quiet);
    e.tick(quiet + Config::default().delay_ms);
    e.tick(quiet + Config::default().delay_ms + PAGEOUT_EVERY_MS);
    assert_eq!(paged(&e).len(), 1);
    // A call starts while it is behind: the page-outs stop at once.
    let mut s = dsnap(99, false);
    s.playing = vec![32];
    e.snapshot(s);
    for k in 2..10 {
        e.tick(quiet + Config::default().delay_ms + k * PAGEOUT_EVERY_MS);
    }
    assert_eq!(paged(&e).len(), 1, "{:?}", paged(&e));
}

/// Saver off, Discord excluded, a game, still starting (`start_ms`), or adopted instead of born in
/// its scope: no page-out.
#[test]
fn no_page_out_when_the_saver_is_off_excluded_a_game_starting_or_adopted() {
    let long = |e: &mut Engine<Model>, from: u64| {
        for k in 0..20 {
            e.tick(from + k * PAGEOUT_EVERY_MS / 2);
        }
    };
    // Off.
    let mut e = discord();
    let saving = behind_from(&mut e, 61_000);
    let mut s = dsnap(99, false);
    s.enabled = false;
    e.snapshot(s);
    long(&mut e, saving + 1);
    assert!(paged(&e).is_empty(), "saver off");
    // Excluded.
    let mut e = discord();
    let saving = behind_from(&mut e, 61_000);
    let mut s = dsnap(99, false);
    s.exclude = vec!["discord".into()];
    e.snapshot(s);
    long(&mut e, saving + 1);
    assert!(paged(&e).is_empty(), "excluded");
    // A game.
    let mut e = discord();
    e.sys.0.borrow_mut().procs.get_mut(&31).unwrap().game = true;
    e.snapshot(dsnap(30, true));
    e.tick(0);
    e.snapshot(dsnap(99, false));
    long(&mut e, 61_000);
    assert!(paged(&e).is_empty(), "game");
    // Behind before its start is over: the saver's Save, no page-out until start_ms has passed.
    let mut e = discord();
    e.snapshot(dsnap(30, true));
    e.tick(0);
    e.snapshot(dsnap(99, false));
    for t in (1..60).map(|s| s * 1_000) {
        e.tick(t);
    }
    assert!(paged(&e).is_empty(), "still starting");
    // Adopted (started the usual way): a plain Memory Saver app.
    let mut e = discord();
    for pid in 30..=32 {
        e.sys.0.borrow_mut().procs.get_mut(&pid).unwrap().cg = ADOPTED.into();
    }
    e.sys.set(ADOPTED, |_| {});
    e.snapshot(dsnap(30, true));
    e.tick(0);
    e.snapshot(dsnap(99, false));
    long(&mut e, 61_000);
    assert!(paged(&e).is_empty(), "adopted");
}

/// An app that is not in the budget table is never paged out by the budget, only given back once.
#[test]
fn apps_without_a_budget_are_given_back_once_and_never_paged_out() {
    let mut e = adopted();
    e.snapshot(snap(99, false));
    for t in (0..=600).step_by(2).map(|s| s * 1_000) {
        e.tick(t);
    }
    assert!(paged(&e).is_empty());
    assert_eq!(e.sys.0.borrow().given.len(), 1);
}
