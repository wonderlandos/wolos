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
}

#[derive(Default)]
struct M {
    procs: BTreeMap<i32, P>,
    cgs: BTreeMap<String, C>,
    adopted: Vec<Vec<String>>,
    given: Vec<(String, u64, Vec<i32>)>,
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
        self.set(cg, |c| c.frozen = frozen);
        true
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
    fn cancel(&self, scope: &str) {
        self.0.borrow_mut().cancelled.push(scope.into());
    }
    fn swapped(&self, pids: &[i32]) -> u64 {
        let m = self.0.borrow();
        pids.iter().filter_map(|p| m.swapped.get(p)).sum()
    }
}

const SCOPE: &str = "/user.slice/user-1000.slice/app-wol-code-20.scope";

/// Hyprland (10) runs wol-realtime (11) and Code: browser 20, zygote 21, renderers 22 23, gpu 24.
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
        args.contains("app-wol-code-20.scope fail 6 PIDs au 5 20 21 24 22 23"),
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
