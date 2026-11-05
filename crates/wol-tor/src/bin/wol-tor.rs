// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol-tor`: starts and runs Atlas Tor groups (ADR 0053). Verbs:
//! `open [--group G] [URL]` (from the session: start the group or open a tab in it),
//! `newcircuit [--group G] HOST`, `tor-run --group G` (the tor unit's main process),
//! `host --group G -- ATLAS` (the browser unit's main process: netns check, relay, Atlas),
//! `cache-run` (the cache unit), `status`, `wait-ready [SECS]`, `relay [ADDR]` (tests and the
//! benchmark), and the dry runs `torrc`, `policies`, `units`, `check-netns`.
#![forbid(unsafe_code)]
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use std::{env, fs, thread};
use wol_tor::control::{Control, read_cookie};
use wol_tor::doctor::{self, Observations};
use wol_tor::group::{Group, Paths};
use wol_tor::host::{self, Command as HostCommand};
use wol_tor::isolation::Isolator;
use wol_tor::mode::{self, Switches};
use wol_tor::relay::{Ready, Relay};
use wol_tor::socks::{Host, Target, client_connect};
use wol_tor::torrc::{Bridge, TorConfig, Transports, tor_argv};
use wol_tor::transparent::{self, AppNs};
use wol_tor::{cache, netns, profile, ready, units};

/// Session variables Atlas needs inside its unit.
const SESSION_ENV: [&str; 5] = [
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_SESSION_TYPE",
    "XDG_CURRENT_DESKTOP",
];
/// How long the host waits for tor's bootstrap before a connect fails.
const READY_WAIT: Duration = Duration::from_secs(30);
/// How long a verb waits to connect to tor's control socket when tor was just started.
const CONTROL_CONNECT_WAIT: Duration = Duration::from_secs(10);
/// How long the host waits for the rest of a command line from the command socket.
const COMMAND_READ_WAIT: Duration = Duration::from_secs(5);
/// Read and write deadline of a one-line exchange on a Unix socket.
const SOCKET_IO_WAIT: Duration = Duration::from_secs(10);
/// A control connection that must answer at once (tor is already running).
const QUICK_CONNECT_WAIT: Duration = Duration::from_secs(5);
/// How long the cache unit lets tor bootstrap before it gives up.
const BOOTSTRAP_DEADLINE: Duration = Duration::from_secs(150);
/// How long the cache unit waits for tor to exit after the request to stop.
const STOP_DEADLINE: Duration = Duration::from_secs(10);
/// Health check interval of the host once tor is bootstrapped.
const HEALTH_POLL: Duration = Duration::from_secs(2);
/// Poll interval of the host while tor is still bootstrapping.
const BOOTSTRAP_WATCH_POLL: Duration = Duration::from_millis(100);
/// How long the host waits before it retries a lost control session.
const HOST_IDLE_POLL: Duration = Duration::from_millis(200);
/// Parking time of the relay verb's idle loop.
const RELAY_IDLE_POLL: Duration = Duration::from_millis(50);
/// Poll interval while the cache unit watches the bootstrap.
const BOOTSTRAP_POLL: Duration = Duration::from_millis(500);
/// Poll interval while the cache unit waits for tor to exit.
const STOP_POLL: Duration = Duration::from_millis(100);
/// Most bridge lines read from the config file.
const MAX_BRIDGES_FILE: u64 = 64 << 10;

type Res<T> = Result<T, String>;

fn runtime_dir() -> Res<PathBuf> {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "XDG_RUNTIME_DIR is not set: run wol-tor inside a session".into())
}

fn home_sub(var: &str, fallback: &str) -> Res<PathBuf> {
    if let Some(p) = env::var_os(var) {
        return Ok(PathBuf::from(p));
    }
    env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(fallback))
        .ok_or_else(|| format!("neither {var} nor HOME is set"))
}

fn cache_dir() -> Res<PathBuf> {
    Ok(home_sub("XDG_STATE_HOME", ".local/state")?.join("wol-tor/cache"))
}

/// The "Keep Tor ready" switch (default off; a missing file is off).
fn keep_ready_on() -> Res<bool> {
    let file = home_sub("XDG_CONFIG_HOME", ".config")?.join(ready::FILE);
    Ok(fs::read_to_string(file).is_ok_and(|t| ready::parse(&t)))
}

/// A line for the journal (stderr): a stream thread the system refused.
fn journal(line: &str) {
    eprintln!("<3>wol-tor: {line}");
}

fn systemctl_user(args: &[&str]) -> Res<()> {
    let s = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()
        .map_err(|e| format!("systemctl could not start: {e}"))?;
    if s.success() {
        Ok(())
    } else {
        Err(format!(
            "systemctl --user {} ended with {s}",
            args.join(" ")
        ))
    }
}

/// `keep-ready on|off`: records the switch, then installs and enables the cache timer, or
/// disables and removes it. Off is the default and writes nothing.
fn set_keep_ready(on: bool) -> Res<()> {
    let cfg = home_sub("XDG_CONFIG_HOME", ".config")?;
    let file = cfg.join(ready::FILE);
    let units_dir = cfg.join("systemd/user");
    let me = env::current_exe().map_err(|e| e.to_string())?;
    if let Some(d) = file.parent() {
        fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    fs::write(&file, ready::render(on)).map_err(|e| format!("{}: {e}", file.display()))?;
    if on {
        fs::create_dir_all(&units_dir).map_err(|e| format!("{}: {e}", units_dir.display()))?;
        for (name, text) in ready::unit_files(true, &me) {
            let path = units_dir.join(name);
            fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        systemctl_user(&["daemon-reload"])?;
        systemctl_user(&["enable", "--now", ready::TIMER_NAME])
    } else {
        let _ = systemctl_user(&["disable", "--now", ready::TIMER_NAME]);
        for name in [ready::TIMER_NAME, ready::SERVICE_NAME] {
            let _ = fs::remove_file(units_dir.join(name));
        }
        let _ = systemctl_user(&["daemon-reload"]);
        Ok(())
    }
}

fn user_bridges_file() -> Res<PathBuf> {
    Ok(home_sub("XDG_CONFIG_HOME", ".config")?.join("wol-tor/bridges"))
}

fn bridges(file: &Path) -> Res<Vec<Bridge>> {
    let mut text = String::new();
    match fs::File::open(file) {
        Ok(f) => {
            f.take(MAX_BRIDGES_FILE)
                .read_to_string(&mut text)
                .map_err(|e| format!("{}: {e}", file.display()))?;
        }
        Err(_) => return Ok(vec![]),
    }
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| Bridge::parse(l).map_err(|e| e.to_string()))
        .collect()
}

fn which(name: &str, var: &str) -> Option<PathBuf> {
    if let Some(p) = env::var_os(var) {
        return Some(PathBuf::from(p));
    }
    ["/usr/bin", "/usr/lib/tor"]
        .iter()
        .map(|d| Path::new(d).join(name))
        .find(|p| p.is_file())
}

fn config(paths: &Paths, gateway: Option<(bool, bool)>) -> Res<TorConfig> {
    Ok(TorConfig {
        data_dir: paths.data.clone(),
        socks: paths.socks.clone(),
        control: paths.control.clone(),
        cookie: paths.cookie.clone(),
        bridges: bridges(&if gateway.is_some() {
            etc_dir().join("bridges")
        } else {
            user_bridges_file()?
        })?,
        transports: Transports {
            lyrebird: which("lyrebird", "WOL_TOR_LYREBIRD"),
            snowflake: which("snowflake-client", "WOL_TOR_SNOWFLAKE"),
        },
        gateway,
    })
}

fn group_arg(args: &mut Vec<String>) -> Res<Group> {
    let name = match args.iter().position(|a| a == "--group") {
        Some(i) if i + 1 < args.len() => {
            let v = args.remove(i + 1);
            args.remove(i);
            v
        }
        Some(_) => return Err("--group needs a name".into()),
        None => "default".into(),
    };
    Group::new(&name).map_err(|e| e.to_string())
}

fn prepare(paths: &Paths, gateway: Option<(bool, bool)>) -> Res<PathBuf> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&paths.data)
        .map_err(|e| format!("{}: {e}", paths.data.display()))?;
    let torrc = paths.dir.join("torrc");
    let text = config(paths, gateway)?
        .render()
        .map_err(|e| e.to_string())?;
    fs::write(&torrc, text).map_err(|e| format!("{}: {e}", torrc.display()))?;
    fs::set_permissions(&torrc, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    Ok(torrc)
}

/// The directory of the machine-wide settings (root-owned); `WOL_TOR_ETC` is for tests.
fn etc_dir() -> PathBuf {
    env::var_os("WOL_TOR_ETC").map_or_else(|| PathBuf::from("/etc/wol-tor"), PathBuf::from)
}

/// The one switch file: root-owned, read at boot by `wol-tor-rules.service`.
fn switches_path() -> PathBuf {
    etc_dir().join("switches")
}

/// What `apply` has done so far this boot (tmpfs: absent after a reboot = nothing applied).
fn applied_path() -> PathBuf {
    env::var_os("WOL_TOR_RUN")
        .map_or_else(|| PathBuf::from("/run/wol-tor"), PathBuf::from)
        .join("applied")
}

/// tor's DataDirectory in the gateway: kept across restarts and boots (guards, consensus).
const GATEWAY_STATE_DIR: &str = "/var/lib/wol-tor/gateway";

/// The tor binary to run: the packaged one, which must be root-owned and not writable by group or
/// others; `WOL_TOR_BIN` is a developer override and skips that check on purpose.
fn trusted_tor() -> Res<PathBuf> {
    let tor = which("tor", "WOL_TOR_BIN").ok_or("tor is not installed (package tor)")?;
    if env::var_os("WOL_TOR_BIN").is_none() {
        let m = fs::metadata(&tor).map_err(|e| format!("{}: {e}", tor.display()))?;
        transparent::tor_binary_trusted(m.uid(), m.mode())
            .map_err(|e| format!("{}: {e}", tor.display()))?;
    }
    Ok(tor)
}

/// The tor unit's main process: config, warm cache, then become tor. With `gateway` it is the
/// machine-wide tor (a system unit, `/run/wol-tor/gateway`, listeners from the switches file).
fn tor_run(g: &Group, gateway: bool) -> Res<()> {
    let (paths, listen) = if gateway {
        let sw = fs::read_to_string(switches_path())
            .map(|t| Switches::parse(&t))
            .unwrap_or_default();
        let mut paths = Paths::new(Path::new("/run"), g).map_err(|e| e.to_string())?;
        paths.data = PathBuf::from(GATEWAY_STATE_DIR);
        (paths, Some((sw.system, !sw.apps.is_empty())))
    } else {
        (
            Paths::new(&runtime_dir()?, g).map_err(|e| e.to_string())?,
            None,
        )
    };
    if listen == Some((false, false)) {
        return Err("the switches name no scope that needs the gateway".into());
    }
    let torrc = prepare(&paths, listen)?;
    if !gateway {
        let _ = cache::seed(&cache_dir()?, &paths.data, SystemTime::now());
    }
    let tor = trusted_tor()?;
    let argv = tor_argv(&tor, &torrc);
    let err = Command::new(&argv[0]).args(&argv[1..]).exec();
    Err(format!("tor could not start: {err}"))
}

/// Connects to tor's control socket, takes ownership, and keeps `ready` true while it is
/// bootstrapped. Reconnects after tor restarts.
fn watch(paths: Paths, ready: Arc<Ready>) {
    loop {
        let session = (|| -> Result<(), String> {
            let cookie = read_cookie(&paths.cookie).map_err(|e| e.to_string())?;
            let mut c = Control::connect(&paths.control, CONTROL_CONNECT_WAIT)
                .map_err(|e| e.to_string())?;
            c.authenticate(&cookie).map_err(|e| e.to_string())?;
            c.take_ownership().map_err(|e| e.to_string())?;
            loop {
                let b = c.bootstrap().map_err(|e| e.to_string())?;
                ready.set(b.done());
                thread::park_timeout(if b.done() {
                    HEALTH_POLL
                } else {
                    BOOTSTRAP_WATCH_POLL
                });
            }
        })();
        ready.set(false);
        if session.is_err() {
            thread::park_timeout(HOST_IDLE_POLL);
        }
    }
}

fn atlas_cmd(atlas: &[String], profile_dir: &Path, url: Option<&str>) -> Command {
    let mut c = Command::new(&atlas[0]);
    c.args(&atlas[1..]).args(profile::atlas_flags(profile_dir));
    if let Some(u) = url {
        c.arg("--").arg(u);
    }
    c
}

fn serve_commands(
    listener: UnixListener,
    iso: Arc<Isolator>,
    atlas: Vec<String>,
    profile_dir: PathBuf,
) {
    for conn in listener.incoming() {
        let Ok(mut conn) = conn else { continue };
        let _ = conn.set_read_timeout(Some(COMMAND_READ_WAIT));
        let mut line = String::new();
        let read = (&mut conn)
            .take(u64::try_from(host::MAX_COMMAND + 2).unwrap_or(4098))
            .read_to_string(&mut line);
        let answer = match read
            .map_err(|e| e.to_string())
            .and_then(|_| host::parse(&line).map_err(|e| e.to_string()))
        {
            Ok(HostCommand::Open(url)) => {
                match atlas_cmd(&atlas, &profile_dir, url.as_deref()).spawn() {
                    Ok(mut child) => {
                        let _ = thread::Builder::new()
                            .name("wol-tor-reap".into())
                            .spawn(move || child.wait());
                        "ok\n".to_string()
                    }
                    Err(e) => format!("error Atlas could not open the tab: {e}\n"),
                }
            }
            Ok(HostCommand::NewCircuit(site)) => format!("ok epoch {}\n", iso.new_circuit(&site)),
            Err(e) => format!("error {e}\n"),
        };
        let _ = conn.write_all(answer.as_bytes());
    }
}

/// The browser unit's main process.
fn host_run(g: &Group, atlas: Vec<String>, shared_tor: Option<PathBuf>) -> Res<i32> {
    if atlas.is_empty() {
        return Err("host needs the Atlas command after --".into());
    }
    let paths = Paths::new(&runtime_dir()?, g).map_err(|e| e.to_string())?;
    // Fail closed, both checks before anything starts: without the Tor policies (proxy, no QUIC,
    // private profile) or without an isolated namespace Atlas never runs.
    profile::verify(&paths.profile).map_err(|e| format!("the Tor profile is not in place: {e}"))?;
    netns::probe(Path::new("/")).map_err(|e| e.to_string())?;
    let listener = TcpListener::bind(units::RELAY_ADDR)
        .map_err(|e| format!("the relay could not listen on {}: {e}", units::RELAY_ADDR))?;
    let ready = Arc::new(Ready::default());
    let iso = Arc::new(Isolator::new(g.clone()));
    let relay = Arc::new(Relay {
        tor_socks: shared_tor.clone().unwrap_or_else(|| paths.socks.clone()),
        isolator: Arc::clone(&iso),
        ready: Arc::clone(&ready),
        ready_wait: READY_WAIT,
    });
    thread::Builder::new()
        .name("wol-tor-relay".into())
        .spawn(move || relay.serve(listener, journal))
        .map_err(|e| e.to_string())?;
    if shared_tor.is_some() {
        // The machine's tor is not ours to own or to watch (TAKEOWNERSHIP would stop it when this
        // group ends); a connect that tor cannot serve ends in the error page.
        ready.set(true);
    } else {
        let (p2, r2) = (paths.clone(), Arc::clone(&ready));
        thread::Builder::new()
            .name("wol-tor-watch".into())
            .spawn(move || watch(p2, r2))
            .map_err(|e| e.to_string())?;
    }
    // Atlas's profile is on the unit's own tmpfs runtime directory, never on disk.
    let profile_dir = paths.atlas_profile.clone();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&profile_dir)
        .map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&paths.host);
    let cmds =
        UnixListener::bind(&paths.host).map_err(|e| format!("{}: {e}", paths.host.display()))?;
    let (a2, d2) = (atlas.clone(), profile_dir.clone());
    thread::Builder::new()
        .name("wol-tor-cmds".into())
        .spawn(move || serve_commands(cmds, iso, a2, d2))
        .map_err(|e| e.to_string())?;
    let url = env::var("WOL_TOR_OPEN")
        .ok()
        .filter(|u| host::check_url(u).is_ok());
    let status = atlas_cmd(&atlas, &profile_dir, url.as_deref())
        .status()
        .map_err(|e| format!("Atlas could not start: {e}"))?;
    let _ = fs::remove_file(&paths.host);
    Ok(status.code().unwrap_or(1))
}

/// How long the doctor waits for the check service over tor.
const CHECK_WAIT: Duration = Duration::from_secs(45);
/// Longest reply read from the check service.
const CHECK_MAX_REPLY: u64 = 8192;
/// The canary query's wait.
const CANARY_WAIT: Duration = Duration::from_secs(5);

/// The switches: the root-owned file, plus "Keep Tor ready", which is a per-user unit and not a
/// boot-time scope.
fn load_switches() -> Res<Switches> {
    let mut sw = fs::read_to_string(switches_path())
        .map(|t| Switches::parse(&t))
        .unwrap_or_default();
    sw.keep_ready = keep_ready_on()?;
    Ok(sw)
}

/// What `apply` has put in place since boot.
fn applied_switches() -> Switches {
    fs::read_to_string(applied_path())
        .map(|t| Switches::parse(&t))
        .unwrap_or_default()
}

/// `switches show | set atlas|system|lan on|off | set keep-ready on|off | app add|remove NAME`.
/// The switch file is root-owned and the only one: changing it needs administrator rights (the
/// daemon runs this), and it prints the plan from what is applied; `apply` runs the plan.
fn switches_verb(args: &[String]) -> Res<()> {
    let old = load_switches()?;
    let mut new = old.clone();
    let on = |v: Option<&String>| match v.map(String::as_str) {
        Some("on") => Ok(true),
        Some("off") => Ok(false),
        _ => Err("expected on or off".to_string()),
    };
    match args.first().map(String::as_str) {
        Some("show") | None => {
            print!("{}", old.render());
            return Ok(());
        }
        Some("set") => match args.get(1).map(String::as_str) {
            Some("atlas") => new.atlas = on(args.get(2))?,
            Some("system") => new.system = on(args.get(2))?,
            Some("lan") => new.lan = on(args.get(2))?,
            Some("keep-ready") => new.keep_ready = on(args.get(2))?,
            _ => return Err("switches set takes atlas, system, lan or keep-ready".into()),
        },
        Some("app") => {
            let name = args.get(2).ok_or("switches app add|remove NAME")?;
            match args.get(1).map(String::as_str) {
                Some("add") => {
                    new.add_app(name).map_err(|e| e.to_string())?;
                }
                Some("remove") => {
                    new.apps.remove(name);
                }
                _ => return Err("switches app add|remove NAME".into()),
            }
        }
        Some(other) => return Err(format!("switches {other:?}: show, set or app")),
    }
    if new.keep_ready != old.keep_ready {
        set_keep_ready(new.keep_ready)?;
    }
    if (new.atlas, new.system, new.lan, &new.apps) != (old.atlas, old.system, old.lan, &old.apps) {
        let file = switches_path();
        if let Some(d) = file.parent() {
            fs::create_dir_all(d).map_err(|e| {
                format!(
                    "{}: {e} (changing the Tor scopes needs administrator rights)",
                    d.display()
                )
            })?;
        }
        let text = Switches {
            keep_ready: false,
            ..new.clone()
        }
        .render();
        shared::atomic::atomic_write(&file, ROOT_FILE_MODE, text.as_bytes()).map_err(|e| {
            format!(
                "{}: {e} (changing the Tor scopes needs administrator rights)",
                file.display()
            )
        })?;
    }
    for a in mode::plan(&applied_switches(), &new) {
        println!("plan {a:?}");
    }
    Ok(())
}

/// Mode of the files the root steps write (switches, applied, rule files): the owner writes, all
/// read; the umask of the unit does not matter.
const ROOT_FILE_MODE: u32 = 0o644;

/// One root-side step of `apply`: a file written atomically (a reader of the rule files sees the
/// old text or the new, never half) or a program started through `privileged_command` (cleared
/// environment, fixed PATH, no shell; STANDARDS 7.1 "Root commands", QA-A9-SEC-12).
fn run_step(st: &transparent::PrivStep) -> Res<()> {
    if let Some((path, text)) = &st.write_file {
        let p = Path::new(path);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        return shared::atomic::atomic_write(p, ROOT_FILE_MODE, text.as_bytes())
            .map_err(|e| format!("{path}: {e}"));
    }
    let mut c = wol_core::privileged::privileged_command(&st.argv[0]);
    c.args(&st.argv[1..]);
    if st.stdin.is_some() {
        c.stdin(std::process::Stdio::piped());
    }
    let mut child = c.spawn().map_err(|e| format!("{}: {e}", st.argv[0]))?;
    if let (Some(text), Some(mut stdin)) = (&st.stdin, child.stdin.take()) {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} ended with {status}", st.argv.join(" ")))
    }
}

/// `apply [--dry-run] [--reload-rules] [--tor-uid N]`: runs the plan from what is applied (nothing,
/// after a boot) to the switch file; run at boot by `wol-tor-rules.service`, after the firewall and
/// before the network. `--reload-rules` only loads the rule files again (a firewall reload).
/// Needs root. On a failed step it stops, leaves what is applied unchanged and says which step.
fn apply_verb(args: &mut Vec<String>) -> Res<i32> {
    let take = |args: &mut Vec<String>, flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .map(|i| args.remove(i))
            .is_some()
    };
    let dry = take(args, "--dry-run");
    let reload = take(args, "--reload-rules");
    let tor_uid = match args.iter().position(|a| a == "--tor-uid") {
        Some(i) if i + 1 < args.len() => {
            let v = args.remove(i + 1);
            args.remove(i);
            v.parse::<u32>().map_err(|_| "--tor-uid is a number")?
        }
        _ => uid_of("tor")?,
    };
    let ctx = mode::Ctx { tor_uid };
    let to = fs::read_to_string(switches_path())
        .map(|t| Switches::parse(&t))
        .unwrap_or_default();
    let list: Vec<transparent::PrivStep> = if reload {
        mode::reload_rules_steps(&to, &ctx)
    } else {
        let from = applied_switches();
        mode::plan(&from, &to)
            .iter()
            .flat_map(|a| mode::steps(a, &from, &to, &ctx))
            .collect()
    };
    if dry {
        print_steps(&list);
        return Ok(0);
    }
    for (i, st) in list.iter().enumerate() {
        run_step(st).map_err(|e| format!("step {} of {}: {e}", i + 1, list.len()))?;
    }
    if !reload {
        let p = applied_path();
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        shared::atomic::atomic_write(&p, ROOT_FILE_MODE, to.render().as_bytes())
            .map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(0)
}

fn print_steps(steps: &[transparent::PrivStep]) {
    for st in steps {
        match (&st.write_file, &st.stdin) {
            (Some((path, text)), _) => println!("write {path}: {}", text.trim_end()),
            (None, Some(text)) => {
                println!("run {} <<EOF", st.argv.join(" "));
                print!("{text}");
                println!("EOF");
            }
            (None, None) => println!("run {}", st.argv.join(" ")),
        }
    }
}

fn uid_of(user: &str) -> Res<u32> {
    let passwd = fs::read_to_string("/etc/passwd").map_err(|e| format!("/etc/passwd: {e}"))?;
    passwd
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            (f.first() == Some(&user))
                .then(|| f.get(2).and_then(|u| u.parse().ok()))
                .flatten()
        })
        .next()
        .ok_or_else(|| format!("no user {user:?} (install the tor package)"))
}

/// `gateway ...`: dry runs that print the privileged work (the daemon runs the same functions).
fn gateway_verb(args: &mut Vec<String>) -> Res<()> {
    let lan = args
        .iter()
        .position(|a| a == "--lan")
        .map(|i| args.remove(i))
        .is_some();
    // APP and INDEX are the two arguments from `at` on.
    let app_ns = |a: &[String], at: usize| -> Res<AppNs> {
        let need = "needs APP INDEX";
        let idx: u8 = a
            .get(at + 1)
            .ok_or(need)?
            .parse()
            .map_err(|_| "INDEX is a number")?;
        AppNs::new(a.get(at).ok_or(need)?, idx).map_err(|e| e.to_string())
    };
    match args.first().map(String::as_str) {
        Some("rules") => match args.get(1).map(String::as_str) {
            Some("system") => {
                let g = transparent::SystemGateway {
                    tor_uid: uid_of("tor")?,
                    lan: if lan {
                        transparent::lan_ranges()
                    } else {
                        vec![]
                    },
                };
                print!("{}", transparent::system_ruleset(&g).render());
            }
            Some("apps") => print!("{}", transparent::app_host_ruleset().render()),
            Some("ns") => print!("{}", transparent::app_netns_ruleset().render()),
            _ => return Err("gateway rules system|apps|ns [--lan]".into()),
        },
        Some("steps") => match args.get(1).map(String::as_str) {
            Some("up") => print_steps(&transparent::gateway_up_steps()),
            Some("down") => print_steps(&transparent::gateway_down_steps()),
            Some("add") => print_steps(&transparent::netns_steps(&app_ns(args, 2)?)),
            Some("remove") => print_steps(&transparent::netns_down_steps(&app_ns(args, 2)?)),
            _ => return Err("gateway steps up|down|add APP INDEX|remove APP INDEX".into()),
        },
        Some("join") => {
            let ns = app_ns(args, 1)?;
            let (uid, gid) = (args.get(3), args.get(4));
            let dd = args
                .iter()
                .position(|a| a == "--")
                .ok_or("gateway join APP INDEX UID GID -- CMD")?;
            let id = |v: Option<&String>| {
                v.and_then(|x| x.parse::<u32>().ok())
                    .ok_or("UID and GID are numbers")
            };
            println!(
                "{}",
                transparent::join_argv(
                    &ns,
                    &env::current_exe().map_err(|e| e.to_string())?,
                    id(uid)?,
                    id(gid)?,
                    &args[dd + 1..]
                )
                .join(" ")
            );
        }
        Some("torrc") => {
            let sw = load_switches()?;
            let paths = Paths::new(
                Path::new("/run"),
                &Group::new("gateway").map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            let cfg = config(&paths, Some((sw.system, !sw.apps.is_empty())))?;
            print!("{}", cfg.render().map_err(|e| e.to_string())?);
        }
        Some("unit") => {
            let me = env::current_exe().map_err(|e| e.to_string())?;
            print!("{}", units::gateway_unit_file(&me, "tor"));
        }
        Some("rules-unit") => {
            let me = env::current_exe().map_err(|e| e.to_string())?;
            print!("{}", units::rules_unit_file(&me));
        }
        Some("nft-dropin") => {
            let me = env::current_exe().map_err(|e| e.to_string())?;
            print!("{}", units::nftables_dropin(&me));
        }
        _ => return Err("gateway rules|steps|join|torrc|unit".into()),
    }
    Ok(())
}

/// `bridges show | set LINE | clear` for the user's Atlas groups.
fn bridges_verb(args: &[String]) -> Res<()> {
    let file = user_bridges_file()?;
    match args.first().map(String::as_str) {
        Some("show") | None => {
            for b in bridges(&file)? {
                println!("{}", b.line());
            }
        }
        Some("clear") => match fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", file.display())),
        },
        Some("set") => {
            let line = args[1..].join(" ");
            let b = Bridge::parse(&line).map_err(|e| e.to_string())?;
            if let Some(d) = file.parent() {
                fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
            }
            fs::write(&file, format!("{}\n", b.line()))
                .map_err(|e| format!("{}: {e}", file.display()))?;
            fs::set_permissions(&file, fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        Some(other) => return Err(format!("bridges {other:?}: show, set or clear")),
    }
    Ok(())
}

/// `new-identity [--group G]`: clean circuits for new connections (tor's NEWNYM).
fn new_identity(paths: &Paths) -> Res<()> {
    let cookie = read_cookie(&paths.cookie).map_err(|e| e.to_string())?;
    let mut c = Control::connect(&paths.control, QUICK_CONNECT_WAIT).map_err(|e| e.to_string())?;
    c.authenticate(&cookie).map_err(|e| e.to_string())?;
    c.command("SIGNAL NEWNYM")
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The check service's verdict over tor's own SOCKS socket (never a direct connection).
fn check_over_tor(paths: &Paths) -> Option<bool> {
    let mut s = UnixStream::connect(&paths.socks).ok()?;
    s.set_read_timeout(Some(CHECK_WAIT)).ok()?;
    s.set_write_timeout(Some(CHECK_WAIT)).ok()?;
    let t = Target {
        host: Host::Domain("check.torproject.org".into()),
        port: 80,
    };
    if client_connect(&mut s, "doctor", "0", &t).ok()? != wol_tor::socks::Reply::Succeeded {
        return None;
    }
    s.write_all(b"GET /api/ip HTTP/1.0\r\nHost: check.torproject.org\r\nConnection: close\r\n\r\n")
        .ok()?;
    let mut out = String::new();
    s.take(CHECK_MAX_REPLY).read_to_string(&mut out).ok()?;
    doctor::parse_is_tor(&out)
}

fn canary_answer() -> Option<String> {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").ok()?;
    sock.set_read_timeout(Some(CANARY_WAIT)).ok()?;
    sock.send_to(
        &doctor::dns_query(doctor::CANARY_NAME)?,
        doctor::CANARY_SERVER,
    )
    .ok()?;
    let mut buf = [0u8; 512];
    let (n, _) = sock.recv_from(&mut buf).ok()?;
    doctor::dns_first_a(&buf[..n])
}

/// `doctor [--group G | --gateway]`: measures, then judges with [`doctor::evaluate`].
fn doctor_verb(args: &mut Vec<String>) -> Res<i32> {
    let gateway = args
        .iter()
        .position(|a| a == "--gateway")
        .map(|i| args.remove(i))
        .is_some();
    let sw = load_switches()?;
    let paths = if gateway {
        Paths::new(
            Path::new("/run"),
            &Group::new("gateway").map_err(|e| e.to_string())?,
        )
    } else {
        Paths::new(&runtime_dir()?, &group_arg(args)?)
    }
    .map_err(|e| e.to_string())?;
    let mut o = Observations {
        now: Some(SystemTime::now()),
        ..Observations::default()
    };
    o.bootstrap = status(&paths).ok().map(|b| b.progress);
    if o.bootstrap == Some(100) {
        o.is_tor = check_over_tor(&paths);
    }
    o.consensus = cache::consensus_window(&paths.data);
    if sw.system {
        o.canary_answer = canary_answer();
        let out = Command::new("nft")
            .args(["list", "table", "inet", transparent::SYSTEM_TABLE])
            .output();
        o.kill_switch_loaded = match out {
            Ok(r) if r.status.success() => Some(true),
            Ok(r) if String::from_utf8_lossy(&r.stderr).contains("not permitted") => None,
            Ok(_) => Some(false),
            Err(_) => None,
        };
    }
    let findings = doctor::evaluate(&o);
    for f in &findings {
        println!("{:<12} {:<12} {}", f.check, f.level.to_string(), f.text);
    }
    Ok(i32::from(doctor::worst(&findings) == doctor::Level::Fail))
}

fn ask(paths: &Paths, line: &str) -> Res<String> {
    let mut s = UnixStream::connect(&paths.host)
        .map_err(|e| format!("the Tor group is not running ({e})"))?;
    s.set_read_timeout(Some(SOCKET_IO_WAIT))
        .map_err(|e| e.to_string())?;
    s.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    s.shutdown(std::net::Shutdown::Write)
        .map_err(|e| e.to_string())?;
    let mut out = String::new();
    s.take(4096)
        .read_to_string(&mut out)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

fn systemctl_active(unit: &str) -> bool {
    // The exit status is the whole answer of `is-active --quiet` (0 active, 3 not); a systemctl
    // that cannot run means no user manager, so the unit is not active either.
    match Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", unit])
        .status()
    {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

fn run_argv(argv: &[String], env_open: Option<&str>) -> Res<()> {
    let mut c = Command::new(&argv[0]);
    c.args(&argv[1..]);
    if let Some(u) = env_open {
        // systemd-run --setenv=NAME copies it from this process.
        c.env("WOL_TOR_OPEN", u);
    }
    let s = c
        .status()
        .map_err(|e| format!("systemd-run could not start: {e}"))?;
    if s.success() {
        Ok(())
    } else {
        Err(format!("systemd-run ended with {s}"))
    }
}

/// The machine tor's SOCKS socket when the machine scope is on and tor is running.
fn machine_tor_socket() -> Option<PathBuf> {
    let on = fs::read_to_string(switches_path())
        .map(|t| Switches::parse(&t))
        .is_ok_and(|s| s.system);
    let sock = Paths::new(Path::new("/run"), &Group::new("gateway").ok()?)
        .ok()?
        .socks;
    (on && sock.exists()).then_some(sock)
}

/// From the session (the bridge, or a person in a terminal): open a tab in the group.
fn open(g: &Group, url: Option<String>) -> Res<()> {
    if let Some(u) = &url {
        host::check_url(u).map_err(|e| e.to_string())?;
    }
    let paths = Paths::new(&runtime_dir()?, g).map_err(|e| e.to_string())?;
    if systemctl_active(&units::host_unit_name(g)) {
        let line = match &url {
            Some(u) => format!("open {u}\n"),
            None => "open\n".into(),
        };
        let a = ask(&paths, &line)?;
        return if a.starts_with("ok") {
            Ok(())
        } else {
            Err(a.trim().to_string())
        };
    }
    let me = env::current_exe().map_err(|e| e.to_string())?;
    let atlas = which("atlas", "WOL_TOR_ATLAS").ok_or("Atlas is not installed")?;
    // The policies and fonts.conf go into place before the browser unit exists (it binds them).
    profile::write_to(&paths.profile).map_err(|e| format!("{}: {e}", paths.profile.display()))?;
    // With the machine scope on, the group has no tor of its own (it would be redirected into the
    // machine's tor: a double hop); its relay uses the machine tor's SOCKS socket.
    let shared = machine_tor_socket();
    if shared.is_none() && !systemctl_active(&units::tor_unit_name(g)) {
        run_argv(&units::tor_unit(g, &me), None)?;
    }
    let mut pass: Vec<&str> = SESSION_ENV
        .iter()
        .copied()
        .filter(|v| env::var_os(v).is_some())
        .collect();
    if url.is_some() {
        pass.push("WOL_TOR_OPEN");
    }
    run_argv(
        &units::host_unit(g, &paths, &me, &atlas, &pass, shared.as_deref()),
        url.as_deref(),
    )
}

/// Bootstrap progress of a group's tor, over its control socket.
fn status(paths: &Paths) -> Res<wol_tor::control::Bootstrap> {
    let cookie = read_cookie(&paths.cookie).map_err(|e| e.to_string())?;
    let mut c = Control::connect(&paths.control, QUICK_CONNECT_WAIT).map_err(|e| e.to_string())?;
    c.authenticate(&cookie).map_err(|e| e.to_string())?;
    c.bootstrap().map_err(|e| e.to_string())
}

/// The country of the current exit, for the status row.
fn exit_country(paths: &Paths) -> Option<String> {
    let cookie = read_cookie(&paths.cookie).ok()?;
    let mut c = Control::connect(&paths.control, QUICK_CONNECT_WAIT).ok()?;
    c.authenticate(&cookie).ok()?;
    c.exit_country().ok().flatten()
}

/// Waits until the group's tor is bootstrapped; prints the milliseconds it took (benchmark M1).
fn wait_ready(paths: &Paths, timeout: Duration) -> Res<u128> {
    let t0 = Instant::now();
    while t0.elapsed() < timeout {
        if status(paths).is_ok_and(|b| b.done()) {
            return Ok(t0.elapsed().as_millis());
        }
        thread::park_timeout(RELAY_IDLE_POLL);
    }
    Err(format!(
        "Tor did not finish bootstrapping in {} s",
        timeout.as_secs()
    ))
}

/// The relay alone on `listen`, no Atlas and no namespace check: for the benchmark (M4) and
/// for tests on a machine without user namespaces. Never started by a unit.
fn relay_only(g: &Group, listen: &str) -> Res<i32> {
    let paths = Paths::new(&runtime_dir()?, g).map_err(|e| e.to_string())?;
    let addr: std::net::SocketAddr = listen
        .parse()
        .map_err(|_| format!("{listen} is not an address"))?;
    if !addr.ip().is_loopback() {
        return Err("the relay listens on loopback only".into());
    }
    let listener = TcpListener::bind(addr).map_err(|e| e.to_string())?;
    let ready = Arc::new(Ready::default());
    let (p2, r2) = (paths.clone(), Arc::clone(&ready));
    thread::Builder::new()
        .name("wol-tor-watch".into())
        .spawn(move || watch(p2, r2))
        .map_err(|e| e.to_string())?;
    let relay = Arc::new(Relay {
        tor_socks: paths.socks,
        isolator: Arc::new(Isolator::new(g.clone())),
        ready,
        ready_wait: READY_WAIT,
    });
    relay.serve(listener, journal).map_err(|e| e.to_string())?;
    Ok(0)
}

/// The cache unit: run tor until bootstrapped, keep its documents and state.
fn cache_run() -> Res<()> {
    // With the machine scope on, a user-run tor would be redirected into the machine's tor.
    if fs::read_to_string(switches_path())
        .map(|t| Switches::parse(&t))
        .is_ok_and(|s| s.system)
    {
        println!("the machine scope keeps its own tor warm: nothing to do");
        return Ok(());
    }
    // The unit sets this; ProtectHome hides the switch's file from it, so the unit file's
    // existence is the switch (`keep-ready off` removes it).
    if env::var(ready::UNIT_ENV).as_deref() != Ok("on") {
        return Err(
            "cache-run is started by wol-tor-cache.service after `wol-tor keep-ready on`".into(),
        );
    }
    let run = env::var_os("RUNTIME_DIRECTORY")
        .map(PathBuf::from)
        .ok_or("RUNTIME_DIRECTORY is not set: cache-run is started by wol-tor-cache.service")?;
    let state = env::var_os("STATE_DIRECTORY")
        .map(PathBuf::from)
        .unwrap_or(cache_dir()?);
    let g = Group::new("cache-run").map_err(|e| e.to_string())?;
    let paths = Paths::new(run.parent().and_then(Path::parent).unwrap_or(&run), &g)
        .map_err(|e| e.to_string())?;
    let torrc = prepare(&paths, None)?;
    let _ = cache::seed(&state, &paths.data, SystemTime::now());
    let tor = trusted_tor()?;
    let argv = tor_argv(&tor, &torrc);
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .spawn()
        .map_err(|e| format!("tor could not start: {e}"))?;
    let deadline = Instant::now() + BOOTSTRAP_DEADLINE;
    let mut owner = None;
    while Instant::now() < deadline && owner.is_none() {
        thread::park_timeout(BOOTSTRAP_POLL);
        let Ok(cookie) = read_cookie(&paths.cookie) else {
            continue;
        };
        let Ok(mut c) = Control::connect(&paths.control, QUICK_CONNECT_WAIT) else {
            continue;
        };
        if c.authenticate(&cookie).is_ok() && c.bootstrap().is_ok_and(|b| b.done()) {
            owner = Some(c);
        }
    }
    let Some(mut c) = owner else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("tor did not finish bootstrapping in 150 s".into());
    };
    // A clean exit writes `state` (the guard, the circuit-build timeout); then keep the files.
    let _ = c.command("SIGNAL HALT");
    let stop = Instant::now() + STOP_DEADLINE;
    while Instant::now() < stop && matches!(child.try_wait(), Ok(None)) {
        thread::park_timeout(STOP_POLL);
    }
    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
    }
    let _ = child.wait();
    let kept = cache::collect(&paths.data, &state).map_err(|e| e.to_string());
    kept.map(|_| ())
}

/// `policies [--write DIR]`: print the Tor instance's policies, or write them with `fonts.conf`.
fn policies_verb(args: &[String]) -> Res<i32> {
    if args.first().map(String::as_str) == Some("--write") {
        let dir = args.get(1).ok_or("--write needs a directory")?;
        profile::write_to(Path::new(dir)).map_err(|e| format!("{dir}: {e}"))?;
        println!(
            "wrote {dir}/{}/{} and {dir}/{}",
            profile::POLICY_SUBDIR,
            profile::POLICY_FILE,
            profile::FONTS_FILE
        );
        return Ok(0);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&profile::policies()).map_err(|e| e.to_string())?
    );
    Ok(0)
}

fn keep_ready_verb(args: &[String]) -> Res<i32> {
    match args.first().map(String::as_str) {
        Some("on") => set_keep_ready(true).map(|()| 0),
        Some("off") => set_keep_ready(false).map(|()| 0),
        Some("status") | None => {
            println!("{}", if keep_ready_on()? { "on" } else { "off" });
            Ok(0)
        }
        Some(other) => Err(format!("keep-ready takes on, off or status, not {other:?}")),
    }
}

fn units_verb(args: &mut Vec<String>) -> Res<i32> {
    let g = group_arg(args)?;
    let me = env::current_exe().map_err(|e| e.to_string())?;
    let paths = Paths::new(&runtime_dir()?, &g).map_err(|e| e.to_string())?;
    println!("{}", units::tor_unit(&g, &me).join(" "));
    let atlas = Path::new("/usr/bin/atlas");
    println!(
        "{}",
        units::host_unit(&g, &paths, &me, atlas, &SESSION_ENV, None).join(" ")
    );
    Ok(0)
}

/// `app-exec -- CMD...`: the first step inside an app's namespace (after `ip netns exec`): empties
/// the host resolvers' socket directories in this private mount namespace, then becomes CMD.
fn app_exec(args: &[String]) -> Res<i32> {
    let cmd = match args.iter().position(|a| a == "--") {
        Some(i) if i + 1 < args.len() => &args[i + 1..],
        _ => return Err("app-exec -- COMMAND...".into()),
    };
    for dir in transparent::MASK_DIRS {
        if Path::new(dir).is_dir() {
            run_step(&transparent::PrivStep {
                argv: transparent::mask_argv(dir),
                stdin: None,
                write_file: None,
            })?;
        }
    }
    let err = Command::new(&cmd[0]).args(&cmd[1..]).exec();
    Err(format!("{}: {err}", cmd[0]))
}

fn main() -> ExitCode {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!(concat!(
            "usage: wol-tor open|newcircuit|new-identity|tor-run|host|cache-run|keep-ready|switches|apply|",
            "gateway|bridges|doctor|status|wait-ready|relay|torrc|policies|units|check-netns [--group G] ..."
        ),);
        return ExitCode::from(2);
    }
    let verb = args.remove(0);
    let result: Res<i32> = (|| match verb.as_str() {
        "open" => {
            let g = group_arg(&mut args)?;
            open(&g, args.first().cloned()).map(|()| 0)
        }
        "newcircuit" => {
            let g = group_arg(&mut args)?;
            let site = args.first().ok_or("newcircuit needs a host")?;
            let paths = Paths::new(&runtime_dir()?, &g).map_err(|e| e.to_string())?;
            print!("{}", ask(&paths, &format!("newcircuit {site}\n"))?);
            Ok(0)
        }
        "tor-run" => {
            let gateway = args
                .iter()
                .position(|a| a == "--gateway")
                .map(|i| args.remove(i))
                .is_some();
            let g = if gateway {
                Group::new("gateway").map_err(|e| e.to_string())?
            } else {
                group_arg(&mut args)?
            };
            tor_run(&g, gateway).map(|()| 0)
        }
        "host" => {
            let g = group_arg(&mut args)?;
            let atlas = match args.iter().position(|a| a == "--") {
                Some(i) => args.split_off(i + 1),
                None => vec![],
            };
            let shared = match args.iter().position(|a| a == "--tor-socks") {
                Some(i) if i + 1 < args.len() => Some(PathBuf::from(&args[i + 1])),
                Some(_) => return Err("--tor-socks needs a path".into()),
                None => None,
            };
            host_run(&g, atlas, shared)
        }
        "cache-run" => cache_run().map(|()| 0),
        "status" => {
            let g = group_arg(&mut args)?;
            let b = status(&Paths::new(&runtime_dir()?, &g).map_err(|e| e.to_string())?)?;
            println!("bootstrap {}% {}", b.progress, b.tag);
            if b.done() {
                let country =
                    exit_country(&Paths::new(&runtime_dir()?, &g).map_err(|e| e.to_string())?);
                println!("exit {}", country.as_deref().unwrap_or("unknown"));
            }
            Ok(if b.done() { 0 } else { 3 })
        }
        "wait-ready" => {
            let g = group_arg(&mut args)?;
            let secs: u64 = args
                .first()
                .map_or(Ok(120), |s| s.parse())
                .map_err(|_| "wait-ready takes seconds")?;
            let ms = wait_ready(
                &Paths::new(&runtime_dir()?, &g).map_err(|e| e.to_string())?,
                Duration::from_secs(secs),
            )?;
            println!("ready_ms {ms}");
            Ok(0)
        }
        "relay" => {
            let g = group_arg(&mut args)?;
            relay_only(&g, args.first().map_or(units::RELAY_ADDR, String::as_str))
        }
        "torrc" => {
            let g = group_arg(&mut args)?;
            let paths = Paths::new(&runtime_dir()?, &g).map_err(|e| e.to_string())?;
            print!(
                "{}",
                config(&paths, None)?.render().map_err(|e| e.to_string())?
            );
            Ok(0)
        }
        "apply" => apply_verb(&mut args),
        "switches" => switches_verb(&args).map(|()| 0),
        "gateway" => gateway_verb(&mut args).map(|()| 0),
        "bridges" => bridges_verb(&args).map(|()| 0),
        "new-identity" => {
            let g = group_arg(&mut args)?;
            new_identity(&Paths::new(&runtime_dir()?, &g).map_err(|e| e.to_string())?).map(|()| 0)
        }
        "doctor" => doctor_verb(&mut args),
        "policies" => policies_verb(&args),
        "keep-ready" => keep_ready_verb(&args),
        "units" => units_verb(&mut args),
        "app-exec" => app_exec(&args),
        "check-netns" => netns::probe(Path::new("/"))
            .map(|()| {
                println!("isolated: only loopback, no route");
                0
            })
            .map_err(|e| e.to_string()),
        other => Err(format!("{other:?} is not a wol-tor verb")),
    })();
    match result {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(e) => {
            eprintln!("wol-tor: {e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(argv: &[&str], write_file: Option<(&Path, &str)>) -> transparent::PrivStep {
        transparent::PrivStep {
            argv: argv.iter().map(|s| (*s).to_owned()).collect(),
            stdin: None,
            write_file: write_file.map(|(p, t)| (p.to_string_lossy().into_owned(), t.to_owned())),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let d = env::temp_dir().join(format!("wol-tor-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// QA-A9-SEC-12: a root step runs with a cleared environment and the fixed PATH, not the
    /// unit's (`CARGO_MANIFEST_DIR` is set by cargo and must not reach the child).
    #[test]
    fn a_root_step_starts_with_a_cleared_environment() {
        let script = r#"test -z "$CARGO_MANIFEST_DIR" && test "$PATH" = /usr/bin:/usr/sbin"#;
        run_step(&step(&["/bin/sh", "-c", script], None)).unwrap();
    }

    /// QA-A9-SEC-12: a rule file is replaced whole (temp name, rename), with the mode the step
    /// means whatever the umask, and no temporary is left behind.
    #[test]
    fn a_rule_file_is_written_atomically_with_its_mode() {
        let d = scratch("rules");
        let f = d.join("sub/rules.nft");
        run_step(&step(&[], Some((&f, "one\n")))).unwrap();
        run_step(&step(&[], Some((&f, "two\n")))).unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "two\n");
        assert_eq!(fs::metadata(&f).unwrap().mode() & 0o7777, ROOT_FILE_MODE);
        let left: Vec<_> = fs::read_dir(f.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, ["rules.nft"], "no temporary left");
        let _ = fs::remove_dir_all(&d);
    }
}
