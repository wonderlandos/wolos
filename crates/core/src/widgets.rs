//! Widget packages and their data sources (archicture/modules/widgets.md). The shell and
//! `wol widgets` both load packages and run sources through here, so they behave the same.
//! Sources run as the user, off the UI thread: command (timeout, CPU limit, nice, own process group,
//! output cap), file (package or home), http (only with a grant for its host; `fetch`/`curl`).
use serde_json::Value;
use shared::widgets::manifest::{Manifest, Problem, Source};
use shared::widgets::{Desktop, MANIFEST_FILE, SYSTEM_PACKAGES_DIR, USER_PACKAGES_DIR};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Largest source output (stdout, file, body) read.
pub const MAX_OUTPUT: usize = 256 * 1024;
/// Network timeout for http sources and the weather service.
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct Package {
    pub id: String,
    pub dir: PathBuf,
    /// Shipped with the system (read-only, trusted).
    pub system: bool,
    pub manifest: Result<Manifest, Vec<Problem>>,
}

impl Package {
    pub fn name(&self) -> &str {
        self.manifest.as_ref().map_or(&self.id, |m| &m.name)
    }
}

/// `$XDG_DATA_HOME`, else `~/.local/share`.
pub fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"))
}

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// Package dirs, user first (a user package overrides a system one with the same id).
pub fn package_dirs(data_home: &Path) -> Vec<(PathBuf, bool)> {
    vec![
        (data_home.join(USER_PACKAGES_DIR), false),
        (PathBuf::from(SYSTEM_PACKAGES_DIR), true),
    ]
}

/// One package folder; the folder name is the id.
pub fn load_package(dir: &Path, system: bool) -> Package {
    let id = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let manifest = match std::fs::read_to_string(dir.join(MANIFEST_FILE)) {
        Ok(json) => Manifest::parse(&id, &json),
        Err(e) => Err(vec![Problem {
            path: MANIFEST_FILE.into(),
            message: format!("cannot read: {e}"),
        }]),
    };
    Package {
        id,
        dir: dir.to_path_buf(),
        system,
        manifest,
    }
}

/// Every package folder (with a widget.json) in `dirs`, sorted by name; first dir wins on equal ids.
pub fn load_packages(dirs: &[(PathBuf, bool)]) -> Vec<Package> {
    let mut out: Vec<Package> = Vec::new();
    for (d, system) in dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        let mut found: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join(MANIFEST_FILE).is_file())
            .collect();
        found.sort();
        for p in found {
            let pkg = load_package(&p, *system);
            if !out.iter().any(|q| q.id == pkg.id) {
                out.push(pkg);
            }
        }
    }
    out.sort_by_key(|a| a.name().to_lowercase());
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceError {
    NotFound(String),
    /// Seconds allowed.
    Timeout(u32),
    Failed {
        code: Option<i32>,
        stderr: String,
    },
    TooLarge,
    BadJson(String),
    /// The user has not allowed this host for this widget.
    NeedsConsent(String),
    Offline(String),
}

impl SourceError {
    /// One sentence: what happened and what happens next (HIG §3 errors).
    pub fn message(&self) -> String {
        match self {
            SourceError::NotFound(p) => format!("Could not find {p}. Reinstall the widget."),
            SourceError::Timeout(t) => {
                format!("The data took longer than {t} s. It will try again later.")
            }
            SourceError::Failed { code, stderr } => {
                let why = stderr.lines().next().unwrap_or("").trim();
                let code = code.map_or("stopped".to_string(), |c| format!("exit {c}"));
                if why.is_empty() {
                    format!("The data source failed ({code}).")
                } else {
                    format!("The data source failed ({code}): {why}")
                }
            }
            SourceError::TooLarge => format!(
                "The data is larger than {} KB. The widget needs a smaller source.",
                MAX_OUTPUT / 1024
            ),
            SourceError::BadJson(e) => format!("The data is not valid JSON ({e})."),
            SourceError::NeedsConsent(h) => format!("Allow this widget to read data from {h}."),
            SourceError::Offline(h) => {
                format!("Could not reach {h}. It will try again later.")
            }
        }
    }
}

pub fn parse_json(bytes: &[u8]) -> Result<Value, SourceError> {
    serde_json::from_slice(bytes)
        .map_err(|e| SourceError::BadJson(format!("line {}, column {}", e.line(), e.column())))
}

/// Runs a package's source once (blocking: call off the UI thread).
pub fn read_source(
    src: &Source,
    pkg: &Path,
    home: &Path,
    allowed: impl Fn(&str) -> bool,
) -> Result<Value, SourceError> {
    let bytes = match src {
        Source::Command { argv, timeout } => {
            let mut argv = argv.clone();
            if let Some(rel) = argv[0].strip_prefix("./") {
                let p = pkg.join(rel);
                if !p.is_file() {
                    return Err(SourceError::NotFound(argv[0].clone()));
                }
                argv[0] = p.to_string_lossy().into_owned();
            }
            run_limited(
                &argv,
                pkg,
                Duration::from_secs(u64::from(*timeout)),
                MAX_OUTPUT,
            )?
        }
        Source::File { path } => {
            let p = match path.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => pkg.join(path),
            };
            read_capped(&p)?
        }
        Source::Http { url } => {
            let host = src.host().unwrap_or_default();
            if !allowed(&host) {
                return Err(SourceError::NeedsConsent(host));
            }
            http_get(url, HTTP_TIMEOUT, MAX_OUTPUT)?
        }
    };
    parse_json(&bytes)
}

fn read_capped(p: &Path) -> Result<Vec<u8>, SourceError> {
    let f = std::fs::File::open(p)
        .map_err(|_| SourceError::NotFound(p.to_string_lossy().into_owned()))?;
    let mut buf = Vec::new();
    f.take(MAX_OUTPUT as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|_| SourceError::NotFound(p.to_string_lossy().into_owned()))?;
    if buf.len() > MAX_OUTPUT {
        return Err(SourceError::TooLarge);
    }
    Ok(buf)
}

/// GET with the system's own client (FreeBSD `fetch(1)`, else `curl`): no TLS stack in WolOS, no
/// cookies, no API keys. Fails on HTTP errors.
pub fn http_get(url: &str, timeout: Duration, max: usize) -> Result<Vec<u8>, SourceError> {
    let host = shared::widgets::manifest::url_host(url).unwrap_or_default();
    let secs = timeout.as_secs().max(1).to_string();
    let argv: Vec<String> = if cfg!(target_os = "freebsd") {
        ["fetch", "-q", "-T", &secs, "-o", "-", url]
            .map(String::from)
            .to_vec()
    } else {
        let max = max.to_string();
        [
            "curl",
            "-fsSL",
            "--proto",
            "=https,http",
            "--max-time",
            &secs,
            "--max-filesize",
            &max,
            url,
        ]
        .map(String::from)
        .to_vec()
    };
    match run_limited(&argv, Path::new("/"), timeout + Duration::from_secs(2), max) {
        Ok(b) => Ok(b),
        Err(SourceError::TooLarge) => Err(SourceError::TooLarge),
        Err(_) => Err(SourceError::Offline(host)),
    }
}

/// argv as the user: stdin closed, own process group (killed whole on timeout), nice 10,
/// RLIMIT_CPU = timeout, stdout capped at `max` bytes. Never a shell.
pub fn run_limited(
    argv: &[String],
    cwd: &Path,
    timeout: Duration,
    max: usize,
) -> Result<Vec<u8>, SourceError> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let (prog, args) = argv
        .split_first()
        .ok_or_else(|| SourceError::NotFound(String::new()))?;
    let cpu = timeout.as_secs().max(1);
    let mut cmd = Command::new(prog);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    // SAFETY: only async-signal-safe calls (setpriority, setrlimit) between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            libc::setpriority(libc::PRIO_PROCESS, 0, 10);
            let lim = libc::rlimit {
                rlim_cur: cpu as libc::rlim_t,
                rlim_max: cpu as libc::rlim_t,
            };
            libc::setrlimit(libc::RLIMIT_CPU, &lim);
            Ok(())
        });
    }
    let mut child = cmd
        .spawn()
        .map_err(|_| SourceError::NotFound(prog.clone()))?;
    let pid = child.id() as libc::pid_t;
    let kill_group = || {
        // SAFETY: signals our own child's process group.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    };
    let mut out = child.stdout.take().expect("piped stdout");
    let mut err = child.stderr.take().expect("piped stderr");
    let reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = (&mut out).take(max as u64 + 1).read_to_end(&mut b);
        b
    });
    let err_reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = (&mut err).take(4096).read_to_end(&mut b);
        b
    });
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                kill_group();
                let _ = child.wait();
                break None;
            }
        }
    };
    // Children left in the group must not keep the pipes (and the readers) open.
    kill_group();
    let bytes = reader.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();
    if bytes.len() > max {
        return Err(SourceError::TooLarge);
    }
    let Some(status) = status else {
        return Err(SourceError::Timeout(timeout.as_secs() as u32));
    };
    if !status.success() {
        return Err(SourceError::Failed {
            code: status.code(),
            stderr,
        });
    }
    Ok(bytes)
}

// ---- The layout file (`widgets.json`), shared by the shell and `wol widgets` ----

/// `$XDG_CONFIG_HOME/wolos/widgets.json`.
pub fn desktop_path(config_home: &Path) -> PathBuf {
    config_home.join(shared::widgets::DESKTOP_FILE)
}

/// The saved layout. A missing file is an empty desktop; an unreadable one is reported and left
/// on disk untouched until the next save moves it aside (`<file>.bad`).
pub fn load_desktop(path: &Path) -> (Desktop, Option<String>) {
    match std::fs::read_to_string(path) {
        Err(_) => (Desktop::default(), None),
        Ok(s) => match Desktop::parse(&s) {
            Ok(d) => (d, None),
            Err(e) => (Desktop::default(), Some(e)),
        },
    }
}

/// Atomic write (temp file + rename). A file that did not parse is kept as `<file>.bad` first.
pub fn save_desktop(path: &Path, d: &Desktop) -> std::io::Result<()> {
    if let Ok(old) = std::fs::read_to_string(path)
        && Desktop::parse(&old).is_err()
    {
        let _ = std::fs::rename(path, path.with_extension("json.bad"));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, d.to_json())?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wol-widgets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn script(dir: &Path, name: &str, body: &str) {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn cmd(argv: &[&str], timeout: u32) -> Source {
        Source::Command {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            timeout,
        }
    }

    #[test]
    fn command_sources_return_json_or_a_reason() {
        let d = tmp("cmd");
        script(&d, "ok.sh", r#"echo '{"online": ['"$1"']}'"#);
        script(&d, "slow.sh", "sleep 5");
        script(&d, "big.sh", "head -c 400000 /dev/zero | tr '\\0' 'a'");
        script(&d, "fail.sh", "echo 'no token' >&2; exit 3");
        script(&d, "bad.sh", "echo '{nope'");
        script(&d, "pwd.sh", r#"printf '{"dir": "%s"}' "$(pwd)""#);
        let home = Path::new("/nonexistent");
        let run = |s: &Source| read_source(s, &d, home, |_| false);
        assert_eq!(
            run(&cmd(&["./ok.sh", "7"], 5)).unwrap(),
            serde_json::json!({"online": [7]})
        );
        assert_eq!(
            run(&cmd(&["./pwd.sh"], 5)).unwrap()["dir"],
            d.to_string_lossy().as_ref()
        );
        let t0 = std::time::Instant::now();
        assert_eq!(run(&cmd(&["./slow.sh"], 1)), Err(SourceError::Timeout(1)));
        assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());
        assert_eq!(run(&cmd(&["./big.sh"], 5)), Err(SourceError::TooLarge));
        let e = run(&cmd(&["./fail.sh"], 5)).unwrap_err();
        assert_eq!(
            e,
            SourceError::Failed {
                code: Some(3),
                stderr: "no token\n".into()
            }
        );
        assert_eq!(e.message(), "The data source failed (exit 3): no token");
        assert_eq!(
            run(&cmd(&["./bad.sh"], 5)),
            Err(SourceError::BadJson("line 1, column 2".into()))
        );
        assert_eq!(
            run(&cmd(&["./missing.sh"], 5)),
            Err(SourceError::NotFound("./missing.sh".into()))
        );
        assert_eq!(
            run(&cmd(&["wol-no-such-program"], 5)),
            Err(SourceError::NotFound("wol-no-such-program".into()))
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn file_sources_read_the_package_or_home() {
        let d = tmp("file");
        let home = tmp("home");
        std::fs::write(d.join("data.json"), r#"{"a": 1}"#).unwrap();
        std::fs::create_dir_all(home.join(".cache")).unwrap();
        std::fs::write(home.join(".cache/x.json"), "[1, 2]").unwrap();
        let f = |p: &str| Source::File { path: p.into() };
        assert_eq!(
            read_source(&f("data.json"), &d, &home, |_| false).unwrap()["a"],
            1
        );
        assert_eq!(
            read_source(&f("~/.cache/x.json"), &d, &home, |_| false).unwrap()[1],
            2
        );
        assert!(matches!(
            read_source(&f("nope.json"), &d, &home, |_| false),
            Err(SourceError::NotFound(_))
        ));
        std::fs::remove_dir_all(d).unwrap();
        std::fs::remove_dir_all(home).unwrap();
    }

    /// Fail closed: no request leaves the machine without a grant for that exact host.
    #[test]
    fn http_sources_need_a_grant_for_their_host() {
        let src = Source::Http {
            url: "https://api.example.com/friends".into(),
        };
        let asked = std::cell::RefCell::new(Vec::new());
        let r = read_source(&src, Path::new("/"), Path::new("/"), |h| {
            asked.borrow_mut().push(h.to_string());
            false
        });
        assert_eq!(r, Err(SourceError::NeedsConsent("api.example.com".into())));
        assert_eq!(*asked.borrow(), ["api.example.com"]);
        assert_eq!(
            r.unwrap_err().message(),
            "Allow this widget to read data from api.example.com."
        );
    }

    #[test]
    fn user_packages_override_system_ones_and_bad_ones_report_why() {
        let user = tmp("user");
        let sys = tmp("sys");
        let w = |dir: &Path, id: &str, json: &str| {
            std::fs::create_dir_all(dir.join(id)).unwrap();
            std::fs::write(dir.join(id).join(MANIFEST_FILE), json).unwrap();
        };
        let ok = |name: &str| format!(r#"{{"name": "{name}", "layout": {{"type": "spacer"}}}}"#);
        w(&sys, "clock-face", &ok("Zebra"));
        w(&sys, "friends", &ok("System friends"));
        w(&user, "friends", &ok("My friends"));
        w(&user, "broken", "{");
        std::fs::create_dir_all(user.join("no-manifest")).unwrap();
        let pkgs = load_packages(&[(user.clone(), false), (sys.clone(), true)]);
        let names: Vec<(&str, bool)> = pkgs.iter().map(|p| (p.name(), p.system)).collect();
        assert_eq!(
            names,
            [("broken", false), ("My friends", false), ("Zebra", true)]
        );
        assert!(pkgs[0].manifest.is_err());
        let dirs = package_dirs(Path::new("/home/a/.local/share"));
        assert_eq!(dirs[0].0, Path::new("/home/a/.local/share/wolos/widgets"));
        assert_eq!(dirs[1], (PathBuf::from(SYSTEM_PACKAGES_DIR), true));
        std::fs::remove_dir_all(user).unwrap();
        std::fs::remove_dir_all(sys).unwrap();
    }

    #[test]
    fn store_round_trips_and_keeps_a_bad_file_aside() {
        let dir = std::env::temp_dir().join(format!("wol-widgets-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = desktop_path(&dir);
        use shared::widgets::{Builtin, Desktop, Kind, Options, Placed, Size};
        assert_eq!(
            load_desktop(&p),
            (Desktop::default(), None),
            "missing = empty"
        );
        let mut d = Desktop::default();
        d.widgets.push(Placed {
            id: "clock".into(),
            kind: Kind::Builtin(Builtin::Clock),
            size: Size::Small,
            col: 1,
            row: 0,
            options: Options::default(),
        });
        save_desktop(&p, &d).unwrap();
        assert_eq!(load_desktop(&p).0, d);
        std::fs::write(&p, "{ not json").unwrap();
        let (empty, err) = load_desktop(&p);
        assert_eq!(empty, Desktop::default());
        assert!(err.unwrap().starts_with("widgets.json:"));
        assert!(
            !p.with_extension("json.bad").exists(),
            "loading never moves files"
        );
        save_desktop(&p, &d).unwrap();
        assert_eq!(
            std::fs::read_to_string(p.with_extension("json.bad")).unwrap(),
            "{ not json"
        );
        assert_eq!(load_desktop(&p).0, d);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
