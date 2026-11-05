// SPDX-License-Identifier: BSD-2-Clause
//! Apps as the session runs them: one cgroup v2 `app-*.scope` per launched app (systemd's
//! launcher scopes under `user@UID.service/app.slice`, and the `app-wol-<id>-<pid>.scope` units
//! Memory Saver moves apps into, ADR 0026). Per scope: memory (`memory.current`, and PSS, the
//! share of pages the app really costs, from each process's `smaps_rollup`), CPU
//! (`cpu.stat usage_usec`), whether it is frozen (`cgroup.events`), and what Memory Saver says
//! it is doing (`shared::memsaver::STATUS`).
use super::Root;
use shared::memsaver::{AppState, Status};

pub const USER_SLICE: &str = "/sys/fs/cgroup/user.slice";

/// Deepest a scope sits below the user slice (`user-1000.slice/user@1000.service/app.slice/
/// app-….scope` is 4); the walk stops there so a deep tree costs nothing.
const MAX_DEPTH: usize = 6;

/// Launchers that put their own name before the app's in the scope name.
const LAUNCHERS: [&str; 11] = [
    "wol", "glib", "gnome", "kde", "flatpak", "snap", "Hyprland", "hyprland", "uwsm", "niri",
    "sway",
];

/// What one scope's files say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    /// `app-gnome-org.gnome.TextEditor-4242.scope`.
    pub unit: String,
    /// Path of the cgroup, from the cgroup root (`/sys/fs/cgroup/...`).
    pub cgroup: String,
    /// [`app_of`] the unit.
    pub app: String,
    pub pids: Vec<i32>,
    /// `memory.current`, bytes.
    pub memory: u64,
    /// `memory.swap.current`: compressed away by Memory Saver or the kernel, bytes.
    pub swap: u64,
    /// `cpu.stat usage_usec`, cumulative.
    pub cpu_usec: u64,
    pub frozen: bool,
}

/// What the Apps page shows in its state column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Rest {
    Running,
    /// Memory Saver gave its memory back; it faults back in as the app is used.
    Resting,
    /// Paused (App Nap or a frozen cgroup): uses no CPU until it is shown again.
    Frozen,
}

/// Energy impact over the last minute, from CPU time (no per-app power counter exists for an
/// unprivileged reader; CPU time is what costs battery on a laptop).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum Energy {
    Low,
    Moderate,
    High,
}

/// One row of the Apps page at one instant.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct AppUse {
    pub unit: String,
    pub app: String,
    pub pids: Vec<i32>,
    /// The pids with their start times, for a Quit that must not hit a reused pid.
    pub targets: Vec<super::procs::Target>,
    pub memory: u64,
    pub swap: u64,
    /// None when not asked for this sample, or no process in it could be read.
    pub pss: Option<u64>,
    /// Percent of one core since the previous sample.
    pub cpu: f64,
    pub frozen: bool,
    pub rest: Rest,
}

/// `\x2d`-style escapes of systemd unit names, decoded.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("\\x") {
        out.push_str(&rest[..i]);
        let hex = rest.get(i + 2..i + 4);
        match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(b) if b.is_ascii() => {
                out.push(b as char);
                rest = &rest[i + 4..];
            }
            _ => {
                out.push_str("\\x");
                rest = &rest[i + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A launch id at the end of a scope name: the pid, or a random hex tag.
fn launch_tag(s: &str) -> bool {
    !s.is_empty()
        && (s.bytes().all(|c| c.is_ascii_digit())
            || (s.len() >= 8 && s.bytes().all(|c| c.is_ascii_hexdigit())))
}

/// The app id inside a scope name: `app-[<launcher>-]<id>[-<pid or tag>][@<tag>].scope`.
pub fn app_of(unit: &str) -> Option<String> {
    let mid = unit.strip_prefix("app-")?.strip_suffix(".scope")?;
    let mid = unescape(mid.split('@').next().unwrap_or(mid));
    let mut s = mid.as_str();
    if let Some((head, tail)) = s.rsplit_once('-')
        && launch_tag(tail)
    {
        s = head;
    }
    if let Some((head, tail)) = s.split_once('-')
        && LAUNCHERS.contains(&head)
        && !tail.is_empty()
    {
        s = tail;
    }
    (!s.is_empty()).then(|| s.to_owned())
}

/// `frozen 1` in `cgroup.events`.
pub fn parse_frozen(events: &str) -> bool {
    events.lines().any(|l| l.trim() == "frozen 1")
}

/// `usage_usec` of `cpu.stat`.
pub fn parse_usage_usec(cpu_stat: &str) -> Option<u64> {
    cpu_stat
        .lines()
        .find_map(|l| l.strip_prefix("usage_usec ")?.trim().parse().ok())
}

/// `Pss:` of `smaps_rollup`, bytes.
pub fn parse_pss(smaps_rollup: &str) -> Option<u64> {
    smaps_rollup.lines().find_map(|l| {
        let kb = l.strip_prefix("Pss:")?.trim().strip_suffix("kB")?;
        kb.trim()
            .parse::<u64>()
            .ok()
            .map(|k| k.saturating_mul(1024))
    })
}

fn number(root: &Root, path: &str) -> u64 {
    root.read(path)
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0)
}

fn scope(root: &Root, cgroup: &str, unit: &str) -> Option<Scope> {
    let app = app_of(unit)?;
    let pids: Vec<i32> = root
        .read(&format!("{cgroup}/cgroup.procs"))?
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        // Never pid 0 or a negative one: `kill` would read it as a whole process group.
        .filter(|p: &i32| *p > 0)
        .collect();
    // An app that has just quit leaves an empty scope behind for a moment.
    if pids.is_empty() {
        return None;
    }
    Some(Scope {
        unit: unit.to_owned(),
        cgroup: cgroup.to_owned(),
        app,
        pids,
        memory: number(root, &format!("{cgroup}/memory.current")),
        swap: number(root, &format!("{cgroup}/memory.swap.current")),
        cpu_usec: root
            .read(&format!("{cgroup}/cpu.stat"))
            .as_deref()
            .and_then(parse_usage_usec)
            .unwrap_or(0),
        frozen: root
            .read(&format!("{cgroup}/cgroup.events"))
            .as_deref()
            .is_some_and(parse_frozen),
    })
}

fn walk(root: &Root, dir: &str, depth: usize, out: &mut Vec<Scope>) {
    for d in root.dirs(dir) {
        let path = format!("{dir}/{d}");
        if d.starts_with("app-") && d.ends_with(".scope") {
            out.extend(scope(root, &path, &d));
        } else if depth < MAX_DEPTH {
            walk(root, &path, depth + 1, out);
        }
    }
}

/// Every app scope with a process in it, of one user (`uid`) or all, in tree order.
pub fn scopes(root: &Root, uid: Option<u32>) -> Vec<Scope> {
    let start = match uid {
        Some(u) => format!("{USER_SLICE}/user-{u}.slice"),
        None => USER_SLICE.to_owned(),
    };
    let mut v = Vec::new();
    walk(root, &start, 0, &mut v);
    v
}

/// PSS of the processes, bytes; None when none could be read (another user's).
pub fn pss(root: &Root, pids: &[i32]) -> Option<u64> {
    pids.iter()
        .filter_map(|p| parse_pss(&root.read(&format!("/proc/{p}/smaps_rollup"))?))
        .reduce(u64::saturating_add)
}

/// What Memory Saver last wrote; None while it is not running.
pub fn saver(root: &Root) -> Option<Status> {
    serde_json::from_str(&root.read(shared::memsaver::STATUS)?).ok()
}

/// Memory Saver's word for the app (by normalised id), if it is tracking it.
pub fn saver_state(status: Option<&Status>, app: &str) -> Option<AppState> {
    let id = shared::memsaver::app_id(app);
    status?.apps.iter().find(|a| a.app == id).map(|a| a.state)
}

pub fn rest(state: Option<AppState>, frozen: bool) -> Rest {
    match state {
        _ if frozen => Rest::Frozen,
        Some(AppState::Napping) => Rest::Frozen,
        Some(AppState::Saving) => Rest::Resting,
        _ => Rest::Running,
    }
}

/// Mean CPU percent below 5 is low, below 30 moderate, else high.
pub fn energy(cpu: &[f64]) -> Energy {
    if cpu.is_empty() {
        return Energy::Low;
    }
    let mean = cpu.iter().sum::<f64>() / cpu.len() as f64;
    if mean < 5.0 {
        Energy::Low
    } else if mean < 30.0 {
        Energy::Moderate
    } else {
        Energy::High
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaped_and_tagged_names_decode() {
        assert_eq!(
            app_of("app-gnome-org.gnome.Console\\x2dDev-12.scope").as_deref(),
            Some("org.gnome.Console-Dev")
        );
        assert_eq!(
            app_of("app-uwsm-foot@0a1b2c3d.scope").as_deref(),
            Some("foot")
        );
        assert_eq!(
            app_of("app-blender-cafe.scope").as_deref(),
            Some("blender-cafe")
        );
        assert_eq!(
            app_of("app-wol-12.scope").as_deref(),
            Some("wol"),
            "only a tag left"
        );
        assert_eq!(app_of("app-x.service"), None);
    }

    #[test]
    fn memory_saver_words_map_to_rest() {
        assert_eq!(rest(None, false), Rest::Running);
        assert_eq!(rest(Some(AppState::Saving), false), Rest::Resting);
        assert_eq!(rest(Some(AppState::Napping), false), Rest::Frozen);
        assert_eq!(rest(Some(AppState::Active), true), Rest::Frozen);
        assert_eq!(parse_pss("Rss: 1 kB\nPss:   7 kB\n"), Some(7 * 1024));
        assert_eq!(parse_pss("Pss: x kB"), None);
    }
}
