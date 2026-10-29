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
const FIRST_HUMAN: u32 = 1000;
const NOBODY: u32 = 65534;

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

/// Shown on the login window: uid >= 1000 (not nobody) with a shell from /etc/shells, sorted by name.
pub fn listed(passwd: &str, shells: &str) -> Vec<LoginUser> {
    let valid: Vec<&str> = shells
        .lines()
        .map(str::trim)
        .filter(|s| s.starts_with('/'))
        .collect();
    let mut v: Vec<LoginUser> = all_users(passwd)
        .into_iter()
        .filter(|u| u.uid >= FIRST_HUMAN && u.uid != NOBODY && valid.contains(&u.shell.as_str()))
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
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

pub fn encode_request(user: &str, password: &[u8]) -> Vec<u8> {
    let mut b = format!("{user}\n").into_bytes();
    b.extend_from_slice(password);
    b
}

pub fn runtime_dir(u: &LoginUser) -> String {
    format!("/var/run/user/{}", u.uid)
}

/// `su -l` as root applies setusercontext(LOGIN_SETALL): groups, uid, login.conf env, HOME, cwd.
/// `locked`: wol-session starts Hyprland locked behind wol-lock (ADR 0012).
pub fn session_command(u: &LoginUser, locked: bool) -> Option<Vec<String>> {
    let lock = if locked { "WOL_SESSION_LOCKED=1 " } else { "" };
    valid_name(&u.name).then(|| {
        vec![
            "/usr/bin/su".into(),
            "-l".into(),
            u.name.clone(),
            "-c".into(),
            format!(
                "exec env XDG_RUNTIME_DIR={} {lock}{SESSION}",
                runtime_dir(u)
            ),
        ]
    })
}

pub const KEYRING_DAEMON: &str = "/usr/bin/gnome-keyring-daemon";

/// gnome-keyring at login: reads the password on stdin, unlocks (or creates) the login keyring once
/// the session runs `gnome-keyring-daemon --start` (hyprland.conf) within 120 s. Not pam_gnome_keyring:
/// OpenPAM fails pam_start when a listed module is missing (no login at all) and runs auth modules
/// after a wrong password (a keyring created with a typo).
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

/// Console (tty1) the notice is drawn on when the login window cannot start.
pub const CONSOLE: &str = "/dev/tty1";

/// Written to ttyv0 before each compositor starts (ADR 0012): vt(4) redraws it whenever no
/// compositor holds the screen, so it must be plain black, with no text and no cursor.
/// Black on black too: later console text (syslog errors) stays invisible. rc.d/wolos_quiet writes
/// the same at the start of boot.
pub const BLANK_CONSOLE: &str = "\x1b[30;40m\x1b[?25l\x1b[H\x1b[2J";
pub const SHOW_CURSOR: &str = "\x1b[?25h";
/// Default colours again (the console notice must be readable).
pub const TEXT_VISIBLE: &str = "\x1b[0m";

/// FreeBSD `VT_LOCKSWITCH` (`_IOW('v', 9, int)`): while set on the active vt, Alt+Fn and
/// Ctrl+Alt+Fn (seatd VT_ACTIVATE) cannot leave the WolOS screen.
pub const VT_LOCKSWITCH: libc::c_ulong = 0x8004_7609;

pub fn vt_lock_arg(lock: bool) -> i32 {
    if lock { 1 } else { 2 }
}

/// `kern.console` lists the active consoles first, `/dev/console` first of all: true when that
/// is a serial port (VM, headless), so boot marks may go to the console without touching the screen.
pub fn serial_primary(kern_console: &str) -> bool {
    kern_console
        .split([',', '/'])
        .next()
        .is_some_and(|c| c.starts_with("ttyu"))
}

pub use shared::login::boot_mark;

/// Compositor hand-offs: a spawn after an exit reports the gap in which vt(4) owned the screen.
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
/// from the greeter exit, and where the recovery login is (getty stays on ttyv1).
pub fn console_notice(dri_present: bool, status: &str) -> String {
    let headline = if dri_present {
        "The WolOS login window could not start."
    } else {
        "WolOS needs a supported graphics card (no /dev/dri: no GPU driver loaded)."
    };
    format!(
        "{TEXT_VISIBLE}{SHOW_CURSOR}\x1b[2J\x1b[H\r\n  WolOS\r\n\r\n  {headline}\r\n\r\n  Reason: login window exited, {status}.\r\n  Details: /var/log/wold.log\r\n\r\n  Recovery console: press Alt+F2.\r\n"
    )
}

/// Greeter respawn policy: a greeter that dies quickly (e.g. Hyprland without DRM) is retried with
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
            assert!(m.contains("Alt+F2"), "recovery console named: {m}");
            assert!(!m.contains("FreeBSD"), "{m}");
            assert!(
                m.starts_with(&format!("{TEXT_VISIBLE}{SHOW_CURSOR}")),
                "notice is readable over black-on-black boot text, with a cursor: {m:?}"
            );
        }
    }

    /// ADR 0012: between two compositors vt(4) redraws ttyv0; it must be black with no cursor, and
    /// any later console text (syslog errors) black on black.
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

    /// FreeBSD sys/consio.h: `#define VT_LOCKSWITCH _IOW('v', 9, int)`; 1 locks, 2 unlocks.
    #[test]
    fn vt_lockswitch_matches_consio() {
        let iow = |g: u8, n: libc::c_ulong, len: libc::c_ulong| {
            0x8000_0000 | (len & 0x1fff) << 16 | libc::c_ulong::from(g) << 8 | n
        };
        assert_eq!(VT_LOCKSWITCH, iow(b'v', 9, 4));
        assert_eq!(VT_LOCKSWITCH, 0x8004_7609);
        assert_eq!((vt_lock_arg(true), vt_lock_arg(false)), (1, 2));
    }

    #[test]
    fn serial_primary_follows_kern_console_order() {
        for (s, want) in [
            ("ttyu0,ttyv0,/ttyv0,ttyu0,", true),
            ("ttyv0,ttyu0,/ttyv0,ttyu0,", false),
            ("ttyv0,/ttyv0,", false),
            ("", false),
        ] {
            assert_eq!(serial_primary(s), want, "{s}");
        }
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
    fn session_runs_through_su_with_private_runtime_dir() {
        let zoe = user("zoe", 1002, "Zoe Lima", "/home/zoe", "/usr/bin/zsh");
        assert_eq!(runtime_dir(&zoe), "/var/run/user/1002");
        let su = |env: &str| {
            [
                "/usr/bin/su".to_string(),
                "-l".into(),
                "zoe".into(),
                "-c".into(),
                format!("exec env XDG_RUNTIME_DIR=/var/run/user/1002 {env}/usr/bin/wol-session"),
            ]
        };
        assert_eq!(session_command(&zoe, false).unwrap(), su(""));
        // Locked start (ADR 0012): wol-session starts Hyprland locked with wol-lock.
        assert_eq!(
            session_command(&zoe, true).unwrap(),
            su("WOL_SESSION_LOCKED=1 ")
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
}
