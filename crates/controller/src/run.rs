// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol` actions. All I/O goes through [`Env`], so every action is testable with [`FakeEnv`].
use crate::cli::{Invocation, help};
use shared::admin::{self as wire, AdminAction, AdminReply, AdminRequest};
use shared::errors::ErrorCode;
use shared::jobs::{JobRequest, JobStatus, Reply};
use std::io;
use std::path::{Path, PathBuf};
use wol_core::theme::{ThemeCatalog, active, config_home};

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
    /// Like `exec`, with one secret in the child's environment (`$PASSWORD`, `$NEWPASSWORD` for
    /// systemd-cryptenroll), never in its arguments. Refused where there is no child to give it to.
    fn exec_secret(&mut self, argv: &[String], var: &str, secret: &str) -> io::Result<i32> {
        let _ = (argv, var, secret);
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
    /// Payload of the `user` key `desc` in root's user keyring (systemd's password cache).
    fn keyring_read(&mut self, desc: &str) -> Option<Vec<u8>> {
        let _ = desc;
        None
    }
    /// Drop that key once used.
    fn keyring_forget(&mut self, desc: &str) {
        let _ = desc;
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
    /// Replace a whole file: `shared::atomic` temp file + rename (`wol hw boot`; S4-10). A file
    /// that exists keeps its mode (a 0600 `wol encrypt hooks` or `folded-layers` stays 0600); a
    /// new one is readable by all (0644).
    fn write_file(&mut self, path: &Path, text: &str) -> io::Result<()> {
        shared::atomic::atomic_write(path, kept_mode(path), text.as_bytes())
    }
    /// [`Env::write_file`] for a secret (a typed passphrase, a recovery key): mode 0600 from the
    /// first byte, never readable by anyone else (S4-10).
    fn write_private(&mut self, path: &Path, text: &str) -> io::Result<()> {
        shared::atomic::atomic_write(path, 0o600, text.as_bytes())
    }
    /// Delete one file; a file that is already gone is not an error (`wol webapp remove`).
    fn remove_file(&mut self, path: &Path) -> io::Result<()> {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }
    /// Create a directory and its parents (`wol webapp add`, `wol toggle`).
    fn make_dir(&mut self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
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
    /// `$XDG_DATA_HOME` when set and non-empty, else `home()/.local/share`.
    fn data_home(&self) -> PathBuf {
        wol_core::theme::apps::data_home(|k| self.var(k))
            .unwrap_or_else(|| self.home().join(".local/share"))
    }
    /// Who acts (journal, audit log): `cli`, `script:<WOL_HELPER>`, `mcp:<client>`.
    fn actor(&self) -> String {
        self.var("WOL_HELPER")
            .map_or_else(|| "cli".into(), |h| format!("script:{h}"))
    }
}

/// The permission bits `path` has now (never set-id bits), 0644 when it does not exist or is not
/// a regular file.
pub(crate) fn kept_mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map_or(0o644, |m| m.permissions().mode() & 0o777)
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
    // `--help` is a global flag: print the usage of this path and run nothing (CLI-1:
    // `wol power shutdown --help` must not shut down).
    if inv.flag("help") {
        env.out(&help(&inv.path));
        return 0;
    }
    // A named script is a helper: policy + audit log (docs/modules/automation.md).
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
        "encrypt.rebind" => crate::seal::rebind(env),
        "encrypt.reseal" => crate::seal::reseal(env),
        "encrypt.password" => crate::seal::password(env, inv),
        "boot.password" => crate::boot::password(env),
        "boot.enroll" => crate::boot::enroll(env),
        "boot.detect" => crate::boot::detect(env, inv),
        "boot.key_screen" => crate::boot::key_screen(env, inv),
        "boot.words" => crate::boot::words(env, inv),
        "boot.note" => crate::boot::note(env, inv),
        "boot.status" => crate::boot::status(env),
        "boot.check" => crate::boot::check(env),
        "boot.bless" => crate::boot::bless(env),
        "seal.apply" => crate::seal::apply(env, inv),
        "seal.check" => crate::seal::check(env),
        "seal.off" => crate::pkg::seal_off(env),
        "layers.fold" => crate::pkg::fold(env, inv),
        "layers.clear" => crate::pkg::clear(env),
        "pkg" => crate::pkg::pkg_verb(env, inv),
        "install.info" | "install.probe" | "install.check" | "install.layout"
        | "install.settings" | "install.account" | "install.facts" | "install.mounts"
        | "install.hostname" => crate::install::verb(env, inv),
        "encrypt.decide" => crate::seal::decide(env, inv),
        "encrypt.enroll" => crate::seal::enroll(env, inv),
        "encrypt.words" => crate::seal::words(env, inv),
        "encrypt.hooks" => crate::seal::hooks(env, inv),
        "update.rollback" => crate::verbs::rollback(env, inv.get("be")),
        "mirror.status" | "mirror.rank" | "mirror.gate" | "mirror.kernel" => {
            crate::mirror::run(env, inv.action, inv.flag("yes"), json)
        }
        "gpu.env" => exec(env, &[GPU_BIN.into(), "gpu".into(), "env".into()]),
        "gpu.hyprconf" => exec(env, &[GPU_BIN.into(), "gpu".into(), "hyprconf".into()]),
        "hw.list" | "hw.status" | "hw.fix" | "hw.boot" | "hw.online" | "hw.compute" => {
            crate::hw::execute(inv, env)
        }
        "audio.doctor" | "audio.reset" | "audio.appmix" => crate::audio_doctor::execute(inv, env),
        a if a.starts_with("games.") => crate::games::execute(inv, env),
        a if a.starts_with("modules.") => crate::modules::execute(inv, env),
        a if a.starts_with("sessions.") => crate::sessions::execute(inv, env),
        a if a.starts_with("plugin.") => crate::plugin::execute(inv, env),
        a if a.starts_with("power.") => crate::power::execute(inv, env),
        a if a.starts_with("firewall.") || a.starts_with("security.") => {
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

/// wol-admind's socket (`WOL_ADMIN_SOCKET` in tests).
fn admin_socket(env: &dyn Env) -> PathBuf {
    env.var("WOL_ADMIN_SOCKET")
        .map_or_else(|| wire::SOCKET.into(), PathBuf::from)
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
    let sock = admin_socket(env);
    let mut password: Vec<u8> = Vec::new(); // empty = use cached authorization
    for attempt in 0..=PASSWORD_TRIES {
        // As long as wol-admind may take for this request, every planned step (S4-4/S4-18).
        let secs = wol_core::admin::reply_secs(req);
        let sent = env.request_wait(&sock, &wire::encode(req, &password), secs);
        crate::authcheck::wipe(&mut password);
        let reply = match sent {
            Ok(r) => AdminReply::parse(&r),
            // A slow action (a package, a boot entry) is still running in wol-admind (S4-18).
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                return fail(
                    env,
                    ErrorCode::Failed,
                    "wol-admind is still working on it; check again later",
                );
            }
            Err(_) => {
                return fail(
                    env,
                    ErrorCode::Unavailable,
                    "administrator service unavailable (wol-admind)",
                );
            }
        };
        match reply {
            AdminReply::NeedsAuth | AdminReply::Denied => {
                if reply == AdminReply::Denied {
                    env.err("wol admin: incorrect password\n");
                }
                // The last try does not ask for a password nobody will check (S4-18).
                if attempt == PASSWORD_TRIES {
                    break;
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
    // Built-ins, theme folders and enabled plugins' theme packs (ADR 0031 themes v2).
    let data = wol_core::theme::apps::data_home(|k| env.var(k));
    let catalog = ThemeCatalog::installed(&home, data.as_deref());
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
/// running GTK apps switch live (settings portal), then the `theme` event on wold for libwol apps
/// (`subscribe("theme")`). Best effort: a missing gsettings or wold is not an error.
/// Every theme change goes through here: `wol theme set|light|dark|toggle`, appearance and accent
/// (verbs.rs `set_theme`) and undo.
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
    retire_hypr_conf(env, &home);
    for argv in apps::gsettings_argv(t)
        .into_iter()
        .chain(apps::strip_argv(t))
    {
        let _ = env.exec(&argv);
    }
    announce_theme(env, &t.id);
}

/// The upgrade to the compositor's Lua configuration (WolOS 0.13): the first theme switch (`wol theme
/// set`, at every session start) carries the 0.56 `.conf` files over (`hyprprefs::retire_plan`) and
/// reloads the compositor once, so the person's keyboard, displays, Settings and shortcuts hold.
/// A stale file stays until its Lua file is written.
fn retire_hypr_conf(env: &mut dyn Env, home: &Path) {
    carry_user_conf(env, home);
    let plan = {
        let read = |r: &str| env.read_file(&home.join(r));
        let present = |r: &str| std::fs::symlink_metadata(home.join(r)).is_ok();
        let shipped = |p: &str| env.read_file(Path::new(p));
        wol_core::hyprprefs::retire_plan(&read, &present, &shipped)
    };
    if plan.remove.is_empty() {
        return;
    }
    for (rel, text) in &plan.write {
        let p = home.join(rel);
        let made = p.parent().map_or(Ok(()), |d| env.make_dir(d));
        if let Err(e) = made.and_then(|()| env.write_file(&p, text)) {
            env.err(&format!("wol: {rel}: {e}\n"));
            return;
        }
    }
    for rel in &plan.remove {
        let _ = env.remove_file(&home.join(rel));
    }
    let _ = env.exec_quiet(&["hyprctl".into(), "reload".into()]);
}

/// Title and body of the notice [`carry_user_conf`] shows.
pub const CARRIED_NOTICE: (&str, &str) = (
    "Your compositor settings moved",
    "Your own lines from hyprland.conf are now comments at the end of hyprland.lua (in ~/.config/hypr). Write each again in Lua to use it.",
);

/// The person's own lines of the 0.12 user hyprland.conf go into hyprland.lua as comments
/// (`hyprprefs::carry_user_conf`), and the person is told once, as a notification.
fn carry_user_conf(env: &mut dyn Env, home: &Path) {
    match wol_core::hyprprefs::carry_user_conf(home) {
        Ok(true) => {
            let (title, body) = CARRIED_NOTICE;
            env.err(&format!("wol: {title}. {body}\n"));
            let argv = ["notify-send", "--app-name=WolOS", "--", title, body];
            let _ = env.exec_quiet(&argv.map(String::from));
        }
        Ok(false) => {}
        Err(e) => env.err(&format!("wol: {}: {e}\n", wol_core::prefs::USER_CONF)),
    }
}

/// `publish {"event":"theme","id":ID}` on wold's socket: wold hands an Everyone event from a client
/// to that client's own uid (and root), so the person's apps hear their own theme change. wold
/// answers nothing; no wold (a chroot, early boot) is fine.
fn announce_theme(env: &mut dyn Env, id: &str) {
    let e = shared::events::Event::Theme { id: id.to_owned() };
    let line = JobRequest::Publish(e.to_line().trim_end().to_owned()).line() + "\n";
    let sock = jobs_socket(env);
    let _ = env.request(&sock, line.as_bytes());
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
    let one_copy = env.read_file(Path::new(up::SINGLE_COPY)).is_some();
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
    let bes = up::be::parse(&env.capture(&up::be::LIST_ARGV).unwrap_or_default());
    let there = bes.iter().find(|b| b.name == slot);
    // A slot that waits for its first boot (a kernel installed with pacman) is built on, not
    // replaced: the update stacks onto it (CORE-6).
    let from = match there {
        Some(b) if b.next_boot && !b.active => up::Base::Pending,
        Some(_) => up::Base::Running { replace: true },
        None => up::Base::Running { replace: false },
    };
    let cmdline = env.read_file(Path::new("/proc/cmdline"));
    let planned = if one_copy {
        up::plan_in_place(date.as_deref())
    } else {
        up::plan_from(&slot, from, date.as_deref(), cmdline.as_deref())
    };
    let Some(steps) = planned else {
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
            for r in up::rollback_from(&slot, from) {
                let _ = env.exec(&r);
            }
            return fail(
                env,
                ErrorCode::Failed,
                &if from == up::Base::Pending {
                    format!("{s} failed ({code}); slot {slot} kept as it was")
                } else {
                    format!("{s} failed ({code}); slot {slot} removed")
                },
            );
        }
    }
    env.out(&format!("updated into {slot}; restart to use it\n"));
    0
}

/// Real side effects.
pub struct RealEnv;

const KEYCTL_SEARCH: libc::c_long = 10;
const KEYCTL_READ: libc::c_long = 11;
const KEYCTL_INVALIDATE: libc::c_long = 21;
const KEY_SPEC_USER_KEYRING: libc::c_long = -4;

/// keyctl(KEYCTL_SEARCH, @u, "user", desc): the key's serial.
fn keyring_search(desc: &str) -> Option<libc::c_long> {
    let ty = std::ffi::CString::new("user").ok()?;
    let d = std::ffi::CString::new(desc).ok()?;
    // SAFETY: NUL-terminated strings that outlive the call; destination keyring 0 = none.
    let id = unsafe {
        libc::syscall(
            libc::SYS_keyctl,
            KEYCTL_SEARCH,
            KEY_SPEC_USER_KEYRING,
            ty.as_ptr(),
            d.as_ptr(),
            0 as libc::c_long,
        )
    };
    (id > 0).then_some(id)
}

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
        // wold control socket and wol-admind greet first (ADR 0033); other sockets do not.
        let greets = socket == jobs_socket(self) || socket == admin_socket(self);
        let mut s = if greets {
            shared::jobs::connect(socket)?
        } else {
            std::os::unix::net::UnixStream::connect(socket)?
        };
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
    fn exec_secret(&mut self, argv: &[String], var: &str, secret: &str) -> io::Result<i32> {
        let st = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .env(var, secret)
            .status()?;
        Ok(st.code().unwrap_or(1))
    }
    fn keyring_read(&mut self, desc: &str) -> Option<Vec<u8>> {
        let id = keyring_search(desc)?;
        let mut buf = vec![0u8; 4096];
        // SAFETY: KEYCTL_READ writes at most buf.len() bytes into buf.
        let n = unsafe {
            libc::syscall(
                libc::SYS_keyctl,
                KEYCTL_READ,
                id,
                buf.as_mut_ptr(),
                buf.len(),
            )
        };
        let n = usize::try_from(n).ok()?;
        buf.truncate(n.min(4096));
        Some(buf)
    }
    fn keyring_forget(&mut self, desc: &str) {
        if let Some(id) = keyring_search(desc) {
            // SAFETY: plain integer arguments.
            unsafe { libc::syscall(libc::SYS_keyctl, KEYCTL_INVALIDATE, id) };
        }
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
    /// (variable, secret) given to `exec_secret`, in order.
    pub secrets: Vec<(String, String)>,
    /// Root's user keyring: (description, payload).
    pub keys: Vec<(String, Vec<u8>)>,
    /// Files deleted through `remove_file`.
    pub removed: Vec<PathBuf>,
    /// Directories made through `make_dir`.
    pub dirs: Vec<PathBuf>,
    /// `secs` of every `request_wait`, in order.
    pub waits: Vec<u32>,
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
    fn request_wait(&mut self, socket: &Path, req: &[u8], secs: u32) -> io::Result<String> {
        self.waits.push(secs);
        self.request(socket, req)
    }
    fn exec(&mut self, argv: &[String]) -> io::Result<i32> {
        self.execs.push(argv.into());
        Ok(if self.exec_codes.is_empty() {
            0
        } else {
            self.exec_codes.remove(0)
        })
    }
    fn exec_secret(&mut self, argv: &[String], var: &str, secret: &str) -> io::Result<i32> {
        self.secrets.push((var.into(), secret.into()));
        self.exec(argv)
    }
    fn keyring_read(&mut self, desc: &str) -> Option<Vec<u8>> {
        self.keys.iter().find(|k| k.0 == desc).map(|k| k.1.clone())
    }
    fn keyring_forget(&mut self, desc: &str) {
        self.keys.retain(|k| k.0 != desc);
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
    fn write_private(&mut self, path: &Path, text: &str) -> io::Result<()> {
        self.written.push((path.into(), text.into()));
        Ok(())
    }
    fn remove_file(&mut self, path: &Path) -> io::Result<()> {
        self.removed.push(path.into());
        Ok(())
    }
    fn make_dir(&mut self, path: &Path) -> io::Result<()> {
        self.dirs.push(path.into());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wol-run-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Audit S4-10: `foo.conf` and `foo.key` written at once each end with their own text (no
    /// shared `foo.wol-tmp`), a 0600 file stays 0600, a new file is 0644, no temporary is left.
    #[test]
    fn write_file_is_atomic_per_file_and_keeps_the_mode() {
        let d = tmp("write");
        let (conf, key) = (d.join("foo.conf"), d.join("foo.key"));
        std::fs::write(&key, "old").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        let threads: Vec<_> = [(conf.clone(), "conf"), (key.clone(), "key")]
            .into_iter()
            .map(|(p, text)| {
                std::thread::spawn(move || {
                    for i in 0..100 {
                        RealEnv.write_file(&p, &format!("{text} {i}\n")).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(std::fs::read_to_string(&conf).unwrap(), "conf 99\n");
        assert_eq!(std::fs::read_to_string(&key).unwrap(), "key 99\n");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&key), 0o600);
        assert_eq!(mode(&conf), 0o644);
        RealEnv.write_private(&d.join("secret"), "s").unwrap();
        assert_eq!(mode(&d.join("secret")), 0o600);
        let left: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(shared::atomic::TMP_SUFFIX))
            .collect();
        assert!(left.is_empty(), "{left:?}");
        std::fs::remove_dir_all(d).unwrap();
    }

    fn admin_req(action: AdminAction, args: &[&str]) -> AdminRequest {
        AdminRequest {
            action,
            args: args.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    /// A theme change tells libwol apps: one `publish` of the `theme` event with the new theme's id
    /// on wold's socket, for `wol theme light|dark|set` and for appearance (verbs.rs `set_theme`).
    /// No wold is not an error: the theme is still written.
    #[test]
    fn theme_changes_publish_the_theme_event_on_wold() {
        let run = |line: &str, e: &mut FakeEnv| {
            let argv: Vec<&str> = line.split_whitespace().collect();
            execute(&crate::cli::parse(&argv).unwrap(), e)
        };
        let published = |e: &FakeEnv| -> Vec<String> {
            e.requests
                .iter()
                .filter(|(p, _)| p == Path::new("/w/wold.sock"))
                .map(|(_, r)| String::from_utf8_lossy(r).into_owned())
                .collect()
        };
        let mut e = FakeEnv {
            home: tmp("theme-event"),
            vars: vec![("WOL_JOBS_SOCKET".into(), "/w/wold.sock".into())],
            replies: vec![String::new(), String::new()],
            ..Default::default()
        };
        assert_eq!(run("theme dark", &mut e), 0, "{}", e.err);
        assert_eq!(run("theme light", &mut e), 0, "{}", e.err);
        assert_eq!(
            published(&e),
            [
                "publish {\"event\":\"theme\",\"id\":\"wol-dark\"}\n",
                "publish {\"event\":\"theme\",\"id\":\"wol-light\"}\n"
            ]
        );
        // What wold reads back is the event a subscriber gets (shared::jobs / shared::events).
        let line = published(&e)[0].clone();
        match JobRequest::parse(line.trim_end()) {
            Some(JobRequest::Publish(j)) => assert_eq!(
                shared::events::Event::parse(&j),
                Some(shared::events::Event::Theme {
                    id: "wol-dark".into()
                })
            ),
            other => panic!("{other:?}"),
        }
        // wold not running (no reply): still written, still 0.
        e.requests.clear();
        assert_eq!(run("theme dark", &mut e), 0, "{}", e.err);
        assert_eq!(published(&e).len(), 1);
        // Appearance goes through the same announcement.
        e.requests.clear();
        assert_eq!(run("appearance set light", &mut e), 0, "{}", e.err);
        assert_eq!(published(&e).len(), 1, "{:?}", e.requests);
        assert!(published(&e)[0].starts_with("publish {\"event\":\"theme\""));
    }

    /// Audit S4-4: a package request waits as long as wol-admind may run pacman; a quick one
    /// keeps a short wait; a reply that does not come in time is not "service unavailable".
    #[test]
    fn admin_requests_wait_per_action() {
        let mut e = FakeEnv {
            replies: vec!["ok\n".into()],
            ..Default::default()
        };
        assert_eq!(
            admin_send(&mut e, &admin_req(AdminAction::PkgInstall, &["gimp"])),
            0
        );
        assert!(e.waits[0] >= 1800, "{:?}", e.waits);
        let mut e = FakeEnv {
            replies: vec!["ok\n".into()],
            ..Default::default()
        };
        assert_eq!(admin_send(&mut e, &admin_req(AdminAction::Revoke, &[])), 0);
        assert!(e.waits[0] < 1800, "{:?}", e.waits);
    }

    /// A request wol-admind is still running: one request, "still working", never unavailable.
    #[test]
    fn a_slow_reply_is_not_service_unavailable() {
        struct Slow(FakeEnv);
        impl Env for Slow {
            fn out(&mut self, s: &str) {
                self.0.out(s)
            }
            fn err(&mut self, s: &str) {
                self.0.err(s)
            }
            fn stdout_tty(&self) -> bool {
                false
            }
            fn var(&self, n: &str) -> Option<String> {
                self.0.var(n)
            }
            fn is_root(&self) -> bool {
                false
            }
            fn request(&mut self, s: &Path, r: &[u8]) -> io::Result<String> {
                self.0.requests.push((s.into(), r.into()));
                Err(io::Error::from(io::ErrorKind::WouldBlock))
            }
            fn exec(&mut self, a: &[String]) -> io::Result<i32> {
                self.0.exec(a)
            }
            fn capture(&mut self, a: &[&str]) -> Option<String> {
                self.0.capture(a)
            }
            fn stdin_line(&mut self) -> Option<String> {
                self.0.stdin_line()
            }
            fn read_file(&self, p: &Path) -> Option<String> {
                self.0.read_file(p)
            }
            fn sys_topic(&mut self, t: &str) -> Option<serde_json::Value> {
                self.0.sys_topic(t)
            }
            fn now_secs(&self) -> u64 {
                0
            }
            fn config_home(&self) -> PathBuf {
                PathBuf::new()
            }
        }
        let mut e = Slow(FakeEnv::default());
        let code = admin_send(&mut e, &admin_req(AdminAction::PkgInstall, &["gimp"]));
        assert_eq!(code, ErrorCode::Failed.exit());
        assert!(e.0.err.contains("still working"), "{}", e.0.err);
        assert!(!e.0.err.contains("unavailable"), "{}", e.0.err);
        assert_eq!(e.0.requests.len(), 1, "never sent twice");
    }

    /// Audit S4-18: three wrong passwords are three requests with a password; a fourth line on
    /// stdin is never read.
    #[test]
    fn admin_reads_no_fourth_password() {
        let mut e = FakeEnv {
            replies: ["auth", "denied", "denied", "denied"]
                .map(|r| format!("{r}\n"))
                .to_vec(),
            stdin: ["a1", "b2", "c3", "d4"].map(String::from).to_vec(),
            ..Default::default()
        };
        let code = admin_send(&mut e, &admin_req(AdminAction::PkgInstall, &["gimp"]));
        assert_eq!(code, ErrorCode::Denied.exit());
        assert_eq!(
            e.requests.len(),
            4,
            "the first asks, three carry a password"
        );
        assert_eq!(e.stdin, ["d4"], "the fourth line stays unread");
    }
}
