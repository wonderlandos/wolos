//! `wol` actions. All I/O goes through [`Env`], so every action is testable with [`FakeEnv`].
use crate::cli::{Invocation, help};
use shared::admin::{self as wire, AdminAction, AdminReply, AdminRequest};
use shared::errors::ErrorCode;
use shared::jobs::{JobRequest, JobStatus, Reply};
use std::io;
use std::path::{Path, PathBuf};
use wol_core::theme::{ThemeCatalog, active, config_home, search_paths};

/// Shell control socket under `$XDG_RUNTIME_DIR`. Wire: one line per connection,
/// `toggle <panel>` | `show <panel>` | `hide <panel>` | `reload`, no reply.
pub const SHELL_SOCKET: &str = "wol-shell.sock";
/// Compat launcher (proxy crate).
pub const RUN_BIN: &str = "wol-run";
/// Owner of GPU selection (realtime crate): `wol-realtime gpu env|hyprconf`.
pub const GPU_BIN: &str = "wol-realtime";
const PASSWORD_TRIES: usize = 3;

/// Side effects `wol` needs.
/// Server latency probe in ms, shareable across threads (`wol mirror rank`).
pub type Prober = Box<dyn Fn(&str) -> Option<u64> + Sync + Send>;

pub trait Env {
    fn out(&mut self, s: &str);
    fn err(&mut self, s: &str);
    fn stdout_tty(&self) -> bool;
    fn var(&self, name: &str) -> Option<String>;
    fn is_root(&self) -> bool;
    /// Connect to a Unix socket, write `req`, half-close, read the reply to EOF.
    fn request(&mut self, socket: &Path, req: &[u8]) -> io::Result<String>;
    /// [`Env::request`] waiting up to `secs` for the reply (helper approvals wait for a person).
    fn request_wait(&mut self, socket: &Path, req: &[u8], secs: u32) -> io::Result<String> {
        let _ = secs;
        self.request(socket, req)
    }
    /// Run argv with inherited stdio; exit status.
    fn exec(&mut self, argv: &[String]) -> io::Result<i32>;
    /// Run argv, capture stdout (None on failure).
    fn capture(&mut self, argv: &[&str]) -> Option<String>;
    fn stdin_line(&mut self) -> Option<String>;
    fn read_file(&self, p: &Path) -> Option<String>;
    fn sys_topic(&mut self, topic: &str) -> Option<serde_json::Value>;
    fn now_secs(&self) -> u64;
    fn config_home(&self) -> PathBuf;
    /// Like `exec`, but the child gets no stdin and its stdout goes to our stderr (`wol mcp`:
    /// stdout carries the protocol).
    fn exec_quiet(&mut self, argv: &[String]) -> io::Result<i32> {
        self.exec(argv)
    }
    /// One raw line from stdin; None at end of input (`wol mcp`).
    fn read_line(&mut self) -> Option<String> {
        let mut l = String::new();
        match io::stdin().read_line(&mut l) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(l),
        }
    }
    /// Server latency probe in ms (`wol mirror rank`); shareable across threads.
    fn prober(&self) -> Prober {
        Box::new(crate::mirror::curl_probe)
    }
    /// Hardware detected now with driver/firmware status (`wol hw`).
    fn hardware(&mut self) -> shared::hardware::HardwareSnapshot {
        wol_core::hardware::read(&wol_core::system::PosixHost)
    }
    /// What Steam and Windows games need, read now (`wol games doctor`).
    fn games_facts(&self) -> wol_core::games::doctor::Facts {
        use wol_core::games::doctor;
        let var = |n: &str| self.var(n);
        let futex = doctor::probe_futex_waitv(&doctor::probe_dir(&var));
        doctor::gather(&wol_core::system::PosixHost, &self.home(), &var, futex)
    }
    /// Full paths of a directory's entries, sorted; empty if missing (`wol power suspend`).
    fn list_dir(&self, p: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(p)
            .map(|d| {
                d.flatten()
                    .map(|e| e.path().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
    /// Replace a whole file: temp file + rename (`wol hw boot`).
    fn write_file(&mut self, path: &Path, text: &str) -> io::Result<()> {
        let tmp = path.with_extension("wol-tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }
    /// Desktop controls (Wi-Fi, audio, brightness, Bluetooth, displays) for this host.
    fn controls(&self) -> wol_core::controls::Controls {
        wol_core::controls::Controls::new(std::sync::Arc::new(wol_core::system::PosixHost))
    }
    /// `$XDG_STATE_HOME`: settings journal, helper audit log.
    fn state_home(&self) -> PathBuf {
        wol_core::journal::state_home()
    }
    fn home(&self) -> PathBuf {
        PathBuf::from(self.var("HOME").unwrap_or_default())
    }
    /// Who acts (journal, audit log): `cli`, `script:<WOL_HELPER>`, `mcp:<client>`.
    fn actor(&self) -> String {
        self.var("WOL_HELPER")
            .map_or_else(|| "cli".into(), |h| format!("script:{h}"))
    }
}

pub(crate) fn fail(env: &mut dyn Env, code: ErrorCode, msg: &str) -> i32 {
    env.err(&format!("wol: {msg}\n"));
    code.exit()
}

pub(crate) fn emit_json(env: &mut dyn Env, v: &serde_json::Value) {
    env.out(&format!(
        "{}\n",
        serde_json::to_string_pretty(v).unwrap_or_default()
    ));
}

/// Human text for a JSON value: `key: value` lines, arrays one item per line.
pub fn plain(v: &serde_json::Value) -> String {
    use serde_json::Value::*;
    match v {
        Object(m) => m
            .iter()
            .map(|(k, x)| format!("{k}: {}\n", inline(x)))
            .collect(),
        Array(a) => a.iter().map(|x| format!("{}\n", inline(x))).collect(),
        x => format!("{}\n", inline(x)),
    }
}

fn inline(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "-".into(),
        serde_json::Value::Array(a) => a.iter().map(inline).collect::<Vec<_>>().join(", "),
        x => x.to_string(),
    }
}

pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    // A named script is a helper: policy + audit log (archicture/modules/automation.md).
    if let (Some(name), Some(rule)) = (env.var("WOL_HELPER"), crate::catalog::rule_of(inv)) {
        return crate::verbs::as_script(inv, env, &name, rule);
    }
    dispatch(inv, env)
}

/// Runs one parsed invocation (no helper gate; `wol mcp` gates before calling this).
pub(crate) fn dispatch(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let json = inv.flag("json");
    match inv.action {
        "help" => {
            let path: Vec<String> = if inv.path.is_empty() {
                vec![]
            } else {
                inv.all("command").iter().map(|s| s.to_string()).collect()
            };
            env.out(&help(&path));
            0
        }
        "about" => topic(env, "about", json),
        "sys" => match inv.get("topic") {
            None => {
                env.out(&format!("{}\n", wol_core::system::TOPICS.join("\n")));
                0
            }
            Some(t) => topic(env, if t == "temps" { "temperatures" } else { t }, json),
        },
        "jobs.list" => jobs_list(env, json),
        "jobs.start" | "jobs.stop" => {
            let label = inv.get("label").unwrap_or_default().to_string();
            let req = if inv.action == "jobs.start" {
                JobRequest::Start(label)
            } else {
                JobRequest::Stop(label)
            };
            jobs_change(env, &req)
        }
        "admin" => admin(env, inv),
        "bundle.run" => {
            let mut argv = vec![
                RUN_BIN.to_string(),
                inv.get("bundle").unwrap_or_default().into(),
            ];
            argv.extend(inv.all("args").iter().map(|s| s.to_string()));
            exec(env, &argv)
        }
        "bundle.plan" => bundle_plan(env, inv.get("bundle").unwrap_or_default(), json),
        "theme.current" | "theme.list" | "theme.set" | "theme.switch" | "theme.css" => {
            theme(env, inv, json)
        }
        "shell.toggle" | "shell.show" | "shell.hide" | "shell.reload" => shell(env, inv),
        "update.check" => update_check(env, json),
        "update.status" => update_status(env, json),
        "base.verify" => base_verify(env, json),
        "update.plan" | "update.apply" => update(env, inv),
        "update.rollback" => crate::verbs::rollback(env, inv.get("be")),
        "mirror.status" | "mirror.rank" | "mirror.gate" | "mirror.kernel" => {
            crate::mirror::run(env, inv.action, inv.flag("yes"), json)
        }
        "gpu.env" => exec(env, &[GPU_BIN.into(), "gpu".into(), "env".into()]),
        "gpu.hyprconf" => exec(env, &[GPU_BIN.into(), "gpu".into(), "hyprconf".into()]),
        "hw.list" | "hw.status" | "hw.fix" | "hw.boot" | "hw.online" => {
            crate::hw::execute(inv, env)
        }
        a if a.starts_with("games.") => crate::games::execute(inv, env),
        a if a.starts_with("power.") => crate::power::execute(inv, env),
        a if a.starts_with("firewall.") || a == "security.status" => {
            crate::security::execute(inv, env)
        }
        "package.install" | "package.remove" | "package.list" | "package.search"
        | "package.updates" => crate::packages::execute(inv, env),
        "widgets.list" | "widgets.add" | "widgets.remove" | "widgets.reload"
        | "widgets.validate" => crate::widgets::execute(inv, env),
        other => crate::verbs::execute(inv, env)
            .unwrap_or_else(|| fail(env, ErrorCode::Failed, &format!("no handler for {other}"))),
    }
}

pub(crate) fn exec(env: &mut dyn Env, argv: &[String]) -> i32 {
    match env.exec(argv) {
        Ok(c) => c,
        Err(e) => fail(env, ErrorCode::Unavailable, &format!("{}: {e}", argv[0])),
    }
}

fn topic(env: &mut dyn Env, t: &str, json: bool) -> i32 {
    let Some(v) = env.sys_topic(t) else {
        let topics = wol_core::system::TOPICS;
        let hint = crate::cli::suggest(t, &topics)
            .map_or(String::new(), |s| format!("; did you mean '{s}'?"));
        return fail(env, ErrorCode::Usage, &format!("unknown topic '{t}'{hint}"));
    };
    // JSON when asked or when stdout is not a TTY (scripts, the shell).
    if json || !env.stdout_tty() {
        emit_json(env, &v);
    } else {
        env.out(&plain(&v));
    }
    0
}

fn jobs_socket(env: &dyn Env) -> PathBuf {
    env.var("WOL_JOBS_SOCKET")
        .map_or_else(|| shared::jobs::SOCKET.into(), PathBuf::from)
}

fn jobs_list(env: &mut dyn Env, json: bool) -> i32 {
    let sock = jobs_socket(env);
    let reply = match env.request(&sock, b"list\n") {
        Ok(r) => r,
        Err(e) => {
            return fail(
                env,
                ErrorCode::Unavailable,
                &format!("wold not reachable ({e})"),
            );
        }
    };
    let Ok(jobs) = serde_json::from_str::<Vec<JobStatus>>(reply.trim()) else {
        return fail(env, ErrorCode::Failed, "bad reply from wold");
    };
    if json || !env.stdout_tty() {
        env.out(&format!(
            "{}\n",
            serde_json::to_string(&jobs).unwrap_or_default()
        ));
    } else {
        let mut s = format!(
            "{:<32} {:<8} {:>7} {:>6} {:>5}\n",
            "LABEL", "STATE", "PID", "STARTS", "EXIT"
        );
        for j in &jobs {
            let o = |x: Option<i32>| x.map_or("-".into(), |v| v.to_string());
            s.push_str(&format!(
                "{:<32} {:<8} {:>7} {:>6} {:>5}\n",
                j.label,
                j.state,
                o(j.pid),
                j.starts,
                o(j.last_exit)
            ));
        }
        env.out(&s);
    }
    0
}

fn jobs_change(env: &mut dyn Env, req: &JobRequest) -> i32 {
    let sock = jobs_socket(env);
    let line = format!("{}\n", req.line());
    match env.request(&sock, line.as_bytes()) {
        Err(e) => fail(
            env,
            ErrorCode::Unavailable,
            &format!("wold not reachable ({e})"),
        ),
        Ok(r) => match serde_json::from_str::<Reply>(r.trim()) {
            Ok(Reply { ok: true, .. }) => 0,
            Ok(Reply { error, .. }) => {
                let msg = error.unwrap_or_else(|| "failed".into());
                let code = if msg.contains("root") || msg.contains("denied") {
                    ErrorCode::Denied
                } else {
                    ErrorCode::Failed
                };
                fail(env, code, &msg)
            }
            Err(_) => fail(env, ErrorCode::Failed, "bad reply from wold"),
        },
    }
}

/// Exit status for an admin reply (IaraOS `iara admin`).
pub fn admin_exit(r: AdminReply) -> i32 {
    match r {
        AdminReply::Ok => 0,
        AdminReply::Invalid => ErrorCode::Usage.exit(),
        AdminReply::Denied | AdminReply::NotAdmin | AdminReply::NeedsAuth => {
            ErrorCode::Denied.exit()
        }
        AdminReply::Failed => 1,
    }
}

fn admin(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let name = inv.get("action").unwrap_or_default();
    let Some(action) = AdminAction::parse(name) else {
        let names: Vec<&str> = AdminAction::ALL.iter().map(|a| a.name()).collect();
        return fail(
            env,
            ErrorCode::Usage,
            &format!("unknown admin action '{name}' ({})", names.join("|")),
        );
    };
    let req = AdminRequest {
        action,
        args: inv.all("args").iter().map(|s| s.to_string()).collect(),
    };
    if let Some(e) = wol_core::admin::validate(&req) {
        return fail(env, ErrorCode::Usage, &e);
    }
    admin_send(env, &req)
}

/// One wol-admind request; asks for the password on stdin while it answers `auth`/`denied`.
pub(crate) fn admin_send(env: &mut dyn Env, req: &AdminRequest) -> i32 {
    let sock = env
        .var("WOL_ADMIN_SOCKET")
        .map_or_else(|| wire::SOCKET.into(), PathBuf::from);
    let mut password: Vec<u8> = Vec::new(); // empty = use cached authorization
    for _ in 0..=PASSWORD_TRIES {
        let reply = match env.request(&sock, &wire::encode(req, &password)) {
            Ok(r) => AdminReply::parse(&r),
            Err(_) => {
                return fail(
                    env,
                    ErrorCode::Unavailable,
                    "administrator service unavailable (wol-admind)",
                );
            }
        };
        crate::authcheck::wipe(&mut password);
        match reply {
            AdminReply::NeedsAuth | AdminReply::Denied => {
                if reply == AdminReply::Denied {
                    env.err("wol admin: incorrect password\n");
                }
                match env.stdin_line() {
                    Some(p) if !p.is_empty() => password = p.into_bytes(),
                    _ => return fail(env, ErrorCode::Denied, reply.message()),
                }
            }
            AdminReply::Ok => return 0,
            r => {
                env.err(&format!("wol admin: {}\n", r.message()));
                return admin_exit(r);
            }
        }
    }
    fail(env, ErrorCode::Denied, AdminReply::Denied.message())
}

fn bundle_plan(env: &mut dyn Env, bundle: &str, json: bool) -> i32 {
    let dir = Path::new(bundle);
    let Some(text) = env.read_file(&dir.join(shared::bundle::INFO_FILE)) else {
        return fail(
            env,
            ErrorCode::NotFound,
            &format!("{bundle}: no {}", shared::bundle::INFO_FILE),
        );
    };
    let info = match shared::bundle::Info::parse(&text) {
        Ok(i) => i,
        Err(e) => return fail(env, ErrorCode::Invalid, &format!("{bundle}: {e}")),
    };
    let host = wol_core::bundle::HostCaps {
        arch: wol_core::bundle::host_arch().into(),
        linux_compat: true,
        proton: env.var("WOL_PROTON").is_some(),
    };
    match wol_core::bundle::resolve(&info, &host) {
        None => fail(
            env,
            ErrorCode::Unsupported,
            &format!("{}: no target runs on this host ({})", info.id, host.arch),
        ),
        Some(t) => {
            if json || !env.stdout_tty() {
                env.out(&format!(
                    "{}\n",
                    serde_json::to_string(t).unwrap_or_default()
                ));
            } else {
                let abi = serde_json::to_value(t.abi)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                env.out(&format!(
                    "{} {}: {abi} {} {}\n",
                    info.id, info.version, t.arch, t.entry
                ));
            }
            0
        }
    }
}

fn theme(env: &mut dyn Env, inv: &Invocation, json: bool) -> i32 {
    let home = env.config_home();
    let catalog = ThemeCatalog::load(&search_paths(&home));
    let file = active::default_file(&home);
    let dark = env
        .read_file(Path::new(wol_core::settings::appearance::FILE))
        .as_deref()
        .map(|t| wol_core::settings::appearance::parse(Some(t)))
        == Some(wol_core::settings::appearance::DARK);
    let current = active::effective(active::get(&file).as_deref(), dark);
    let set = |env: &mut dyn Env, id: &str| -> i32 {
        if let Err(e) = catalog.resolve(id) {
            return fail(env, ErrorCode::NotFound, &format!("theme '{id}': {e:?}"));
        }
        let journal = wol_core::journal::Journal::new(&env.state_home(), &home);
        let text = format!("{id}\n");
        match journal.save_path("theme", &env.actor(), &file, Some(&text), env.now_secs()) {
            Ok(_) => {
                if let Ok(t) = catalog.resolve(id) {
                    restyle_apps(env, &t);
                }
                env.out(&format!("{id}\n"));
                0
            }
            Err(e) => fail(env, ErrorCode::Failed, &format!("{}: {e}", file.display())),
        }
    };
    match inv.action {
        "theme.current" => {
            env.out(&format!("{current}\n"));
            0
        }
        "theme.list" => {
            let list = catalog.list();
            if json {
                let v: Vec<_> = list.iter().map(|t| serde_json::json!({"id": t.id, "name": t.name, "variant": t.variant.name()})).collect();
                env.out(&format!("{}\n", serde_json::Value::from(v)));
            } else {
                let s: String = list
                    .iter()
                    .map(|t| {
                        format!(
                            "{}{:<20} {:<6} {}\n",
                            if t.id == current { "* " } else { "  " },
                            t.id,
                            t.variant.name(),
                            t.name
                        )
                    })
                    .collect();
                env.out(&s);
            }
            0
        }
        "theme.set" => set(env, inv.get("id").unwrap_or_default()),
        "theme.switch" => {
            let cmd = inv.path.last().map_or("", String::as_str);
            let Ok(cur) = catalog
                .resolve(&current)
                .or_else(|_| catalog.resolve(shared::theme::Variant::Light.default_theme()))
            else {
                return fail(env, ErrorCode::Failed, "no built-in themes");
            };
            match active::switch(cmd, &cur) {
                Some(id) => set(env, id),
                None => fail(env, ErrorCode::Usage, cmd),
            }
        }
        _ => {
            let id = inv.get("id").unwrap_or(&current).to_string();
            match catalog.resolve(&id) {
                Ok(t) => {
                    env.out(&t.css());
                    0
                }
                Err(e) => fail(env, ErrorCode::NotFound, &format!("theme '{id}': {e:?}")),
            }
        }
    }
}

/// Third-party toolkits follow the theme: regenerate GTK/Qt/foot files, then gsettings so
/// running GTK apps switch live (settings portal). Best effort: a missing gsettings is not an error.
pub(crate) fn restyle_apps(env: &mut dyn Env, t: &wol_core::theme::ResolvedTheme) {
    use wol_core::theme::apps;
    let home = env.config_home();
    if let Err(e) = apps::install(t, &apps::user_dir(&home), &home) {
        env.err(&format!("wol: app themes: {e}\n"));
    }
    if let Some(data) = apps::data_home(|k| env.var(k))
        && let Err(e) = apps::link_data(t, &apps::user_dir(&home), &data)
    {
        env.err(&format!("wol: app themes: {e}\n"));
    }
    for argv in apps::gsettings_argv(t) {
        let _ = env.exec(&argv);
    }
}

/// Shell wire line for a shell.* invocation.
pub fn shell_line(inv: &Invocation) -> String {
    match inv.action {
        "shell.reload" => "reload\n".into(),
        a => format!(
            "{} {}\n",
            a.trim_start_matches("shell."),
            inv.get("panel").unwrap_or_default()
        ),
    }
}

fn shell(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let Some(rt) = env.var("XDG_RUNTIME_DIR") else {
        return fail(
            env,
            ErrorCode::Unavailable,
            "XDG_RUNTIME_DIR unset (no session)",
        );
    };
    let sock = Path::new(&rt).join(SHELL_SOCKET);
    match env.request(&sock, shell_line(inv).as_bytes()) {
        Ok(_) => 0,
        Err(e) => fail(
            env,
            ErrorCode::Unavailable,
            &format!("shell not running ({e})"),
        ),
    }
}

/// Refresh the package catalogue through wol-admind (no password: the person at the computer)
/// so "up to date" is never read from a stale catalogue.
fn refresh_catalogue(env: &mut dyn Env) -> bool {
    let sock = env
        .var("WOL_ADMIN_SOCKET")
        .map_or_else(|| wire::SOCKET.into(), PathBuf::from);
    let req = AdminRequest {
        action: AdminAction::PkgRefresh,
        args: vec![],
    };
    env.request_wait(&sock, &wire::encode(&req, b""), 180)
        .is_ok_and(|r| AdminReply::parse(&r) == AdminReply::Ok)
}

fn update_check(env: &mut dyn Env, json: bool) -> i32 {
    let refreshed = refresh_catalogue(env);
    let out = env.capture(&wol_core::update::PENDING_ARGV);
    let Some(mut pending) = wol_core::update::parse_pending(out.as_deref()) else {
        return fail(env, ErrorCode::Unavailable, "pacman -Qu failed");
    };
    // A newer release the daily check found, until installed (`wol update apply`).
    if let Some(b) = update_snapshot(env).and_then(|u| u.base) {
        pending.push(format!("WolOS {b}"));
    }
    if pending.is_empty() && !refreshed {
        // Never claim "up to date" from a catalogue that could not be refreshed.
        return fail(
            env,
            ErrorCode::Unavailable,
            "could not check for updates right now (are you online?)",
        );
    }
    if json || !env.stdout_tty() {
        env.out(&format!("{}\n", serde_json::Value::from(pending)));
    } else if pending.is_empty() {
        env.out("up to date\n");
    } else {
        env.out(&format!("{}\n", pending.join("\n")));
    }
    0
}

/// The daily check's findings (wol_core::update::check), without a release installed since.
fn update_snapshot(env: &mut dyn Env) -> Option<wol_core::update::check::Updates> {
    use wol_core::update::check::{DIR, USERLAND_ARGV, Updates};
    let keys = wol_core::package::read_keys(Path::new(
        &env.var("WOL_KEYS_DIR")
            .unwrap_or_else(|| shared::package::KEYS_DIR.into()),
    ));
    let f = |n: &str| env.read_file(&Path::new(DIR).join(n)).unwrap_or_default();
    let u = Updates::from_files(f, &keys)?;
    let userland = u.base.as_ref().and_then(|_| env.capture(&USERLAND_ARGV));
    Some(u.installed(userland.as_deref()))
}

/// Running WolOS base: (os-release VERSION_ID, kernel release, kernel is linux-wolos).
fn running_base(env: &mut dyn Env) -> (Option<String>, String, bool) {
    let base = env
        .read_file(Path::new("/etc/os-release"))
        .and_then(|t| wol_core::base::version(&t));
    let kernel = env
        .read_file(Path::new("/proc/sys/kernel/osrelease"))
        .map(|k| k.trim().to_owned())
        .unwrap_or_default();
    let wolos = wol_core::games::doctor::is_wolos_kernel(&kernel);
    (base, kernel, wolos)
}

/// `wol base verify`: every file of the wolos-base manifest against its sha256. `WOL_ROOT`
/// checks another root (an installed disk).
fn base_verify(env: &mut dyn Env, json: bool) -> i32 {
    let root = env.var("WOL_ROOT").unwrap_or_default();
    let at = |p: &str| PathBuf::from(format!("{root}{p}"));
    let Some(text) = env.read_file(&at(wol_core::base::MANIFEST)) else {
        return fail(
            env,
            ErrorCode::NotFound,
            &format!("no base manifest at {}", wol_core::base::MANIFEST),
        );
    };
    let Some(entries) = wol_core::base::parse_manifest(&text) else {
        return fail(env, ErrorCode::Failed, "base manifest is malformed");
    };
    let findings =
        wol_core::base::verify(&entries, &|p| wol_core::package::sha256_file(&at(p)).ok());
    if json || !env.stdout_tty() {
        env.out(&format!(
            "{}\n",
            serde_json::json!({"files": entries.len(), "findings": findings})
        ));
    } else if findings.is_empty() {
        env.out(&format!("{} base files intact\n", entries.len()));
    } else {
        for f in &findings {
            env.out(&format!("{} {}\n", f.state, f.path));
        }
    }
    if findings.is_empty() {
        0
    } else {
        ErrorCode::Failed.exit()
    }
}

/// `wol update status`: null / "not checked yet" before the first check.
fn update_status(env: &mut dyn Env, json: bool) -> i32 {
    let u = update_snapshot(env);
    let (base, kernel, wolos) = running_base(env);
    if json || !env.stdout_tty() {
        let v = u.map(|u| {
            let mut v = serde_json::json!(u);
            v["count"] = u.count().into();
            v["base_version"] = base.clone().into();
            v["kernel"] = kernel.clone().into();
            v["kernel_is_wolos"] = wolos.into();
            v
        });
        env.out(&format!("{}\n", v.unwrap_or_default()));
        return 0;
    }
    let Some(u) = u else {
        env.out("not checked yet\n");
        return 0;
    };
    let mut s = format!(
        "base {}, kernel {}{}\nchecked {}\n",
        base.as_deref().unwrap_or("unknown"),
        if kernel.is_empty() {
            "unknown"
        } else {
            &kernel
        },
        if wolos { " (linux-wolos)" } else { "" },
        u.checked
    );
    for p in &u.packages {
        let tag = if u.security.contains(p) {
            " (security fix)"
        } else {
            ""
        };
        s.push_str(&format!("{p}{tag}\n"));
    }
    if let Some(b) = &u.base {
        s.push_str(&format!("WolOS {b}\n"));
    }
    for k in &u.kernels {
        s.push_str(&format!(
            "{k}: your own kernel, kept (WolOS boots linux-wolos)\n"
        ));
    }
    if u.count() == 0 {
        s.push_str("up to date\n");
    }
    env.out(&s);
    0
}

/// `wol update plan|apply [slot] [--release YYYY/MM/DD]`: update into the other root slot
/// (`wol_core::update::plan`); the release defaults to the one the daily check found.
fn update(env: &mut dyn Env, inv: &Invocation) -> i32 {
    use wol_core::update::{self as up, Step};
    let running = env
        .read_file(Path::new("/proc/cmdline"))
        .and_then(|c| up::running_slot(&c));
    let slot = match inv.get("be") {
        Some(s) => s.to_string(),
        None => running
            .and_then(up::other_slot)
            .unwrap_or(up::SLOTS[1])
            .to_string(),
    };
    if Some(slot.as_str()) == running {
        return fail(
            env,
            ErrorCode::Usage,
            &format!("{slot} is the running system; update the other slot"),
        );
    }
    let date = inv
        .get("release")
        .map(String::from)
        .or_else(|| update_snapshot(env).and_then(|u| u.snapshot));
    let replace = env
        .capture(&up::be::LIST_ARGV)
        .is_some_and(|t| up::be::parse(&t).iter().any(|b| b.name == slot));
    let Some(steps) = up::plan(&slot, replace, date.as_deref()) else {
        return fail(
            env,
            ErrorCode::Usage,
            &format!("bad slot '{slot}' or release date"),
        );
    };
    if inv.action == "update.plan" {
        env.out(&steps.iter().map(|s| format!("{s}\n")).collect::<String>());
        return 0;
    }
    if !inv.flag("yes") {
        return fail(
            env,
            ErrorCode::Usage,
            "update apply changes the system; pass --yes",
        );
    }
    if !env.is_root() {
        return fail(env, ErrorCode::Denied, "update apply needs root");
    }
    for s in &steps {
        env.err(&format!("wol update: {s}\n"));
        let code = match s {
            Step::Run(argv) => env.exec(argv).unwrap_or(127),
            Step::Write { path, text } => i32::from(env.write_file(Path::new(path), text).is_err()),
        };
        if code != 0 {
            for r in up::rollback(&slot) {
                let _ = env.exec(&r);
            }
            return fail(
                env,
                ErrorCode::Failed,
                &format!("{s} failed ({code}); slot {slot} removed"),
            );
        }
    }
    env.out(&format!("updated into {slot}; restart to use it\n"));
    0
}

/// Real side effects.
pub struct RealEnv;

impl Env for RealEnv {
    fn out(&mut self, s: &str) {
        use io::Write;
        let _ = io::stdout().write_all(s.as_bytes());
    }
    fn err(&mut self, s: &str) {
        use io::Write;
        let _ = io::stderr().write_all(s.as_bytes());
    }
    fn stdout_tty(&self) -> bool {
        use std::io::IsTerminal;
        io::stdout().is_terminal()
    }
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    }
    fn is_root(&self) -> bool {
        // SAFETY: geteuid has no preconditions.
        unsafe { libc::geteuid() == 0 }
    }
    fn request(&mut self, socket: &Path, req: &[u8]) -> io::Result<String> {
        self.request_wait(socket, req, 30)
    }
    fn request_wait(&mut self, socket: &Path, req: &[u8], secs: u32) -> io::Result<String> {
        use io::{Read, Write};
        let mut s = std::os::unix::net::UnixStream::connect(socket)?;
        s.set_read_timeout(Some(std::time::Duration::from_secs(secs.into())))?;
        s.write_all(req)?;
        s.shutdown(std::net::Shutdown::Write)?;
        let mut out = String::new();
        s.read_to_string(&mut out)?;
        Ok(out)
    }
    fn exec(&mut self, argv: &[String]) -> io::Result<i32> {
        let st = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .status()?;
        Ok(st.code().unwrap_or(1))
    }
    fn exec_quiet(&mut self, argv: &[String]) -> io::Result<i32> {
        use std::os::fd::AsFd;
        let stderr = io::stderr().as_fd().try_clone_to_owned()?;
        let st = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(stderr))
            .status()?;
        Ok(st.code().unwrap_or(1))
    }
    fn capture(&mut self, argv: &[&str]) -> Option<String> {
        wol_core::system::run_with_timeout(argv, std::time::Duration::from_secs(60))
    }
    fn stdin_line(&mut self) -> Option<String> {
        let mut l = String::new();
        io::stdin().read_line(&mut l).ok()?;
        Some(l.trim_end_matches(['\n', '\r']).to_owned())
    }
    fn read_file(&self, p: &Path) -> Option<String> {
        std::fs::read_to_string(p).ok()
    }
    fn sys_topic(&mut self, topic: &str) -> Option<serde_json::Value> {
        wol_core::system::SystemApi::default_host().topic_json(topic)
    }
    fn now_secs(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
    fn config_home(&self) -> PathBuf {
        config_home()
    }
}

/// Recording fake for tests.
#[derive(Default)]
pub struct FakeEnv {
    pub out: String,
    pub err: String,
    pub tty: bool,
    pub root: bool,
    pub vars: Vec<(String, String)>,
    /// (socket, request bytes) sent.
    pub requests: Vec<(PathBuf, Vec<u8>)>,
    /// Replies in order; empty = connection refused.
    pub replies: Vec<String>,
    pub execs: Vec<Vec<String>>,
    pub exec_codes: Vec<i32>,
    pub captured: Option<String>,
    pub stdin: Vec<String>,
    pub files: Vec<(PathBuf, String)>,
    pub home: PathBuf,
    /// Per-command captures (`argv joined by spaces`, stdout); else `captured`.
    pub captures: Vec<(String, String)>,
    /// Host behind `controls()` (Linux fake when None).
    pub host: Option<std::sync::Arc<wol_core::system::fake::FakeHost>>,
    /// wol-admind as seen by `controls()`: replies in order (none left = not running), requests sent.
    pub admin: std::sync::Arc<std::sync::Mutex<FakeAdmin>>,
    /// `hardware()` results in order (the last one repeats); empty = read `host`.
    pub hw: Vec<shared::hardware::HardwareSnapshot>,
    /// Files replaced through `write_file`.
    pub written: Vec<(PathBuf, String)>,
}

#[derive(Default)]
pub struct FakeAdmin {
    pub replies: Vec<AdminReply>,
    /// `action args.. pw=<password>` per request.
    pub sent: Vec<String>,
}

impl Env for FakeEnv {
    fn out(&mut self, s: &str) {
        self.out.push_str(s);
    }
    fn err(&mut self, s: &str) {
        self.err.push_str(s);
    }
    fn stdout_tty(&self) -> bool {
        self.tty
    }
    fn var(&self, name: &str) -> Option<String> {
        self.vars.iter().find(|v| v.0 == name).map(|v| v.1.clone())
    }
    fn is_root(&self) -> bool {
        self.root
    }
    fn request(&mut self, socket: &Path, req: &[u8]) -> io::Result<String> {
        self.requests.push((socket.into(), req.into()));
        if self.replies.is_empty() {
            return Err(io::Error::from(io::ErrorKind::ConnectionRefused));
        }
        Ok(self.replies.remove(0))
    }
    fn exec(&mut self, argv: &[String]) -> io::Result<i32> {
        self.execs.push(argv.into());
        Ok(if self.exec_codes.is_empty() {
            0
        } else {
            self.exec_codes.remove(0)
        })
    }
    /// `probe <url>` captures give the latency in ms; none = unreachable.
    fn prober(&self) -> Prober {
        let c = self.captures.clone();
        Box::new(move |u| {
            let k = format!("probe {u}");
            c.iter().find(|x| x.0 == k)?.1.trim().parse().ok()
        })
    }
    fn capture(&mut self, argv: &[&str]) -> Option<String> {
        let line = argv.join(" ");
        match self.captures.iter().find(|c| c.0 == line) {
            Some(c) => Some(c.1.clone()),
            None => self.captured.clone(),
        }
    }
    fn stdin_line(&mut self) -> Option<String> {
        (!self.stdin.is_empty()).then(|| self.stdin.remove(0))
    }
    fn read_line(&mut self) -> Option<String> {
        self.stdin_line()
    }
    /// Directories and files implied by the fake files' paths.
    fn list_dir(&self, p: &Path) -> Vec<String> {
        let mut v: Vec<String> = self
            .files
            .iter()
            .filter_map(|f| f.0.strip_prefix(p).ok())
            .filter_map(|r| r.components().next())
            .map(|c| p.join(c).to_string_lossy().into_owned())
            .collect();
        v.sort();
        v.dedup();
        v
    }
    /// Fake files first, then real files under `home` (tests use a temp home).
    fn read_file(&self, p: &Path) -> Option<String> {
        self.files
            .iter()
            .find(|f| f.0 == p)
            .map(|f| f.1.clone())
            .or_else(|| {
                (!self.home.as_os_str().is_empty() && p.starts_with(&self.home))
                    .then(|| std::fs::read_to_string(p).ok())
                    .flatten()
            })
    }
    fn controls(&self) -> wol_core::controls::Controls {
        use std::sync::Arc;
        let host = self
            .host
            .clone()
            .unwrap_or_else(|| Arc::new(wol_core::system::fake::FakeHost::new(false)));
        let admin = self.admin.clone();
        wol_core::controls::Controls {
            host,
            stdin: Arc::new(|_, _| Some(String::new())),
            admin: Arc::new(move |req, pw| {
                let mut a = admin.lock().unwrap_or_else(|e| e.into_inner());
                a.sent.push(format!(
                    "{} {} pw={pw}",
                    req.action.name(),
                    req.args.join(" ")
                ));
                (!a.replies.is_empty()).then(|| a.replies.remove(0))
            }),
            wlan: "wlan0".into(),
        }
    }
    fn state_home(&self) -> PathBuf {
        self.home.join("state")
    }
    fn home(&self) -> PathBuf {
        self.home.clone()
    }
    fn sys_topic(&mut self, topic: &str) -> Option<serde_json::Value> {
        (topic == "about").then(|| serde_json::json!({"os": "WolOS", "kernel": "15.1"}))
    }
    fn now_secs(&self) -> u64 {
        1000
    }
    fn config_home(&self) -> PathBuf {
        self.home.clone()
    }
    fn games_facts(&self) -> wol_core::games::doctor::Facts {
        let host = self
            .host
            .clone()
            .unwrap_or_else(|| std::sync::Arc::new(wol_core::system::fake::FakeHost::new(true)));
        let var = |n: &str| self.var(n);
        let futex = wol_core::games::doctor::FutexWaitv::Unknown;
        wol_core::games::doctor::gather(&*host, &self.home, &var, futex)
    }
    fn hardware(&mut self) -> shared::hardware::HardwareSnapshot {
        match self.hw.len() {
            0 => self
                .host
                .as_ref()
                .map(|h| wol_core::hardware::read(&**h))
                .unwrap_or_default(),
            1 => self.hw[0].clone(),
            _ => self.hw.remove(0),
        }
    }
    fn write_file(&mut self, path: &Path, text: &str) -> io::Result<()> {
        self.written.push((path.into(), text.into()));
        Ok(())
    }
}
