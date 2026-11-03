// SPDX-License-Identifier: GPL-2.0-or-later
//! Pure parts of `wol-login` (IaraOS ADR 0024): passwd parsing, auth request framing, login flow, session plan.

pub use shared::login::SOCKET;
pub const GREETER: &str = "/usr/bin/wol-greeter-session";
pub const UI_USER: &str = "_wollogin";
pub const SESSION: &str = "/usr/bin/wol-session";
pub const CONFIG_FILE: &str = "/etc/wolos/login.json";
pub const PASSWD_FILE: &str = "/etc/passwd";
pub const SHELLS_FILE: &str = "/etc/shells";
pub const MAX_USER: usize = 32;
pub const MAX_PASSWORD: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginUser {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    /// First GECOS field, `&` = capitalised login name; login name if empty.
    pub full_name: String,
    pub home: String,
    pub shell: String,
}

pub fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_USER
        && !s.starts_with('-')
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-'))
}

pub fn display_name(name: &str, gecos: &str) -> String {
    let g = gecos.split(',').next().unwrap_or("");
    if g.is_empty() {
        return name.into();
    }
    let mut cap = name.to_string();
    if let Some(f) = cap.get_mut(0..1) {
        f.make_ascii_uppercase();
    }
    g.replace('&', &cap)
}

pub fn all_users(passwd: &str) -> Vec<LoginUser> {
    passwd
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            Some(LoginUser {
                name: f[0].into(),
                uid: f[2].parse().ok()?,
                gid: f[3].parse().ok()?,
                full_name: display_name(f[0], f[4]),
                home: f[5].into(),
                shell: f[6].into(),
            })
        })
        .collect()
}

/// Shown on the login window and accepted by [`LoginFlow::submit`]: [`shared::login::login_names`]
/// (one rule for both sides, audit5 S9-1), sorted by name.
pub fn listed(passwd: &str, shells: &str) -> Vec<LoginUser> {
    let names = shared::login::login_names(passwd, shells);
    let mut v: Vec<LoginUser> = all_users(passwd)
        .into_iter()
        .filter(|u| names.contains(&u.name))
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v.dedup_by(|a, b| a.name == b.name);
    v
}

pub fn find(passwd: &str, name: &str) -> Option<LoginUser> {
    all_users(passwd).into_iter().find(|u| u.name == name)
}

/// `{"autologin":"<name>"}`; anything else = no auto-login.
pub fn parse_config(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let a = v.get("autologin")?.as_str()?;
    valid_name(a).then(|| a.to_owned())
}

/// Login window -> wol-login: `user\n` then the password (one trailing `\n` dropped).
/// Returns user and password byte range; None if malformed.
pub fn parse_request(data: &[u8]) -> Option<(String, std::ops::Range<usize>)> {
    let nl = data.iter().position(|b| *b == b'\n')?;
    if nl == 0 || nl > MAX_USER {
        return None;
    }
    let user = std::str::from_utf8(&data[..nl]).ok()?;
    if !valid_name(user) {
        return None;
    }
    let mut end = data.len();
    if end > nl + 1 && data[end - 1] == b'\n' {
        end -= 1;
    }
    let pw = &data[nl + 1..end];
    if pw.len() > MAX_PASSWORD || pw.contains(&0) {
        return None;
    }
    Some((user.into(), nl + 1..end))
}

pub use shared::login::{Power, encode_power};

/// What wol-login (root) runs for each power request.
pub fn power_argv(p: Power) -> [&'static str; 3] {
    match p {
        Power::Sleep => ["/usr/sbin/acpiconf", "-s", "3"],
        Power::Restart => ["/sbin/shutdown", "-r", "now"],
        Power::ShutDown => ["/sbin/shutdown", "-p", "now"],
    }
}

/// One login socket request. Commands start with `!`, which no account name can.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// `user\n` + password; the password is this byte range of the request.
    Auth {
        user: String,
        password: std::ops::Range<usize>,
    },
    Power(Power),
    SetupCreateUser(NewAccount),
    SetupWifi {
        ssid: String,
        password: Option<String>,
    },
    /// Networks around (wol-login scans: `_wollogin` cannot reach wpa_supplicant).
    SetupWifiScan,
}

pub fn parse(data: &[u8]) -> Option<Request> {
    let Some((verb, body)) = shared::login::command(data) else {
        return parse_request(data).map(|(user, password)| Request::Auth { user, password });
    };
    match verb {
        shared::login::SETUP_CREATE_ACCOUNT | shared::login::SETUP_CREATE_USER => {
            NewAccount::decode_verb(verb, body).map(Request::SetupCreateUser)
        }
        shared::login::SETUP_WIFI => shared::login::decode_wifi(body)
            .map(|(ssid, password)| Request::SetupWifi { ssid, password }),
        shared::login::SETUP_WIFI_SCAN => body.is_empty().then_some(Request::SetupWifiScan),
        v => Power::from_verb(v)
            .filter(|_| body.is_empty())
            .map(Request::Power),
    }
}

/// Policy: power only while the greeter shows (no session, no login in flight).
/// Setup requests only while no login user exists (first boot, ADR 0004).
pub fn allowed(r: &Request, phase: Phase, users_exist: bool) -> bool {
    phase == Phase::Greeter
        && match r {
            Request::Auth { .. } | Request::Power(_) => true,
            Request::SetupCreateUser(_) | Request::SetupWifi { .. } | Request::SetupWifiScan => {
                !users_exist
            }
        }
}

pub use shared::login::NewAccount;

pub use shared::login::HINTS_DIR;
pub use wol_core::settings::users::validate_account;
use wol_core::theme::accent;

/// One privileged setup step, run by wol-login in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupStep {
    Run {
        argv: Vec<String>,
        stdin: String,
    },
    /// Whole file; `user_owned` files (and their new parent dirs) are chowned to the account.
    Write {
        path: String,
        content: String,
        user_owned: bool,
    },
    /// Directory owned by the account (new parents too), with `mode`.
    Dir {
        path: String,
        mode: u32,
    },
}

/// Account first (`useradd`, then the password on `chpasswd`'s stdin), then per-user and
/// system preferences.
pub fn setup_plan(a: &NewAccount) -> Vec<SetupStep> {
    let home = format!("/home/{}", a.name);
    let argv = wol_core::settings::users::add_argv(&a.name, &a.full_name, true).unwrap_or_default();
    let w = |path: String, content: String, user_owned: bool| SetupStep::Write {
        path,
        content,
        user_owned,
    };
    let mut v = vec![
        SetupStep::Run {
            argv,
            stdin: String::new(),
        },
        SetupStep::Run {
            argv: vec!["chpasswd".into()],
            stdin: format!("{}:{}\n", a.name, a.password),
        },
        // gnome-keyring creates the login keyring here at the first login (wol-login, wol-lock).
        SetupStep::Dir {
            path: format!("{home}/.local/share/keyrings"),
            mode: 0o700,
        },
        w(
            format!("{home}/.login_conf"),
            format!("me:\\\n\t:charset=UTF-8:\\\n\t:lang={}:\n", a.locale),
            true,
        ),
        w(
            format!("{home}/.config/wol/setup.json"),
            serde_json::json!({
                "locale": a.locale,
                "keymap": a.keymap,
                "appearance": a.appearance,
            })
            .to_string(),
            true,
        ),
    ];
    let variant = match a.appearance.as_str() {
        "light" => Some(shared::theme::Variant::Light),
        "dark" => Some(shared::theme::Variant::Dark),
        _ => None, // Auto: follows the appearance (a file only for another accent)
    };
    if let Some(id) = accent::theme_id(variant, &a.accent) {
        let config = std::path::PathBuf::from(format!("{home}/.config"));
        let file = wol_core::theme::active::default_file(&config);
        v.push(w(file.to_string_lossy().into_owned(), id, true));
    }
    if !a.hint.is_empty() {
        v.push(w(
            format!("{HINTS_DIR}/{}", a.name),
            format!("{}\n", a.hint),
            false,
        ));
    }
    if shared::login::AVATAR_TONES.contains(&a.avatar.as_str()) {
        v.push(w(
            format!("{}/{}", shared::login::AVATARS_DIR, a.name),
            format!("{}\n", a.avatar),
            false,
        ));
    }
    if a.appearance != "auto" {
        v.push(w(
            wol_core::settings::appearance::FILE.into(),
            a.appearance.clone(),
            false,
        ));
    }
    v
}

/// Runs `plan` under `root` (`/` in wol-login). `run` executes a [`SetupStep::Run`] (true = ok)
/// and `find` looks the new account up afterwards; nothing else happens when either fails. The
/// account owns its directories and `user_owned` files. Returns the new account.
pub fn apply_setup(
    plan: &[SetupStep],
    root: &std::path::Path,
    mut run: impl FnMut(&[String], &str) -> bool,
    find: impl Fn() -> Option<LoginUser>,
) -> Option<LoginUser> {
    let at = |p: &str| root.join(p.trim_start_matches('/'));
    let mut user: Option<LoginUser> = None;
    for step in plan {
        match step {
            SetupStep::Run { argv, stdin } => {
                if !run(argv, stdin) {
                    return None;
                }
                user = Some(find()?);
            }
            SetupStep::Dir { path, mode } => {
                let owner = user.as_ref().map(|u| (u.uid, u.gid));
                if let Err(e) = make_dir(&at(path), *mode, owner) {
                    eprintln!("wol-login: setup: {path}: {e}");
                }
            }
            SetupStep::Write {
                path,
                content,
                user_owned,
            } => {
                let owner = user
                    .as_ref()
                    .filter(|_| *user_owned)
                    .map(|u| (u.uid, u.gid));
                if let Err(e) = write_file(&at(path), content, owner) {
                    eprintln!("wol-login: setup: {path}: {e}");
                }
            }
        }
    }
    user
}

/// Creates `dir` (and missing parents, chowned with it to `owner`), then sets `mode`.
fn make_dir(dir: &std::path::Path, mode: u32, owner: Option<(u32, u32)>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut missing: Vec<&std::path::Path> = dir.ancestors().take_while(|d| !d.exists()).collect();
    std::fs::create_dir_all(dir)?;
    if let Some((u, g)) = owner {
        missing.reverse();
        for d in missing {
            std::os::unix::fs::chown(d, Some(u), Some(g))?;
        }
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode))
}

/// Writes `content` (mode 0644), creating parents; with `owner`, new parents and the file are chowned.
fn write_file(
    p: &std::path::Path,
    content: &str,
    owner: Option<(u32, u32)>,
) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = p.parent() {
        let mut missing: Vec<&std::path::Path> =
            dir.ancestors().take_while(|d| !d.exists()).collect();
        std::fs::create_dir_all(dir)?;
        if let Some((u, g)) = owner {
            missing.reverse();
            for d in missing {
                std::os::unix::fs::chown(d, Some(u), Some(g))?;
            }
        }
    }
    std::fs::write(p, content)?;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o644))?;
    if let Some((u, g)) = owner {
        std::os::unix::fs::chown(p, Some(u), Some(g))?;
    }
    Ok(())
}

/// The steps that open `name`'s home with the password just checked (audit S4-20). None: the home
/// is encrypted and closed but the password is not UTF-8 text, so homectl cannot be given it and
/// the session must not start without its files. A plain or already open home needs no steps.
pub fn home_unlock_steps(
    name: &str,
    state: Option<wol_core::home_crypt::State>,
    password: &[u8],
) -> Option<Vec<wol_core::home_crypt::Cmd>> {
    match std::str::from_utf8(password) {
        Ok(pw) => Some(wol_core::home_crypt::unlock(name, state, pw)),
        Err(_) => {
            let steps = wol_core::home_crypt::unlock(name, state, "");
            steps.is_empty().then_some(steps)
        }
    }
}

pub fn encode_request(user: &str, password: &[u8]) -> Vec<u8> {
    let mut b = format!("{user}\n").into_bytes();
    b.extend_from_slice(password);
    b
}

pub fn runtime_dir(u: &LoginUser) -> String {
    format!("/var/run/user/{}", u.uid)
}

/// Root helper that opens the user's PAM session (service [`SESSION_PAM_SERVICE`]: pam_systemd
/// makes the logind session, /run/user and user@.service) and runs a command in the user's login
/// shell: `wol-session-pam USER COMMAND`.
pub const SESSION_PAM: &str = "/usr/lib/wol-session-pam";
pub const SESSION_PAM_SERVICE: &std::ffi::CStr = c"wol-login";

/// `locked`: wol-session starts the compositor locked behind wol-lock (ADR 0012).
pub fn session_command(u: &LoginUser, locked: bool) -> Option<Vec<String>> {
    let lock = if locked { "WOL_SESSION_LOCKED=1 " } else { "" };
    valid_name(&u.name).then(|| {
        vec![
            SESSION_PAM.into(),
            u.name.clone(),
            format!("exec env {lock}{SESSION}"),
        ]
    })
}

pub const KEYRING_DAEMON: &str = "/usr/bin/gnome-keyring-daemon";

/// gnome-keyring at login: reads the password on stdin, unlocks (or creates) the login keyring once
/// the session runs `gnome-keyring-daemon --start` (hyprland.conf) within 120 s. Not the PAM keyring
/// module: wol-login hands the password over only after it accepted it (a typo never creates or
/// unlocks a keyring), and the login PAM stack does not depend on the keyring package (without the
/// daemon this is a no-op).
pub fn keyring_login_command(u: &LoginUser) -> Option<Vec<String>> {
    valid_name(&u.name).then(|| {
        vec![
            "/usr/bin/su".into(),
            "-l".into(),
            u.name.clone(),
            "-c".into(),
            format!(
                "exec env XDG_RUNTIME_DIR={} {KEYRING_DAEMON} --daemonize --login",
                runtime_dir(u)
            ),
        ]
    })
}

/// Logout: the daemon is bound to the ended session's D-Bus; the next login starts a fresh one.
pub fn keyring_stop_command(u: &LoginUser) -> Vec<String> {
    vec![
        "pkill".into(),
        "-U".into(),
        u.uid.to_string(),
        "-f".into(),
        KEYRING_DAEMON.into(),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Greeter,
    Authenticating,
    Session,
}

/// greeter -> authenticating -> session -> greeter. Auto-login once per flow, never after a logout.
/// One listed account: its session starts locked instead of the greeter (ADR 0012).
#[derive(Debug)]
pub struct LoginFlow {
    pub listed: Vec<LoginUser>,
    autologin: Option<LoginUser>,
    autologin_used: bool,
    pending: Option<LoginUser>,
    pub phase: Phase,
    pub user: Option<LoginUser>,
    /// The session starts behind its lock screen (no one authenticated yet).
    pub locked: bool,
    /// A locked start died before it could be unlocked: the login window until a session runs healthy.
    lock_broken: bool,
    /// The session comes from the setup assistant (its Done page is still up).
    from_setup: bool,
}

impl LoginFlow {
    pub fn new(listed: Vec<LoginUser>, autologin: Option<LoginUser>) -> Self {
        Self {
            listed,
            autologin,
            autologin_used: false,
            pending: None,
            phase: Phase::Greeter,
            user: None,
            locked: false,
            lock_broken: false,
            from_setup: false,
        }
    }

    /// Boot and every logout: auto-login (once), else the only account locked, else the greeter.
    pub fn start(&mut self) -> Phase {
        self.locked = false;
        self.from_setup = false;
        if self.autologin.is_some() && !self.autologin_used {
            self.autologin_used = true;
            self.phase = Phase::Session;
            self.user = self.autologin.clone();
        } else if let [only] = self.listed.as_slice()
            && !self.lock_broken
        {
            self.phase = Phase::Session;
            self.user = Some(only.clone());
            self.locked = true;
        } else {
            self.phase = Phase::Greeter;
            self.user = None;
        }
        self.phase
    }

    pub fn submit(&mut self, name: &str) -> bool {
        if self.phase != Phase::Greeter {
            return false;
        }
        self.pending = self
            .listed
            .iter()
            .find(|u| u.name == name)
            .or(self.autologin.as_ref().filter(|a| a.name == name))
            .cloned();
        if self.pending.is_none() {
            return false;
        }
        self.phase = Phase::Authenticating;
        self.user = self.pending.clone();
        true
    }

    pub fn authenticated(&mut self, ok: bool) -> Phase {
        if self.phase != Phase::Authenticating {
            return self.phase;
        }
        if ok {
            self.phase = Phase::Session;
            self.user = self.pending.clone();
        } else {
            self.phase = Phase::Greeter;
            self.user = None;
        }
        self.phase
    }

    /// Setup assistant created the first account: straight into its session.
    pub fn setup_done(&mut self, user: LoginUser) -> Phase {
        if self.phase == Phase::Greeter && self.listed.is_empty() {
            self.phase = Phase::Session;
            self.user = Some(user);
            self.from_setup = true;
        }
        self.phase
    }

    pub fn from_setup(&self) -> bool {
        self.from_setup
    }

    pub fn session_ended(&mut self) -> Phase {
        self.phase = Phase::Greeter;
        self.user = None;
        self.locked = false;
        self.phase
    }

    /// See [`failed_early`]: the login window takes over, with its backoff and console notice.
    pub fn locked_start_failed(&mut self) {
        self.lock_broken = true;
    }

    pub fn session_ran_healthy(&mut self) {
        self.lock_broken = false;
    }
}

/// A session that exited with an error before [`GreeterRespawn::HEALTHY`] (no GPU, lock screen
/// broken). Never a reason to start unlocked: a failed locked start falls back to the login window.
pub fn failed_early(ran: std::time::Duration, ok: bool) -> bool {
    !ok && ran < GreeterRespawn::HEALTHY
}

/// `!setup-wifi` answer once wpa_supplicant reloaded its networks (ADR 0013): Some(reply) when
/// decided, None to keep polling. The session's rules (`Controls::wait_joined`): a secured network
/// that refuses us is a wrong password; giving up after `JOIN_WAIT` is an error, never "ok".
pub fn wifi_reply(
    state: wol_core::wifi::JoinState,
    secured: bool,
    waited_out: bool,
) -> Option<&'static str> {
    use wol_core::wifi::JoinState;
    match state {
        JoinState::Joined => Some("ok"),
        JoinState::Refused if secured => Some("denied"),
        JoinState::Refused => Some("error"),
        JoinState::Trying if waited_out => Some("error"),
        JoinState::Trying => None,
    }
}

/// `!setup-wifi` answer from NetworkManager's join (`nmcli dev wifi connect` waits for the result):
/// a secured network that refuses us is a wrong password, anything else failing is an error.
pub fn nm_join_reply(joined: bool, secured: bool) -> &'static str {
    match (joined, secured) {
        (true, _) => "ok",
        (false, true) => "denied",
        (false, false) => "error",
    }
}

/// How long the greeter may take to leave on its own once the session is decided (ADR 0012): the
/// login window fades to black and exits; the setup assistant first shows Done until "Get Started".
/// Then it is stopped either way.
pub fn handoff_wait(from_setup: bool) -> std::time::Duration {
    std::time::Duration::from_secs(if from_setup { 600 } else { 2 })
}

/// The console VT. The login window and every session run on it: the seat daemon takes the VT
/// that is active at boot (tty1; no getty there or on any other VT, logind.conf.d/wolos.conf), and
/// the notice is drawn on it when the login window cannot start.
pub const CONSOLE: &str = "/dev/tty1";

/// Written to [`CONSOLE`] before each compositor starts (ADR 0012): when a compositor exits, the
/// seat daemon puts the VT back in text mode and the kernel redraws its text, so it must be plain
/// black, with no text and no cursor. Black on black too: later kernel console text stays
/// invisible. The kernel command line (`quiet`, `vt.global_cursor_default=0`) keeps the VT so from
/// the start of boot.
pub const BLANK_CONSOLE: &str = "\x1b[30;40m\x1b[?25l\x1b[H\x1b[2J";
pub const SHOW_CURSOR: &str = "\x1b[?25h";
/// Default colours again (the console notice must be readable).
pub const TEXT_VISIBLE: &str = "\x1b[0m";

/// Linux `VT_LOCKSWITCH` (`linux/vt.h`, `0x560B`): no argument, needs `CAP_SYS_TTY_CONFIG`, works
/// on any VT descriptor. The lock is global: while it is set the kernel ignores every VT switch,
/// Alt+Fn on a text VT and the seat daemon's `VT_ACTIVATE` for a compositor's Ctrl+Alt+Fn alike,
/// so nothing but WolOS is ever on screen (ADR 0012). Graphics mode (`KDSETMODE`), keyboard mode
/// and `VT_SETMODE` are the seat daemon's job when a compositor opens the seat; wol-login draws
/// nothing on the VT but escape sequences, so it never sets them.
pub const VT_LOCKSWITCH: libc::c_ulong = 0x560B;
/// Linux `VT_UNLOCKSWITCH` (`linux/vt.h`, `0x560C`): VT switching works again.
pub const VT_UNLOCKSWITCH: libc::c_ulong = 0x560C;

/// The ioctl that locks (true) or unlocks (false) VT switching.
pub fn vt_switch_request(lock: bool) -> libc::c_ulong {
    if lock { VT_LOCKSWITCH } else { VT_UNLOCKSWITCH }
}

/// Errors that mean there is no VT to manage: no such device (kernel without VTs, a container) or
/// a descriptor that is no VT (`ENOTTY`: a pty or a file, e.g. under a nested compositor).
pub fn no_vt(e: &std::io::Error) -> bool {
    matches!(
        e.raw_os_error(),
        Some(libc::ENOENT | libc::ENXIO | libc::ENODEV | libc::ENOTTY)
    )
}

/// What [`console_step`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum VtStep {
    /// Switching set and the text written.
    Done,
    /// No VT here: nothing to do, not an error.
    NoVt,
    /// Something to log (the reasons, `; ` between them).
    Failed(String),
}

/// One console step (ADR 0012): open the console VT, lock (before a compositor) or unlock (for the
/// notice) VT switching, then write `text`. `open` and `ioctl` are the device layer, so tests run
/// it on fakes. No VT at all is [`VtStep::NoVt`] and nothing is written; a failed ioctl (e.g.
/// `EPERM` without `CAP_SYS_TTY_CONFIG`) still writes the text, the screen matters more.
pub fn console_step<D: std::io::Write>(
    open: impl FnOnce() -> std::io::Result<D>,
    ioctl: impl FnOnce(&D, libc::c_ulong) -> std::io::Result<()>,
    lock: bool,
    text: &str,
) -> VtStep {
    let mut tty = match open() {
        Ok(t) => t,
        Err(e) if no_vt(&e) => return VtStep::NoVt,
        Err(e) => return VtStep::Failed(format!("open: {e}")),
    };
    let mut failed = Vec::new();
    let name = if lock {
        "VT_LOCKSWITCH"
    } else {
        "VT_UNLOCKSWITCH"
    };
    match ioctl(&tty, vt_switch_request(lock)) {
        Ok(()) => {}
        Err(e) if e.raw_os_error() == Some(libc::ENOTTY) => return VtStep::NoVt,
        Err(e) => failed.push(format!("{name}: {e}")),
    }
    if let Err(e) = tty.write_all(text.as_bytes()).and_then(|()| tty.flush()) {
        failed.push(format!("write: {e}"));
    }
    if failed.is_empty() {
        VtStep::Done
    } else {
        VtStep::Failed(failed.join("; "))
    }
}

/// The kernel's enabled consoles, `/dev/console` last (the service manager reads it the same way).
pub const CONSOLE_ACTIVE: &str = "/sys/class/tty/console/active";
/// Kernel command line: its last `console=` entry is `/dev/console` when [`CONSOLE_ACTIVE`] is
/// unreadable.
pub const KERNEL_CMDLINE: &str = "/proc/cmdline";

/// The device behind `/dev/console` (`ttyS0`, `tty0`, ...): the last name in
/// [`CONSOLE_ACTIVE`], else the last `console=` on the kernel command line (options after `,`
/// dropped). None when neither names one.
pub fn primary_console(active: Option<&str>, cmdline: Option<&str>) -> Option<String> {
    let last = |it: &mut dyn Iterator<Item = String>| it.last().filter(|c| !c.is_empty());
    active
        .and_then(|a| last(&mut a.split_whitespace().map(str::to_owned)))
        .or_else(|| {
            cmdline.and_then(|c| {
                last(&mut c.split_whitespace().filter_map(|w| {
                    w.strip_prefix("console=")
                        .map(|v| v.split(',').next().unwrap_or("").to_owned())
                }))
            })
        })
}

/// True when `/dev/console` is a serial port (VM, headless: `ttyS0`, `ttyAMA0`, `hvc0`, ...),
/// so boot marks may go to the console without touching the screen. A virtual terminal
/// (`tty` + digits; bare `tty` is no console) is the screen.
pub fn serial_primary(console: Option<&str>) -> bool {
    console.is_some_and(|c| {
        let vt = c
            .strip_prefix("tty")
            .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()));
        !c.is_empty() && !vt
    })
}

/// [`serial_primary`] for the consoles read from `active` and `cmdline` (paths, so tests can
/// point them at fixtures).
pub fn serial_console_at(active: &std::path::Path, cmdline: &std::path::Path) -> bool {
    let read = |p: &std::path::Path| std::fs::read_to_string(p).ok();
    serial_primary(primary_console(read(active).as_deref(), read(cmdline).as_deref()).as_deref())
}

pub use shared::login::boot_mark;

/// Compositor hand-offs: a spawn after an exit reports the gap in which the text VT showed.
#[derive(Debug, Default)]
pub struct Timeline {
    exited_at: Option<u64>,
}

impl Timeline {
    pub fn spawned(&mut self, uptime_ms: u64, what: &str) -> String {
        let gap = self
            .exited_at
            .take()
            .map(|t| format!(" (vt gap {} ms)", uptime_ms.saturating_sub(t)))
            .unwrap_or_default();
        boot_mark(uptime_ms, &format!("{what} spawn{gap}"))
    }

    pub fn exited(&mut self, uptime_ms: u64, what: &str) -> String {
        self.exited_at = Some(uptime_ms);
        boot_mark(uptime_ms, &format!("{what} exit"))
    }
}

/// Full-screen text for the console when the greeter keeps failing: what is wrong, the raw reason
/// from the greeter exit, where the details are, and how to restart (no VT has a getty, so there
/// is no console login to point to).
pub fn console_notice(dri_present: bool, status: &str) -> String {
    let headline = if dri_present {
        "The WolOS login window could not start."
    } else {
        "WolOS needs a supported graphics card (no /dev/dri: no GPU driver loaded)."
    };
    format!(
        "{TEXT_VISIBLE}{SHOW_CURSOR}\x1b[2J\x1b[H\r\n  WolOS\r\n\r\n  {headline}\r\n\r\n  Reason: login window exited, {status}.\r\n  Details: journalctl -b -u wold\r\n\r\n  Restart: press Ctrl+Alt+Delete, or hold the power button.\r\n"
    )
}

/// Greeter respawn policy: a greeter that dies quickly (e.g. the compositor without DRM) is retried with
/// exponential backoff (1 s .. 5 min) and logged once per failure streak, never spun.
#[derive(Debug, Default)]
pub struct GreeterRespawn {
    quick_exits: u32,
}

/// What to do after a greeter exit.
#[derive(Debug, PartialEq, Eq)]
pub struct Respawn {
    pub delay: std::time::Duration,
    pub log: bool,
}

impl GreeterRespawn {
    /// A greeter that ran at least this long counts as healthy and resets the backoff.
    pub const HEALTHY: std::time::Duration = std::time::Duration::from_secs(30);
    pub const MAX_DELAY: std::time::Duration = std::time::Duration::from_secs(300);

    pub fn exited(&mut self, ran: std::time::Duration) -> Respawn {
        if ran >= Self::HEALTHY {
            self.quick_exits = 0;
        }
        let n = self.quick_exits;
        self.quick_exits = n.saturating_add(1);
        let delay = std::time::Duration::from_secs(1u64 << n.min(16)).min(Self::MAX_DELAY);
        Respawn { delay, log: n == 0 }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn console_notice_names_the_reason() {
        let cases = [
            (
                false,
                "signal: 6 (SIGABRT)",
                "WolOS needs a supported graphics card",
            ),
            (
                true,
                "exit status: 1",
                "The WolOS login window could not start",
            ),
        ];
        for (dri, status, headline) in cases {
            let m = console_notice(dri, status);
            assert!(m.contains(headline), "{m}");
            assert!(m.contains(status), "{m}");
            // No VT has a getty (logind.conf.d/wolos.conf): no console login to point to, and
            // wold logs to the journal, not to a file.
            assert!(!m.contains("Alt+F"), "no console login exists: {m}");
            assert!(!m.contains("/var/log/"), "{m}");
            assert!(m.contains("journalctl -b -u wold"), "{m}");
            assert!(m.contains("Ctrl+Alt+Delete"), "how to restart: {m}");
            assert!(!m.contains("FreeBSD"), "{m}");
            assert!(
                m.starts_with(&format!("{TEXT_VISIBLE}{SHOW_CURSOR}")),
                "notice is readable over black-on-black boot text, with a cursor: {m:?}"
            );
        }
    }

    /// ADR 0012: between two compositors the kernel redraws the console VT's text; it must be black
    /// with no cursor, and any later kernel console text black on black.
    #[test]
    fn console_blank_clears_and_hides_the_cursor() {
        assert_eq!(BLANK_CONSOLE, "\x1b[30;40m\x1b[?25l\x1b[H\x1b[2J");
        assert_eq!(SHOW_CURSOR, "\x1b[?25h");
        assert_eq!(TEXT_VISIBLE, "\x1b[0m");
    }

    /// ADR 0012: the window that asked for the session fades to black and exits on its own; the
    /// setup assistant still shows its Done page (account created) until "Get Started".
    #[test]
    fn setup_wifi_answers_only_once_joined() {
        use wol_core::wifi::JoinState::*;
        assert_eq!(nm_join_reply(true, true), "ok");
        assert_eq!(nm_join_reply(false, true), "denied");
        assert_eq!(nm_join_reply(false, false), "error");
        assert_eq!(wifi_reply(Joined, true, false), Some("ok"));
        assert_eq!(
            wifi_reply(Refused, true, false),
            Some("denied"),
            "wrong password"
        );
        assert_eq!(wifi_reply(Refused, false, false), Some("error"));
        assert_eq!(wifi_reply(Trying, true, false), None, "keep polling");
        assert_eq!(
            wifi_reply(Trying, true, true),
            Some("error"),
            "never ok without joining"
        );
    }

    #[test]
    fn greeter_hands_off_after_its_own_fade() {
        use std::time::Duration;
        assert_eq!(handoff_wait(false), Duration::from_secs(2));
        assert_eq!(handoff_wait(true), Duration::from_secs(600));
        let ana = user("ana", 1001, "Ana", "/home/ana", "/bin/sh");
        let mut f = LoginFlow::new(vec![], None);
        assert_eq!(f.start(), Phase::Greeter);
        assert!(!f.from_setup());
        assert_eq!(f.setup_done(ana), Phase::Session);
        assert!(f.from_setup());
        f.session_ended();
        f.start();
        assert!(!f.from_setup());
    }

    /// linux/vt.h: `#define VT_LOCKSWITCH 0x560B`, `#define VT_UNLOCKSWITCH 0x560C` ('V' << 8 | n).
    /// The old base's `_IOW('v', 9, int)` (0x80047609) is no VT ioctl on Linux.
    #[test]
    fn vt_lock_requests_match_linux_vt_h() {
        let v = |n: libc::c_ulong| libc::c_ulong::from(b'V') << 8 | n;
        assert_eq!((VT_LOCKSWITCH, VT_UNLOCKSWITCH), (v(0x0B), v(0x0C)));
        assert_eq!(vt_switch_request(true), 0x560B);
        assert_eq!(vt_switch_request(false), 0x560C);
        assert_ne!(VT_LOCKSWITCH, 0x8004_7609);
    }

    /// Fake console VT: every ioctl and write lands in one ordered log; `write_err` makes writes
    /// fail with that errno ([`step_on_fake`] fakes open and ioctl errors).
    #[derive(Default)]
    struct FakeVt {
        log: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
        write_err: Option<i32>,
    }

    impl std::io::Write for FakeVt {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if let Some(e) = self.write_err {
                return Err(std::io::Error::from_raw_os_error(e));
            }
            let t = String::from_utf8_lossy(b).into_owned();
            self.log.borrow_mut().push(format!("write {t:?}"));
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Runs [`console_step`] on a [`FakeVt`]: (result, log).
    fn step_on_fake(
        open_err: Option<i32>,
        ioctl_err: Option<i32>,
        write_err: Option<i32>,
        lock: bool,
        text: &str,
    ) -> (VtStep, Vec<String>) {
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let vt = FakeVt {
            log: log.clone(),
            write_err,
        };
        let open = || match open_err {
            Some(e) => Err(std::io::Error::from_raw_os_error(e)),
            None => Ok(vt),
        };
        let ioctl = |d: &FakeVt, req: libc::c_ulong| {
            d.log.borrow_mut().push(format!("ioctl {req:#x}"));
            ioctl_err.map_or(Ok(()), |e| Err(std::io::Error::from_raw_os_error(e)))
        };
        let got = console_step(open, ioctl, lock, text);
        let log = log.borrow().clone();
        (got, log)
    }

    /// ADR 0012: before a compositor, switching is locked and the VT blanked; for the notice it is
    /// unlocked first, then the notice drawn.
    #[test]
    fn console_step_locks_before_a_compositor_and_unlocks_for_the_notice() {
        let (got, log) = step_on_fake(None, None, None, true, BLANK_CONSOLE);
        assert_eq!(got, VtStep::Done);
        assert_eq!(
            log,
            [
                "ioctl 0x560b".to_owned(),
                format!("write {BLANK_CONSOLE:?}")
            ]
        );
        let notice = console_notice(true, "exit status: 1");
        let (got, log) = step_on_fake(None, None, None, false, &notice);
        assert_eq!(got, VtStep::Done);
        assert_eq!(
            log,
            ["ioctl 0x560c".to_owned(), format!("write {notice:?}")]
        );
    }

    /// No VT at all (kernel without VTs, a container, a nested compositor's pty): a quiet no-op.
    #[test]
    fn console_step_without_a_vt_is_a_no_op() {
        for e in [libc::ENOENT, libc::ENXIO, libc::ENODEV, libc::ENOTTY] {
            for lock in [true, false] {
                let (got, log) = step_on_fake(Some(e), None, None, lock, BLANK_CONSOLE);
                assert_eq!(got, VtStep::NoVt, "open errno {e}");
                assert!(log.is_empty(), "nothing touched: {log:?}");
            }
        }
        // The device opens but is no VT: nothing written to it either.
        let (got, log) = step_on_fake(None, Some(libc::ENOTTY), None, true, BLANK_CONSOLE);
        assert_eq!(got, VtStep::NoVt);
        assert_eq!(log, ["ioctl 0x560b"]);
    }

    /// Real failures are reported; the text still reaches the screen when only the ioctl failed.
    #[test]
    fn console_step_reports_real_failures() {
        let (got, log) = step_on_fake(None, Some(libc::EPERM), None, true, BLANK_CONSOLE);
        let VtStep::Failed(why) = got else {
            panic!("EPERM is a failure: {got:?}")
        };
        assert!(why.starts_with("VT_LOCKSWITCH: "), "{why}");
        assert_eq!(log.len(), 2, "blank written anyway: {log:?}");
        let (got, _) = step_on_fake(None, Some(libc::EPERM), None, false, "x");
        assert!(
            matches!(&got, VtStep::Failed(w) if w.starts_with("VT_UNLOCKSWITCH: ")),
            "{got:?}"
        );
        let (got, log) = step_on_fake(Some(libc::EACCES), None, None, true, BLANK_CONSOLE);
        assert!(
            matches!(&got, VtStep::Failed(w) if w.starts_with("open: ")),
            "{got:?}"
        );
        assert!(log.is_empty());
        let (got, log) = step_on_fake(None, None, Some(libc::EIO), true, BLANK_CONSOLE);
        assert!(
            matches!(&got, VtStep::Failed(w) if w.starts_with("write: ")),
            "{got:?}"
        );
        assert_eq!(log, ["ioctl 0x560b"], "locked even when the write fails");
    }

    #[test]
    fn primary_console_is_the_last_active_one() {
        for (active, cmdline, want) in [
            (Some("tty0 ttyS0\n"), None, Some("ttyS0")),
            (Some("ttyS0 tty0\n"), None, Some("tty0")),
            (Some("tty1\n"), Some("console=ttyS0"), Some("tty1")),
            (
                None,
                Some("quiet console=tty0 console=ttyS0,115200n8 splash"),
                Some("ttyS0"),
            ),
            (Some("\n"), Some("console=hvc0"), Some("hvc0")),
            (None, Some("quiet splash"), None),
            (None, None, None),
        ] {
            assert_eq!(
                primary_console(active, cmdline).as_deref(),
                want,
                "{active:?} {cmdline:?}"
            );
        }
    }

    #[test]
    fn serial_primary_tells_serial_from_the_screen() {
        for (c, want) in [
            (Some("ttyS0"), true),
            (Some("ttyAMA0"), true),
            (Some("hvc0"), true),
            (Some("ttyUSB0"), true),
            (Some("tty0"), false),
            (Some("tty12"), false),
            (Some("tty"), false),
            (Some(""), false),
            (None, false),
        ] {
            assert_eq!(serial_primary(c), want, "{c:?}");
        }
    }

    /// Reads the real files' format from fixture paths; a missing active list falls back to
    /// the command line.
    #[test]
    fn serial_console_at_reads_fake_paths() {
        let dir = std::env::temp_dir().join(format!("wol-login-console-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (active, cmdline) = (dir.join("active"), dir.join("cmdline"));
        std::fs::write(&cmdline, "root=UUID=x console=ttyS0,115200\n").unwrap();
        assert!(serial_console_at(&active, &cmdline), "cmdline fallback");
        std::fs::write(&active, "ttyS0 tty0\n").unwrap();
        assert!(!serial_console_at(&active, &cmdline), "active wins");
        std::fs::write(&active, "tty0 ttyS0\n").unwrap();
        assert!(serial_console_at(&active, &cmdline));
        assert!(!serial_console_at(&dir.join("none"), &dir.join("none")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Boot timeline (ADR 0012): one grep-able line per stage, uptime in ms; hops carry the gap
    /// between one compositor's exit and the next one's start.
    #[test]
    fn boot_marks_are_timestamped_and_hops_measured() {
        assert_eq!(
            boot_mark(12_345, "greeter spawn"),
            "wolos-boot: 12345 ms greeter spawn"
        );
        let mut t = Timeline::default();
        assert_eq!(
            t.spawned(1_000, "greeter"),
            "wolos-boot: 1000 ms greeter spawn"
        );
        assert_eq!(
            t.exited(9_000, "greeter"),
            "wolos-boot: 9000 ms greeter exit"
        );
        assert_eq!(
            t.spawned(9_420, "session"),
            "wolos-boot: 9420 ms session spawn (vt gap 420 ms)"
        );
    }

    #[test]
    fn greeter_respawn_backs_off_and_logs_once() {
        use std::time::Duration as D;
        let mut r = GreeterRespawn::default();
        // (ran for, expected delay, log this exit)
        let quick = D::from_secs(1);
        let cases = [
            (quick, 1, true),
            (quick, 2, false),
            (quick, 4, false),
            (quick, 8, false),
            (quick, 16, false),
            (quick, 32, false),
            (quick, 64, false),
            (quick, 128, false),
            (quick, 256, false),
            (quick, 300, false),
            (quick, 300, false),
            // a greeter that stayed up resets the backoff and logs again
            (D::from_secs(120), 1, true),
            (quick, 2, false),
        ];
        for (i, (ran, secs, log)) in cases.into_iter().enumerate() {
            let next = r.exited(ran);
            assert_eq!(
                (next.delay, next.log),
                (D::from_secs(secs), log),
                "case {i}"
            );
        }
    }

    use super::*;

    const PASSWD: &str = "# comment
root:*:0:0:Charlie &:/root:/bin/sh
nobody:*:65534:65534:Unprivileged user:/nonexistent:/usr/sbin/nologin
_wollogin:*:911:911:WolOS login window:/var/empty:/usr/sbin/nologin
zoe:*:1002:1002:Zoe Lima,,,:/home/zoe:/usr/bin/zsh
ana:*:1001:1001:&:/home/ana:/bin/sh
bot:*:1003:1003:Bot:/home/bot:/usr/sbin/nologin
broken:line
";
    const SHELLS: &str = "/bin/sh\n/bin/csh\n/usr/bin/zsh\n";

    fn user(name: &str, uid: u32, full: &str, home: &str, shell: &str) -> LoginUser {
        LoginUser {
            name: name.into(),
            uid,
            gid: uid,
            full_name: full.into(),
            home: home.into(),
            shell: shell.into(),
        }
    }

    #[test]
    fn the_listed_accounts_are_the_accepted_ones_without_etc_shells() {
        // audit5 S9-1: the login window lists exactly what wol-login accepts.
        let pw = "root:*:0:0::/root:/bin/sh\n\
                  bo:*:1002:1002:Bo Peep:/home/bo:/usr/bin/fish\n\
                  svc:*:1003:1003::/var/svc:/usr/sbin/nologin\n";
        for shells in ["", "/bin/sh\n", "/usr/bin/fish\n"] {
            let l = listed(pw, shells);
            assert_eq!(
                l.iter().map(|u| u.name.clone()).collect::<Vec<_>>(),
                shared::login::login_names(pw, shells),
                "{shells:?}"
            );
            assert_eq!(
                l.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(),
                ["bo"]
            );
            assert!(!LoginFlow::new(l.clone(), None).submit("svc"));
            let mut f = LoginFlow::new(l, None);
            assert!(f.submit("bo"), "{shells:?}");
            assert_eq!(f.user.as_ref().map(|u| u.name.as_str()), Some("bo"));
        }
    }

    #[test]
    fn lists_human_users_with_valid_shell_sorted() {
        let u = listed(PASSWD, SHELLS);
        assert_eq!(
            u.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(),
            ["ana", "zoe"]
        );
        assert_eq!(
            u[1],
            user("zoe", 1002, "Zoe Lima", "/home/zoe", "/usr/bin/zsh")
        );
    }

    #[test]
    fn finds_any_account_with_display_name() {
        for (name, want) in [
            ("root", Some("Charlie Root")),
            ("ana", Some("Ana")),
            ("bot", Some("Bot")),
            ("missing", None),
        ] {
            assert_eq!(
                find(PASSWD, name).map(|u| u.full_name),
                want.map(String::from),
                "{name}"
            );
        }
    }

    #[test]
    fn parses_login_config() {
        for (json, want) in [
            (r#"{"autologin":"root"}"#, Some("root")),
            (r#"{"autologin":""}"#, None),
            ("{}", None),
            ("not json", None),
            (r#"{"autologin":"a b;rm"}"#, None),
        ] {
            assert_eq!(parse_config(json).as_deref(), want, "{json}");
        }
    }

    #[test]
    fn parses_auth_requests() {
        for (raw, u, pw) in [
            ("ana\nsecret\n", "ana", "secret"),
            ("ana\nsecret", "ana", "secret"),
            ("ana\n", "ana", ""),
            ("ana\nline1\nline2\n", "ana", "line1\nline2"),
        ] {
            let (got, r) = parse_request(raw.as_bytes()).unwrap();
            assert_eq!((got.as_str(), &raw[r]), (u, pw));
        }
        let long_user = format!("{}\npw", "a".repeat(33));
        let long_pw = format!("ana\n{}", "p".repeat(513));
        for bad in [
            "",
            "ana",
            "\npw",
            "-ana\npw",
            "a/b\npw",
            "ana\npw\0x",
            &long_user,
            &long_pw,
        ] {
            assert!(parse_request(bad.as_bytes()).is_none(), "{bad:?}");
        }
        let (u, r) = parse_request(&encode_request("ana", b"pw")).unwrap();
        assert_eq!((u.as_str(), r), ("ana", 4..6));
    }

    #[test]
    fn parses_power_requests() {
        for (raw, want) in [
            ("!sleep\n", Some(Power::Sleep)),
            ("!restart", Some(Power::Restart)),
            ("!shutdown\n", Some(Power::ShutDown)),
            ("!reboot\n", None),
            ("!shutdown\nextra", None),
        ] {
            let got = match parse(raw.as_bytes()) {
                Some(Request::Power(p)) => Some(p),
                _ => None,
            };
            assert_eq!(got, want, "{raw:?}");
        }
        // A user called "sleep" still authenticates: commands carry a `!` no name can start with.
        assert!(matches!(parse(b"sleep\npw"), Some(Request::Auth { user, .. }) if user == "sleep"));
        assert_eq!(power_argv(Power::ShutDown), ["/sbin/shutdown", "-p", "now"]);
        assert_eq!(power_argv(Power::Restart), ["/sbin/shutdown", "-r", "now"]);
        assert_eq!(power_argv(Power::Sleep), ["/usr/sbin/acpiconf", "-s", "3"]);
        assert_eq!(encode_power(Power::Sleep), b"!sleep\n");
    }

    #[test]
    fn power_only_from_the_greeter() {
        let r = Request::Power(Power::ShutDown);
        assert!(allowed(&r, Phase::Greeter, true));
        assert!(allowed(&r, Phase::Greeter, false));
        assert!(!allowed(&r, Phase::Authenticating, true));
        assert!(!allowed(&r, Phase::Session, true));
    }

    fn account() -> NewAccount {
        NewAccount {
            name: "alice".into(),
            full_name: "Alice Liddell".into(),
            hint: "white rabbit".into(),
            locale: "en_GB.UTF-8".into(),
            keymap: "gb".into(),
            appearance: "dark".into(),
            avatar: "green".into(),
            accent: "blue".into(),
            password: "curiouser".into(),
        }
    }

    #[test]
    fn parses_setup_requests() {
        assert_eq!(
            parse(&account().encode()),
            Some(Request::SetupCreateUser(account()))
        );
        // ADR 0004 verb: no avatar, default accent.
        let old = b"!setup-create-user\nalice\nAlice\n\nen_US.UTF-8\nus\nauto\npw";
        let Some(Request::SetupCreateUser(a)) = parse(old) else {
            panic!("0004 request")
        };
        assert_eq!((a.avatar.as_str(), a.accent.as_str()), ("", "violet"));
        assert_eq!(
            parse(&shared::login::encode_wifi_scan()),
            Some(Request::SetupWifiScan)
        );
        assert_eq!(parse(b"!setup-wifi-scan\nextra"), None);
        assert_eq!(
            parse(&shared::login::encode_wifi("Home", "")),
            Some(Request::SetupWifi {
                ssid: "Home".into(),
                password: None
            })
        );
    }

    #[test]
    fn validates_new_accounts() {
        assert_eq!(validate_account(&account()), Ok(()));
        type Edit = fn(&mut NewAccount);
        let bad: [(&str, Edit); 9] = [
            ("name", |a| a.name = "Alice".into()),
            ("name", |a| a.name = "root".into()),
            ("full name", |a| a.full_name = "a:b".into()),
            ("hint", |a| a.hint = "curiouser".into()),
            ("locale", |a| a.locale = "en_GB".into()),
            ("keymap", |a| a.keymap = "us;rm".into()),
            ("appearance", |a| a.appearance = "pink".into()),
            ("password", |a| a.password = "".into()),
            ("password", |a| a.password = "a\0b".into()),
        ];
        for (why, f) in bad {
            let mut a = account();
            f(&mut a);
            assert_eq!(validate_account(&a), Err(why), "{a:?}");
        }
        let mut auto = account();
        auto.appearance = "auto".into();
        auto.hint = "".into();
        auto.keymap = "us(intl)".into();
        assert_eq!(validate_account(&auto), Ok(()));
    }

    #[test]
    fn setup_only_while_no_user_exists() {
        for r in [
            Request::SetupCreateUser(account()),
            Request::SetupWifi {
                ssid: "Home".into(),
                password: None,
            },
            Request::SetupWifiScan,
        ] {
            assert!(allowed(&r, Phase::Greeter, false));
            assert!(!allowed(&r, Phase::Greeter, true));
            assert!(!allowed(&r, Phase::Session, false));
        }
    }

    /// The Setup Assistant's account: administrator (wheel), zsh, and a private keyring dir so the
    /// login keyring is created at the first login (ADR 0013).
    #[test]
    fn setup_account_is_an_admin_with_zsh_and_a_keyring_dir() {
        let root = std::env::temp_dir().join(format!("wol-setup-apply-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // SAFETY: getuid/getgid have no preconditions.
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        let me = user("alice", uid, "Alice Liddell", "/home/alice", "/usr/bin/zsh");
        let mut ran = Vec::new();
        let created = apply_setup(
            &setup_plan(&account()),
            &root,
            |argv, stdin| {
                ran.push((argv.join(" "), stdin.to_string()));
                true
            },
            || Some(LoginUser { gid, ..me.clone() }),
        );
        assert_eq!(created.map(|u| u.name), Some("alice".into()));
        let (argv, _) = &ran[0];
        assert!(argv.starts_with("useradd -m "), "{argv}");
        assert!(argv.ends_with(" alice"), "{argv}");
        assert!(argv.contains(" -s /usr/bin/zsh "), "zsh: {argv}");
        assert!(
            argv.contains(" -G wheel,seat,video,input,audio "),
            "administrator (wheel): {argv}"
        );
        assert_eq!(
            ran[1],
            ("chpasswd".to_string(), "alice:curiouser\n".to_string()),
            "password on stdin, never in argv"
        );
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let keyrings = root.join("home/alice/.local/share/keyrings");
        let m = std::fs::metadata(&keyrings).expect("keyring dir");
        assert!(m.is_dir());
        assert_eq!(m.permissions().mode() & 0o777, 0o700);
        assert_eq!((m.uid(), m.gid()), (uid, gid));
        for f in [
            "home/alice/.config/wol/setup.json",
            "home/alice/.config/wolos/theme",
            "var/db/wolos/avatars/alice",
        ] {
            assert!(root.join(f).is_file(), "{f}");
        }
        let _ = std::fs::remove_dir_all(&root);
        // pw failed: nothing else is written.
        let root2 = root.with_extension("fail");
        assert!(apply_setup(&setup_plan(&account()), &root2, |_, _| false, || None).is_none());
        assert!(!root2.join("home/alice").exists());
    }

    #[test]
    fn setup_plan_creates_user_then_writes_prefs() {
        let p = setup_plan(&account());
        assert_eq!(
            p[0],
            SetupStep::Run {
                argv: [
                    "useradd",
                    "-m",
                    "-c",
                    "Alice Liddell",
                    "-s",
                    "/usr/bin/zsh",
                    "-G",
                    "wheel,seat,video,input,audio",
                    "alice"
                ]
                .map(String::from)
                .to_vec(),
                stdin: String::new(),
            }
        );
        assert_eq!(
            p[2],
            SetupStep::Dir {
                path: "/home/alice/.local/share/keyrings".into(),
                mode: 0o700,
            }
        );
        let writes: Vec<(&str, &str, bool)> = p[3..]
            .iter()
            .map(|s| match s {
                SetupStep::Write {
                    path,
                    content,
                    user_owned,
                } => (path.as_str(), content.as_str(), *user_owned),
                _ => panic!("{s:?}"),
            })
            .collect();
        assert_eq!(
            writes,
            [
                (
                    "/home/alice/.login_conf",
                    "me:\\\n\t:charset=UTF-8:\\\n\t:lang=en_GB.UTF-8:\n",
                    true
                ),
                (
                    "/home/alice/.config/wol/setup.json",
                    r#"{"appearance":"dark","keymap":"gb","locale":"en_GB.UTF-8"}"#,
                    true
                ),
                ("/home/alice/.config/wolos/theme", "wol-dark-blue", true),
                ("/var/db/wolos/hints/alice", "white rabbit\n", false),
                ("/var/db/wolos/avatars/alice", "green\n", false),
                ("/etc/wolos/appearance", "dark", false),
            ]
        );
        // Auto + default accent + no hint/avatar: account, password, keyring dir, login_conf, setup.json.
        let mut auto = account();
        auto.appearance = "auto".into();
        auto.hint = "".into();
        auto.avatar = "".into();
        auto.accent = "violet".into();
        assert_eq!(setup_plan(&auto).len(), 5);
        // Auto with another accent follows the appearance in that accent; unknown values are dropped.
        auto.accent = "blue".into();
        let theme = |a: &NewAccount| {
            setup_plan(a).into_iter().find_map(|s| match s {
                SetupStep::Write { path, content, .. } if path.ends_with("/wolos/theme") => {
                    Some(content)
                }
                _ => None,
            })
        };
        assert_eq!(theme(&auto).as_deref(), Some("wol-auto-blue"));
        auto.accent = "neon;rm".into();
        auto.avatar = "../../etc".into();
        assert_eq!(theme(&auto), None);
        assert_eq!(
            setup_plan(&auto).len(),
            5,
            "unknown avatar colour not written"
        );
    }

    #[test]
    fn setup_done_goes_straight_to_the_session() {
        let ana = user("ana", 1001, "Ana", "/home/ana", "/bin/sh");
        let mut f = LoginFlow::new(vec![], None);
        f.start();
        assert_eq!(f.setup_done(ana.clone()), Phase::Session);
        assert_eq!(f.user, Some(ana.clone()));
        assert!(!f.locked, "the account was just created with its password");
        let zoe = user("zoe", 1002, "Zoe Lima", "/home/zoe", "/bin/sh");
        let mut g = LoginFlow::new(vec![ana.clone(), zoe], None);
        g.start();
        assert_eq!(g.setup_done(ana), Phase::Greeter);
    }

    /// ADR 0012: one account = no greeter hop; its session starts behind the lock screen. Auto-login
    /// (once per boot), the setup assistant and the login window start it unlocked.
    #[test]
    fn single_account_starts_behind_its_lock_screen() {
        let ana = user("ana", 1001, "Ana", "/home/ana", "/bin/sh");
        let zoe = user("zoe", 1002, "Zoe Lima", "/home/zoe", "/bin/sh");
        let (locked, open) = (Some(true), Some(false));
        // (listed, autologin, (phase, locked) at boot, after the first logout)
        let cases = [
            (vec![ana.clone()], None, locked, locked),
            (vec![ana.clone()], Some(ana.clone()), open, locked),
            (vec![ana.clone(), zoe.clone()], None, None, None),
            (
                vec![ana.clone(), zoe.clone()],
                Some(zoe.clone()),
                open,
                None,
            ),
            (vec![], None, None, None),
        ];
        let step = |f: &mut LoginFlow| match f.start() {
            Phase::Session => Some(f.locked),
            p => {
                assert_eq!(p, Phase::Greeter);
                assert!(!f.locked);
                None
            }
        };
        for (i, (listed, auto, boot, relogin)) in cases.into_iter().enumerate() {
            let mut f = LoginFlow::new(listed.clone(), auto);
            assert_eq!(step(&mut f), boot, "case {i} boot");
            if boot == locked {
                assert_eq!(
                    f.user.as_ref(),
                    listed.first(),
                    "case {i}: the only account"
                );
            }
            f.session_ended();
            assert!(!f.locked, "case {i}");
            assert_eq!(step(&mut f), relogin, "case {i} after logout");
        }
        // Login window: the password was just checked; the session starts unlocked.
        let mut g = LoginFlow::new(vec![ana.clone(), zoe], None);
        g.start();
        g.submit("ana");
        assert_eq!(g.authenticated(true), Phase::Session);
        assert!(!g.locked);
    }

    #[test]
    fn a_locked_start_that_dies_early_hands_over_to_the_login_window() {
        use std::time::Duration;
        let quick = Duration::from_secs(3);
        let healthy = GreeterRespawn::HEALTHY;
        // (ran, exit ok) -> died before anyone could unlock it
        for (ran, ok, early) in [
            (quick, false, true),
            (quick, true, false), // a quick logout is not a failure
            (healthy, false, false),
            (healthy, true, false),
        ] {
            assert_eq!(failed_early(ran, ok), early, "{ran:?} {ok}");
        }
        let ana = user("ana", 1001, "Ana", "/home/ana", "/bin/sh");
        let mut f = LoginFlow::new(vec![ana.clone()], None);
        assert_eq!(f.start(), Phase::Session);
        assert!(f.locked);
        f.locked_start_failed();
        f.session_ended();
        // The login window (backoff, console notice) until a session runs healthy again.
        assert_eq!(f.start(), Phase::Greeter);
        assert!(f.submit("ana"));
        assert_eq!(f.authenticated(true), Phase::Session);
        assert!(!f.locked);
        f.session_ran_healthy();
        f.session_ended();
        assert_eq!(f.start(), Phase::Session);
        assert!(f.locked);
    }

    #[test]
    fn session_opens_a_pam_session_as_the_user() {
        // wol-session-pam: PAM session (pam_systemd: logind session, /run/user, user@.service with
        // PipeWire and the portals), then the user's login shell runs wol-session.
        let zoe = user("zoe", 1002, "Zoe Lima", "/home/zoe", "/usr/bin/zsh");
        assert_eq!(runtime_dir(&zoe), "/var/run/user/1002");
        let argv = |env: &str| {
            [
                SESSION_PAM.to_string(),
                "zoe".into(),
                format!("exec env {env}/usr/bin/wol-session"),
            ]
        };
        assert_eq!(SESSION_PAM, "/usr/lib/wol-session-pam");
        assert_eq!(session_command(&zoe, false).unwrap(), argv(""));
        // Locked start (ADR 0012): wol-session starts the compositor locked with wol-lock.
        assert_eq!(
            session_command(&zoe, true).unwrap(),
            argv("WOL_SESSION_LOCKED=1 ")
        );
        assert!(session_command(&user("a;b", 1, "", "", ""), false).is_none());
    }

    #[test]
    fn keyring_unlocks_from_the_login_password_in_the_session_runtime_dir() {
        // Geary/Calendar nagged for the "login" keyring: the daemon must get the password at login and
        // listen where the session looks ($XDG_RUNTIME_DIR/keyring), then stop at logout.
        let zoe = user("zoe", 1002, "Zoe Lima", "/home/zoe", "/usr/bin/zsh");
        assert_eq!(
            keyring_login_command(&zoe).unwrap(),
            [
                "/usr/bin/su",
                "-l",
                "zoe",
                "-c",
                "exec env XDG_RUNTIME_DIR=/var/run/user/1002 /usr/bin/gnome-keyring-daemon --daemonize --login"
            ]
        );
        assert!(keyring_login_command(&user("a;b", 1, "", "", "")).is_none());
        assert_eq!(
            keyring_stop_command(&zoe),
            ["pkill", "-U", "1002", "-f", KEYRING_DAEMON]
        );
    }

    #[test]
    fn greeter_flow() {
        let ana = user("ana", 1001, "Ana", "/home/ana", "/bin/sh");
        let zoe = user("zoe", 1002, "Zoe Lima", "/home/zoe", "/bin/sh");
        let mut f = LoginFlow::new(vec![ana.clone(), zoe], None);
        assert_eq!(f.start(), Phase::Greeter);
        assert!(!f.submit("root"));
        assert!(f.submit("ana"));
        assert_eq!(f.phase, Phase::Authenticating);
        assert!(!f.submit("ana"), "one at a time");
        assert_eq!(f.authenticated(false), Phase::Greeter);
        assert!(f.user.is_none());
        f.submit("ana");
        assert_eq!(f.authenticated(true), Phase::Session);
        assert_eq!(f.user, Some(ana));
        assert_eq!(f.session_ended(), Phase::Greeter);
    }

    #[test]
    fn autologin_once_never_after_logout_and_from_greeter() {
        let ana = user("ana", 1001, "Ana", "/home/ana", "/bin/sh");
        let root = user("root", 0, "Charlie Root", "/root", "/bin/sh");
        let zoe = user("zoe", 1002, "Zoe Lima", "/home/zoe", "/bin/sh");
        let mut f = LoginFlow::new(vec![ana.clone(), zoe], Some(root.clone()));
        assert_eq!(f.start(), Phase::Session);
        assert_eq!(f.user, Some(root));
        assert_eq!(f.session_ended(), Phase::Greeter);
        assert_eq!(f.start(), Phase::Greeter);
        assert!(f.submit("root"));
        let mut g = LoginFlow::new(vec![], None);
        g.start();
        assert!(!g.submit("ana"));
        g.listed = vec![ana];
        assert!(g.submit("ana"));
    }

    /// Audit S4-20: a password that is not text never starts a session over a closed encrypted
    /// home; a plain or open home needs nothing either way.
    #[test]
    fn a_non_text_password_does_not_skip_the_home_unlock() {
        use wol_core::home_crypt::State;
        let closed = Some(State {
            encrypted: true,
            key_loaded: false,
            mounted: false,
        });
        let open = Some(State {
            encrypted: true,
            key_loaded: true,
            mounted: true,
        });
        let bad = b"pass\xff\xfe";
        assert_eq!(super::home_unlock_steps("ana", closed, bad), None);
        assert_eq!(super::home_unlock_steps("ana", open, bad), Some(vec![]));
        assert_eq!(super::home_unlock_steps("ana", None, bad), Some(vec![]));
        let steps = super::home_unlock_steps("ana", closed, b"s3cret-pw").unwrap();
        assert!(!steps.is_empty());
    }
}
