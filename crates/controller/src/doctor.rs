// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol doctor [area]` and `wol doctor fix <id>` (docs/modules/doctor.md): every area of the
//! computer (sound, games, updates, security, storage, network, power, display, hardware) as one list of
//! [`Check`]s in plain words, each with the fix it can run. The facts and rules stay where they
//! live (`audio_doctor`, `wol_core::games::doctor`, `wol_core::security`, `wol_core::storage`,
//! `wol_core::netinfo`, `wol_core::power`, `wol_core::hardware`); this module only words them for
//! a person and runs a fix through the path the owning verb uses (wol-admind for root changes).
use crate::cli::Invocation;
use crate::run::{Env, admin_send_said, emit_json, fail};
use shared::admin::{AdminAction, AdminRequest};
use shared::errors::ErrorCode;
use shared::hardware::{HardwareSnapshot, Kind};
use shared::shell::ApprovalRequest;
use std::cell::RefCell;
use std::path::Path;
use wol_core::games::doctor as games;

pub mod groups;
pub mod hw;
pub mod tls;

/// Every area, in the order the Doctor app lists them.
pub const AREAS: &[&str] = &[
    "sound", "games", "updates", "security", "storage", "network", "power", "display", "hardware",
];
/// Areas run only when asked for by name (`wol doctor tls` connects to servers outside the
/// computer, so neither plain `wol doctor` nor the Doctor app runs it).
pub const ON_ASK: &[&str] = &[tls::AREA];
/// What `wol doctor <area>` accepts: [`AREAS`], then [`ON_ASK`].
pub const CHOICES: &[&str] = &[
    "sound", "games", "updates", "security", "storage", "network", "power", "display", "hardware",
    "tls",
];

/// Last-check age that turns the update check into a warning: three days.
const UPDATE_STALE_SECS: u64 = 3 * 86_400;
/// Malware signatures older than this are worth a look.
const SIGNATURES_STALE_DAYS: u64 = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
    Skipped,
}

/// What a fix does, run by [`fix`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// wol-admind requests, in order (asks for the password like every admin verb).
    Admin(Vec<AdminRequest>),
    /// One command as the person (their own services and settings).
    User(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Fix {
    /// The button: "Install it", "Turn on the firewall".
    pub title: String,
    /// The command it runs, for the "Show the command" disclosure.
    pub command: String,
    pub needs_admin: bool,
    #[serde(skip)]
    pub action: Action,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Check {
    /// `<area>.<name>`, stable: `wol doctor fix <id>`.
    pub id: String,
    pub area: &'static str,
    pub title: String,
    pub status: Status,
    /// Plain words for a person.
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
    /// The error text when the check itself could not run (it crashed): for the report, apart from
    /// the words in `detail`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn check(area: &'static str, name: &str, title: &str, status: Status, detail: &str) -> Check {
    Check {
        id: format!("{area}.{name}"),
        area,
        title: title.into(),
        status,
        detail: detail.into(),
        fix: None,
        error: None,
    }
}

impl Check {
    fn with(mut self, fix: Option<Fix>) -> Self {
        self.fix = fix;
        self
    }
}

impl Fix {
    fn admin(title: &str, reqs: Vec<AdminRequest>) -> Option<Self> {
        if reqs.is_empty() {
            return None;
        }
        let command = reqs
            .iter()
            .map(|r| {
                let mut l = format!("wol admin {}", r.action.name());
                for a in &r.args {
                    l.push(' ');
                    l.push_str(a);
                }
                l
            })
            .collect::<Vec<_>>()
            .join("; ");
        Some(Fix {
            title: title.into(),
            command,
            needs_admin: true,
            action: Action::Admin(reqs),
        })
    }

    fn user(title: &str, argv: Vec<String>) -> Option<Self> {
        (!argv.is_empty()).then(|| Fix {
            title: title.into(),
            command: argv.join(" "),
            needs_admin: false,
            action: Action::User(argv),
        })
    }
}

fn req(action: AdminAction, args: &[&str]) -> AdminRequest {
    AdminRequest {
        action,
        args: args.iter().map(|s| s.to_string()).collect(),
    }
}

/// The runnable part of an existing doctor's fix line: `[sudo] pacman -S [--needed] pkgs` (one
/// wol-admind install each), `systemctl --user start units`, `wpctl set-volume id level`. Advice
/// ("plug in a microphone", "enable [multilib]") and root commands with no wol-admind action
/// give None: the detail says what to do.
pub fn action_of(line: &str) -> Option<Action> {
    let first = line
        .split(';')
        .next()?
        .split(" (")
        .next()?
        .split(" or ")
        .next()?;
    let words: Vec<&str> = first.split_whitespace().collect();
    // A doctor's line may put a privilege word before `pacman`; wol-admind is that path here.
    let words = match words.as_slice() {
        [_, "pacman", ..] => &words[1..],
        _ => &words[..],
    };
    match words {
        ["pacman", "-S", rest @ ..] => {
            let reqs: Vec<AdminRequest> = rest
                .iter()
                .filter(|w| !w.starts_with('-') && wol_core::store::is_pkg_name(w))
                .map(|p| req(AdminAction::PkgInstall, &[p]))
                .collect();
            (!reqs.is_empty()).then_some(Action::Admin(reqs))
        }
        ["systemctl", "--user", "start", units @ ..] if !units.is_empty() => {
            Some(Action::User(words.iter().map(|s| s.to_string()).collect()))
        }
        ["wpctl", "set-volume", id, level]
            if id.bytes().all(|b| b.is_ascii_digit()) && level.parse::<f64>().is_ok() =>
        {
            Some(Action::User(words.iter().map(|s| s.to_string()).collect()))
        }
        _ => None,
    }
}

/// [`action_of`] as a [`Fix`] with a button title.
fn fix_of(line: &str, title: &str) -> Option<Fix> {
    match action_of(line)? {
        Action::Admin(r) => Fix::admin(title, r),
        Action::User(a) => Fix::user(title, a),
    }
}

// ---------------------------------------------------------------- sound

/// `wol audio doctor`'s checks, worded for a person.
fn sound(env: &mut dyn Env) -> Vec<Check> {
    use crate::audio_doctor::Status as S;
    let mut v: Vec<Check> = crate::audio_doctor::checks(env)
        .into_iter()
        .map(|c| {
            let st = match c.status {
                S::Ok => Status::Ok,
                S::Warn => Status::Warn,
                S::Fail => Status::Fail,
            };
            let ok = st == Status::Ok;
            let (title, detail, button): (&str, String, &str) = match c.id {
                "pipewire" => (
                    "Sound service",
                    if ok {
                        "Sound plays through the system.".into()
                    } else {
                        "The sound service is stopped, so nothing plays.".into()
                    },
                    "Start it again",
                ),
                "wireplumber" => (
                    "Speakers and microphones",
                    if ok {
                        "Your speakers and microphones are found and remembered.".into()
                    } else {
                        "The part that finds speakers and microphones is stopped, so volume and device choices do nothing.".into()
                    },
                    "Start it again",
                ),
                "realtime" => (
                    "Smooth sound when busy",
                    match st {
                        Status::Ok => "Sound gets the first turn on the processor, so it stays smooth when the computer is busy.".into(),
                        Status::Warn => "Sound still gets priority, through the backup path WolOS sets up.".into(),
                        _ => "Sound waits behind other work, so it can stutter when the computer is busy.".into(),
                    },
                    "Install it",
                ),
                "config" => (
                    "WolOS sound tuning",
                    if ok {
                        "The WolOS tuning for games, calls and microphones is in place.".into()
                    } else {
                        "Part of the WolOS sound tuning is missing or was changed.".into()
                    },
                    "Put it back",
                ),
                "input-level" => (
                    "Microphone level",
                    if ok {
                        "Your microphone records at a clean level.".into()
                    } else if c.fix.starts_with("plug in") {
                        "No microphone is connected. Plug one in or pair one to talk and record.".into()
                    } else {
                        "Your microphone is set louder than it can record cleanly, so your voice may crackle.".into()
                    },
                    "Set the right level",
                ),
                "clock" => (
                    "Sound timing",
                    if c.detail.contains("Pro Audio") {
                        "Sound timing is set by hand in Settings > Sound > Pro Audio.".into()
                    } else {
                        "Sound timing adjusts itself to each app.".into()
                    },
                    "",
                ),
                "xruns" => (
                    "Dropouts",
                    if ok {
                        "No sound dropouts since the devices started.".into()
                    } else {
                        let n = c.detail.split_whitespace().next().unwrap_or("Some");
                        format!(
                            "Sound dropped out {n} times. A larger buffer in Settings > Sound > Pro Audio, or another USB port, stops it."
                        )
                    },
                    "",
                ),
                "rnnoise" => (
                    "Background noise removal",
                    if ok {
                        "Background noise removal is ready for your microphone.".into()
                    } else {
                        "Background noise removal is not installed, so its switch in Sound settings does nothing.".into()
                    },
                    "Install it",
                ),
                "obs" => (
                    "Recording and streaming",
                    if c.detail.ends_with("not installed") {
                        "No recording app is installed.".into()
                    } else if ok {
                        "Your recording app captures app and desktop sound.".into()
                    } else {
                        "Your recording app cannot capture app and desktop sound yet.".into()
                    },
                    "Install it",
                ),
                "portal" => (
                    "Screen recording",
                    if ok {
                        "Apps can ask to record your screen.".into()
                    } else {
                        "Apps cannot ask to record your screen, so screen capture shows nothing.".into()
                    },
                    "Install it",
                ),
                _ => ("Sound", c.detail.clone(), "Fix it"),
            };
            let st = if c.id == "obs" && c.detail.ends_with("not installed") {
                Status::Skipped
            } else {
                st
            };
            let fix = if st == Status::Ok || button.is_empty() {
                None
            } else {
                fix_of(&c.fix, button)
            };
            check("sound", c.id, title, st, &detail).with(fix)
        })
        .collect();
    v.push(devices(
        &env.hardware(),
        "sound",
        "devices",
        "Sound devices",
        &[Kind::Audio],
        "Every sound device has its driver.",
        "No sound card was found.",
    ));
    v
}

// ---------------------------------------------------------------- games

fn games(env: &mut dyn Env) -> Vec<Check> {
    games::checks(&env.games_facts())
        .into_iter()
        .map(|c| {
            let st = match c.status {
                games::Status::Ok => Status::Ok,
                games::Status::Info => Status::Skipped,
                games::Status::Warn => Status::Warn,
                games::Status::Fail => Status::Fail,
            };
            let ok = st == Status::Ok;
            let (title, detail, button): (&str, String, &str) = match c.id {
                "base" => (
                    "WolOS for games",
                    if ok {
                        "You run the WolOS kernel, tuned for games.".into()
                    } else {
                        "You run your own kernel. WolOS keeps it as it is.".into()
                    },
                    "",
                ),
                "multilib" => (
                    "Game package source",
                    if ok {
                        "The package source that carries Steam and its 32-bit parts is on.".into()
                    } else {
                        "The package source that carries Steam and its 32-bit parts is off, so Steam cannot be installed.".into()
                    },
                    "",
                ),
                "steam" => (
                    "Steam",
                    if ok {
                        "Steam is installed.".into()
                    } else {
                        "Steam is not installed.".into()
                    },
                    "Install Steam",
                ),
                "drm" => (
                    "Graphics driver",
                    if ok {
                        "The graphics driver is running.".into()
                    } else {
                        "No graphics driver is running, so games cannot start.".into()
                    },
                    "",
                ),
                "lib32-gl" => (
                    "Graphics for Steam",
                    if ok {
                        "The 32-bit graphics parts Steam needs are installed.".into()
                    } else {
                        "The 32-bit graphics parts Steam needs are missing, so Steam cannot open.".into()
                    },
                    "Install them",
                ),
                "vulkan-icd" => (
                    "Driver for modern games",
                    if ok {
                        "A graphics driver for modern games is installed.".into()
                    } else {
                        "No graphics driver for modern games is installed.".into()
                    },
                    "Install it",
                ),
                "vulkan" => (
                    "Graphics card for games",
                    match st {
                        Status::Ok => format!(
                            "Games run on {}.",
                            c.detail.split(" (").next().unwrap_or("your graphics card")
                        ),
                        Status::Warn => "The graphics test tool is missing, so WolOS cannot see which card games use.".into(),
                        _ => "Games would run on the processor instead of the graphics card, and crawl.".into(),
                    },
                    "Install it",
                ),
                "nvidia" => (
                    "NVIDIA driver",
                    if ok {
                        "The NVIDIA driver and its 32-bit parts match.".into()
                    } else {
                        "The 32-bit parts of the NVIDIA driver are missing or older than the driver.".into()
                    },
                    "Install them",
                ),
                "futex-waitv" => (
                    "Fast waiting for Windows games",
                    match st {
                        Status::Ok => "Windows games use the fastest way to wait.".into(),
                        Status::Warn => "Windows games use a slower way to wait. They still run.".into(),
                        _ => "Checked when a game first starts.".into(),
                    },
                    "",
                ),
                "ntsync" => (
                    "Windows timing",
                    if ok {
                        "Windows games get native timing.".into()
                    } else {
                        "Windows games use the standard timing, which works well.".into()
                    },
                    "",
                ),
                "wine" => (
                    "Windows apps outside Steam",
                    if ok {
                        "Windows apps outside Steam can run.".into()
                    } else {
                        "Not installed. Steam games do not need it.".into()
                    },
                    "Install it",
                ),
                _ => ("Games", c.detail.clone(), "Fix it"),
            };
            let fix = if ok || button.is_empty() {
                None
            } else {
                fix_of(&c.fix, button)
            };
            check("games", c.id, title, st, &detail).with(fix)
        })
        .collect()
}

// ---------------------------------------------------------------- updates

fn updates(env: &mut dyn Env) -> Vec<Check> {
    let now = env.now_secs();
    let Some(u) = crate::run::update_snapshot(env) else {
        return vec![
            check(
                "updates",
                "checked",
                "Looking for updates",
                // A fresh install before its first daily look: nothing is wrong yet (QA-100 H3-5).
                Status::Skipped,
                "WolOS looks for updates every day. The first look has not happened yet.",
            ),
            check(
                "updates",
                "waiting",
                "Updates",
                Status::Skipped,
                "Shown after the first look for updates.",
            ),
        ];
    };
    let days = now.saturating_sub(u.checked) / 86_400;
    let when = match days {
        0 => "today".to_string(),
        1 => "yesterday".to_string(),
        d => format!("{d} days ago"),
    };
    let stale = now.saturating_sub(u.checked) > UPDATE_STALE_SECS;
    let mut v = vec![check(
        "updates",
        "checked",
        "Looking for updates",
        if stale { Status::Warn } else { Status::Ok },
        &if stale {
            format!("WolOS last looked for updates {when}. It looks again once you are online.")
        } else {
            format!("WolOS looked for updates {when}.")
        },
    )];
    let pkgs = u.packages.len();
    let sec = u.security.len();
    let upgrade = || Fix::admin("Install updates", vec![req(AdminAction::PkgUpgrade, &[])]);
    v.push(match (pkgs, &u.base) {
        (0, None) => check(
            "updates",
            "waiting",
            "Updates",
            Status::Ok,
            "Everything is up to date.",
        ),
        (0, Some(b)) => check(
            "updates",
            "waiting",
            "Updates",
            Status::Warn,
            &format!("WolOS {b} is ready. Open Settings > Software Update to install it."),
        ),
        (n, _) if sec > 0 => check(
            "updates",
            "waiting",
            "Updates",
            Status::Fail,
            &format!(
                "{n} {} ready. {sec} of them {} security holes.",
                plural(n, "update is", "updates are"),
                plural(sec, "closes", "close")
            ),
        )
        .with(upgrade()),
        (n, _) => check(
            "updates",
            "waiting",
            "Updates",
            Status::Warn,
            &format!(
                "{n} {} ready to install.",
                plural(n, "update is", "updates are")
            ),
        )
        .with(upgrade()),
    });
    v
}

fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

// ---------------------------------------------------------------- security

fn security(env: &mut dyn Env) -> Vec<Check> {
    let (st, malware) = {
        let (now, uid) = (env.now_secs(), env.capture(&["id", "-u"]));
        let uid = uid.and_then(|u| u.trim().parse().ok()).unwrap_or(u32::MAX);
        let cell = RefCell::new(&mut *env);
        let probe = |argv: &[&str]| cell.borrow_mut().capture(argv);
        let st = wol_core::security::status(&probe);
        let read = |p: &str| cell.borrow().read_file(Path::new(p));
        (st, wol_core::malware::status(&probe, &read, uid, now))
    };
    let on = || {
        Fix::admin(
            "Turn on the firewall",
            vec![req(AdminAction::Firewall, &["on"])],
        )
    };
    let mut v = vec![match (st.firewall, st.firewall_running) {
        (Some(true), Some(false)) => check(
            "security",
            "firewall",
            "Firewall",
            Status::Warn,
            "The firewall is on but not guarding right now.",
        )
        .with(on()),
        (Some(true), _) => check(
            "security",
            "firewall",
            "Firewall",
            Status::Ok,
            "The firewall is on. Nothing comes in unless you ask for it.",
        ),
        (Some(false), _) => check(
            "security",
            "firewall",
            "Firewall",
            Status::Fail,
            "The firewall is off, so other computers on the network can reach this one.",
        )
        .with(on()),
        (None, _) => check(
            "security",
            "firewall",
            "Firewall",
            Status::Skipped,
            "This computer has no firewall to check.",
        ),
    }];
    v.push(if st.remote_login == Some(true) {
        check(
            "security",
            "remote-login",
            "Remote Login",
            Status::Warn,
            "People on your network can sign in to this computer from another one.",
        )
        .with(Fix::admin(
            "Turn off Remote Login",
            vec![req(AdminAction::RemoteLogin, &["off"])],
        ))
    } else {
        check(
            "security",
            "remote-login",
            "Remote Login",
            Status::Ok,
            "Nobody can sign in to this computer from another one.",
        )
    });
    v.push(if st.system_log_private {
        check(
            "security",
            "system-log",
            "System log",
            Status::Ok,
            "The system log stays on this computer.",
        )
    } else {
        check(
            "security",
            "system-log",
            "System log",
            Status::Warn,
            "A program on this computer can send the system log over the network.",
        )
    });
    let known: Vec<_> = st.hardening.iter().filter(|c| c.now.is_some()).collect();
    let weaker = known.iter().filter(|c| !c.ok()).count();
    v.push(match (known.len(), weaker) {
        (0, _) => check(
            "security",
            "protections",
            "Core protections",
            Status::Skipped,
            "The core protections are read on a running WolOS system.",
        ),
        (_, 0) => check(
            "security",
            "protections",
            "Core protections",
            Status::Ok,
            "Every protection WolOS sets in the system core is on.",
        ),
        (_, n) => check(
            "security",
            "protections",
            "Core protections",
            Status::Warn,
            &format!(
                "{n} {} weaker than WolOS sets {}. A restart puts {} back.",
                plural(
                    n,
                    "protection in the system core is",
                    "protections in the system core are"
                ),
                plural(n, "it", "them"),
                plural(n, "it", "them")
            ),
        ),
    });
    // QA-FILES-2: accounts an older release left in input/audio (doctor/groups.rs).
    let (group, passwd) = (
        env.read_file(Path::new("/etc/group")),
        env.read_file(Path::new("/etc/passwd")),
    );
    v.push(groups::check(group.as_deref(), passwd.as_deref()));
    // QA-FILES-25: the last administrator removed from wheel.
    v.push(groups::administrators_check(
        group.as_deref(),
        passwd.as_deref(),
    ));
    // ADR 0055: the system TLS policy and the browser's TLS rows (files only, no network).
    v.extend(tls::file_checks(env));
    v.extend(malware_checks(&malware));
    v
}

/// The malware check and what it stopped.
fn malware_checks(malware: &wol_core::malware::Status) -> Vec<Check> {
    let mut v = Vec::new();
    v.push(match (malware.on, malware.signatures_age_days) {
        (Some(true), Some(d)) if d > SIGNATURES_STALE_DAYS => check(
            "security",
            "malware",
            "Malware check",
            Status::Warn,
            &format!("The malware check knows threats up to {d} days ago. It catches up once you are online."),
        ),
        (Some(true), _) => check(
            "security",
            "malware",
            "Malware check",
            Status::Ok,
            "The malware check is on and up to date.",
        ),
        (Some(false), _) => check(
            "security",
            "malware",
            "Malware check",
            Status::Warn,
            "The malware check is off. Turn it on in Settings > Privacy & Security.",
        ),
        (None, _) => check(
            "security",
            "malware",
            "Malware check",
            Status::Skipped,
            "The malware check runs on a WolOS system.",
        ),
    });
    v.push(match malware.blocked.len() {
        0 => check(
            "security",
            "stopped",
            "Harmful files",
            Status::Ok,
            "No harmful files were found.",
        ),
        n => {
            let names: Vec<&str> = malware
                .blocked
                .iter()
                .take(3)
                .map(|b| wol_core::malware::file_name(&b.path))
                .collect();
            check(
                "security",
                "stopped",
                "Harmful files",
                Status::Warn,
                &format!(
                    "WolOS stopped {n} harmful {}: {}. They cannot run. Review them in Settings > Privacy & Security.",
                    plural(n, "file", "files"),
                    names.join(", ")
                ),
            )
        }
    });
    v
}

// ---------------------------------------------------------------- storage

fn storage(env: &mut dyn Env) -> Vec<Check> {
    use wol_core::storage::human;
    let Some(s) = wol_core::storage::read(&*env.controls().host) else {
        return vec![check(
            "storage",
            "space",
            "Free space",
            Status::Skipped,
            "The startup disk could not be read.",
        )];
    };
    let free = s.total.saturating_sub(s.used);
    let pct = (s.used.saturating_mul(100))
        .checked_div(s.total)
        .unwrap_or(0);
    let mut v = vec![if pct >= 95 {
        check(
            "storage",
            "space",
            "Free space",
            Status::Fail,
            &format!(
                "The startup disk is almost full: {} free of {}. Apps and updates can fail until you make room.",
                human(free),
                human(s.total)
            ),
        )
    } else if pct >= 85 {
        check(
            "storage",
            "space",
            "Free space",
            Status::Warn,
            &format!(
                "{} free of {}. Make room soon so updates have space.",
                human(free),
                human(s.total)
            ),
        )
    } else {
        check(
            "storage",
            "space",
            "Free space",
            Status::Ok,
            &format!("{} free of {}.", human(free), human(s.total)),
        )
    }];
    v.push(match s.encrypted {
        Some(true) => check(
            "storage",
            "encryption",
            "Disk lock",
            Status::Ok,
            "Your files are locked with your password whenever the computer is off.",
        ),
        Some(false) => check(
            "storage",
            "encryption",
            "Disk lock",
            Status::Warn,
            "Your files are not locked. Anyone who takes the disk can read them.",
        ),
        None => check(
            "storage",
            "encryption",
            "Disk lock",
            Status::Skipped,
            "This disk does not say whether it is locked.",
        ),
    });
    v.push(if s.zfs {
        check(
            "storage",
            "restore-points",
            "Restore points",
            Status::Ok,
            &format!(
                "{} {} let you go back to how things were before an update.",
                s.boot_environments,
                plural(
                    usize::try_from(s.boot_environments).unwrap_or(usize::MAX),
                    "restore point",
                    "restore points"
                )
            ),
        )
    } else {
        check(
            "storage",
            "restore-points",
            "Restore points",
            Status::Skipped,
            "This disk does not keep restore points.",
        )
    });
    v
}

// ---------------------------------------------------------------- network

fn network(env: &mut dyn Env) -> Vec<Check> {
    let info = wol_core::netinfo::read(&*env.controls().host);
    let up = info.interfaces.iter().find(|i| i.up && !i.ipv4.is_empty());
    let online = info.gateway.is_some();
    let mut v = vec![if online {
        let how = match up.map(|i| i.kind) {
            Some(wol_core::netinfo::IfKind::WiFi) => "Connected through Wi-Fi.",
            Some(wol_core::netinfo::IfKind::Ethernet) => "Connected through a cable.",
            _ => "Connected to a network.",
        };
        check("network", "connection", "Connection", Status::Ok, how)
    } else {
        check(
            "network",
            "connection",
            "Connection",
            Status::Fail,
            "Not connected to a network. Join a Wi-Fi network or plug in a cable.",
        )
    }];
    v.push(match (online, info.dns.is_empty()) {
        (false, _) => check(
            "network",
            "names",
            "Finding websites",
            Status::Skipped,
            "Checked once you are connected.",
        ),
        (true, true) => check(
            "network",
            "names",
            "Finding websites",
            Status::Fail,
            "The network gives no way to find websites by name, so pages do not open.",
        ),
        (true, false) => check(
            "network",
            "names",
            "Finding websites",
            Status::Ok,
            "Websites are found by name.",
        ),
    });
    v.push(devices(
        &env.hardware(),
        "network",
        "devices",
        "Network devices",
        &[Kind::Wifi, Kind::Ethernet, Kind::Bluetooth],
        "Every Wi-Fi, cable and Bluetooth device has its driver.",
        "No Wi-Fi, cable or Bluetooth device was found.",
    ));
    v
}

// ---------------------------------------------------------------- power

fn power(env: &mut dyn Env) -> Vec<Check> {
    use wol_core::power::policy;
    let f = {
        let cell = RefCell::new(&mut *env);
        policy::facts(&|argv: &[&str]| cell.borrow_mut().capture(argv))
    };
    let mut v = vec![match (f.battery, f.source) {
        (None, _) => check(
            "power",
            "battery",
            "Battery",
            Status::Skipped,
            "This computer runs from the wall and has no battery.",
        ),
        (Some(b), policy::Source::Battery) if b <= 10 => check(
            "power",
            "battery",
            "Battery",
            Status::Warn,
            &format!("The battery is at {b}%. Plug in soon."),
        ),
        (Some(b), policy::Source::Battery) => check(
            "power",
            "battery",
            "Battery",
            Status::Ok,
            &format!("The battery is at {b}%."),
        ),
        (Some(b), policy::Source::Ac) => check(
            "power",
            "battery",
            "Battery",
            Status::Ok,
            &format!("Plugged in, the battery is at {b}%."),
        ),
    }];
    v.push(if f.sleep {
        check(
            "power",
            "sleep",
            "Sleep",
            Status::Ok,
            "This computer can sleep and wake where you left off.",
        )
    } else if f.battery.is_none() || in_vm(env) {
        // A virtual machine or a desktop with no sleep state: that is how it is built, not
        // something to fix, and nothing here could fix it (QA-100 H3-5).
        check(
            "power",
            "sleep",
            "Sleep",
            Status::Skipped,
            "This computer does not sleep, so it stays on when you step away. Desktops and virtual machines are often built this way.",
        )
    } else {
        check(
            "power",
            "sleep",
            "Sleep",
            Status::Warn,
            "This computer cannot sleep, so it stays on when you step away.",
        )
    });
    v
}

/// The system runs in a virtual machine (`systemd-detect-virt --vm` names one; `none` or no
/// answer is real hardware).
fn in_vm(env: &mut dyn Env) -> bool {
    // A failed run (not a systemd system) names no machine: real hardware, said out loud.
    match env.capture(&["systemd-detect-virt", "--vm"]) {
        Some(s) => !matches!(s.trim(), "" | "none"),
        None => false,
    }
}

// ---------------------------------------------------------------- display

fn display(env: &mut dyn Env) -> Vec<Check> {
    let monitors = env.controls().monitors();
    let mut v = vec![match monitors.as_slice() {
        [] => check(
            "display",
            "screens",
            "Screens",
            Status::Skipped,
            "Screens are checked from the desktop.",
        ),
        ms => {
            let list: Vec<String> = ms
                .iter()
                .map(|m| format!("{} × {} at {} Hz", m.width, m.height, m.refresh.round()))
                .collect();
            check(
                "display",
                "screens",
                "Screens",
                Status::Ok,
                &format!(
                    "{} {}: {}.",
                    ms.len(),
                    plural(ms.len(), "screen", "screens"),
                    list.join(", ")
                ),
            )
        }
    }];
    let snap = env.hardware();
    v.push(devices(
        &snap,
        "display",
        "devices",
        "Graphics",
        &[Kind::Gpu, Kind::Backlight],
        "The graphics card and screen brightness have their drivers.",
        "No graphics card was found.",
    ));
    // Cameras are pictures, not sound (QA-100 H3-5): their own row next to the screens.
    v.push(devices(
        &snap,
        "display",
        "cameras",
        "Cameras",
        &[Kind::Webcam],
        "Every camera has its driver.",
        "No camera was found.",
    ));
    v
}

// ---------------------------------------------------------------- hardware

/// Devices of `kinds` that are not ready, with `wol hw fix --yes`'s requests as the fix, at most
/// [`MAX_DOCTOR_STEPS`] of them so the password sheet can ask (QA-100 H3-4): over the cap the
/// module loads are left to the next start (`wol hw boot` loads them), then only the first
/// packages are asked and the detail says the rest come after "Check again".
///
/// [`MAX_DOCTOR_STEPS`]: shared::shell::MAX_DOCTOR_STEPS
fn devices(
    snap: &HardwareSnapshot,
    area: &'static str,
    name: &str,
    title: &str,
    kinds: &[Kind],
    ok: &str,
    none: &str,
) -> Check {
    let mine: Vec<_> = snap
        .devices
        .iter()
        .filter(|d| kinds.contains(&d.kind))
        .collect();
    let bad: Vec<_> = mine
        .iter()
        .filter(|d| d.state != shared::hardware::State::Ready)
        .collect();
    if mine.is_empty() {
        return check(area, name, title, Status::Skipped, none);
    }
    if bad.is_empty() {
        return check(area, name, title, Status::Ok, ok);
    }
    let names: Vec<&str> = bad.iter().map(|d| d.name.as_str()).collect();
    let (reqs, later) = device_steps(&bad);
    if reqs.is_empty() {
        // Nothing in the catalogue drives it: the device runs on the generic path.
        return check(
            area,
            name,
            title,
            Status::Warn,
            &format!(
                "{} {} no driver of {} own, so {} only at a basic level.",
                names.join(", "),
                plural(bad.len(), "has", "have"),
                plural(bad.len(), "its", "their"),
                plural(bad.len(), "it works", "they work")
            ),
        );
    }
    let mut detail = format!(
        "{} {} a driver: {}.",
        bad.len(),
        plural(bad.len(), "device needs", "devices need"),
        names.join(", ")
    );
    match later {
        Later::Nothing => {}
        Later::AtStart => detail.push_str(" Some of the drivers start the next time the computer starts."),
        Later::More(n) => detail.push_str(&format!(
            " The fix installs the first {MAX} drivers. Choose Check again afterwards for the other {n}. Some of the drivers start the next time the computer starts.",
            MAX = shared::shell::MAX_DOCTOR_STEPS
        )),
    }
    check(area, name, title, Status::Fail, &detail).with(Fix::admin("Install the drivers", reqs))
}

/// What a device fix leaves for later because one sheet asks at most
/// [`shared::shell::MAX_DOCTOR_STEPS`] requests.
#[derive(Debug, PartialEq, Eq)]
enum Later {
    Nothing,
    /// The module loads, left to the next start (`wol hw boot`).
    AtStart,
    /// The module loads, and this many packages for a second fix.
    More(usize),
}

/// The fix's wol-admind requests: module loads then package installs, each once; over the cap the
/// packages only (a package's module loads at the next start), cut at the cap.
fn device_steps(bad: &[&&shared::hardware::Device]) -> (Vec<AdminRequest>, Later) {
    let max = shared::shell::MAX_DOCTOR_STEPS;
    let mut klds: Vec<AdminRequest> = Vec::new();
    let mut pkgs: Vec<AdminRequest> = Vec::new();
    for d in bad {
        for k in &d.missing_klds {
            let r = req(AdminAction::HwLoad, &[k]);
            if !klds.contains(&r) {
                klds.push(r);
            }
        }
        for p in &d.missing_packages {
            let r = req(AdminAction::PkgInstall, &[p]);
            if !pkgs.contains(&r) {
                pkgs.push(r);
            }
        }
    }
    if klds.len() + pkgs.len() <= max {
        klds.extend(pkgs);
        return (klds, Later::Nothing);
    }
    if pkgs.is_empty() {
        // Only module loads, too many for one sheet: the first ones now, the rest at the next start.
        klds.truncate(max);
        return (klds, Later::AtStart);
    }
    if pkgs.len() <= max {
        return (pkgs, Later::AtStart);
    }
    let more = pkgs.len() - max;
    pkgs.truncate(max);
    (pkgs, Later::More(more))
}

// ---------------------------------------------------------------- run

/// The checks of one area; unknown areas give none. An area whose checks crash, or find nothing to
/// look at, is one row that says so ([`guarded`]): the report is never empty and never lost.
pub fn area_checks(env: &mut dyn Env, area: &str) -> Vec<Check> {
    // An on-ask area (the TLS probes, ADR 0055) runs only by name; an unknown one gives none.
    let Some(area) = AREAS.iter().chain(ON_ASK).find(|a| **a == area).copied() else {
        return Vec::new();
    };
    guarded(area, || match area {
        "sound" => sound(env),
        "games" => games(env),
        "updates" => updates(env),
        "security" => security(env),
        "storage" => storage(env),
        "network" => network(env),
        "power" => power(env),
        tls::AREA => tls::checks(env),
        "hardware" => hw::checks(env),
        _ => display(env),
    })
}

/// What a person calls an area (the Doctor app's sidebar words).
fn area_name(area: &str) -> &'static str {
    match area {
        "sound" => "Sound",
        "games" => "Games",
        "updates" => "Updates",
        "security" => "Security",
        "storage" => "Storage",
        "network" => "Network",
        "power" => "Power",
        "hardware" => "Hardware",
        tls::AREA => "Secure connections",
        _ => "Displays",
    }
}

/// Runs one area's checks so that a crash in one (a panic in a check or in what it reads) cannot
/// take the whole report with it: the area becomes a failed `<area>.check` row with the error text
/// and what to do, and the other areas still report. An area that returns nothing is a warning row
/// for the same reason: a report with nothing in it looks like "everything is fine".
fn guarded(area: &'static str, run: impl FnOnce() -> Vec<Check>) -> Vec<Check> {
    let name = area_name(area);
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(v) if !v.is_empty() => v,
        Ok(_) => {
            let mut c = check(
                area,
                "check",
                &format!("{name} checks"),
                Status::Warn,
                &format!("Doctor found nothing to look at in {name}. Choose Check again."),
            );
            c.error = Some("the area returned no checks".into());
            vec![c]
        }
        Err(payload) => {
            let why = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "it stopped without saying why".into());
            let mut c = check(
                area,
                "check",
                &format!("{name} checks"),
                Status::Fail,
                &format!(
                    "Doctor could not finish looking at {name}. It stopped with: {}. Choose Check again. If it stops again, restart the computer.",
                    why.trim().trim_end_matches('.')
                ),
            );
            c.error = Some(why);
            vec![c]
        }
    }
}

/// Every area in [`AREAS`] order, or the one asked for (an [`ON_ASK`] area only by name).
pub fn checks(env: &mut dyn Env, area: Option<&str>) -> Vec<Check> {
    if let Some(a) = area.filter(|a| ON_ASK.contains(a)) {
        return area_checks(env, a);
    }
    AREAS
        .iter()
        .filter(|a| area.is_none_or(|x| x == **a))
        .flat_map(|a| area_checks(env, a))
        .collect()
}

pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let json = inv.flag("json") || !env.stdout_tty();
    match inv.action {
        "doctor.check" => run(env, inv.get("area"), json),
        "doctor.fix" => fix(env, inv.get("id").unwrap_or_default(), json),
        other => fail(env, ErrorCode::Failed, &format!("no handler for {other}")),
    }
}

fn mark(s: Status) -> &'static str {
    match s {
        Status::Ok => "ok",
        Status::Warn => "warn",
        Status::Fail => "FAIL",
        Status::Skipped => "-",
    }
}

/// One line per check, its fix under it, then the verdict.
pub fn report(all: &[Check]) -> String {
    let mut t = String::new();
    let mut area = "";
    for c in all {
        if c.area != area {
            area = c.area;
            t.push_str(&format!("{area}\n"));
        }
        t.push_str(&format!(
            "  {:<5} {:<26} {}\n",
            mark(c.status),
            c.title,
            c.detail
        ));
        if let Some(f) = c.fix.as_ref().filter(|_| c.status != Status::Ok) {
            t.push_str(&format!(
                "{:<34}fix: wol doctor fix {} ({})\n",
                "", c.id, f.title
            ));
        }
    }
    let failed = all.iter().filter(|c| c.status == Status::Fail).count();
    let warned = all.iter().filter(|c| c.status == Status::Warn).count();
    t.push_str(&match (failed, warned) {
        (0, 0) => "Everything is fine.\n".to_string(),
        (0, w) => format!(
            "Everything works; {w} {} worth a look.\n",
            plural(w, "thing", "things")
        ),
        (f, _) => format!("{f} {} to fix.\n", plural(f, "problem", "problems")),
    });
    t
}

fn run(env: &mut dyn Env, area: Option<&str>, json: bool) -> i32 {
    let all = checks(env, area);
    let ok = all.iter().all(|c| c.status != Status::Fail);
    if json {
        let now = env.now_secs();
        emit_json(
            env,
            &serde_json::json!({ "checked": now, "ready": ok, "checks": all }),
        );
    } else {
        env.out(&report(&all));
    }
    if ok { 0 } else { ErrorCode::Failed.exit() }
}

/// The check `id` as it is now, when its area and id exist.
fn find(env: &mut dyn Env, id: &str) -> Option<Check> {
    let area = id.split('.').next().unwrap_or_default();
    if !AREAS.contains(&area) && !ON_ASK.contains(&area) {
        return None;
    }
    area_checks(env, area).into_iter().find(|c| c.id == id)
}

/// Whether `id` names a check (its area exists and has it).
pub fn exists(env: &mut dyn Env, id: &str) -> bool {
    find(env, id).is_some()
}

/// A fix worked when its commands did and the check is fine now: a check that was worth a look
/// and still is, or a fix that moved a failure to a warning, did not (QA-WEB-23: Doctor said
/// "Fixed." for a warning that stayed).
fn fixed(code: i32, after: Status) -> bool {
    code == 0 && after == Status::Ok
}

/// The on-screen approval for `wol doctor fix <id>` asked by a helper or a system web app (ADR
/// 0048): the fix's wol-admind requests as a [`ApprovalRequest::DoctorFix`] sheet the shell runs
/// after the password. `Ok(None)`: the fix runs as the person (their own services and settings),
/// so it is a consent sheet and `wol doctor fix` itself runs it after Allow. An error: nothing to
/// run.
///
/// [`ApprovalRequest::DoctorFix`]: shared::shell::ApprovalRequest::DoctorFix
pub fn approval(env: &mut dyn Env, id: &str) -> Result<Option<ApprovalRequest>, String> {
    let c = find(env, id).ok_or_else(|| format!("WolOS has no check {id}."))?;
    let Some(f) = c.fix.filter(|_| c.status != Status::Ok) else {
        return Err(format!("{}: nothing to fix. {}", c.title, c.detail));
    };
    let Action::Admin(reqs) = f.action else {
        return Ok(None);
    };
    let request = ApprovalRequest::DoctorFix {
        id: c.id,
        title: f.title,
        steps: steps_of(&reqs),
    };
    match request.doctor_steps() {
        Some(_) => Ok(Some(request)),
        None => Err(format!("{id}: this fix cannot be asked on screen.")),
    }
}

/// wol-admind requests as `[action, args..]` steps (the [`ApprovalRequest::DoctorFix`] shape).
fn steps_of(reqs: &[AdminRequest]) -> Vec<Vec<String>> {
    reqs.iter()
        .map(|r| {
            std::iter::once(r.action.name().to_string())
                .chain(r.args.iter().cloned())
                .collect()
        })
        .collect()
}

/// The steps check `c`'s fix would still send (none when it is fine or runs as the person).
fn steps_left(c: &Check) -> Vec<Vec<String>> {
    match c.fix.as_ref().filter(|_| c.status != Status::Ok) {
        Some(Fix {
            action: Action::Admin(reqs),
            ..
        }) => steps_of(reqs),
        _ => Vec::new(),
    }
}

/// `wol doctor fix --json`'s answer after the shell ran the fix (the same shape as [`fix`]'s).
pub fn after(env: &mut dyn Env, id: &str) -> serde_json::Value {
    let after = find(env, id);
    let ok = after.as_ref().is_some_and(|c| fixed(0, c.status));
    serde_json::json!({ "id": id, "ran": true, "ok": ok, "check": after })
}

/// What to do after a fix that did not work, for the person and for a web app: the same words.
const FIX_NEXT: &str = "Choose Check again to see how it stands now. If it keeps failing, restart the computer and try again.";

/// The answer of a fix that never started or did not work (`--json`; the same keys as a fix that
/// ran, so a script reads `ok`, `message` and `error` either way): never only an exit status.
fn fix_report(id: &str, ran: bool, exit: i32, error: &str, message: &str) -> serde_json::Value {
    let mut v = serde_json::json!({
        "id": id,
        "ran": ran,
        "ok": false,
        "exit": exit,
        "message": message,
        "next": FIX_NEXT,
    });
    if !error.is_empty() {
        v["error"] = error.into();
    }
    v
}

/// The last line a command printed: the sentence that says why it failed.
fn last_line(text: &str) -> String {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// The answer after the shell ran a fix that stopped at a step that did not work (QA-100 H3-2):
/// [`after`]'s shape with `ok: false`, the check as it is now (the row shows what the steps that
/// worked changed), and which step stopped it. `asked` are the steps the sheet showed;
/// a step that is no longer needed now counts as done, the first one still needed is the one that
/// did not work. `why` is the shell's sentence.
pub fn after_failed(
    env: &mut dyn Env,
    id: &str,
    asked: &[Vec<String>],
    why: &str,
) -> serde_json::Value {
    let after = find(env, id);
    let left = after.as_ref().map(steps_left).unwrap_or_default();
    let of = asked.len();
    let done = asked.iter().filter(|s| !left.contains(s)).count();
    let step = asked
        .iter()
        .position(|s| left.contains(s))
        .map_or(done.min(of.saturating_sub(1)) + 1, |i| i + 1);
    let why = why.trim().trim_end_matches('.');
    let message = match (of, why.is_empty()) {
        (0 | 1, true) => "The fix did not work.".to_string(),
        (0 | 1, false) => format!("The fix did not work: {why}."),
        (_, true) => format!("Step {step} of {of} did not work."),
        (_, false) => format!("Step {step} of {of} did not work: {why}."),
    };
    // One shape with `fix_report` (the CLI's answer when a fix does not work): exit, error and
    // next too, so an app reads the same keys from the sheet's path and from the CLI's.
    let mut v = fix_report(id, true, ErrorCode::Failed.exit(), why, &message);
    v["check"] = serde_json::json!(after);
    v["step"] = step.into();
    v["of"] = of.into();
    v["done"] = done.into();
    v
}

/// Run one check's fix, then check its area again. A helper or a web app (any actor but a person
/// at `wol`) never sends a wol-admind request from here: theirs went through [`approval`].
///
/// A fix that cannot start or does not work is a report, not a status: `--json` carries `message`
/// (what failed), `error` (the text the failing step gave), `next` (what to do), `exit` and the
/// check as it is now; the text report prints the same on stderr.
fn fix(env: &mut dyn Env, id: &str, json: bool) -> i32 {
    let area = id.split('.').next().unwrap_or_default();
    let found = (AREAS.contains(&area) || ON_ASK.contains(&area))
        .then(|| area_checks(env, area).into_iter().find(|c| c.id == id))
        .flatten();
    let Some(c) = found else {
        let code = fail(env, ErrorCode::NotFound, &format!("no check {id}"));
        if json {
            let v = fix_report(
                id,
                false,
                code,
                &format!("no check {id}"),
                &format!("WolOS has no check {id}."),
            );
            emit_json(env, &v);
        }
        return code;
    };
    let Some(f) = c.fix.clone().filter(|_| c.status != Status::Ok) else {
        if json {
            emit_json(
                env,
                &serde_json::json!({ "id": id, "ran": false, "ok": c.status == Status::Ok, "check": c }),
            );
        } else {
            env.out(&format!("{}: nothing to run. {}\n", c.title, c.detail));
        }
        return if c.status == Status::Ok {
            0
        } else {
            ErrorCode::Unsupported.exit()
        };
    };
    let mut said = String::new();
    let code = match &f.action {
        // The check changed between the sheet and now: ask again, never on a cached password.
        Action::Admin(_) if env.actor() != "cli" => {
            let why = format!(
                "{} needs the administrator now; ask again to see the password sheet",
                c.title
            );
            let code = fail(env, ErrorCode::Denied, &why);
            if json {
                emit_json(env, &fix_report(id, false, code, &why, &format!("{why}.")));
            }
            return code;
        }
        Action::Admin(reqs) => {
            let mut code = 0;
            for r in reqs {
                let c = admin_send_said(env, r, &mut said);
                if c != 0 {
                    code = c;
                    break;
                }
            }
            code
        }
        // Captured, so `--json` stays one document; what it printed is the reason it failed.
        Action::User(argv) => match env.exec_output(argv) {
            Ok((c, out)) => {
                if c != 0 {
                    said = last_line(&out);
                }
                c
            }
            Err(_) => match env.exec(argv) {
                Ok(c) => c,
                Err(_) => {
                    said = "The command could not be started.".into();
                    ErrorCode::Unavailable.exit()
                }
            },
        },
    };
    let after = area_checks(env, area)
        .into_iter()
        .find(|c| c.id == id)
        .unwrap_or(c);
    let fixed = fixed(code, after.status);
    let exit = if fixed {
        0
    } else if code != 0 {
        code
    } else {
        ErrorCode::Failed.exit()
    };
    let message = match (said.trim().trim_end_matches('.'), code) {
        ("", 0) => format!("{} ran, but it is still not fine.", f.title),
        ("", _) => "The fix did not work.".to_string(),
        (why, _) => format!("The fix did not work: {why}."),
    };
    if json {
        let mut v = serde_json::json!({ "id": id, "ran": true, "ok": fixed, "check": after });
        if !fixed {
            v = {
                let mut r = fix_report(id, true, exit, &said, &message);
                r["check"] = serde_json::json!(after);
                r
            };
        }
        emit_json(env, &v);
    } else if fixed {
        env.out(&format!("{}: {}\n", after.title, after.detail));
    } else {
        env.err(&format!(
            "wol: {} is not fixed: {}\nwol: {message} {FIX_NEXT}\n",
            after.title, after.detail
        ));
    }
    exit
}

#[cfg(test)]
mod tests {
    use super::*;

    /// QA-WEB-23: only a check that is fine after the fix was fixed.
    #[test]
    fn a_warning_that_stays_is_not_fixed() {
        assert!(fixed(0, Status::Ok));
        assert!(!fixed(0, Status::Warn), "still worth a look");
        assert!(!fixed(0, Status::Fail));
        assert!(!fixed(1, Status::Ok), "the commands failed");
        assert!(!fixed(0, Status::Skipped), "not checked is not fixed");
    }

    #[test]
    fn install_lines_become_one_admin_install_each() {
        let Some(Action::Admin(r)) = action_of("sudo pacman -S --needed vulkan-radeon lib32-mesa")
        else {
            panic!("no action");
        };
        assert_eq!(
            r,
            vec![
                req(AdminAction::PkgInstall, &["vulkan-radeon"]),
                req(AdminAction::PkgInstall, &["lib32-mesa"]),
            ]
        );
        let Some(Action::Admin(r)) = action_of("pacman -S rtkit; systemctl enable --now x") else {
            panic!("no action");
        };
        assert_eq!(r, vec![req(AdminAction::PkgInstall, &["rtkit"])]);
        let Some(Action::Admin(r)) =
            action_of("pacman -S wolos-system (reinstalls /usr/share/x fragments)")
        else {
            panic!("no action");
        };
        assert_eq!(r, vec![req(AdminAction::PkgInstall, &["wolos-system"])]);
    }

    #[test]
    fn user_lines_run_as_the_person_and_advice_runs_nothing() {
        assert_eq!(
            action_of("systemctl --user start wireplumber.service"),
            Some(Action::User(
                ["systemctl", "--user", "start", "wireplumber.service"]
                    .map(String::from)
                    .to_vec()
            ))
        );
        assert_eq!(
            action_of("wpctl set-volume 52 1.0000 (0 dB) or Settings > Sound > Input"),
            Some(Action::User(
                ["wpctl", "set-volume", "52", "1.0000"]
                    .map(String::from)
                    .to_vec()
            ))
        );
        for advice in [
            "",
            "plug in or pair a microphone",
            "enable [multilib] in /etc/pacman.conf, then sudo pacman -Sy",
            "systemctl start rtkit-daemon.service",
            "wpctl set-volume @DEFAULT@ 1; rm -rf /",
            "Settings > Sound > Pro Audio: a larger buffer",
        ] {
            assert_eq!(action_of(advice), None, "{advice}");
        }
    }

    /// Every wol-admind action a fix sends rides on the Doctor sheet (`shared::shell::DOCTOR_ACTIONS`).
    #[test]
    fn every_admin_action_of_a_fix_is_a_doctor_action() {
        let src = include_str!("doctor.rs");
        let mut n = 0;
        for part in src.split("req(AdminAction::").skip(1) {
            let name: String = part
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            if name.is_empty() {
                continue; // this test's own pattern
            }
            let a = AdminAction::ALL
                .into_iter()
                .find(|a| format!("{a:?}") == name)
                .unwrap_or_else(|| panic!("{name}"));
            assert!(shared::shell::DOCTOR_ACTIONS.contains(&a), "{name}");
            n += 1;
        }
        assert!(n >= 5, "{n}");
    }

    /// QA-DOC-2: a fix of three drivers whose second one did not work: the answer carries the
    /// check as it is now, which step stopped, and a sentence that says so.
    #[test]
    fn a_fix_that_stopped_midway_says_which_step() {
        use shared::hardware::{Device, HardwareSnapshot, Kind, State};
        let dev = |p: &str| Device {
            kind: Kind::Wifi,
            name: format!("Card {p}"),
            state: State::NeedsFirmware,
            missing_packages: vec![p.to_string()],
            ..Default::default()
        };
        // Now only b and c are still missing: a was installed before the second step stopped.
        let mut env = crate::run::FakeEnv {
            hw: vec![HardwareSnapshot {
                devices: vec![dev("b-fw"), dev("c-fw")],
            }],
            ..Default::default()
        };
        let step = |p: &str| vec!["pkg-install".to_string(), p.to_string()];
        let asked = vec![step("a-fw"), step("b-fw"), step("c-fw")];
        let v = after_failed(
            &mut env,
            "network.devices",
            &asked,
            "The password was wrong. ",
        );
        assert_eq!(v["ok"], false);
        assert_eq!(v["ran"], true);
        assert_eq!(
            (v["step"].as_u64(), v["of"].as_u64(), v["done"].as_u64()),
            (Some(2), Some(3), Some(1))
        );
        assert_eq!(
            v["message"],
            "Step 2 of 3 did not work: The password was wrong."
        );
        assert_eq!(
            v["check"]["id"], "network.devices",
            "the row as it is now: {v}"
        );
        // One shape with the CLI's answer (`fix_report`): an app reads exit, error and next from
        // the sheet's path exactly as from `wol doctor fix --json`.
        let cli = fix_report("network.devices", true, 1, "The password was wrong", "m");
        for key in cli.as_object().unwrap().keys() {
            assert!(v.get(key).is_some(), "after_failed lacks {key}: {v}");
        }
        assert_eq!(v["error"], "The password was wrong");
        assert_eq!(v["next"], FIX_NEXT);
        assert_eq!(v["exit"], ErrorCode::Failed.exit());
        // One step only, or no reason given.
        let v = after_failed(&mut env, "network.devices", &[step("b-fw")], "");
        assert_eq!(v["message"], "The fix did not work.");
        let v = after_failed(&mut env, "network.devices", &asked, "");
        assert_eq!(v["message"], "Step 2 of 3 did not work.");
    }

    #[test]
    fn report_points_at_doctor_fix() {
        let c = check("security", "firewall", "Firewall", Status::Fail, "off").with(Fix::admin(
            "Turn on the firewall",
            vec![req(AdminAction::Firewall, &["on"])],
        ));
        let t = report(&[c]);
        assert!(t.contains("fix: wol doctor fix security.firewall"), "{t}");
        assert!(t.ends_with("1 problem to fix.\n"), "{t}");
    }

    /// A helper or a web app never sends a wol-admind request from `wol doctor fix`; the refusal is
    /// an answer with its words in `--json` too, not only a status.
    #[test]
    fn a_fix_asked_by_a_helper_is_refused_with_words_in_json() {
        let mut env = crate::run::FakeEnv {
            captures: vec![(
                "systemctl show -p UnitFileState --value nftables.service".into(),
                "disabled\n".into(),
            )],
            vars: vec![("WOL_HELPER".into(), "test-helper".into())],
            ..Default::default()
        };
        let code = fix(&mut env, "security.firewall", true);
        assert_eq!(code, ErrorCode::Denied.exit());
        assert!(
            env.requests.is_empty(),
            "no wol-admind request from a helper"
        );
        let v: serde_json::Value = serde_json::from_str(&env.out).expect(&env.out);
        assert_eq!(
            (v["ran"].as_bool(), v["ok"].as_bool()),
            (Some(false), Some(false)),
            "{v}"
        );
        assert!(
            v["error"].as_str().unwrap().contains("administrator"),
            "{v}"
        );
        assert!(v["next"].is_string() && v["message"].is_string(), "{v}");
    }

    /// An area that returns nothing is a row that says so, never an empty report.
    #[test]
    fn an_area_with_no_checks_is_a_warning_row() {
        let rows = guarded("games", Vec::new);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].id.as_str(), rows[0].status),
            ("games.check", Status::Warn)
        );
        assert!(rows[0].detail.contains("Games"), "{}", rows[0].detail);
        let rows = guarded("games", || panic!("{}", String::from("the list was cut")));
        assert_eq!(
            (rows[0].status, rows[0].error.as_deref()),
            (Status::Fail, Some("the list was cut"))
        );
        let rows = guarded("power", || std::panic::panic_any(7u8));
        assert!(
            rows[0]
                .error
                .as_deref()
                .unwrap()
                .contains("without saying why")
        );
    }
}
