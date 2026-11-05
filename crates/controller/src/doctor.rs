// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol doctor [area]` and `wol doctor fix <id>` (docs/modules/doctor.md): every area of the
//! computer (sound, games, updates, security, storage, network, power, display, memory) as one list of
//! [`Check`]s in plain words, each with the fix it can run. The facts and rules stay where they
//! live (`audio_doctor`, `wol_core::games::doctor`, `wol_core::security`, `wol_core::storage`,
//! `wol_core::netinfo`, `wol_core::power`, `wol_core::hardware`); this module only words them for
//! a person and runs a fix through the path the owning verb uses (wol-admind for root changes).
use crate::cli::Invocation;
use crate::run::{Env, admin_send, emit_json, fail};
use shared::admin::{AdminAction, AdminRequest};
use shared::errors::ErrorCode;
use shared::hardware::{HardwareSnapshot, Kind};
use shared::shell::ApprovalRequest;
use std::cell::RefCell;
use std::path::Path;
use wol_core::games::doctor as games;

pub mod groups;

/// Every area, in the order the Doctor app lists them.
pub const AREAS: &[&str] = &[
    "sound", "games", "updates", "security", "storage", "network", "power", "display", "memory",
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
}

fn check(area: &'static str, name: &str, title: &str, status: Status, detail: &str) -> Check {
    Check {
        id: format!("{area}.{name}"),
        area,
        title: title.into(),
        status,
        detail: detail.into(),
        fix: None,
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
        "Sound devices",
        &[Kind::Audio, Kind::Webcam],
        "Every sound device and camera has its driver.",
        "No sound card or camera was found.",
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
                Status::Warn,
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
    v.push(devices(
        &env.hardware(),
        "display",
        "Graphics",
        &[Kind::Gpu, Kind::Backlight],
        "The graphics card and screen brightness have their drivers.",
        "No graphics card was found.",
    ));
    v
}

// ---------------------------------------------------------------- hardware

/// Devices of `kinds` that are not ready, with `wol hw fix --yes`'s requests as the fix.
fn devices(
    snap: &HardwareSnapshot,
    area: &'static str,
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
        return check(area, "devices", title, Status::Skipped, none);
    }
    if bad.is_empty() {
        return check(area, "devices", title, Status::Ok, ok);
    }
    let names: Vec<&str> = bad.iter().map(|d| d.name.as_str()).collect();
    let mut reqs: Vec<AdminRequest> = Vec::new();
    for d in &bad {
        for k in &d.missing_klds {
            reqs.push(req(AdminAction::HwLoad, &[k]));
        }
    }
    for d in &bad {
        for p in &d.missing_packages {
            let r = req(AdminAction::PkgInstall, &[p]);
            if !reqs.contains(&r) {
                reqs.push(r);
            }
        }
    }
    reqs.dedup();
    if reqs.is_empty() {
        // Nothing in the catalogue drives it: the device runs on the generic path.
        return check(
            area,
            "devices",
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
    check(
        area,
        "devices",
        title,
        Status::Fail,
        &format!(
            "{} {} a driver: {}.",
            bad.len(),
            plural(bad.len(), "device needs", "devices need"),
            names.join(", ")
        ),
    )
    .with(Fix::admin("Install the drivers", reqs))
}

// ---------------------------------------------------------------- memory

/// What the memory tuning for apps' engines is doing (ADR 0061, docs/modules/runtime-tuning.md):
/// file fault-around, sharing of identical memory (KSM) with the account rule, and Memory Saver's
/// page-out of apps with a budget. One row, `memory.saver`; the worst of the three is its status.
fn memory(env: &mut dyn Env) -> Vec<Check> {
    use wol_core::jsruntime::{self, KsmPolicy};
    let mut status = Status::Ok;
    let mut worse = |s: Status| {
        if matches!((status, s), (Status::Ok, Status::Warn | Status::Fail))
            || matches!((status, s), (Status::Warn, Status::Fail))
        {
            status = s;
        }
    };
    let mut parts: Vec<String> = Vec::new();

    // Fault-around: debugfs is root's; a person reads whether the boot step that writes it ran.
    let want = jsruntime::FAULT_AROUND_BYTES.to_string();
    let fault = env
        .read_file(Path::new(jsruntime::MM_WRITES[0].path))
        .map(|v| v.trim().to_owned());
    match fault {
        Some(v) if v == want => {
            parts.push("Apps map only the file pages they touch (4 KB at a time).".into())
        }
        Some(v) => {
            worse(Status::Warn);
            parts.push(format!(
                "Apps map file pages {v} bytes at a time instead of {want}, so each is charged more memory than it uses."
            ));
        }
        None => match env
            .capture(&[
                "systemctl",
                "show",
                "wolos-mm.service",
                "-p",
                "LoadState",
                "-p",
                "ActiveState",
                "-p",
                "Result",
            ])
            .as_deref()
            .map(unit_ran)
        {
            Some(Some(true)) => parts.push(
                "Apps map only the file pages they touch, as set when the computer started.".into(),
            ),
            Some(Some(false)) => {
                worse(Status::Warn);
                parts.push(
                    "The memory settings for apps weren’t applied when the computer started. Restart to apply them."
                        .into(),
                );
            }
            _ => {
                parts.push("The memory settings for apps aren’t installed on this computer.".into())
            }
        },
    }

    // KSM: what runs, and why, by the same rule wolos-mm applies at boot.
    let policy =
        jsruntime::ksm_policy(&env.read_file(Path::new("/etc/passwd")).unwrap_or_default());
    match env.read_file(Path::new(jsruntime::KSM_RUN)).map(|v| v.trim().to_owned()) {
        None => parts.push("This kernel can’t share identical memory between apps.".into()),
        Some(run) if run == policy.run_value() => parts.push(match policy {
            KsmPolicy::On => {
                let pages = env
                    .read_file(Path::new("/sys/kernel/mm/ksm/pages_sharing"))
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .unwrap_or(0);
                format!(
                    "Identical memory of apps is shared ({} now), because this computer has one account.",
                    wol_core::memsaver::size(pages * 4096)
                )
            }
            KsmPolicy::Off { accounts: 0 } => {
                "Identical memory isn’t shared: no account was found on this computer.".into()
            }
            KsmPolicy::Off { accounts } => format!(
                "Identical memory isn’t shared: this computer has {accounts} accounts, and sharing would let one person learn what another’s apps hold."
            ),
        }),
        Some(_) => {
            worse(Status::Warn);
            parts.push(match policy {
                KsmPolicy::On => "Identical memory isn’t shared yet. Restart to turn sharing on.".into(),
                KsmPolicy::Off { accounts } => format!(
                    "Identical memory is still shared though this computer now has {accounts} accounts. Restart to turn sharing off."
                ),
            });
        }
    }

    // Page-out: Memory Saver's own state, from the session's switch and wold's status file.
    let prefs = env
        .read_file(&wol_core::memsaver::path(&env.config_home()))
        .and_then(|t| serde_json::from_str::<wol_core::memsaver::Prefs>(&t).ok())
        .unwrap_or_default();
    let st = env
        .read_file(Path::new(shared::memsaver::STATUS))
        .and_then(|t| serde_json::from_str::<shared::memsaver::Status>(&t).ok());
    parts.push(pageout_line(prefs.enabled, st.as_ref()));
    if prefs.enabled && st.is_none() {
        worse(Status::Warn);
    }
    vec![check(
        "memory",
        "saver",
        "Memory Saver",
        status,
        &parts.join(" "),
    )]
}

/// `ActiveState=` and `Result=` of a oneshot: Some(true) it ran and succeeded, Some(false) it
/// failed or never ran, None when systemd does not know the unit.
fn unit_ran(show: &str) -> Option<bool> {
    let get = |k: &str| {
        show.lines()
            .find_map(|l| l.strip_prefix(k)?.strip_prefix('='))
            .map(str::trim)
    };
    if get("LoadState")? == "not-found" {
        return None;
    }
    Some(get("ActiveState")? == "active" && get("Result")? == "success")
}

/// The page-out part of `memory.saver`, in plain words: off, not running, or what each app with a
/// memory budget is going through.
fn pageout_line(enabled: bool, st: Option<&shared::memsaver::Status>) -> String {
    use shared::memsaver::AppState;
    if !enabled {
        return "Memory Saver is off, so apps keep their memory.".into();
    }
    let Some(st) = st else {
        return "Memory Saver is on but isn’t running yet.".into();
    };
    let budgeted: Vec<String> = st
        .apps
        .iter()
        .filter(|a| wol_core::memsaver::budget::of(&a.app).is_some())
        .map(|a| {
            let name = title_of(&a.app);
            match a.state {
                AppState::Saving => {
                    // The saver's own give-back, then one per page-out.
                    let n = a.pageouts.saturating_add(1);
                    let times = if n == 1 { "time" } else { "times" };
                    format!("{name} is in the background and gave its memory back {n} {times}.")
                }
                AppState::Playing => {
                    format!("{name} is in a call or playing sound, so it keeps its memory.")
                }
                AppState::Napping => format!("{name} is paused."),
                AppState::Excluded => format!("{name} always keeps its memory."),
                AppState::Waiting => format!("{name} went to the background a moment ago."),
                AppState::Active | AppState::Game => {
                    format!("{name} is in use and keeps to its limit.")
                }
            }
        })
        .collect();
    if budgeted.is_empty() {
        "Memory Saver is on. No app with a memory limit is open.".into()
    } else {
        format!("Memory Saver is on. {}", budgeted.join(" "))
    }
}

/// "discord" -> "Discord".
fn title_of(id: &str) -> String {
    let mut c = id.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------- run

/// The checks of one area; unknown areas give none.
pub fn area_checks(env: &mut dyn Env, area: &str) -> Vec<Check> {
    match area {
        "sound" => sound(env),
        "games" => games(env),
        "updates" => updates(env),
        "security" => security(env),
        "storage" => storage(env),
        "network" => network(env),
        "power" => power(env),
        "display" => display(env),
        "memory" => memory(env),
        _ => Vec::new(),
    }
}

/// Every area in [`AREAS`] order, or the one asked for.
pub fn checks(env: &mut dyn Env, area: Option<&str>) -> Vec<Check> {
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
    if !AREAS.contains(&area) {
        return None;
    }
    area_checks(env, area).into_iter().find(|c| c.id == id)
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
        steps: reqs
            .iter()
            .map(|r| {
                std::iter::once(r.action.name().to_string())
                    .chain(r.args.iter().cloned())
                    .collect()
            })
            .collect(),
    };
    match request.doctor_steps() {
        Some(_) => Ok(Some(request)),
        None => Err(format!("{id}: this fix cannot be asked on screen.")),
    }
}

/// `wol doctor fix --json`'s answer after the shell ran the fix (the same shape as [`fix`]'s).
pub fn after(env: &mut dyn Env, id: &str) -> serde_json::Value {
    let after = find(env, id);
    let ok = after.as_ref().is_some_and(|c| c.status != Status::Fail);
    serde_json::json!({ "id": id, "ran": true, "ok": ok, "check": after })
}

/// Run one check's fix, then check its area again. A helper or a web app (any actor but a person
/// at `wol`) never sends a wol-admind request from here: theirs went through [`approval`].
fn fix(env: &mut dyn Env, id: &str, json: bool) -> i32 {
    let area = id.split('.').next().unwrap_or_default();
    if !AREAS.contains(&area) {
        return fail(env, ErrorCode::NotFound, &format!("no check {id}"));
    }
    let Some(c) = area_checks(env, area).into_iter().find(|c| c.id == id) else {
        return fail(env, ErrorCode::NotFound, &format!("no check {id}"));
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
    let code = match &f.action {
        // The check changed between the sheet and now: ask again, never on a cached password.
        Action::Admin(_) if env.actor() != "cli" => {
            return fail(
                env,
                ErrorCode::Denied,
                &format!(
                    "{} needs the administrator now; ask again to see the password sheet",
                    c.title
                ),
            );
        }
        Action::Admin(reqs) => {
            let mut code = 0;
            for r in reqs {
                let c = admin_send(env, r);
                if c != 0 {
                    code = c;
                    break;
                }
            }
            code
        }
        // Captured, so `--json` stays one document.
        Action::User(argv) => match env.exec_output(argv) {
            Ok((c, _)) => c,
            Err(_) => match env.exec(argv) {
                Ok(c) => c,
                Err(_) => ErrorCode::Unavailable.exit(),
            },
        },
    };
    let after = area_checks(env, area)
        .into_iter()
        .find(|c| c.id == id)
        .unwrap_or(c);
    let fixed = code == 0 && after.status != Status::Fail;
    if json {
        emit_json(
            env,
            &serde_json::json!({ "id": id, "ran": true, "ok": fixed, "check": after }),
        );
    } else if fixed {
        env.out(&format!("{}: {}\n", after.title, after.detail));
    } else {
        env.err(&format!(
            "wol: {} is not fixed: {}\n",
            after.title, after.detail
        ));
    }
    if fixed {
        0
    } else if code != 0 {
        code
    } else {
        ErrorCode::Failed.exit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
