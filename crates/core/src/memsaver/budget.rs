// SPDX-License-Identifier: BSD-2-Clause
//! Per-app memory budgets (ADR 0027, amendment 2026-11-04; numbers in
//! docs/bench/memsaver-discord.md). A budgeted app is held to a whole-scope PSS target in front
//! and in the background, with four tools:
//!
//! 1. A start in the right cgroup ([`launch_argv`], the shell's launch path): the cached pages
//!    of the app's own files are dropped ([`Budget::evict`]), then the app is born in
//!    `app-<engine name>-<pid>.scope` of the user's manager. Charges never move with a process:
//!    an app adopted after it started keeps its first pages charged to the session scope (which
//!    `memory.min` protects), and a limit on the new scope cannot reach them. The scope takes the
//!    name the engine would give the scope it moves itself into (Chromium's
//!    `app-<name>-<pid>.scope`), so that move finds the unit taken and the app stays put.
//! 2. Engine switches ([`crate::webruntime::third_party_flags`]) on that start, when the budget is
//!    `lean` and a row earned its place: none today, none gained anything in the benchmark.
//! 3. `memory.high` on the scope once the window is up ([`Budget::high`]): one value in front,
//!    a lower one in the background. Fixed values: a limit that follows the measured PSS was
//!    tried and oscillated into a stall (docs/bench/memsaver-discord.md, "governor").
//! 4. In the background, a freeze duty cycle ([`Cycle`]): frozen [`Duty::stop_ms`], thawed
//!    [`Duty::run_ms`], so the app reads its socket at least every `stop_ms` while the kernel
//!    buffers what arrives meanwhile.
//!
//! Pure (facts and a clock in, numbers out) except [`evict`], which the launcher calls.

/// One MiB in bytes.
pub const MIB: u64 = 1 << 20;

/// Background duty cycle: frozen this long, then thawed this long, repeated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Duty {
    pub stop_ms: u64,
    pub run_ms: u64,
}

/// One app's budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// App id (`shared::memsaver::app_id` of the window class).
    pub app: &'static str,
    /// The command the app's desktop entry runs (its file name), for the launch path.
    pub exec: &'static str,
    /// The name the app's engine gives its own scope (`app-<name>-<pid>.scope`).
    pub engine_name: &'static str,
    /// The promise: PSS of every process in the app's scope, in MiB, in front and behind.
    pub pss_mib: u64,
    /// `memory.high` in front (in use), MiB. Above the PSS target by what the scope is charged
    /// for that is not PSS (kernel memory, unmapped cache).
    pub front_high_mib: u64,
    /// `memory.high` in the background (from the saver's delay on), MiB.
    pub back_high_mib: u64,
    /// No limit for this long after the window first shows: a cold start under the limit never
    /// finished in the benchmark.
    pub start_ms: u64,
    /// `memory.swap.max` of the scope, MiB: what the app may keep compressed.
    pub swap_max_mib: u64,
    /// Background duty cycle; None: the app keeps running behind, only the limit holds it.
    pub duty: Option<Duty>,
    /// The shipped engine switches (`webruntime::third_party_flags`, none today) go on its start.
    pub lean: bool,
    /// Directories under the user's home whose cached pages are dropped before a start.
    pub evict: &'static [&'static str],
    /// The benchmark record of these numbers.
    pub record: &'static str,
}

/// The configuration measured on Discord, the default of every Electron app with its own engine
/// in [`TABLE`].
pub const ELECTRON: Budget = Budget {
    app: "",
    exec: "",
    engine_name: "",
    pss_mib: 100,
    front_high_mib: 150,
    back_high_mib: 100,
    start_ms: 60_000,
    swap_max_mib: 2048,
    // Off: the cycle (frozen 4 s, thawed 1 s) made a message in the first minute behind wait up
    // to 56 s and saved no measured memory (docs/bench/memsaver-discord.md, "Responsiveness").
    // The code stays for a row that earns it.
    duty: None,
    lean: true,
    evict: &[],
    record: "docs/bench/memsaver-discord.md",
};

/// Every budgeted app.
pub const TABLE: &[Budget] = &[Budget {
    app: "discord",
    exec: "discord",
    engine_name: "discord",
    evict: &[".config/discord"],
    ..ELECTRON
}];

/// The budget of an app id, if it has one.
pub fn of(app: &str) -> Option<&'static Budget> {
    of_in(TABLE, app)
}

/// [`of`] in a given table (the engine holds one, so a test can run a row of its own).
pub fn of_in(table: &'static [Budget], app: &str) -> Option<&'static Budget> {
    table.iter().find(|b| b.app == app)
}

/// The budget of the program a launch runs (`argv[0]`, path or name), if it has one.
pub fn of_exec(argv0: &str) -> Option<&'static Budget> {
    let name = argv0.rsplit('/').next().unwrap_or(argv0);
    TABLE.iter().find(|b| b.exec == name)
}

/// Where the app is, for its limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Still starting (`start_ms`), the saver off, excluded, or a game: no limit.
    Free,
    Front,
    Back,
}

impl Budget {
    /// The scope unit the app starts in: the engine's own name for it, so the engine's move
    /// into a scope of its own fails with "unit exists" and the app stays put.
    pub fn unit(&self, pid: u32) -> String {
        format!("app-{}-{pid}.scope", self.engine_name)
    }

    /// `memory.high` in bytes for a place; None: no limit.
    pub fn high(&self, at: Place) -> Option<u64> {
        match at {
            Place::Free => None,
            Place::Front => Some(self.front_high_mib * MIB),
            Place::Back => Some(self.back_high_mib * MIB),
        }
    }
}

/// `systemd-run` argv that starts `argv` in the app's budget scope of the user's manager, with
/// the lean switches when the budget has them. `pid` is the process id the app's main process
/// will have: the launcher's own, when it execs systemd-run (which execs the app in place).
pub fn launch_argv(b: &Budget, pid: u32, argv: &[String]) -> Vec<String> {
    let flags = if b.lean {
        crate::webruntime::third_party_flags()
    } else {
        Vec::new()
    };
    launch_argv_with(b, pid, argv, &flags)
}

/// [`launch_argv`] with the engine switches given.
fn launch_argv_with(b: &Budget, pid: u32, argv: &[String], flags: &[String]) -> Vec<String> {
    let mut a: Vec<String> = ["systemd-run", "--user", "--scope", "--quiet", "--collect"]
        .map(String::from)
        .into();
    a.push(format!("--unit={}", b.unit(pid)));
    // The start runs without a limit (`start_ms`); wold sets memory.high once the window is up.
    a.push(format!("--property=MemorySwapMax={}M", b.swap_max_mib));
    a.push("--".into());
    a.extend(argv.iter().cloned());
    a.extend(flags.iter().cloned());
    a
}

/// [`launch_argv`] for a launcher that cannot know the pid: `sh -c 'exec systemd-run ...
/// --unit=app-<name>-$$.scope ... -- "$@" <lean switches>' sh <argv...>`. The shell's `$$` is the
/// pid systemd-run, the app's start script and the app keep through their execs.
pub fn launch_sh(b: &Budget, argv: &[String]) -> Vec<String> {
    let flags = if b.lean {
        crate::webruntime::third_party_flags()
    } else {
        Vec::new()
    };
    launch_sh_with(b, argv, &flags)
}

/// [`launch_sh`] with the engine switches given.
fn launch_sh_with(b: &Budget, argv: &[String], flags: &[String]) -> Vec<String> {
    fn q(w: &str) -> String {
        format!("'{}'", w.replace('\'', "'\\''"))
    }
    let mut script = format!(
        "exec systemd-run --user --scope --quiet --collect --unit=app-{}-$$.scope \
         --property=MemorySwapMax={}M -- \"$@\"",
        b.engine_name, b.swap_max_mib
    );
    for f in flags {
        script.push(' ');
        script.push_str(&q(f));
    }
    let mut a = vec!["sh".to_owned(), "-c".to_owned(), script, "sh".to_owned()];
    a.extend(argv.iter().cloned());
    a
}

/// Drop the cached pages of the app's own files ([`Budget::evict`], under `home`) that nothing
/// maps, so its start reads them again into the scope it starts in (`posix_fadvise` DONTNEED;
/// pages a running process maps stay). Returns the bytes of the files asked. Symlinks are not
/// followed; a missing directory is nothing to do.
pub fn evict(home: &std::path::Path, b: &Budget) -> u64 {
    use std::os::unix::io::AsRawFd;
    fn walk(dir: &std::path::Path, depth: u32, total: &mut u64) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() && depth > 0 {
                walk(&e.path(), depth - 1, total);
            } else if t.is_file()
                && let Ok(f) = std::fs::File::open(e.path())
            {
                // SAFETY: the descriptor is open and owned by `f`, which outlives the call;
                // posix_fadvise reads and writes no memory of ours.
                if unsafe { libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) }
                    == 0
                {
                    *total += f.metadata().map_or(0, |m| m.len());
                }
            }
        }
    }
    let mut total = 0;
    for d in b.evict {
        walk(&home.join(d), 8, &mut total);
    }
    total
}

/// Where a budgeted app is in its background duty cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cycle {
    /// In use (or guarded): never frozen by the cycle.
    Off,
    Stopped {
        since: u64,
    },
    Running {
        since: u64,
    },
}

/// What the cycle asks of the scope's `cgroup.freeze`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Keep,
    Freeze,
    Thaw,
}

impl Cycle {
    /// One step. `cycling`: the app is in the background past the saver's delay and nothing
    /// guards it (not in use, not playing, not excluded, not a game).
    pub fn step(self, cycling: bool, now: u64, d: &Duty) -> (Cycle, Gate) {
        match (self, cycling) {
            (Cycle::Stopped { .. }, false) => (Cycle::Off, Gate::Thaw),
            (_, false) => (Cycle::Off, Gate::Keep),
            (Cycle::Off, true) => (Cycle::Stopped { since: now }, Gate::Freeze),
            (Cycle::Stopped { since }, true) if now.saturating_sub(since) >= d.stop_ms => {
                (Cycle::Running { since: now }, Gate::Thaw)
            }
            (Cycle::Running { since }, true) if now.saturating_sub(since) >= d.run_ms => {
                (Cycle::Stopped { since: now }, Gate::Freeze)
            }
            (c, true) => (c, Gate::Keep),
        }
    }

    /// When the next step is due (monotonic ms), None when the cycle is off.
    pub fn due(self, d: &Duty) -> Option<u64> {
        match self {
            Cycle::Off => None,
            Cycle::Stopped { since } => Some(since + d.stop_ms),
            Cycle::Running { since } => Some(since + d.run_ms),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_has_the_100_mb_budget_and_unknown_apps_have_none() {
        let d = of("discord").expect("discord is budgeted");
        assert_eq!(d.pss_mib, 100);
        assert_eq!(d.engine_name, "discord");
        assert_eq!(of_exec("/usr/bin/discord"), Some(d));
        assert_eq!(of_exec("discord"), Some(d));
        assert_eq!(of_exec("discordia"), None);
        assert_eq!(d.duty, None, "the freeze cycle ships off");
        assert!(
            ELECTRON.duty.is_none(),
            "so does the default row of the next app"
        );
        assert!(d.back_high_mib <= d.front_high_mib);
        assert!(d.lean);
        assert_eq!(of("code"), None, "the editor is not budgeted");
        assert_eq!(of("helium"), None, "nor the browser");
        assert_eq!(of(""), None, "the default row is not an app");
        for b in TABLE {
            assert!(
                !b.app.is_empty() && !b.engine_name.is_empty() && !b.exec.is_empty(),
                "{b:?}"
            );
            assert!(b.record.starts_with("docs/bench/"), "{b:?}");
            assert_eq!(b.app, shared::memsaver::app_id(b.app), "ids are normalised");
            assert!(
                b.evict
                    .iter()
                    .all(|e| !e.starts_with('/') && !e.contains("..")),
                "{b:?}"
            );
        }
    }

    #[test]
    fn limits_by_place() {
        let d = of("discord").unwrap();
        assert_eq!(d.high(Place::Free), None);
        assert_eq!(d.high(Place::Front), Some(150 * MIB));
        assert_eq!(d.high(Place::Back), Some(100 * MIB));
    }

    #[test]
    fn the_launch_takes_the_scope_name_the_engine_would_move_into() {
        let d = of("discord").unwrap();
        assert_eq!(d.unit(4242), "app-discord-4242.scope");
        let args = ["discord".to_owned(), "--start-minimized".to_owned()];
        let s = launch_argv(d, 4242, &args).join(" ");
        assert_eq!(
            s,
            "systemd-run --user --scope --quiet --collect --unit=app-discord-4242.scope \
             --property=MemorySwapMax=2048M -- discord --start-minimized",
            "no engine switch ships, and no limit while it starts"
        );
        let with = launch_argv_with(d, 4242, &args, &["--renderer-process-limit=1".into()]);
        assert_eq!(with.last().unwrap(), "--renderer-process-limit=1");
    }

    /// The `sh -c` launch, run for real with a stand-in systemd-run that prints its arguments:
    /// the unit carries the pid the app keeps, the app's own arguments and the lean switches
    /// arrive word for word (the script-engine switch holds a space).
    #[test]
    fn the_shell_launch_names_the_scope_after_the_pid_the_app_keeps() {
        let d = of("discord").unwrap();
        let dir = std::env::temp_dir().join(format!("wol-budget-sh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("systemd-run");
        std::fs::write(
            &fake,
            "#!/bin/sh\necho \"pid=$$\"\nfor a in \"$@\"; do echo \"[$a]\"; done\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let flags = [
            "--renderer-process-limit=1".to_owned(),
            "--js-flags=--optimize-for-size --x='y'".to_owned(),
        ];
        let a = launch_sh_with(d, &["discord".into(), "it's".into()], &flags);
        assert_eq!(a[..2], ["sh", "-c"]);
        let path = format!(
            "{}:{}",
            dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let out = std::process::Command::new(&a[0])
            .args(&a[1..])
            .env("PATH", path)
            .output()
            .unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        let pid = text.lines().next().unwrap().strip_prefix("pid=").unwrap();
        assert!(
            text.contains(&format!("[--unit=app-discord-{pid}.scope]")),
            "{text}"
        );
        assert!(text.contains("[--]\n[discord]\n[it's]\n"), "{text}");
        for f in &flags {
            assert!(text.contains(&format!("[{f}]")), "{f}: {text}");
        }
        assert!(
            launch_sh(d, &["discord".into()])[2].ends_with("-- \"$@\""),
            "no engine switch ships"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn evict_reads_only_the_budgets_own_directories() {
        let home = std::env::temp_dir().join(format!("wol-budget-evict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let app = home.join(".config/discord/app-1/resources");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("app.asar"), vec![7u8; 4096]).unwrap();
        std::fs::write(home.join(".config/other"), vec![7u8; 999]).unwrap();
        let d = of("discord").unwrap();
        assert_eq!(evict(&home, d), 4096, "the app's file, not its neighbour");
        assert_eq!(evict(&home.join("nowhere"), d), 0);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_duty_cycle_freezes_four_seconds_runs_one_and_thaws_when_used() {
        let d = Duty {
            stop_ms: 4_000,
            run_ms: 1_000,
        };
        let mut c = Cycle::Off;
        let mut gates = Vec::new();
        for (t, cycling) in [
            (0, false),
            (100, true),
            (2_000, true),
            (4_100, true),
            (4_600, true),
            (5_100, true),
            (9_100, true),
            (9_500, false),
            (9_600, false),
        ] {
            let (n, g) = c.step(cycling, t, &d);
            if g != Gate::Keep {
                gates.push((t, g));
            }
            c = n;
        }
        assert_eq!(
            gates,
            vec![
                (100, Gate::Freeze),
                (4_100, Gate::Thaw),
                (5_100, Gate::Freeze),
                (9_100, Gate::Thaw),
            ],
            "used again while running: nothing to thaw"
        );
        assert_eq!(c, Cycle::Off);
        // Used while frozen: thawed at once.
        let (c, g) = Cycle::Stopped { since: 0 }.step(false, 1, &d);
        assert_eq!((c, g), (Cycle::Off, Gate::Thaw));
        // The next step is due at the end of the current half.
        assert_eq!(Cycle::Stopped { since: 10 }.due(&d), Some(4_010));
        assert_eq!(Cycle::Running { since: 10 }.due(&d), Some(1_010));
        assert_eq!(Cycle::Off.due(&d), None);
    }
}
