// SPDX-License-Identifier: GPL-2.0-or-later
//! wol-login (root, wold job `org.wol.login`, IaraOS ADR 0024): greeter -> authenticating -> session -> greeter.
//! Privileged steps: PAM check for the `_wollogin` UI, then the user session via `su -l`. Passwords never logged.
//! Every program it starts as root is built by `wol_core::privileged::privileged_command` (cleared
//! environment, fixed PATH, `LC_ALL=C`; R2C-16). Two add one thing back, said at the line: the
//! login window keeps the system `LANG` and has no `LC_ALL` (it speaks the person's language), and
//! homectl gets the secrets it reads from its environment. `crates/controller/tests/privileged.rs`
//! fails on a launch that bypasses it.
use controller::authcheck::{Auth, wipe};
use controller::login::*;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::Child;
use std::time::{Duration, Instant};
use wol_core::privileged::{capture, capture_input, privileged_command};

const PAM_SERVICE: &std::ffi::CStr = c"wol-login";
const FAIL_DELAY: Duration = Duration::from_secs(2);
/// Largest request: setup account (6 short fields + password).
const MAX_REQUEST: usize = 2048;
const WLAN: &str = "wlan0";

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn listed_on_system() -> Vec<LoginUser> {
    listed(&read(PASSWD_FILE), &read(SHELLS_FILE))
}

fn main() {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("wol-login: must run as root");
        std::process::exit(1);
    }
    let auto = parse_config(&read(CONFIG_FILE)).and_then(|n| find(&read(PASSWD_FILE), &n));
    let mut marks = Marks::new();
    marks.emit(boot_mark(uptime_ms(), "wol-login start"));
    let mut flow = LoginFlow::new(listed_on_system(), auto);
    let mut phase = flow.start();
    let mut respawn = GreeterRespawn::default();
    // wold (or this program) was restarted under a session that is still on the seat: wait for
    // it instead of starting a login window the seat daemon refuses (QA L2-39/39b).
    if let Some(running) = running_session() {
        wait_session_end(&running);
        lock_home(&running.user);
        flow.session_ended();
        flow.listed = listed_on_system();
        phase = flow.start();
    }
    loop {
        if phase == Phase::Session
            && let Some(user) = flow.user.clone()
        {
            let what = if flow.locked {
                "session (locked)"
            } else {
                "session"
            };
            console_ready();
            let started = Instant::now();
            // How soon it ended decides (`failed_early`); how, only whether a person asked.
            let ended = run_session(&user, flow.locked, || marks.spawned(what));
            marks.exited(what);
            settle_notice(
                std::path::Path::new(NOTICE_FILE),
                started.elapsed(),
                ended,
                || session_facts(&user),
            );
            if failed_early(started.elapsed()) && flow.locked {
                println!("wol-login: locked start failed; the login window takes over");
                flow.locked_start_failed();
            } else if started.elapsed() >= GreeterRespawn::HEALTHY {
                flow.session_ran_healthy();
            }
            flow.session_ended();
            flow.listed = listed_on_system();
            phase = flow.start();
            continue;
        }
        flow.listed = listed_on_system();
        phase = run_greeter(&mut flow, &mut respawn, &mut marks);
    }
}

/// ms since boot: CLOCK_MONOTONIC counts from boot (time in suspend excluded).
fn uptime_ms() -> u64 {
    shared::sys::monotonic_ns() / 1_000_000
}

/// Boot timeline (ADR 0012): stdout (wold.log), the journal via logger and, when the console is a
/// serial port, the console too; never the screen.
struct Marks {
    timeline: Timeline,
    serial: bool,
}

impl Marks {
    fn new() -> Self {
        Marks {
            timeline: Timeline::default(),
            serial: serial_console_at(
                std::path::Path::new(CONSOLE_ACTIVE),
                std::path::Path::new(KERNEL_CMDLINE),
            ),
        }
    }

    fn emit(&self, line: String) {
        println!("{line}");
        let _ = privileged_command("/usr/bin/logger")
            .args(["-p", "user.notice", "-t", "wol-login", &line])
            .status();
        if self.serial {
            let _ = std::fs::write("/dev/console", format!("{line}\r\n"));
        }
    }

    fn spawned(&mut self, what: &str) {
        let line = self.timeline.spawned(uptime_ms(), what);
        self.emit(line);
    }

    fn exited(&mut self, what: &str) {
        let line = self.timeline.exited(uptime_ms(), what);
        self.emit(line);
    }
}

/// Before each compositor: the console VT black without a cursor (the kernel shows its text
/// whenever no compositor holds the seat) and VT switching locked, so nothing but WolOS is ever on
/// screen (ADR 0012).
fn console_ready() {
    on_console(true, BLANK_CONSOLE);
}

/// The console notice, with VT switching unlocked again (no compositor owns the screen).
fn console_notice_shown(dri: bool, status: &str, retry_in: Duration) {
    on_console(false, &console_notice(dri, status, retry_in));
}

/// [`console_step`] on the real console VT; no VT at all (a container, a nested compositor) is a
/// quiet no-op.
fn on_console(lock: bool, text: &str) {
    let open = || std::fs::OpenOptions::new().write(true).open(CONSOLE);
    if let VtStep::Failed(why) = console_step(open, vt_ioctl, lock, text) {
        eprintln!("wol-login: {CONSOLE}: {why}");
    }
}

/// `VT_LOCKSWITCH` / `VT_UNLOCKSWITCH` on an open descriptor.
fn vt_ioctl(tty: &std::fs::File, request: libc::c_ulong) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    debug_assert!(request == VT_LOCKSWITCH || request == VT_UNLOCKSWITCH);
    // SAFETY: both requests take no argument (the kernel ignores the third one, no pointer is
    // read); the fd stays open for the call.
    if unsafe { libc::ioctl(tty.as_raw_fd(), request, 0) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn run_greeter(flow: &mut LoginFlow, respawn: &mut GreeterRespawn, marks: &mut Marks) -> Phase {
    let Some(ui) = find(&read(PASSWD_FILE), UI_USER) else {
        return start_failed(respawn, &format!("no {UI_USER} account"));
    };
    let _ = std::fs::remove_file(SOCKET);
    let listener = match UnixListener::bind(SOCKET) {
        Ok(l) => l,
        Err(e) => return start_failed(respawn, &format!("{SOCKET}: {e}")),
    };
    // anyone may connect; the peer uid decides
    let _ = std::fs::set_permissions(SOCKET, std::fs::Permissions::from_mode(0o666));
    let _ = listener.set_nonblocking(true);
    println!("wol-login: greeter");
    // First boot (no login user yet): the setup assistant instead of the login window (ADR 0004).
    let mut cmd = privileged_command(GREETER);
    // The login window speaks the system language: LANG passes, LC_ALL=C does not (its wrapper
    // sets PATH, XDG_RUNTIME_DIR and HOME itself).
    cmd.env_remove("LC_ALL");
    if let Ok(lang) = std::env::var("LANG") {
        cmd.env("LANG", lang);
    }
    if flow.listed.is_empty() {
        cmd.arg("setup");
    }
    console_ready();
    let mut greeter = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_file(SOCKET);
            return start_failed(respawn, &format!("{GREETER}: {e}"));
        }
    };
    marks.spawned("greeter");
    let started = Instant::now();
    let phase = loop {
        if let Ok(Some(st)) = greeter.try_wait() {
            let next = respawn.exited(started.elapsed());
            let dri = std::path::Path::new("/dev/dri").exists();
            if next.log {
                println!(
                    "wol-login: greeter exited {st}; retrying with backoff up to {}s (/dev/dri present: {dri})",
                    GreeterRespawn::MAX_DELAY.as_secs()
                );
            }
            // Never leave a raw text console: say what is wrong on the console VT after every
            // exit of a streak but the first (a single crash respawns at once and must not flash
            // the text, QA-DOC-11). The next try blanks the VT (console_ready), so a notice drawn
            // once was gone after one second and a failing greeter left a black screen with no
            // way out shown (vt-lock journey, 2026-11-03).
            if next.notice {
                console_notice_shown(dri, &st.to_string(), next.delay);
            }
            std::thread::sleep(next.delay);
            break Phase::Greeter;
        }
        match listener.accept() {
            Ok((mut client, _)) => {
                if shared::sys::peer(&client).map(|p| p.uid) != Some(ui.uid) {
                    continue; // only the login window
                }
                if answer(&mut client, flow) == Phase::Session {
                    break Phase::Session;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    if phase == Phase::Session {
        // Seamless hand-off (ADR 0012): the window fades to black and exits by itself.
        let deadline = Instant::now() + handoff_wait(flow.from_setup());
        while Instant::now() < deadline && matches!(greeter.try_wait(), Ok(None)) {
            std::thread::sleep(Duration::from_millis(50));
        }
        if matches!(greeter.try_wait(), Ok(None)) {
            println!("{}", handoff_gave_up(flow.from_setup()));
        }
    }
    finish_greeter(&mut greeter, stop_tree);
    marks.exited("greeter");
    let _ = std::fs::remove_file(SOCKET);
    phase
}

/// The greeter cannot be started at all (no `_wollogin`, socket bind or program spawn failed):
/// the console says what is wrong, with VT switching unlocked, and the next try waits the
/// back-off (QA-SHADOW-15/38: it used to sleep 5 s on a blanked, locked VT, forever, with
/// nothing on screen).
fn start_failed(respawn: &mut GreeterRespawn, why: &str) -> Phase {
    eprintln!("wol-login: the login window cannot start: {why}");
    let dri = std::path::Path::new("/dev/dri").exists();
    let open = || std::fs::OpenOptions::new().write(true).open(CONSOLE);
    let (next, step) = greeter_start_failed(respawn, why, dri, open, vt_ioctl);
    if let VtStep::Failed(why) = step {
        eprintln!("wol-login: {CONSOLE}: {why}");
    }
    std::thread::sleep(next.delay);
    Phase::Greeter
}

/// After a greeter's loop ends: stop what is left of its tree, unless the child was already
/// reaped (`try_wait` returned its status): its pid is free by now, and after the respawn delay
/// signalling it as root could hit another process (QA-SHADOW-37). `stop` is [`stop_tree`].
fn finish_greeter(greeter: &mut Child, stop: impl FnOnce(&mut Child)) {
    if matches!(greeter.try_wait(), Ok(Some(_))) {
        return;
    }
    stop(greeter);
}

/// A running Wayland session of a person on seat0, if any (`loginctl`; asked while logind
/// answers). None when there is none or logind did not answer within the time given.
fn running_session() -> Option<SeatSession> {
    let ui = find(&read(PASSWD_FILE), UI_USER).map_or(u32::MAX, |u| u.uid);
    let t = Duration::from_secs(5);
    let list = capture(
        &["loginctl", "list-sessions", "--no-legend", "--no-pager"],
        t,
    )?;
    seat0_user_session(&list, ui, |id| {
        capture(
            &[
                "loginctl",
                "show-session",
                id,
                "-p",
                "Class",
                "-p",
                "Type",
                "-p",
                "Leader",
                "-p",
                "State",
            ],
            t,
        )
        .unwrap_or_default()
    })
}

/// Blocks until the session's leader is gone: nobody here is its parent, so it is looked at, not
/// waited for.
fn wait_session_end(s: &SeatSession) {
    println!(
        "wol-login: {} already has session {} on seat0 (leader {}); not starting a login window",
        s.user, s.id, s.leader
    );
    // /proc/<pid> exists until the leader is reaped (by init: it is nobody's child here).
    let leader = std::path::PathBuf::from(format!("/proc/{}", s.leader));
    while leader.exists() {
        std::thread::sleep(Duration::from_millis(500));
    }
    println!("wol-login: session {} of {} ended", s.id, s.user);
}

/// What the end of a short session depends on: free bytes on the home's disk, and whether the
/// account can write its `~/.config`.
fn session_facts(user: &LoginUser) -> (Option<u64>, Option<bool>) {
    let t = Duration::from_secs(5);
    let free = capture(&["df", "-B1", "--output=avail", &user.home], t)
        .as_deref()
        .and_then(parse_df_avail);
    let config = format!("{}/.config", user.home);
    let writable = privileged_command("runuser")
        .args(["-u", &user.name, "--", "test", "-w", &config])
        .status()
        .ok()
        .map(|s| s.success());
    (free, writable)
}

/// After a session: when it ended within seconds and nobody asked it to ([`ended_on_request`]), the
/// sentence for the login window goes to `path` (atomically, readable by the window's account);
/// any other end removes a notice left by an earlier one. `facts` is asked only for the first
/// case. Returns the sentence written.
fn settle_notice(
    path: &std::path::Path,
    ran: Duration,
    ended: Option<(Option<i32>, Option<i32>)>,
    facts: impl FnOnce() -> (Option<u64>, Option<bool>),
) -> Option<&'static str> {
    let (code, signal) = ended.unwrap_or((None, None));
    if !failed_early(ran) || ended_on_request(code, signal) {
        let _ = std::fs::remove_file(path);
        return None;
    }
    let (free, writable) = facts();
    let sentence = early_end_sentence(free, writable);
    println!("wol-login: the session ended early: {sentence}");
    if let Err(e) = shared::atomic::atomic_write(path, 0o644, sentence.as_bytes()) {
        eprintln!("wol-login: {}: {e}", path.display());
    }
    Some(sentence)
}

fn answer(client: &mut UnixStream, flow: &mut LoginFlow) -> Phase {
    let mut buf = [0u8; MAX_REQUEST];
    let _ = client.set_nonblocking(false);
    let _ = client.set_read_timeout(Some(Duration::from_secs(5)));
    let mut n = 0;
    while n < buf.len() {
        match client.read(&mut buf[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    let req = parse(&buf[..n]).filter(|r| allowed(r, flow.phase, !flow.listed.is_empty()));
    let phase = match req {
        Some(Request::Auth { user, password }) if flow.submit(&user) => {
            let mut result = match std::ffi::CString::new(user.clone()) {
                Ok(u) => controller::pam::authenticate(PAM_SERVICE, &u, &buf[password.clone()]),
                Err(_) => Auth::Error,
            };
            let mut unlock = Unlock::Opened;
            if result == Auth::Ok {
                unlock = unlock_home(&user, &buf[password.clone()]);
                if unlock != Unlock::Opened {
                    result = Auth::Error; // no session without its files
                }
            }
            println!("wol-login: {user}: {result:?}");
            let phase = flow.authenticated(result == Auth::Ok);
            let line = match result {
                Auth::Ok => "ok",
                Auth::Denied => "denied",
                Auth::Error => unlock_failure_reply(unlock),
            };
            reply(client, line, result != Auth::Ok);
            if let (Phase::Session, Some(u)) = (phase, flow.user.as_ref()) {
                start_keyring(u, &buf[password]);
            }
            phase
        }
        Some(Request::SetupCreateUser(a)) => {
            let name = a.name.clone();
            let r = validate_account(&a).map_err(|why| {
                eprintln!("wol-login: setup: invalid {why}");
                "denied"
            });
            let created = r.and_then(|()| run_setup(&a).ok_or("error"));
            if let Ok(u) = &created {
                start_keyring(u, a.password.as_bytes());
            }
            drop(a);
            match created {
                Ok(user) => {
                    println!("wol-login: setup created {name}");
                    reply(client, "ok", false);
                    flow.setup_done(user)
                }
                Err(line) => {
                    reply(client, line, true);
                    flow.phase
                }
            }
        }
        Some(Request::SetupWifi { ssid, password }) => {
            let answer = controller::login::nm_join_reply(
                join_wifi(&ssid, password.as_deref()),
                password.is_some(),
            );
            println!("wol-login: setup wifi {ssid}: {answer}");
            reply(client, answer, answer != "ok");
            flow.phase
        }
        Some(Request::SetupWifiScan) => {
            let nets = scan_wifi();
            println!("wol-login: setup wifi scan: {} networks", nets.len());
            reply(client, &shared::login::encode_networks(&nets), false);
            flow.phase
        }
        Some(Request::Power(p)) => {
            let a = power_argv(p);
            println!("wol-login: {}", p.verb());
            // The window hears "ok" only when the request was taken: a command that did not start
            // or exited non-zero within POWER_WAIT is "error" (QA-FILES-1: it said ok for a
            // missing acpiconf). `systemctl suspend` blocks until the computer wakes: still running
            // after POWER_WAIT means taken.
            let taken = run_power(a);
            reply(client, if taken { "ok" } else { "error" }, false);
            flow.phase
        }
        _ => {
            reply(client, "denied", true);
            flow.phase
        }
    };
    wipe(&mut buf);
    phase
}

/// How long a power command may run before it counts as taken (`suspend` blocks until wake-up).
const POWER_WAIT: Duration = Duration::from_secs(3);

/// Starts `argv` as root; true when it exited 0 or is still running after [`POWER_WAIT`].
fn run_power(argv: &[&str]) -> bool {
    let Some((prog, args)) = argv.split_first() else {
        return false;
    };
    let mut child = match privileged_command(prog).args(args).spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("wol-login: {prog}: {e}");
            return false;
        }
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                if !st.success() {
                    eprintln!("wol-login: {prog} {}: {st}", args.join(" "));
                }
                return st.success();
            }
            Ok(None) if start.elapsed() >= POWER_WAIT => {
                // Named; a thread the system refuses is not a failed power request (the child
                // is then reaped when wol-login ends).
                let _ = std::thread::Builder::new()
                    .name("power-wait".into())
                    .spawn(move || {
                        let _ = child.wait();
                    });
                return true;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return false,
        }
    }
}

/// Runs the setup plan; the new account on success.
fn run_setup(a: &NewAccount) -> Option<LoginUser> {
    let run = |argv: &[String], stdin: &str| {
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        capture_input(&argv, stdin, Duration::from_secs(60)).is_some()
    };
    apply_setup(&setup_plan(a), std::path::Path::new("/"), run, || {
        find(&read(PASSWD_FILE), &a.name)
    })
}

/// Asks NetworkManager for a fresh scan and lists what it heard.
fn scan_wifi() -> Vec<shared::login::SeenNetwork> {
    let out = privileged_command("nmcli")
        .args([
            "-t",
            "-f",
            "IN-USE,SIGNAL,SECURITY,SSID",
            "dev",
            "wifi",
            "list",
            "--rescan",
            "yes",
        ])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    wol_core::controls::parse_nmcli_wifi(&out)
        .into_iter()
        .map(|n| shared::login::SeenNetwork {
            ssid: n.ssid,
            signal: n.signal,
            secure: n.secure,
            connected: n.connected,
        })
        .collect()
}

/// An `AdminStep::Write`: the whole new file appears at once, with its mode from the first byte
/// (QA-DATA-14: truncate-then-write left a cut file after a power cut, and a secret was readable
/// for an instant before the mode was set).
fn write_step(path: &std::path::Path, mode: u32, content: &wol_core::admin::WriteContent) -> bool {
    let cur = std::fs::read_to_string(path).ok();
    shared::atomic::atomic_write(path, mode, content.render(cur.as_deref()).as_bytes()).is_ok()
}

/// Same allow-listed steps as wol-admind's Wi-Fi join (NetworkManager, password on stdin).
fn join_wifi(ssid: &str, password: Option<&str>) -> bool {
    let mut args = vec![WLAN.to_string(), ssid.to_string()];
    args.extend(password.map(String::from));
    let req = shared::admin::AdminRequest {
        action: shared::admin::AdminAction::WifiJoin,
        args,
    };
    // The same facts wol-admind plans with: the saved profiles (a retry after a wrong password
    // changes the profile in place, never makes a second one with the same name) and the scan
    // (WPA3-only networks need `sae`).
    let probe = |argv: &[&str]| capture(argv, Duration::from_secs(5));
    let Ok(steps) = wol_core::admin::plan_with(&req, &probe) else {
        return false;
    };
    // A wrong password leaves no profile behind: asked before the plan runs.
    let cleanup = wol_core::admin::wifi_join_cleanup(&req, &probe);
    let ok = steps.into_iter().all(|s| match s {
        wol_core::admin::AdminStep::Run(argv) => privileged_command(&argv[0])
            .args(&argv[1..])
            .status()
            .is_ok_and(|st| st.success()),
        wol_core::admin::AdminStep::RunInput { argv, input } => {
            let a: Vec<&str> = argv.iter().map(String::as_str).collect();
            capture_input(&a, &input, Duration::from_secs(60)).is_some()
        }
        wol_core::admin::AdminStep::Write {
            path,
            mode,
            content,
        } => write_step(std::path::Path::new(&path), mode, &content),
    });
    if let (false, Some(del)) = (ok, cleanup) {
        let _ = privileged_command(&del[0]).args(&del[1..]).status();
    }
    ok
}

/// Fixed delay on every non-ok answer (ADR 0023, 0024).
fn reply(client: &mut UnixStream, answer: &str, delay: bool) {
    if delay {
        std::thread::sleep(FAIL_DELAY);
    }
    let _ = client.write_all(format!("{answer}\n").as_bytes());
}

fn make_runtime_dir(user: &LoginUser) {
    let _ = std::fs::create_dir_all("/var/run/user");
    let _ = privileged_command("/usr/bin/install")
        .args([
            "-d",
            "-o",
            &user.uid.to_string(),
            "-g",
            &user.gid.to_string(),
            "-m",
            "0700",
            &runtime_dir(user),
        ])
        .status();
}

/// `su -l` sessions holding a login keyring daemon open: one per signed-in account, reaped when
/// its session ends ([`stop_keyring`]).
static KEYRINGS: std::sync::Mutex<Vec<Child>> = std::sync::Mutex::new(Vec::new());

/// Login keyring unlocked with the password just checked (no-op without gnome-keyring). The daemon
/// runs in the foreground under `su -l`, which this keeps alive until the session has ended: its
/// logind session keeps `/run/user/UID` (and the daemon's socket) there for the real session to
/// find (QA live-1 H2/H3, FILES-11).
fn start_keyring(user: &LoginUser, password: &[u8]) {
    if !std::path::Path::new(KEYRING_DAEMON).exists() {
        return;
    }
    let (Some(cmd), Ok(pw)) = (keyring_login_command(user), std::str::from_utf8(password)) else {
        return;
    };
    make_runtime_dir(user); // the daemon's control socket lives there
    let spawned = privileged_command(&cmd[0])
        .args(&cmd[1..])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else {
        eprintln!("wol-login: {}: keyring daemon did not start", user.name);
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        // Closed after the write: the daemon reads the password up to the end of its input.
        let _ = stdin.write_all(pw.as_bytes());
    }
    let socket = keyring_socket(user);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline
        && !std::path::Path::new(&socket).exists()
        && matches!(child.try_wait(), Ok(None))
    {
        std::thread::sleep(Duration::from_millis(50));
    }
    if !std::path::Path::new(&socket).exists() {
        eprintln!("wol-login: {}: keyring daemon did not start", user.name);
    }
    if let Ok(mut all) = KEYRINGS.lock() {
        all.push(child);
    }
}

/// After the session: stops the daemon and the `su -l` that held its logind session open.
fn stop_keyring(user: &LoginUser) {
    let stop = keyring_stop_command(user);
    let _ = privileged_command(&stop[0]).args(&stop[1..]).status();
    if let Ok(mut all) = KEYRINGS.lock() {
        for mut c in all.drain(..) {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Runs the session to its end; its (exit code, signal), None when it did not start.
fn run_session(
    user: &LoginUser,
    locked: bool,
    spawned: impl FnOnce(),
) -> Option<(Option<i32>, Option<i32>)> {
    make_runtime_dir(user);
    let cmd = session_command(user, locked)?;
    println!("wol-login: session {} ({})", user.name, user.uid);
    // `su -l` builds the login environment of the account itself.
    let st = privileged_command(&cmd[0])
        .args(&cmd[1..])
        .spawn()
        .and_then(|mut c| {
            spawned();
            c.wait()
        });
    println!("wol-login: session {} ended {st:?}", user.name);
    if std::path::Path::new(KEYRING_DAEMON).exists() {
        stop_keyring(user);
    }
    lock_home(&user.name);
    st.ok().map(|s| {
        use std::os::unix::process::ExitStatusExt;
        (s.code(), s.signal())
    })
}

/// "Encrypt my files" (wol_core::home_crypt): state of the home's dataset; None for a plain folder.
fn home_state(name: &str) -> Option<wol_core::home_crypt::State> {
    let argv = wol_core::home_crypt::status_argv(name)?;
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    wol_core::home_crypt::parse_status(capture(&argv, Duration::from_secs(10)).as_deref())
}

fn run_crypt(c: &wol_core::home_crypt::Cmd) -> bool {
    let argv: Vec<&str> = c.argv.iter().map(String::as_str).collect();
    let t = Duration::from_secs(60);
    if !c.env.is_empty() {
        // Secrets homectl reads from its environment (never argv): the only variables added to
        // the cleared environment.
        return privileged_command(&c.argv[0])
            .args(&c.argv[1..])
            .envs(c.env.iter().map(|(k, v)| (k, v)))
            .status()
            .is_ok_and(|s| s.success());
    }
    match &c.stdin {
        Some(pw) => capture_input(&argv, pw, t).is_some(),
        None => capture(&argv, t).is_some(),
    }
}

/// Opens an encrypted home with the password just checked ([`Unlock::Opened`] when it is ready or
/// plain).
fn unlock_home(name: &str, password: &[u8]) -> Unlock {
    let Some(steps) = home_unlock_steps(name, home_state(name), password) else {
        eprintln!("wol-login: {name}: the encrypted home needs a password typed as text");
        return Unlock::NeedsText;
    };
    if steps.iter().all(run_crypt) {
        Unlock::Opened
    } else {
        eprintln!("wol-login: {name}: the encrypted home did not open");
        Unlock::Failed
    }
}

/// Logout: locks the home again; stays open while something still uses a file in it.
fn lock_home(name: &str) {
    let steps = wol_core::home_crypt::lock(name, home_state(name));
    if !steps.iter().all(run_crypt) {
        println!("wol-login: {name}: home still in use, left open");
    }
}

fn stop_tree(child: &mut Child) {
    let ps = privileged_command("ps")
        .args(wol_core::jobs::PS_ARGS)
        .output();
    let pairs = ps
        .map(|o| wol_core::jobs::parse_ps_pairs(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    let mut tree = wol_core::jobs::process_tree(child.id() as i32, &pairs);
    if tree.is_empty() {
        tree.push(child.id() as i32);
    }
    let alive = |p: i32| unsafe { libc::kill(p, 0) } == 0; // SAFETY: signal 0 only checks existence
    for &p in &tree {
        // SAFETY: plain kill(2).
        unsafe { libc::kill(p, libc::SIGTERM) };
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && tree.iter().any(|p| alive(*p)) {
        let _ = child.try_wait();
        std::thread::sleep(Duration::from_millis(50));
    }
    for &p in &tree {
        // SAFETY: plain kill(2).
        unsafe { libc::kill(p, libc::SIGKILL) };
    }
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// QA-DATA-14: a Setup write replaces the file whole, with the asked mode from the first
    /// byte (the temporary is created with it) and no temporary left behind.
    #[test]
    fn admin_step_write_atomic() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("wol-login-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("secret.nmconnection");
        std::fs::write(&file, "old and much longer than the new content\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let fixed = wol_core::admin::WriteContent::Fixed("psk=new\n".into());
        assert!(write_step(&file, 0o600, &fixed));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "psk=new\n");
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(left.len(), 1, "no temporary file stays: {left:?}");
        // A folder that is not there: nothing is created and the step fails.
        assert!(!write_step(&dir.join("missing").join("x"), 0o600, &fixed));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// QA-LIVE-33 / QA-DATA-3 / QA-DATA-14-style: the sentence really reaches the file the login
    /// window reads, a crash within seconds writes it, and a logout, a long session or a later
    /// healthy end takes a stale one away.
    #[test]
    fn early_end_notice_file_is_written_and_cleared() {
        let dir = std::env::temp_dir().join(format!("wol-login-notice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("wol-login-notice");
        let soon = Duration::from_secs(1);
        let facts_full = || (Some(0), Some(true));
        let never = || panic!("facts are asked only for a crash within seconds");

        // A crash within a second on a full disk.
        let said = settle_notice(&file, soon, Some((Some(1), None)), facts_full).unwrap();
        assert!(said.contains("disk is full"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), said);
        // A session that never started counts the same way.
        assert!(settle_notice(&file, soon, None, facts_full).is_some());
        // The person logged out within seconds: status 0, or logind's TERM. The old notice goes.
        assert_eq!(
            settle_notice(&file, soon, Some((Some(0), None)), never),
            None
        );
        assert!(!file.exists());
        settle_notice(&file, soon, Some((Some(1), None)), facts_full);
        assert_eq!(
            settle_notice(&file, soon, Some((None, Some(libc::SIGTERM))), never),
            None
        );
        assert!(!file.exists());
        // A crash after a healthy run is not a failed start.
        settle_notice(&file, soon, Some((Some(1), None)), facts_full);
        let long = Duration::from_secs(60);
        assert_eq!(
            settle_notice(&file, long, Some((Some(1), None)), never),
            None
        );
        assert!(!file.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// QA-SHADOW-37: a greeter whose exit `try_wait` already returned is not signalled again
    /// (its pid may belong to another process by then); one still running is stopped.
    #[test]
    fn no_stop_tree_after_try_wait_some() {
        let mut done = std::process::Command::new("true").spawn().unwrap();
        done.wait().unwrap(); // reaped: `try_wait` answers with the same status from now on
        let mut stopped = Vec::new();
        finish_greeter(&mut done, |c| stopped.push(c.id()));
        assert!(stopped.is_empty(), "already reaped: nothing to signal");

        let mut alive = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        finish_greeter(&mut alive, |c| stopped.push(c.id()));
        assert_eq!(stopped, [alive.id()]);
        alive.kill().unwrap();
        alive.wait().unwrap();
    }

    /// The real ioctl layer on a file that is no VT (a nested compositor's case): ENOTTY, so the
    /// step is a no-op and nothing is written; a missing device path is a no-op too.
    #[test]
    fn real_ioctl_on_a_non_vt_is_a_no_op() {
        let dir = std::env::temp_dir().join(format!("wol-login-vt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("tty1");
        std::fs::write(&fake, "").unwrap();
        let open = || std::fs::OpenOptions::new().write(true).open(&fake);
        assert_eq!(
            console_step(open, vt_ioctl, true, BLANK_CONSOLE),
            VtStep::NoVt
        );
        assert_eq!(
            std::fs::read(&fake).unwrap(),
            b"",
            "nothing written to a non-VT"
        );
        let missing = || {
            std::fs::OpenOptions::new()
                .write(true)
                .open(dir.join("none"))
        };
        assert_eq!(console_step(missing, vt_ioctl, false, "x"), VtStep::NoVt);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
