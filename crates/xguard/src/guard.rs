// SPDX-License-Identifier: GPL-2.0-or-later
//! The first-use guard: a small process that the kernel asks only about the first execute of a
//! file outside the sealed system, and the first open of a download.
//!
//! - Execute: a fanotify permission mark on the mounts in scope. Opens, reads and stats on those
//!   mounts raise no event; the system volume has no mark at all.
//! - After a clean verdict the file gets an ignore mark: the kernel stops asking, and drops the
//!   mark itself when the file is written. The verdict store answers after the mark is evicted.
//! - Downloads: files closed after writing in a Downloads folder get the quarantine tag and an
//!   open mark of their own, removed once checked.
//!
//! Anything that goes wrong here allows the file: a guard must never stop the computer from
//! working. One exception (audit S4): a download whose deep check could not run at all (the
//! worker died, or its rule sets do not load) is refused for now and not remembered, so the next
//! open asks again; the reason goes to the log. When this process ends, the kernel lets every
//! waiting call through.
use crate::ctl::{self, Reply};
use crate::scan::{Engine, kind, now};
use crate::store::{Entry, FileId, State, Store};
use crate::{Kind, hexs};
use md5::Md5;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use wol_core::malware::{Blocked, findings_file};

const FAN_OPEN_PERM: u64 = 0x0001_0000;
const FAN_OPEN_EXEC_PERM: u64 = 0x0004_0000;
const FAN_CLOSE_WRITE: u64 = 0x0000_0008;
const FAN_EVENT_ON_CHILD: u64 = 0x0800_0000;
const FAN_MARK_ADD: u32 = 1;
const FAN_MARK_REMOVE: u32 = 2;
const FAN_MARK_MOUNT: u32 = 0x10;
const FAN_MARK_EVICTABLE: u32 = 0x200;
const FAN_MARK_IGNORE: u32 = 0x400;
const FAN_ALLOW: u32 = 1;
const FAN_DENY: u32 = 2;
const FAN_CLASS_CONTENT: u32 = 4;
const FAN_CLOEXEC: u32 = 1;
const FAN_UNLIMITED_QUEUE: u32 = 0x10;
const FAN_UNLIMITED_MARKS: u32 = 0x20;

/// The tag a download carries: when it arrived.
pub const TAG: &str = "user.wolos.quarantine";
/// Browsers, wget and curl leave this on what they fetch.
pub const ORIGIN: &str = "user.xdg.origin.url";
/// What the Privacy & Security page and `wol security` read.
pub use wol_core::malware::STATE_DIR;
pub const STATUS: &str = "status.json";

/// Above this an untagged file runs at once and is checked in the background.
const LARGE: u64 = 1 << 30;
/// How much of a download or asked file above [`LARGE`] the guard reads before it answers
/// (S4-5): [`LARGE`]. Unit tests read 128 MiB through the same code (a debug build matches at
/// tens of MiB/s).
const LARGE_READ: u64 = if cfg!(test) { 2 * WHOLE } else { LARGE };
/// Largest file held in memory whole and handed to the deep worker whole; above it the lean pass
/// streams in [`PIECE`]s, the worker runs the rule sets over pieces (`scan --stream`, R2C-4) and
/// the sweep does the whole-file look later (audit S8). Each of the [`THREADS`]
/// may hold one in the guard and one in its worker at once: [`MEMORY_NEED`] must fit the unit's
/// `MemoryMax` (crates/xguard/tests/units.rs).
pub const WHOLE: u64 = 64 << 20;
/// One read of the streamed lean pass, and the bytes kept across reads so a pattern that spans
/// two reads is still seen.
const PIECE: usize = 8 << 20;
const OVERLAP: usize = 1 << 20;
/// Room for the mapped signatures, the compiled YARA rules in a worker and the heap.
pub const RESERVE: u64 = 128 << 20;
/// Most memory the guard and its workers use together.
pub const MEMORY_NEED: u64 = THREADS as u64 * 2 * WHOLE + RESERVE;
/// The `why` of a scan whose rule sets did not load: the deep check did not run.
pub const RULES_DID_NOT_LOAD: &str = "the rule sets did not load";
/// The database stays mapped this long after the last scan.
const LINGER: Duration = Duration::from_secs(30);
/// Findings kept for the page.
const KEEP: usize = 50;
pub const THREADS: usize = 3;

/// What the guard needs to do on a kernel question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Allow,
    Deny,
    /// Refused now, not remembered: the check could not run (audit S4). The next open asks again.
    Unchecked,
}

/// A deep check that did not answer: refuse a download for now, let anything else run (it is
/// remembered as not deep, so the sweep looks again). A caller on the socket counts as a
/// download: it hears "unchecked", not "clean".
pub fn on_deep_failure(tagged: bool) -> Answer {
    if tagged {
        Answer::Unchecked
    } else {
        Answer::Allow
    }
}

/// The deep worker's report, or None when it did not really look (its rule sets did not load).
pub fn usable(r: crate::Report) -> Option<crate::Report> {
    let broken = r.skipped.iter().any(|s| s.why == RULES_DID_NOT_LOAD);
    (!broken).then_some(r)
}

/// Remembered verdict -> answer, or None to scan.
pub fn from_store(e: Option<Entry>) -> Option<Answer> {
    match e?.state {
        State::Clean | State::Allowed => Some(Answer::Allow),
        State::Found => Some(Answer::Deny),
    }
}

/// Does this file deserve the rule sets before it may start, not just hashes and patterns?
pub fn needs_deep(tagged: bool, removable: bool, head: &[u8], name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    tagged
        || removable
        || kind(head) == Kind::Pe
        || [".appimage", ".exe", ".msi", ".wol"]
            .iter()
            .any(|e| lower.ends_with(e))
}

/// Mount points to guard in `mountinfo`: the `roots` and removable media, never the system volume.
pub fn scope_mounts(mountinfo: &str, roots: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = mountinfo
        .lines()
        .filter_map(|l| l.split(' ').nth(4))
        .filter(|m| {
            *m != "/"
                && (roots.iter().any(|r| m == r)
                    || REMOVABLE.iter().any(|r| m.starts_with(&format!("{r}/"))))
        })
        .map(|m| m.replace("\\040", " "))
        .collect();
    out.sort();
    out.dedup();
    out
}

const REMOVABLE: [&str; 2] = ["/media", "/run/media"];

/// Fixed places that can hold something new to run.
pub const ROOTS: [&str; 4] = ["/home", "/tmp", "/var/tmp", "/mnt"];

pub fn is_removable(path: &str) -> bool {
    REMOVABLE.iter().any(|r| path.starts_with(&format!("{r}/")))
}

/// Most of `~/.config/user-dirs.dirs` root reads (audit C7-2).
pub const USER_DIRS_CAP: u64 = 64 * 1024;

/// An account's `user-dirs.dirs`, read as root without trusting it (audit C7-2): no final
/// symlink, a FIFO does not block the open, only a regular file owned by `uid`, at most
/// [`USER_DIRS_CAP`] bytes. None otherwise.
pub fn read_user_dirs(path: &str, uid: u32) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let f = shared::sys::open_named(path).ok()?;
    let md = f.metadata().ok()?;
    if !md.is_file() || md.uid() != uid {
        return None;
    }
    let mut text = String::new();
    f.take(USER_DIRS_CAP).read_to_string(&mut text).ok()?;
    Some(text)
}

/// `dir` opened as a folder strictly inside `home` that `uid` owns (audit C7-3). Each step below
/// `home` is opened from the one before with `O_NOFOLLOW | O_DIRECTORY`, so a symlink anywhere
/// on the way is refused, and the descriptor is what gets marked: swapping a folder for a symlink
/// after the check cannot move the mark to `/usr/bin` or another account's files. A directory
/// open never blocks (no FIFO can be opened with `O_DIRECTORY`). None when any of that fails.
pub fn open_owned_dir(dir: &str, home: &str, uid: u32) -> Option<OwnedFd> {
    use std::os::unix::fs::MetadataExt;
    let home = home.trim_end_matches('/');
    let rest = dir
        .strip_prefix(home)?
        .strip_prefix('/')?
        .trim_end_matches('/');
    if home.is_empty() || rest.is_empty() {
        return None;
    }
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let open_at = |at: RawFd, name: &str| -> Option<OwnedFd> {
        let c = std::ffi::CString::new(name).ok()?;
        // SAFETY: plain syscall on a NUL-terminated name; a valid fd is owned from here on.
        let fd = unsafe { libc::openat(at, c.as_ptr(), flags) };
        // SAFETY: openat returned a new descriptor nobody else owns.
        (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
    };
    let mut cur = open_at(libc::AT_FDCWD, home)?;
    for part in rest.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return None;
        }
        cur = open_at(cur.as_raw_fd(), part)?;
    }
    let f = std::fs::File::from(cur);
    let md = f.metadata().ok()?;
    (md.is_dir() && md.uid() == uid).then(|| OwnedFd::from(f))
}

/// Downloads folder of every login account: (`/etc/passwd`, user-dirs reader (path, uid), folder
/// check and open (dir, home, uid), e.g. [`open_owned_dir`]). A folder that fails the check falls
/// back to `<home>/Downloads`, and that too must pass, else the account has none.
pub fn download_dirs<T>(
    passwd: &str,
    user_dirs: &dyn Fn(&str, u32) -> Option<String>,
    open: &dyn Fn(&str, &str, u32) -> Option<T>,
) -> Vec<(String, T)> {
    passwd
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            let uid: u32 = f.get(2)?.parse().ok()?;
            let home = *f.get(5)?;
            ((1000..60000).contains(&uid) && home.starts_with("/home/")).then_some((home, uid))
        })
        .filter_map(|(home, uid)| {
            let dirs = user_dirs(&format!("{home}/.config/user-dirs.dirs"), uid);
            let d = wol_core::downloads::dir(dirs.as_deref(), home);
            if let Some(t) = open(&d, home, uid) {
                return Some((d, t));
            }
            let fallback = wol_core::downloads::dir(None, home);
            open(&fallback, home, uid).map(|t| (fallback, t))
        })
        .collect()
}

/// Newest first, at most [`KEEP`], one per path.
pub fn remember(list: &mut Vec<Blocked>, b: Blocked) {
    list.retain(|o| o.path != b.path);
    list.insert(0, b);
    list.truncate(KEEP);
}

/// Options of a guard run.
#[derive(Clone, Debug)]
pub struct Options {
    pub state: PathBuf,
    pub rules: PathBuf,
    /// Explicit mounts (tests); empty: [`ROOTS`] and removable media.
    pub scope: Vec<PathBuf>,
    /// Explicit folders treated as Downloads (tests); empty: every account's.
    pub downloads: Vec<PathBuf>,
    /// The deep worker; None: hashes and patterns only.
    pub worker: Option<PathBuf>,
    pub socket: PathBuf,
}

/// Counters for tests and benchmarks (`status.json`).
#[derive(Default)]
pub struct Counters {
    pub asked: AtomicU64,
    pub scanned: AtomicU64,
    pub denied: AtomicU64,
}

struct Job {
    file: std::fs::File,
    /// The kernel's question, or none for a caller on the socket.
    event: Option<RawFd>,
    open_mark: bool,
    reply: Option<UnixStream>,
    /// Open Anyway: remember this content as the person's choice.
    allow: bool,
    /// The file as the kernel sees it: questions about the same file share one scan (S4-3).
    id: Option<FileId>,
    /// A socket request's place in [`QUEUE_CAP`], given back when the job is dropped.
    slot: Option<shared::bounded::Slot>,
    /// Questions about the same file that arrived while this one waited: same answer.
    followers: Vec<Job>,
}

impl Job {
    fn new(file: std::fs::File, event: Option<RawFd>, open_mark: bool) -> Self {
        Self {
            id: FileId::of(&file).ok(),
            file,
            event,
            open_mark,
            reply: None,
            allow: false,
            slot: None,
            followers: Vec::new(),
        }
    }

    /// Two questions one scan can answer: the same file, the same verb, both from the kernel or
    /// both from the socket (a caller always gets the deep check).
    fn same(&self, o: &Job) -> bool {
        self.id.is_some()
            && self.id == o.id
            && self.allow == o.allow
            && self.reply.is_some() == o.reply.is_some()
    }
}

struct Shared {
    fan: RawFd,
    o: Options,
    store: Mutex<Store>,
    engine: Mutex<(Option<Arc<Engine>>, Instant)>,
    queue: Mutex<VecDeque<Job>>,
    ready: Condvar,
    counters: Counters,
    findings: Mutex<Vec<Blocked>>,
    rules_version: AtomicU64,
    stop: AtomicBool,
    /// Write end of the event loop's wake-up pair: a socket request was queued, so the loop
    /// refreshes the status and lets the database go later (it no longer polls the socket).
    waker: Option<UnixStream>,
}

fn mark(fan: RawFd, flags: u32, mask: u64, fd: RawFd, path: *const libc::c_char) -> bool {
    // SAFETY: plain syscall; path is NUL-terminated or NULL (the object is then `fd`).
    unsafe { libc::fanotify_mark(fan, flags, mask, fd, path) == 0 }
}

fn mark_path(fan: RawFd, flags: u32, mask: u64, path: &str) -> bool {
    let Ok(c) = std::ffi::CString::new(path) else {
        return false;
    };
    mark(fan, flags, mask, libc::AT_FDCWD, c.as_ptr())
}

fn answer(fan: RawFd, fd: RawFd, a: Answer) {
    let r = libc::fanotify_response {
        fd,
        response: if a == Answer::Allow {
            FAN_ALLOW
        } else {
            FAN_DENY
        },
    };
    let n = std::mem::size_of_val(&r);
    // SAFETY: writes one response struct.
    let _ = unsafe { libc::write(fan, std::ptr::from_ref(&r).cast(), n) };
}

/// Trust this file until it is written: the kernel stops asking.
fn stop_asking(fan: RawFd, fd: RawFd) {
    let flags = FAN_MARK_ADD | FAN_MARK_IGNORE | FAN_MARK_EVICTABLE;
    mark(fan, flags, FAN_OPEN_EXEC_PERM, fd, std::ptr::null());
}

fn path_of(fd: RawFd) -> String {
    std::fs::read_link(format!("/proc/self/fd/{fd}"))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn has_xattr(fd: RawFd, name: &str) -> bool {
    let Ok(c) = std::ffi::CString::new(name) else {
        return false;
    };
    // SAFETY: size 0 asks for the length only.
    unsafe { libc::fgetxattr(fd, c.as_ptr(), std::ptr::null_mut(), 0) >= 0 }
}

/// A download: by this guard's tag, or by the origin address a browser or fetcher left.
pub fn is_tagged(fd: RawFd) -> bool {
    has_xattr(fd, TAG) || has_xattr(fd, ORIGIN)
}

fn set_tag(fd: RawFd) {
    let (Ok(c), v) = (std::ffi::CString::new(TAG), now().to_string()) else {
        return;
    };
    // SAFETY: name and value outlive the call.
    unsafe { libc::fsetxattr(fd, c.as_ptr(), v.as_ptr().cast(), v.len(), 0) };
}

/// First 16 bytes of a digest: the cache key.
fn key16(sha: &[u8; 32]) -> [u8; 16] {
    let mut k = [0; 16];
    k.copy_from_slice(&sha[..16]);
    k
}

/// What a `check`/`allow` caller may hand over: a regular file. A FIFO, socket or device
/// (`/dev/zero`) would block or never end, so it is refused before any read. A file above
/// [`LARGE`] is let in: the guard reads at most [`LARGE`] of it and answers `unchecked` unless a
/// pattern is found there (S4-5).
pub fn checkable(file: &std::fs::File) -> Result<u64, &'static str> {
    use std::os::unix::fs::FileTypeExt;
    let m = file.metadata().map_err(|_| "cannot stat")?;
    let t = m.file_type();
    if !t.is_file() || t.is_fifo() || t.is_char_device() || t.is_block_device() || t.is_socket() {
        return Err("not a regular file");
    }
    Ok(m.len())
}

/// Most socket requests (`check`/`allow`) being read, queued or scanned at once, over all callers
/// (S4-3). Each holds two descriptors (the connection and the file).
pub const QUEUE_CAP: usize = 64;
/// Most socket requests of one account at once: one account cannot fill [`QUEUE_CAP`].
pub const PER_UID: usize = 8;
/// Longest a caller on the socket may take to send its verb and file.
const CLIENT_READ: Duration = Duration::from_secs(2);
/// Descriptors the guard holds besides the queue: fanotify, mountinfo, the listener, stdio, the
/// mapped database and, per worker thread, the file, its clone for the deep worker and the
/// worker's pipes.
pub const FD_RESERVE: usize = 32 + THREADS * 4;
/// The descriptors the socket queue and the workers may need at most; the unit's `LimitNOFILE`
/// must leave room above it for the kernel's questions, which hold one each while queued
/// (crates/xguard/tests/units.rs).
pub const FD_NEED: usize = QUEUE_CAP * 2 + FD_RESERVE;

/// A place for one socket request of the caller on `c`, or None after telling it `unchecked`:
/// over [`PER_UID`] or [`QUEUE_CAP`], or the kernel did not say who it is.
fn admit(quota: &shared::bounded::Quota, c: &UnixStream) -> Option<shared::bounded::Slot> {
    let slot = ctl::peer_uid(c).and_then(|u| quota.admit(u));
    if slot.is_none() {
        let _ = c.set_write_timeout(Some(CLIENT_READ));
        ctl::write_reply(c, &unchecked());
    }
    slot
}

fn unchecked() -> Reply {
    Reply {
        state: "unchecked".into(),
        name: String::new(),
    }
}

/// Longest one deep scan may take (R2C-5). The kernel holds the opener until the guard answers,
/// and there are only [`THREADS`] workers: past this the worker is killed and the file is
/// treated like a deep check that did not run ([`on_deep_failure`]).
pub const DEEP_DEADLINE: Duration = Duration::from_secs(60);
/// Most a worker may write on its standard output (one JSON report line).
const DEEP_OUT: u64 = 1 << 20;

/// How one deep scan ended.
#[derive(Debug)]
pub enum DeepOutcome {
    Report(crate::Report),
    /// The worker did not start, died, wrote no usable report, or its rules did not load.
    Failed,
    /// The worker ran past the deadline and was killed.
    TimedOut,
}

/// The worker's command: a fresh process with no privileges. `stream`: the file is above
/// [`WHOLE`], the worker reads it in pieces (R2C-4).
fn deep_command(worker: &Path, rules: &Path, stream: bool) -> std::process::Command {
    deep_command_as(worker, rules, stream, true)
}

/// [`deep_command`]; `nobody`: run it as uid/gid 65534 (only root may switch, so a library
/// caller that is not root keeps its own account, still without new privileges; S4-16).
pub(crate) fn deep_command_as(
    worker: &Path,
    rules: &Path,
    stream: bool,
    nobody: bool,
) -> std::process::Command {
    use std::os::unix::process::CommandExt;
    let mut c = std::process::Command::new(worker);
    c.args(["scan", "--json"]);
    if stream {
        c.arg("--stream");
    }
    c.arg("--rules").arg(rules).arg("-");
    // SAFETY: prctl and setresuid are async-signal-safe.
    unsafe {
        c.pre_exec(|| {
            libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
            Ok(())
        });
    }
    if nobody {
        c.uid(65534).gid(65534);
    }
    c
}

/// Run a deep-scan command with the file on its standard input and wait at most `deadline`
/// (R2C-5). The worker runs in a process group of its own: at the end the whole group is killed
/// (anything it started that still holds its output dies with it), while the worker is not yet
/// reaped, so the group's number is still its own and never an unrelated pid. After a timeout
/// the report reader is never waited for.
pub fn run_deep(
    mut c: std::process::Command,
    file: &std::fs::File,
    deadline: Duration,
) -> DeepOutcome {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    let Ok(clone) = file.try_clone() else {
        return DeepOutcome::Failed;
    };
    c.stdin(clone).stdout(Stdio::piped()).stderr(Stdio::null());
    c.process_group(0);
    let Ok(mut child) = c.spawn() else {
        return DeepOutcome::Failed;
    };
    let Ok(pid) = libc::pid_t::try_from(child.id()) else {
        let _ = child.kill();
        let _ = child.wait();
        return DeepOutcome::Failed;
    };
    // The report is read on its own thread so a full pipe never stalls the wait below.
    let (tx, rx) = std::sync::mpsc::channel();
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut v = Vec::new();
            let _ = out.take(DEEP_OUT).read_to_end(&mut v);
            let _ = tx.send(v);
        });
    } else {
        let _ = tx.send(Vec::new());
    }
    let start = Instant::now();
    let (mut ended, mut out) = (false, None);
    let timed_out = loop {
        if out.is_none() {
            match rx.try_recv() {
                Ok(v) => out = Some(v),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => out = Some(Vec::new()),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if !ended && exited_unreaped(pid) {
            ended = true;
            // What the worker left behind may hold its output open: stop it so the reader ends.
            kill_group(pid);
        }
        if ended && out.is_some() {
            break false;
        }
        if start.elapsed() >= deadline {
            break true;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    kill_group(pid);
    let _ = child.wait();
    if timed_out {
        return DeepOutcome::TimedOut;
    }
    let out = out.unwrap_or_default();
    out.split(|b| *b == b'\n')
        .next()
        .and_then(|l| serde_json::from_slice(l).ok())
        .and_then(usable)
        .map_or(DeepOutcome::Failed, DeepOutcome::Report)
}

/// The worker `pid` (a child of this process) has ended, left as a zombie (`WNOWAIT`): its pid,
/// and so its process group's number, stays taken until [`std::process::Child::wait`]. An error
/// (no such child) counts as ended.
fn exited_unreaped(pid: libc::pid_t) -> bool {
    // SAFETY: siginfo_t is plain data; waitid only writes into it.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let flags = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
    // SAFETY: valid pointer to a local; `pid` is our own unreaped child.
    let r = unsafe { libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, flags) };
    // SAFETY: waitid filled `info` (si_pid 0: the child is still running).
    r != 0 || unsafe { info.si_pid() } != 0
}

/// SIGKILL to the worker's process group (its pgid is its own pid, `process_group(0)`). Only
/// called while the worker is unreaped, so the number cannot belong to anyone else.
fn kill_group(pid: libc::pid_t) {
    // SAFETY: plain integer arguments.
    unsafe { libc::killpg(pid, libc::SIGKILL) };
}

/// What a deep check means for the answer: `Ok((finding, ran_deep))` to go on, or `Err(answer)`
/// to answer now without remembering. `streamed`: the file was read in pieces, so even a clean
/// result is remembered as not deep and the sweep looks again with the whole file.
pub fn after_deep(
    o: DeepOutcome,
    download: bool,
    streamed: bool,
) -> Result<(Option<String>, bool), Answer> {
    match o {
        DeepOutcome::Report(r) => {
            let finished = r.verdict != crate::Verdict::Incomplete && !streamed;
            Ok((r.findings.first().map(|f| f.name.clone()), finished))
        }
        DeepOutcome::Failed | DeepOutcome::TimedOut => match on_deep_failure(download) {
            Answer::Allow => Ok((None, false)),
            a => Err(a),
        },
    }
}

/// Patterns piece by piece with an overlap, and the whole-file hashes from streamed digests:
/// memory stays at one [`PIECE`] whatever the size (audit S8). Reads at most `limit` bytes.
fn stream(
    e: &Engine,
    f: &mut std::fs::File,
    path: &str,
    limit: u64,
) -> Option<(Vec<String>, [u8; 32])> {
    let mut r = f.take(limit);
    let (mut buf, mut keep) = (vec![0u8; PIECE + OVERLAP], 0usize);
    let (mut md5, mut sha, mut size) = (Md5::new(), Sha256::new(), 0u64);
    let mut names = Vec::new();
    loop {
        let mut got = 0;
        while keep + got < buf.len() {
            match r.read(&mut buf[keep + got..]) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return None,
            }
        }
        if got == 0 {
            break;
        }
        let end = keep + got;
        md5.update(&buf[keep..end]);
        sha.update(&buf[keep..end]);
        size += got as u64;
        names.extend(
            e.findings(&buf[..end], None, path)
                .into_iter()
                .map(|f| f.name),
        );
        keep = OVERLAP.min(end);
        buf.copy_within(end - keep..end, 0);
    }
    let sha = <[u8; 32]>::from(sha.finalize());
    let md5 = <[u8; 16]>::from(md5.finalize());
    match e.hash_findings(size, &md5, &sha, path) {
        None => return Some((Vec::new(), sha)),
        Some(h) => names.extend(h.into_iter().map(|f| f.name)),
    }
    names.sort();
    names.dedup();
    Some((names, sha))
}

impl Shared {
    fn engine(&self) -> Option<Arc<Engine>> {
        let mut g = self.engine.lock().unwrap_or_else(|p| p.into_inner());
        g.1 = Instant::now();
        if g.0.is_none() {
            g.0 = Engine::open(&self.o.rules).ok().map(Arc::new);
            if let Some(e) = &g.0 {
                let v = e.versions().iter().copied().max().unwrap_or(0);
                self.rules_version.store(v.into(), Relaxed);
            }
        }
        g.0.clone()
    }

    /// Hash and match a file (first use only): whole in memory up to [`WHOLE`], streamed above.
    /// The flag says it was read whole (only then may the deep worker look). None: unreadable or
    /// no signatures.
    fn lean(&self, f: &mut std::fs::File, path: &str) -> Option<(Vec<String>, [u8; 32], bool)> {
        use std::io::{Seek, SeekFrom};
        let e = self.engine()?;
        self.counters.scanned.fetch_add(1, Relaxed);
        let mut d = Vec::new();
        // Never more than the cap, whatever the file claims: it may grow while being read.
        (&mut *f).take(WHOLE + 1).read_to_end(&mut d).ok()?;
        if d.len() as u64 > WHOLE {
            drop(d);
            f.seek(SeekFrom::Start(0)).ok()?;
            let (names, sha) = stream(&e, f, path, LARGE)?;
            return Some((names, sha, false));
        }
        let sha = <[u8; 32]>::from(Sha256::digest(&d));
        let names = e.findings(&d, Some(&sha), path);
        Some((names.into_iter().map(|f| f.name).collect(), sha, true))
    }

    fn block(&self, path: &str, name: &str, sha: &[u8], uid: u32) {
        self.counters.denied.fetch_add(1, Relaxed);
        let mut l = self.findings.lock().unwrap_or_else(|p| p.into_inner());
        remember(
            &mut l,
            Blocked {
                path: path.into(),
                name: name.into(),
                sha256: hexs(sha),
                when: now(),
                uid,
            },
        );
        save(&self.o.state, &l, uid);
    }

    /// Open Anyway: this content runs from now on, on this machine.
    fn allow(&self, j: &mut Job) -> (Answer, String) {
        let path = path_of(j.file.as_raw_fd());
        let (Ok(id), Some((_, sha, _))) = (FileId::of(&j.file), self.lean(&mut j.file, &path))
        else {
            return (Answer::Allow, String::new());
        };
        let key = key16(&sha);
        self.put(&id, State::Allowed, 0, true, key);
        let mut l = self.findings.lock().unwrap_or_else(|p| p.into_inner());
        let uid = l.iter().find(|b| b.path == path).map(|b| b.uid);
        l.retain(|b| b.path != path);
        if let Some(uid) = uid {
            save(&self.o.state, &l, uid);
        }
        (Answer::Allow, String::new())
    }

    /// A question that needs a scan: the answer, and what was found.
    fn check(&self, j: &mut Job) -> (Answer, String) {
        use std::io::{Seek, SeekFrom};
        let fd = j.file.as_raw_fd();
        let path = path_of(fd);
        let Ok(id) = FileId::of(&j.file) else {
            return (Answer::Allow, String::new());
        };
        let mut head = [0u8; 16];
        let _ = std::os::unix::fs::FileExt::read_at(&j.file, &mut head, 0);
        let tagged = is_tagged(fd);
        let asked = j.reply.is_some();
        let deep = asked || needs_deep(tagged, is_removable(&path), &head, &path);
        // S4-5: above the cap an ordinary file runs at once and the sweep looks later. A download
        // or a file somebody asked about gets the streamed lean pass over its first LARGE bytes
        // (hashes and patterns), never the deep check here: a pattern there refuses it, else it
        // is unchecked for now, never allowed and never given the ignore mark.
        if id.size > LARGE {
            if !(tagged || asked) {
                return (Answer::Allow, String::new());
            }
            return self.check_large(j, &path);
        }
        let Some((names, sha, whole)) = self.lean(&mut j.file, &path) else {
            return (Answer::Allow, String::new());
        };
        let rules = self.rules_version.load(Relaxed) as u32;
        let key = key16(&sha);
        let known = self
            .store
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .by_sha(&sha);
        if known.is_some_and(|k| k.state == State::Allowed) {
            self.put(&id, State::Allowed, rules, true, key);
            return (Answer::Allow, String::new());
        }
        let mut found = names.first().cloned();
        let mut ran_deep = false;
        if found.is_none()
            && deep
            && let Some(w) = &self.o.worker
        {
            let _ = j.file.seek(SeekFrom::Start(0));
            // Above WHOLE the worker reads the file in pieces (R2C-4): a download is never
            // allowed and remembered without the rule sets having looked at it.
            let streamed = !whole;
            let o = run_deep(
                deep_command(w, &self.o.rules, streamed),
                &j.file,
                DEEP_DEADLINE,
            );
            match &o {
                DeepOutcome::Report(r) if r.verdict == crate::Verdict::Incomplete => {
                    eprintln!("wol-xguard: {path}: deep check incomplete, the sweep looks again");
                }
                DeepOutcome::Report(_) => {}
                DeepOutcome::Failed => eprintln!(
                    "wol-xguard: {path}: the deep check did not run (worker ended or its rule sets do not load)"
                ),
                DeepOutcome::TimedOut => eprintln!(
                    "wol-xguard: {path}: the deep check took longer than {} s and was stopped",
                    DEEP_DEADLINE.as_secs()
                ),
            }
            match after_deep(o, tagged || asked, streamed) {
                Ok((f, d)) => {
                    found = f;
                    ran_deep = d;
                }
                Err(a) => {
                    eprintln!("wol-xguard: {path}: download refused until it can be checked");
                    return (a, String::new());
                }
            }
        }
        let state = if found.is_some() {
            State::Found
        } else {
            State::Clean
        };
        self.put(&id, state, rules, ran_deep, key);
        if let Some(v) = verity_key(fd) {
            let e = Entry {
                state,
                rules,
                deep: ran_deep,
                sha: key,
            };
            self.store
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .alias(&v, e);
        }
        match found {
            Some(n) => {
                use std::os::unix::fs::MetadataExt;
                let uid = j.file.metadata().map_or(0, |m| m.uid());
                self.block(&path, &n, &sha, uid);
                (Answer::Deny, n)
            }
            None => (Answer::Allow, String::new()),
        }
    }

    /// A download or asked file above [`LARGE`]: patterns over what [`stream`] reads. Found:
    /// refused and remembered like any finding. Otherwise [`Answer::Unchecked`] (the deep check
    /// did not run: `on_deep_failure(true)`), nothing remembered, so the sweep's verdict or Open
    /// Anyway decides.
    fn check_large(&self, j: &mut Job, path: &str) -> (Answer, String) {
        let found = self.engine().and_then(|e| {
            self.counters.scanned.fetch_add(1, Relaxed);
            let (names, sha) = stream(&e, &mut j.file, path, LARGE_READ)?;
            Some((names.first()?.clone(), sha))
        });
        match found {
            Some((n, sha)) => {
                use std::os::unix::fs::MetadataExt;
                if let Ok(id) = FileId::of(&j.file) {
                    let rules = u32::try_from(self.rules_version.load(Relaxed)).unwrap_or(u32::MAX);
                    self.put(&id, State::Found, rules, false, key16(&sha));
                }
                let uid = j.file.metadata().map_or(0, |m| m.uid());
                self.block(path, &n, &sha, uid);
                (Answer::Deny, n)
            }
            // Larger than LARGE: refused until the background check or Open Anyway.
            None => (on_deep_failure(true), String::new()),
        }
    }

    fn put(&self, id: &FileId, state: State, rules: u32, deep: bool, sha: [u8; 16]) {
        self.store.lock().unwrap_or_else(|p| p.into_inner()).put(
            id,
            Entry {
                state,
                rules,
                deep,
                sha,
            },
        );
    }

    fn finish(&self, j: &Job, a: Answer, name: &str) {
        let fd = j.file.as_raw_fd();
        if a == Answer::Allow {
            if j.open_mark {
                mark(
                    self.fan,
                    FAN_MARK_REMOVE,
                    FAN_OPEN_PERM,
                    fd,
                    std::ptr::null(),
                );
            }
            stop_asking(self.fan, fd);
        }
        if let Some(e) = j.event {
            answer(self.fan, e, a);
        }
        if let Some(r) = &j.reply {
            let state = match (a, j.allow) {
                (Answer::Deny, _) => "found",
                (Answer::Unchecked, _) => "unchecked",
                (Answer::Allow, true) => "allowed",
                (Answer::Allow, false) => "clean",
            };
            ctl::write_reply(
                r,
                &Reply {
                    state: state.into(),
                    name: name.into(),
                },
            );
        }
        for f in &j.followers {
            self.finish(f, a, name);
        }
    }

    fn worker(&self) {
        loop {
            let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
            while q.is_empty() {
                if self.stop.load(Relaxed) {
                    return;
                }
                q = self.ready.wait(q).unwrap_or_else(|p| p.into_inner());
            }
            let Some(mut j) = q.pop_front() else { continue };
            drop(q);
            // A panic in a scan keeps this worker alive (without an answer the kernel would wait
            // on the event forever). It answers like a deep check that did not run: a download
            // is refused for now and asked about again, anything else is allowed (audit S4).
            let (a, name) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if j.allow {
                    self.allow(&mut j)
                } else {
                    self.check(&mut j)
                }
            }))
            .unwrap_or_else(|_| {
                let fd = j.file.as_raw_fd();
                eprintln!("wol-xguard: {}: the check stopped on a bug", path_of(fd));
                let asked = j.reply.is_some();
                (
                    on_deep_failure(!j.allow && (asked || is_tagged(fd))),
                    String::new(),
                )
            });
            self.finish(&j, a, &name);
        }
    }

    fn status(&self) {
        let v = serde_json::json!({
            "running": true,
            "pid": std::process::id(),
            "asked": self.counters.asked.load(Relaxed),
            "scanned": self.counters.scanned.load(Relaxed),
            "denied": self.counters.denied.load(Relaxed),
        });
        write_json(&self.o.state.join(STATUS), 0o644, None, &v);
    }

    /// One kernel event. Remembered files are answered right here, on this thread.
    fn event(&self, fd: RawFd, mask: u64) {
        // SAFETY: the kernel gave this descriptor to us alone.
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        let file = std::fs::File::from(owned);
        if mask & FAN_CLOSE_WRITE != 0 {
            set_tag(fd);
            mark(self.fan, FAN_MARK_ADD, FAN_OPEN_PERM, fd, std::ptr::null());
            return;
        }
        self.counters.asked.fetch_add(1, Relaxed);
        let open_mark = mask & FAN_OPEN_PERM != 0;
        let remembered = self.remembered(&file);
        let job = Job::new(file, Some(fd), open_mark);
        match remembered {
            Some(a) => self.finish(&job, a, ""),
            None => self.enqueue(job),
        }
    }

    /// By identity, else by the digest of a file that has fs-verity: both without reading it.
    fn remembered(&self, file: &std::fs::File) -> Option<Answer> {
        let id = FileId::of(file).ok()?;
        let mut s = self.store.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(e) = s.by_id(&id) {
            return from_store(Some(e));
        }
        let e = verity_key(file.as_raw_fd()).and_then(|k| s.by_key(&k))?;
        s.put(&id, e);
        from_store(Some(e))
    }

    /// Queue a question; one about a file already waiting rides along with it (S4-3), so a
    /// burst of execs of one new file is one scan.
    fn enqueue(&self, job: Job) {
        let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(lead) = q.iter_mut().find(|l| l.same(&job)) {
            lead.followers.push(job);
            return;
        }
        q.push_back(job);
        self.ready.notify_one();
    }

    /// The socket's own thread (S4-3): the event loop never waits on a caller. Each connection is
    /// read on a thread of its own, at most [`QUEUE_CAP`] at once ([`PER_UID`] per account); a
    /// caller over its place hears `unchecked` at once.
    fn accept_loop(self: &Arc<Self>, listener: &UnixListener) {
        let quota = shared::bounded::Quota::new(PER_UID, QUEUE_CAP);
        loop {
            let c = match listener.accept() {
                Ok((c, _)) => c,
                // EMFILE and friends: wait a little instead of spinning.
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
            };
            let Some(slot) = admit(&quota, &c) else {
                continue;
            };
            let s = self.clone();
            // No thread: the connection and its slot are dropped, the caller sees it close.
            let _ = std::thread::Builder::new()
                .name("xguard-client".into())
                .spawn(move || s.client(c, slot));
        }
    }

    /// One caller on the socket, on its own thread. Its slot travels with the job.
    fn client(&self, s: UnixStream, slot: shared::bounded::Slot) {
        let _ = s.set_nonblocking(false);
        let _ = s.set_read_timeout(Some(CLIENT_READ));
        let _ = s.set_write_timeout(Some(CLIENT_READ));
        let Ok((verb, Some(fd))) = ctl::recv_fd(&s) else {
            return;
        };
        let file = std::fs::File::from(fd);
        if checkable(&file).is_err() {
            return;
        }
        let allow = match verb.as_str() {
            "check" => false,
            // Only the file's owner (or an administrator acting as root) may wave it through.
            "allow" => {
                use std::os::unix::fs::MetadataExt;
                let owner = file.metadata().map(|m| m.uid()).ok();
                let peer = ctl::peer_uid(&s);
                if peer.is_none() || (peer != Some(0) && peer != owner) {
                    return;
                }
                true
            }
            _ => return,
        };
        let mut job = Job::new(file, None, false);
        job.reply = Some(s);
        job.allow = allow;
        job.slot = Some(slot);
        self.enqueue(job);
        if let Some(w) = &self.waker {
            // Nonblocking: a full pair already means the loop will wake.
            let _ = std::io::Write::write(&mut &*w, &[1]);
        }
    }

    /// Lets the database go once nothing has been scanned for a while. True: come back later.
    fn tidy(&self) -> bool {
        let mut g = self.engine.lock().unwrap_or_else(|p| p.into_inner());
        if g.0.is_none() {
            return false;
        }
        if g.1.elapsed() < LINGER {
            return true;
        }
        g.0 = None;
        drop(g);
        // SAFETY: glibc call with no arguments that matter.
        unsafe { libc::malloc_trim(0) };
        false
    }
}

/// Cache key of a file that has fs-verity: its digest, obtained from the kernel without reading.
fn verity_key(fd: RawFd) -> Option<[u8; 16]> {
    #[repr(C)]
    struct Measure {
        algorithm: u16,
        size: u16,
        digest: [u8; 64],
    }
    let mut m = Measure {
        algorithm: 0,
        size: 64,
        digest: [0; 64],
    };
    // FS_IOC_MEASURE_VERITY. Fails with ENODATA on a file without fs-verity.
    // SAFETY: the kernel writes at most `size` digest bytes into the struct.
    let r = unsafe { libc::ioctl(fd, 0xc004_6686u64 as _, &mut m) };
    (r == 0).then(|| {
        let mut h = Sha256::new();
        h.update(b"verity");
        h.update(m.algorithm.to_le_bytes());
        h.update(&m.digest[..usize::from(m.size).min(64)]);
        let mut k = [0; 16];
        k.copy_from_slice(&h.finalize()[..16]);
        k
    })
}

/// `v` as JSON at `path`, atomically (S4-10: a unique temporary, never a shared `.tmp`), with
/// `mode` and, when given, owner `uid` set before the file appears.
/// Best effort, like every state write of the guard: the page shows the last good file.
fn write_json(path: &Path, mode: u32, uid: Option<u32>, v: &impl serde::Serialize) {
    if let Ok(t) = serde_json::to_vec(v) {
        let _ = shared::atomic::write_as(path, mode, uid, &t);
    }
}

/// One account's list: only that account can read it, since the paths are its own. Owner and
/// mode 0600 are set on the temporary, so the list is never readable by anyone else (S4-10).
fn save(state: &Path, all: &[Blocked], uid: u32) {
    let path = state.join(findings_file(uid));
    let mine: Vec<&Blocked> = all.iter().filter(|b| b.uid == uid).collect();
    write_json(&path, 0o600, Some(uid), &mine);
}

/// The guard's folder is its own (audit F10): created 0755 when missing, refused when it is a
/// link, someone else's, or writable by others, since what is in it decides what runs. A cache
/// there that is not the guard's own (left before the folder was locked) is dropped, not read.
pub fn own_state(state: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let err = |e: std::io::Error| format!("{}: {e}", state.display());
    if std::fs::symlink_metadata(state).is_err() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(state)
            .map_err(err)?;
    }
    let m = std::fs::symlink_metadata(state).map_err(err)?;
    if !m.is_dir() || m.uid() != shared::sys::euid() || m.mode() & 0o022 != 0 {
        return Err(format!(
            "{}: not a folder this account alone may write",
            state.display()
        ));
    }
    let v = state.join("verdicts");
    if let Ok(vm) = std::fs::symlink_metadata(&v)
        && (!vm.is_file() || vm.uid() != shared::sys::euid())
    {
        std::fs::remove_file(&v).map_err(err)?;
    }
    Ok(())
}

/// Every account's list from a former run: only `findings-<uid>.json` owned by that account (or
/// by the guard), as [`save`] leaves them.
fn load_findings(state: &Path) -> Vec<Blocked> {
    use std::os::unix::fs::MetadataExt;
    let Ok(rd) = std::fs::read_dir(state) else {
        return Vec::new();
    };
    let texts: Vec<String> = rd
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let uid = n
                .strip_prefix("findings-")
                .and_then(|r| r.strip_suffix(".json"))
                .and_then(|u| u.parse::<u32>().ok());
            uid.is_some_and(|u| {
                e.metadata()
                    .is_ok_and(|m| m.is_file() && (m.uid() == u || m.uid() == shared::sys::euid()))
            })
        })
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    wol_core::malware::merge(&refs)
}

fn mark_scope(fan: RawFd, o: &Options) {
    let mounts: Vec<String> = if o.scope.is_empty() {
        let info = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        scope_mounts(&info, &ROOTS)
    } else {
        o.scope.iter().map(|p| p.display().to_string()).collect()
    };
    for m in mounts {
        if !mark_path(fan, FAN_MARK_ADD | FAN_MARK_MOUNT, FAN_OPEN_EXEC_PERM, &m) {
            eprintln!("wol-xguard: cannot guard {m}");
        }
    }
    // Audit C7-2: files the accounts own are read off this thread (it answers permission events),
    // one pass at a time; a pass already running covers this change too.
    static MARKING: AtomicBool = AtomicBool::new(false);
    if MARKING.swap(true, Relaxed) {
        return;
    }
    let explicit: Vec<String> = o
        .downloads
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    let spawned = std::thread::Builder::new()
        .name("xguard-downloads".into())
        .spawn(move || {
            let mask = FAN_CLOSE_WRITE | FAN_EVENT_ON_CHILD;
            if explicit.is_empty() {
                let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
                // C7-3: the folder that was checked, by descriptor, never its name again.
                for (_, fd) in download_dirs(&passwd, &read_user_dirs, &open_owned_dir) {
                    mark(fan, FAN_MARK_ADD, mask, fd.as_raw_fd(), std::ptr::null());
                }
            } else {
                for d in explicit {
                    mark_path(fan, FAN_MARK_ADD, mask, &d);
                }
            }
            MARKING.store(false, Relaxed);
        });
    if spawned.is_err() {
        MARKING.store(false, Relaxed);
        eprintln!("wol-xguard: cannot watch download folders");
    }
}

/// Runs until the process ends.
pub fn run(o: Options) -> Result<(), String> {
    own_state(&o.state)?;
    let store = Store::open(&o.state.join("verdicts")).map_err(|e| e.to_string())?;
    let flags = FAN_CLASS_CONTENT | FAN_CLOEXEC | FAN_UNLIMITED_QUEUE | FAN_UNLIMITED_MARKS;
    let event_flags = (libc::O_RDONLY | libc::O_LARGEFILE | libc::O_CLOEXEC) as u32;
    // SAFETY: plain syscall.
    let fan = unsafe { libc::fanotify_init(flags, event_flags) };
    if fan < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    mark_scope(fan, &o);
    let findings = load_findings(&o.state);
    let (wake_tx, mut wake_rx) = UnixStream::pair().map_err(|e| e.to_string())?;
    wake_tx.set_nonblocking(true).map_err(|e| e.to_string())?;
    wake_rx.set_nonblocking(true).map_err(|e| e.to_string())?;
    let s = Arc::new(Shared {
        fan,
        o,
        store: Mutex::new(store),
        engine: Mutex::new((None, Instant::now())),
        queue: Mutex::new(VecDeque::new()),
        ready: Condvar::new(),
        counters: Counters::default(),
        findings: Mutex::new(findings),
        rules_version: AtomicU64::new(0),
        stop: AtomicBool::new(false),
        waker: Some(wake_tx),
    });
    s.status();
    if let Some(d) = s.o.socket.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::remove_file(&s.o.socket);
    let listener = UnixListener::bind(&s.o.socket).map_err(|e| e.to_string())?;
    let _ = std::fs::set_permissions(
        &s.o.socket,
        std::os::unix::fs::PermissionsExt::from_mode(0o666),
    );
    for _ in 0..THREADS {
        let s = s.clone();
        std::thread::spawn(move || s.worker());
    }
    {
        let s = s.clone();
        std::thread::Builder::new()
            .name("xguard-socket".into())
            .spawn(move || s.accept_loop(&listener))
            .map_err(|e| e.to_string())?;
    }
    let mounts = std::fs::File::open("/proc/self/mountinfo").map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 16 << 10];
    let mut pfd = [
        libc::pollfd {
            fd: fan,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: mounts.as_raw_fd(),
            events: libc::POLLPRI,
            revents: 0,
        },
        libc::pollfd {
            fd: wake_rx.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let (mut last, mut dirty) = (Instant::now(), false);
    loop {
        // No timer at rest: with nothing to tidy, this process sleeps until the kernel asks.
        let wake = if dirty { 5_000 } else { -1 };
        // S4-3: never a caller's socket here (they have their own thread), only the kernel's
        // descriptors and the wake-up pair, whose reads never block.
        // SAFETY: three valid pollfds.
        let n = unsafe { libc::poll(pfd.as_mut_ptr(), 3, wake) };
        if n > 0 && pfd[2].revents != 0 {
            let mut drain = [0u8; 64];
            while matches!(wake_rx.read(&mut drain), Ok(n) if n > 0) {}
            dirty = true;
        }
        if n > 0 && pfd[1].revents != 0 {
            mark_scope(fan, &s.o);
        }
        if n > 0 && pfd[0].revents != 0 {
            // SAFETY: reads into the buffer; the kernel writes whole events.
            let got = unsafe { libc::read(fan, buf.as_mut_ptr().cast(), buf.len()) };
            let mut at = 0;
            let size = std::mem::size_of::<libc::fanotify_event_metadata>();
            dirty = true;
            while got > 0 && at + size <= got as usize {
                // SAFETY: at + size is inside what the kernel wrote; read_unaligned copies.
                let m: libc::fanotify_event_metadata =
                    unsafe { std::ptr::read_unaligned(buf.as_ptr().add(at).cast()) };
                if m.fd >= 0 {
                    s.event(m.fd, m.mask);
                }
                // A length shorter than the header would loop forever; past the end, stop.
                let Ok(len) = usize::try_from(m.event_len) else {
                    break;
                };
                if len < size {
                    break;
                }
                at = match at.checked_add(len) {
                    Some(n) => n,
                    None => break,
                };
            }
        }
        if dirty && last.elapsed() >= Duration::from_secs(5) {
            last = Instant::now();
            s.status();
            dirty = s.tidy();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Audit F10: the guard's folder must be its own and closed to others; a link or a cache it did
    /// not make is not used.
    #[test]
    fn the_guard_keeps_its_state_in_a_folder_only_it_may_write() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("xguard-own-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let s = d.join("guard");
        own_state(&s).unwrap();
        let m = std::fs::metadata(&s).unwrap();
        assert!(m.is_dir() && m.permissions().mode() & 0o777 == 0o755);
        // A cache that is a link (say, to a file another account fills) is dropped, not followed.
        let theirs = d.join("theirs");
        std::fs::write(&theirs, b"x").unwrap();
        std::os::unix::fs::symlink(&theirs, s.join("verdicts")).unwrap();
        own_state(&s).unwrap();
        assert!(std::fs::symlink_metadata(s.join("verdicts")).is_err());
        assert_eq!(std::fs::read(&theirs).unwrap(), b"x");
        // A list that is a link is not read.
        std::os::unix::fs::symlink(&theirs, s.join("findings-0.json")).unwrap();
        assert!(load_findings(&s).is_empty());
        // Writable by others, or a link in place of the folder: refused.
        std::fs::set_permissions(&s, std::fs::Permissions::from_mode(0o775)).unwrap();
        assert!(own_state(&s).is_err());
        let l = d.join("linked");
        std::os::unix::fs::symlink(&d, &l).unwrap();
        assert!(own_state(&l).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn remembered_verdicts_answer_without_a_scan() {
        let e = |state| {
            Some(Entry {
                state,
                rules: 1,
                deep: false,
                sha: [0; 16],
            })
        };
        assert_eq!(from_store(e(State::Clean)), Some(Answer::Allow));
        assert_eq!(from_store(e(State::Allowed)), Some(Answer::Allow));
        assert_eq!(from_store(e(State::Found)), Some(Answer::Deny));
        assert_eq!(from_store(None), None);
    }

    /// Audit S4: a deep check that could not run refuses a download for now and is not
    /// remembered; other files run.
    #[test]
    fn a_download_that_could_not_be_checked_is_refused_for_now() {
        assert_eq!(on_deep_failure(true), Answer::Unchecked);
        assert_eq!(on_deep_failure(false), Answer::Allow);
        let report = |why: &str| crate::Report {
            verdict: crate::Verdict::Incomplete,
            findings: Vec::new(),
            skipped: vec![crate::Skipped {
                path: "-".into(),
                why: why.into(),
            }],
            files: 1,
            bytes: 0,
            sha256: String::new(),
            signatures_built: 0,
            signatures_age_days: None,
            signatures_versions: Vec::new(),
        };
        assert!(usable(report(RULES_DID_NOT_LOAD)).is_none());
        assert!(usable(report("too large to unpack")).is_some());
    }

    /// Audit S8: what one check holds in memory stays far below the cap the unit sets.
    #[test]
    fn a_large_file_is_streamed_not_held() {
        const { assert!(WHOLE < LARGE) };
        const { assert!((PIECE + OVERLAP) as u64 <= WHOLE) };
        const { assert!(MEMORY_NEED < 1 << 30) };
    }

    /// R2C-5: a deep scan that hangs is stopped at the deadline; a download is then refused for
    /// now, anything else runs and is not remembered as deep.
    #[test]
    fn a_slow_deep_scan_is_stopped_at_the_deadline() {
        let f = std::fs::File::open("/etc/passwd").unwrap();
        let mut c = std::process::Command::new("sh");
        c.args(["-c", "sleep 30"]);
        let start = Instant::now();
        let o = run_deep(c, &f, Duration::from_millis(500));
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
        assert!(matches!(o, DeepOutcome::TimedOut), "{o:?}");
        assert_eq!(after_deep(o, true, false), Err(Answer::Unchecked));
        assert_eq!(
            after_deep(DeepOutcome::TimedOut, false, false),
            Ok((None, false))
        );
        assert_eq!(
            after_deep(DeepOutcome::Failed, true, true),
            Err(Answer::Unchecked)
        );
    }

    /// Audit 8 #3: what a worker leaves behind holding its output never holds the guard: on
    /// the deadline the whole process group is killed, and a worker that ended takes its
    /// leftovers with it at once.
    #[test]
    fn a_deep_scan_worker_leaves_nothing_behind() {
        let f = std::fs::File::open("/etc/passwd").unwrap();
        let mut c = std::process::Command::new("sh");
        c.args(["-c", "sleep 30 & sleep 30; :"]);
        let start = Instant::now();
        let o = run_deep(c, &f, Duration::from_millis(500));
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
        assert!(matches!(o, DeepOutcome::TimedOut), "{o:?}");
        let mut c = std::process::Command::new("sh");
        c.args(["-c", "sleep 30 & exit 0"]);
        let start = Instant::now();
        let o = run_deep(c, &f, Duration::from_secs(20));
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
        assert!(matches!(o, DeepOutcome::Failed), "no report: {o:?}");
    }

    /// R2C-4: a download above WHOLE goes to the worker in stream mode; what its rules find is
    /// refused, and a clean streamed look is not remembered as deep.
    #[test]
    fn a_large_download_gets_the_rule_sets_in_pieces() {
        let report = |verdict, name: Option<&str>| crate::Report {
            verdict,
            findings: name
                .map(|n| crate::Finding {
                    name: n.into(),
                    how: "rule".into(),
                    path: "-".into(),
                })
                .into_iter()
                .collect(),
            skipped: Vec::new(),
            files: 1,
            bytes: 0,
            sha256: String::new(),
            signatures_built: 0,
            signatures_age_days: None,
            signatures_versions: Vec::new(),
        };
        let line = serde_json::to_string(&report(crate::Verdict::Found, Some("Evil"))).unwrap();
        let dir = std::env::temp_dir().join(format!("xguard-deep-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let out = dir.join("report.json");
        std::fs::write(&out, format!("{line}\n")).unwrap();
        let f = std::fs::File::open("/etc/passwd").unwrap();
        // A fake worker: it must be asked for pieces, reads its input, then reports.
        let mut c = deep_command(Path::new("sh"), Path::new("/nonexistent"), true);
        let argv: Vec<String> = c
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(argv.iter().any(|a| a == "--stream"), "{argv:?}");
        c = std::process::Command::new("sh");
        c.arg("-c")
            .arg(format!("cat >/dev/null; cat {}", out.display()));
        let o = run_deep(c, &f, Duration::from_secs(10));
        assert_eq!(after_deep(o, true, true), Ok((Some("Evil".into()), false)));
        let clean = DeepOutcome::Report(report(crate::Verdict::Clean, None));
        assert_eq!(after_deep(clean, true, true), Ok((None, false)));
        let whole = DeepOutcome::Report(report(crate::Verdict::Clean, None));
        assert_eq!(after_deep(whole, true, false), Ok((None, true)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_earns_the_rule_sets_before_it_starts() {
        assert!(needs_deep(true, false, b"#!/bin/sh", "x"));
    }

    #[test]
    fn check_refuses_what_is_not_a_regular_file() {
        let start = Instant::now();
        let zero = std::fs::File::open("/dev/zero");
        assert!(zero.is_ok_and(|f| checkable(&f).is_err()));
        let dir = std::env::temp_dir().join(format!("xguard-fifo-{}", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        let c = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()).ok();
        // SAFETY: valid NUL-terminated path.
        let made = c.is_some_and(|c| unsafe { libc::mkfifo(c.as_ptr(), 0o600) } == 0);
        assert!(made);
        // O_NONBLOCK: opening a FIFO for reading with no writer must not wait.
        use std::os::unix::fs::OpenOptionsExt;
        let fifo = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&dir);
        assert!(fifo.is_ok_and(|f| checkable(&f).is_err()));
        let _ = std::fs::remove_file(&dir);
        let reg =
            std::fs::File::open("/etc/hostname").or_else(|_| std::fs::File::open("/etc/passwd"));
        assert!(reg.is_ok_and(|f| checkable(&f).is_ok()));
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(needs_deep(false, true, b"#!/bin/sh", "x"));
        assert!(needs_deep(false, false, b"MZ\x90", "x"));
        assert!(needs_deep(false, false, b"\x7fELF", "/home/a/App.AppImage"));
        assert!(needs_deep(false, false, b"", "setup.MSI"));
        assert!(!needs_deep(false, false, b"\x7fELF", "/home/a/build/a.out"));
    }

    #[test]
    fn only_the_scope_is_marked_never_the_system() {
        let info = "22 1 0:21 / / rw - btrfs /dev/x rw\n\
                    23 22 0:22 /@home /home rw - btrfs /dev/x rw\n\
                    24 22 0:23 / /tmp rw - tmpfs tmpfs rw\n\
                    25 22 0:24 / /usr rw - overlay x rw\n\
                    26 22 0:25 / /media/USB\\040Disk rw - exfat /dev/sdb1 rw\n\
                    27 22 0:26 / /run/media/ann/Stick rw - vfat /dev/sdc1 rw\n";
        assert_eq!(
            scope_mounts(info, &ROOTS),
            ["/home", "/media/USB Disk", "/run/media/ann/Stick", "/tmp"]
        );
        assert!(!scope_mounts(info, &["/"]).contains(&"/".to_string()));
    }

    #[test]
    fn download_folders_come_from_each_account() {
        let passwd = "root:x:0:0::/root:/bin/bash\n\
                      nobody:x:65534:65534::/:/bin/false\n\
                      ann:x:1000:1000::/home/ann:/bin/zsh\n\
                      bob:x:1001:1001::/home/bob:/bin/zsh\n";
        let dirs = download_dirs(
            passwd,
            &|p: &str, _| {
                p.contains("bob")
                    .then(|| "XDG_DOWNLOAD_DIR=\"$HOME/Baixados\"\n".to_string())
            },
            &|_, _, _| Some(()),
        );
        let dirs: Vec<String> = dirs.into_iter().map(|d| d.0).collect();
        assert_eq!(dirs, ["/home/ann/Downloads", "/home/bob/Baixados"]);
    }

    /// Audit C7-3: a folder outside the account's home (a system path, another account's home,
    /// a symlink out) is never watched; the account's own Downloads is.
    #[test]
    fn download_folders_stay_inside_the_accounts_home() {
        use std::os::unix::fs::MetadataExt;
        let t = std::env::temp_dir().join(format!("wol-xg-c73-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let ann = t.join("ann");
        let bob = t.join("bob");
        std::fs::create_dir_all(ann.join("Downloads")).unwrap();
        std::fs::create_dir_all(bob.join("bin")).unwrap();
        // Our uid: the owner of what this test just made.
        let uid = std::fs::metadata(&t).unwrap().uid();
        std::os::unix::fs::symlink(&bob, ann.join("out")).unwrap();
        std::os::unix::fs::symlink(ann.join("Downloads"), ann.join("dl-link")).unwrap();
        let home = ann.to_str().unwrap();
        let owned = |d: &str, h: &str, u| open_owned_dir(d, h, u).is_some();
        for bad in ["/usr/bin".to_string(), format!("{}/bin", bob.display())] {
            assert!(!owned(&bad, home, uid), "{bad}");
        }
        assert!(!owned(&format!("{home}/out/bin"), home, uid), "symlink out");
        assert!(
            !owned(&format!("{home}/dl-link"), home, uid),
            "a symlink is refused even when it points inside"
        );
        assert!(!owned(&format!("{home}/../bob/bin"), home, uid), "..");
        assert!(!owned(home, home, uid), "home itself");
        assert!(owned(&format!("{home}/Downloads"), home, uid));
        assert!(owned(&format!("{home}/Downloads/"), home, uid));
        assert!(
            !owned(&format!("{home}/Downloads"), home, uid + 1),
            "another owner"
        );
        // The descriptor is the folder that was checked, whatever happens to its name later.
        let fd = open_owned_dir(&format!("{home}/Downloads"), home, uid).unwrap();
        std::fs::rename(ann.join("Downloads"), ann.join("moved")).unwrap();
        std::os::unix::fs::symlink(&bob, ann.join("Downloads")).unwrap();
        let real = std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).unwrap();
        assert_eq!(real, ann.join("moved"));
        let passwd = "ann:x:1000:1000::/home/ann:/bin/zsh\n";
        let dirs = download_dirs(
            passwd,
            &|_, _| Some("XDG_DOWNLOAD_DIR=\"/usr/bin\"\n".into()),
            &|d, _, _| d.starts_with("/home/ann/").then_some(()),
        );
        let dirs: Vec<String> = dirs.into_iter().map(|d| d.0).collect();
        assert_eq!(dirs, ["/home/ann/Downloads"]);
        let _ = std::fs::remove_dir_all(&t);
    }

    /// Audit C7-2: a FIFO, a symlink to /dev/zero, another owner's file or a huge file never
    /// block or flood the reader.
    #[test]
    fn user_dirs_are_read_without_trusting_the_file() {
        use std::os::unix::fs::MetadataExt;
        let t = std::env::temp_dir().join(format!("wol-xg-c72-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        // Our uid: the owner of what this test just made.
        let uid = std::fs::metadata(&t).unwrap().uid();
        let fifo = t.join("fifo");
        let made = std::process::Command::new("mkfifo")
            .args(["-m", "600"])
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let zero = t.join("zero");
        std::os::unix::fs::symlink("/dev/zero", &zero).unwrap();
        let start = Instant::now();
        assert_eq!(read_user_dirs(fifo.to_str().unwrap(), uid), None);
        assert_eq!(read_user_dirs(zero.to_str().unwrap(), uid), None);
        assert_eq!(read_user_dirs("/dev/zero", uid), None);
        assert!(start.elapsed() < Duration::from_millis(500));
        let ok = t.join("ok");
        std::fs::write(&ok, "XDG_DOWNLOAD_DIR=\"$HOME/D\"\n").unwrap();
        assert!(read_user_dirs(ok.to_str().unwrap(), uid).is_some());
        assert_eq!(read_user_dirs(ok.to_str().unwrap(), uid + 1), None);
        let big = t.join("big");
        std::fs::write(&big, vec![b'#'; 1 << 20]).unwrap();
        let got = read_user_dirs(big.to_str().unwrap(), uid).unwrap();
        assert_eq!(got.len() as u64, USER_DIRS_CAP);
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn the_list_of_findings_stays_short_and_fresh() {
        let b = |p: &str, when| Blocked {
            path: p.into(),
            name: "T".into(),
            sha256: String::new(),
            when,
            uid: 1000,
        };
        let mut l = Vec::new();
        for i in 0..80 {
            remember(&mut l, b(&format!("/f{i}"), i));
        }
        assert_eq!(l.len(), KEEP);
        assert_eq!(l[0].path, "/f79");
        remember(&mut l, b("/f70", 99));
        assert_eq!(l.iter().filter(|x| x.path == "/f70").count(), 1);
        assert_eq!(l[0].when, 99);
    }

    #[test]
    fn removable_paths() {
        assert!(is_removable("/run/media/ann/Stick/x"));
        assert!(is_removable("/media/d/x"));
        assert!(!is_removable("/home/a/media/x"));
    }

    const MARKER: &str = "XGUARD-UNIT-MARKER-CAFEBABE-5678";

    /// A guard without the kernel: no fanotify descriptor, no workers, rules with one marker.
    fn bare(tag: &str) -> (Arc<Shared>, PathBuf) {
        use crate::automaton::{Pattern, Place, Tok};
        let dir = std::env::temp_dir().join(format!("xguard-unit-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["rules", "state", "work"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        let mut b = crate::db::Builder::default();
        assert!(b.rule(
            "Test.Marker",
            vec![Pattern {
                toks: vec![Tok::Lit(MARKER.as_bytes().to_vec())],
                place: Place::Anywhere,
                kinds: 0,
                id: 0,
                part: 0,
            }]
        ));
        std::fs::write(dir.join("rules/test.xgdb"), b.build(1, 1)).unwrap();
        let o = Options {
            state: dir.join("state"),
            rules: dir.join("rules"),
            scope: Vec::new(),
            downloads: Vec::new(),
            worker: None,
            socket: dir.join("sock"),
        };
        let s = Shared {
            fan: -1,
            store: Mutex::new(Store::open(&dir.join("state/verdicts")).unwrap()),
            o,
            engine: Mutex::new((None, Instant::now())),
            queue: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
            counters: Counters::default(),
            findings: Mutex::new(Vec::new()),
            rules_version: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            waker: None,
        };
        (Arc::new(s), dir)
    }

    fn asked(file: std::fs::File) -> (Job, UnixStream) {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let mut j = Job::new(file, None, false);
        j.reply = Some(ours);
        (j, theirs)
    }

    /// S4-3: a caller over its account's share of the queue hears `unchecked` at once, without
    /// anything being read from it; a freed place admits the next one.
    #[test]
    fn a_caller_over_the_cap_hears_unchecked_at_once() {
        let quota = shared::bounded::Quota::new(PER_UID, QUEUE_CAP);
        let mut held = Vec::new();
        for _ in 0..PER_UID {
            let (srv, cli) = UnixStream::pair().unwrap();
            held.push((admit(&quota, &srv).expect("within the cap"), srv, cli));
        }
        let (srv, cli) = UnixStream::pair().unwrap();
        let start = Instant::now();
        assert!(admit(&quota, &srv).is_none());
        drop(srv);
        let mut line = String::new();
        std::io::Read::read_to_string(&mut &cli, &mut line).unwrap();
        let r: Reply = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(r.state, "unchecked");
        assert!(start.elapsed() < Duration::from_millis(500));
        held.pop();
        let (srv, _cli) = UnixStream::pair().unwrap();
        assert!(admit(&quota, &srv).is_some(), "a freed place admits again");
        const { assert!(PER_UID < QUEUE_CAP) };
    }

    /// S4-3: questions about one file share one scan and one answer.
    #[test]
    fn questions_about_one_file_ride_along_and_share_the_answer() {
        let (s, dir) = bare("dedup");
        let p = dir.join("work/tool");
        std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
        let other = dir.join("work/other");
        std::fs::write(&other, "#!/bin/sh\nexit 1\n").unwrap();
        let mut ends = Vec::new();
        for _ in 0..5 {
            let (j, end) = asked(std::fs::File::open(&p).unwrap());
            s.enqueue(j);
            ends.push(end);
        }
        let (j, _o) = asked(std::fs::File::open(&other).unwrap());
        s.enqueue(j);
        // An Open Anyway about the same file is its own question.
        let (mut j, _a) = asked(std::fs::File::open(&p).unwrap());
        j.allow = true;
        s.enqueue(j);
        {
            let q = s.queue.lock().unwrap();
            assert_eq!(q.len(), 3, "one lead per file and verb");
            assert_eq!(q[0].followers.len(), 4);
        }
        let mut lead = s.queue.lock().unwrap().pop_front().unwrap();
        let (a, name) = s.check(&mut lead);
        assert_eq!((a, name.as_str()), (Answer::Allow, ""));
        s.finish(&lead, a, &name);
        for e in &ends {
            let mut line = String::new();
            std::io::BufRead::read_line(&mut std::io::BufReader::new(e), &mut line).unwrap();
            assert!(line.contains("\"clean\""), "{line}");
        }
        assert_eq!(
            s.counters.scanned.load(Relaxed),
            1,
            "five questions, one scan"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// S4-5: a file above LARGE that somebody asks about (or that was downloaded) is never
    /// allowed unread: the streamed pass refuses a pattern in its first bytes, and without one
    /// the answer is "unchecked", not remembered.
    #[test]
    fn a_large_asked_file_is_unchecked_or_refused_never_allowed() {
        let (s, dir) = bare("large");
        let clean = dir.join("work/big-clean.AppImage");
        std::fs::File::create(&clean)
            .unwrap()
            .set_len(LARGE + 1)
            .unwrap();
        let (mut j, _e) = asked(std::fs::File::open(&clean).unwrap());
        assert_eq!(s.check(&mut j).0, Answer::Unchecked);
        let id = FileId::of(&j.file).unwrap();
        assert!(
            s.store.lock().unwrap().by_id(&id).is_none(),
            "nothing remembered"
        );

        let bad = dir.join("work/big-bad.AppImage");
        std::fs::write(&bad, format!("ELF {MARKER}")).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&bad)
            .unwrap()
            .set_len(LARGE + 1)
            .unwrap();
        let (mut j, _e) = asked(std::fs::File::open(&bad).unwrap());
        let (a, name) = s.check(&mut j);
        assert_eq!((a, name.as_str()), (Answer::Deny, "Test.Marker"));

        // A tagged download from the kernel (no caller) gets the same rule, when the
        // filesystem keeps user xattrs.
        let f = std::fs::File::open(&clean).unwrap();
        set_tag(f.as_raw_fd());
        if is_tagged(f.as_raw_fd()) {
            let mut j = Job::new(f, None, false);
            assert_eq!(s.check(&mut j).0, Answer::Unchecked);
        }
        // An untagged file nobody asked about runs at once; the sweep looks later.
        let plain = dir.join("work/big-plain");
        std::fs::File::create(&plain)
            .unwrap()
            .set_len(LARGE + 1)
            .unwrap();
        let mut j = Job::new(std::fs::File::open(&plain).unwrap(), None, false);
        assert_eq!(s.check(&mut j).0, Answer::Allow);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// S4-10: an account's findings are its own from the first moment: 0600 and owned by it.
    #[test]
    fn findings_are_written_for_their_account_only() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("xguard-findings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let me = shared::sys::euid();
        let b = Blocked {
            path: "/home/a/x".into(),
            name: "T".into(),
            sha256: String::new(),
            when: 1,
            uid: me,
        };
        save(&dir, &[b], me);
        let m = std::fs::metadata(dir.join(findings_file(me))).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o600);
        assert_eq!(m.uid(), me);
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [findings_file(me)], "no temporary left");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
