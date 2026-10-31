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
    argv: Vec<String>,
    cg: String,
    game: bool,
}

#[derive(Clone, Default)]
struct C {
    current: u64,
    cpu: u64,
    psi: f32,
    high: Option<Option<u64>>,
    frozen: bool,
    memory_on: bool,
}

#[derive(Default)]
struct M {
    procs: BTreeMap<i32, P>,
    cgs: BTreeMap<String, C>,
    adopted: Vec<Vec<String>>,
    reclaimed: Vec<(String, u64)>,
    prefetched: Vec<Vec<i32>>,
}

#[derive(Default)]
struct Model(RefCell<M>);

impl Model {
    fn add(&self, pid: i32, ppid: i32, comm: &str, cmd: &str) {
        let mut m = self.0.borrow_mut();
        m.cgs.entry(SESSION.into()).or_default();
        m.procs.insert(
            pid,
            P {
                uid: 1000,
                ppid,
                comm: comm.into(),
                argv: cmd.split(' ').map(String::from).collect(),
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
    fn argv(&self, pid: i32) -> Vec<String> {
        self.0
            .borrow()
            .procs
            .get(&pid)
            .map(|p| p.argv.clone())
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
    fn mkdir(&self, cg: &str) -> bool {
        self.0.borrow_mut().cgs.entry(cg.into()).or_default();
        true
    }
    fn attach(&self, cg: &str, pid: i32) -> bool {
        let mut m = self.0.borrow_mut();
        let ok = m.cgs.contains_key(cg);
        if let (true, Some(p)) = (ok, m.procs.get_mut(&pid)) {
            p.cg = cg.into();
        }
        ok
    }
    fn enable_memory(&self, cg: &str) -> bool {
        self.set(cg, |c| c.memory_on = true);
        true
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
    fn swap(&self, _: &str) -> u64 {
        0
    }
    fn cpu_usec(&self, cg: &str) -> u64 {
        self.cg(cg).cpu
    }
    fn pressure(&self, cg: &str) -> f32 {
        self.cg(cg).psi
    }
    fn set_high(&self, cg: &str, bytes: Option<u64>) -> bool {
        self.set(cg, |c| c.high = Some(bytes));
        true
    }
    fn reclaim(&self, cg: &str, bytes: u64) -> bool {
        self.0.borrow_mut().reclaimed.push((cg.into(), bytes));
        self.set(cg, |c| c.current = c.current.saturating_sub(bytes));
        true
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
    fn prefetch(&self, pids: Vec<i32>) {
        self.0.borrow_mut().prefetched.push(pids);
    }
}

const SCOPE: &str = "/user.slice/user-1000.slice/app-wol-code-20.scope";

/// Hyprland (10) runs wol-realtime (11) and Code: browser 20, zygote 21, renderers 22 23, gpu 24.
fn session() -> Engine<Model> {
    let m = Model::default();
    m.add(10, 1, "Hyprland", "Hyprland");
    m.add(11, 10, "wol-realtime", "wol-realtime");
    m.add(20, 10, "code", "/opt/code/code");
    m.add(21, 20, "code", "/opt/code/code --type=zygote");
    m.add(22, 21, "code", "/opt/code/code --type=renderer");
    m.add(23, 21, "code", "/opt/code/code --type=renderer");
    m.add(24, 20, "code", "/opt/code/code --type=gpu-process");
    let mut e = Engine::new(m, Config::default(), Tune::default());
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

/// Code adopted and arranged, then 300 MiB in each renderer and the browser.
fn adopted() -> Engine<Model> {
    adopted_with(|_| {})
}

fn adopted_with(f: impl FnOnce(&Model)) -> Engine<Model> {
    let mut e = session();
    f(&e.sys);
    e.snapshot(snap(20, true));
    e.tick(0);
    e.tick(1);
    for r in ["main", "renderer", "gpu", "zygote"] {
        e.sys
            .set(&format!("{SCOPE}/{r}"), |c| c.current = 300 * MIB);
    }
    e
}

fn group(r: &str) -> String {
    format!("{SCOPE}/{r}")
}

#[test]
fn a_window_moves_its_whole_app_into_its_own_scope_with_one_group_per_role() {
    let e = adopted();
    let m = e.sys.0.borrow();
    assert_eq!(m.adopted.len(), 1, "asked systemd once");
    let args = m.adopted[0].join(" ");
    assert!(
        args.contains("app-wol-code-20.scope fail 7 PIDs au 5 20 21 24 22 23"),
        "{args}"
    );
    assert!(args.contains("BindsTo as 1 session-2.scope"), "{args}");
    drop(m);
    for (pid, r) in [
        (20, "main"),
        (21, "zygote"),
        (22, "renderer"),
        (23, "renderer"),
        (24, "gpu"),
    ] {
        assert_eq!(e.sys.where_is(pid), group(r), "pid {pid}");
    }
    assert_eq!(e.sys.where_is(10), SESSION, "the compositor stays");
    assert_eq!(e.sys.where_is(11), SESSION, "wol-realtime stays");
    assert!(e.sys.cg(SCOPE).memory_on);
}

#[test]
fn background_apps_give_memory_back_after_thirty_seconds_and_get_it_back_on_focus() {
    let mut e = adopted();
    e.snapshot(snap(99, false));
    let st = e.tick(1_000);
    assert_eq!(st.apps[0].state, AppState::Waiting);
    assert!(
        e.sys.0.borrow().reclaimed.is_empty(),
        "nothing inside the delay"
    );
    let st = e.tick(31_000);
    assert_eq!(st.apps[0].state, AppState::Saving);
    // Renderers: 2 x 40 MiB floor; browser 48; gpu 40; zygote 16.
    for (r, limit) in [("renderer", 80), ("main", 48), ("gpu", 40), ("zygote", 16)] {
        assert_eq!(e.sys.cg(&group(r)).high, Some(Some(limit * MIB)), "{r}");
        assert_eq!(
            e.sys.cg(&group(r)).current,
            limit * MIB,
            "{r} reclaimed to its ceiling"
        );
    }
    assert_eq!(st.saved, (1200 - 184) * MIB);
    // Focus (wold's QoS path) restores at once, GPU first in the prefetch.
    e.focus(20);
    let st = e.tick(32_000);
    assert_eq!(st.apps[0].state, AppState::Active);
    for r in ["renderer", "main", "gpu", "zygote"] {
        assert_eq!(e.sys.cg(&group(r)).high, Some(None), "{r}");
    }
    let m = e.sys.0.borrow();
    assert_eq!(m.prefetched.len(), 1);
    assert_eq!(m.prefetched[0][0], 24, "gpu process first");
    assert_eq!(m.prefetched[0][1], 20, "then the browser");
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
    assert!(e.sys.0.borrow().reclaimed.is_empty());
    assert!(
        !matches!(e.sys.cg(&group("renderer")).high, Some(Some(_))),
        "no ceiling"
    );
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
    assert!(e.sys.0.borrow().reclaimed.is_empty());
}

#[test]
fn memory_pressure_in_a_saved_app_lets_go_of_its_ceilings() {
    let mut e = adopted();
    e.snapshot(snap(99, false));
    e.tick(1_000);
    e.tick(31_000);
    e.sys.set(&group("renderer"), |c| c.psi = 40.0);
    let st = e.tick(33_000);
    assert_eq!(st.apps[0].state, AppState::Released);
    for r in ["renderer", "main", "gpu", "zygote"] {
        assert_eq!(e.sys.cg(&group(r)).high, Some(None), "{r}");
    }
}

#[test]
fn busy_renderers_at_their_ceiling_get_more_room() {
    let mut e = adopted();
    e.snapshot(snap(99, false));
    e.tick(1_000);
    e.tick(31_000);
    for (i, t) in (32_000..=40_000).step_by(1_000).enumerate() {
        // 20 % of a core: 200 ms of CPU per second.
        e.sys
            .set(&group("renderer"), |c| c.cpu = (i as u64 + 1) * 200_000);
        e.tick(t);
    }
    assert_eq!(
        e.sys.cg(&group("renderer")).high,
        Some(Some(96 * MIB)),
        "2 x (40 + 8)"
    );
}

#[test]
fn processes_started_from_the_dock_are_adopted_at_launch_but_saved_only_once_they_show_a_window() {
    let m = Model::default();
    m.add(10, 1, "Hyprland", "Hyprland");
    m.add(11, 10, "wol-realtime", "wol-realtime");
    m.add(12, 10, "wol-shell", "wol-shell");
    m.add(30, 12, "slack", "/opt/slack/slack");
    m.add(40, 10, "pipewire", "pipewire");
    let mut e = Engine::new(m, Config::default(), Tune::default());
    assert!(e.connect(1000, 11));
    e.snapshot(Snapshot {
        enabled: true,
        ..Snapshot::default()
    });
    e.tick(0);
    let slack = "/user.slice/user-1000.slice/app-wol-slack-30.scope";
    for (pid, why) in [(12, "shell"), (40, "audio"), (11, "agent")] {
        assert_eq!(e.sys.where_is(pid), SESSION, "{why} never moves");
    }
    assert_eq!(e.sys.where_is(30), format!("{slack}/native"));
    e.sys
        .set(&format!("{slack}/native"), |c| c.current = 400 * MIB);
    let st = e.tick(600_000);
    assert!(st.apps.is_empty(), "no window yet: not managed");
    assert!(e.sys.0.borrow().reclaimed.is_empty());
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
    e.snapshot(snap(99, false));
    e.tick(1_000);
    e.tick(31_000);
    let st = e.disconnect(32_000);
    assert!(!st.enabled);
    assert_eq!(e.sys.cg(&group("renderer")).high, Some(None));
}

#[test]
fn app_nap_freezes_hidden_idle_apps_and_thaws_them_on_focus() {
    let mut e = adopted();
    let mut s = snap(99, false);
    s.nap = true;
    e.snapshot(s.clone());
    e.tick(1_000);
    e.tick(31_000);
    let st = e.tick(11 * 60_000);
    assert_eq!(st.apps[0].state, AppState::Napping);
    assert!(e.sys.cg(SCOPE).frozen);
    s.front = 20;
    e.snapshot(s);
    e.tick(11 * 60_000 + 10);
    assert!(!e.sys.cg(SCOPE).frozen);
}
