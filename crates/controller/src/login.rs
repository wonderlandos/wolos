// SPDX-License-Identifier: GPL-2.0-or-later
//! Pure parts of `wol-login` (IaraOS ADR 0024): passwd parsing, auth request framing, login flow, session plan.

pub use shared::login::SOCKET;
pub const GREETER: &str = "/usr/bin/wol-greeter-session";
pub const UI_USER: &str = "_wollogin";
pub const SESSION: &str = "/usr/bin/wol-session";
pub const CONFIG_FILE: &str = "/etc/wolos/login.json";
pub const PASSWD_FILE: &str = "/etc/passwd";
pub const GROUP_FILE: &str = "/etc/group";
/// The group seatd opens the display for (wol-session: `LIBSEAT_BACKEND=seatd`). Every account
/// WolOS makes is in it (`wol_core::settings::users::ACCOUNT_GROUPS`, the Setup Assistant).
pub const SEAT_GROUP: &str = "seat";
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

/// What wol-login (root) runs for each power request: the same `systemctl` verbs as the session
/// menu ([`wol_core::admin::session_power_argv`]; logind locks and saves state), one list for both.
pub fn power_argv(p: Power) -> &'static [&'static str] {
    wol_core::admin::session_power_argv(p.verb()).unwrap_or(&[])
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

/// The system language file (`man 5 locale.conf`).
pub const LOCALE_CONF: &str = "/etc/locale.conf";
/// The login window's keyboard (layout, variant): the first account's, `hl.config` lines that
/// desktop/overlay/usr/share/wolos/hypr/login.lua loads when present.
pub const LOGIN_KEYBOARD: &str = "/etc/wolos/login-keyboard.lua";

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
        // The language chosen in Setup is the computer's: /etc/locale.conf (systemd and the login
        // shells' profile.d read it; the installer generated every locale). The FreeBSD
        // ~/.login_conf this wrote before was read by nothing (QA-FILES-3, QA-FDAY-2).
        w(LOCALE_CONF.into(), format!("LANG={}\n", a.locale), false),
    ];
    // QA-ACC-10: the login window typed every password on the US layout (a German or ABNT2
    // password with ß, ç or @ failed three times). The first account's layout is the login
    // window's too: login.lua loads this file when it is there.
    if let Some(lua) = wol_core::prefs::keyboard_lua(&a.keymap, 1) {
        v.push(w(LOGIN_KEYBOARD.into(), lua, false));
    }
    v.extend([w(
        format!("{home}/.config/wol/setup.json"),
        serde_json::json!({
            "locale": a.locale,
            "keymap": a.keymap,
            "appearance": a.appearance,
        })
        .to_string(),
        true,
    )]);
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

/// How opening a home with the password just checked ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unlock {
    Opened,
    /// The home is encrypted and the password is not text homectl can be given.
    NeedsText,
    Failed,
}

/// The reply that carries [`Unlock::NeedsText`] to the login window (QA-SHADOW-39): the window
/// shows [`TEXT_PASSWORD_SENTENCE`]; a window that does not know it shows its generic error line.
pub const TEXT_PASSWORD_REPLY: &str = "error-text-password";
pub const TEXT_PASSWORD_SENTENCE: &str = "Your files open only with a password made of typed characters. Sign in with a password you can type, or ask the person who looks after this computer.";

/// The login window's reply for a correct password whose home did not open: the specific line for
/// a non-text password, else the generic `error`.
pub fn unlock_failure_reply(u: Unlock) -> &'static str {
    match u {
        Unlock::NeedsText => TEXT_PASSWORD_REPLY,
        Unlock::Opened | Unlock::Failed => "error",
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
/// makes the logind session, /run/user and user@.service) and starts the session program in it:
/// `wol-session-pam USER PROGRAM [NAME=VALUE]` ([`session_exec`]).
pub const SESSION_PAM: &str = "/usr/lib/wol-session-pam";
pub const SESSION_PAM_SERVICE: &std::ffi::CStr = c"wol-login";

/// The only variable wol-login hands the session program: start locked (ADR 0012).
pub const SESSION_LOCKED_VAR: &str = "WOL_SESSION_LOCKED=1";

/// `locked`: wol-session starts the compositor locked behind wol-lock (ADR 0012). The session
/// program runs directly, never through the account's login shell: a non-POSIX one (pwsh, rsh)
/// could not run the `<shell> -l -c "exec env ..."` line, and the person was back at the login
/// window after every right password (QA-SHADOW-43). wol-session builds the login environment
/// itself (/etc/profile).
pub fn session_command(u: &LoginUser, locked: bool) -> Option<Vec<String>> {
    valid_name(&u.name).then(|| {
        let mut v = vec![SESSION_PAM.into(), u.name.clone(), SESSION.into()];
        if locked {
            v.push(SESSION_LOCKED_VAR.into());
        }
        v
    })
}

/// wol-session-pam's arguments after USER: the program it may start ([`SESSION`] only) and the
/// variables it may set ([`SESSION_LOCKED_VAR`] only). None for anything else.
pub fn session_exec(args: &[String]) -> Option<(String, Vec<(String, String)>)> {
    let (prog, vars) = args.split_first()?;
    if prog != SESSION || vars.len() > 1 || vars.iter().any(|v| v != SESSION_LOCKED_VAR) {
        return None;
    }
    let env = vars
        .iter()
        .filter_map(|v| v.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    Some((prog.clone(), env))
}

/// `env`: su's "shell" for the keyring daemon ([`keyring_login_command`]), so no shell line is
/// ever given to the account's login shell (QA-SHADOW-43).
pub const KEYRING_RUNNER: &str = "/usr/bin/env";

pub const KEYRING_DAEMON: &str = "/usr/bin/gnome-keyring-daemon";

/// gnome-keyring at login: reads the password on stdin, unlocks (or creates) the login keyring once
/// the session runs `gnome-keyring-daemon --start` (hyprland.lua) within 120 s. Not the PAM keyring
/// module: wol-login hands the password over only after it accepted it (a typo never creates or
/// unlocks a keyring), `wol-session-pam` has no password to give a PAM module, and the login PAM
/// stack does not depend on the keyring package (without the daemon this is a no-op).
///
/// The daemon stays in the foreground, and wol-login keeps the `su -l` that runs it until the
/// session ends ([`keyring_socket`] says when it is up). `su -l` has its own logind session: when
/// the daemon forked away and `su` returned, that was the user's last session, logind removed
/// `/run/user/UID` with the daemon's control socket, and the real session started a second, locked
/// daemon whose first secret request made gcr ask to "Choose password for new keyring" (QA live-1
/// H2/H3: Cancel left the browser without a network stack).
///
/// `-s /usr/bin/env` ([`KEYRING_RUNNER`]): su starts the daemon through `env`, not through the
/// account's login shell, which could not run a POSIX line (pwsh; QA-SHADOW-43). The daemon's
/// socket must land in the runtime folder `su`'s PAM session mounted (pam_systemd,
/// user-runtime-dir@): wol-login checks [`runtime_dir_mounted`] once it is up (QA-FILES-11).
pub fn keyring_login_command(u: &LoginUser) -> Option<Vec<String>> {
    valid_name(&u.name).then(|| {
        vec![
            "/usr/bin/su".into(),
            "-l".into(),
            "-s".into(),
            KEYRING_RUNNER.into(),
            // su's own options end here: the daemon's --login and --foreground are not su's.
            "--".into(),
            u.name.clone(),
            format!("XDG_RUNTIME_DIR={}", runtime_dir(u)),
            KEYRING_DAEMON.into(),
            "--foreground".into(),
            "--login".into(),
        ]
    })
}

/// `dir` is a mount point in `mountinfo` (/proc/self/mountinfo: field 5 is the mount point,
/// octal escapes for spaces). /var/run is /run. A socket made in the plain folder wol-login
/// prepares would be hidden by logind's tmpfs when the session starts, and the session would ask
/// to "Choose password for new keyring" (QA-FILES-11).
pub fn runtime_dir_mounted(mountinfo: &str, dir: &str) -> bool {
    let dir = dir
        .strip_prefix("/var")
        .filter(|d| d.starts_with("/run/"))
        .unwrap_or(dir);
    mountinfo
        .lines()
        .filter_map(|l| l.split(' ').nth(4))
        .any(|m| m.replace("\\040", " ") == dir)
}

/// The daemon's control socket: once it exists the login keyring is unlocked and the session's
/// `gnome-keyring-daemon --start` finds the daemon instead of starting a locked one.
pub fn keyring_socket(u: &LoginUser) -> String {
    format!("{}/keyring/control", runtime_dir(u))
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

/// `user` may open the display: a member of [`SEAT_GROUP`] in /etc/group (`group_text`), listed
/// or by its primary group. An account made by hand without it gets a compositor that dies while
/// starting (e2e D6), so wol-login never starts its session.
pub fn can_open_display(group_text: &str, user: &LoginUser) -> bool {
    group_text.lines().any(|l| {
        let f: Vec<&str> = l.split(':').collect();
        f.len() == 4
            && f[0] == SEAT_GROUP
            && (f[2].parse() == Ok(user.gid) || f[3].split(',').any(|m| m.trim() == user.name))
    })
}

/// The login window's one-line answer to a password (shared::login): `no_display` = the password
/// was right but [`can_open_display`] said no, so the window names the real reason.
pub fn auth_reply(result: &crate::authcheck::Auth, no_display: bool) -> &'static str {
    use crate::authcheck::Auth;
    match result {
        Auth::Ok => "ok",
        Auth::Denied => "denied",
        Auth::Error if no_display => shared::login::REPLY_NO_DISPLAY,
        Auth::Error => "error",
    }
}

/// A session that ended sooner than this never started: nobody logs out within two seconds of
/// the compositor's first frame. The compositor's launcher (start-hyprland) reports a compositor
/// that died while starting as a clean exit (e2e D6: an account outside group `seat` restarted
/// every 0.7 s, black screen, 780 crash reports in 40 minutes).
pub const QUICK_SESSION: std::time::Duration = std::time::Duration::from_secs(2);

/// A session that ended before [`GreeterRespawn::HEALTHY`], by whatever exit code (no GPU, lock
/// screen broken, a compositor that quits on a config error with status 0: QA-SYS-10 saw a tight
/// respawn loop of `su -l`). Never a reason to start unlocked: a locked start that ends this soon
/// falls back to the login window, which has the backoff and the console notice.
pub fn failed_early(ran: std::time::Duration) -> bool {
    ran < GreeterRespawn::HEALTHY
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
/// Then it is stopped either way, with a line in the journal ([`handoff_gave_up`]). Two minutes
/// is long enough to read the Done card; a hung Setup used to hold the session back ten minutes
/// (QA-DOC-12).
pub fn handoff_wait(from_setup: bool) -> std::time::Duration {
    std::time::Duration::from_secs(if from_setup { 120 } else { 2 })
}

/// The journal line when the greeter was still up after [`handoff_wait`].
pub fn handoff_gave_up(from_setup: bool) -> String {
    format!(
        "wol-login: the {} was still open {} s after the session was decided; stopping it",
        if from_setup {
            "setup assistant"
        } else {
            "login window"
        },
        handoff_wait(from_setup).as_secs()
    )
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

/// Greeter respawn policy: a greeter that dies quickly (e.g. the compositor without DRM) is retried with
/// exponential backoff (1 s .. 5 min) and logged once per failure streak, never spun; the console
/// notice is drawn from the second exit of a streak ([`console_notice`]).
#[derive(Debug, Default)]
pub struct GreeterRespawn {
    quick_exits: u32,
}

/// What to do after a greeter exit.
#[derive(Debug, PartialEq, Eq)]
pub struct Respawn {
    pub delay: std::time::Duration,
    pub log: bool,
    /// Draw the console notice: from the second quick exit of a streak. One crash of the setup
    /// assistant or login window respawns at once; the text console flashed between (QA-DOC-11).
    pub notice: bool,
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
        Respawn {
            delay,
            log: n == 0,
            notice: n >= 1,
        }
    }

    /// The greeter never started (its socket could not be bound, the program could not be
    /// executed). Counts into the same streak and backoff as an exit, but the notice is drawn at
    /// once: there is no crash to ride out, the VT was already blanked and locked
    /// ([`console_ready`] in wol-login) and nothing else would ever say what is wrong (QA-SHADOW-15,
    /// QA-SHADOW-38).
    pub fn start_failed(&mut self) -> Respawn {
        Respawn {
            notice: true,
            ..self.exited(std::time::Duration::ZERO)
        }
    }
}

mod console;
mod start;
pub use console::{
    BLANK_CONSOLE, CONSOLE, SHOW_CURSOR, TEXT_VISIBLE, VT_LOCKSWITCH, VT_UNLOCKSWITCH, VtStep,
    console_failure_notice, console_notice, console_step, no_vt, retry_words, vt_switch_request,
};
pub use start::{
    NOTICE_FILE, SeatSession, early_end_sentence, ended_on_request, greeter_start_failed,
    parse_df_avail, seat0_user_session,
};

#[cfg(test)]
mod tests {

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
        assert_eq!(handoff_wait(true), Duration::from_secs(120));
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

    /// QA-DOC-11: a single crash of wol-setup respawns without the text-console notice.
    #[test]
    fn notice_from_second_exit() {
        use std::time::Duration as D;
        let mut r = GreeterRespawn::default();
        let quick = D::from_secs(1);
        assert!(!r.exited(quick).notice, "first exit: respawn quietly");
        assert!(r.exited(quick).notice, "second exit: say what is wrong");
        assert!(r.exited(quick).notice);
        assert!(
            !r.exited(D::from_secs(120)).notice,
            "a healthy run starts the streak over"
        );
        assert!(r.exited(quick).notice);
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
        assert_eq!(encode_power(Power::Sleep), b"!sleep\n");
    }

    /// QA-FILES-1: the login window's power row ran FreeBSD commands (`acpiconf`, `shutdown -p`)
    /// that do not exist on Linux while the window said ok. The three verbs are systemd's.
    #[test]
    fn power_argv_is_systemctl() {
        assert_eq!(power_argv(Power::Sleep), ["systemctl", "suspend"]);
        assert_eq!(power_argv(Power::Restart), ["systemctl", "reboot"]);
        assert_eq!(power_argv(Power::ShutDown), ["systemctl", "poweroff"]);
        for p in Power::ALL {
            assert_eq!(
                Some(power_argv(p)),
                wol_core::admin::session_power_argv(p.verb()),
                "{p:?}: the session menu and the login window run the same command"
            );
        }
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
            argv.contains(" -G wheel,seat,video "),
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

    /// QA-FILES-2: the Setup Assistant's account is not in `input` (every keyboard, the lock
    /// screen's password included) nor `audio`; no Run step of the plan names either group.
    #[test]
    fn useradd_argv_no_input() {
        for step in setup_plan(&account()) {
            if let SetupStep::Run { argv, .. } = step {
                for (i, a) in argv.iter().enumerate() {
                    if i > 0 && argv[i - 1] == "-G" {
                        let groups: Vec<&str> = a.split(',').collect();
                        assert!(!groups.contains(&"input"), "{argv:?}");
                        assert!(!groups.contains(&"audio"), "{argv:?}");
                    }
                }
            }
        }
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
                    "wheel,seat,video",
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
        let gb = wol_core::prefs::keyboard_lua("gb", 1).unwrap();
        assert!(gb.contains("kb_layout = \"gb\""), "{gb}");
        // QA-ACC-10: the login window's compositor loads it.
        let login = include_str!("../../../desktop/overlay/usr/share/wolos/hypr/login.lua");
        assert!(login.contains(&format!("local keyboard_lua = \"{LOGIN_KEYBOARD}\"")));
        assert!(login.lines().any(|l| l.trim() == "require(keyboard_lua)"));
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
                ("/etc/locale.conf", "LANG=en_GB.UTF-8\n", false),
                (LOGIN_KEYBOARD, gb.as_str(), false),
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
        // Auto + default accent + no hint/avatar: account, password, keyring dir, locale.conf,
        // the login window's keyboard, setup.json.
        let mut auto = account();
        auto.appearance = "auto".into();
        auto.hint = "".into();
        auto.avatar = "".into();
        auto.accent = "violet".into();
        assert_eq!(setup_plan(&auto).len(), 6);
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
            6,
            "unknown avatar colour not written"
        );
    }

    /// QA-FILES-3 / QA-FDAY-2: the language picked in Setup reaches the computer. The plan writes
    /// `LANG=` into /etc/locale.conf (not the FreeBSD ~/.login_conf, which nothing reads on Linux),
    /// and the value is one the installer generated (`region::locale_gen`).
    #[test]
    fn setup_plan_writes_locale_conf() {
        for (_, label) in wol_core::region::LANGUAGES {
            assert!(!label.is_empty());
        }
        for (locale, _) in wol_core::region::LANGUAGES {
            let mut a = account();
            a.locale = locale.to_string();
            assert_eq!(validate_account(&a), Ok(()), "{locale}");
            let plan = setup_plan(&a);
            let conf = plan.iter().find_map(|s| match s {
                SetupStep::Write {
                    path,
                    content,
                    user_owned: false,
                } if path == "/etc/locale.conf" => Some(content.clone()),
                _ => None,
            });
            assert_eq!(conf, Some(format!("LANG={locale}\n")), "{locale}");
            assert!(
                wol_core::region::locale_gen("").contains(&format!("{locale} UTF-8\n")),
                "{locale} is generated at install"
            );
            assert!(
                !plan.iter().any(|s| matches!(s,
                    SetupStep::Write { path, .. } if path.ends_with(".login_conf"))),
                "no FreeBSD login_conf"
            );
        }
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

    /// QA-SYS-10: a session that exits 0 within seconds (the compositor quitting cleanly on a
    /// config error) used to be respawned at once, forever. Any end under HEALTHY is a broken
    /// locked start: the login window (with the greeter backoff) takes over.
    #[test]
    fn quick_exit_is_lock_broken() {
        use std::time::Duration;
        assert!(failed_early(Duration::ZERO));
        assert!(failed_early(Duration::from_secs(2)));
        assert!(failed_early(
            GreeterRespawn::HEALTHY - Duration::from_millis(1)
        ));
        assert!(!failed_early(GreeterRespawn::HEALTHY));
        let ana = user("ana", 1001, "Ana", "/home/ana", "/bin/sh");
        let mut f = LoginFlow::new(vec![ana], None);
        assert_eq!(
            f.start(),
            Phase::Session,
            "the first start is the locked session"
        );
        if failed_early(Duration::from_secs(1)) {
            f.locked_start_failed();
        }
        f.session_ended();
        assert_eq!(
            f.start(),
            Phase::Greeter,
            "a clean exit after one second does not respawn the session"
        );
    }

    /// QA-DOC-12: the wait for a hung Setup is bounded to a couple of minutes, not ten, and says so.
    /// QA-SHADOW-39: a correct non-text password on an encrypted home gets the specific reply, not
    /// the generic `error` (every other failure stays generic).
    #[test]
    fn non_utf8_password_reply_sentence() {
        use wol_core::home_crypt::State;
        let closed = Some(State {
            encrypted: true,
            key_loaded: false,
            mounted: false,
        });
        let bad = b"\xff\xfe-not-text";
        // home_unlock_steps says "cannot": that is Unlock::NeedsText on the wire.
        assert_eq!(home_unlock_steps("ana", closed, bad), None);
        assert_eq!(unlock_failure_reply(Unlock::NeedsText), TEXT_PASSWORD_REPLY);
        assert_eq!(unlock_failure_reply(Unlock::Failed), "error");
        assert_ne!(TEXT_PASSWORD_REPLY, "error");
        assert!(
            TEXT_PASSWORD_SENTENCE.contains("typed"),
            "{TEXT_PASSWORD_SENTENCE}"
        );
        // A login window that does not know the reply reads it as an error, never as ok.
        assert_eq!(parse_reply_as_old_window(TEXT_PASSWORD_REPLY), "error");
    }

    fn parse_reply_as_old_window(r: &str) -> &'static str {
        match r {
            "ok" => "ok",
            "denied" => "denied",
            _ => "error",
        }
    }

    #[test]
    fn handoff_wait_bounded() {
        use std::time::Duration;
        assert!(handoff_wait(true) <= Duration::from_secs(120));
        assert!(
            handoff_wait(true) > handoff_wait(false),
            "Done is read before Get Started"
        );
        assert_eq!(handoff_wait(false), Duration::from_secs(2));
        let line = handoff_gave_up(true);
        assert!(
            line.contains("setup assistant") && line.contains("120 s"),
            "{line}"
        );
        assert!(handoff_gave_up(false).contains("login window"));
    }

    #[test]
    fn a_right_password_without_the_seat_group_gets_its_own_answer() {
        use crate::authcheck::Auth;
        assert_eq!(auth_reply(&Auth::Ok, false), "ok");
        assert_eq!(auth_reply(&Auth::Denied, false), "denied");
        assert_eq!(auth_reply(&Auth::Error, false), "error");
        assert_eq!(auth_reply(&Auth::Error, true), "no-display");
        assert_eq!(shared::login::REPLY_NO_DISPLAY, "no-display");
    }

    #[test]
    fn only_members_of_the_seat_group_open_the_display() {
        let ana = user("ana", 1001, "Ana", "/home/ana", "/usr/bin/zsh");
        let group = "wheel:x:998:ana\nseat:x:990:bob, ana\nvideo:x:985:\n";
        assert!(can_open_display(group, &ana));
        assert!(!can_open_display("seat:x:990:bob\nseats:x:9:ana\n", &ana));
        assert!(!can_open_display("seat:x:990:anabel\n", &ana));
        // the group as the account's primary group (gid of `user()` is the uid)
        assert!(can_open_display("seat:x:1001:\n", &ana));
        assert!(!can_open_display("", &ana));
    }

    #[test]
    fn a_locked_start_that_dies_early_hands_over_to_the_login_window() {
        use std::time::Duration;
        let quick = Duration::from_secs(3);
        let healthy = GreeterRespawn::HEALTHY;
        // ran -> ended before anyone could use it
        for (ran, early) in [
            (quick, true),
            (healthy, false),
            // e2e D6: the compositor died while starting and its launcher exited 0.
            (Duration::from_millis(700), true),
        ] {
            assert_eq!(failed_early(ran), early, "{ran:?}");
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
        // PipeWire and the portals), then wol-session itself, no shell in between.
        let zoe = user("zoe", 1002, "Zoe Lima", "/home/zoe", "/usr/bin/zsh");
        assert_eq!(runtime_dir(&zoe), "/var/run/user/1002");
        assert_eq!(SESSION_PAM, "/usr/lib/wol-session-pam");
        assert_eq!(
            session_command(&zoe, false).unwrap(),
            [SESSION_PAM, "zoe", "/usr/bin/wol-session"]
        );
        // Locked start (ADR 0012): wol-session starts the compositor locked with wol-lock.
        assert_eq!(
            session_command(&zoe, true).unwrap(),
            [
                SESSION_PAM,
                "zoe",
                "/usr/bin/wol-session",
                "WOL_SESSION_LOCKED=1"
            ]
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
                "-s",
                "/usr/bin/env",
                "--",
                "zoe",
                "XDG_RUNTIME_DIR=/var/run/user/1002",
                "/usr/bin/gnome-keyring-daemon",
                "--foreground",
                "--login"
            ]
        );
        assert!(keyring_login_command(&user("a;b", 1, "", "", "")).is_none());
        assert_eq!(keyring_socket(&zoe), "/var/run/user/1002/keyring/control");
        assert_eq!(
            keyring_stop_command(&zoe),
            ["pkill", "-U", "1002", "-f", KEYRING_DAEMON]
        );
    }

    /// QA-SHADOW-43: the session starts the same way whatever the account's login shell (pwsh
    /// could not run `exec env ...`), and so does the login keyring daemon (`su -s /usr/bin/env`):
    /// no shell line at all, and wol-session-pam starts nothing but wol-session.
    #[test]
    fn session_command_independent_of_shell() {
        let shells = [
            "/usr/bin/zsh",
            "/bin/bash",
            "/usr/bin/pwsh",
            "/usr/bin/fish",
            "/usr/bin/rsh",
        ];
        let argvs: Vec<_> = shells
            .iter()
            .map(|sh| {
                let u = user("alice", 1000, "Alice", "/home/alice", sh);
                let cmd = session_command(&u, true).unwrap();
                (
                    cmd.clone(),
                    session_exec(&cmd[2..]),
                    keyring_login_command(&u).unwrap(),
                )
            })
            .collect();
        assert!(argvs.windows(2).all(|w| w[0] == w[1]), "{argvs:#?}");
        let (_, run, keyring) = &argvs[0];
        assert_eq!(
            run.clone(),
            Some((
                SESSION.to_string(),
                vec![("WOL_SESSION_LOCKED".to_string(), "1".to_string())]
            ))
        );
        assert_eq!(
            keyring[..6],
            ["/usr/bin/su", "-l", "-s", KEYRING_RUNNER, "--", "alice"]
        );
        assert!(!keyring.iter().any(|a| a == "-c"), "{keyring:?}");
        // wol-session-pam starts wol-session and nothing else.
        let s = |v: &[&str]| session_exec(&v.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert!(s(&[SESSION]).is_some_and(|(_, e)| e.is_empty()));
        assert!(s(&["/bin/sh", "-c", "id"]).is_none());
        assert!(s(&[SESSION, "LD_PRELOAD=/tmp/x.so"]).is_none());
        assert!(s(&[]).is_none());
    }

    /// QA-FILES-11: the keyring daemon's socket counts only inside logind's runtime mount.
    #[test]
    fn runtime_dir_mountpoint_before_keyring() {
        let mi = "24 1 0:22 / /run rw - tmpfs tmpfs rw\n\
                  88 24 0:51 / /run/user/1000 rw,nosuid - tmpfs tmpfs rw,size=1M,mode=700,uid=1000\n";
        assert!(runtime_dir_mounted(mi, "/var/run/user/1000"));
        assert!(runtime_dir_mounted(mi, "/run/user/1000"));
        assert!(!runtime_dir_mounted(mi, "/var/run/user/1002"));
        assert!(!runtime_dir_mounted(mi, "/run/user/100"));
    }

    /// QA live-1 H2/H3: the daemon that unlocks the login keyring must not outlive its `su -l`
    /// session as an orphan: it runs in the foreground, so wol-login can hold that session (and
    /// `/run/user/UID`, the daemon's socket) open until the real session ends.
    #[test]
    fn keyring_daemon_stays_in_the_foreground() {
        let zoe = user("zoe", 1002, "Zoe Lima", "/home/zoe", "/bin/sh");
        let cmd = keyring_login_command(&zoe).unwrap();
        assert!(
            cmd.ends_with(&["--foreground".into(), "--login".into()]),
            "{cmd:?}"
        );
        assert!(!cmd.iter().any(|a| a == "--daemonize"), "{cmd:?}");
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

    /// The greeter (login window, setup assistant) and the live image's installer start the
    /// compositor on its Lua configuration (the 0.57 series reads no other), a file that ships.
    #[test]
    fn greeter_and_installer_start_on_the_lua_configuration() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let read = |rel: &str| std::fs::read_to_string(repo.join(rel)).unwrap();
        let ships = |name: &str| {
            let f = format!("desktop/overlay/usr/share/wolos/hypr/{name}.lua");
            assert!(repo.join(&f).is_file(), "{f} ships");
        };
        let g = read(&format!("desktop/overlay{GREETER}"));
        let start = g
            .lines()
            .find(|l| l.contains("Hyprland -c "))
            .expect("the greeter starts the compositor");
        assert!(
            start.ends_with(" Hyprland -c /usr/share/wolos/hypr/$conf.lua"),
            "{start}"
        );
        for conf in ["login", "setup"] {
            assert!(g.contains(&format!("conf={conf}")), "{conf}");
            ships(conf);
        }
        let i = read("files/usr/lib/wolos/installer-session");
        let start = i.lines().find(|l| l.contains("Hyprland -c ")).unwrap();
        assert!(
            start.ends_with(" Hyprland -c /usr/share/wolos/hypr/installer.lua"),
            "{start}"
        );
        ships("installer");
        assert!(
            !g.contains(".conf") && !i.contains(".conf"),
            "no .conf left"
        );
    }
}
