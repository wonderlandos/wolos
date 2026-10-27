//! wol-admind allow-list (IaraOS ADR 0026): validation, fixed argv / whole-file steps, policy. Pure.
use crate::settings::{appearance, idle, keybinds, timezones, users};
use crate::wifi;
use crate::{bluetooth, power, printers, sharing};
use shared::admin::{AdminAction, AdminRequest};
use std::collections::HashMap;

pub const MAX_IDLE_SECS: u32 = 86_400;
pub const MAX_FULL_NAME: usize = 64;

/// One step: a fixed argv (never a shell) or a whole-file write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdminStep {
    Run(Vec<String>),
    /// Fixed argv with a secret on stdin (never argv): e.g. `pw usermod -h 0`.
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
    /// Merge `key=value` lines into the current sysctl.conf.local (allow-listed keys only).
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
    /// Add (with name) or drop (None) one device in the current hcsecd.conf.
    Hcsecd {
        addr: String,
        name: Option<String>,
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
            Self::Hcsecd { addr, name: None } => bluetooth::remove_hcsecd(current, addr),
            Self::Hcsecd {
                addr,
                name: Some(n),
            } => bluetooth::upsert_hcsecd(current, addr, n),
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
fn iface(s: &str) -> bool {
    digits_after(s, "wlan", 4, 9)
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
        AdminAction::UserAdd => ok(n == 4
            && users::is_valid_name(a[0])
            && a[1].chars().count() <= MAX_FULL_NAME
            && (a[2] == "yes" || a[2] == "no")
            && users::is_valid_password(a[3])),
        AdminAction::UserRemove => ok(n == 1 && users::is_valid_name(a[0]) && a[0] != "root"),
        AdminAction::TimeZone => ok(n == 1 && timezones::is_valid(a[0])),
        AdminAction::PkgUpgrade | AdminAction::PkgRefresh | AdminAction::Revoke => ok(n == 0),
        AdminAction::PkgInstall => ok(n == 1 && crate::store::install_plan(a[0]).is_some()),
        AdminAction::PowerMode => ok(n == 1 && power::policy::Mode::parse(a[0]).is_some()),
        AdminAction::BootEnvironment => ok(n == 1 && crate::update::is_valid_be(a[0])),
        AdminAction::HwLoad => ok(n == 1 && crate::hardware::is_catalogue_kld(a[0])),
        AdminAction::WolInstall => {
            ok(n == 2 && is_wol_path(a[0]) && (a[1] == "signed" || a[1] == "unsigned"))
        }
        AdminAction::WolRemove => ok(n == 1 && shared::package::is_app_id(a[0])),
        AdminAction::Ntp | AdminAction::RemoteLogin | AdminAction::Firewall => {
            ok(n == 1 && on_off(a[0]))
        }
        AdminAction::Hostname => ok(n == 1 && sharing::is_valid_hostname(a[0])),
        AdminAction::Lid => ok(n == 1 && (a[0] == "sleep" || a[0] == "nothing")),
        AdminAction::PrinterAdd => {
            ok(n == 2 && printers::is_valid_name(a[0]) && printers::is_valid_uri(a[1]))
        }
        AdminAction::PrinterRemove | AdminAction::PrinterDefault => {
            ok(n == 1 && printers::is_valid_name(a[0]))
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
/// plan for this machine and power source now (`power::policy`). Nothing steers the CPU (no Speed
/// Shift, powerd off): Err, the switch is hidden there.
pub fn power_steps(
    mode: power::policy::Mode,
    f: &power::policy::Facts,
) -> Result<Vec<AdminStep>, String> {
    if f.cpu == power::policy::Cpu::Fixed {
        return Err("power-mode: no Speed Shift preference and powerd is off".into());
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
        AdminAction::WifiJoin => vec![
            AdminStep::Write {
                path: wifi::CONF_PATH.into(),
                mode: 0o600,
                content: WriteContent::WifiUpsert {
                    ssid: a[1].clone(),
                    password: a.get(2).filter(|p| !p.is_empty()).cloned(),
                    hidden: a.get(3).is_some_and(|h| h == "hidden"),
                },
            },
            argv(&["wpa_cli", "-i", &a[0], "reconfigure"]),
        ],
        AdminAction::WifiPower => vec![argv(&[
            "ifconfig",
            &a[0],
            if a[1] == "on" { "up" } else { "down" },
        ])],
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
                input: format!("{}\n", a[3]),
            },
        ],
        AdminAction::UserRemove => vec![AdminStep::Run(
            users::remove_argv(&a[0]).unwrap_or_default(),
        )],
        AdminAction::TimeZone => vec![AdminStep::Run(timezones::set_argv(&a[0]))],
        AdminAction::PkgRefresh => vec![argv(&["pkg", "update", "-q"])],
        AdminAction::PkgUpgrade => {
            // FreeBSD's fixes the daily check fetched (update::check), then the packages.
            use crate::update::check;
            let ready = check::base_ready_argv();
            let ready: Vec<&str> = ready.iter().map(String::as_str).collect();
            let base = probe(&ready).map(|_| AdminStep::Run(check::base_install_argv()));
            base.into_iter()
                .chain([argv(&["pkg", "upgrade", "-y"])])
                .collect()
        }
        // Revoke: the daemon handles it. Packages: verified and planned by the daemon itself.
        AdminAction::Revoke | AdminAction::WolInstall | AdminAction::WolRemove => vec![],
        AdminAction::PowerMode => power_steps(
            power::policy::Mode::parse(&a[0]).unwrap_or_default(),
            &power::policy::facts(probe),
        )?,
        AdminAction::PkgInstall => crate::store::install_plan(&a[0])
            .unwrap_or_default()
            .into_iter()
            .map(AdminStep::Run)
            .collect(),
        AdminAction::BootEnvironment => {
            vec![AdminStep::Run(crate::update::be::activate_argv(&a[0]))]
        }
        AdminAction::HwLoad => vec![argv(&["kldload", "-n", &a[0]])],
        AdminAction::Ntp => service_steps(
            "ntpd",
            a[0] == "on",
            &["ntpd_enable=YES", "ntpd_sync_on_start=YES"],
            &["ntpd_enable=NO"],
            probe,
        ),
        AdminAction::RemoteLogin => service_steps(
            "sshd",
            a[0] == "on",
            &["sshd_enable=YES"],
            &["sshd_enable=NO"],
            probe,
        ),
        // `service pf onestart` loads the rules and enables pf whether or not it runs (it never
        // refuses a running pf); `onestop` disables it only when enabled.
        AdminAction::Firewall if a[0] == "on" => vec![
            argv(&[
                "sysrc",
                "pf_enable=YES",
                &format!("pf_rules={}", sharing::PF_RULES),
            ]),
            argv(&["service", "pf", "onestart"]),
        ],
        AdminAction::Firewall => vec![
            argv(&["service", "pf", "onestop"]),
            argv(&["sysrc", "pf_enable=NO"]),
        ],
        AdminAction::Hostname => vec![
            argv(&["sysrc", &format!("hostname={}", a[0])]),
            argv(&["hostname", &a[0]]),
        ],
        AdminAction::Lid => {
            let key = "hw.acpi.lid_switch_state";
            let v = power::lid_value(if a[0] == "sleep" {
                power::Lid::Sleep
            } else {
                power::Lid::Nothing
            });
            vec![
                argv(&["sysctl", &format!("{key}={v}")]),
                AdminStep::Write {
                    path: power::SYSCTL_LOCAL.into(),
                    mode: 0o644,
                    content: WriteContent::SysctlUpsert(vec![(key.into(), v.into())]),
                },
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
        AdminAction::BtPair | AdminAction::BtForget => vec![
            AdminStep::Write {
                path: bluetooth::HCSECD_CONF.into(),
                mode: 0o600,
                content: WriteContent::Hcsecd {
                    addr: a[0].to_ascii_lowercase(),
                    name: a.get(1).cloned(),
                },
            },
            argv(&["service", "hcsecd", "onerestart"]),
        ],
        AdminAction::WifiForget => vec![
            AdminStep::Write {
                path: wifi::CONF_PATH.into(),
                mode: 0o600,
                content: WriteContent::WifiRemove { ssid: a[1].clone() },
            },
            argv(&["wpa_cli", "-i", &a[0], "reconfigure"]),
        ],
        AdminAction::SessionPower => {
            vec![argv(session_power_argv(&a[0]).unwrap_or_default())]
        }
    })
}

/// Session menu power verbs. Sleep goes through rc.suspend, which locks every session first
/// (`wol power suspend`).
pub fn session_power_argv(verb: &str) -> Option<&'static [&'static str]> {
    Some(match verb {
        "sleep" => &["acpiconf", "-s", "3"],
        "restart" => &["shutdown", "-r", "now"],
        "shutdown" => &["shutdown", "-p", "now"],
        _ => return None,
    })
}

/// rc.conf switch for `svc`, then start or stop it now unless it already is so (rc.subr refuses
/// to start a running service or stop a stopped one). Running = `service <svc> onestatus` exits 0.
fn service_steps(
    svc: &str,
    on: bool,
    enable: &[&str],
    disable: &[&str],
    probe: &dyn Fn(&[&str]) -> Option<String>,
) -> Vec<AdminStep> {
    let mut sysrc = vec!["sysrc"];
    sysrc.extend(if on { enable } else { disable });
    let mut steps = vec![argv(&sysrc)];
    let running = probe(&["service", svc, "onestatus"]).is_some();
    if on != running {
        steps.push(argv(&[
            "service",
            svc,
            if on { "onestart" } else { "onestop" },
        ]));
    }
    steps
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
/// one of them, and restart or shut down only when no other account would lose its session.
pub fn user_level_ok(r: &AdminRequest, uid: u32, sessions: &[u32]) -> bool {
    let ends_sessions =
        r.action == AdminAction::SessionPower && r.args.first().is_some_and(|v| v != "sleep");
    is_user_level(r.action)
        && sessions.contains(&uid)
        && !(ends_sessions && sessions.iter().any(|s| *s != uid))
}

/// Package changes made in place: wol-admind creates a restore point (boot environment) first
/// (`update::be::restore_point`).
pub fn needs_restore_point(a: AdminAction) -> bool {
    matches!(
        a,
        AdminAction::PkgUpgrade
            | AdminAction::PkgInstall
            | AdminAction::WolInstall
            | AdminAction::WolRemove
    )
}

/// An unsigned `.wol` install is approved with the password every time (never the cache).
pub fn needs_fresh_password(r: &AdminRequest) -> bool {
    r.action == AdminAction::WolInstall && r.args.get(1).is_some_and(|t| t == "unsigned")
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
        AdminAction::UserAdd => format!("{name} {}", r.args.first().map_or("", String::as_str)),
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
    /// A network that does not announce its name ("Other Network…").
    pub fn wifi_join_hidden(iface: &str, ssid: &str, password: Option<&str>) -> AdminRequest {
        req(
            AdminAction::WifiJoin,
            &[iface, ssid, password.unwrap_or(""), "hidden"],
        )
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
    use super::*;
    use AdminAction::*;

    fn r(a: AdminAction, args: &[&str]) -> AdminRequest {
        AdminRequest {
            action: a,
            args: args.iter().map(|s| (*s).to_owned()).collect(),
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
            (WifiPower, &["em0", "off"], false),
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
            (PkgRefresh, &[], true),
            (PkgRefresh, &["x"], false),
            (Revoke, &[], true),
            (Revoke, &["x"], false),
            (PkgInstall, &["steam"], true),
            (PkgInstall, &["htop"], true),
            (PkgInstall, &["-y"], false),
            (PkgInstall, &["a", "b"], false),
            (BootEnvironment, &["wol-before-update-1"], true),
            (BootEnvironment, &["default"], true),
            (BootEnvironment, &["-F"], false),
            (BootEnvironment, &["a;b"], false),
            (BootEnvironment, &[], false),
            (HwLoad, &["if_iwlwifi"], true),
            (HwLoad, &["coretemp"], true),
            (HwLoad, &["evil"], false),
            (HwLoad, &["/tmp/evil.ko"], false),
            (HwLoad, &["if_iwlwifi", "x"], false),
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
    fn plan_maps_to_fixed_argv() {
        let show = |s: &AdminStep| match s {
            AdminStep::Run(v) => v.join(" "),
            AdminStep::RunInput { argv, .. } => format!("{} <stdin", argv.join(" ")),
            AdminStep::Write { path, .. } => format!("write {path}"),
        };
        let cases: &[(AdminAction, &[&str], &str)] = &[
            (AudioDefault, &["pcm1"], "sysctl hw.snd.default_unit=1"),
            (SessionPower, &["sleep"], "acpiconf -s 3"),
            (SessionPower, &["restart"], "shutdown -r now"),
            (SessionPower, &["shutdown"], "shutdown -p now"),
            (WifiPower, &["wlan0", "off"], "ifconfig wlan0 down"),
            (Bluetooth, &["on"], "service bluetooth onestart ubt0"),
            (
                UserAdd,
                &["bob", "Bob B", "yes", "s3cret-pw"],
                "pw useradd -n bob -c Bob B -m -s /usr/local/bin/zsh -G wheel,video,operator; pw usermod -n bob -h 0 <stdin",
            ),
            (UserRemove, &["bob"], "pw userdel -n bob -r"),
            (TimeZone, &["Europe/Lisbon"], "tzsetup Europe/Lisbon"),
            (PkgUpgrade, &[], "pkg upgrade -y"),
            (PkgRefresh, &[], "pkg update -q"),
            (BootEnvironment, &["wol-1"], "bectl activate wol-1"),
            (
                PkgInstall,
                &["steam"],
                "sysrc linux_enable=YES; service linux start; pkg install -y linux_base-rl9 linux-steam-utils",
            ),
            (
                WifiJoin,
                &["wlan0", "Home"],
                "write /etc/wpa_supplicant.conf; wpa_cli -i wlan0 reconfigure",
            ),
            (
                Appearance,
                &["dark"],
                "write /usr/local/etc/wolos/appearance",
            ),
            (Idle, &["60"], "write /usr/local/etc/wolos/idle.json"),
            (Revoke, &[], ""),
            (HwLoad, &["iichid"], "kldload -n iichid"),
        ];
        for (a, args, want) in cases {
            let steps = plan(&r(*a, args)).unwrap();
            assert_eq!(steps.iter().map(show).collect::<Vec<_>>().join("; "), *want);
        }
        assert!(plan(&r(PkgUpgrade, &["-f"])).is_err());
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

    const SPEED_SHIFT: &[(&str, &str)] = &[
        ("sysctl -n hw.ncpu", "4\n"),
        ("sysctl -n dev.hwpstate_intel.0.epp", "50\n"),
    ];

    /// Update Now installs what the daily check fetched: FreeBSD's fixes (when ready), then the
    /// packages, after one restore point (wol-admind).
    #[test]
    fn package_upgrade_installs_fetched_freebsd_fixes_first() {
        let ready: Facts = &[(
            "freebsd-update -f /var/db/wolos/updates/freebsd-update.conf updatesready",
            "There are updates available to install.\n",
        )];
        let p = plan_with(&r(PkgUpgrade, &[]), &probe(ready)).unwrap();
        assert_eq!(
            shown(&p),
            "freebsd-update -f /var/db/wolos/updates/freebsd-update.conf install; pkg upgrade -y"
        );
        let p = plan_with(&r(PkgUpgrade, &[]), &probe(&[])).unwrap();
        assert_eq!(shown(&p), "pkg upgrade -y", "nothing fetched");
    }

    /// Regression: on Speed Shift CPUs the hardware module turns powerd off, so
    /// `service powerd restart` failed and Low Power Mode never switched. The mode is saved for
    /// `wol power apply` (AC changes, boot, resume); the power source picks the preference.
    #[test]
    fn power_mode_saves_the_mode_and_runs_the_policy() {
        let p = plan_with(&r(PowerMode, &["low"]), &probe(SPEED_SHIFT)).unwrap();
        assert_eq!(
            shown(&p),
            "write /usr/local/etc/wolos/power.json; sysctl dev.hwpstate_intel.0.epp=85 \
             dev.hwpstate_intel.1.epp=85 dev.hwpstate_intel.2.epp=85 dev.hwpstate_intel.3.epp=85"
        );
        let AdminStep::Write { content, mode, .. } = &p[0] else {
            panic!()
        };
        assert_eq!(
            (content.render(Some("{\"mode\":\"auto\"}")).as_str(), *mode),
            ("{\"mode\":\"low\"}\n", 0o644)
        );
        for (m, epp) in [("auto", "50"), ("high", "0")] {
            let p = plan_with(&r(PowerMode, &[m]), &probe(SPEED_SHIFT)).unwrap();
            assert!(
                shown(&p).contains(&format!("sysctl dev.hwpstate_intel.0.epp={epp} ")),
                "{m}: {}",
                shown(&p)
            );
        }
        const ON_BATTERY: &[(&str, &str)] = &[
            ("sysctl -n hw.ncpu", "1\n"),
            ("sysctl -n dev.hwpstate_intel.0.epp", "50\n"),
            ("sysctl -n hw.acpi.acline", "0\n"),
        ];
        let p = plan_with(&r(PowerMode, &["auto"]), &probe(ON_BATTERY)).unwrap();
        assert!(shown(&p).ends_with("sysctl dev.hwpstate_intel.0.epp=75"));
    }

    #[test]
    fn power_mode_steers_what_the_machine_has() {
        const POWERD: &[(&str, &str)] = &[
            ("sysctl -n hw.ncpu", "8\n"),
            ("service powerd enabled", ""),
            ("sysrc -n powerd_flags", "-a hiadaptive\n"),
        ];
        const BOTH: &[(&str, &str)] = &[
            ("sysctl -n hw.ncpu", "1\n"),
            ("sysctl -n dev.hwpstate_intel.0.epp", "85\n"),
            ("service powerd enabled", ""),
        ];
        for (facts, mode, want) in [
            (
                POWERD,
                "low",
                "write /usr/local/etc/wolos/power.json; sysrc powerd_flags=-a minimum -b minimum; \
                 service powerd restart",
            ),
            (POWERD, "auto", "write /usr/local/etc/wolos/power.json"),
            (
                POWERD,
                "high",
                "write /usr/local/etc/wolos/power.json; sysrc powerd_flags=-a maximum -b hiadaptive; \
                 service powerd restart",
            ),
            (
                BOTH,
                "low",
                "write /usr/local/etc/wolos/power.json; sysctl dev.hwpstate_intel.0.epp=85",
            ),
        ] {
            let p = plan_with(&r(PowerMode, &[mode]), &probe(facts)).unwrap();
            assert_eq!(shown(&p), want, "{facts:?} {mode}");
        }
        assert!(
            plan_with(
                &r(PowerMode, &["low"]),
                &probe(&[("sysctl -n hw.ncpu", "4\n")])
            )
            .is_err(),
            "neither EPP nor powerd: nothing to control"
        );
        assert!(plan(&r(PowerMode, &["low"])).is_err(), "no facts");
    }

    #[test]
    fn settings_verbs_allow_list() {
        let app = "/usr/local/bin/wayvnc";
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
            (Lid, &["sleep"], true),
            (Lid, &["nothing"], true),
            (Lid, &["S5"], false),
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
            (Consent, &[app, "screen-capture", "allow"], true),
            (Consent, &[app, "input-injection", "deny"], true),
            (Consent, &[app, "shell-layer", "allow"], false),
            (Consent, &[app, "screen-capture", "maybe"], false),
            (Consent, &["wayvnc", "screen-capture", "allow"], false),
            (
                Consent,
                &["/usr/../etc/x", "screen-capture", "allow"],
                false,
            ),
            (Consent, &[app, "screen-capture"], false),
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
            (WifiForget, &["em0", "Home"], false),
            (WifiForget, &["wlan0", "Ho\"me"], true),
            (WifiForget, &["wlan0", "Ho\nme"], false),
            (WifiForget, &["wlan0"], false),
        ];
        for (a, args, ok) in cases {
            assert_eq!(validate(&r(*a, args)).is_none(), *ok, "{a:?} {args:?}");
        }
    }

    #[test]
    fn settings_verbs_plan() {
        const NTPD_UP: &[(&str, &str)] = &[("service ntpd onestatus", "ntpd is running\n")];
        const SSHD_UP: &[(&str, &str)] = &[("service sshd onestatus", "sshd is running\n")];
        let printer = ["HP", "ipp://10.0.0.5/ipp/print"];
        let app = "/usr/local/bin/wayvnc";
        let cases: &[(AdminAction, &[&str], Facts, &str)] = &[
            (
                Ntp,
                &["on"],
                &[],
                "sysrc ntpd_enable=YES ntpd_sync_on_start=YES; service ntpd onestart",
            ),
            (
                Ntp,
                &["on"],
                NTPD_UP,
                "sysrc ntpd_enable=YES ntpd_sync_on_start=YES",
            ),
            (
                Ntp,
                &["off"],
                NTPD_UP,
                "sysrc ntpd_enable=NO; service ntpd onestop",
            ),
            (Ntp, &["off"], &[], "sysrc ntpd_enable=NO"),
            (
                RemoteLogin,
                &["on"],
                &[],
                "sysrc sshd_enable=YES; service sshd onestart",
            ),
            (RemoteLogin, &["on"], SSHD_UP, "sysrc sshd_enable=YES"),
            (
                RemoteLogin,
                &["off"],
                SSHD_UP,
                "sysrc sshd_enable=NO; service sshd onestop",
            ),
            (
                Hostname,
                &["julias-laptop"],
                &[],
                "sysrc hostname=julias-laptop; hostname julias-laptop",
            ),
            (
                Firewall,
                &["on"],
                &[],
                "sysrc pf_enable=YES pf_rules=/usr/local/share/wolos/pf.conf; service pf onestart",
            ),
            (
                Firewall,
                &["off"],
                &[],
                "service pf onestop; sysrc pf_enable=NO",
            ),
            (
                Lid,
                &["nothing"],
                &[],
                "sysctl hw.acpi.lid_switch_state=NONE; write /etc/sysctl.conf.local",
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
                &[app, "screen-capture", "allow"],
                &[],
                "write /usr/local/etc/wolos/consent.json",
            ),
            (
                BtPair,
                &["00:11:22:aa:bb:cc", "MX Keys"],
                &[],
                "write /etc/bluetooth/hcsecd.conf; service hcsecd onerestart",
            ),
            (
                BtForget,
                &["00:11:22:aa:bb:cc"],
                &[],
                "write /etc/bluetooth/hcsecd.conf; service hcsecd onerestart",
            ),
            (
                WifiForget,
                &["wlan0", "Home"],
                &[],
                "write /etc/wpa_supplicant.conf; wpa_cli -i wlan0 reconfigure",
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
        let (c, mode) = content(Lid, &["sleep"]);
        assert_eq!(
            (
                c.render(Some("kern.coredump=0\nhw.acpi.lid_switch_state=NONE\n")),
                mode
            ),
            (
                "kern.coredump=0\nhw.acpi.lid_switch_state=S3\n".into(),
                0o644
            )
        );
        let (c, mode) = content(
            Consent,
            &["/usr/local/bin/wayvnc", "screen-capture", "allow"],
        );
        let cur = r#"{"grants":[{"app":"/usr/local/bin/grim","capability":"screen-capture"}]}"#;
        assert_eq!(crate::consent::parse(&c.render(Some(cur))).len(), 2);
        assert_eq!(mode, 0o644);
        let (c, _) = content(Consent, &["/usr/local/bin/grim", "screen-capture", "deny"]);
        assert!(crate::consent::parse(&c.render(Some(cur))).is_empty());
        let (c, mode) = content(BtPair, &["00:11:22:AA:BB:CC", "MX Keys"]);
        let paired = c.render(Some("# hcsecd.conf\n"));
        assert!(
            paired.starts_with("# hcsecd.conf\n") && paired.contains("bdaddr\t00:11:22:aa:bb:cc;")
        );
        assert_eq!(mode, 0o600);
        let (c, _) = content(BtForget, &["00:11:22:aa:bb:cc"]);
        assert_eq!(c.render(Some(&paired)), "# hcsecd.conf\n\n");
        let (c, mode) = content(WifiForget, &["wlan0", "Home"]);
        let wpa = "ctrl_interface=/var/run/wpa_supplicant\n\nnetwork={\n\tssid=\"Home\"\n\tpsk=\"pw123456\"\n}\nnetwork={\n\tssid=\"Work\"\n\tkey_mgmt=NONE\n}\n";
        assert_eq!(
            c.render(Some(wpa)),
            "ctrl_interface=/var/run/wpa_supplicant\n\nnetwork={\n\tssid=\"Work\"\n\tkey_mgmt=NONE\n}\n"
        );
        assert_eq!(mode, 0o600);
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
        let p = plan(&r(WifiJoin, &["wlan0", "Home", "password1"])).unwrap();
        let AdminStep::Write { content, mode, .. } = &p[0] else {
            panic!()
        };
        assert_eq!(*mode, 0o600);
        assert!(content.render(None).contains("psk=\"password1\""));
        let p = plan(&requests::wifi_join_hidden("wlan0", "Attic", None)).unwrap();
        let AdminStep::Write { content, .. } = &p[0] else {
            panic!()
        };
        assert!(
            content
                .render(None)
                .ends_with("\tssid=\"Attic\"\n\tscan_ssid=1\n\tkey_mgmt=NONE\n\tpriority=10\n}\n")
        );
    }

    #[test]
    fn user_password_goes_to_stdin_only() {
        let p = plan(&r(UserAdd, &["bob", "Bob", "no", "s3cret-pw"])).unwrap();
        let AdminStep::RunInput { argv, input } = &p[1] else {
            panic!()
        };
        assert_eq!(input, "s3cret-pw\n");
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
    /// password (macOS). Everything system-wide still needs an administrator; restart and shut
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
            (Consent, false),
            (HwLoad, false),
            (RemoteLogin, false),
        ] {
            assert_eq!(is_user_level(a), user_level, "{a:?}");
        }
        let join = r(WifiJoin, &["wlan0", "Home", "psk"]);
        let off = r(SessionPower, &["shutdown"]);
        let sleep = r(SessionPower, &["sleep"]);
        for (req, uid, sessions, want) in [
            (&join, 1002, &[1002][..], true),
            (&join, 1002, &[1001, 1002][..], true),
            (&join, 1002, &[][..], false),
            (&join, 1002, &[1001][..], false),
            (&off, 1002, &[1002][..], true),
            (&off, 1002, &[1001, 1002][..], false),
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
        assert!(needs_restore_point(WolInstall) && needs_restore_point(WolRemove));
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
