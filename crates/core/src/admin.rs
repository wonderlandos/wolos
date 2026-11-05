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
    /// `content` is the full new file text; `current` is what the daemon read (for merges like sysctl.d).
    Write {
        path: String,
        mode: u32,
        content: WriteContent,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteContent {
    Fixed(String),
    /// Merge `key=value` lines into the current [`power::SYSCTL_LOCAL`] (allow-listed keys only).
    SysctlUpsert(Vec<(String, String)>),
    /// `key=on|off` set in the current auto-update file, other lines kept.
    AutoUpdate {
        key: &'static str,
        on: bool,
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
            Self::SysctlUpsert(kv) => kv
                .iter()
                .filter(|(k, v)| power::is_allowed_sysctl(k, v))
                .fold(current.unwrap_or("").to_owned(), |conf, (k, v)| {
                    power::upsert_sysctl(Some(&conf), k, v)
                }),
            Self::AutoUpdate { key, on } => crate::update::auto::set(current, key, *on),
            Self::Consent {
                app,
                capability,
                allow,
            } => crate::consent::set_over(
                current,
                &crate::consent::defaults_in(std::path::Path::new(crate::consent::DEFAULTS_DIR)),
                app,
                capability,
                *allow,
            ),
        }
    }
}

/// Read by the error-report sender: `send=1` turns it on (Settings > Privacy).
pub const REPORTS_CONF: &str = "/etc/wolos/reports.conf";
/// The hourly timer of the error-report sender.
pub const REPORTS_TIMER: &str = "wolos-report.timer";

fn on_off(s: &str) -> bool {
    s == "on" || s == "off"
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

/// NetworkManager join (the image ships NetworkManager). Open network: `nmcli dev wifi connect`.
/// With a password: the profile made or changed in place and then
/// `nmcli connection up ... passwd-file /dev/stdin`, the password on stdin, never argv (NET2-1:
/// `dev wifi connect ... passwd-file` is not valid nmcli). `exists`: a Wi-Fi profile of that name
/// is saved; `key_mgmt`: [`crate::wifi::nm_key_mgmt`].
fn nm_join(
    ssid: &str,
    password: Option<&str>,
    hidden: bool,
    exists: bool,
    key_mgmt: &str,
) -> Vec<AdminStep> {
    use crate::wifi;
    match password {
        Some(p) => wifi::nm_profile_argvs(ssid, hidden, key_mgmt, exists)
            .into_iter()
            .map(AdminStep::Run)
            .chain([AdminStep::RunInput {
                argv: wifi::nm_up_argv(ssid),
                input: wifi::nm_secret_input(p),
            }])
            .collect(),
        None => {
            let mut v = vec!["nmcli", "dev", "wifi", "connect", ssid];
            if hidden {
                v.extend(["hidden", "yes"]);
            }
            vec![argv(&v)]
        }
    }
}

/// What to run when a password join failed: delete the profile the join made, so a wrong password
/// leaves nothing behind. None when the request is not a password join or the profile was saved
/// before it (then it is the person's and stays). Ask BEFORE running the plan (`probe` sees the
/// profiles as they were) and run the argv only if the plan failed.
pub fn wifi_join_cleanup(
    r: &AdminRequest,
    probe: &dyn Fn(&[&str]) -> Option<String>,
) -> Option<Vec<String>> {
    if r.action != AdminAction::WifiJoin || validate(r).is_some() {
        return None;
    }
    let password = r.args.get(2).is_some_and(|p| !p.is_empty());
    let ssid = r.args.get(1)?;
    let saved = probe(&crate::wifi::NM_PROFILES_ARGV)
        .is_some_and(|l| crate::wifi::nm_has_profile(&l, ssid));
    (password && !saved).then(|| crate::wifi::nm_cleanup_argv(ssid))
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
        // [name] keeps the home folder; [name, "with-files"] deletes it too (QA-DATA-8).
        AdminAction::UserRemove => ok((n == 1 || (n == 2 && a[1] == USER_REMOVE_WITH_FILES))
            && users::is_valid_name(a[0])
            && a[0] != "root"),
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
        AdminAction::AutoUpdate => {
            ok((n == 1 && on_off(a[0])) || (n == 2 && a[0] == "metered" && on_off(a[1])))
        }
        AdminAction::Reports | AdminAction::CoolingHelper => ok(n == 1 && on_off(a[0])),
        AdminAction::MirrorGate => ok(n == 0),
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
        // Only a channel this release offers (stable, QA-STORE-68).
        AdminAction::SystemChannel => ok(n == 1 && shared::packages::OFFERED.contains(&a[0])),
        // NET-3: every value is checked here again (it is written into a shell file).
        AdminAction::Proxy => ok(crate::netconf::proxy_from_args(&a).is_some()),
        AdminAction::DnsOverTls => ok(crate::netconf::dot_from_args(&a).is_some()),
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
    let steps = match r.action {
        // Update Now is a whole slot change (snapshot, pacman, initramfs, seal, boot entry).
        AdminAction::PkgUpgrade => crate::update::pkg::update_now_most_steps(&r.args),
        _ => plan(r).map_or(1, |s| s.len().max(1)),
    };
    let steps = u32::try_from(steps).unwrap_or(u32::MAX);
    steps
        .saturating_mul(step_secs(r.action))
        .saturating_add(REPLY_MARGIN_SECS)
}

/// [`plan_with`] without facts (actions that need none).
/// Second argument of `user-remove` that also deletes the home folder (`userdel -r`).
pub const USER_REMOVE_WITH_FILES: &str = "with-files";

/// Steps for a valid request, with no facts probed (see [`plan_with`]).
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
        AdminAction::WifiJoin => nm_join(
            &a[1],
            a.get(2).filter(|p| !p.is_empty()).map(String::as_str),
            a.get(3).is_some_and(|h| h == "hidden"),
            probe(&crate::wifi::NM_PROFILES_ARGV)
                .is_some_and(|l| crate::wifi::nm_has_profile(&l, &a[1])),
            crate::wifi::nm_key_mgmt(
                &probe(&crate::wifi::NM_SCAN_ARGV).unwrap_or_default(),
                &a[1],
            ),
        ),
        AdminAction::WifiPower => vec![argv(&["nmcli", "radio", "wifi", &a[1]])],
        // bluetoothd as a systemd service (the FreeBSD `service bluetooth onestart ubt0` was dead
        // code on Linux, QA-SHADOW-42); the radio itself is `bluetoothctl power` as the user.
        // On: clear a hit start limit first (a bluetoothd that crashed five times in a minute is
        // "failed" and refuses to start), then start it; Settings powers the radio as the user
        // afterwards. `bluetoothctl` cannot start it: BlueZ's client waits for org.bluez for ever.
        AdminAction::Bluetooth if a[0] == "on" => vec![
            argv(&["systemctl", "reset-failed", "bluetooth.service"]),
            argv(&["systemctl", "start", "bluetooth.service"]),
        ],
        AdminAction::Bluetooth => vec![argv(&["systemctl", "stop", "bluetooth.service"])],
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
            // QA-LIVE-17: the homes' subvolume keeps its cap (85 % of the pool) so no account can
            // fill the system's share; an image made before the cap gets it with the next account.
            AdminStep::Run(vec![users::HOME_QUOTA.into()]),
        ],
        // QA-DATA-8: never under a signed-in person (their session would lose its files and
        // processes), and the home folder stays unless deleting it was asked for.
        AdminAction::UserRemove => {
            if probe(&[
                "loginctl",
                "show-user",
                &a[0],
                "--property=State",
                "--value",
            ])
            .is_some_and(|s| !s.trim().is_empty())
            {
                return Err(format!(
                    "{} is signed in: they log out first, then the account can be removed",
                    a[0]
                ));
            }
            let argv = if a.get(1).is_some_and(|w| w == USER_REMOVE_WITH_FILES) {
                users::remove_with_files_argv(&a[0])
            } else {
                users::remove_argv(&a[0])
            };
            vec![AdminStep::Run(argv.unwrap_or_default())]
        }
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
        // The update check now: catalogue refreshed in a temporary database (`pacman -Sy` cannot
        // lock the sealed one, ADR 0016), findings in `update::check::DIR` for every account.
        AdminAction::PkgRefresh => vec![argv(&[crate::update::check::SCRIPT, "now"])],
        // Revoke: the daemon handles it. Packages: verified and planned by the daemon itself.
        // Update Now: planned by the daemon from the running slot, the restore point it keeps
        // (`crate::update::pkg::slot_change`).
        AdminAction::PkgUpgrade
        | AdminAction::Revoke
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
        // privilege-cve M4: sshd behind a firewall that drops port 22 does nothing, and the only
        // way out was Firewall off. Remote Login opens (or closes) port 22 itself.
        AdminAction::RemoteLogin => remote_login_steps(
            a[0] == "on",
            crate::sharing::firewall_reload_wanted(
                probe(&crate::sharing::FIREWALL_ENABLED_ARGV).as_deref(),
            ),
        ),
        AdminAction::Firewall => firewall_steps(a[0] == "on"),
        AdminAction::MalwareSignatures => crate::malware::clam_argvs(a[0] == "on")
            .into_iter()
            .map(AdminStep::Run)
            .collect(),
        // QA-NET-5: the receiver's ports open with it (same mechanism as Remote Login).
        AdminAction::AirPlayReceiver => airplay_steps(
            a[0] == "on",
            crate::sharing::firewall_reload_wanted(
                probe(&crate::sharing::FIREWALL_ENABLED_ARGV).as_deref(),
            ),
        ),
        AdminAction::Hostname => vec![argv(&["hostnamectl", "set-hostname", &a[0]])],
        // `on|off` is the download switch, `metered on|off` the metered-connection one: each keeps
        // the other's line.
        AdminAction::AutoUpdate => {
            let (key, on) = match a.as_slice() {
                [on] => ("download", on),
                [_, on] => ("metered", on),
                _ => return Err("auto-update: bad arguments".into()),
            };
            vec![AdminStep::Write {
                path: crate::update::auto::FILE.into(),
                mode: 0o644,
                content: WriteContent::AutoUpdate {
                    key,
                    on: on == "on",
                },
            }]
        }
        // The sender reads `send=1` (files/usr/lib/wolos/report); the timer follows the switch.
        AdminAction::Reports => {
            let on = a[0] == "on";
            let mut steps = vec![write(
                REPORTS_CONF,
                0o644,
                format!("send={}\n", u8::from(on)),
            )];
            steps.extend(service_steps(REPORTS_TIMER, on));
            steps
        }
        // `wol doctor hardware`: the repo gate as root, and Intel's thermal helper with its unit.
        AdminAction::MirrorGate => vec![argv(&["wol", "mirror", "gate", "--yes"])],
        AdminAction::CoolingHelper => service_steps("thermald.service", a[0] == "on"),
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
        // QA-NET-17: every saved profile that joins this SSID, by UUID ("Home", "Home 1"), never
        // a profile that merely shares the name. None found: the name itself (an older profile
        // whose SSID could not be read).
        AdminAction::WifiForget => {
            let uuids: Vec<String> = probe(&crate::wifi::NM_UUIDS_ARGV)
                .map(|l| crate::wifi::parse_wifi_uuids(&l))
                .unwrap_or_default()
                .into_iter()
                .filter(|u| {
                    let a = crate::wifi::nm_ssid_argv(u);
                    let a: Vec<&str> = a.iter().map(String::as_str).collect();
                    probe(&a).is_some_and(|s| crate::wifi::unescape(s.trim()) == r.args[1])
                })
                .collect();
            if uuids.is_empty() {
                vec![argv(&["nmcli", "connection", "delete", "id", &a[1]])]
            } else {
                uuids
                    .iter()
                    .map(|u| argv(&["nmcli", "connection", "delete", "uuid", u]))
                    .collect()
            }
        }
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
        AdminAction::Proxy => {
            let a: Vec<&str> = a.iter().map(String::as_str).collect();
            let c = crate::netconf::proxy_from_args(&a).ok_or("proxy: bad arguments")?;
            vec![
                write(
                    crate::netconf::PROXY_CONF,
                    0o644,
                    crate::netconf::render_proxy_conf(&c),
                ),
                write(
                    crate::netconf::PROXY_PROFILE,
                    0o644,
                    crate::netconf::render_proxy_profile(&c),
                ),
            ]
        }
        AdminAction::DnsOverTls => {
            let a: Vec<&str> = a.iter().map(String::as_str).collect();
            let c = crate::netconf::dot_from_args(&a).ok_or("dns-over-tls: bad arguments")?;
            vec![
                argv(&["mkdir", "-p", "-m", "0755", "/etc/systemd/resolved.conf.d"]),
                write(
                    crate::netconf::DOT_DROPIN,
                    0o644,
                    crate::netconf::render_dot(&c),
                ),
                argv(&crate::netconf::RESOLVED_RESTART_ARGV),
            ]
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

/// Firewall on/off. Off also deletes the table by hand: the unit's own stop does it
/// ([`crate::sharing::FIREWALL_DROP_IN`]), but a running system that has not restarted since the
/// drop-in changed still has a oneshot unit that reads "inactive", and `disable --now` on it
/// unloads nothing (QA-NET-1). `destroy` is no error when the table is already gone.
fn firewall_steps(on: bool) -> Vec<AdminStep> {
    let mut v = service_steps("nftables.service", on);
    if !on {
        v.push(argv(&crate::sharing::FIREWALL_DELETE_ARGV));
    }
    v
}

/// Remote Login: the firewall rule for port 22 ([`crate::sharing::SSH_RULE_FILE`]), the firewall
/// ruleset loaded again when the firewall is on (`firewall_on`, [`crate::sharing::firewall_reload_wanted`]),
/// and sshd. On opens the port before sshd listens; off stops sshd first.
fn remote_login_steps(on: bool, firewall_on: bool) -> Vec<AdminStep> {
    use crate::sharing::{FIREWALL_RELOAD_ARGV, SSH_RULE_FILE, ssh_rule};
    let mut rule = vec![write(SSH_RULE_FILE, 0o644, ssh_rule(on))];
    if firewall_on {
        rule.push(argv(&FIREWALL_RELOAD_ARGV));
    }
    let unit = service_steps("sshd.service", on);
    if on {
        rule.into_iter().chain(unit).collect()
    } else {
        unit.into_iter().chain(rule).collect()
    }
}

/// AirPlay Receiver: its rule file ([`crate::sharing::AIRPLAY_RULE_FILE`]), the ruleset loaded
/// again when the firewall is on, and shairport-sync. On opens the ports before it listens; off
/// stops it first.
fn airplay_steps(on: bool, firewall_on: bool) -> Vec<AdminStep> {
    use crate::sharing::{AIRPLAY_RULE_FILE, AIRPLAY_UNIT, FIREWALL_RELOAD_ARGV, airplay_rule};
    let mut rule = vec![write(AIRPLAY_RULE_FILE, 0o644, airplay_rule(on))];
    if firewall_on {
        rule.push(argv(&FIREWALL_RELOAD_ARGV));
    }
    let unit = service_steps(AIRPLAY_UNIT, on);
    if on {
        rule.into_iter().chain(unit).collect()
    } else {
        unit.into_iter().chain(rule).collect()
    }
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
        (A::Reports, Cached),
        // NET-3: where every connection or name lookup of this computer goes (a proxy or a DNS
        // server of someone else's choosing sees it all).
        (A::Proxy, Always),
        (A::DnsOverTls, Always),
        // Package repositories change under every later update: the password, every time.
        (A::MirrorGate, Always),
        (A::CoolingHelper, Cached),
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

/// Whether the account `user` (else the one whose uid is `uid` in `passwd`) is an administrator
/// by `group_file` ([`is_admin`]). A process the compositor started has no `$USER`, so the name
/// follows the uid; neither: not an administrator (QA-A7-17).
pub fn account_is_admin(user: Option<&str>, uid: u32, passwd: &str, group_file: &str) -> bool {
    let from_passwd = || {
        passwd.lines().find_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            (f.get(2)?.parse::<u32>().ok()? == uid).then(|| f[0])
        })
    };
    user.filter(|u| !u.is_empty())
        .or_else(from_passwd)
        .is_some_and(|name| is_admin(name, u32::MAX, group_file))
}

/// [`account_is_admin`] for the signed-in account: `user` is its `$USER` (the caller reads the
/// environment), the uid is this process's, and the files are the system's `/etc/passwd` and
/// `/etc/group`.
pub fn this_account_is_admin(user: Option<&str>) -> bool {
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    account_is_admin(
        user,
        shared::sys::euid(),
        &read("/etc/passwd"),
        &read("/etc/group"),
    )
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
mod tests;
