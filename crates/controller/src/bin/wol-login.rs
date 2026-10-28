//! wol-login (root, wold job `org.wol.login`, IaraOS ADR 0024): greeter -> authenticating -> session -> greeter.
//! Privileged steps: PAM check for the `_wollogin` UI, then the user session via `su -l`. Passwords never logged.
use controller::authcheck::{Auth, wipe};
use controller::login::*;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

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
            let ok = run_session(&user, flow.locked, || marks.spawned(what));
            marks.exited(what);
            if failed_early(started.elapsed(), ok) && flow.locked {
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

/// ms since boot: CLOCK_MONOTONIC counts from boot on FreeBSD.
fn uptime_ms() -> u64 {
    shared::sys::monotonic_ns() / 1_000_000
}

/// Boot timeline (ADR 0012): stdout (wold.log), syslog (/var/log/messages) and, when the console is a
/// serial port, the console too; never the screen.
struct Marks {
    timeline: Timeline,
    serial: bool,
}

impl Marks {
    fn new() -> Self {
        let console = Command::new("/sbin/sysctl")
            .args(["-n", "kern.console"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        Marks {
            timeline: Timeline::default(),
            serial: serial_primary(&console),
        }
    }

    fn emit(&self, line: String) {
        println!("{line}");
        let _ = Command::new("/usr/bin/logger")
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

/// Before each compositor: ttyv0 black without a cursor (vt(4) shows it whenever no compositor holds
/// the screen) and Alt+Fn locked, so nothing but WolOS is ever on screen (ADR 0012).
fn console_ready() {
    match std::fs::OpenOptions::new().write(true).open(CONSOLE) {
        Ok(mut tty) => {
            let _ = tty.write_all(BLANK_CONSOLE.as_bytes());
            vt_lock(&tty, true);
        }
        Err(e) => eprintln!("wol-login: {CONSOLE}: {e}"),
    }
}

/// The console notice: Alt+F2 (recovery getty) must work again.
fn console_notice_shown(dri: bool, status: &str) {
    match std::fs::OpenOptions::new().write(true).open(CONSOLE) {
        Ok(mut tty) => {
            vt_lock(&tty, false);
            if let Err(e) = tty.write_all(console_notice(dri, status).as_bytes()) {
                eprintln!("wol-login: {CONSOLE}: {e}");
            }
        }
        Err(e) => eprintln!("wol-login: {CONSOLE}: {e}"),
    }
}

/// No-op where ttyv0 does not exist (the open fails first).
fn vt_lock(tty: &std::fs::File, lock: bool) {
    use std::os::fd::AsRawFd;
    let arg: libc::c_int = vt_lock_arg(lock);
    // SAFETY: VT_LOCKSWITCH reads one int from the pointer; the fd is an open vt tty.
    if unsafe { libc::ioctl(tty.as_raw_fd(), VT_LOCKSWITCH, &arg) } != 0 {
        eprintln!(
            "wol-login: VT_LOCKSWITCH: {}",
            std::io::Error::last_os_error()
        );
    }
}

fn run_greeter(flow: &mut LoginFlow, respawn: &mut GreeterRespawn, marks: &mut Marks) -> Phase {
    let Some(ui) = find(&read(PASSWD_FILE), UI_USER) else {
        eprintln!("wol-login: no {UI_USER} user");
        std::thread::sleep(Duration::from_secs(5));
        return Phase::Greeter;
    };
    let _ = std::fs::remove_file(SOCKET);
    let listener = match UnixListener::bind(SOCKET) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("wol-login: {SOCKET}: {e}");
            std::thread::sleep(Duration::from_secs(5));
            return Phase::Greeter;
        }
    };
    // anyone may connect; the peer uid decides
    let _ = std::fs::set_permissions(SOCKET, std::fs::Permissions::from_mode(0o666));
    let _ = listener.set_nonblocking(true);
    println!("wol-login: greeter");
    // First boot (no login user yet): the setup assistant instead of the login window (ADR 0004).
    let mut cmd = Command::new(GREETER);
    if flow.listed.is_empty() {
        cmd.arg("setup");
    }
    console_ready();
    let mut greeter = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("wol-login: {GREETER}: {e}");
            std::thread::sleep(Duration::from_secs(5));
            return Phase::Greeter;
        }
    };
    marks.spawned("greeter");
    let started = Instant::now();
    let phase = loop {
        if let Ok(Some(st)) = greeter.try_wait() {
            let next = respawn.exited(started.elapsed());
            if next.log {
                let dri = std::path::Path::new("/dev/dri").exists();
                println!(
                    "wol-login: greeter exited {st}; retrying with backoff up to {}s (/dev/dri present: {dri})",
                    GreeterRespawn::MAX_DELAY.as_secs()
                );
                // Never leave a raw text console: say what is wrong on ttyv0.
                console_notice_shown(dri, &st.to_string());
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
    }
    stop_tree(&mut greeter);
    marks.exited("greeter");
    let _ = std::fs::remove_file(SOCKET);
    phase
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
            if result == Auth::Ok && !unlock_home(&user, &buf[password.clone()]) {
                result = Auth::Error; // no session without its files
            }
            println!("wol-login: {user}: {result:?}");
            let phase = flow.authenticated(result == Auth::Ok);
            let line = match result {
                Auth::Ok => "ok",
                Auth::Denied => "denied",
                Auth::Error => "error",
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
            let answer = if join_wifi(&ssid, password.as_deref()) {
                wait_joined(&ssid, password.is_some())
            } else {
                "error"
            };
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
            reply(client, "ok", false);
            if let Err(e) = Command::new(a[0]).args(&a[1..]).status() {
                eprintln!("wol-login: {}: {e}", a[0]);
            }
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

/// Runs the setup plan; the new account on success.
fn run_setup(a: &NewAccount) -> Option<LoginUser> {
    let run = |argv: &[String], stdin: &str| {
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        wol_core::controls::run_with_input(&argv, stdin, Duration::from_secs(60)).is_some()
    };
    apply_setup(&setup_plan(a), std::path::Path::new("/"), run, || {
        find(&read(PASSWD_FILE), &a.name)
    })
}

/// Asks wpa_supplicant for a fresh scan, waits for it, then lists what it heard.
fn scan_wifi() -> Vec<shared::login::SeenNetwork> {
    let out = |args: &[&str]| {
        Command::new("wpa_cli")
            .args(["-i", WLAN])
            .args(args)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    out(&["scan"]);
    std::thread::sleep(Duration::from_secs(3));
    let cur = wol_core::controls::parse_wpa_status(&out(&["status"]));
    wol_core::controls::parse_scan_results(&out(&["scan_results"]), cur.as_deref())
        .into_iter()
        .map(|n| shared::login::SeenNetwork {
            ssid: n.ssid,
            signal: n.signal,
            secure: n.secure,
            connected: n.connected,
        })
        .collect()
}

/// Polls wpa_supplicant until `ssid` is joined, refused or `JOIN_WAIT` passed (see [`wifi_reply`]).
fn wait_joined(ssid: &str, secured: bool) -> &'static str {
    let out = |args: &[&str]| {
        Command::new("wpa_cli")
            .args(["-i", WLAN])
            .args(args)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    let start = Instant::now();
    loop {
        let state = wol_core::wifi::join_state(&out(&["status"]), &out(&["list_networks"]), ssid);
        let waited_out = start.elapsed() >= wol_core::controls::JOIN_WAIT;
        if let Some(r) = wifi_reply(state, secured, waited_out) {
            return r;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Same allow-listed steps as wol-admind's Wi-Fi join (wpa_supplicant.conf upsert + reconfigure).
fn join_wifi(ssid: &str, password: Option<&str>) -> bool {
    let mut args = vec![WLAN.to_string(), ssid.to_string()];
    args.extend(password.map(String::from));
    let req = shared::admin::AdminRequest {
        action: shared::admin::AdminAction::WifiJoin,
        args,
    };
    let Ok(steps) = wol_core::admin::plan(&req) else {
        return false;
    };
    steps.into_iter().all(|s| match s {
        wol_core::admin::AdminStep::Run(argv) => Command::new(&argv[0])
            .args(&argv[1..])
            .status()
            .is_ok_and(|st| st.success()),
        wol_core::admin::AdminStep::RunInput { argv, input } => {
            let a: Vec<&str> = argv.iter().map(String::as_str).collect();
            wol_core::controls::run_with_input(&a, &input, Duration::from_secs(60)).is_some()
        }
        wol_core::admin::AdminStep::Write {
            path,
            mode,
            content,
        } => {
            let cur = std::fs::read_to_string(&path).ok();
            std::fs::write(&path, content.render(cur.as_deref())).is_ok()
                && std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).is_ok()
        }
    })
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
    let _ = Command::new("/usr/bin/install")
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

/// Login keyring unlocked with the password just checked (no-op without gnome-keyring).
fn start_keyring(user: &LoginUser, password: &[u8]) {
    if !std::path::Path::new(KEYRING_DAEMON).exists() {
        return;
    }
    let (Some(cmd), Ok(pw)) = (keyring_login_command(user), std::str::from_utf8(password)) else {
        return;
    };
    make_runtime_dir(user); // the daemon's control socket lives there
    let argv: Vec<&str> = cmd.iter().map(String::as_str).collect();
    if wol_core::controls::run_with_input(&argv, pw, Duration::from_secs(10)).is_none() {
        eprintln!("wol-login: {}: keyring daemon did not start", user.name);
    }
}

/// Runs the session to its end; true when it exited cleanly (logout).
fn run_session(user: &LoginUser, locked: bool, spawned: impl FnOnce()) -> bool {
    make_runtime_dir(user);
    let Some(cmd) = session_command(user, locked) else {
        return false;
    };
    println!("wol-login: session {} ({})", user.name, user.uid);
    let st = Command::new(&cmd[0])
        .args(&cmd[1..])
        .spawn()
        .and_then(|mut c| {
            spawned();
            c.wait()
        });
    println!("wol-login: session {} ended {st:?}", user.name);
    if std::path::Path::new(KEYRING_DAEMON).exists() {
        let stop = keyring_stop_command(user);
        let _ = Command::new(&stop[0]).args(&stop[1..]).status();
    }
    lock_home(&user.name);
    st.is_ok_and(|s| s.success())
}

/// "Encrypt my files" (wol_core::home_crypt): state of the home's dataset; None for a plain folder.
fn home_state(name: &str) -> Option<wol_core::home_crypt::State> {
    let argv = wol_core::home_crypt::status_argv(name)?;
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    wol_core::home_crypt::parse_status(
        wol_core::system::run_with_timeout(&argv, Duration::from_secs(10)).as_deref(),
    )
}

fn run_crypt(c: &wol_core::home_crypt::Cmd) -> bool {
    let argv: Vec<&str> = c.argv.iter().map(String::as_str).collect();
    let t = Duration::from_secs(60);
    if !c.env.is_empty() {
        // Secrets homectl reads from its environment (never argv).
        return Command::new(&c.argv[0])
            .args(&c.argv[1..])
            .envs(c.env.iter().map(|(k, v)| (k, v)))
            .stdin(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    }
    match &c.stdin {
        Some(pw) => wol_core::controls::run_with_input(&argv, pw, t).is_some(),
        None => wol_core::system::run_with_timeout(&argv, t).is_some(),
    }
}

/// Opens an encrypted home with the password just checked; true when it is ready (or plain).
fn unlock_home(name: &str, password: &[u8]) -> bool {
    let Ok(pw) = std::str::from_utf8(password) else {
        return true;
    };
    let steps = wol_core::home_crypt::unlock(name, home_state(name), pw);
    let ok = steps.iter().all(run_crypt);
    if !ok {
        eprintln!("wol-login: {name}: the encrypted home did not open");
    }
    ok
}

/// Logout: locks the home again; stays open while something still uses a file in it.
fn lock_home(name: &str) {
    let steps = wol_core::home_crypt::lock(name, home_state(name));
    if !steps.iter().all(run_crypt) {
        println!("wol-login: {name}: home still in use, left open");
    }
}

fn stop_tree(child: &mut Child) {
    let ps = Command::new("ps").args(wol_core::jobs::PS_ARGS).output();
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
