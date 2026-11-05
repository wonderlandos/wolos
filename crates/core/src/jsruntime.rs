// SPDX-License-Identifier: BSD-2-Clause
//! Runtime tuning for the JavaScript engines under third-party apps (ADR 0061; every number is
//! in docs/bench/js-runtime-x.md, the full table in docs/modules/runtime-tuning.md).
//!
//! Two layers, one source of truth each:
//!
//! 1. **System** ([`MM_WRITES`], [`ksm_policy`]): the kernel knobs the apps' memory answers to,
//!    written at boot by `wolos-mm apply` (`files/usr/lib/systemd/system/wolos-mm.service`), and the
//!    rule that KSM runs only on a machine with one human account.
//! 2. **Per app class** ([`runtime_flags`]): the command line switches, environment and process
//!    settings ([`Runtime`]) for an Electron app (V8), a WebKitGTK app (JSC) or Bun (JSC). A row
//!    reaches the launch only when it is [`Status::Shipped`]; a row measured without a gain stays
//!    in the table as [`Status::NoGain`] with its number, so nobody tries it a second time.
//!
//! The launch path (`memsaver::budget`, `webruntime::third_party_flags`) owns *when* an app is
//! started in a scope; this module only says *what* the engine is given. It touches no file and no
//! process: the caller applies [`Runtime::args`], [`Runtime::env`] and, for
//! [`Runtime::ksm_merge`], `prctl(PR_SET_MEMORY_MERGE)` before the exec.
//!
//! The kernel calls (KSM opt-in, page-out) are `/usr/bin/wolos-mm` (crates/controller/src/bin/wolos-mm.rs),
//! the one program here with `unsafe`; [`Runtime::prefix`] is how a launcher puts it in front of an app.
//!
//! [`pageout_ranges`] is the pure half of the page-out that took Discord to 96 MiB of PSS in the
//! background: the ranges of `/proc/<pid>/maps` that `process_madvise(MADV_PAGEOUT)` is asked
//! about. Both callers use it: `wolos-mm pageout PID` and Memory Saver (`crates/api`).
//! [`pageout_allowed`] is the rule for paging out another process.

/// Which engine an app runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Electron or Chromium: V8, PartitionAlloc.
    Electron,
    /// WebKitGTK: JavaScriptCore, bmalloc.
    WebKitGtk,
    /// Bun: JavaScriptCore, mimalloc.
    Bun,
}

impl Class {
    /// Every class.
    pub const ALL: [Class; 3] = [Class::Electron, Class::WebKitGtk, Class::Bun];

    /// Stable word for tables and logs.
    pub fn id(self) -> &'static str {
        match self {
            Class::Electron => "electron",
            Class::WebKitGtk => "webkitgtk",
            Class::Bun => "bun",
        }
    }

    /// The class of a program by file name (path or name); None: not an engine host this module
    /// knows (the caller knows Electron apps from its own table, there is no safe guess by name).
    pub fn of_exec(argv0: &str) -> Option<Class> {
        let name = argv0.rsplit('/').next().unwrap_or(argv0);
        match name {
            "bun" => Some(Class::Bun),
            "MiniBrowser" | "epiphany" | "WebKitWebProcess" => Some(Class::WebKitGtk),
            _ => None,
        }
    }
}

/// How hard the runtime trades speed for memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// What costs no speed anyone can measure.
    Default,
    /// Also the rows that cost script speed (no JIT): for apps held to a memory budget.
    Lean,
}

/// What one row sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knob {
    /// One command line word (`--name` or `--name=value`).
    Arg(&'static str),
    /// One V8 switch, merged into the single `--js-flags=` word.
    JsFlag(&'static str),
    /// One environment variable.
    Env(&'static str, &'static str),
    /// `prctl(PR_SET_MEMORY_MERGE, 1)` before the exec: the process tree is scanned by KSM.
    KsmMerge,
}

/// Whether a row reaches the launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Measured and shipped.
    Shipped,
    /// Measured, saves memory, costs speed: only in [`Mode::Lean`].
    LeanOnly,
    /// Measured without a gain (or worse): in the table so it is not tried again.
    NoGain,
}

/// One measured row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: &'static str,
    pub class: Class,
    pub knob: Knob,
    pub status: Status,
    /// MiB of PSS the row saved in the benchmark (negative: it cost memory). Whole app, same
    /// start, same minute; the run-to-run spread is about 5 MiB.
    pub saves_mib: i32,
    /// What it measured on, and what it cost.
    pub note: &'static str,
}

impl Row {
    fn applies(&self, class: Class, mode: Mode) -> bool {
        self.class == class
            && match self.status {
                Status::Shipped => true,
                Status::LeanOnly => mode == Mode::Lean,
                Status::NoGain => false,
            }
    }
}

/// Every engine knob WolOS measured. Order is the order on the command line and in the environment.
pub const ROWS: &[Row] = &[
    // Electron / Chromium / V8 (Discord 1.0.168, Electron 44.10, Chromium 152, login page).
    Row {
        id: "electron-ksm",
        class: Class::Electron,
        knob: Knob::KsmMerge,
        status: Status::Shipped,
        saves_mib: 29,
        note: "anon 307 -> 280 MiB after 2 minutes (6.2k pages merged, PartitionAlloc 142 -> 113); ksmd 2% of a core, falling",
    },
    Row {
        id: "electron-lite-mode",
        class: Class::Electron,
        knob: Knob::JsFlag("--lite-mode"),
        status: Status::LeanOnly,
        saves_mib: 13,
        note: "implies jitless and optimize-for-size: WebAssembly stops working and scripts run several times slower",
    },
    Row {
        id: "electron-optimize-for-size",
        class: Class::Electron,
        knob: Knob::JsFlag("--optimize-for-size"),
        status: Status::NoGain,
        saves_mib: -5,
        note: "with --max-semi-space-size=1: 541 MiB against 534, inside the run-to-run spread",
    },
    Row {
        id: "electron-single-threaded-gc",
        class: Class::Electron,
        knob: Knob::JsFlag("--single-threaded-gc"),
        status: Status::NoGain,
        saves_mib: 0,
        note: "with --max-lazy: 536 MiB against 534",
    },
    Row {
        id: "electron-in-process-gpu",
        class: Class::Electron,
        knob: Knob::Arg("--in-process-gpu"),
        status: Status::NoGain,
        saves_mib: 0,
        note: "with NetworkServiceInProcess2: one process fewer, the sum is the same (537 MiB)",
    },
    Row {
        id: "electron-renderer-limit",
        class: Class::Electron,
        knob: Knob::Arg("--renderer-process-limit=1"),
        status: Status::NoGain,
        saves_mib: -10,
        note: "with --disable-site-isolation-trials and one raster thread: 594 MiB while in front (shmem +40 MiB), 544 behind",
    },
    Row {
        id: "electron-malloc-env",
        class: Class::Electron,
        knob: Knob::Env("MALLOC_ARENA_MAX", "1"),
        status: Status::NoGain,
        saves_mib: 0,
        note: "with MALLOC_TRIM_THRESHOLD_=65536: 537 MiB; the heap is PartitionAlloc, glibc holds 8 MiB",
    },
    // WebKitGTK / JavaScriptCore (MiniBrowser 2.54.2 on the same login page: 762 MiB).
    Row {
        id: "webkitgtk-ksm",
        class: Class::WebKitGtk,
        knob: Knob::KsmMerge,
        status: Status::Shipped,
        saves_mib: 118,
        note: "anon 577 -> 459 MiB (bmalloc and glibc pages full of zeros and repeated objects)",
    },
    Row {
        id: "webkitgtk-no-jit",
        class: Class::WebKitGtk,
        knob: Knob::Env("JSC_useJIT", "false"),
        status: Status::LeanOnly,
        saves_mib: 92,
        note: "762 -> 670 MiB; the interpreter only: scripts several times slower",
    },
    Row {
        id: "webkitgtk-one-gc-thread",
        class: Class::WebKitGtk,
        knob: Knob::Env("JSC_useConcurrentGC", "false"),
        status: Status::NoGain,
        saves_mib: -4,
        note: "with one marker and one compiler thread: 766 MiB against 762",
    },
    // Bun (1.4.4, vite build of site/: peak RSS, median of 5).
    Row {
        id: "bun-arena-lazy-commit",
        class: Class::Bun,
        knob: Knob::Env("MIMALLOC_ARENA_EAGER_COMMIT", "0"),
        status: Status::Shipped,
        saves_mib: 491,
        note: "peak RSS 859 -> 368 MiB, wall time 0.91 -> 0.82 s: the allocator no longer touches the arena it reserves",
    },
    Row {
        id: "bun-purge-now",
        class: Class::Bun,
        knob: Knob::Env("MIMALLOC_PURGE_DELAY", "0"),
        status: Status::Shipped,
        saves_mib: 47,
        note: "on top of the lazy commit: 368 -> 321 MiB peak, wall time within the spread; alone 888 -> 705 MiB",
    },
    Row {
        id: "bun-smol",
        class: Class::Bun,
        knob: Knob::Arg("--smol"),
        status: Status::NoGain,
        saves_mib: 19,
        note: "888 -> 869 MiB peak (2%): less than the spread; with the allocator rows above, 321 -> 321",
    },
    Row {
        id: "bun-no-jit",
        class: Class::Bun,
        knob: Knob::Env("BUN_JSC_useJIT", "0"),
        status: Status::LeanOnly,
        saves_mib: 82,
        note: "888 -> 806 MiB peak for 0.75 -> 0.94 s (+25%): a tool held to a budget, never a default",
    },
];

/// What an app of a class is started with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Runtime {
    /// Words appended to the app's command line (one word each; `--js-flags=` is one word).
    pub args: Vec<String>,
    /// Variables set for the app.
    pub env: Vec<(String, String)>,
    /// `prctl(PR_SET_MEMORY_MERGE, 1)` before the exec (KSM scans the process tree; the flag is
    /// inherited by every child).
    pub ksm_merge: bool,
}

impl Runtime {
    /// The program words to put in front of the app's whole command (also in front of
    /// `systemd-run`: the setting is inherited through every exec): `wolos-mm exec-merge --` when
    /// KSM scanning is asked, nothing otherwise.
    pub fn prefix(&self) -> Vec<String> {
        if self.ksm_merge {
            EXEC_MERGE.iter().map(|s| (*s).to_owned()).collect()
        } else {
            Vec::new()
        }
    }
}

/// `wolos-mm exec-merge --`: sets `PR_SET_MEMORY_MERGE` and execs the rest of the line.
pub const EXEC_MERGE: [&str; 3] = ["/usr/bin/wolos-mm", "exec-merge", "--"];

/// The measured defaults of a class, plus the speed-for-memory rows in [`Mode::Lean`].
pub fn runtime_flags(class: Class, mode: Mode) -> Runtime {
    let mut rt = Runtime::default();
    let mut js: Vec<&str> = Vec::new();
    for row in ROWS.iter().filter(|r| r.applies(class, mode)) {
        match row.knob {
            Knob::Arg(a) => rt.args.push(a.to_owned()),
            Knob::JsFlag(f) => js.push(f),
            Knob::Env(k, v) => rt.env.push((k.to_owned(), v.to_owned())),
            Knob::KsmMerge => rt.ksm_merge = true,
        }
    }
    if !js.is_empty() {
        // The engine keeps one value of a switch: V8 flags travel as one word.
        rt.args.push(format!("--js-flags={}", js.join(" ")));
    }
    rt
}

// ---- System layer ------------------------------------------------------------------------

/// One kernel knob `wolos-mm apply` writes at boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MmWrite {
    pub path: &'static str,
    pub value: &'static str,
    pub why: &'static str,
}

/// Page-fault granularity of file mappings (debugfs; a kernel without it keeps the default of
/// 65536). Every mapped file page a process touches pulls its neighbours into the page table in
/// the same fault; each of those counts in the app's PSS whether it is used or not. Measured on
/// the Discord binary (218 MB): file-backed PSS 215 MiB at 65536, 164 at 16384, 144 at 8192, 126 at
/// 4096; startup cost 8% more CPU and 45% more minor faults, window and frame times unchanged.
pub const FAULT_AROUND_BYTES: u32 = 4096;

/// KSM pages scanned per wake-up and the sleep between (ms): 5000 pages a second is 2% of a core
/// at first and falls as pages are marked unmergeable. The kernel's scan-time advisor at 30 s
/// target burned 35% of a core for the same merges, so it stays off.
pub const KSM_PAGES_TO_SCAN: u32 = 500;
/// Sleep between two KSM scans, ms (see [`KSM_PAGES_TO_SCAN`]).
pub const KSM_SLEEP_MS: u32 = 100;

/// The knobs written whatever the machine is, in order.
pub const MM_WRITES: &[MmWrite] = &[
    MmWrite {
        path: "/sys/kernel/debug/fault_around_bytes",
        value: "4096",
        why: "file-backed PSS of a big Electron binary 215 -> 126 MiB; the pages a process does not touch are not mapped",
    },
    MmWrite {
        path: "/sys/kernel/mm/ksm/pages_to_scan",
        value: "500",
        why: "5000 pages/s: merges the same pages as 100000/s in two minutes, at 2% of a core instead of 26%",
    },
    MmWrite {
        path: "/sys/kernel/mm/ksm/sleep_millisecs",
        value: "100",
        why: "ten wake-ups a second while an opted-in app runs",
    },
];

/// The switch of the KSM scanner, written after [`MM_WRITES`] and only by [`ksm_policy`].
pub const KSM_RUN: &str = "/sys/kernel/mm/ksm/run";

/// Whether KSM may run on this machine. KSM merges identical pages of every process that opted in,
/// system-wide and across users, and a write to a merged page is slower than to a private one: a
/// process of one person can learn whether another person's process holds a page with a given
/// content (the memory-deduplication side channel). So it runs only where there is nobody to
/// learn it from: one human account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KsmPolicy {
    /// One human account: `run=1`.
    On,
    /// Several accounts (or none known): `run=0`, and `exec-merge` asks the kernel for nothing.
    Off { accounts: usize },
}

impl KsmPolicy {
    /// The value for [`KSM_RUN`].
    pub fn run_value(self) -> &'static str {
        match self {
            KsmPolicy::On => "1",
            KsmPolicy::Off { .. } => "0",
        }
    }

    /// The line for the journal.
    pub fn log_line(self) -> String {
        match self {
            KsmPolicy::On => "KSM on: one human account".to_owned(),
            KsmPolicy::Off { accounts } => format!(
                "KSM off: {accounts} human accounts (merging pages across accounts is a timing side channel)"
            ),
        }
    }
}

/// The policy for a machine, from its passwd text (the list Settings > Accounts and the Setup
/// Assistant keep: uid 1000 and up with a login shell, `settings::users::parse`).
pub fn ksm_policy(passwd: &str) -> KsmPolicy {
    match crate::settings::users::parse(passwd).len() {
        1 => KsmPolicy::On,
        accounts => KsmPolicy::Off { accounts },
    }
}

// ---- Page-out --------------------------------------------------------------------------

/// What `process_madvise(MADV_PAGEOUT)` is asked about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pageout {
    /// Heaps, stacks and named anonymous memory that are writable, shared memory (memfd, /dev/shm),
    /// and writable private file mappings (data, relocated tables): goes to swap. What Memory
    /// Saver asks.
    Anon,
    /// Also the read-only mappings and the code of the executable and its libraries: dropped and
    /// read again from the page cache or the disk on touch. What `wolos-mm pageout PID all` asks.
    All,
}

/// THE function that says which ranges of `/proc/<pid>/maps` are paged out (Memory Saver's
/// `crates/api/src/memsaver/pageout.rs` and `wolos-mm pageout` both call it). One `(start, length)`
/// per mapping, to hand to `process_madvise` one call per range: a call with several ranges stops
/// silently at the first one the kernel refuses (measured: 18 GiB asked, 11 MiB paged out). The
/// kernel moves only pages that this process alone maps. Special mappings (vvar, vdso, vsyscall)
/// and shared file mappings are never asked.
pub fn pageout_ranges(maps: &str, what: Pageout) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for line in maps.lines() {
        let mut f = line.split_ascii_whitespace();
        let (Some(range), Some(perms), Some(_off), Some(_dev), Some(inode)) =
            (f.next(), f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        let path = f.collect::<Vec<_>>().join(" ");
        let Some((a, b)) = range.split_once('-') else {
            continue;
        };
        let (Ok(a), Ok(b)) = (usize::from_str_radix(a, 16), usize::from_str_radix(b, 16)) else {
            continue;
        };
        let p = perms.as_bytes();
        if p.len() < 4 || b <= a {
            continue;
        }
        if path.starts_with("[vvar") || path.starts_with("[vsyscall") || path == "[vdso]" {
            continue;
        }
        let (writable, private) = (p[1] == b'w', p[3] == b'p');
        let shm = path.starts_with("/memfd:") || path.starts_with("/dev/shm/");
        let file = path.starts_with('/') && !shm;
        let anon = inode == "0"
            && (path.is_empty()
                || path == "[heap]"
                || path.starts_with("[stack")
                || path.starts_with("[anon:"));
        let wanted = match what {
            Pageout::Anon => shm || (private && writable && (anon || file)),
            Pageout::All => shm || (private && (anon || file)),
        };
        if wanted {
            out.push((a, b - a));
        }
    }
    out
}

/// Why `wolos-mm pageout` refuses a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The process is not in an `app-*.scope`: it is a service, the session or the system.
    NotAppScope,
    /// The process belongs to another user.
    OtherUser,
    /// `--any` is for root.
    AnyNeedsRoot,
}

/// Whether a cgroup file (`/proc/<pid>/cgroup`, v2: one `0::/path` line) puts the process in an
/// app scope: a path component `app-<something>.scope`.
pub fn in_app_scope(cgroup: &str) -> bool {
    cgroup.lines().any(|l| {
        l.rsplit_once("::").is_some_and(|(_, path)| {
            path.split('/').any(|c| {
                c.starts_with("app-") && c.ends_with(".scope") && c.len() > "app-.scope".len()
            })
        })
    })
}

/// The rule for a page-out of another process: the same user's and inside an app scope, unless the
/// caller is root and says `--any`.
pub fn pageout_allowed(
    cgroup: &str,
    target_uid: u32,
    caller_uid: u32,
    any: bool,
) -> Result<(), Refusal> {
    if any {
        return if caller_uid == 0 {
            Ok(())
        } else {
            Err(Refusal::AnyNeedsRoot)
        };
    }
    if target_uid != caller_uid {
        return Err(Refusal::OtherUser);
    }
    if !in_app_scope(cgroup) {
        return Err(Refusal::NotAppScope);
    }
    Ok(())
}

/// Whether a pidfd still names `pid`, from its `/proc/self/fdinfo/<fd>` text: the `Pid:` line
/// says `-1` once the process is gone (and the line is missing on a kernel without pidfds). Both
/// page-out callers ask it before every `process_madvise` call, so a process that exits halfway
/// is left at once.
pub fn pidfd_names(fdinfo: &str, pid: u32) -> bool {
    fdinfo
        .lines()
        .find_map(|l| l.strip_prefix("Pid:"))
        .and_then(|v| v.trim().parse::<i64>().ok())
        == Some(i64::from(pid))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(c: Class, m: Mode) -> Runtime {
        runtime_flags(c, m)
    }

    #[test]
    fn a_pidfd_names_its_process_until_it_is_gone() {
        let live = "pos:\t0\nflags:\t02000002\nmnt_id:\t15\nino:\t1057\nPid:\t4242\nNSpid:\t4242\n";
        assert!(pidfd_names(live, 4242));
        assert!(!pidfd_names(live, 4243), "another pid");
        assert!(!pidfd_names("Pid:\t-1\nNSpid:\t-1\n", 4242), "exited");
        assert!(!pidfd_names("pos:\t0\n", 4242), "no Pid line: not a pidfd");
    }

    #[test]
    fn electron_default_asks_for_ksm_and_no_switch() {
        let r = rt(Class::Electron, Mode::Default);
        assert!(r.ksm_merge);
        assert!(r.args.is_empty(), "{:?}", r.args);
        assert!(r.env.is_empty());
    }

    #[test]
    fn ksm_is_a_prefix_not_a_flag() {
        assert_eq!(
            rt(Class::Electron, Mode::Default).prefix(),
            ["/usr/bin/wolos-mm", "exec-merge", "--"]
        );
        assert!(rt(Class::Bun, Mode::Default).prefix().is_empty());
    }

    #[test]
    fn lean_electron_adds_one_js_flags_word() {
        let r = rt(Class::Electron, Mode::Lean);
        assert_eq!(r.args, ["--js-flags=--lite-mode"]);
        assert!(r.ksm_merge);
    }

    #[test]
    fn bun_default_sets_the_allocator_and_nothing_else() {
        let r = rt(Class::Bun, Mode::Default);
        assert_eq!(
            r.env,
            [
                ("MIMALLOC_ARENA_EAGER_COMMIT".into(), "0".into()),
                ("MIMALLOC_PURGE_DELAY".into(), "0".into())
            ]
        );
        assert!(r.args.is_empty(), "--smol measured no gain");
        assert!(!r.ksm_merge);
    }

    #[test]
    fn bun_lean_adds_the_interpreter_only_to_the_allocator_rows() {
        let l = rt(Class::Bun, Mode::Lean);
        assert_eq!(l.env.len(), 3);
        assert_eq!(l.env[2], ("BUN_JSC_useJIT".to_owned(), "0".to_owned()));
    }

    #[test]
    fn webkitgtk_no_jit_only_in_lean() {
        let d = rt(Class::WebKitGtk, Mode::Default);
        assert!(d.env.is_empty() && d.ksm_merge);
        let l = rt(Class::WebKitGtk, Mode::Lean);
        assert_eq!(l.env, [("JSC_useJIT".to_owned(), "false".to_owned())]);
    }

    #[test]
    fn a_no_gain_row_never_reaches_a_launch() {
        for c in Class::ALL {
            for m in [Mode::Default, Mode::Lean] {
                let r = rt(c, m);
                for row in ROWS.iter().filter(|r| r.status == Status::NoGain) {
                    match row.knob {
                        Knob::Arg(a) => assert!(!r.args.iter().any(|x| x == a), "{}", row.id),
                        Knob::Env(k, _) => {
                            assert!(!r.env.iter().any(|(x, _)| x == k), "{}", row.id)
                        }
                        Knob::JsFlag(f) => {
                            assert!(!r.args.iter().any(|x| x.contains(f)), "{}", row.id)
                        }
                        Knob::KsmMerge => unreachable!("ksm rows ship"),
                    }
                }
            }
        }
    }

    #[test]
    fn rows_are_unique_honest_and_one_word() {
        let ids: std::collections::BTreeSet<_> = ROWS.iter().map(|r| r.id).collect();
        assert_eq!(ids.len(), ROWS.len(), "ids are unique");
        for r in ROWS {
            assert!(r.id.starts_with(r.class.id()), "{}", r.id);
            assert!(!r.note.is_empty(), "{}: a row says what it measured", r.id);
            match r.status {
                Status::Shipped | Status::LeanOnly => {
                    assert!(r.saves_mib > 0, "{}: shipped rows saved memory", r.id)
                }
                // The spread is 5 MiB; a "no gain" row with a big gain would be a shipped one.
                Status::NoGain => assert!(r.saves_mib <= 20, "{}", r.id),
            }
            match r.knob {
                Knob::Arg(a) => assert!(a.starts_with("--") && !a.contains(' '), "{a}"),
                Knob::JsFlag(f) => assert!(f.starts_with("--") && !f.contains(' '), "{f}"),
                Knob::Env(k, v) => {
                    assert!(!k.is_empty() && !k.contains('=') && !v.contains(' '), "{k}")
                }
                Knob::KsmMerge => {}
            }
        }
    }

    #[test]
    fn exec_names_map_to_classes() {
        assert_eq!(Class::of_exec("/usr/bin/bun"), Some(Class::Bun));
        assert_eq!(
            Class::of_exec("/usr/lib/webkitgtk-6.0/MiniBrowser"),
            Some(Class::WebKitGtk)
        );
        assert_eq!(Class::of_exec("discord"), None);
    }

    #[test]
    fn the_boot_table_and_its_constants_agree() {
        let get = |p: &str| {
            MM_WRITES
                .iter()
                .find(|w| w.path.ends_with(p))
                .unwrap()
                .value
        };
        assert_eq!(get("fault_around_bytes"), FAULT_AROUND_BYTES.to_string());
        assert_eq!(get("pages_to_scan"), KSM_PAGES_TO_SCAN.to_string());
        assert_eq!(get("sleep_millisecs"), KSM_SLEEP_MS.to_string());
        // `run` is not in the table: only the account rule decides it.
        assert!(MM_WRITES.iter().all(|w| w.path != KSM_RUN));
    }

    fn passwd(humans: &[&str]) -> String {
        let mut s = String::from(
            "root:x:0:0::/root:/bin/bash\nnobody:x:65534:65534::/:/usr/bin/nologin\n\
             _wollogin:x:970:970::/:/usr/bin/nologin\n",
        );
        for (i, h) in humans.iter().enumerate() {
            s.push_str(&format!(
                "{h}:x:{}:{}:{h}:/home/{h}:/usr/bin/zsh\n",
                1000 + i,
                1000 + i
            ));
        }
        s
    }

    #[test]
    fn ksm_runs_for_exactly_one_human_account() {
        assert_eq!(ksm_policy(&passwd(&["alice"])), KsmPolicy::On);
        assert_eq!(KsmPolicy::On.run_value(), "1");
        let two = ksm_policy(&passwd(&["alice", "bob"]));
        assert_eq!(two, KsmPolicy::Off { accounts: 2 });
        assert_eq!(two.run_value(), "0");
        assert!(
            two.log_line().contains("2 human accounts"),
            "{}",
            two.log_line()
        );
        // No account yet, or a service user with a shell only: nobody to run it for, nothing to start it.
        assert_eq!(ksm_policy(&passwd(&[])), KsmPolicy::Off { accounts: 0 });
        assert_eq!(ksm_policy(""), KsmPolicy::Off { accounts: 0 });
    }

    /// One row per mapping: the line of `/proc/<pid>/maps`, whether the default page-out
    /// (Memory Saver's) asks about it, whether `all` does. The only table for the filter.
    const MAPS: &[(&str, bool, bool)] = &[
        (
            "55d0a0000000-55d0a0100000 r-xp 00000000 08:01 1234 /opt/code/code",
            false,
            true,
        ),
        (
            "55d0a0100000-55d0a0200000 r--p 00100000 08:01 1234 /opt/code/code",
            false,
            true,
        ),
        (
            "55d0a0200000-55d0a0300000 rw-p 00200000 08:01 1234 /opt/code/code",
            true,
            true,
        ),
        (
            "55d0a1000000-55d0a2000000 rw-p 00000000 00:00 0 [heap]",
            true,
            true,
        ),
        (
            "7f0000000000-7f0000400000 rw-p 00000000 00:00 0",
            true,
            true,
        ),
        (
            "7f0000400000-7f0000500000 r--p 00000000 00:00 0",
            false,
            true,
        ),
        (
            "7f0000500000-7f0000501000 ---p 00000000 00:00 0",
            false,
            true,
        ),
        (
            "7f1000000000-7f1000200000 rw-s 00000000 00:01 77 /memfd:shared_mem (deleted)",
            true,
            true,
        ),
        (
            "7f1100000000-7f1100001000 rw-s 00000000 00:01 78 /dev/shm/x",
            true,
            true,
        ),
        (
            "7f2000000000-7f2000001000 rw-p 00000000 00:00 0 [anon:v8]",
            true,
            true,
        ),
        (
            "7f2100000000-7f2100001000 rw-p 00000000 00:00 0 [anon:partition_alloc]",
            true,
            true,
        ),
        (
            "7ffc00000000-7ffc00021000 rw-p 00000000 00:00 0 [stack]",
            true,
            true,
        ),
        (
            "7f3000000000-7f3000100000 r--s 00000000 08:01 55 /usr/share/fonts/x.ttf",
            false,
            false,
        ),
        (
            "7f3100000000-7f3100100000 rw-s 00000000 08:01 56 /var/cache/x.db",
            false,
            false,
        ),
        (
            "7ffc10000000-7ffc10002000 r--p 00000000 00:00 0 [vvar]",
            false,
            false,
        ),
        (
            "7ffc10002000-7ffc10004000 r-xp 00000000 00:00 0 [vdso]",
            false,
            false,
        ),
        (
            "ffffffffff600000-ffffffffff601000 --xp 00000000 00:00 0 [vsyscall]",
            false,
            false,
        ),
        ("not a maps line", false, false),
        ("1000-0fff rw-p 00000000 00:00 0", false, false),
    ];

    fn start_of(line: &str) -> Option<usize> {
        usize::from_str_radix(line.split('-').next()?, 16).ok()
    }

    #[test]
    fn pageout_ranges_follow_the_one_table() {
        let text: String = MAPS.iter().map(|(l, ..)| format!("{l}\n")).collect();
        let default = pageout_ranges(&text, Pageout::Anon);
        let all = pageout_ranges(&text, Pageout::All);
        for (line, in_default, in_all) in MAPS {
            let Some(start) = start_of(line) else {
                continue;
            };
            assert_eq!(
                default.iter().any(|(a, _)| *a == start),
                *in_default,
                "default: {line}"
            );
            assert_eq!(all.iter().any(|(a, _)| *a == start), *in_all, "all: {line}");
        }
        assert_eq!(default.len(), MAPS.iter().filter(|r| r.1).count());
        assert_eq!(all.len(), MAPS.iter().filter(|r| r.2).count());
        assert_eq!(default[0], (0x55d0a0200000, 0x100000), "start and length");
    }

    #[test]
    fn app_scopes_are_found_by_path_component() {
        assert!(in_app_scope(
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-discord-4242.scope\n"
        ));
        assert!(in_app_scope("0::/user.slice/app-wol-discord-1.scope/sub\n"));
        assert!(!in_app_scope(
            "0::/user.slice/user-1000.slice/session-3.scope\n"
        ));
        assert!(!in_app_scope("0::/system.slice/wold.service\n"));
        assert!(!in_app_scope("0::/app-.scope\n"), "a name is needed");
        assert!(
            !in_app_scope("0::/x/myapp-discord.scope\n"),
            "app- is a prefix of the component"
        );
        assert!(!in_app_scope(""));
    }

    #[test]
    fn a_page_out_of_another_process_needs_the_same_user_and_an_app_scope() {
        let app = "0::/user.slice/app.slice/app-x-1.scope\n";
        let svc = "0::/system.slice/sshd.service\n";
        assert_eq!(pageout_allowed(app, 1000, 1000, false), Ok(()));
        assert_eq!(
            pageout_allowed(svc, 1000, 1000, false),
            Err(Refusal::NotAppScope)
        );
        assert_eq!(
            pageout_allowed(app, 1001, 1000, false),
            Err(Refusal::OtherUser)
        );
        // Root is no exception without --any, and --any is root's alone.
        assert_eq!(
            pageout_allowed(app, 1000, 0, false),
            Err(Refusal::OtherUser)
        );
        assert_eq!(pageout_allowed(svc, 1000, 0, true), Ok(()));
        assert_eq!(
            pageout_allowed(app, 1000, 1000, true),
            Err(Refusal::AnyNeedsRoot)
        );
    }
}
