// SPDX-License-Identifier: BSD-2-Clause
//! wol-admind allow-list (IaraOS ADR 0026): validation, fixed argv / whole-file steps, policy. Pure.
use crate::settings::{appearance, idle, keybinds, timezones, users};
use crate::{bluetooth, power, printers, sharing};
use crate::{flatpak, wifi};
use shared::admin::{AdminAction, AdminRequest};
use std::collections::HashMap;

pub const MAX_IDLE_SECS: u32 = 86_400;
pub const MAX_FULL_NAME: usize = 64;

/// One step: a fixed argv (never a shell) or a whole-file write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdminStep {
    Run(Vec<String>),
    /// Fixed argv with a secret on stdin (never argv): e.g. `chpasswd`.
    RunInput {
        argv: Vec<String>,
        input: String,
    },
    /// `content` is the full new file text; `current` is what the daemon read (for merges like wpa_supplicant.conf).
    Write {
        path: String,
        mode: u32,
        content: WriteContent,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteContent {
    Fixed(String),
    /// Merge Wi-Fi network into the current wpa_supplicant.conf.
    WifiUpsert {
        ssid: String,
        password: Option<String>,
        hidden: bool,
    },
    /// Merge `key=value` lines into the current [`power::SYSCTL_LOCAL`] (allow-listed keys only).
    SysctlUpsert(Vec<(String, String)>),
    /// Drop one Wi-Fi network from the current wpa_supplicant.conf.
    WifiRemove {
        ssid: String,
    },
    /// Add or drop one grant in the compositor consent database.
    Consent {
        app: String,
        capability: String,
        allow: bool,
    },
}

impl WriteContent {
    pub fn render(&self, current: Option<&str>) -> String {
        match self {
            Self::Fixed(s) => s.clone(),
            Self::WifiUpsert {
                ssid,
                password,
                hidden,
            } => wifi::upsert_network_with(current, ssid, password.as_deref(), *hidden),
            Self::SysctlUpsert(kv) => kv
                .iter()
                .filter(|(k, v)| power::is_allowed_sysctl(k, v))
                .fold(current.unwrap_or("").to_owned(), |conf, (k, v)| {
                    power::upsert_sysctl(Some(&conf), k, v)
                }),
            Self::WifiRemove { ssid } => wifi::remove_network(current, ssid),
            Self::Consent {
                app,
                capability,
                allow,
            } => crate::consent::set(current, app, capability, *allow),
        }
    }
}

fn on_off(s: &str) -> bool {
    s == "on" || s == "off"
}
fn digits_after(s: &str, prefix: &str, min: usize, max: usize) -> bool {
    s.len() > min
        && s.len() < max
        && s.starts_with(prefix)
        && s[prefix.len()..].bytes().all(|b| b.is_ascii_digit())
}
/// A network interface name as the kernel accepts it (`dev_valid_name`):
/// 1..=15 bytes, not `.`/`..`, no `/`, `:` or whitespace. Control bytes and a
/// leading `-` are refused too, so the name can never read as an option.
fn iface(s: &str) -> bool {
    (1..=15).contains(&s.len())
        && s != "."
        && s != ".."
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_graphic() && b != b'/' && b != b':')
}
fn pcm(s: &str) -> bool {
    digits_after(s, "pcm", 3, 7)
}

fn binds(json: &str) -> Option<String> {
    match keybinds::parse(json) {
        None => Some("keybinds not JSON".into()),
        Some(b)
            if b.keys().all(|k| keybinds::is_valid_combo(k))
                && b.values().all(|v| !v.is_empty()) =>
        {
            None
        }
        Some(_) => Some("bad keybinds".into()),
    }
}

/// NetworkManager join (the image ships NetworkManager): the password goes on stdin, never argv.
fn nm_join(ssid: &str, password: Option<&str>, hidden: bool) -> AdminStep {
    let mut v = vec!["nmcli", "dev", "wifi", "connect", ssid];
    if hidden {
        v.extend(["hidden", "yes"]);
    }
    match password {
        Some(p) => {
            v.extend(["passwd-file", "/dev/stdin"]);
            AdminStep::RunInput {
                argv: v.iter().map(|s| (*s).to_owned()).collect(),
                input: format!("802-11-wireless-security.psk:{p}\n"),
            }
        }
        None => argv(&v),
    }
}

/// Error text, or None when allowed.
pub fn validate(r: &AdminRequest) -> Option<String> {
    let a: Vec<&str> = r.args.iter().map(String::as_str).collect();
    let n = a.len();
    let invalid = || Some(format!("invalid {} request", r.action.name()));
    let ok = |c: bool| if c { None } else { invalid() };
    match r.action {
        AdminAction::WifiJoin if (n == 2 || n == 3) && iface(a[0]) => {
            wifi::validate(a[1], a.get(2).copied()).map(String::from)
        }
        // Hidden network: `<iface> <ssid> <password or empty> hidden`.
        AdminAction::WifiJoin if n == 4 && iface(a[0]) && a[3] == "hidden" => {
            wifi::validate(a[1], Some(a[2]).filter(|p| !p.is_empty())).map(String::from)
        }
        AdminAction::WifiPower => ok(n == 2 && iface(a[0]) && on_off(a[1])),
        AdminAction::Bluetooth => ok(n == 1 && on_off(a[0])),
        AdminAction::AudioDefault => ok(n == 1 && pcm(a[0])),
        AdminAction::Appearance => {
            ok(n == 1 && (a[0] == appearance::LIGHT || a[0] == appearance::DARK))
        }
        AdminAction::Keybinds if n == 1 => binds(a[0]),
        AdminAction::Idle => ok(n == 1
            && (1..7).contains(&a[0].len())
            && a[0].bytes().all(|b| b.is_ascii_digit())
            && a[0].parse::<u32>().is_ok_and(|v| v <= MAX_IDLE_SECS)),
        // Audit S4-6: a service or WolOS name (`live`, `git`, `systemd-*`) is never a login account.
        AdminAction::UserAdd => ok(n == 4
            && users::is_valid_name(a[0])
            && !users::is_reserved(a[0])
            && a[1].chars().count() <= MAX_FULL_NAME
            && (a[2] == "yes" || a[2] == "no")
            && users::is_valid_password(a[3])),
        AdminAction::UserRemove => ok(n == 1 && users::is_valid_name(a[0]) && a[0] != "root"),
        AdminAction::UserPassword => ok(n == 2
            && users::is_valid_name(a[0])
            && a[0] != "root"
            && users::is_valid_password(a[1])),
        AdminAction::TimeZone => ok(n == 1 && timezones::is_valid(a[0])),
        // Audit S2: optional holds, the packages a full update leaves where they are (pins).
        AdminAction::PkgUpgrade => {
            ok(n <= shared::packages::MAX_NAMES && a.iter().all(|x| shared::packages::name_ok(x)))
        }
        AdminAction::PkgRefresh
        | AdminAction::Revoke
        | AdminAction::AurOpen
        | AdminAction::BootEnroll => ok(n == 0),
        AdminAction::PkgInstall => ok(n == 1 && crate::store::install_plan(a[0]).is_some()),
        AdminAction::PkgRemove => ok(n == 1 && crate::store::remove_plan(a[0]).is_some()),
        AdminAction::PowerMode => ok(n == 1 && power::policy::Mode::parse(a[0]).is_some()),
        AdminAction::BootEnvironment => ok(n == 1 && crate::update::is_valid_be(a[0])),
        AdminAction::HwLoad => ok(n == 1 && crate::hardware::is_catalogue_kld(a[0])),
        AdminAction::WolInstall => {
            ok(n == 2 && is_wol_path(a[0]) && (a[1] == "signed" || a[1] == "unsigned"))
        }
        AdminAction::WolRemove => ok(n == 1 && shared::package::is_app_id(a[0])),
        AdminAction::Ntp
        | AdminAction::RemoteLogin
        | AdminAction::Firewall
        | AdminAction::MalwareSignatures
        | AdminAction::AirPlayReceiver => ok(n == 1 && on_off(a[0])),
        AdminAction::Hostname => ok(n == 1 && sharing::is_valid_hostname(a[0])),
        AdminAction::Lid => ok(n == 1 && (a[0] == "sleep" || a[0] == "nothing")),
        AdminAction::AutoUpdate => ok(n == 1 && on_off(a[0])),
        AdminAction::PrinterAdd => {
            ok(n == 2 && printers::is_valid_name(a[0]) && printers::is_valid_uri(a[1]))
        }
        AdminAction::PrinterRemove | AdminAction::PrinterDefault => {
            ok(n == 1 && printers::is_valid_name(a[0]))
        }
        // Not a grant (the compositor's rules are static, the portal asks): say so, not "invalid".
        AdminAction::Consent if n == 3 && crate::consent::refusal(a[1]).is_some() => {
            crate::consent::refusal(a[1]).map(String::from)
        }
        AdminAction::Consent => ok(n == 3
            && crate::consent::is_valid_grant(a[0], a[1])
            && (a[2] == "allow" || a[2] == "deny")),
        AdminAction::BtPair => ok(n == 2 && bluetooth::is_valid_addr(a[0]) && bt_name(a[1])),
        AdminAction::BtForget => ok(n == 1 && bluetooth::is_valid_addr(a[0])),
        AdminAction::WifiForget => {
            ok(n == 2 && iface(a[0]) && wifi::validate(a[1], None).is_none())
        }
        AdminAction::SessionPower => ok(n == 1 && session_power_argv(a[0]).is_some()),
        AdminAction::FlatpakOverride => ok(flatpak::is_app_id(a.first().copied().unwrap_or(""))
            && match &a[1..] {
                ["reset"] => true,
                [perm, v] => {
                    flatpak::Perm::parse(perm).is_some() && (*v == "allow" || *v == "deny")
                }
                _ => false,
            }),
        AdminAction::FlatpakInstall | AdminAction::FlatpakRemove => {
            ok(n == 1 && flatpak::is_app_id(a[0]))
        }
        AdminAction::PkgInstallFile => ok(n == 2 && is_staged_package(a[0]) && is_sha256_hex(a[1])),
        AdminAction::SystemRollback => ok(n == 0),
        AdminAction::SystemChannel => ok(n == 1 && shared::packages::CHANNELS.contains(&a[0])),
        _ => invalid(),
    }
}

fn argv(v: &[&str]) -> AdminStep {
    AdminStep::Run(v.iter().map(|s| (*s).to_owned()).collect())
}

fn write(path: &str, mode: u32, s: String) -> AdminStep {
    AdminStep::Write {
        path: path.into(),
        mode,
        content: WriteContent::Fixed(s),
    }
}

/// `sysctl hw.snd.default_unit=N` for `pcmN`.
pub fn audio_default_argv(pcm: &str) -> Vec<String> {
    vec![
        "sysctl".into(),
        format!("hw.snd.default_unit={}", pcm.trim_start_matches("pcm")),
    ]
}

/// Power mode: save it for `wol power apply` (AC changes, boot, resume), then run the policy
/// plan for this machine and power source now (`power::policy`). Nothing steers the CPU (no
/// power-profiles-daemon): Err, the switch is hidden there.
pub fn power_steps(
    mode: power::policy::Mode,
    f: &power::policy::Facts,
) -> Result<Vec<AdminStep>, String> {
    if f.cpu == power::policy::Cpu::Fixed {
        return Err("power-mode: power-profiles-daemon is not running".into());
    }
    let mut steps = vec![write(
        power::policy::STATE_FILE,
        0o644,
        power::policy::State { mode }.to_json(),
    )];
    steps.extend(
        power::policy::plan(f, mode)
            .argv()
            .into_iter()
            .map(AdminStep::Run),
    );
    Ok(steps)
}

/// The actions whose steps run for minutes (pacman, Flatpak, `.wol` installs, Undo Update): one
/// table that wol-admind's runner and every client's reply wait read, so both sides agree (S4-4).
pub const LONG_ACTIONS: [AdminAction; 9] = [
    AdminAction::PkgUpgrade,
    AdminAction::PkgInstallFile,
    AdminAction::SystemRollback,
    AdminAction::PkgInstall,
    AdminAction::PkgRemove,
    AdminAction::FlatpakInstall,
    AdminAction::FlatpakRemove,
    AdminAction::WolInstall,
    AdminAction::WolRemove,
];
/// How long wol-admind lets one step of a [`LONG_ACTIONS`] action run, in seconds.
pub const LONG_STEP_SECS: u32 = 1800;
/// How long wol-admind lets one step of any other action run, in seconds.
pub const STEP_SECS: u32 = 30;
/// What a client adds to the steps' time for the password check, copies and the reply itself.
pub const REPLY_MARGIN_SECS: u32 = 60;

/// The longest one wol-admind step of `a` may run (seconds): [`LONG_STEP_SECS`] or [`STEP_SECS`].
pub fn step_secs(a: AdminAction) -> u32 {
    if LONG_ACTIONS.contains(&a) {
        LONG_STEP_SECS
    } else {
        STEP_SECS
    }
}

/// How long a client waits for wol-admind's reply to `r` (seconds): every planned step at its
/// [`step_secs`] plus [`REPLY_MARGIN_SECS`]. An action the daemon plans itself (`.wol` installs)
/// or an invalid request counts as one step, so the wait is never shorter than the daemon's.
pub fn reply_secs(r: &AdminRequest) -> u32 {
    let steps = plan(r).map_or(1, |s| s.len().max(1));
    let steps = u32::try_from(steps).unwrap_or(u32::MAX);
    steps
        .saturating_mul(step_secs(r.action))
        .saturating_add(REPLY_MARGIN_SECS)
}

/// [`plan_with`] without facts (actions that need none).
pub fn plan(r: &AdminRequest) -> Result<Vec<AdminStep>, String> {
    plan_with(r, &|_| None)
}

/// Steps for a valid request. Revoke has none (the daemon handles it). `probe` runs read-only
/// fixed argv for facts some actions need (stdout, None when it failed); planning stays pure.
pub fn plan_with(
    r: &AdminRequest,
    probe: &dyn Fn(&[&str]) -> Option<String>,
) -> Result<Vec<AdminStep>, String> {
    if let Some(e) = validate(r) {
        return Err(e);
    }
    let a = &r.args;
    Ok(match r.action {
        AdminAction::WifiJoin => vec![nm_join(
            &a[1],
            a.get(2).filter(|p| !p.is_empty()).map(String::as_str),
            a.get(3).is_some_and(|h| h == "hidden"),
        )],
        AdminAction::WifiPower => vec![argv(&["nmcli", "radio", "wifi", &a[1]])],
        AdminAction::Bluetooth => {
            vec![argv(&[
                "service",
                "bluetooth",
                if a[0] == "on" { "onestart" } else { "onestop" },
                "ubt0",
            ])]
        }
        AdminAction::AudioDefault => vec![AdminStep::Run(audio_default_argv(&a[0]))],
        AdminAction::Appearance => vec![write(appearance::FILE, 0o644, a[0].clone())],
        AdminAction::Keybinds => {
            vec![write(
                keybinds::FILE,
                0o644,
                keybinds::to_json(&keybinds::parse(&a[0]).unwrap_or_default()),
            )]
        }
        AdminAction::Idle => vec![write(
            idle::FILE,
            0o644,
            idle::to_json(a[0].parse().unwrap_or(0)),
        )],
        AdminAction::UserAdd => vec![
            AdminStep::Run(users::add_argv(&a[0], &a[1], a[2] == "yes").unwrap_or_default()),
            AdminStep::RunInput {
                argv: users::password_argv(&a[0]).unwrap_or_default(),
                input: format!("{}:{}\n", a[0], a[3]),
            },
        ],
        AdminAction::UserRemove => vec![AdminStep::Run(
            users::remove_argv(&a[0]).unwrap_or_default(),
        )],
        // The account first; then the disk's password slot follows it (only for the account the
        // installer named; `wol encrypt password` never fails the change).
        AdminAction::UserPassword => vec![
            AdminStep::RunInput {
                argv: users::password_argv(&a[0]).unwrap_or_default(),
                input: format!("{}:{}\n", a[0], a[1]),
            },
            AdminStep::RunInput {
                argv: ["wol", "encrypt", "password", &a[0]]
                    .map(String::from)
                    .to_vec(),
                input: format!("{}\n", a[1]),
            },
        ],
        AdminAction::TimeZone => vec![AdminStep::Run(timezones::set_argv(&a[0]))],
        AdminAction::PkgRefresh => vec![argv(&["pacman", "-Sy"])],
        // Inside the pinned release only `[wolos]` moves; a new release is a new root slot.
        // Holds (validated package names) stay at their version: `--ignore=a,b`.
        AdminAction::PkgUpgrade if a.is_empty() => vec![argv(&["pacman", "-Syu", "--noconfirm"])],
        AdminAction::PkgUpgrade => {
            let hold = format!("--ignore={}", a.join(","));
            vec![argv(&["pacman", "-Syu", "--noconfirm", &hold])]
        }
        // Revoke: the daemon handles it. Packages: verified and planned by the daemon itself.
        AdminAction::Revoke
        | AdminAction::AurOpen
        | AdminAction::WolInstall
        | AdminAction::WolRemove => vec![],
        AdminAction::PowerMode => power_steps(
            power::policy::Mode::parse(&a[0]).unwrap_or_default(),
            &power::policy::facts(probe),
        )?,
        AdminAction::PkgInstall => crate::store::install_plan(&a[0])
            .unwrap_or_default()
            .into_iter()
            .map(AdminStep::Run)
            .collect(),
        AdminAction::PkgRemove => crate::store::remove_plan(&a[0])
            .unwrap_or_default()
            .into_iter()
            .map(AdminStep::Run)
            .collect(),
        AdminAction::BootEnvironment => {
            vec![AdminStep::Run(crate::update::be::activate_argv(&a[0]))]
        }
        AdminAction::HwLoad => vec![argv(&["modprobe", &a[0]])],
        // Queues the key once (prints `enrolled` and changes nothing when it is already in db).
        AdminAction::BootEnroll => vec![argv(&["wol", "boot", "enroll"])],
        AdminAction::Ntp => vec![argv(&[
            "timedatectl",
            "set-ntp",
            if a[0] == "on" { "true" } else { "false" },
        ])],
        AdminAction::RemoteLogin => service_steps("sshd.service", a[0] == "on"),
        AdminAction::Firewall => service_steps("nftables.service", a[0] == "on"),
        AdminAction::MalwareSignatures => crate::malware::clam_argvs(a[0] == "on")
            .into_iter()
            .map(AdminStep::Run)
            .collect(),
        AdminAction::AirPlayReceiver => service_steps(crate::sharing::AIRPLAY_UNIT, a[0] == "on"),
        AdminAction::Hostname => vec![argv(&["hostnamectl", "set-hostname", &a[0]])],
        AdminAction::AutoUpdate => vec![write(
            crate::update::auto::FILE,
            0o644,
            crate::update::auto::conf(a[0] == "on"),
        )],
        AdminAction::Lid => {
            let l = if a[0] == "sleep" {
                power::Lid::Sleep
            } else {
                power::Lid::Nothing
            };
            vec![
                write(power::LID_CONF, 0o644, power::lid_conf(l)),
                argv(&power::LOGIND_RELOAD_ARGV),
            ]
        }
        AdminAction::PrinterAdd => vec![AdminStep::Run(
            printers::add_argv(&a[0], &a[1]).unwrap_or_default(),
        )],
        AdminAction::PrinterRemove => vec![AdminStep::Run(
            printers::remove_argv(&a[0]).unwrap_or_default(),
        )],
        AdminAction::PrinterDefault => vec![AdminStep::Run(
            printers::default_argv(&a[0]).unwrap_or_default(),
        )],
        AdminAction::Consent => vec![AdminStep::Write {
            path: crate::consent::FILE.into(),
            mode: 0o644,
            content: WriteContent::Consent {
                app: a[0].clone(),
                capability: a[1].clone(),
                allow: a[2] == "allow",
            },
        }],
        AdminAction::BtPair => bluetooth::pair_argvs(&a[0])
            .into_iter()
            .flatten()
            .map(|s| argv(&s))
            .collect(),
        AdminAction::BtForget => bluetooth::forget_argv(&a[0])
            .map(|s| argv(&s))
            .into_iter()
            .collect(),
        AdminAction::WifiForget => vec![argv(&["nmcli", "connection", "delete", "id", &a[1]])],
        AdminAction::SessionPower => {
            vec![argv(session_power_argv(&a[0]).unwrap_or_default())]
        }
        AdminAction::FlatpakOverride => vec![AdminStep::Run(match a[1].as_str() {
            "reset" => flatpak::reset_argv(flatpak::Scope::System, &a[0]),
            perm => flatpak::Perm::parse(perm)
                .map(|p| p.override_argv(flatpak::Scope::System, &a[0], a[2] == "allow"))
                .unwrap_or_default(),
        })],
        AdminAction::FlatpakInstall => vec![argv(&[
            "flatpak",
            "install",
            "--system",
            "--noninteractive",
            "-y",
            flatpak::REMOTE,
            &a[0],
        ])],
        AdminAction::FlatpakRemove => vec![argv(&[
            "flatpak",
            "uninstall",
            "--system",
            "--noninteractive",
            "-y",
            &a[0],
        ])],
        // ADR 0034: the sealed-system pacman on wold's root-only copy, never the caller's file.
        AdminAction::PkgInstallFile => vec![argv(&[
            "wol",
            "pkg",
            "--",
            "-U",
            "--noconfirm",
            "--",
            &a[0],
        ])],
        AdminAction::SystemRollback => vec![argv(&["wol", "update", "rollback"])],
        AdminAction::SystemChannel => {
            vec![write(CHANNEL_FILE, 0o644, format!("{}\n", a[0]))]
        }
    })
}

/// Session menu power verbs (logind locks every session before sleep).
pub fn session_power_argv(verb: &str) -> Option<&'static [&'static str]> {
    Some(match verb {
        "sleep" => &["systemctl", "suspend"],
        "restart" => &["systemctl", "reboot"],
        "shutdown" => &["systemctl", "poweroff"],
        _ => return None,
    })
}

/// Enable and start (or disable and stop) a systemd unit; both are no-ops when already so.
fn service_steps(unit: &str, on: bool) -> Vec<AdminStep> {
    vec![argv(&[
        "systemctl",
        if on { "enable" } else { "disable" },
        "--now",
        unit,
    ])]
}

/// Bluetooth device name for hcsecd.conf: 1..=64 characters that cannot end its quoted string or block.
fn bt_name(s: &str) -> bool {
    (1..=64).contains(&s.chars().count())
        && !s
            .chars()
            .any(|c| c.is_control() || matches!(c, '"' | '\\' | '{' | '}' | ';' | '#'))
}

/// Device URI without `user:password@` (logs).
fn without_userinfo(uri: &str) -> String {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return uri.into();
    };
    let host_end = rest.find('/').unwrap_or(rest.len());
    match rest[..host_end].rfind('@') {
        Some(at) => format!("{scheme}://{}", &rest[at + 1..]),
        None => uri.into(),
    }
}

/// Everyday changes the person at the computer makes without a password, like on a Mac: power
/// mode, Wi-Fi, Bluetooth, sleep, restart, shut down. wol-admind still logs each one.
pub fn is_user_level(a: AdminAction) -> bool {
    use AdminAction as A;
    matches!(
        a,
        A::PowerMode
            | A::PkgRefresh
            | A::WifiJoin
            | A::WifiPower
            | A::WifiForget
            | A::Bluetooth
            | A::BtPair
            | A::BtForget
            | A::SessionPower
    )
}

/// [`is_user_level`] for `uid`, given the accounts with a desktop session (`sessions`): only from
/// one of them, and restart or shut down only when no other account would lose its session
/// ([`signed_in_others`] names who is in the way, for the refusal).
pub fn user_level_ok(r: &AdminRequest, uid: u32, sessions: &[u32]) -> bool {
    is_user_level(r.action)
        && sessions.contains(&uid)
        && signed_in_others(r, uid, sessions).is_empty()
}

/// The other accounts (uids, in `sessions` order, once each) whose session `r` would end: a
/// restart or shut down asked by `uid` while they are signed in. Empty for sleep and every other
/// action. This is why the lock screen's Restart is refused (audit6 S6-5).
pub fn signed_in_others(r: &AdminRequest, uid: u32, sessions: &[u32]) -> Vec<u32> {
    let ends_sessions =
        r.action == AdminAction::SessionPower && r.args.first().is_some_and(|v| v != "sleep");
    let mut others = Vec::new();
    if ends_sessions {
        for s in sessions {
            if *s != uid && !others.contains(s) {
                others.push(*s);
            }
        }
    }
    others
}

/// When an administrator request may use the session's cached authorization ([`AuthCache`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fresh {
    /// The session's recent password is enough.
    Cached,
    /// The password is typed for this request, every time.
    Always,
    /// Fresh only for an unsigned `.wol` (`<path> unsigned`).
    WhenUnsigned,
}

/// The fresh-password matrix (audit A8/B9): every [`AdminAction`] has one row. Accounts, remote
/// access, the firewall, app consent, Flatpak permissions, what boots next and Secure Boot keys
/// are changes a borrowed unlocked session must not make without the password. An action missing
/// here is treated as [`Fresh::Always`]; `every_action_is_classified` fails the build instead.
pub const FRESH_PASSWORD: &[(AdminAction, Fresh)] = {
    use AdminAction as A;
    use Fresh::{Always, Cached, WhenUnsigned};
    &[
        (A::WifiJoin, Cached),
        (A::WifiPower, Cached),
        (A::Bluetooth, Cached),
        (A::AudioDefault, Cached),
        (A::Appearance, Cached),
        (A::Keybinds, Cached),
        (A::Idle, Cached),
        (A::UserAdd, Always),
        (A::UserRemove, Always),
        (A::UserPassword, Always),
        (A::TimeZone, Cached),
        (A::PkgUpgrade, Cached),
        (A::PkgRefresh, Cached),
        (A::Revoke, Cached),
        (A::PkgInstall, Cached),
        (A::PkgRemove, Cached),
        (A::AurOpen, Always),
        (A::PowerMode, Cached),
        (A::BootEnvironment, Always),
        (A::HwLoad, Cached),
        (A::WolInstall, WhenUnsigned),
        (A::WolRemove, Cached),
        (A::Ntp, Cached),
        (A::Hostname, Cached),
        (A::RemoteLogin, Always),
        (A::AirPlayReceiver, Cached),
        (A::Firewall, Always),
        (A::MalwareSignatures, Cached),
        (A::Lid, Cached),
        (A::PrinterAdd, Cached),
        (A::PrinterRemove, Cached),
        (A::PrinterDefault, Cached),
        (A::Consent, Always),
        (A::BtPair, Cached),
        (A::BtForget, Cached),
        (A::WifiForget, Cached),
        (A::SessionPower, Cached),
        (A::FlatpakOverride, Always),
        (A::FlatpakInstall, Cached),
        (A::FlatpakRemove, Cached),
        (A::AutoUpdate, Cached),
        (A::BootEnroll, Always),
        // Audit C7-1: unreviewed code as root, an older release, another update stream.
        (A::PkgInstallFile, Always),
        (A::SystemRollback, Always),
        (A::SystemChannel, Always),
    ]
};

/// The row of `a` in [`FRESH_PASSWORD`]; [`Fresh::Always`] when it has none.
pub fn fresh_rule(a: AdminAction) -> Fresh {
    FRESH_PASSWORD
        .iter()
        .find(|(x, _)| *x == a)
        .map_or(Fresh::Always, |(_, f)| *f)
}

/// True when `r` must be approved with a password typed for it, never the cache
/// ([`FRESH_PASSWORD`]).
pub fn needs_fresh_password(r: &AdminRequest) -> bool {
    match fresh_rule(r.action) {
        Fresh::Cached => false,
        Fresh::Always => true,
        Fresh::WhenUnsigned => r.args.get(1).is_some_and(|t| t == "unsigned"),
    }
}

/// The system update channel wopkg and `wol update` read (`stable`, `beta` or `nightly`).
pub const CHANNEL_FILE: &str = "/etc/wolos/channel";

/// wold's staged copy of a locally built package (ADR 0034):
/// `<STAGING_DIR>/<32 hex digits>/<file>.pkg.tar.zst`, one fresh `0700` root directory per call,
/// so nothing but root can have put a file there. wol-admind's own `.wol` staging directories under
/// the same root carry dashes and never match.
pub fn is_staged_package(p: &str) -> bool {
    let Some(rest) = p
        .strip_prefix(shared::package::STAGING_DIR)
        .and_then(|r| r.strip_prefix('/'))
    else {
        return false;
    };
    let Some((dir, file)) = rest.split_once('/') else {
        return false;
    };
    dir.len() == 32
        && dir.bytes().all(|b| b.is_ascii_hexdigit())
        && !file.contains('/')
        && !file.starts_with(['-', '.'])
        && shared::packages::file_ok(p)
}

/// A SHA-256 as the sheet showed it (ADR 0034): 64 lowercase hex digits.
pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Absolute `.wol` path without `..` or control characters.
fn is_wol_path(p: &str) -> bool {
    p.starts_with('/')
        && p.len() <= 1024
        && p.ends_with(".wol")
        && !p.split('/').any(|c| c == "..")
        && !p.chars().any(char::is_control)
}

/// Log text without secrets (Wi-Fi password and keybind JSON left out).
pub fn describe(r: &AdminRequest) -> String {
    let name = r.action.name();
    match r.action {
        AdminAction::WifiJoin => format!(
            "{name} {} ssid={}",
            r.args.first().map_or("", String::as_str),
            r.args.get(1).map_or("", String::as_str)
        ),
        AdminAction::Keybinds => name.into(),
        AdminAction::UserAdd | AdminAction::UserPassword => {
            format!("{name} {}", r.args.first().map_or("", String::as_str))
        }
        AdminAction::PrinterAdd if r.args.len() == 2 => {
            format!("{name} {} {}", r.args[0], without_userinfo(&r.args[1]))
        }
        _ if r.args.is_empty() => name.into(),
        _ => format!("{name} {}", r.args.join(" ")),
    }
}

/// Request builders (one place for argument order).
pub mod requests {
    use super::*;
    fn req(action: AdminAction, args: &[&str]) -> AdminRequest {
        AdminRequest {
            action,
            args: args.iter().map(|s| (*s).to_owned()).collect(),
        }
    }
    pub fn wifi_join(iface: &str, ssid: &str, password: Option<&str>) -> AdminRequest {
        match password {
            Some(p) => req(AdminAction::WifiJoin, &[iface, ssid, p]),
            None => req(AdminAction::WifiJoin, &[iface, ssid]),
        }
    }
    pub fn appearance(dark: bool) -> AdminRequest {
        req(
            AdminAction::Appearance,
            &[if dark { "dark" } else { "light" }],
        )
    }
    pub fn idle(secs: u32) -> AdminRequest {
        req(AdminAction::Idle, &[&secs.to_string()])
    }
    pub fn pkg_upgrade() -> AdminRequest {
        req(AdminAction::PkgUpgrade, &[])
    }
    pub fn revoke() -> AdminRequest {
        req(AdminAction::Revoke, &[])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdminDecision {
    Allow,
    NotAdmin,
    NeedPassword,
    Authenticate,
}

pub const ADMIN_GROUP: &str = "wheel";
pub const CACHE_SECS: u64 = 300;
/// Fixed delay on every refusal.
pub const REFUSAL_DELAY_MS: u64 = 2000;

/// Member of `wheel` in group(5) text, or primary gid 0.
pub fn is_admin(user: &str, gid: u32, group_file: &str) -> bool {
    gid == 0
        || group_file.lines().any(|l| {
            let f: Vec<&str> = l.trim().split(':').collect();
            f.len() >= 4 && f[0] == ADMIN_GROUP && f[3].split(',').any(|m| m.trim() == user)
        })
}

/// Root needs no password; unknown peers (uid None) and non-admins refused.
pub fn decide(uid: Option<u32>, admin: bool, cached: bool, has_password: bool) -> AdminDecision {
    match uid {
        Some(0) => AdminDecision::Allow,
        None => AdminDecision::NotAdmin,
        _ if !admin => AdminDecision::NotAdmin,
        _ if cached => AdminDecision::Allow,
        _ if has_password => AdminDecision::Authenticate,
        _ => AdminDecision::NeedPassword,
    }
}

/// Authorizations per (uid, session id), valid [`CACHE_SECS`]. Times in seconds on a monotonic clock.
#[derive(Default, Debug)]
pub struct AuthCache {
    granted: HashMap<(u32, i32), u64>,
}

impl AuthCache {
    pub fn grant(&mut self, uid: u32, session: i32, now: u64) {
        self.granted.insert((uid, session), now + CACHE_SECS);
    }
    pub fn valid(&self, uid: u32, session: i32, now: u64) -> bool {
        self.granted
            .get(&(uid, session))
            .is_some_and(|until| now < *until)
    }
    pub fn revoke(&mut self, uid: u32, session: i32) {
        self.granted.remove(&(uid, session));
    }
}

#[cfg(test)]
mod tests {

    /// Audit S4-4: a pacman request is waited for as long as wol-admind may run it; quick
    /// actions keep a short wait.
    #[test]
    fn reply_waits_cover_the_daemon_steps() {
        let r = |action, args: &[&str]| AdminRequest {
            action,
            args: args.iter().map(|s| (*s).to_string()).collect(),
        };
        assert!(reply_secs(&r(AdminAction::PkgInstall, &["gimp"])) >= 1800);
        assert!(reply_secs(&r(AdminAction::PkgUpgrade, &[])) >= 1800);
        assert!(reply_secs(&r(AdminAction::WolInstall, &["/home/a/x.wol", "signed"])) >= 1800);
        assert!(reply_secs(&r(AdminAction::FlatpakInstall, &["org.gimp.GIMP"])) >= 1800);
        assert_eq!(step_secs(AdminAction::WifiJoin), 30);
        assert!(reply_secs(&r(AdminAction::Revoke, &[])) <= 120);
        for a in AdminAction::ALL {
            let want = if LONG_ACTIONS.contains(&a) { 1800 } else { 30 };
            assert_eq!(step_secs(a), want, "{}", a.name());
        }
    }
    use super::*;
    use AdminAction::*;

    fn r(a: AdminAction, args: &[&str]) -> AdminRequest {
        AdminRequest {
            action: a,
            args: args.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn iface_follows_kernel_netdev_names() {
        for ok in [
            "wlan0",
            "wlp2s0",
            "wlx00c0ca123456",
            "wlp0s20f3",
            "a",
            "x".repeat(15).as_str(),
        ] {
            assert!(iface(ok), "{ok}");
        }
        for bad in [
            "",
            ".",
            "..",
            "wl/an0",
            "wl an0",
            "wlan0\n",
            "wl:an0",
            "x".repeat(16).as_str(),
            "-wlan0",
            "wlan0\t",
        ] {
            assert!(!iface(bad), "{bad:?}");
        }
    }

    #[test]
    fn allow_list_table() {
        let long = "x".repeat(200);
        let cases: &[(AdminAction, &[&str], bool)] = &[
            (AudioDefault, &["pcm0"], true),
            (AudioDefault, &["pcm12"], true),
            (AudioDefault, &["pcm"], false),
            (AudioDefault, &["pcm0; rm -rf /"], false),
            (AudioDefault, &["pcm0", "x"], false),
            (AudioDefault, &["-pcm1"], false),
            (WifiPower, &["wlan0", "on"], true),
            (WifiPower, &["wlan0", "off"], true),
            (WifiPower, &["em/0", "off"], false),
            (WifiPower, &["wlan0", "destroy"], false),
            (WifiPower, &["wlan0 down", "on"], false),
            (WifiJoin, &["wlan0", "Home"], true),
            (WifiJoin, &["wlan0", "Home", "password1"], true),
            (WifiJoin, &["wlan0", "Ho\"me"], true),
            (WifiJoin, &["wlan0", "Ho\tme"], false),
            (WifiJoin, &["wlan0", "Home", "short"], false),
            (WifiJoin, &["wlan0", "Attic", "", "hidden"], true),
            (WifiJoin, &["wlan0", "Attic", "password1", "hidden"], true),
            (WifiJoin, &["wlan0", "Attic", "short", "hidden"], false),
            (WifiJoin, &["wlan0", "Attic", "password1", "visible"], false),
            (Bluetooth, &["on"], true),
            (Bluetooth, &["maybe"], false),
            (Appearance, &["dark"], true),
            (Appearance, &["light"], true),
            (Appearance, &["../x"], false),
            (Idle, &["300"], true),
            (Idle, &["0"], true),
            (Idle, &["-1"], false),
            (Idle, &["1e3"], false),
            (Idle, &["99999999"], false),
            (Keybinds, &[r#"{"super+t":["spawn","foot"]}"#], true),
            (Keybinds, &[r#"{"hyper+x":["x"]}"#], false),
            (Keybinds, &["not json"], false),
            (UserAdd, &["bob", "Bob Builder", "yes", "s3cret-pw"], true),
            (UserAdd, &["bob", "", "no", "s3cret-pw"], true),
            (UserAdd, &["bob", "Bob", "no"], false), // no password: refused
            (UserAdd, &["bob", "Bob", "no", "short"], false),
            (UserAdd, &["-G", "x", "no", "s3cret-pw"], false),
            (UserAdd, &["bob", "Bob", "maybe", "s3cret-pw"], false),
            (UserAdd, &["bob", &long, "no", "s3cret-pw"], false),
            (UserAdd, &["live", "Live", "no", "password1"], false),
            (UserAdd, &["systemd-x", "X", "no", "password1"], false),
            (UserAdd, &["git", "Git", "yes", "password1"], false),
            (UserRemove, &["bob"], true),
            (UserRemove, &["root"], false),
            (UserRemove, &["bob;id"], false),
            (TimeZone, &["America/Sao_Paulo"], true),
            (TimeZone, &["UTC"], true),
            (TimeZone, &["../../etc/passwd"], false),
            (TimeZone, &["/etc/passwd"], false),
            (TimeZone, &["-r"], false),
            (PkgUpgrade, &[], true),
            (PkgUpgrade, &["-f"], false),
            (PkgUpgrade, &["firefox", "gimp"], true),
            (PkgUpgrade, &["firefox", "--root=/x"], false),
            (PkgUpgrade, &["a,b"], false),
            (PkgRefresh, &[], true),
            (PkgRefresh, &["x"], false),
            (Revoke, &[], true),
            (Revoke, &["x"], false),
            (PkgInstall, &["steam"], true),
            (PkgInstall, &["htop"], true),
            (PkgInstall, &["-y"], false),
            (PkgInstall, &["a", "b"], false),
            (PkgRemove, &["gimp"], true),
            (
                FlatpakOverride,
                &["org.gimp.GIMP", "filesystems=home", "deny"],
                true,
            ),
            (
                FlatpakOverride,
                &["org.gimp.GIMP", "filesystems=home", "allow"],
                true,
            ),
            (FlatpakOverride, &["org.gimp.GIMP", "reset"], true),
            (
                FlatpakOverride,
                &["org.gimp.GIMP", "--filesystem=/", "allow"],
                false,
            ),
            (
                FlatpakOverride,
                &["org.gimp.GIMP", "filesystems=home", "maybe"],
                false,
            ),
            (FlatpakOverride, &["--user", "reset"], false),
            (FlatpakOverride, &[], false),
            (FlatpakInstall, &["org.gimp.GIMP"], true),
            (FlatpakInstall, &["gimp"], false),
            (FlatpakRemove, &["-y"], false),
            (PkgRemove, &["-dd"], false),
            (PkgRemove, &[], false),
            (BootEnvironment, &["wolos-a"], true),
            (BootEnvironment, &["default"], false),
            (BootEnvironment, &["-F"], false),
            (BootEnvironment, &["a;b"], false),
            (BootEnvironment, &[], false),
            (HwLoad, &["iwlwifi"], true),
            (HwLoad, &["i915"], true),
            (HwLoad, &["evil"], false),
            (HwLoad, &["/tmp/evil.ko"], false),
            (HwLoad, &["iwlwifi", "x"], false),
            (PowerMode, &["low"], true),
            (PowerMode, &["auto"], true),
            (PowerMode, &["high"], true),
            (PowerMode, &["max"], false),
            (PowerMode, &[], false),
            (SessionPower, &["sleep"], true),
            (SessionPower, &["restart"], true),
            (SessionPower, &["shutdown"], true),
            (SessionPower, &["logout"], false),
            (SessionPower, &["sleep", "now"], false),
        ];
        for (a, args, ok) in cases {
            assert_eq!(validate(&r(*a, args)).is_none(), *ok, "{a:?} {args:?}");
        }
    }

    #[test]
    fn wifi_join_uses_networkmanager_with_password_on_stdin() {
        let p = plan(&r(WifiJoin, &["wlan0", "Home", "password1"])).unwrap();
        let AdminStep::RunInput { argv, input } = &p[0] else {
            panic!("{p:?}")
        };
        assert_eq!(
            argv.join(" "),
            "nmcli dev wifi connect Home passwd-file /dev/stdin"
        );
        assert_eq!(input, "802-11-wireless-security.psk:password1\n");
        assert!(!argv.iter().any(|a| a.contains("password1")));
        let p = plan(&r(WifiJoin, &["wlan0", "Attic", "", "hidden"])).unwrap();
        assert!(
            matches!(&p[0], AdminStep::Run(v) if v.join(" ") == "nmcli dev wifi connect Attic hidden yes")
        );
        assert!(!format!("{p:?}").contains("wpa"), "no wpa_supplicant left");
    }

    #[test]
    fn plan_maps_to_fixed_argv() {
        let show = |s: &AdminStep| match s {
            AdminStep::Run(v) => v.join(" "),
            AdminStep::RunInput { argv, .. } => format!("{} <stdin", argv.join(" ")),
            AdminStep::Write { path, .. } => format!("write {path}"),
        };
        let cases: &[(AdminAction, &[&str], &str)] = &[
            (AudioDefault, &["pcm1"], "sysctl hw.snd.default_unit=1"),
            (SessionPower, &["sleep"], "systemctl suspend"),
            (SessionPower, &["restart"], "systemctl reboot"),
            (SessionPower, &["shutdown"], "systemctl poweroff"),
            (WifiPower, &["wlan0", "off"], "nmcli radio wifi off"),
            (Bluetooth, &["on"], "service bluetooth onestart ubt0"),
            (
                UserAdd,
                &["bob", "Bob B", "yes", "s3cret-pw"],
                "useradd -m -c Bob B -s /usr/bin/zsh -G wheel,seat,video,input,audio bob; chpasswd <stdin",
            ),
            (UserRemove, &["bob"], "userdel -r bob"),
            (
                TimeZone,
                &["Europe/Lisbon"],
                "timedatectl set-timezone Europe/Lisbon",
            ),
            (PkgUpgrade, &[], "pacman -Syu --noconfirm"),
            (
                PkgUpgrade,
                &["firefox", "gimp"],
                "pacman -Syu --noconfirm --ignore=firefox,gimp",
            ),
            (PkgRefresh, &[], "pacman -Sy"),
            (
                BootEnvironment,
                &["wolos-b"],
                "bootctl set-default wolos-b.conf",
            ),
            (
                PkgInstall,
                &["steam"],
                "pacman -S --needed --noprogressbar --noconfirm steam",
            ),
            (PkgRemove, &["gimp"], "pacman -Rns --noconfirm gimp"),
            (
                FlatpakOverride,
                &["org.gimp.GIMP", "shared=network", "deny"],
                "flatpak override --system --unshare=network org.gimp.GIMP",
            ),
            (
                FlatpakOverride,
                &["org.gimp.GIMP", "reset"],
                "flatpak override --system --reset org.gimp.GIMP",
            ),
            (
                FlatpakInstall,
                &["org.gimp.GIMP"],
                "flatpak install --system --noninteractive -y flathub org.gimp.GIMP",
            ),
            (
                FlatpakRemove,
                &["org.gimp.GIMP"],
                "flatpak uninstall --system --noninteractive -y org.gimp.GIMP",
            ),
            (WifiJoin, &["wlan0", "Home"], "nmcli dev wifi connect Home"),
            (Appearance, &["dark"], "write /etc/wolos/appearance"),
            (Idle, &["60"], "write /etc/wolos/idle.json"),
            (Revoke, &[], ""),
            (HwLoad, &["i2c_hid_acpi"], "modprobe i2c_hid_acpi"),
            (BootEnroll, &[], "wol boot enroll"),
        ];
        for (a, args, want) in cases {
            let steps = plan(&r(*a, args)).unwrap();
            assert_eq!(steps.iter().map(show).collect::<Vec<_>>().join("; "), *want);
        }
        assert!(plan(&r(PkgUpgrade, &["-f"])).is_err());
        assert!(plan(&r(BootEnroll, &["--force"])).is_err());
        assert!(
            !is_user_level(BootEnroll),
            "Secure Boot setup is an administrator's change"
        );
    }

    fn shown(steps: &[AdminStep]) -> String {
        steps
            .iter()
            .map(|s| match s {
                AdminStep::Run(v) => v.join(" "),
                AdminStep::RunInput { argv, .. } => format!("{} <stdin", argv.join(" ")),
                AdminStep::Write { path, .. } => format!("write {path}"),
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// (probe command line, stdout) answers.
    type Facts = &'static [(&'static str, &'static str)];

    /// Probe that answers `facts`; anything else failed.
    fn probe(facts: Facts) -> impl Fn(&[&str]) -> Option<String> {
        move |argv| {
            let line = argv.join(" ");
            facts.iter().find(|f| f.0 == line).map(|f| f.1.to_string())
        }
    }

    const SPEED_SHIFT: &[(&str, &str)] =
        &[("nproc", "4\n"), ("powerprofilesctl get", "balanced\n")];

    /// Update Now installs the `[wolos]` fixes inside the pinned release (a new release is a new
    /// root slot: `wol update apply`).
    #[test]
    fn package_upgrade_stays_inside_the_release() {
        let p = plan_with(&r(PkgUpgrade, &[]), &probe(&[])).unwrap();
        assert_eq!(shown(&p), "pacman -Syu --noconfirm");
    }

    /// The mode is saved for `wol power apply` (AC changes, boot, resume); the plan switches the
    /// power-profiles-daemon profile only when it differs.
    #[test]
    fn power_mode_saves_the_mode_and_runs_the_policy() {
        let p = plan_with(&r(PowerMode, &["low"]), &probe(SPEED_SHIFT)).unwrap();
        assert_eq!(
            shown(&p),
            "write /etc/wolos/power.json; powerprofilesctl set power-saver"
        );
        let AdminStep::Write { content, mode, .. } = &p[0] else {
            panic!()
        };
        assert_eq!(
            (content.render(Some("{\"mode\":\"auto\"}")).as_str(), *mode),
            ("{\"mode\":\"low\"}\n", 0o644)
        );
        for (m, want) in [
            ("auto", "write /etc/wolos/power.json"),
            (
                "high",
                "write /etc/wolos/power.json; powerprofilesctl set performance",
            ),
        ] {
            let p = plan_with(&r(PowerMode, &[m]), &probe(SPEED_SHIFT)).unwrap();
            assert_eq!(shown(&p), want, "{m}");
        }
    }

    #[test]
    fn power_mode_steers_what_the_machine_has() {
        assert!(
            plan_with(&r(PowerMode, &["low"]), &probe(&[("nproc", "4\n")])).is_err(),
            "no power-profiles-daemon: nothing to control"
        );
        assert!(plan(&r(PowerMode, &["low"])).is_err(), "no facts");
    }

    #[test]
    fn settings_verbs_allow_list() {
        let app = "/usr/bin/wayvnc";
        let cases: &[(AdminAction, &[&str], bool)] = &[
            (Ntp, &["on"], true),
            (Ntp, &["off"], true),
            (Ntp, &["yes"], false),
            (Ntp, &[], false),
            (Hostname, &["julias-laptop"], true),
            (Hostname, &["-n"], false),
            (Hostname, &["a.b"], false),
            (Hostname, &["x", "y"], false),
            (RemoteLogin, &["on"], true),
            (RemoteLogin, &["start"], false),
            (Firewall, &["off"], true),
            (Firewall, &["on", "x"], false),
            (MalwareSignatures, &["on"], true),
            (MalwareSignatures, &["maybe"], false),
            (AirPlayReceiver, &["on"], true),
            (AirPlayReceiver, &["off"], true),
            (AirPlayReceiver, &["start"], false),
            (AirPlayReceiver, &[], false),
            (Lid, &["sleep"], true),
            (Lid, &["nothing"], true),
            (Lid, &["S5"], false),
            (AutoUpdate, &["on"], true),
            (AutoUpdate, &["off"], true),
            (AutoUpdate, &["yes"], false),
            (AutoUpdate, &[], false),
            (
                PrinterAdd,
                &["HP_LaserJet", "ipp://10.0.0.5/ipp/print"],
                true,
            ),
            (
                PrinterAdd,
                &["HP LaserJet", "ipp://10.0.0.5/ipp/print"],
                false,
            ),
            (PrinterAdd, &["HP", "file:///etc/shadow"], false),
            (PrinterAdd, &["HP", "-E"], false),
            (PrinterAdd, &["HP"], false),
            (PrinterRemove, &["HP_LaserJet"], true),
            (PrinterRemove, &["-x"], false),
            (PrinterDefault, &["HP_LaserJet"], true),
            (PrinterDefault, &["a/b"], false),
            (Consent, &[app, "window-list", "allow"], true),
            (Consent, &[app, "input-injection", "deny"], true),
            (Consent, &[app, "shell-layer", "allow"], false),
            (Consent, &[app, "screen-capture", "allow"], false),
            (Consent, &[app, "screen-capture", "deny"], false),
            (Consent, &[app, "window-list", "maybe"], false),
            (Consent, &["wayvnc", "window-list", "allow"], false),
            (Consent, &["/usr/../etc/x", "window-list", "allow"], false),
            (Consent, &[app, "window-list"], false),
            (BtPair, &["00:11:22:aa:bb:cc", "AirPods Pro"], true),
            (BtPair, &["00:11:22:AA:BB:CC", "MX Keys"], true),
            (BtPair, &["00:11:22:aa:bb", "x"], false),
            (BtPair, &["00:11:22:aa:bb:cc", ""], false),
            (BtPair, &["00:11:22:aa:bb:cc", "a\"b"], false),
            (BtPair, &["00:11:22:aa:bb:cc", "a}b"], false),
            (BtPair, &["00:11:22:aa:bb:cc", &"x".repeat(65)], false),
            (BtForget, &["00:11:22:aa:bb:cc"], true),
            (BtForget, &["all"], false),
            (WifiForget, &["wlan0", "Home"], true),
            (WifiForget, &["wl an0", "Home"], false),
            (WifiForget, &["wlan0", "Ho\"me"], true),
            (WifiForget, &["wlan0", "Ho\nme"], false),
            (WifiForget, &["wlan0"], false),
        ];
        for (a, args, ok) in cases {
            assert_eq!(validate(&r(*a, args)).is_none(), *ok, "{a:?} {args:?}");
        }
    }

    /// Screen recording is no consent grant: refused with why, not as a malformed request.
    #[test]
    fn consent_refuses_screen_capture_with_why() {
        for verdict in ["allow", "deny"] {
            let why = validate(&r(
                AdminAction::Consent,
                &["/usr/bin/obs", "screen-capture", verdict],
            ))
            .unwrap();
            assert!(
                why.contains("not a grant") && why.contains("ask each time"),
                "{why}"
            );
        }
        assert_eq!(
            validate(&r(
                AdminAction::Consent,
                &["/usr/bin/obs", "teleport", "allow"]
            ))
            .as_deref(),
            Some("invalid consent request")
        );
    }

    #[test]
    fn settings_verbs_plan() {
        let printer = ["HP", "ipp://10.0.0.5/ipp/print"];
        let app = "/usr/bin/wayvnc";
        let cases: &[(AdminAction, &[&str], Facts, &str)] = &[
            (Ntp, &["on"], &[], "timedatectl set-ntp true"),
            (Ntp, &["off"], &[], "timedatectl set-ntp false"),
            (
                RemoteLogin,
                &["on"],
                &[],
                "systemctl enable --now sshd.service",
            ),
            (
                RemoteLogin,
                &["off"],
                &[],
                "systemctl disable --now sshd.service",
            ),
            (
                AirPlayReceiver,
                &["on"],
                &[],
                "systemctl enable --now shairport-sync.service",
            ),
            (
                AirPlayReceiver,
                &["off"],
                &[],
                "systemctl disable --now shairport-sync.service",
            ),
            (
                Hostname,
                &["julias-laptop"],
                &[],
                "hostnamectl set-hostname julias-laptop",
            ),
            (
                Firewall,
                &["on"],
                &[],
                "systemctl enable --now nftables.service",
            ),
            (
                Firewall,
                &["off"],
                &[],
                "systemctl disable --now nftables.service",
            ),
            (
                Lid,
                &["nothing"],
                &[],
                "write /etc/systemd/logind.conf.d/50-wolos-lid.conf; systemctl kill -s HUP systemd-logind",
            ),
            (
                PrinterAdd,
                &printer,
                &[],
                "lpadmin -p HP -E -v ipp://10.0.0.5/ipp/print -m everywhere",
            ),
            (PrinterRemove, &["HP"], &[], "lpadmin -x HP"),
            (PrinterDefault, &["HP"], &[], "lpadmin -d HP"),
            (
                Consent,
                &[app, "window-list", "allow"],
                &[],
                "write /etc/wolos/consent.json",
            ),
            (
                BtPair,
                &["00:11:22:aa:bb:cc", "MX Keys"],
                &[],
                "bluetoothctl pair 00:11:22:aa:bb:cc; bluetoothctl trust 00:11:22:aa:bb:cc; bluetoothctl connect 00:11:22:aa:bb:cc",
            ),
            (
                BtForget,
                &["00:11:22:aa:bb:cc"],
                &[],
                "bluetoothctl remove 00:11:22:aa:bb:cc",
            ),
            (
                WifiForget,
                &["wlan0", "Home"],
                &[],
                "nmcli connection delete id Home",
            ),
        ];
        for (a, args, facts, want) in cases {
            let p = plan_with(&r(*a, args), &probe(facts)).unwrap();
            assert_eq!(shown(&p), *want, "{a:?} {args:?} {facts:?}");
        }
    }

    /// Merging writes keep everything else in the file.
    #[test]
    fn settings_verbs_merge_files() {
        let content = |a: AdminAction, args: &[&str]| {
            plan(&r(a, args))
                .unwrap()
                .into_iter()
                .find_map(|s| match s {
                    AdminStep::Write { content, mode, .. } => Some((content, mode)),
                    _ => None,
                })
                .unwrap()
        };
        let (c, mode) = content(AutoUpdate, &["off"]);
        assert_eq!(
            (c.render(None), mode),
            (crate::update::auto::conf(false), 0o644)
        );
        let (c, mode) = content(Lid, &["sleep"]);
        assert_eq!(
            (c.render(Some("[Login]\nHandleLidSwitch=ignore\n")), mode),
            (power::lid_conf(power::Lid::Sleep), 0o644)
        );
        let (c, mode) = content(Consent, &["/usr/bin/wayvnc", "window-list", "allow"]);
        let cur = r#"{"grants":[{"app":"/usr/bin/wlrctl","capability":"window-list"}]}"#;
        assert_eq!(crate::consent::parse(&c.render(Some(cur))).len(), 2);
        assert_eq!(mode, 0o644);
        let (c, _) = content(Consent, &["/usr/bin/wlrctl", "window-list", "deny"]);
        assert!(crate::consent::parse(&c.render(Some(cur))).is_empty());
        // An old screen-capture entry is dropped by the next write.
        let old = r#"{"grants":[{"app":"/usr/bin/grim","capability":"screen-capture"}]}"#;
        assert!(!c.render(Some(old)).contains("screen-capture"));
    }

    /// A printer URI may carry credentials (`ipp://user:pass@host`): never logged.
    #[test]
    fn describe_hides_printer_credentials() {
        assert_eq!(
            describe(&r(
                PrinterAdd,
                &["HP", "ipps://bob:hunter22@print.example/ipp"]
            )),
            "printer-add HP ipps://print.example/ipp"
        );
        assert_eq!(
            describe(&r(PrinterAdd, &["HP", "ipp://10.0.0.5/a@b"])),
            "printer-add HP ipp://10.0.0.5/a@b"
        );
        assert_eq!(describe(&r(Hostname, &["wolos"])), "hostname wolos");
    }

    #[test]
    fn write_contents() {
        let p = plan(&r(Idle, &["60"])).unwrap();
        let AdminStep::Write { content, mode, .. } = &p[0] else {
            panic!()
        };
        assert_eq!(
            (content.render(None).as_str(), *mode),
            (r#"{"lock_after_secs":60}"#, 0o644)
        );
    }

    #[test]
    fn user_password_goes_to_stdin_only() {
        let p = plan(&r(UserAdd, &["bob", "Bob", "no", "s3cret-pw"])).unwrap();
        let AdminStep::RunInput { argv, input } = &p[1] else {
            panic!()
        };
        assert_eq!(input, "bob:s3cret-pw\n");
        assert!(argv.iter().all(|a| !a.contains("s3cret")));
        assert!(
            !p.iter()
                .any(|s| matches!(s, AdminStep::Run(v) if v.iter().any(|a| a.contains("s3cret"))))
        );
        assert_eq!(
            describe(&r(UserAdd, &["bob", "Bob", "no", "s3cret-pw"])),
            "user-add bob"
        );
    }

    /// One path changes the account password and the disk's password slot with it.
    #[test]
    fn a_new_password_reaches_the_account_and_the_disk_on_stdin_only() {
        assert!(validate(&r(UserPassword, &["ann", "new-s3cret"])).is_none());
        assert!(validate(&r(UserPassword, &["ann", "short"])).is_some());
        assert!(validate(&r(UserPassword, &["root", "new-s3cret"])).is_some());
        assert!(validate(&r(UserPassword, &["-G", "new-s3cret"])).is_some());
        let p = plan(&r(UserPassword, &["ann", "new-s3cret"])).unwrap();
        let steps: Vec<(String, String)> = p
            .iter()
            .map(|s| match s {
                AdminStep::RunInput { argv, input } => (argv.join(" "), input.clone()),
                _ => panic!("secrets only on stdin"),
            })
            .collect();
        assert_eq!(
            steps,
            [
                ("chpasswd".into(), "ann:new-s3cret\n".into()),
                ("wol encrypt password ann".into(), "new-s3cret\n".into())
            ]
        );
        assert_eq!(
            describe(&r(UserPassword, &["ann", "new-s3cret"])),
            "user-password ann"
        );
        assert!(
            needs_fresh_password(&r(UserPassword, &["ann", "new-s3cret"])),
            "an open session alone never changes the password"
        );
    }

    #[test]
    fn every_action_is_classified() {
        for a in AdminAction::ALL {
            let rows = FRESH_PASSWORD.iter().filter(|(x, _)| *x == a).count();
            assert_eq!(rows, 1, "{a:?} needs exactly one row in FRESH_PASSWORD");
        }
        assert_eq!(FRESH_PASSWORD.len(), AdminAction::ALL.len());
        for a in [
            UserAdd,
            UserRemove,
            UserPassword,
            RemoteLogin,
            Firewall,
            Consent,
            FlatpakOverride,
            BootEnvironment,
            AurOpen,
            BootEnroll,
            PkgInstallFile,
            SystemRollback,
            SystemChannel,
        ] {
            assert_eq!(fresh_rule(a), Fresh::Always, "{a:?}");
        }
        assert!(!needs_fresh_password(&r(WolInstall, &["/a.wol", "signed"])));
        assert!(needs_fresh_password(&r(
            WolInstall,
            &["/a.wol", "unsigned"]
        )));
        assert!(!needs_fresh_password(&r(PkgUpgrade, &[])));
    }

    /// Audit C7-1 (ADR 0034): what wold used to run as root after a plain Allow is an
    /// administrator action. A local package installs only wold's own staged copy, through the
    /// sealed-system pacman with `--` before the path; never a path the caller names.
    #[test]
    fn root_package_changes_are_admin_actions_on_the_staged_copy() {
        let staged = format!(
            "{}/0123456789abcdef0123456789ABCDEF/x-1-1-x86_64.pkg.tar.zst",
            shared::package::STAGING_DIR
        );
        let sha = "0123456789abcdef".repeat(4);
        let steps = plan(&r(PkgInstallFile, &[&staged, &sha])).unwrap();
        assert_eq!(
            shown(&steps),
            format!("wol pkg -- -U --noconfirm -- {staged}")
        );
        let s = shared::package::STAGING_DIR;
        for bad in [
            "/home/ann/x-1-1-x86_64.pkg.tar.zst".to_owned(),
            format!("{s}/x-1-1-x86_64.pkg.tar.zst"),
            format!("{s}/1234-5678-abc/x-1-1-x86_64.pkg.tar.zst"),
            format!("{s}/0123456789abcdef0123456789abcdef/../x.pkg.tar.zst"),
            format!("{s}/0123456789abcdef0123456789abcdef/a/x.pkg.tar.zst"),
            format!("{s}/0123456789abcdef0123456789abcdef/-x.pkg.tar.zst"),
            format!("{s}/0123456789abcdef0123456789abcdef/x.tar"),
            format!("{s}/0123456789abcdef0123456789abcdeg/x.pkg.tar.zst"),
            format!("{s}/0123456789abcdef0123456789abcdef/x\n.pkg.tar.zst"),
        ] {
            assert!(
                validate(&r(PkgInstallFile, &[&bad, &sha])).is_some(),
                "{bad}"
            );
        }
        assert!(validate(&r(PkgInstallFile, &[])).is_some());
        // ADR 0034: the SHA-256 the sheet showed comes along (wol-admind checks it) and is logged.
        assert!(
            validate(&r(PkgInstallFile, &[&staged])).is_some(),
            "no digest"
        );
        let upper = sha.to_uppercase();
        let short = &sha[..63];
        for bad in [upper.as_str(), short, "", &format!("{sha}0")] {
            assert!(
                validate(&r(PkgInstallFile, &[&staged, bad])).is_some(),
                "{bad}"
            );
        }
        assert_eq!(
            describe(&r(PkgInstallFile, &[&staged, &sha])),
            format!("pkg-install-file {staged} {sha}")
        );
        assert_eq!(
            shown(&plan(&r(SystemRollback, &[])).unwrap()),
            "wol update rollback"
        );
        assert!(validate(&r(SystemRollback, &["x"])).is_some());
        let ch = plan(&r(SystemChannel, &["beta"])).unwrap();
        assert!(matches!(
            &ch[..],
            [AdminStep::Write { path, mode: 0o644, content: WriteContent::Fixed(t) }]
                if path == CHANNEL_FILE && t == "beta\n"
        ));
        for bad in ["alpha", "", "--x", "beta\nnightly"] {
            assert!(validate(&r(SystemChannel, &[bad])).is_some(), "{bad:?}");
        }
        for a in [PkgInstallFile, SystemRollback, SystemChannel] {
            assert!(!is_user_level(a), "{a:?} is an administrator's change");
            assert!(needs_fresh_password(&r(a, &[])), "{a:?}");
        }
        assert!(LONG_ACTIONS.contains(&PkgInstallFile));
    }

    #[test]
    fn describe_hides_secrets() {
        assert_eq!(
            describe(&r(WifiJoin, &["wlan0", "Home", "password1"])),
            "wifi-join wlan0 ssid=Home"
        );
        assert_eq!(describe(&r(Keybinds, &["{}"])), "keybinds");
        assert_eq!(describe(&r(Idle, &["60"])), "idle 60");
        assert_eq!(describe(&r(Revoke, &[])), "revoke");
    }

    #[test]
    fn policy_table() {
        let group = "wheel:*:0:root,klee\nstaff:*:20:bob\n";
        assert!(is_admin("klee", 1001, group));
        assert!(!is_admin("bob", 1002, group));
        assert!(is_admin("bob", 0, group));
        use AdminDecision as D;
        for (uid, admin, cached, pw, want) in [
            (Some(0), false, false, false, D::Allow),
            (None, true, true, true, D::NotAdmin),
            (Some(1001), false, true, true, D::NotAdmin),
            (Some(1001), true, true, false, D::Allow),
            (Some(1001), true, false, true, D::Authenticate),
            (Some(1001), true, false, false, D::NeedPassword),
        ] {
            assert_eq!(decide(uid, admin, cached, pw), want);
        }
    }

    /// The person at the computer joins Wi-Fi, pairs a keyboard, sleeps or shuts down without a
    /// password. Everything system-wide still needs an administrator; restart and shut
    /// down too while another account has a desktop session (it would lose its work).
    #[test]
    fn console_user_needs_no_password_for_everyday_changes() {
        for (a, user_level) in [
            (PowerMode, true),
            (WifiJoin, true),
            (WifiPower, true),
            (WifiForget, true),
            (Bluetooth, true),
            (BtPair, true),
            (BtForget, true),
            (SessionPower, true),
            (PkgRefresh, true),
            (PkgUpgrade, false),
            (UserAdd, false),
            (Firewall, false),
            (MalwareSignatures, false),
            (AirPlayReceiver, false),
            (Consent, false),
            (HwLoad, false),
            (RemoteLogin, false),
        ] {
            assert_eq!(is_user_level(a), user_level, "{a:?}");
        }
        let join = r(WifiJoin, &["wlan0", "Home", "psk"]);
        let off = r(SessionPower, &["shutdown"]);
        let sleep = r(SessionPower, &["sleep"]);
        // The lock screen's Restart (audit6 S6-5): refused while another account is signed in,
        // taken when the session is the only one; its caption says so.
        let restart = r(SessionPower, &["restart"]);
        for (req, uid, sessions, want) in [
            (&join, 1002, &[1002][..], true),
            (&join, 1002, &[1001, 1002][..], true),
            (&join, 1002, &[][..], false),
            (&join, 1002, &[1001][..], false),
            (&off, 1002, &[1002][..], true),
            (&off, 1002, &[1001, 1002][..], false),
            (&restart, 1002, &[1002][..], true),
            (&restart, 1002, &[1001, 1002][..], false),
            (&sleep, 1002, &[1001, 1002][..], true),
            (&r(PkgUpgrade, &[]), 1002, &[1002][..], false),
        ] {
            assert_eq!(
                user_level_ok(req, uid, sessions),
                want,
                "{:?} {uid} {sessions:?}",
                req.action
            );
        }
        // Who is in the way of a restart or shut down, for the refusal: each other account once.
        for (req, sessions, want) in [
            (&restart, &[1001, 1002, 1001, 1003][..], &[1001, 1003][..]),
            (&off, &[1002][..], &[][..]),
            (&sleep, &[1001, 1002][..], &[][..]),
            (&join, &[1001, 1002][..], &[][..]),
        ] {
            assert_eq!(signed_in_others(req, 1002, sessions), want, "{sessions:?}");
        }
    }

    #[test]
    fn auth_cache_expires_and_revokes() {
        let mut c = AuthCache::default();
        c.grant(1001, 7, 100);
        assert!(c.valid(1001, 7, 399));
        assert!(!c.valid(1001, 7, 400));
        assert!(!c.valid(1001, 8, 101));
        c.revoke(1001, 7);
        assert!(!c.valid(1001, 7, 101));
        assert_eq!(
            requests::wifi_join("wlan0", "H", None).args,
            vec!["wlan0", "H"]
        );
    }

    /// `.wol` installs (ADR 0011): a local file path and the trust the caller saw; the daemon
    /// verifies again and runs the steps itself, after a restore point.
    #[test]
    fn wol_package_actions() {
        let cases: &[(AdminAction, &[&str], bool)] = &[
            (
                WolInstall,
                &["/home/ann/Downloads/raider-3.1.0.wol", "signed"],
                true,
            ),
            (
                WolInstall,
                &["/home/ann/.cache/wolos/packages/a.wol", "unsigned"],
                true,
            ),
            (WolInstall, &["/home/ann/a.wol", "trusted"], false),
            (WolInstall, &["a.wol", "signed"], false),
            (WolInstall, &["/home/ann/../../etc/a.wol", "signed"], false),
            (WolInstall, &["/home/ann/a.pkg", "signed"], false),
            (WolInstall, &["/home/ann/a\nb.wol", "signed"], false),
            (WolInstall, &["/home/ann/a.wol"], false),
            (WolRemove, &["io.github.adbeveridge.Raider"], true),
            (WolRemove, &["raider"], false),
            (WolRemove, &["../x.y"], false),
        ];
        for (a, args, ok) in cases {
            assert_eq!(validate(&r(*a, args)).is_none(), *ok, "{a:?} {args:?}");
        }
        assert!(
            plan(&r(WolInstall, &["/h/a.wol", "signed"]))
                .unwrap()
                .is_empty()
        );
        assert!(needs_fresh_password(&r(
            WolInstall,
            &["/h/a.wol", "unsigned"]
        )));
        assert!(!needs_fresh_password(&r(
            WolInstall,
            &["/h/a.wol", "signed"]
        )));
        assert!(!needs_fresh_password(&r(PkgUpgrade, &[])));
    }
}
