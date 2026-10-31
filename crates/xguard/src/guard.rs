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
//! working. When this process ends, the kernel lets every waiting call through.
use crate::ctl::{self, Reply};
use crate::scan::{Engine, kind, now};
use crate::store::{Entry, FileId, State, Store};
use crate::{Kind, hexs};
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
/// The database stays mapped this long after the last scan.
const LINGER: Duration = Duration::from_secs(30);
/// Findings kept for the page.
const KEEP: usize = 50;
const THREADS: usize = 3;

/// What the guard needs to do on a kernel question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Allow,
    Deny,
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

/// Downloads folder of every login account: (`/etc/passwd`, user-dirs reader).
pub fn download_dirs(passwd: &str, user_dirs: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    passwd
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            let uid: u32 = f.get(2)?.parse().ok()?;
            let home = *f.get(5)?;
            ((1000..60000).contains(&uid) && home.starts_with("/home/")).then_some(home)
        })
        .map(|home| {
            let dirs = user_dirs(&format!("{home}/.config/user-dirs.dirs"));
            wol_core::downloads::dir(dirs.as_deref(), home)
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

/// Spawn one deep scan: a fresh process with no privileges, the file on its standard input.
fn deep_scan(worker: &Path, rules: &Path, file: &std::fs::File) -> Option<crate::Report> {
    use std::os::unix::process::CommandExt;
    let clone = file.try_clone().ok()?;
    let mut c = std::process::Command::new(worker);
    c.args(["scan", "--json", "--rules"]).arg(rules).arg("-");
    c.stdin(clone).stderr(std::process::Stdio::null());
    // SAFETY: prctl and setresuid are async-signal-safe.
    unsafe {
        c.pre_exec(|| {
            libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
            Ok(())
        });
    }
    c.uid(65534).gid(65534);
    let out = c.output().ok()?;
    serde_json::from_slice(out.stdout.split(|b| *b == b'\n').next()?).ok()
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

    /// Read a whole file (first use only), hash and match it. None: unreadable or no signatures.
    fn lean(&self, f: &mut std::fs::File, path: &str) -> Option<(Vec<String>, [u8; 32])> {
        let e = self.engine()?;
        self.counters.scanned.fetch_add(1, Relaxed);
        let mut d = Vec::new();
        f.read_to_end(&mut d).ok()?;
        let sha = <[u8; 32]>::from(Sha256::digest(&d));
        let names = e.findings(&d, Some(&sha), path);
        Some((names.into_iter().map(|f| f.name).collect(), sha))
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
        let (Ok(id), Some((_, sha))) = (FileId::of(&j.file), self.lean(&mut j.file, &path)) else {
            return (Answer::Allow, String::new());
        };
        let key: [u8; 16] = sha[..16].try_into().expect("16 bytes");
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
        if id.size > LARGE && !deep {
            return (Answer::Allow, String::new());
        }
        let Some((names, sha)) = self.lean(&mut j.file, &path) else {
            return (Answer::Allow, String::new());
        };
        let rules = self.rules_version.load(Relaxed) as u32;
        let key: [u8; 16] = sha[..16].try_into().expect("16 bytes");
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
            if let Some(r) = deep_scan(w, &self.o.rules, &j.file) {
                ran_deep = true;
                found = r.findings.first().map(|f| f.name.clone());
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
            let (a, name) = if j.allow {
                self.allow(&mut j)
            } else {
                self.check(&mut j)
            };
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
        write_json(&self.o.state.join(STATUS), &v);
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
        let job = Job {
            event: Some(fd),
            open_mark,
            reply: None,
            allow: false,
            file,
        };
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

    fn enqueue(&self, job: Job) {
        let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        q.push_back(job);
        self.ready.notify_one();
    }

    /// One caller on the socket.
    fn client(&self, s: UnixStream) {
        let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
        let Ok((verb, Some(fd))) = ctl::recv_fd(&s) else {
            return;
        };
        let file = std::fs::File::from(fd);
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
        self.enqueue(Job {
            event: None,
            open_mark: false,
            reply: Some(s),
            allow,
            file,
        });
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

fn write_json(path: &Path, v: &impl serde::Serialize) {
    let tmp = path.with_extension("tmp");
    if let Ok(t) = serde_json::to_string(v)
        && std::fs::write(&tmp, t).is_ok()
    {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// One account's list: only that account can read it, since the paths are its own.
fn save(state: &Path, all: &[Blocked], uid: u32) {
    use std::os::unix::fs::PermissionsExt;
    let path = state.join(findings_file(uid));
    let mine: Vec<&Blocked> = all.iter().filter(|b| b.uid == uid).collect();
    write_json(&path, &mine);
    let _ = std::os::unix::fs::chown(&path, Some(uid), None);
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
}

/// Every account's list from a former run.
fn load_findings(state: &Path) -> Vec<Blocked> {
    let Ok(rd) = std::fs::read_dir(state) else {
        return Vec::new();
    };
    let texts: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("findings-"))
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
    let dirs: Vec<String> = if o.downloads.is_empty() {
        let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
        download_dirs(&passwd, &|p: &str| std::fs::read_to_string(p).ok())
    } else {
        o.downloads
            .iter()
            .map(|p| p.display().to_string())
            .collect()
    };
    for d in dirs {
        mark_path(fan, FAN_MARK_ADD, FAN_CLOSE_WRITE | FAN_EVENT_ON_CHILD, &d);
    }
}

/// Runs until the process ends.
pub fn run(o: Options) -> Result<(), String> {
    std::fs::create_dir_all(&o.state).map_err(|e| e.to_string())?;
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
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    for _ in 0..THREADS {
        let s = s.clone();
        std::thread::spawn(move || s.worker());
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
            fd: listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let (mut last, mut dirty) = (Instant::now(), false);
    loop {
        // No timer at rest: with nothing to tidy, this process sleeps until the kernel asks.
        let wake = if dirty { 5_000 } else { -1 };
        // SAFETY: three valid pollfds.
        let n = unsafe { libc::poll(pfd.as_mut_ptr(), 3, wake) };
        if n > 0 && pfd[2].revents != 0 {
            while let Ok((c, _)) = listener.accept() {
                let _ = c.set_nonblocking(false);
                s.client(c);
            }
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
                at += m.event_len as usize;
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

    #[test]
    fn what_earns_the_rule_sets_before_it_starts() {
        assert!(needs_deep(true, false, b"#!/bin/sh", "x"));
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
        let dirs = download_dirs(passwd, &|p: &str| {
            p.contains("bob")
                .then(|| "XDG_DOWNLOAD_DIR=\"$HOME/Baixados\"\n".to_string())
        });
        assert_eq!(dirs, ["/home/ann/Downloads", "/home/bob/Baixados"]);
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
}
