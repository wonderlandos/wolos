// SPDX-License-Identifier: GPL-2.0-or-later
//! The background pass: rechecks what can bring malware in after new signatures arrive. Started by
//! a timer, only ever on AC power with everyone idle and no game; it stops the moment that changes
//! and picks up next time, because each finished file is remembered. Nothing stays in memory.
use shared::par::{Cancelled, for_range};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use wol_core::power::policy::AWAY_FLAG as IDLE_FLAG;

/// On mains power: a mains supply reports online, or the machine has no battery at all.
pub fn on_ac(power_supply: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(power_supply) else {
        return true;
    };
    let (mut battery, mut online) = (false, false);
    for e in rd.flatten() {
        let read = |n: &str| std::fs::read_to_string(e.path().join(n)).unwrap_or_default();
        match read("type").trim() {
            "Battery" => battery = true,
            "Mains" | "USB" => online |= read("online").trim() == "1",
            _ => {}
        }
    }
    online || !battery
}

/// Every open session is idle; with no session there is nobody to disturb.
pub fn everyone_idle(run_user: &Path) -> bool {
    std::fs::read_dir(run_user).map_or(true, |rd| {
        rd.flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| n.parse::<u32>().ok())
                    .is_some_and(|u| u >= 1000)
            })
            .filter(|e| e.path().join("hypr").exists())
            .all(|e| e.path().join(IDLE_FLAG).exists())
    })
}

fn is_game_name(comm: &str) -> bool {
    comm.starts_with("wine") || comm == "gamescope" || comm.to_lowercase().ends_with(".exe")
}

/// Any process of a game: a Wine program, gamescope, or something Steam started.
pub fn game_running(proc: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(proc) else {
        return false;
    };
    rd.flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
        })
        .any(|e| {
            let comm = std::fs::read_to_string(e.path().join("comm")).unwrap_or_default();
            is_game_name(comm.trim())
                || std::fs::read(e.path().join("environ")).is_ok_and(|env| {
                    env.split(|b| *b == 0)
                        .any(|kv| kv.starts_with(b"SteamAppId=") || kv.starts_with(b"SteamGameId="))
                })
        })
}

/// May background work run now?
pub fn gate(sys_power: &Path, run_user: &Path, proc: &Path) -> bool {
    on_ac(sys_power) && everyone_idle(run_user) && !game_running(proc)
}

/// Folder names never worth walking: build output, caches, and game libraries (signed by their
/// store, huge, and read during play).
const SKIP: [&str; 9] = [
    ".cache",
    ".git",
    "node_modules",
    "target",
    ".cargo",
    ".rustup",
    "steamapps",
    ".steam",
    ".var",
];

/// The kinds of file in scope.
pub fn in_scope(name: &str, executable: bool, tagged: bool) -> bool {
    let n = name.to_ascii_lowercase();
    executable
        || tagged
        || [".appimage", ".exe", ".msi", ".wol", ".sh", ".desktop"]
            .iter()
            .any(|e| n.ends_with(e))
}

/// Files in scope under `dir`, not following links.
pub fn targets(dir: &Path, tagged: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let (Ok(t), name) = (e.file_type(), e.file_name()) else {
            continue;
        };
        let n = name.to_string_lossy();
        let p = e.path();
        if t.is_dir() {
            if !SKIP.contains(&n.as_ref()) {
                targets(&p, tagged, out);
            }
        } else if t.is_file() {
            let exec = e
                .metadata()
                .is_ok_and(|m| m.permissions().mode() & 0o111 != 0);
            if in_scope(&n, exec, tagged(&p)) {
                out.push(p);
            }
        }
    }
}

/// This thread and its children run at the Background class: only spare CPU and disk time.
pub fn lowest_qos() -> bool {
    use shared::qos::{QosClass, SchedPolicy};
    debug_assert!(matches!(
        QosClass::Background.policy(),
        SchedPolicy::IdPrio(_)
    ));
    let p = libc::sched_param { sched_priority: 0 };
    // SAFETY: plain syscalls; 3 << 13 is the idle I/O class, who 1 is this process's thread.
    unsafe {
        libc::sched_setscheduler(0, libc::SCHED_IDLE, &p) == 0
            && libc::syscall(libc::SYS_ioprio_set, 1, 0, 3 << 13) == 0
    }
}

/// Fork/join over `files`, a few at a time. Leaves ask `open` first; the first no ends the pass.
pub fn pass(
    files: &[PathBuf],
    open: &(dyn Fn() -> bool + Sync),
    one: &(dyn Fn(&Path) + Sync),
) -> Result<(), Cancelled> {
    let cancel = AtomicBool::new(false);
    for_range(0..files.len(), 4, &cancel, |from, to| {
        for f in &files[from..to] {
            if cancel.load(Relaxed) {
                return;
            }
            if open() {
                one(f);
            } else {
                cancel.store(true, Relaxed);
            }
        }
    })?;
    if cancel.load(Relaxed) {
        Err(Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xguard-sweep-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn supply(d: &Path, name: &str, kind: &str, online: &str) {
        std::fs::create_dir_all(d.join(name)).unwrap();
        std::fs::write(d.join(name).join("type"), kind).unwrap();
        std::fs::write(d.join(name).join("online"), online).unwrap();
    }

    #[test]
    fn power_table() {
        let d = tmp("power");
        assert!(on_ac(&d), "a desktop has no battery");
        supply(&d, "BAT0", "Battery\n", "");
        assert!(!on_ac(&d), "a battery and no charger");
        supply(&d, "AC", "Mains\n", "0\n");
        assert!(!on_ac(&d));
        supply(&d, "AC", "Mains\n", "1\n");
        assert!(on_ac(&d));
    }

    #[test]
    fn idle_means_every_desktop_session() {
        let d = tmp("idle");
        assert!(everyone_idle(&d), "nobody logged in");
        std::fs::create_dir_all(d.join("1000/hypr")).unwrap();
        std::fs::create_dir_all(d.join("911")).unwrap();
        assert!(!everyone_idle(&d));
        std::fs::write(d.join("1000").join(IDLE_FLAG), "").unwrap();
        assert!(everyone_idle(&d));
        std::fs::create_dir_all(d.join("1001/hypr")).unwrap();
        assert!(!everyone_idle(&d), "someone else is at the computer");
    }

    #[test]
    fn a_game_closes_the_gate() {
        let d = tmp("game");
        assert!(!game_running(&d));
        std::fs::create_dir_all(d.join("100")).unwrap();
        std::fs::write(d.join("100/comm"), "bash\n").unwrap();
        std::fs::write(d.join("100/environ"), "HOME=/x\0").unwrap();
        assert!(!game_running(&d));
        std::fs::write(d.join("100/environ"), "HOME=/x\0SteamAppId=730\0").unwrap();
        assert!(game_running(&d));
        for c in ["wineserver", "gamescope", "Game.exe"] {
            assert!(is_game_name(c), "{c}");
        }
        assert!(!is_game_name("firefox"));
    }

    #[test]
    fn walking_finds_the_scope_and_skips_the_rest() {
        let d = tmp("walk");
        std::fs::create_dir_all(d.join("a/.cache")).unwrap();
        std::fs::create_dir_all(d.join("a/steamapps")).unwrap();
        for f in [
            "a/run.AppImage",
            "a/setup.exe",
            "a/notes.txt",
            "a/photo.jpg",
            "a/tagged.pdf",
            "a/.cache/x.exe",
            "a/steamapps/game.exe",
        ] {
            std::fs::write(d.join(f), "x").unwrap();
        }
        let tool = d.join("a/tool");
        std::fs::write(&tool, "x").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut out = Vec::new();
        targets(&d, &|p| p.ends_with("tagged.pdf"), &mut out);
        out.sort();
        let names: Vec<_> = out
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, ["run.AppImage", "setup.exe", "tagged.pdf", "tool"]);
    }

    #[test]
    fn every_file_is_visited_when_the_gate_stays_open() {
        let files: Vec<PathBuf> = (0..100).map(|i| format!("/f{i}").into()).collect();
        let seen = Mutex::new(Vec::new());
        assert!(
            pass(&files, &|| true, &|p| seen
                .lock()
                .unwrap()
                .push(p.to_owned()))
            .is_ok()
        );
        assert_eq!(seen.lock().unwrap().len(), 100);
    }

    #[test]
    fn a_closed_gate_stops_the_pass_at_once() {
        let files: Vec<PathBuf> = (0..10_000).map(|i| format!("/f{i}").into()).collect();
        let n = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        let r = pass(&files, &|| n.fetch_add(1, Relaxed) < 20, &|_| {
            done.fetch_add(1, Relaxed);
        });
        assert_eq!(r, Err(Cancelled));
        assert!(done.load(Relaxed) <= 20, "{}", done.load(Relaxed));
    }

    #[test]
    fn the_background_class_is_really_the_lowest() {
        std::thread::spawn(|| {
            assert!(lowest_qos());
            // SAFETY: plain syscall.
            assert_eq!(unsafe { libc::sched_getscheduler(0) }, libc::SCHED_IDLE);
        })
        .join()
        .unwrap();
    }
}
