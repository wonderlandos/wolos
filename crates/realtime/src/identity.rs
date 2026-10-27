//! Which program a client is (ADR 0013): peer pid -> executable and arguments -> policy identity.

use crate::policy::{Identity, identity_from};

/// Executable path and argv of a process, from the OS.
fn exe_and_args(pid: i32) -> Option<(String, Vec<String>)> {
    let mut buf = [0u8; 1024];
    let n = shared::sys::executable(pid, &mut buf)?;
    Some((
        String::from_utf8_lossy(&buf[..n]).into_owned(),
        shared::sys::args(pid)?,
    ))
}

/// Identity of the process behind `pid`; unknown processes get an empty identity (no privileges).
/// The identity is a canonical path (symlinks and `..` resolved), so path tricks cannot fake a system component.
pub fn of_pid(pid: i32) -> Identity {
    let Some((exe, args)) = exe_and_args(pid) else {
        return Identity {
            code: String::new(),
        };
    };
    Identity {
        code: canonical(&identity_from(&exe, &args).code),
    }
}

use shared::sys::canonical;

/// Canonical system libexec folder (it may be a symlink, e.g. /usr/local -> /work/local).
pub fn libexec() -> &'static str {
    static LIBEXEC: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    LIBEXEC.get_or_init(|| canonical(crate::policy::LIBEXEC))
}

/// Consent grants (ADR 0013) with canonical app paths. Re-read per connection so edits apply at once.
pub fn grants() -> Vec<(String, String)> {
    let json = std::fs::read_to_string("/usr/local/etc/wolos/consent.json").unwrap_or_default();
    crate::policy::Consent::parse(&json)
        .pairs()
        .into_iter()
        .map(|(app, cap)| (canonical(&app), cap))
        .collect()
}
