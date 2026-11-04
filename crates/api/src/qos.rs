// SPDX-License-Identifier: GPL-2.0-or-later
//! QoS service: applies `shared::qos::QosClass` policy; frontmost app boosting from wol-realtime.
//! It also keeps scx_wolos's class grants (audit A19/S11, kernel/scx_wolos/INTERFACE.md): the
//! compositor and the shell may take the interactive class, the frontmost app the foreground
//! class, and key 0 says grants are in use. Without the pinned map (stock kernel, scx_wolos not
//! running) the grant calls do nothing.
use shared::qos::{ForegroundTracker, QosClass, SchedPolicy, parse_foreground};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};

/// Sets a process's scheduling class. Implementations must not allocate on success.
pub trait Applier: Send + Sync {
    fn apply(&self, pid: i32, class: QosClass) -> bool;
    /// Tell the GPU driver the frontmost pid (i915 foreground-only waitboost, ADR 0008). Best effort.
    fn gpu_foreground(&self, _pid: i32) {}
}

/// Linux mapping of a class, per thread: (scheduler, nice, I/O class, slice). The FreeBSD realtime class
/// has no safe Linux twin (SCHED_RR can starve the compositor): it becomes the strongest nice
/// boost until a sched_ext scheduler takes QoS over (ADR 0016).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Linux {
    /// SCHED_OTHER or SCHED_IDLE.
    pub idle: bool,
    pub nice: i8,
    /// ioprio_set value: IOPRIO_CLASS_IDLE, or best effort with the level nice implies.
    pub ioprio: i32,
    /// EEVDF request size (`sched_attr.sched_runtime`), 0 = kernel default. Without scx_wolos.
    pub slice_ns: u64,
}

const IOPRIO_CLASS_SHIFT: i32 = 13;
const IOPRIO_CLASS_BE: i32 = 2;
const IOPRIO_CLASS_IDLE: i32 = 3;
/// Strongest boost for UserInteractive (FreeBSD rtprio).
pub const INTERACTIVE_NICE: i8 = -15;
/// Slice of the two boosted classes: a shorter request waits less after every wakeup, and 2 ms
/// still fits a frame's work in one go. Longer slices on nice 0 made heavy frames later.
pub const BOOST_SLICE_NS: u64 = 2_000_000;

pub fn linux(class: QosClass) -> Linux {
    let be = |nice: i8| (IOPRIO_CLASS_BE << IOPRIO_CLASS_SHIFT) | ((i32::from(nice) + 20) / 5);
    match class.policy() {
        SchedPolicy::RtPrio(_) => Linux {
            idle: false,
            nice: INTERACTIVE_NICE,
            ioprio: be(INTERACTIVE_NICE),
            slice_ns: BOOST_SLICE_NS,
        },
        SchedPolicy::Nice(n) => Linux {
            idle: false,
            nice: n,
            ioprio: be(n),
            slice_ns: if n < 0 { BOOST_SLICE_NS } else { 0 },
        },
        SchedPolicy::IdPrio(_) => Linux {
            idle: true,
            nice: 19,
            ioprio: IOPRIO_CLASS_IDLE << IOPRIO_CLASS_SHIFT,
            slice_ns: 0,
        },
    }
}

/// WolKernel hint interface (README.wolos.md in the kernel tree); absent on a stock kernel: no-op.
/// `foreground_pid` feeds the i915 waitboost (only the frontmost app raises GPU clocks) and the OOM victim bias.
pub const HINTS: &str = "/sys/kernel/wolos";

/// Session policy wold sets at start: the frontmost app runs at least at half capacity with exact
/// timers (Wine/Proton frame limiters wake on time); background threads coalesce wakeups by 50 ms;
/// its busiest thread keeps a core to itself.
pub const HINT_DEFAULTS: [(&str, i64); 4] = [
    ("foreground_uclamp_min", 512),
    ("foreground_timerslack_ns", 0),
    ("background_timerslack_ns", 50_000_000),
    ("shield", 1),
];

/// Write one hint (an existing file only; sysfs never creates). True when written.
pub fn hint(dir: &Path, name: &str, value: i64) -> bool {
    use std::io::Write;
    let mut buf = [0u8; 24];
    let mut w = &mut buf[..];
    if write!(w, "{value}").is_err() {
        return false;
    }
    let n = 24 - w.len();
    std::fs::OpenOptions::new()
        .write(true)
        .open(dir.join(name))
        .and_then(|mut f| f.write_all(&buf[..n]))
        .is_ok()
}

/// Apply [`HINT_DEFAULTS`]; returns how many the kernel took.
pub fn hint_defaults(dir: &Path) -> usize {
    HINT_DEFAULTS
        .iter()
        .filter(|(f, v)| hint(dir, f, *v))
        .count()
}

/// Real policy: every thread of the process (Linux priorities are per thread).
pub struct Sched;

impl Applier for Sched {
    fn apply(&self, pid: i32, class: QosClass) -> bool {
        if pid <= 0 {
            return false;
        }
        let l = linux(class);
        let mut any = false;
        let ok = for_each_thread(pid, |tid| {
            any = true;
            set_thread(tid, l)
        });
        let oom = oom_adj(class).is_none_or(|adj| set_oom_adj(pid, adj));
        grant_for(&PinnedMap(GRANTS_PIN), &SESSION_GRANTS, pid, class);
        ok && any && oom
    }

    fn gpu_foreground(&self, pid: i32) {
        hint(Path::new(HINTS), "foreground_pid", pid.into());
    }
}

/// Per process (not per thread). Stack buffers only: no allocation.
fn set_oom_adj(pid: i32, adj: i16) -> bool {
    use std::io::Write;
    let (mut path, mut val) = ([0u8; 40], [0u8; 8]);
    let mut p = &mut path[..];
    let mut v = &mut val[..];
    if write!(p, "/proc/{pid}/oom_score_adj").is_err() || write!(v, "{adj}").is_err() {
        return false;
    }
    let (pn, vn) = (40 - p.len(), 8 - v.len());
    let Ok(path) = std::str::from_utf8(&path[..pn]) else {
        return false;
    };
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|mut f| f.write_all(&val[..vn]))
        .is_ok()
}

/// `struct sched_attr`, first version; the kernel leaves the uclamp fields alone.
#[repr(C)]
#[derive(Default)]
struct SchedAttr {
    size: u32,
    policy: u32,
    flags: u64,
    nice: i32,
    priority: u32,
    runtime: u64,
    deadline: u64,
    period: u64,
}

fn set_thread(tid: i32, l: Linux) -> bool {
    let attr = SchedAttr {
        size: size_of::<SchedAttr>() as u32,
        policy: if l.idle {
            libc::SCHED_IDLE
        } else {
            libc::SCHED_OTHER
        } as u32,
        nice: l.nice.into(),
        runtime: l.slice_ns,
        ..SchedAttr::default()
    };
    // SAFETY: plain syscalls with integer arguments and a valid sched_attr. setpriority stays for
    // SCHED_IDLE, which ignores sched_nice.
    unsafe {
        libc::syscall(libc::SYS_sched_setattr, tid, &raw const attr, 0u32) == 0
            && libc::setpriority(libc::PRIO_PROCESS, tid as _, i32::from(l.nice)) == 0
            && libc::syscall(
                libc::SYS_ioprio_set,
                1, /* IOPRIO_WHO_PROCESS */
                tid,
                l.ioprio,
            ) == 0
    }
}

/// Calls `f` for every thread id in /proc/<pid>/task; false when the directory cannot be read or
/// any call failed. Does not allocate (opendir/readdir with a stack path).
fn for_each_thread(pid: i32, mut f: impl FnMut(i32) -> bool) -> bool {
    use std::io::Write;
    let mut path = [0u8; 32];
    let mut w = &mut path[..];
    if write!(w, "/proc/{pid}/task\0").is_err() {
        return false;
    }
    // SAFETY: path is NUL-terminated; the DIR is closed below; dirent names are NUL-terminated.
    unsafe {
        let dir = libc::opendir(path.as_ptr().cast());
        if dir.is_null() {
            return false;
        }
        let mut ok = true;
        loop {
            let e = libc::readdir(dir);
            if e.is_null() {
                break;
            }
            let name = std::ffi::CStr::from_ptr((*e).d_name.as_ptr()).to_bytes();
            if let Some(tid) = std::str::from_utf8(name)
                .ok()
                .and_then(|s| s.parse::<i32>().ok())
            {
                ok &= f(tid);
            }
        }
        libc::closedir(dir);
        ok
    }
}

/// Apply one foreground switch: previous app -> Default, new one -> UserInteractive. `mine` says
/// whether a pid still belongs to the account that reported it (S4-1): a previous app whose pid
/// now names another account's process is left alone.
pub fn foreground(
    tracker: &mut ForegroundTracker,
    pid: i32,
    mine: &dyn Fn(i32) -> bool,
    applier: &dyn Applier,
) {
    let (demoted, promoted) = tracker.switch(pid);
    if demoted > 0 && mine(demoted) && !applier.apply(demoted, QosClass::Default) {
        eprintln!("wold: qos pid {demoted} -> default failed");
    }
    if promoted > 0 {
        if !applier.apply(promoted, QosClass::UserInteractive) {
            eprintln!("wold: qos pid {promoted} -> user-interactive failed");
        }
        applier.gpu_foreground(promoted);
    }
}

/// Accounts whose frontmost app is tracked at once (one per seat, with room to spare). One more
/// takes the place of the account seen longest ago.
pub const SEATS: usize = 64;

/// The frontmost app of each account (audit S4-1): one account's focus change demotes only its
/// own previous app, never another seat's. Room for [`SEATS`] is made once, up front, so the hot
/// path never allocates.
pub struct Foregrounds {
    seats: Vec<(u32, ForegroundTracker)>,
}

impl Default for Foregrounds {
    fn default() -> Self {
        Self {
            seats: Vec::with_capacity(SEATS),
        }
    }
}

impl Foregrounds {
    /// The tracker of account `uid`, made on first use.
    pub fn of(&mut self, uid: u32) -> &mut ForegroundTracker {
        let at = match self.seats.iter().position(|(u, _)| *u == uid) {
            Some(i) => i,
            None => {
                if self.seats.len() >= SEATS {
                    self.seats.remove(0); // shifts in place: no allocation
                }
                self.seats.push((uid, ForegroundTracker::default()));
                self.seats.len() - 1
            }
        };
        &mut self.seats[at].1
    }
}

/// Serve one connection of account `uid` (from the kernel): `foreground <pid>` lines until EOF.
/// A pid is acted on only when `owner` says it belongs to `uid` (audit S4-1: the peer check is
/// the executable, and any account may run it against an IPC of its own that names any pid).
/// One fixed buffer; no allocation.
pub fn serve(
    mut conn: impl Read,
    uid: u32,
    owner: &dyn Fn(i32) -> Option<u32>,
    foregrounds: &std::sync::Mutex<Foregrounds>,
    applier: &dyn Applier,
) {
    let mine = |pid: i32| owner(pid) == Some(uid);
    let mut buf = [0u8; 256];
    let mut filled = 0;
    loop {
        let n = match conn.read(&mut buf[filled..]) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        filled += n;
        while let Some(nl) = buf[..filled].iter().position(|b| *b == b'\n') {
            if let Some(pid) = parse_foreground(&buf[..nl])
                && mine(pid)
            {
                let mut f = foregrounds.lock().unwrap_or_else(|p| p.into_inner());
                foreground(f.of(uid), pid, &mine, applier);
            }
            buf.copy_within(nl + 1..filled, 0);
            filled -= nl + 1;
        }
        if filled == buf.len() {
            filled = 0; // overlong line: drop it
        }
    }
}

/// OOM bias per class (`/proc/<pid>/oom_score_adj`): Background work dies first. Raising needs no
/// privilege. None: leave the process's value alone.
pub fn oom_adj(class: QosClass) -> Option<i16> {
    (class == QosClass::Background).then_some(300)
}

/// One process as `/proc/<pid>/stat` names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proc {
    pub pid: i32,
    pub ppid: i32,
    /// The kernel's `comm` (15 bytes at most).
    pub comm: Vec<u8>,
}

/// `pid (comm) S ppid ...`: the command may hold spaces and parentheses, so it runs to the last
/// `)`.
fn parse_stat(text: &[u8]) -> Option<Proc> {
    let open = text.iter().position(|b| *b == b'(')?;
    let close = text.iter().rposition(|b| *b == b')')?;
    let pid = std::str::from_utf8(text.get(..open)?)
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let rest = std::str::from_utf8(text.get(close + 1..)?).ok()?;
    let ppid = rest.split_whitespace().nth(1)?.parse().ok()?;
    Some(Proc {
        pid,
        ppid,
        comm: text.get(open + 1..close)?.to_vec(),
    })
}

/// Every process under `proc_dir`, from ONE read of `stat` each. The 5 s session tick used to
/// read `comm` of every pid three times over (protect, grant, precompile: 144 reads a second on a
/// small desktop, H17 R2-03b); now it reads each pid once and every step works on this list.
pub fn scan(proc_dir: &Path) -> Vec<Proc> {
    std::fs::read_dir(proc_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.parse::<u32>().is_ok())
        })
        .filter_map(|e| parse_stat(&std::fs::read(e.path().join("stat")).ok()?))
        .collect()
}

/// The processes of `procs` whose `comm` is one of `comms`.
fn named<'a>(procs: &'a [Proc], comms: &'a [&[u8]]) -> impl Iterator<Item = &'a Proc> + 'a {
    procs
        .iter()
        .filter(move |p| comms.contains(&p.comm.as_slice()))
}

/// Session processes the OOM killer must spare (a dead compositor ends the whole session).
pub const SESSION_COMMS: [&[u8]; 2] = [b"Hyprland", b"wol-shell"];
pub const SESSION_OOM_ADJ: i16 = -900;

/// Give every [`SESSION_COMMS`] process in `procs` [`SESSION_OOM_ADJ`]; idempotent (only writes
/// values that differ). Returns how many were written.
pub fn protect_session(proc_dir: &Path, procs: &[Proc]) -> usize {
    let want = SESSION_OOM_ADJ.to_string();
    named(procs, &SESSION_COMMS)
        .filter(|p| {
            let adj = proc_dir.join(p.pid.to_string()).join("oom_score_adj");
            std::fs::read_to_string(&adj).is_ok_and(|v| v.trim() != want)
                && std::fs::write(&adj, &want).is_ok()
        })
        .count()
}

/// `oom_score_adj` is inherited by fork: everything the compositor or the shell starts (every app
/// from the launcher, every `exec-once`) would sit at [`SESSION_OOM_ADJ`] too, shielded like the
/// session itself while logind, seatd and wold sit at 0 and are killed before a memory hog (QA
/// L2-30). Every process in `procs` that carries exactly that value and is neither a session
/// process nor wold itself (whose unit sets it) goes back to 0. Looked at by value, not by
/// ancestry: an app that double-forked and was reparented to init or `systemd --user` keeps the
/// value for good otherwise. A value somebody chose (anything else) is left alone. Raising needs
/// no privilege. Returns how many.
pub fn reset_inherited_oom_adj(proc_dir: &Path, procs: &[Proc]) -> usize {
    let want = SESSION_OOM_ADJ.to_string();
    // A pid always fits (pid_max is at most 2^22); one that did not would match no process.
    let wold = i32::try_from(std::process::id()).unwrap_or(-1);
    procs
        .iter()
        .filter(|p| p.pid != wold && !SESSION_COMMS.contains(&p.comm.as_slice()))
        .filter(|p| {
            let adj = proc_dir.join(p.pid.to_string()).join("oom_score_adj");
            std::fs::read_to_string(&adj).is_ok_and(|v| v.trim() == want)
                && std::fs::write(&adj, "0").is_ok()
        })
        .count()
}

/// wold's 5 s session tick: one pass over `proc_dir`, then the compositor and shell are protected
/// and given their scheduler grant, what they started is not protected with them, and shader
/// precompilers go to the background (the session starts after wold: re-checking is idempotent).
pub fn session_tick(proc_dir: &Path) {
    let procs = scan(proc_dir);
    protect_session(proc_dir, &procs);
    reset_inherited_oom_adj(proc_dir, &procs);
    // A fake /proc (tests) names pids that are not real: they get no grant.
    if proc_dir == Path::new("/proc") {
        grant_session(proc_dir, &PinnedMap(GRANTS_PIN), &SESSION_GRANTS, &procs);
    }
    tag_precompile(&procs, &Sched);
}

/// scx_wolos's grants map, pinned by its loader (kernel/scx_wolos/scx_wolos.c), NUL-terminated.
pub const GRANTS_PIN: &[u8] = b"/sys/fs/bpf/wolos/boosted\0";
/// Present: grants are in use (`WOLOS_GRANTS_KEY`, kernel/scx_wolos/scx_wolos_class.h).
pub const GRANTS_KEY: u32 = 0;
/// Highest class a granted process may take: `C_INTERACTIVE`, `C_FOREGROUND`.
pub const GRANT_INTERACTIVE: u32 = 0;
pub const GRANT_FOREGROUND: u32 = 1;
/// Session processes tracked at once (a compositor and a shell per seat, with room to spare).
pub const SESSION_SLOTS: usize = 8;

/// tgid -> class writes into scx_wolos's grants map. Real: [`PinnedMap`]; tests: a fake.
pub trait GrantMap {
    /// Insert or replace; false when the map is absent or refuses.
    fn update(&self, key: u32, value: u32) -> bool;
    /// False when the map is absent or has no such key.
    fn delete(&self, key: u32) -> bool;
}

/// Frontmost-app grants wold keeps track of at once (R2C-11); the oldest goes first past it.
pub const FRONT_TRACKED: usize = 256;

/// The tgids holding the session's interactive grant; the frontmost-app path leaves them alone.
/// Also the frontmost-app grants wold wrote, so the next session tick removes those of processes
/// that ended (R2C-11: a reused tgid never inherits one, and the map never fills up).
pub struct SessionGrants([AtomicI32; SESSION_SLOTS], std::sync::Mutex<Vec<u32>>);

impl SessionGrants {
    pub const fn new() -> Self {
        Self(
            [const { AtomicI32::new(0) }; SESSION_SLOTS],
            std::sync::Mutex::new(Vec::new()),
        )
    }

    fn front(&self) -> std::sync::MutexGuard<'_, Vec<u32>> {
        self.1.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn contains(&self, tgid: i32) -> bool {
        tgid > 0 && self.0.iter().any(|s| s.load(Ordering::Relaxed) == tgid)
    }
}

impl std::default::Default for SessionGrants {
    fn default() -> Self {
        Self::new()
    }
}

static SESSION_GRANTS: SessionGrants = SessionGrants::new();

/// The frontmost app's grant follows its class: boosted classes may take foreground, any other
/// class loses the grant. Session processes keep theirs. True when the map took the change.
pub fn grant_for(map: &dyn GrantMap, session: &SessionGrants, pid: i32, class: QosClass) -> bool {
    let Ok(tgid) = u32::try_from(pid) else {
        return false;
    };
    if tgid == GRANTS_KEY || session.contains(pid) {
        return false;
    }
    match class {
        QosClass::UserInteractive | QosClass::UserInitiated => {
            if !map.update(tgid, GRANT_FOREGROUND) {
                return false;
            }
            let mut front = session.front();
            if !front.contains(&tgid) {
                if front.len() >= FRONT_TRACKED {
                    let oldest = front.remove(0);
                    map.delete(oldest);
                }
                front.push(tgid);
            }
            true
        }
        _ => {
            session.front().retain(|t| *t != tgid);
            map.delete(tgid)
        }
    }
}

/// The compositor and the shell under `proc_dir` get the interactive grant, ones that ended lose
/// it, and key 0 turns grants on once one is written (so the compositor is never capped by its
/// own missing grant). Returns how many grants were written.
pub fn grant_session(
    proc_dir: &Path,
    map: &dyn GrantMap,
    session: &SessionGrants,
    procs: &[Proc],
) -> usize {
    let now: Vec<i32> = named(procs, &SESSION_COMMS)
        .map(|p| p.pid)
        .filter(|p| *p > 0)
        .take(SESSION_SLOTS)
        .collect();
    for slot in &session.0 {
        let old = slot.load(Ordering::Relaxed);
        if old > 0 && !now.contains(&old) {
            map.delete(old.unsigned_abs());
            slot.store(0, Ordering::Relaxed);
        }
    }
    let mut written = 0;
    for pid in now {
        if !map.update(pid.unsigned_abs(), GRANT_INTERACTIVE) {
            continue;
        }
        written += 1;
        if !session.contains(pid)
            && let Some(free) = session.0.iter().find(|s| s.load(Ordering::Relaxed) == 0)
        {
            free.store(pid, Ordering::Relaxed);
        }
    }
    // R2C-11: frontmost-app grants of processes that ended go.
    session.front().retain(|t| {
        let alive = proc_dir.join(t.to_string()).exists();
        if !alive {
            map.delete(*t);
        }
        alive
    });
    if written > 0 {
        map.update(GRANTS_KEY, 0);
    } else if session.0.iter().all(|s| s.load(Ordering::Relaxed) == 0) {
        // No session process: grants are off until one runs again (R2C-11).
        map.delete(GRANTS_KEY);
    }
    written
}

/// `BPF_MAP_TYPE_HASH`.
const BPF_MAP_TYPE_HASH: u32 = 1;

/// The grants map scx_wolos pins: a hash of 4-byte tgids to 4-byte grants. wold writes 4-byte
/// keys and values, so a map of any other shape (a version skew) is never written (R2C-11).
pub fn grants_shape_ok(map_type: u32, key_size: u32, value_size: u32) -> bool {
    map_type == BPF_MAP_TYPE_HASH && key_size == 4 && value_size == 4
}

/// A BPF map pinned in bpffs, opened for each write: the loader re-pins a new map when it restarts,
/// so a kept descriptor would write into one nobody reads. No allocation.
pub struct PinnedMap(pub &'static [u8]);

const BPF_MAP_UPDATE_ELEM: libc::c_long = 2;
const BPF_MAP_DELETE_ELEM: libc::c_long = 3;
const BPF_OBJ_GET: libc::c_long = 7;
const BPF_OBJ_GET_INFO_BY_FD: libc::c_long = 15;

/// `union bpf_attr` for BPF_OBJ_GET_INFO_BY_FD.
#[repr(C, align(8))]
struct InfoByFd {
    bpf_fd: u32,
    info_len: u32,
    info: u64,
}

/// The head of `struct bpf_map_info`; the kernel fills no more than `info_len` bytes.
#[repr(C)]
#[derive(Default)]
struct MapInfoHead {
    map_type: u32,
    id: u32,
    key_size: u32,
    value_size: u32,
    max_entries: u32,
    map_flags: u32,
}

/// `union bpf_attr` for BPF_OBJ_GET.
#[repr(C, align(8))]
struct ObjGet {
    pathname: u64,
    bpf_fd: u32,
    file_flags: u32,
}

/// `union bpf_attr` for the element commands.
#[repr(C, align(8))]
struct Elem {
    map_fd: u32,
    pad: u32,
    key: u64,
    value: u64,
    flags: u64,
}

impl PinnedMap {
    fn with(&self, f: impl FnOnce(u32) -> bool) -> bool {
        if self.0.last() != Some(&0) {
            return false;
        }
        let attr = ObjGet {
            pathname: self.0.as_ptr() as u64,
            bpf_fd: 0,
            file_flags: 0,
        };
        // SAFETY: bpf(2) reads a NUL-terminated path from `pathname`, which outlives the call.
        let fd = unsafe {
            libc::syscall(
                libc::SYS_bpf,
                BPF_OBJ_GET,
                &raw const attr,
                size_of::<ObjGet>() as u32,
            )
        };
        let Ok(fd) = i32::try_from(fd) else {
            return false;
        };
        if fd < 0 {
            return false;
        }
        let ok = Self::shape_ok(fd.unsigned_abs()) && f(fd.unsigned_abs());
        // SAFETY: closes the descriptor opened above.
        unsafe { libc::close(fd) };
        ok
    }

    /// Is the map behind `fd` the shape wold writes ([`grants_shape_ok`])?
    fn shape_ok(fd: u32) -> bool {
        let mut info = MapInfoHead::default();
        let attr = InfoByFd {
            bpf_fd: fd,
            info_len: size_of::<MapInfoHead>() as u32,
            info: std::ptr::from_mut(&mut info) as u64,
        };
        // SAFETY: bpf(2) writes at most `info_len` bytes into `info`, which outlives the call.
        let r = unsafe {
            libc::syscall(
                libc::SYS_bpf,
                BPF_OBJ_GET_INFO_BY_FD,
                &raw const attr,
                size_of::<InfoByFd>() as u32,
            )
        };
        r == 0 && grants_shape_ok(info.map_type, info.key_size, info.value_size)
    }

    fn elem(fd: u32, cmd: libc::c_long, key: &u32, value: Option<&u32>) -> bool {
        let attr = Elem {
            map_fd: fd,
            pad: 0,
            key: std::ptr::from_ref(key) as u64,
            value: value.map_or(0, |v| std::ptr::from_ref(v) as u64),
            flags: 0, // BPF_ANY
        };
        // SAFETY: bpf(2) reads 4-byte key and value from pointers that outlive the call.
        unsafe {
            libc::syscall(
                libc::SYS_bpf,
                cmd,
                &raw const attr,
                size_of::<Elem>() as u32,
            ) == 0
        }
    }
}

impl GrantMap for PinnedMap {
    fn update(&self, key: u32, value: u32) -> bool {
        self.with(|fd| Self::elem(fd, BPF_MAP_UPDATE_ELEM, &key, Some(&value)))
    }

    fn delete(&self, key: u32) -> bool {
        self.with(|fd| Self::elem(fd, BPF_MAP_DELETE_ELEM, &key, None))
    }
}

/// Shader precompilers, as the kernel truncates them in `comm`.
pub const PRECOMPILE_COMMS: [&[u8]; 1] = [b"fossilize_repla"];

/// Put every [`PRECOMPILE_COMMS`] process in `procs` in [`QosClass::Background`]. Re-applying is
/// harmless and threads inherit it. Returns how many were tagged.
pub fn tag_precompile(procs: &[Proc], applier: &dyn Applier) -> usize {
    named(procs, &PRECOMPILE_COMMS)
        .filter(|p| applier.apply(p.pid, QosClass::Background))
        .count()
}

/// Only the realtime agent may boost (canonical executable path of the peer).
pub const REALTIME_EXE: &str = "/usr/bin/wol-realtime";

pub fn peer_allowed(peer_exe: &[u8], allowed: &str) -> bool {
    peer_exe == allowed.as_bytes()
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Recorder(pub Mutex<Vec<(i32, QosClass)>>);

    impl Applier for Recorder {
        fn apply(&self, pid: i32, class: QosClass) -> bool {
            if let Ok(mut v) = self.0.try_lock()
                && v.len() < v.capacity()
            {
                v.push((pid, class));
            }
            true
        }
    }

    /// Every pid is this account's (uid 1000).
    pub fn ours(_: i32) -> Option<u32> {
        Some(1000)
    }

    #[test]
    fn serve_applies_switches_in_order() {
        let r = Recorder(Mutex::new(Vec::with_capacity(16)));
        let t = Mutex::new(Foregrounds::default());
        let input: &[u8] = b"foreground 10\nforeground 10\ngarbage\nforeground 20\nforeground 3";
        serve(input, 1000, &ours, &t, &r);
        use QosClass::*;
        assert_eq!(
            *r.0.lock().unwrap(),
            vec![(10, UserInteractive), (10, Default), (20, UserInteractive)]
        );
    }

    #[derive(Default)]
    struct GpuRecorder(Mutex<Vec<i32>>);
    impl Applier for GpuRecorder {
        fn apply(&self, _: i32, _: QosClass) -> bool {
            true
        }
        fn gpu_foreground(&self, pid: i32) {
            self.0.lock().unwrap().push(pid);
        }
    }

    #[test]
    fn every_switch_tells_the_gpu_driver_the_frontmost_pid() {
        let r = GpuRecorder::default();
        let t = Mutex::new(Foregrounds::default());
        serve(
            &b"foreground 10\nforeground 20\nforeground 0\n"[..],
            1000,
            &ours,
            &t,
            &r,
        );
        assert_eq!(*r.0.lock().unwrap(), vec![10, 20]);
    }

    #[test]
    fn kernel_hints_name_the_frontmost_app_and_set_the_session_policy() {
        let dir = std::env::temp_dir().join(format!("wol-hints-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (f, _) in HINT_DEFAULTS {
            std::fs::write(dir.join(f), "").unwrap();
        }
        std::fs::write(dir.join("foreground_pid"), "").unwrap();
        assert_eq!(hint_defaults(&dir), HINT_DEFAULTS.len());
        assert_eq!(
            std::fs::read_to_string(dir.join("foreground_uclamp_min")).unwrap(),
            "512"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("foreground_timerslack_ns")).unwrap(),
            "0"
        );
        assert_eq!(std::fs::read_to_string(dir.join("shield")).unwrap(), "1");
        assert_eq!(
            std::fs::read_to_string(dir.join("background_timerslack_ns")).unwrap(),
            "50000000"
        );
        assert!(hint(&dir, "foreground_pid", 4242));
        assert_eq!(
            std::fs::read_to_string(dir.join("foreground_pid")).unwrap(),
            "4242"
        );
        // A stock kernel has no /sys/kernel/wolos: nothing is written, nothing fails.
        let none = dir.join("absent");
        assert_eq!(hint_defaults(&none), 0);
        assert!(!hint(&none, "foreground_pid", 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn overlong_lines_are_dropped() {
        let r = Recorder(Mutex::new(Vec::with_capacity(4)));
        let t = Mutex::new(Foregrounds::default());
        let mut input = vec![b'x'; 300];
        input.extend_from_slice(b"\nforeground 7\n");
        serve(&input[..], 1000, &ours, &t, &r);
        assert_eq!(*r.0.lock().unwrap(), vec![(7, QosClass::UserInteractive)]);
    }

    #[test]
    fn linux_class_table() {
        use QosClass::*;
        let be = |level: i32| (2 << 13) | level;
        let ms = 1_000_000;
        for (c, idle, nice, io, slice_ns) in [
            (UserInteractive, false, -15, be(1), 2 * ms),
            (UserInitiated, false, -10, be(2), 2 * ms),
            (Default, false, 0, be(4), 0),
            (Utility, false, 10, be(6), 0),
            (Background, true, 19, 3 << 13, 0),
        ] {
            assert_eq!(
                linux(c),
                Linux {
                    idle,
                    nice,
                    ioprio: io,
                    slice_ns
                },
                "{c:?}"
            );
        }
    }

    /// Lowering a child needs no privilege: every thread gets the nice value, Background is
    /// SCHED_IDLE. A missing pid fails.
    #[test]
    fn sched_lowers_a_real_child() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let utility = Sched.apply(pid, QosClass::Utility);
        // SAFETY: plain syscalls.
        let nice = unsafe { libc::getpriority(libc::PRIO_PROCESS, pid as _) };
        let background = Sched.apply(pid, QosClass::Background);
        let policy = unsafe { libc::sched_getscheduler(pid) };
        let oom = std::fs::read_to_string(format!("/proc/{pid}/oom_score_adj")).unwrap_or_default();
        child.kill().ok();
        child.wait().ok();
        assert!(utility && nice == 10, "utility={utility} nice={nice}");
        assert!(background && policy == libc::SCHED_IDLE, "policy={policy}");
        assert_eq!(oom.trim(), "300", "Background raises oom_score_adj");
        assert!(!Sched.apply(i32::MAX, QosClass::Utility));
        assert!(!Sched.apply(0, QosClass::Utility));
    }

    /// The slice reaches EEVDF and a class without one drops it. A kernel before 6.12 ignores it.
    #[test]
    fn slice_is_set_and_cleared_on_a_real_child() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let quiet = linux(QosClass::Default);
        let default = runtime_of(pid);
        let set = set_thread(
            pid,
            Linux {
                slice_ns: 2_000_000,
                ..quiet
            },
        );
        let with = runtime_of(pid);
        let cleared = set_thread(pid, quiet);
        let without = runtime_of(pid);
        child.kill().ok();
        child.wait().ok();
        assert!(set && cleared);
        assert_eq!((with, without), (2_000_000, default));
    }

    fn runtime_of(tid: i32) -> u64 {
        let mut a = SchedAttr::default();
        // SAFETY: a is a valid buffer of the size passed.
        let r = unsafe {
            libc::syscall(
                libc::SYS_sched_getattr,
                tid,
                &raw mut a,
                size_of::<SchedAttr>() as u32,
                0u32,
            )
        };
        assert_eq!(r, 0);
        a.runtime
    }

    #[test]
    fn background_is_first_for_the_oom_killer() {
        use QosClass::*;
        assert_eq!(oom_adj(Background), Some(300));
        for c in [UserInteractive, UserInitiated, Default, Utility] {
            assert_eq!(oom_adj(c), None, "{c:?}");
        }
    }

    /// A `/proc/<pid>/stat` line the way the kernel writes it.
    fn stat_line(pid: &str, ppid: i32, comm: &str) -> String {
        format!(
            "{pid} ({}) S {ppid} {pid} {pid} 0 -1 4194560\n",
            comm.trim()
        )
    }

    /// One fake process: `stat` and `oom_score_adj`.
    fn put_proc(d: &Path, pid: i32, ppid: i32, comm: &str, adj: &str) {
        let dir = d.join(pid.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("stat"), stat_line(&pid.to_string(), ppid, comm)).unwrap();
        std::fs::write(dir.join("oom_score_adj"), format!("{adj}\n")).unwrap();
    }

    /// QA L2-30: the -900 given to the compositor and the shell is inherited by everything they
    /// start; wold hands it back to 0 below them (kernel logind/seatd/wold would die before a
    /// memory hog otherwise), also an app reparented to init. A chosen value, wold and the session
    /// itself stay.
    #[test]
    fn apps_started_by_the_session_do_not_keep_its_oom_protection() {
        let d = std::env::temp_dir().join(format!("wol-qos-inherit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        put_proc(&d, 10, 1, "Hyprland", "-900");
        put_proc(&d, 11, 10, "wol-shell", "-900");
        put_proc(&d, 12, 11, "wol-settings", "-900"); // an app: inherited
        put_proc(&d, 13, 12, "helper", "-900"); // below the app: inherited
        put_proc(&d, 14, 10, "sh", "-500"); // chosen by somebody: kept
        put_proc(&d, 15, 1, "double-forked", "-900"); // reparented to init: inherited all the same
        put_proc(&d, 16, 10, "foot", "0");
        let wold = std::process::id() as i32;
        put_proc(&d, wold, 1, "wold", "-900"); // wold's own unit value stays
        let adj = |pid: i32| {
            std::fs::read_to_string(d.join(pid.to_string()).join("oom_score_adj")).unwrap()
        };
        assert_eq!(reset_inherited_oom_adj(&d, &scan(&d)), 3);
        for (pid, want) in [
            (10, "-900"),
            (11, "-900"),
            (12, "0"),
            (13, "0"),
            (14, "-500"),
            (15, "0"),
            (16, "0"),
        ] {
            assert_eq!(adj(pid).trim(), want, "pid {pid}");
        }
        assert_eq!(adj(wold).trim(), "-900", "wold keeps its unit's value");
        assert_eq!(reset_inherited_oom_adj(&d, &scan(&d)), 0, "idempotent");
        std::fs::remove_dir_all(&d).ok();
    }

    /// `stat` is read once per pid and parsed with commands that hold spaces and parentheses.
    #[test]
    fn stat_names_pid_parent_and_command() {
        let p = parse_stat(b"42 (a (b) c) S 7 42 42 0 -1 4194560 1 2 3").unwrap();
        assert_eq!((p.pid, p.ppid, p.comm.as_slice()), (42, 7, &b"a (b) c"[..]));
        assert_eq!(parse_stat(b"garbage"), None);
        assert_eq!(parse_stat(b"x (y) S 1"), None);
    }

    /// H17 R2-03b: one tick reads each pid's `stat` once, not three scans of `comm`.
    #[test]
    fn a_tick_reads_each_pid_once() {
        let d = std::env::temp_dir().join(format!("wol-qos-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for pid in 1..=20 {
            put_proc(&d, pid, 1, "app", "0");
        }
        // The only file that names a process to the three steps is `stat`: no `comm` file is
        // there to be read, and the tick still finds the compositor.
        put_proc(&d, 30, 1, "Hyprland", "0");
        assert!(!d.join("30").join("comm").exists());
        let procs = scan(&d);
        assert_eq!(procs.len(), 21);
        assert_eq!(protect_session(&d, &procs), 1);
        std::fs::remove_dir_all(&d).ok();
    }

    /// Fake /proc: compositor and shell get -900 once; other processes are untouched.
    #[test]
    fn session_is_protected_from_the_oom_killer() {
        let d = std::env::temp_dir().join(format!("wol-qos-oom-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (pid, comm, adj) in [
            ("10", "Hyprland\n", "0\n"),
            ("11", "wol-shell\n", "0\n"),
            ("12", "atlas\n", "0\n"),
            ("13", "Hyprland\n", "-900\n"),
            ("self", "Hyprland\n", "0\n"),
        ] {
            std::fs::create_dir_all(d.join(pid)).unwrap();
            std::fs::write(d.join(pid).join("comm"), comm).unwrap();
            std::fs::write(d.join(pid).join("stat"), stat_line(pid, 1, comm)).unwrap();
            std::fs::write(d.join(pid).join("oom_score_adj"), adj).unwrap();
        }
        let adj = |pid: &str| std::fs::read_to_string(d.join(pid).join("oom_score_adj")).unwrap();
        assert_eq!(protect_session(&d, &scan(&d)), 2);
        assert_eq!(adj("10").trim(), "-900");
        assert_eq!(adj("11").trim(), "-900");
        assert_eq!(adj("12").trim(), "0");
        assert_eq!(adj("self").trim(), "0", "only numeric pid dirs");
        assert_eq!(protect_session(&d, &scan(&d)), 0, "idempotent");
        std::fs::remove_dir_all(&d).ok();
    }

    /// A grants map in memory (audit S11).
    #[derive(Default)]
    struct FakeMap(Mutex<std::collections::BTreeMap<u32, u32>>);

    impl GrantMap for FakeMap {
        fn update(&self, key: u32, value: u32) -> bool {
            self.0.lock().unwrap().insert(key, value);
            true
        }
        fn delete(&self, key: u32) -> bool {
            self.0.lock().unwrap().remove(&key).is_some()
        }
    }

    /// Compositor and shell get the interactive grant, then key 0; ones that end lose it; the
    /// frontmost app gets foreground and loses it when it goes back; the session keeps its grants.
    #[test]
    fn the_scheduler_hears_who_may_be_interactive() {
        let d = std::env::temp_dir().join(format!("wol-qos-grants-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (pid, comm) in [
            ("10", "Hyprland\n"),
            ("11", "wol-shell\n"),
            ("12", "atlas\n"),
        ] {
            std::fs::create_dir_all(d.join(pid)).unwrap();
            std::fs::write(d.join(pid).join("comm"), comm).unwrap();
            std::fs::write(d.join(pid).join("stat"), stat_line(pid, 1, comm)).unwrap();
        }
        let (map, session) = (FakeMap::default(), SessionGrants::new());
        let grants = || map.0.lock().unwrap().clone();
        assert_eq!(grant_session(&d, &map, &session, &scan(&d)), 2);
        let want: std::collections::BTreeMap<u32, u32> = [
            (GRANTS_KEY, 0),
            (10, GRANT_INTERACTIVE),
            (11, GRANT_INTERACTIVE),
        ]
        .into();
        assert_eq!(grants(), want);
        assert!(grant_for(&map, &session, 12, QosClass::UserInteractive));
        assert_eq!(grants().get(&12), Some(&GRANT_FOREGROUND));
        assert!(grant_for(&map, &session, 12, QosClass::Default));
        assert_eq!(grants().get(&12), None);
        // The shell coming to the front, or going back, keeps its interactive grant.
        assert!(!grant_for(&map, &session, 11, QosClass::UserInteractive));
        assert!(!grant_for(&map, &session, 11, QosClass::Default));
        assert_eq!(grants().get(&11), Some(&GRANT_INTERACTIVE));
        assert!(
            !grant_for(&map, &session, 0, QosClass::UserInteractive),
            "key 0 is the switch"
        );
        assert!(!grant_for(&map, &session, -1, QosClass::UserInteractive));
        // The shell ends: its grant goes, the switch stays.
        std::fs::remove_dir_all(d.join("11")).unwrap();
        assert_eq!(grant_session(&d, &map, &session, &scan(&d)), 1);
        assert_eq!(grants().get(&11), None);
        assert_eq!(grants().get(&GRANTS_KEY), Some(&0));
        assert!(!session.contains(11));
        std::fs::remove_dir_all(&d).ok();
    }

    /// R2C-11: a frontmost app that ended loses its row at the next tick (a reused tgid never
    /// inherits it); with no session process key 0 goes; a map of another shape is never written.
    #[test]
    fn stale_grants_are_removed() {
        let d = std::env::temp_dir().join(format!("wol-qos-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (pid, comm) in [("10", "Hyprland\n"), ("20", "app\n"), ("21", "app\n")] {
            std::fs::create_dir_all(d.join(pid)).unwrap();
            std::fs::write(d.join(pid).join("comm"), comm).unwrap();
            std::fs::write(d.join(pid).join("stat"), stat_line(pid, 1, comm)).unwrap();
        }
        let (map, session) = (FakeMap::default(), SessionGrants::new());
        let grants = || map.0.lock().unwrap().clone();
        assert_eq!(grant_session(&d, &map, &session, &scan(&d)), 1);
        assert!(grant_for(&map, &session, 20, QosClass::UserInteractive));
        assert!(grant_for(&map, &session, 21, QosClass::UserInteractive));
        std::fs::remove_dir_all(d.join("20")).unwrap();
        grant_session(&d, &map, &session, &scan(&d));
        assert_eq!(grants().get(&20), None, "ended while frontmost");
        assert_eq!(grants().get(&21), Some(&GRANT_FOREGROUND));
        assert_eq!(grants().get(&GRANTS_KEY), Some(&0));
        // The session ends: its rows and key 0 go.
        std::fs::remove_dir_all(d.join("10")).unwrap();
        assert_eq!(grant_session(&d, &map, &session, &scan(&d)), 0);
        assert_eq!(grants().get(&GRANTS_KEY), None);
        assert_eq!(grants().get(&10), None);
        // The tracked list never grows past its cap.
        for t in 1000..1000 + FRONT_TRACKED as i32 + 5 {
            grant_for(&map, &session, t, QosClass::UserInteractive);
        }
        assert_eq!(session.front().len(), FRONT_TRACKED);
        assert_eq!(grants().get(&1000), None, "the oldest went with its row");
        assert!(grants_shape_ok(BPF_MAP_TYPE_HASH, 4, 4));
        assert!(!grants_shape_ok(BPF_MAP_TYPE_HASH, 4, 8));
        assert!(!grants_shape_ok(BPF_MAP_TYPE_HASH, 8, 4));
        assert!(!grants_shape_ok(2, 4, 4), "an array is not the grants map");
        std::fs::remove_dir_all(&d).ok();
    }

    /// No session yet: no grant, and key 0 stays off so nothing is capped.
    #[test]
    fn no_session_leaves_grants_off() {
        let d = std::env::temp_dir().join(format!("wol-qos-nogrants-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let (map, session) = (FakeMap::default(), SessionGrants::new());
        assert_eq!(grant_session(&d, &map, &session, &scan(&d)), 0);
        assert!(map.0.lock().unwrap().is_empty());
        std::fs::remove_dir_all(&d).ok();
    }

    /// A machine without scx_wolos (no pin, or not root): every write is a quiet no-op.
    #[test]
    fn an_absent_grants_map_is_silent() {
        let none = PinnedMap(b"/sys/fs/bpf/wolos-test-absent/boosted\0");
        assert!(!none.update(1, GRANT_FOREGROUND));
        assert!(!none.delete(1));
        assert!(!PinnedMap(b"no-nul").update(1, 1));
        let session = SessionGrants::new();
        assert!(!grant_for(&none, &session, 1, QosClass::UserInteractive));
    }

    /// The constants mirror the scheduler: class order, the grants key and the pin path.
    #[test]
    fn grant_constants_match_scx_wolos() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../kernel/scx_wolos/");
        let class = std::fs::read_to_string(format!("{root}scx_wolos_class.h")).unwrap();
        assert!(class.contains("enum { C_INTERACTIVE, C_FOREGROUND,"));
        assert!(
            class
                .lines()
                .any(|l| l.starts_with("#define WOLOS_GRANTS_KEY") && l.contains("\t0\t"))
        );
        let loader = std::fs::read_to_string(format!("{root}scx_wolos.c")).unwrap();
        assert!(loader.contains(r#"#define GRANTS_DIR "/sys/fs/bpf/wolos""#));
        assert!(loader.contains(r#"#define GRANTS_PIN GRANTS_DIR "/boosted""#));
        assert_eq!(GRANTS_PIN, b"/sys/fs/bpf/wolos/boosted\0");
    }

    #[test]
    fn shader_precompile_runs_in_the_background() {
        let d = std::env::temp_dir().join(format!("wol-qos-foz-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (pid, comm) in [
            ("20", "fossilize_repla\n"),
            ("21", "fossilize_repla\n"),
            ("22", "cs2\n"),
            ("self", "fossilize_repla\n"),
        ] {
            std::fs::create_dir_all(d.join(pid)).unwrap();
            std::fs::write(d.join(pid).join("comm"), comm).unwrap();
            std::fs::write(d.join(pid).join("stat"), stat_line(pid, 1, comm)).unwrap();
        }
        let r = Recorder(Mutex::new(Vec::with_capacity(8)));
        assert_eq!(tag_precompile(&scan(&d), &r), 2);
        let mut got = r.0.lock().unwrap().clone();
        got.sort();
        assert_eq!(
            got,
            [(20, QosClass::Background), (21, QosClass::Background)]
        );
        std::fs::remove_dir_all(&d).ok();
    }

    /// Audit S4-1: a peer may only steer its own account's processes. A pid of another account
    /// is never promoted, and a previous app whose pid now names another account is not demoted.
    #[test]
    fn a_peer_steers_only_its_own_accounts_processes() {
        let r = Recorder(Mutex::new(Vec::with_capacity(16)));
        let t = Mutex::new(Foregrounds::default());
        // pid 20 is uid 1001's; the peer is uid 1000.
        let owner = |pid: i32| Some(if pid == 20 { 1001 } else { 1000 });
        serve(&b"foreground 10\nforeground 20\n"[..], 1000, &owner, &t, &r);
        assert_eq!(*r.0.lock().unwrap(), vec![(10, QosClass::UserInteractive)]);
        // The same through a fake /proc and the real owner lookup.
        let d = crate::memsaver::procs::tests::fake(
            "qos-owner",
            &[
                (10, 1, 1000, "app", "/s", "app", ""),
                (20, 1, 1001, "other", "/s", "other", ""),
            ],
        );
        let r = Recorder(Mutex::new(Vec::with_capacity(16)));
        let t = Mutex::new(Foregrounds::default());
        let owner = |pid: i32| crate::memsaver::procs::owner(&d, pid);
        serve(
            &b"foreground 20\nforeground 10\nforeground 30\n"[..],
            1000,
            &owner,
            &t,
            &r,
        );
        assert_eq!(*r.0.lock().unwrap(), vec![(10, QosClass::UserInteractive)]);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Audit S4-1: two seats, two trackers: the second account's switch never demotes the first
    /// account's frontmost app.
    #[test]
    fn one_accounts_switch_never_demotes_another_seat() {
        let r = Recorder(Mutex::new(Vec::with_capacity(16)));
        let t = Mutex::new(Foregrounds::default());
        let owner = |pid: i32| Some(if pid < 100 { 1000 } else { 1001 });
        serve(&b"foreground 10\n"[..], 1000, &owner, &t, &r);
        serve(
            &b"foreground 110\nforeground 120\n"[..],
            1001,
            &owner,
            &t,
            &r,
        );
        use QosClass::*;
        assert_eq!(
            *r.0.lock().unwrap(),
            vec![
                (10, UserInteractive),
                (110, UserInteractive),
                (110, Default),
                (120, UserInteractive)
            ],
            "pid 10 stays in front for uid 1000"
        );
        // A previous app whose pid was reused by another account is left alone.
        let r = Recorder(Mutex::new(Vec::with_capacity(16)));
        let t = Mutex::new(Foregrounds::default());
        let reused = std::sync::atomic::AtomicBool::new(false);
        let owner = |pid: i32| {
            Some(
                if pid == 10 && reused.load(std::sync::atomic::Ordering::Relaxed) {
                    1001
                } else {
                    1000
                },
            )
        };
        serve(&b"foreground 10\n"[..], 1000, &owner, &t, &r);
        reused.store(true, std::sync::atomic::Ordering::Relaxed);
        serve(&b"foreground 11\n"[..], 1000, &owner, &t, &r);
        assert_eq!(
            *r.0.lock().unwrap(),
            vec![(10, UserInteractive), (11, UserInteractive)]
        );
    }

    #[test]
    fn seats_are_bounded_and_kept_per_account() {
        let mut f = Foregrounds::default();
        let n = u32::try_from(SEATS).unwrap();
        for uid in 0..n + 10 {
            assert_eq!(f.of(uid).switch(7), (0, 7));
        }
        assert_eq!(f.seats.len(), SEATS);
        assert!(f.seats.capacity() <= SEATS, "never grew");
        // The newest seats kept their state.
        assert_eq!(f.of(n + 9).switch(7), (0, 0));
    }

    #[test]
    fn peer_check() {
        assert!(peer_allowed(b"/usr/bin/wol-realtime", REALTIME_EXE));
        assert!(!peer_allowed(b"/tmp/wol-realtime", REALTIME_EXE));
    }

    #[test]
    fn hot_path_is_fast() {
        let r = Recorder(Mutex::new(Vec::new())); // capacity 0: records nothing, never allocates
        let t = Mutex::new(Foregrounds::default());
        let mut input = Vec::new();
        for i in 0..10_000 {
            input.extend_from_slice(format!("foreground {}\n", 100 + i % 2).as_bytes());
        }
        let start = std::time::Instant::now();
        serve(&input[..], 1000, &ours, &t, &r);
        assert!(
            start.elapsed() < std::time::Duration::from_millis(50),
            "{:?}",
            start.elapsed()
        );
    }
}
