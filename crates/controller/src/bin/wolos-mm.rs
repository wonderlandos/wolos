// SPDX-License-Identifier: GPL-2.0-or-later
//! wolos-mm: the kernel half of the runtime tuning for JavaScript engines (ADR 0061,
//! docs/modules/runtime-tuning.md). The tables, the account rule for KSM and the page-out filter
//! are `wol_core::jsruntime`; this program holds the only two `unsafe` blocks, the calls std does
//! not wrap.
//!
//! ```text
//! wolos-mm apply [--passwd FILE] [--sysroot DIR]
//!                                 boot (wolos-mm.service): write the knobs, and KSM `run` only
//!                                 when the machine has one human account; logs which. The two
//!                                 options point at a fake passwd and /sys tree for tests
//! wolos-mm ksm                    status dump of KSM, printed once (no loop, no change)
//! wolos-mm exec-merge [--] /ABS/PATH [ARG...]
//!                                 ask KSM to scan this process (inherited by every child, exec
//!                                 included; skipped with a log line on a machine with several
//!                                 accounts), then exec the program. A bare name is refused
//! wolos-mm pageout [--any] PID [anon|all]
//!                                 process_madvise(MADV_PAGEOUT) over PID's mappings
//!                                 (CAP_SYS_NICE for another process). Refused unless PID is the
//!                                 caller's own user's and inside an app-*.scope; root may pass --any
//! ```
use std::io::{self, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, ExitCode};
use wol_core::jsruntime::{
    KSM_RUN, MM_WRITES, Pageout, Refusal, ksm_policy, pageout_allowed, pageout_ranges, pidfd_names,
};

/// `PR_SET_MEMORY_MERGE` (Linux 6.4; not in the libc crate).
const PR_SET_MEMORY_MERGE: libc::c_int = 67;

/// Ask KSM to scan every page of this process and of the processes it starts.
fn set_merge() -> io::Result<()> {
    // SAFETY: prctl with constant arguments only sets a flag of the calling process.
    let r = unsafe { libc::prctl(PR_SET_MEMORY_MERGE, 1, 0, 0, 0) };
    if r == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// What a page-out did: ranges the kernel accepted with their bytes, and ranges it refused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Report {
    ranges: u32,
    bytes: u64,
    refused: u32,
}

/// Who may be paged out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rule {
    /// The same user's, inside an app scope; root may add `any`.
    App { any: bool },
    /// No check: the tests page out their own memory.
    #[cfg(test)]
    Unchecked,
}

#[derive(Debug)]
enum Error {
    Io(io::Error),
    Refused(Refusal),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// Whether the pidfd still names `pid` (`jsruntime::pidfd_names` over its fdinfo).
fn still_target(fd: libc::c_int, pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")).is_ok_and(|s| pidfd_names(&s, pid))
}

/// `process_madvise(MADV_PAGEOUT)` over the mappings of `pid` (`pageout_ranges`), one call per
/// range (a call with several stops at the first one the kernel refuses). The pid is pinned with
/// `pidfd_open` first; its cgroup, owner and maps are read only after, and the pidfd is asked
/// again that it still names the process before every call. The call does not depend on
/// which cgroup the pages are charged to.
fn pageout(pid: u32, what: Pageout, rule: Rule) -> Result<Report, Error> {
    let pid_t =
        libc::pid_t::try_from(pid).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: pidfd_open takes integers and returns a new descriptor or -1; the descriptor is
    // owned here and closed below, on every path; each iovec lives across its call, names memory
    // of the target process (not of this one) and the kernel only reads the iovec itself.
    unsafe {
        let fd = libc::syscall(libc::SYS_pidfd_open, pid_t, 0u32);
        if fd < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let Ok(fd) = libc::c_int::try_from(fd) else {
            return Err(io::Error::from(io::ErrorKind::InvalidData).into());
        };
        let result = (|| -> Result<Report, Error> {
            match rule {
                Rule::App { any } => {
                    let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup"))?;
                    let target = std::fs::metadata(format!("/proc/{pid}"))?.uid();
                    let caller = std::fs::metadata("/proc/self")?.uid();
                    pageout_allowed(&cgroup, target, caller, any).map_err(Error::Refused)?;
                }
                #[cfg(test)]
                Rule::Unchecked => {}
            }
            let maps = std::fs::read_to_string(format!("/proc/{pid}/maps"))?;
            if !still_target(fd, pid) {
                return Err(io::Error::from(io::ErrorKind::NotFound).into());
            }
            let mut rep = Report::default();
            for (start, len) in pageout_ranges(&maps, what) {
                // Asked again before every call: a process that exits halfway is left at once.
                if !still_target(fd, pid) {
                    break;
                }
                let iov = libc::iovec {
                    iov_base: start as *mut libc::c_void,
                    iov_len: len,
                };
                let r = libc::syscall(
                    libc::SYS_process_madvise,
                    fd,
                    &iov as *const libc::iovec,
                    1usize,
                    libc::MADV_PAGEOUT,
                    0u32,
                );
                if r < 0 {
                    rep.refused += 1;
                } else {
                    rep.ranges += 1;
                    rep.bytes += u64::try_from(r).unwrap_or(0);
                }
            }
            Ok(rep)
        })();
        libc::close(fd);
        result
    }
}

/// Write one knob under `root` (`/` on a real machine); a file the kernel does not have is skipped.
fn write_knob(out: &mut impl Write, root: &Path, path: &str, value: &str) -> bool {
    let file = root.join(path.trim_start_matches('/'));
    if !file.exists() {
        let _ = writeln!(out, "skip  {path} (not in this kernel)");
        return true;
    }
    match std::fs::write(&file, value) {
        Ok(()) => {
            let _ = writeln!(out, "wrote {path} = {value}");
            true
        }
        Err(e) => {
            let _ = writeln!(out, "FAIL  {path} = {value}: {e}");
            false
        }
    }
}

/// Write the boot knobs, then KSM `run` as the account rule says; true when no write failed.
fn apply(out: &mut impl Write, passwd: &Path, root: &Path) -> bool {
    let mut ok = true;
    for w in MM_WRITES {
        ok &= write_knob(out, root, w.path, w.value);
    }
    // No passwd file is no account: KSM stays off.
    let policy = ksm_policy(&std::fs::read_to_string(passwd).unwrap_or_default());
    let _ = writeln!(out, "{}", policy.log_line());
    ok & write_knob(out, root, KSM_RUN, policy.run_value())
}

/// KSM state, once.
fn ksm(out: &mut impl Write) {
    for f in [
        "run",
        "pages_to_scan",
        "sleep_millisecs",
        "pages_shared",
        "pages_sharing",
        "pages_unshared",
        "full_scans",
    ] {
        let v = std::fs::read_to_string(format!("/sys/kernel/mm/ksm/{f}"))
            .map_or_else(|_| "n/a".to_owned(), |s| s.trim().to_owned());
        let _ = writeln!(out, "{f} {v}");
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: wolos-mm apply [--passwd FILE] [--sysroot DIR]\n\
         \x20      wolos-mm ksm                      (status dump, printed once)\n\
         \x20      wolos-mm exec-merge [--] /ABS/PATH [ARG...]\n\
         \x20      wolos-mm pageout [--any] PID [anon|all]"
    );
    ExitCode::from(2)
}

/// The value of `--name VALUE` in `args`, removed from it.
fn take_option(args: &mut Vec<String>, name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.remove(i);
    (i < args.len()).then(|| args.remove(i))
}

fn main() -> ExitCode {
    let mut a: Vec<String> = std::env::args().skip(1).collect();
    let stdout = &mut io::stdout();
    match a.first().map(String::as_str) {
        Some("apply") => {
            a.remove(0);
            let passwd = take_option(&mut a, "--passwd").unwrap_or_else(|| "/etc/passwd".into());
            let root = take_option(&mut a, "--sysroot").unwrap_or_else(|| "/".into());
            if !a.is_empty() {
                return usage();
            }
            ExitCode::from(u8::from(!apply(
                stdout,
                Path::new(&passwd),
                Path::new(&root),
            )))
        }
        Some("ksm") => {
            ksm(stdout);
            ExitCode::SUCCESS
        }
        Some("exec-merge") => {
            let rest = &a[1..];
            let rest = rest.strip_prefix(&["--".to_owned()]).unwrap_or(rest);
            let Some((cmd, args)) = rest.split_first() else {
                return usage();
            };
            // The path is what runs: a bare name would be looked up in the caller's PATH.
            if !cmd.starts_with('/') {
                eprintln!("wolos-mm: exec-merge needs an absolute path, not `{cmd}`");
                return ExitCode::from(2);
            }
            let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
            let policy = ksm_policy(&passwd);
            if policy.run_value() == "1" {
                // Best effort: a kernel before 6.4 or without KSM runs the app as it was.
                if let Err(e) = set_merge() {
                    eprintln!("wolos-mm: KSM not requested: {e}");
                }
            } else {
                eprintln!("wolos-mm: {}", policy.log_line());
            }
            let e = Command::new(cmd).args(args).exec();
            eprintln!("wolos-mm: {cmd}: {e}");
            ExitCode::from(127)
        }
        Some("pageout") => {
            a.remove(0);
            let any = a
                .iter()
                .position(|x| x == "--any")
                .map(|i| a.remove(i))
                .is_some();
            let (Some(Ok(pid)), what) = (
                a.first().map(|p| p.parse::<u32>()),
                a.get(1).map(String::as_str),
            ) else {
                return usage();
            };
            let what = match what {
                None | Some("anon") => Pageout::Anon,
                Some("all") => Pageout::All,
                Some(_) => return usage(),
            };
            match pageout(pid, what, Rule::App { any }) {
                Ok(r) => {
                    println!(
                        "ranges {} refused {} MiB {}",
                        r.ranges,
                        r.refused,
                        r.bytes >> 20
                    );
                    ExitCode::SUCCESS
                }
                Err(Error::Refused(why)) => {
                    eprintln!("wolos-mm: pageout {pid} refused: {why:?}");
                    ExitCode::from(3)
                }
                Err(Error::Io(e)) => {
                    eprintln!("wolos-mm: pageout {pid}: {e}");
                    ExitCode::from(1)
                }
            }
        }
        _ => usage(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_out_of_ones_own_private_memory_is_accepted() {
        // 32 MiB touched, then asked about; no capability is needed for the caller's own pid.
        let buf = vec![7u8; 32 << 20];
        let r =
            pageout(std::process::id(), Pageout::Anon, Rule::Unchecked).expect("own maps readable");
        assert!(r.ranges > 0, "{r:?}");
        assert!(r.bytes >= 32 << 20, "{r:?}");
        assert_eq!(buf[buf.len() - 1], 7, "the page comes back on touch");
    }

    #[test]
    fn a_page_out_of_a_pid_that_is_gone_is_an_error() {
        assert!(pageout(u32::MAX - 1, Pageout::Anon, Rule::Unchecked).is_err());
    }

    #[test]
    fn a_target_outside_an_app_scope_is_refused_before_any_range_is_read() {
        // The test process runs in a session or service cgroup, never in an app-*.scope.
        let own = std::fs::read_to_string("/proc/self/cgroup").unwrap();
        if wol_core::jsruntime::in_app_scope(&own) {
            return;
        }
        let r = pageout(std::process::id(), Pageout::Anon, Rule::App { any: false });
        assert!(
            matches!(r, Err(Error::Refused(Refusal::NotAppScope))),
            "{r:?}"
        );
    }

    #[test]
    fn any_is_root_only() {
        if std::fs::metadata("/proc/self").unwrap().uid() == 0 {
            return;
        }
        let r = pageout(std::process::id(), Pageout::Anon, Rule::App { any: true });
        assert!(
            matches!(r, Err(Error::Refused(Refusal::AnyNeedsRoot))),
            "{r:?}"
        );
    }

    #[test]
    fn options_are_taken_out_of_the_arguments() {
        let mut a: Vec<String> = ["--passwd", "p", "--sysroot", "r"].map(String::from).into();
        assert_eq!(take_option(&mut a, "--passwd").as_deref(), Some("p"));
        assert_eq!(take_option(&mut a, "--sysroot").as_deref(), Some("r"));
        assert!(a.is_empty());
        let mut b = vec!["--passwd".to_owned()];
        assert_eq!(take_option(&mut b, "--passwd"), None, "no value");
    }
}
