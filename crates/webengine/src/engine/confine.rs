// SPDX-License-Identifier: BSD-2-Clause
//! The confinement of the engine's content process (ADR 0042). The engine's own content sandbox
//! (a user namespace and a chroot, entered from a multi-threaded host, under a 2015 allowlist of 21
//! system calls) does not load a page at the pin, so the shim holds the content process itself,
//! before the engine runs a line of the page's script ([`enter`], called by
//! `wwe_run_content_process` while the process still has one thread):
//!
//! - `no_new_privs`, so nothing it executes or maps can gain a privilege;
//! - a Landlock ruleset ([`Plan`]): files are read-only and only the system's (`/usr`, the fonts, a
//!   few files of `/etc`, this process's own `/proc` and the CPU and cgroup facts of `/sys`); no
//!   home directory, no profile directory (it is the host's: the content process has no storage),
//!   no `/tmp`, no `/run`, nothing writable but `/dev/null`; no TCP bind or connect; no signal to
//!   a process outside it and no connect to an abstract unix socket outside it; and where the
//!   kernel has `RESOLVE_UNIX`, the only pathname unix socket it can connect to is the engine's
//!   IPC bootstrap socket (the directory of the token);
//! - a seccomp filter ([`program`]): `socket` and `socketpair` for `AF_UNIX` only (no IP, no raw,
//!   no netlink, no packet socket: the page's network goes through the host), no `execve`, no
//!   `ptrace`, no mount or namespace call, no `bpf`, no `io_uring`, no `perf_event_open`, no key
//!   ring, no module or reboot call, and a clone that asks for a new namespace is refused.
//!
//! It fails closed: a kernel without Landlock [`MIN_ABI`], an architecture the filter has no table
//! for, a rule that cannot be added or a filter that cannot be installed is an error and the
//! process does not run the page. [`preflight`] asks the same of the host before the engine starts
//! so the failure is an error of `wwe_engine_new` and not a content process that dies while the
//! host waits for it.
//!
//! The plain-Rust half (the plan and the filter program) is data: `tests/confinement.rs` includes
//! this file, runs the program against a model of the kernel's filter machine and confines a child
//! process of its own, without the engine.
use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};

/// The oldest Landlock ABI that is enough: filesystem rules (1-3, 5), TCP rules (4) and the
/// scopes of abstract unix sockets and signals (6).
pub const MIN_ABI: u32 = 6;
/// Whether a kernel without `LANDLOCK_ACCESS_FS_RESOLVE_UNIX` (pathname unix sockets cannot be
/// told apart) is refused. Off: the content process runs and `enter` reports `unix_resolve:
/// false`, which the engine library logs; a pathname socket of the user (the session bus, the
/// manager's private socket) is then reachable from the content process. On: it does not run.
pub const REQUIRE_UNIX_RESOLVE: bool = true;

// Landlock (linux/landlock.h; the numbers are the same on every architecture).
const SYS_CREATE_RULESET: libc::c_long = 444;
const SYS_ADD_RULE: libc::c_long = 445;
const SYS_RESTRICT_SELF: libc::c_long = 446;
const CREATE_RULESET_VERSION: usize = 1;
const PR_SET_NO_NEW_PRIVS: usize = 38;
const RULE_PATH_BENEATH: usize = 1;
const FS_EXECUTE: u64 = 1 << 0;
const FS_WRITE_FILE: u64 = 1 << 1;
const FS_READ_FILE: u64 = 1 << 2;
const FS_READ_DIR: u64 = 1 << 3;
/// Every filesystem access of ABI 1 to 5 (bits 0 to 15): handled, so denied unless a rule grants it.
const FS_ALL: u64 = (1 << 16) - 1;
const FS_RESOLVE_UNIX: u64 = 1 << 16;
const NET_ALL: u64 = 0b11; // bind and connect of TCP
const SCOPE_ALL: u64 = 0b11; // abstract unix sockets and signals
/// The file-only accesses: a rule on a file with a directory access is refused by the kernel.
const FS_FILE_ONLY: u64 = FS_EXECUTE | FS_WRITE_FILE | FS_READ_FILE;

/// What a path of the plan may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    /// Read files and list directories.
    Read,
    /// Read and execute (the system's libraries are mapped executable; `execve` itself is refused
    /// by the filter).
    ReadExec,
    /// Read and write a device file.
    Device,
    /// Connect to the pathname unix sockets in the directory (kernels with `RESOLVE_UNIX`).
    Sockets,
}

impl Grant {
    fn bits(self) -> u64 {
        match self {
            Grant::Read => FS_READ_FILE | FS_READ_DIR,
            Grant::ReadExec => FS_READ_FILE | FS_READ_DIR | FS_EXECUTE,
            Grant::Device => FS_READ_FILE | FS_WRITE_FILE,
            Grant::Sockets => FS_RESOLVE_UNIX,
        }
    }
}

/// The paths a content process may use and how. Everything else is denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub paths: Vec<(PathBuf, Grant)>,
}

/// The files and directories of the system a page's script and layout read.
const SYSTEM: &[&str] = &["/usr", "/lib", "/lib64", "/bin", "/sbin"];
const READ: &[&str] = &[
    "/etc/fonts",
    "/etc/ld.so.cache",
    "/etc/ld.so.conf",
    "/etc/ld.so.conf.d",
    "/etc/localtime",
    "/etc/timezone",
    "/etc/os-release",
    "/etc/passwd",
    "/etc/group",
    "/etc/nsswitch.conf",
    "/var/cache/fontconfig",
    "/proc/self",
    "/proc/cpuinfo",
    "/proc/meminfo",
    "/proc/sys/kernel",
    "/proc/sys/vm",
    "/sys/devices/system",
    "/sys/fs/cgroup",
    "/dev/urandom",
    "/dev/random",
    "/dev/zero",
];
const DEVICES: &[&str] = &["/dev/null"];

/// The directory of the engine's bootstrap socket: the token is its absolute path
/// (`<tmp>/<random>/socket`). The directory must be a private one two levels down: a token that
/// names `/socket` or `/tmp/socket` would open every socket of `/` or `/tmp` (the display's, the
/// agent's).
pub fn socket_dir(token: &str) -> Result<PathBuf, String> {
    let path = Path::new(token);
    let plain = path
        .components()
        .all(|c| matches!(c, Component::RootDir | Component::Normal(_)));
    let tidy = !token.contains("//") && !token.contains("/./") && !token.ends_with('/');
    let dir = path
        .parent()
        .filter(|_| path.is_absolute() && plain && tidy);
    match dir {
        Some(d) if d.components().count() >= 3 && !token.contains('\0') => Ok(d.to_path_buf()),
        _ => Err(format!(
            "the content process token {token:?} is not the path of a socket in a private directory"
        )),
    }
}

impl Plan {
    /// The plan for the content process started with `token`.
    pub fn new(token: &str) -> Result<Plan, String> {
        let mut paths = Vec::new();
        paths.extend(SYSTEM.iter().map(|p| (PathBuf::from(p), Grant::ReadExec)));
        paths.extend(READ.iter().map(|p| (PathBuf::from(p), Grant::Read)));
        paths.extend(DEVICES.iter().map(|p| (PathBuf::from(p), Grant::Device)));
        paths.push((socket_dir(token)?, Grant::Sockets));
        Ok(Plan { paths })
    }
}

// Seccomp (linux/seccomp.h, linux/filter.h, linux/audit.h).
const BPF_LD_W_ABS: u16 = 0x20;
const BPF_JEQ_K: u16 = 0x15;
const BPF_JGE_K: u16 = 0x35;
const BPF_JSET_K: u16 = 0x45;
const BPF_RET_K: u16 = 0x06;
const RET_KILL_PROCESS: u32 = 0x8000_0000;
const RET_ERRNO: u32 = 0x0005_0000;
const RET_ALLOW: u32 = 0x7fff_0000;
const OFF_NR: u32 = 0;
const OFF_ARCH: u32 = 4;
const OFF_ARG0: u32 = 16;
const AF_UNIX: u32 = 1;
const EPERM: u32 = 1;
const ENOSYS: u32 = 38;
/// `CLONE_NEWNS | NEWCGROUP | NEWUTS | NEWIPC | NEWUSER | NEWPID | NEWNET | NEWTIME`.
pub const CLONE_NEW_MASK: u32 = 0x0002_0000
    | 0x0200_0000
    | 0x0400_0000
    | 0x0800_0000
    | 0x1000_0000
    | 0x2000_0000
    | 0x4000_0000
    | 0x80;
/// x86-64 numbers the x32 ABI at this bit; the filter refuses all of them.
const X32_BIT: u32 = 0x4000_0000;

/// One instruction of a classic BPF program (`struct sock_filter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Insn {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

const fn insn(code: u16, jt: u8, jf: u8, k: u32) -> Insn {
    Insn { code, jt, jf, k }
}

/// The system call numbers of one architecture the filter is built from.
#[derive(Debug, Clone)]
pub struct Table {
    /// `AUDIT_ARCH_*` of the architecture; a call under another is killed.
    pub arch: u32,
    /// x86-64: refuse the x32 numbers.
    pub x32: bool,
    /// Refused with `EPERM`.
    pub deny: Vec<u32>,
    pub clone: u32,
    pub clone3: u32,
    /// `socket` and `socketpair`: only `AF_UNIX`.
    pub sockets: [u32; 2],
}

/// Where a jump goes: the next instruction or a label.
#[derive(Clone, Copy)]
enum To {
    Next,
    Label(usize),
}

const KILL: usize = 0;
const DENY: usize = 1;
const NOSYS: usize = 2;
const ALLOW: usize = 3;
const CLONE: usize = 4;
const SOCKET: usize = 5;
const LABELS: usize = 6;

/// A program with labels, resolved to the relative jumps of BPF.
struct Asm {
    items: Vec<(u16, u32, To, To)>,
    at: [Option<usize>; LABELS],
}

impl Asm {
    fn new() -> Asm {
        Asm {
            items: Vec::new(),
            at: [None; LABELS],
        }
    }
    fn op(&mut self, code: u16, k: u32, jt: To, jf: To) {
        self.items.push((code, k, jt, jf));
    }
    fn bind(&mut self, label: usize) {
        self.at[label] = Some(self.items.len());
    }
    fn finish(self) -> Result<Vec<Insn>, String> {
        let jump = |from: usize, to: To| match to {
            To::Next => Ok(0),
            To::Label(l) => {
                let target = self.at[l].ok_or("a label of the filter is not bound")?;
                u8::try_from(target - from - 1)
                    .map_err(|_| "the filter's jump is too far".to_string())
            }
        };
        self.items
            .iter()
            .enumerate()
            .map(|(i, &(code, k, jt, jf))| Ok(insn(code, jump(i, jt)?, jump(i, jf)?, k)))
            .collect()
    }
}

/// The filter as a program: killed under a foreign architecture; `deny` and the x32 numbers
/// `EPERM`; `clone3` `ENOSYS` (its flags sit in memory the filter cannot read, and libc falls back
/// to `clone`); a `clone` with a namespace flag, and a socket that is not `AF_UNIX`, `EPERM`; every
/// other call allowed (what the page may do is the Landlock ruleset's to decide).
pub fn program(t: &Table) -> Result<Vec<Insn>, String> {
    let mut a = Asm::new();
    a.op(BPF_LD_W_ABS, OFF_ARCH, To::Next, To::Next);
    a.op(BPF_JEQ_K, t.arch, To::Next, To::Label(KILL));
    a.op(BPF_LD_W_ABS, OFF_NR, To::Next, To::Next);
    if t.x32 {
        a.op(BPF_JGE_K, X32_BIT, To::Label(DENY), To::Next);
    }
    for &nr in &t.deny {
        a.op(BPF_JEQ_K, nr, To::Label(DENY), To::Next);
    }
    a.op(BPF_JEQ_K, t.clone3, To::Label(NOSYS), To::Next);
    a.op(BPF_JEQ_K, t.clone, To::Label(CLONE), To::Next);
    for nr in t.sockets {
        a.op(BPF_JEQ_K, nr, To::Label(SOCKET), To::Next);
    }
    a.op(BPF_RET_K, RET_ALLOW, To::Next, To::Next);
    // clone: a namespace flag in the first argument is refused.
    a.bind(CLONE);
    a.op(BPF_LD_W_ABS, OFF_ARG0, To::Next, To::Next);
    a.op(
        BPF_JSET_K,
        CLONE_NEW_MASK,
        To::Label(DENY),
        To::Label(ALLOW),
    );
    // socket and socketpair: the domain (first argument) must be AF_UNIX.
    a.bind(SOCKET);
    a.op(BPF_LD_W_ABS, OFF_ARG0, To::Next, To::Next);
    a.op(BPF_JEQ_K, AF_UNIX, To::Label(ALLOW), To::Label(DENY));
    a.bind(DENY);
    a.op(BPF_RET_K, RET_ERRNO | EPERM, To::Next, To::Next);
    a.bind(NOSYS);
    a.op(BPF_RET_K, RET_ERRNO | ENOSYS, To::Next, To::Next);
    a.bind(ALLOW);
    a.op(BPF_RET_K, RET_ALLOW, To::Next, To::Next);
    a.bind(KILL);
    a.op(BPF_RET_K, RET_KILL_PROCESS, To::Next, To::Next);
    a.finish()
}

/// The filter for this architecture; `None` where it has no table (the content process is then
/// not confined, so it does not run).
pub fn native_table() -> Option<Table> {
    #[cfg(target_arch = "x86_64")]
    const ARCH: Option<(u32, bool)> = Some((0xc000_003e, true));
    #[cfg(target_arch = "aarch64")]
    const ARCH: Option<(u32, bool)> = Some((0xc000_00b7, false));
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    const ARCH: Option<(u32, bool)> = None;
    let (arch, x32) = ARCH?;
    // A system call number is a small positive constant; the one that is not would be u32::MAX,
    // which the x32 check (and the kernel) refuse.
    let nr = |n: libc::c_long| u32::try_from(n).unwrap_or(u32::MAX);
    let deny = [
        libc::SYS_execve,
        libc::SYS_execveat,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_kcmp,
        libc::SYS_pidfd_getfd,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_fsopen,
        libc::SYS_fsconfig,
        libc::SYS_fsmount,
        libc::SYS_fspick,
        libc::SYS_move_mount,
        libc::SYS_open_tree,
        libc::SYS_mount_setattr,
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_open_by_handle_at,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_userfaultfd,
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_kexec_load,
        libc::SYS_kexec_file_load,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_reboot,
        libc::SYS_swapon,
        libc::SYS_swapoff,
        libc::SYS_acct,
        libc::SYS_quotactl,
        libc::SYS_syslog,
    ];
    Some(Table {
        arch,
        x32,
        deny: deny.into_iter().map(nr).collect(),
        clone: nr(libc::SYS_clone),
        clone3: nr(libc::SYS_clone3),
        sockets: [nr(libc::SYS_socket), nr(libc::SYS_socketpair)],
    })
}

/// One system call; the error is the `errno`. Private: the arguments are integers and addresses
/// of values the caller keeps alive for the call, and each use below says which.
fn sys(nr: libc::c_long, a: [usize; 3]) -> Result<usize, i32> {
    // SAFETY: only the calls below pass through here, each with integers or the address of a live
    // value of the type the call reads (a ruleset or rule attribute, or a filter program).
    let r = unsafe { libc::syscall(nr, a[0], a[1], a[2], 0usize, 0usize) };
    usize::try_from(r).map_err(|_| std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
}

/// The kernel's Landlock ABI, or why there is none.
pub fn landlock_abi() -> Result<u32, String> {
    match sys(SYS_CREATE_RULESET, [0, 0, CREATE_RULESET_VERSION]) {
        Ok(v) => Ok(u32::try_from(v).unwrap_or(0)),
        Err(e) => Err(format!(
            "the kernel has no Landlock ({}); the web runtime needs ABI {MIN_ABI} (Linux 6.12)",
            std::io::Error::from_raw_os_error(e)
        )),
    }
}

/// Whether this host can confine a content process: the ABI, the architecture. Returns the ABI.
pub fn preflight() -> Result<u32, String> {
    let abi = landlock_abi()?;
    if abi < MIN_ABI {
        return Err(format!(
            "Landlock ABI {abi} is too old to confine the content process; the web runtime needs ABI {MIN_ABI} (Linux 6.12)"
        ));
    }
    if native_table().is_none() {
        return Err("the content process filter has no table for this architecture".into());
    }
    Ok(abi)
}

/// What [`enter`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub abi: u32,
    /// Pathname unix sockets are confined to the bootstrap directory (the kernel has
    /// `RESOLVE_UNIX`).
    pub unix_resolve: bool,
    /// Paths of the plan that do not exist here (nothing is granted for them).
    pub missing: Vec<PathBuf>,
}

#[repr(C)]
struct RulesetAttr {
    handled_fs: u64,
    handled_net: u64,
    scoped: u64,
}

#[repr(C, packed)]
struct PathBeneath {
    allowed: u64,
    parent_fd: i32,
}

#[repr(C)]
struct Fprog {
    len: u16,
    filter: *const Insn,
}

fn add_rule(ruleset: i32, path: &Path, grant: u64) -> Result<bool, String> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let dir = file
        .metadata()
        .map_err(|e| format!("{}: {e}", path.display()))?
        .is_dir();
    let allowed = if dir { grant } else { grant & FS_FILE_ONLY };
    let attr = PathBeneath {
        allowed,
        parent_fd: file.as_raw_fd(),
    };
    let ruleset = usize::try_from(ruleset).unwrap_or(0);
    sys(
        SYS_ADD_RULE,
        [
            ruleset,
            RULE_PATH_BENEATH,
            (&raw const attr).expose_provenance(),
        ],
    )
    .map_err(|e| {
        format!(
            "{}: {}",
            path.display(),
            std::io::Error::from_raw_os_error(e)
        )
    })?;
    Ok(true)
}

/// The ruleset for `plan`; with `RESOLVE_UNIX` handled when the kernel knows it. Returns its fd.
fn ruleset(unix_resolve: bool) -> Result<i32, i32> {
    let attr = RulesetAttr {
        handled_fs: FS_ALL | if unix_resolve { FS_RESOLVE_UNIX } else { 0 },
        handled_net: NET_ALL,
        scoped: SCOPE_ALL,
    };
    let fd = sys(
        SYS_CREATE_RULESET,
        [
            (&raw const attr).expose_provenance(),
            std::mem::size_of::<RulesetAttr>(),
            0,
        ],
    )?;
    i32::try_from(fd).map_err(|_| 0)
}

fn one_thread() -> Result<(), String> {
    let n = std::fs::read_dir("/proc/self/task")
        .map_err(|e| format!("/proc/self/task: {e}"))?
        .count();
    if n == 1 {
        Ok(())
    } else {
        Err(format!(
            "the content process has {n} threads; it is confined before any thread but the first starts"
        ))
    }
}

/// Confines this process for good: `no_new_privs`, the Landlock ruleset of [`Plan::new`]`(token)`
/// and the seccomp filter of [`native_table`]. Call it as the first thing a content process does,
/// before the engine starts a thread; later threads and children inherit it.
pub fn enter(token: &str) -> Result<Applied, String> {
    let abi = preflight()?;
    let plan = Plan::new(token)?;
    let table = native_table().ok_or("no filter table for this architecture")?;
    one_thread()?;
    let errno = |what: &'static str| {
        move |e: i32| format!("{what}: {}", std::io::Error::from_raw_os_error(e))
    };
    sys(libc::SYS_prctl, [PR_SET_NO_NEW_PRIVS, 1, 0]).map_err(errno("no_new_privs"))?;
    let (fd, unix_resolve) = match ruleset(true) {
        Ok(fd) => (fd, true),
        Err(libc::EINVAL) if !REQUIRE_UNIX_RESOLVE => {
            (ruleset(false).map_err(errno("landlock ruleset"))?, false)
        }
        Err(libc::EINVAL) => return Err("the kernel has no Landlock RESOLVE_UNIX".into()),
        Err(e) => return Err(errno("landlock ruleset")(e)),
    };
    let mut missing = Vec::new();
    for (path, grant) in &plan.paths {
        if *grant == Grant::Sockets && !unix_resolve {
            continue;
        }
        if !add_rule(fd, path, grant.bits())? {
            missing.push(path.clone());
        }
    }
    // The ruleset's descriptor is close-on-exec and ours; it goes once the domain is entered.
    let raw = usize::try_from(fd).unwrap_or(0);
    sys(SYS_RESTRICT_SELF, [raw, 0, 0]).map_err(errno("landlock restrict"))?;
    sys(libc::SYS_close, [raw, 0, 0]).map_err(errno("close"))?;
    let prog = program(&table)?;
    let fprog = Fprog {
        len: u16::try_from(prog.len()).map_err(|_| "the filter is too long")?,
        filter: prog.as_ptr(),
    };
    // SECCOMP_SET_MODE_FILTER = 1, SECCOMP_FILTER_FLAG_TSYNC = 1.
    sys(
        libc::SYS_seccomp,
        [1, 1, (&raw const fprog).expose_provenance()],
    )
    .map_err(errno("seccomp"))?;
    Ok(Applied {
        abi,
        unix_resolve,
        missing,
    })
}
