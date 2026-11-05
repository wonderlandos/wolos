// SPDX-License-Identifier: BSD-2-Clause
//! `atlas-torrent`: the Atlas browser's torrent helper.
//!
//! ```text
//! atlas-torrent launch OPTIONS   start `serve` as the confined user service (what the browser runs)
//! atlas-torrent launch --check-only OPTIONS
//!                                run `serve --check-only` in a unit with the same fences
//!                                (`atlas-torrent-check`) and print what it saw from inside
//! atlas-torrent serve OPTIONS [--token-file PATH] [--check-only]
//!                                serve the socket in this process (what the service runs); it first
//!                                checks its own confinement and refuses otherwise (confined.rs)
//! tools feature only (tests and the e2e, never in the package):
//! atlas-torrent make FILE --tracker URL --out FILE.torrent       a .torrent of FILE
//! atlas-torrent seed FILE.torrent --dir DIR --state DIR --port N [--rate BPS]  share DIR's copy
//! serve --unconfined (tests outside a unit), launch/serve --local-peers (loopback trackers and peers)
//!
//! OPTIONS: --downloads DIR --state DIR [--socket PATH] [--peer-exe PATH]... [--sharing]
//!          [--no-dht] [--idle SECONDS] [--tracker URL]...
//! ```
#![forbid(unsafe_code)]

use atlas_torrent::{confined, engine, sandbox, server};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

/// Seconds the service waits for the browser to come back before it stops (a restart of Atlas fits).
const IDLE_DEFAULT: u64 = 20;

#[derive(Default, Debug)]
struct Opts {
    downloads: Option<PathBuf>,
    state: Option<PathBuf>,
    socket: Option<PathBuf>,
    peer_exe: Vec<PathBuf>,
    sharing: bool,
    no_dht: bool,
    idle: Option<u64>,
    trackers: Vec<String>,
    port: Option<u16>,
    dir: Option<PathBuf>,
    out: Option<PathBuf>,
    rate: Option<u32>,
    token_file: Option<PathBuf>,
    check_only: bool,
    unconfined: bool,
    local_peers: bool,
    rest: Vec<String>,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut i = 0;
    let val = |i: &mut usize| -> Result<String, String> {
        *i += 1;
        args.get(*i)
            .cloned()
            .ok_or_else(|| format!("{} needs a value", args[*i - 1]))
    };
    while i < args.len() {
        match args[i].as_str() {
            "--downloads" => o.downloads = Some(val(&mut i)?.into()),
            "--state" => o.state = Some(val(&mut i)?.into()),
            "--socket" => o.socket = Some(val(&mut i)?.into()),
            "--peer-exe" => o.peer_exe.push(val(&mut i)?.into()),
            "--sharing" => o.sharing = true,
            "--no-dht" => o.no_dht = true,
            "--idle" => o.idle = Some(val(&mut i)?.parse().map_err(|_| "--idle takes seconds")?),
            "--tracker" => o.trackers.push(val(&mut i)?),
            "--port" => o.port = Some(val(&mut i)?.parse().map_err(|_| "--port takes a number")?),
            "--dir" => o.dir = Some(val(&mut i)?.into()),
            "--rate" => {
                o.rate = Some(
                    val(&mut i)?
                        .parse()
                        .map_err(|_| "--rate takes bytes per second")?,
                )
            }
            "--out" => o.out = Some(val(&mut i)?.into()),
            "--token-file" => o.token_file = Some(val(&mut i)?.into()),
            "--check-only" => o.check_only = true,
            #[cfg(feature = "tools")]
            "--unconfined" => o.unconfined = true,
            #[cfg(feature = "tools")]
            "--local-peers" => o.local_peers = true,
            a if a.starts_with("--") => return Err(format!("unknown option {a}")),
            a => o.rest.push(a.to_string()),
        }
        i += 1;
    }
    Ok(o)
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

fn need(p: &Option<PathBuf>, what: &str) -> Result<PathBuf, String> {
    let p = p.clone().ok_or_else(|| format!("{what} is needed"))?;
    if !p.is_absolute() {
        return Err(format!("{what} must be an absolute path"));
    }
    Ok(p)
}

fn serve_args(
    o: &Opts,
    downloads: &Path,
    state: &Path,
    socket: &Path,
    token_file: &Path,
) -> Vec<String> {
    let mut a = vec![
        "--downloads".to_string(),
        downloads.display().to_string(),
        "--state".into(),
        state.display().to_string(),
        "--socket".into(),
        socket.display().to_string(),
        "--token-file".into(),
        token_file.display().to_string(),
    ];
    if o.local_peers {
        a.push("--local-peers".into());
    }
    for e in &o.peer_exe {
        a.push("--peer-exe".into());
        a.push(e.display().to_string());
    }
    if o.sharing {
        a.push("--sharing".into());
    }
    if o.no_dht {
        a.push("--no-dht".into());
    }
    a.push("--idle".into());
    a.push(o.idle.unwrap_or(IDLE_DEFAULT).to_string());
    for t in &o.trackers {
        a.push("--tracker".into());
        a.push(t.clone());
    }
    a
}

/// How often `launch` looks for the helper's socket (100 times: 5 s).
const SOCKET_POLL: Duration = Duration::from_millis(50);
/// How often the tests' seeder prints its rows.
#[cfg(feature = "tools")]
const SEED_REPORT: Duration = Duration::from_secs(2);

/// The service manager's programs, never found through the browser's PATH (DL-SEC-15).
const SYSTEMD_RUN: &str = "/usr/bin/systemd-run";
const SYSTEMCTL: &str = "/usr/bin/systemctl";

/// Why `--downloads` cannot be handed to the unit as the only writable folder: it is the home
/// folder or holds it, or it holds the helper's state or the account's settings (DL-SEC-05: the
/// bind would give the helper the whole home read-write).
fn downloads_refusal(downloads: &Path, home: Option<&Path>, state: &Path) -> Option<String> {
    let mut guarded: Vec<PathBuf> = vec![state.to_path_buf()];
    if let Some(h) = home {
        guarded.push(h.to_path_buf());
        guarded.push(h.join(".config"));
        guarded.push(h.join(".local"));
        guarded.push(h.join(".ssh"));
    }
    if guarded.iter().any(|g| g.starts_with(downloads)) || downloads.parent().is_none() {
        return Some(format!(
            "{} holds your home or settings folders: choose a download folder inside your home \
             folder (like Downloads) for torrents",
            downloads.display()
        ));
    }
    None
}

/// What `launch` saw: started, or why not (the browser shows the text; the exit status tells a
/// refused confinement apart).
#[derive(Debug)]
enum LaunchError {
    Refused(String),
    Other(String),
}

impl From<String> for LaunchError {
    fn from(s: String) -> Self {
        LaunchError::Other(s)
    }
}

impl From<&str> for LaunchError {
    fn from(s: &str) -> Self {
        LaunchError::Other(s.to_string())
    }
}

fn launch(o: &Opts) -> Result<(), LaunchError> {
    let downloads = need(&o.downloads, "--downloads")?;
    let state = need(&o.state, "--state")?;
    let socket = o
        .socket
        .clone()
        .or_else(|| server::default_socket(&env))
        .ok_or("no XDG_RUNTIME_DIR")?;
    let run = socket.parent().ok_or("socket has no folder")?.to_path_buf();
    for p in [&downloads, &state, &run] {
        if !sandbox::fit(p) {
            return Err(format!(
                "{} cannot be handed to the service: {}",
                p.display(),
                sandbox::UNFIT_TEXT
            )
            .into());
        }
    }
    let home = env("HOME").map(PathBuf::from).filter(|h| h.is_absolute());
    if let Some(why) = downloads_refusal(&downloads, home.as_deref(), &state) {
        return Err(why.into());
    }
    std::fs::create_dir_all(&state).map_err(|e| format!("state folder: {e}"))?;
    std::fs::create_dir_all(&run).map_err(|e| format!("socket folder: {e}"))?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // A fresh token for the browser that runs this launch (printed to it on stdout, handed to the
    // service in a file it reads once); a helper of an earlier browser session is stopped first,
    // its jobs continue in the new one.
    // DL-SEC-01: first the same fences in a unit that only looks from inside and ends; the real
    // helper is started only when that unit saw every fence (and checks again itself).
    let seen = check_in_unit(o, &exe, &downloads, &state, &run, &socket)?;
    if o.check_only {
        print!("{seen}");
        return Ok(());
    }
    let token = new_token()?;
    let refused = run.join(confined::REFUSED_FILE);
    let _ = std::fs::remove_file(&refused);
    let tfile = server::write_token_file(&run, &token, &new_token()?[..16])?;
    let argv = sandbox::argv(
        &exe,
        &downloads,
        &state,
        &run,
        &serve_args(o, &downloads, &state, &socket, &tfile),
    );
    // systemd-run --user finds the user manager through $XDG_RUNTIME_DIR/bus; a session whose runtime
    // folder is elsewhere (a nested compositor) still has the account's own one.
    let has_bus = |d: &Path| d.join("bus").exists() || d.join("systemd").join("private").exists();
    let own = PathBuf::from(format!("/run/user/{}", shared::sys::euid()));
    let user_rt = (!env("XDG_RUNTIME_DIR").is_some_and(|d| has_bus(Path::new(&d)))
        && has_bus(&own))
    .then_some(own);
    let mut stop = std::process::Command::new(SYSTEMCTL);
    stop.args(["--user", "stop", sandbox::UNIT])
        .stderr(std::process::Stdio::null());
    let mut cmd = std::process::Command::new(SYSTEMD_RUN);
    cmd.args(&argv[1..]).stdout(std::process::Stdio::null());
    if let Some(rt) = &user_rt {
        stop.env("XDG_RUNTIME_DIR", rt);
        cmd.env("XDG_RUNTIME_DIR", rt);
    }
    let _ = stop.status();
    let _ = std::fs::remove_file(&socket);
    let fail = |e: LaunchError| {
        let _ = std::fs::remove_file(&tfile);
        e
    };
    let st = cmd.status().map_err(|e| {
        fail(format!("systemd-run: {e} (the helper does not run unconfined)").into())
    })?;
    if !st.success() {
        return Err(fail(
            format!("systemd-run failed ({st}); the helper does not run unconfined").into(),
        ));
    }
    for _ in 0..100 {
        if let Ok(why) = std::fs::read_to_string(&refused) {
            let _ = std::fs::remove_file(&refused);
            return Err(fail(LaunchError::Refused(why.trim().to_string())));
        }
        if socket.exists() {
            println!("token {token}");
            return Ok(());
        }
        std::thread::sleep(SOCKET_POLL);
    }
    Err(fail(
        "the helper did not open its socket in 5 seconds".into(),
    ))
}

/// The same unit fences in a unit of its own (`atlas-torrent-check`) that only runs the
/// confinement check from inside and ends; its output. `launch` runs it before every start, and
/// `launch --check-only` prints it (the e2e asserts the effective confinement with it, DL-SEC-01).
/// A unit the service manager cannot set up (exit status 200-245: namespaces, capabilities,
/// seccomp) is a refusal too.
fn check_in_unit(
    o: &Opts,
    exe: &Path,
    downloads: &Path,
    state: &Path,
    run: &Path,
    socket: &Path,
) -> Result<String, LaunchError> {
    let mut serve = vec![
        "--downloads".to_string(),
        downloads.display().to_string(),
        "--state".into(),
        state.display().to_string(),
        "--socket".into(),
        socket.display().to_string(),
        "--check-only".into(),
    ];
    if o.no_dht {
        serve.push("--no-dht".into());
    }
    let mut argv = sandbox::argv(exe, downloads, state, run, &serve);
    for a in argv.iter_mut() {
        if a == &format!("--unit={}", sandbox::UNIT) {
            *a = format!("--unit={}-check", sandbox::UNIT);
        }
    }
    argv.insert(1, "--wait".into());
    argv.insert(2, "--pipe".into());
    let refused = run.join(confined::REFUSED_FILE);
    let _ = std::fs::remove_file(&refused);
    let out = std::process::Command::new(SYSTEMD_RUN)
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("systemd-run: {e}"))?;
    if let Ok(why) = std::fs::read_to_string(&refused) {
        let _ = std::fs::remove_file(&refused);
        return Err(LaunchError::Refused(why.trim().to_string()));
    }
    match out.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Some(n) if (200..=245).contains(&n) => Err(LaunchError::Refused(format!(
            "{} (the service manager could not set up its fences: status {n})",
            confined::REFUSED_TEXT
        ))),
        _ => Err(format!(
            "systemd-run failed ({}): {}; the helper does not run unconfined",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )
        .into()),
    }
}

fn new_token() -> Result<String, String> {
    let mut b = [0u8; 32];
    let mut f = std::fs::File::open("/dev/urandom").map_err(|e| e.to_string())?;
    std::io::Read::read_exact(&mut f, &mut b).map_err(|e| e.to_string())?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .max_blocking_threads(8)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}

fn serve(o: &Opts) -> Result<(), String> {
    let downloads = need(&o.downloads, "--downloads")?;
    let state = need(&o.state, "--state")?;
    let socket = o
        .socket
        .clone()
        .or_else(|| server::default_socket(&env))
        .ok_or("no XDG_RUNTIME_DIR")?;
    let mut exes: Vec<PathBuf> = o
        .peer_exe
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .collect();
    if exes.is_empty() {
        exes.push(PathBuf::from(shared::atlas::BROWSER));
    }
    // DL-SEC-01: look at this process's own view before anything from a stranger is parsed, and
    // refuse to serve unless every fence of the unit is there.
    if !o.unconfined {
        let home = env("HOME")
            .map(PathBuf::from)
            .filter(|h| h.is_absolute())
            .ok_or("no HOME")?;
        let gaps = confined::gaps(&confined::look(&home, &[&downloads, &state]));
        if !gaps.is_empty() {
            let line = confined::refusal_line(&gaps);
            if let Some(run) = socket.parent() {
                let _ = std::fs::write(run.join(confined::REFUSED_FILE), format!("{line}\n"));
            }
            if let Some(t) = &o.token_file {
                let _ = std::fs::remove_file(t);
            }
            return Err(Refusal(line).into());
        }
    }
    if o.check_only {
        println!("confined");
        return Ok(());
    }
    // DL-SEC-06: the token comes in a file read once and removed (None without one: only the
    // browser program, seen by /proc/<pid>/exe, may connect).
    let token = o.token_file.as_deref().and_then(server::take_token_file);
    let rt = runtime()?;
    rt.block_on(async move {
        let eng = engine::Engine::start(engine::Config {
            downloads,
            state,
            sharing: o.sharing,
            dht: !o.no_dht,
            extra_trackers: o.trackers.clone(),
            listen_port: None,
            upload_bps: None,
            local_peers: o.local_peers,
        })
        .await
        .map_err(|e| format!("engine: {e}"))?;
        let l = server::bind(&socket).map_err(|e| format!("socket {}: {e}", socket.display()))?;
        server::run(
            l,
            eng,
            server::Peers { exes, token },
            o.idle.map(Duration::from_secs),
        )
        .await;
        Ok(())
    })
}

/// A refused confinement (exit status [`confined::REFUSED_EXIT`]).
struct Refusal(String);

impl From<Refusal> for String {
    fn from(r: Refusal) -> String {
        format!("{REFUSAL_MARK}{}", r.0)
    }
}

/// How `main` tells a refusal apart in the error text.
const REFUSAL_MARK: &str = "refused: ";

#[cfg(feature = "tools")]
fn make(o: &Opts) -> Result<(), String> {
    let file = PathBuf::from(o.rest.get(1).ok_or("make FILE")?);
    let out = o.out.clone().ok_or("--out is needed")?;
    let rt = runtime()?;
    let bytes = rt.block_on(async {
        let r = librqbit::create_torrent(
            &file,
            librqbit::CreateTorrentOptions {
                name: None,
                trackers: o.trackers.clone(),
                piece_length: Some(256 * 1024),
            },
            &librqbit::spawn_utils::BlockingSpawner::new(2),
        )
        .await
        .map_err(|e| format!("{e:#}"))?;
        println!("{}", r.info_hash().as_string());
        r.as_bytes().map_err(|e| format!("{e:#}"))
    })?;
    std::fs::write(&out, &bytes).map_err(|e| e.to_string())
}

#[cfg(feature = "tools")]
fn seed(o: &Opts) -> Result<(), String> {
    let t = PathBuf::from(o.rest.get(1).ok_or("seed FILE.torrent")?);
    let dir = need(&o.dir, "--dir")?;
    let state = need(&o.state, "--state")?;
    let bytes = std::fs::read(&t).map_err(|e| e.to_string())?;
    let rt = runtime()?;
    rt.block_on(async move {
        let eng = engine::Engine::start(engine::Config {
            downloads: dir.clone(),
            state,
            sharing: true,
            dht: false,
            extra_trackers: Vec::new(),
            listen_port: Some(o.port.unwrap_or(51413)),
            upload_bps: o.rate.and_then(std::num::NonZeroU32::new),
            local_peers: true,
        })
        .await
        .map_err(|e| format!("engine: {e}"))?;
        let id = eng.seed_existing(bytes, &dir).await?;
        println!("seeding {id}");
        loop {
            tokio::time::sleep(SEED_REPORT).await;
            for j in eng.jobs().await {
                println!("{:?} {}/{} peers {}", j.state, j.have, j.total, j.peers);
            }
        }
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = match parse(&args) {
        // the e2e's browser starts `launch` itself: in a tools build the environment may ask for
        // the loopback swarm too
        #[cfg(feature = "tools")]
        Ok(mut o) => {
            if env("ATLAS_TORRENT_LOCAL_PEERS").as_deref() == Some("1") {
                o.local_peers = true;
            }
            o
        }
        #[cfg(not(feature = "tools"))]
        Ok(o) => o,
        Err(e) => {
            eprintln!("atlas-torrent: {e}");
            return ExitCode::from(2);
        }
    };
    let r = match o.rest.first().map(String::as_str) {
        Some("launch") => match launch(&o) {
            Ok(()) => Ok(()),
            Err(LaunchError::Refused(why)) => Err(format!("{REFUSAL_MARK}{why}")),
            Err(LaunchError::Other(e)) => Err(e),
        },
        Some("serve") => serve(&o),
        #[cfg(feature = "tools")]
        Some("make") => make(&o),
        #[cfg(feature = "tools")]
        Some("seed") => seed(&o),
        _ => Err("usage: atlas-torrent launch|serve (see the source header)".into()),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("atlas-torrent: {e}");
            if e.starts_with(REFUSAL_MARK) {
                ExitCode::from(confined::REFUSED_EXIT)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}
