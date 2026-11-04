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
    /// Run argv with stdout and stderr captured: (exit status, stdout, stderr). None when it
    /// could not run. For a tool whose non-zero exit is an answer (`pacman -Qu` exits 1 when
    /// nothing is pending). Where a build has no such child, `capture`'s answer stands (0).
    fn capture_status(&mut self, argv: &[&str]) -> Option<(i32, String, String)> {
        self.capture(argv).map(|o| (0, o, String::new()))
    }
    fn stdin_line(&mut self) -> Option<String>;
    fn read_file(&self, p: &Path) -> Option<String>;
    /// The account's "Give me more time" setting (`$XDG_CONFIG_HOME/wol/timeouts`). The real
    /// environment opens the file without following a link or waiting on a pipe, so a FIFO put
    /// there cannot hang `wol`; the fake reads it from its files.
    fn patience(&self) -> shared::patience::Patience {
        self.read_file(&self.config_home().join(shared::patience::FILE))
            .map_or(shared::patience::Patience::Normal, |t| {
                shared::patience::Patience::parse(&t)
            })
    }
    /// Whether something is at `p` (a file that exists but cannot be read is not absent).
    fn file_exists(&self, p: &Path) -> bool {
        std::fs::symlink_metadata(p).is_ok()
    }
    fn sys_topic(&mut self, topic: &str) -> Option<serde_json::Value>;
    fn now_secs(&self) -> u64;
    fn config_home(&self) -> PathBuf;
    /// Like `exec`, but the child gets no stdin and its stdout goes to our stderr (`wol mcp`:
    /// stdout carries the protocol).
    fn exec_quiet(&mut self, argv: &[String]) -> io::Result<i32> {
        self.exec(argv)
    }
    /// Starts argv and does not wait for what it opens: no stdin, stdout or stderr shared with
    /// us, its own process group, and the launcher itself is waited for only briefly (`wol open`
    /// returns once the app is asked for; QA-PLUG-23: it held the caller's pipes until the app
    /// quit, which blocked `su -l bob -c 'wol open ...'` for minutes).
    fn exec_detached(&mut self, argv: &[String]) -> io::Result<i32> {
        self.exec(argv)
    }
    /// Run argv with no stdin and stdout captured; (exit status, stdout). Unlike `capture`, a
    /// non-zero exit keeps its output (`wol shard check --json` reports findings and exits 1).
    /// Refused where there is no child to run.
    fn exec_output(&mut self, argv: &[String]) -> io::Result<(i32, String)> {
        let _ = argv;
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
    /// Like `exec`, with one secret in the child's environment (`$PASSWORD`, `$NEWPASSWORD` for
    /// systemd-cryptenroll), never in its arguments. Refused where there is no child to give it to.
    fn exec_secret(&mut self, argv: &[String], var: &str, secret: &str) -> io::Result<i32> {
        let _ = (argv, var, secret);
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
    /// Like `exec`, with `input` on the child's stdin (`wol script on` sends the script to ssh).
    fn exec_input(&mut self, argv: &[String], input: &str) -> io::Result<i32> {
        let _ = (argv, input);
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
    /// The administrator password: on a terminal "Password: " and no echo (`secret`); otherwise
    /// one line of stdin, as [`Env::stdin_line`].
    fn read_password(&mut self) -> Option<String> {
        self.stdin_line()
    }
    /// One raw line from stdin; None at end of input (`wol mcp`).
    fn read_line(&mut self) -> Option<String> {
        // At most MAX_LINE bytes are kept (QA-PLUG-59): the rest of a longer line is read and
        // dropped, and the caller gets the TOO_LONG marker instead of 3 MB of text.
        use std::io::BufRead;
        let stdin = io::stdin();
        let mut stdin = stdin.lock();
        let mut l: Vec<u8> = Vec::new();
        let mut too_long = false;
        loop {
            let (used, done) = match stdin.fill_buf() {
                Ok([]) | Err(_) => break,
                Ok(buf) => {
                    let (chunk, done) = match buf.iter().position(|b| *b == b'\n') {
                        Some(i) => (&buf[..=i], true),
                        None => (buf, false),
                    };
                    if l.len() + chunk.len() > crate::mcp::MAX_LINE {
                        too_long = true;
                    } else if !too_long {
                        l.extend_from_slice(chunk);
                    }
                    (chunk.len(), done)
                }
            };
            stdin.consume(used);
            if done {
                break;
            }
        }
        if too_long {
            return Some(crate::mcp::TOO_LONG.to_owned());
        }
        if l.is_empty() {
            return None;
        }
        Some(String::from_utf8_lossy(&l).into_owned())
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
    /// Take the exclusive lock file `path` without waiting (`flock`, released when the [`Lock`]
    /// drops or the process ends): `Ok(None)` while another run holds it (`wol update apply`, the
    /// pacman wrapper; audit8 stability2 #2).
    fn try_lock(&mut self, path: &Path) -> io::Result<Option<Lock>> {
        flock_file(path)
    }
    /// Whether another process holds `path` (root's 0600 lock file) now: asked of `/proc/locks`,
    /// so it takes nothing and needs no permission on the file (`shared::sys::flock_held_by`).
    fn lock_held(&self, path: &Path) -> bool {
        shared::sys::flock_held_by(path, 0)
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
    /// The helper name this process acts under, if any (QA-PLUG-6): `WOL_HELPER`, else
    /// [`AGENT_HELPER`] when the environment says an assistant's shell started it
    /// (`CLAUDECODE=1`, which Claude Code sets for its Bash tool). A person's own shell has
    /// neither, and is `cli`.
    fn helper_name(&self) -> Option<String> {
        self.var("WOL_HELPER")
            .or_else(|| (self.var(AGENT_ENV).as_deref() == Some("1")).then(|| AGENT_HELPER.into()))
    }
    /// Who acts (journal, audit log): `cli`, `script:<helper name>`, `mcp:<client>`.
    fn actor(&self) -> String {
        self.helper_name()
            .map_or_else(|| "cli".into(), |h| format!("script:{h}"))
    }
}

/// An exclusive lock from [`Env::try_lock`]; dropping it releases the lock.
pub struct Lock {
    /// Holds the locked file (or a fake): only its drop matters.
    _held: Box<dyn std::any::Any>,
}

/// [`Env::try_lock`] on the real file system: `shared::sys::try_flock` (0600, no symlink,
/// `flock(LOCK_EX | LOCK_NB)`), the one implementation wol-admind holds too.
fn flock_file(path: &Path) -> io::Result<Option<Lock>> {
    Ok(shared::sys::try_flock(path)?.map(|f| Lock { _held: Box::new(f) }))
}

/// Take [`wol_core::update::LOCK`] for one change to a slot or a layer, or say why not: only one
/// such change runs at a time (audit8 stability2 #2: a second `wol update apply` or pacman would
/// delete or snapshot over the slot the first one is building). Err = the exit code to return.
pub(crate) fn update_lock(env: &mut dyn Env) -> Result<Lock, i32> {
    match env.try_lock(Path::new(wol_core::update::LOCK)) {
        Ok(Some(l)) => Ok(l),
        Ok(None) => Err(fail(env, ErrorCode::Unavailable, wol_core::update::BUSY)),
        Err(e) => Err(fail(
            env,
            ErrorCode::Failed,
            &format!("{}: {e}", wol_core::update::LOCK),
        )),
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
    // A verb that is not a helper verb is for people: under a helper name it is refused, not run
    // (fail closed; `plugin allow-host`, `plugin enable` and the like are never a helper's).
    if let Some(name) = env.helper_name() {
        match crate::catalog::rule_of(inv) {
            Some(rule) => return crate::verbs::as_script(inv, env, &name, rule),
            None if !HELPER_OPEN.contains(&inv.action) => {
                return crate::verbs::refuse_people_verb(inv, env, &name);
            }
            None => {}
        }
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
            // Words that are no command are a question: the commands and manual sections that
            // mean it (docs/modules/search.md), on the device.
            if !path.is_empty() && !crate::helpfind::names_a_command(&path) {
                let found = crate::helpfind::find(&path, wolsem::installed());
                env.out(&crate::helpfind::text(&path, &found));
                return if found.is_empty() {
                    ErrorCode::NotFound.exit()
                } else {
                    0
                };
            }
            env.out(&help(&path));
            0
        }
        "version" => {
            env.out(&format!("wol {}\n", env!("CARGO_PKG_VERSION")));
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
        "theme.current" | "theme.list" | "theme.set" | "theme.apply" | "theme.switch"
        | "theme.css" => theme(env, inv, json),
        "shell.toggle" | "shell.show" | "shell.hide" | "shell.reload" => shell(env, inv),
        "shell.call" | "shell.status" => shell_ask(env, inv, json),
        "update.check" => update_check(env, json),
        "update.status" => update_status(env, json),
        "update.channel" => update_channel(env, json),
        "base.verify" => base_verify(env, json),
        "update.plan" | "update.apply" => update(env, inv),
        "update.chroot" => update_chroot(env, inv),
        "update.install" => update_install(env, json),
        "webapp.call" => crate::webapp::run(inv, env),
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
        "layers.list" => crate::pkg::list(env, inv),
        "layers.fold" => crate::pkg::fold(env, inv),
        "layers.clear" => crate::pkg::clear(env),
        "pkg" => crate::pkg::pkg_verb(env, inv),
        "hint" => crate::hint::run(env, inv),
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
        "hw.list" | "hw.status" | "hw.fix" | "hw.boot" | "hw.online" | "hw.compute"
        | "hw.report" => crate::hw::execute(inv, env),
        "audio.doctor" | "audio.reset" | "audio.appmix" | "audio.profile" => {
            crate::audio_doctor::execute(inv, env)
        }
        "doctor.check" | "doctor.fix" => crate::doctor::execute(inv, env),
        a if a.starts_with("games.") => crate::games::execute(inv, env),
        a if a.starts_with("modules.") => crate::modules::execute(inv, env),
        a if a.starts_with("sessions.") => crate::sessions::execute(inv, env),
        a if a.starts_with("plugin.") => crate::plugin::execute(inv, env),
        a if a.starts_with("shard.") => crate::shard::execute(inv, env),
        a if a.starts_with("app.") => crate::app::execute(inv, env),
        a if a.starts_with("script.") => crate::script::execute(inv, env),
        a if a.starts_with("power.") => crate::power::execute(inv, env),
        a if a.starts_with("firewall.") || a.starts_with("security.") => {
            crate::security::execute(inv, env)
        }
        "package.install" | "package.remove" | "package.list" | "package.search"
        | "package.updates" => crate::packages::execute(inv, env),
        "widgets.list" | "widgets.add" | "widgets.remove" | "widgets.reload"
        | "widgets.validate" => crate::widgets::execute(inv, env),
        "network.ip" => crate::netverbs::execute(inv, env),
        a if ["vpn.", "proxy.", "dns.", "airplane."]
            .iter()
            .any(|p| a.starts_with(p)) =>
        {
            crate::netverbs::execute(inv, env)
        }
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
                // wold refuses a root-only request from anyone else before it reaches a job
                // (api/control.rs `handle`): that is the refusal code. Any other error text, such as
                // "permission denied: /var/lib/…" from a job, is a failure, not a refusal.
                let code = if req.needs_root() && !env.is_root() {
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
    admin_send_said(env, req, &mut String::new())
}

/// [`admin_send`], and the sentence it printed for the person when the request did not go through
/// is also put in `said_out` (the Doctor's report carries it: a fix that fails says why, never
/// only an exit status).
pub(crate) fn admin_send_said(env: &mut dyn Env, req: &AdminRequest, said_out: &mut String) -> i32 {
    // The message goes to stderr as `wol: <msg>` and into `said_out`.
    fn stop(env: &mut dyn Env, said_out: &mut String, code: ErrorCode, msg: &str) -> i32 {
        *said_out = msg.to_string();
        fail(env, code, msg)
    }
    let sock = admin_socket(env);
    let mut password: Vec<u8> = Vec::new(); // empty = use cached authorization
    for attempt in 0..=PASSWORD_TRIES {
        // As long as wol-admind may take for this request, every planned step (S4-4/S4-18).
        let secs = wol_core::admin::reply_secs(req);
        let sent = env.request_wait(&sock, &wire::encode(req, &password), secs);
        crate::authcheck::wipe(&mut password);
        let said = sent.as_ref().ok().and_then(|r| wire::failure_message(r));
        let reply = match sent {
            Ok(r) => AdminReply::parse(&r),
            // A slow action (a package, a boot entry) is still running in wol-admind (S4-18).
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                return stop(
                    env,
                    said_out,
                    ErrorCode::Failed,
                    "wol-admind is still working on it; check again later",
                );
            }
            Err(_) => {
                return stop(
                    env,
                    said_out,
                    ErrorCode::Unavailable,
                    "administrator service unavailable (wol-admind)",
                );
            }
        };
        match reply {
            AdminReply::NeedsAuth | AdminReply::Denied => {
                if reply == AdminReply::Denied {
                    env.err("wol admin: incorrect password\n");
                    *said_out = "incorrect password".to_string();
                }
                // The last try does not ask for a password nobody will check (S4-18).
                if attempt == PASSWORD_TRIES {
                    break;
                }
                match env.read_password() {
                    Some(p) if !p.is_empty() => password = p.into_bytes(),
                    _ => return stop(env, said_out, ErrorCode::Denied, reply.message()),
                }
            }
            AdminReply::Ok => return 0,
            r => {
                // wol-admind's own sentence when it gave one (the program's last line, QA-100 LIVE3-19).
                let why = said.as_deref().unwrap_or_else(|| r.message());
                env.err(&format!("wol admin: {why}\n"));
                *said_out = why.to_string();
                return admin_exit(r);
            }
        }
    }
    stop(
        env,
        said_out,
        ErrorCode::Denied,
        AdminReply::Denied.message(),
    )
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
            // Piped, a list is JSON (QA-PWR-16: a script got the text table).
            if json || !env.stdout_tty() {
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
        // Session start (exec-once): the toolkits follow the theme in use and nothing is saved.
        // No theme file means Auto; writing the shown one ended Auto and journaled a change the
        // person never made (QA-SET-63, SET-R2-10).
        "theme.apply" => match catalog.resolve(&current) {
            Ok(t) => {
                restyle_apps(env, &t);
                env.out(&format!("{current}\n"));
                0
            }
            Err(e) => fail(
                env,
                ErrorCode::NotFound,
                &format!("theme '{current}': {e:?}"),
            ),
        },
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
/// running GTK apps switch live (settings portal), running foot terminals by signal
/// (`apps::terminal_argv`), then the `theme` event on wold for libwol apps
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
    // foot rereads nothing while it runs: its two colour sections are switched by signal.
    let foot = apps::foot_is_ours(&apps::user_dir(&home), &home)
        .then(|| apps::terminal_argv(t, shared::sys::euid()));
    for argv in apps::gsettings_argv(t) {
        let _ = env.exec(&argv);
    }
    restyle_compositor(env, &apps::strip_argv(t));
    if let Some(argv) = foot {
        let _ = env.exec(&argv);
    }
    announce_theme(env, &t.id);
}

/// The upgrade to the compositor's Lua configuration (WolOS 0.13): the first restyle (`wol theme
/// apply`, at every session start) carries the 0.56 `.conf` files over (`hyprprefs::retire_plan`) and
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

/// The compositor's colours, live. hyprctl answers each keyword on stdout, "ok" when it took it
/// and its error otherwise: the "ok" lines are dropped (`wol appearance set` printed ten of them),
/// anything else goes to stderr. A hyprctl that fails to run (no compositor: a tty, ssh, a
/// chroot) is said once and the other keywords are not tried.
fn restyle_compositor(env: &mut dyn Env, argvs: &[Vec<String>]) {
    for argv in argvs {
        let a: Vec<&str> = argv.iter().map(String::as_str).collect();
        let Some(out) = env.capture(&a) else {
            env.err("wol: the window title bars keep their colours until the desktop restarts (hyprctl did not answer)\n");
            return;
        };
        for line in out
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && *l != "ok")
        {
            env.err(&format!("wol: {}: {line}\n", argv[2..].join(" ")));
        }
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

/// `wol shell call <plugin> <fn> [json...]` and `wol shell status`: one line to the shell, one
/// JSON line back (docs/modules/plugins.md "IPC handlers"). `call` prints the reply line as is
/// and exits 1 when it says `"ok":false`; `status` prints JSON (`--json` or no TTY) or one line
/// per panel and surface.
fn shell_ask(env: &mut dyn Env, inv: &Invocation, json: bool) -> i32 {
    let line = if inv.action == "shell.call" {
        let raw = inv.all("json").join(" ");
        let args = if raw.trim().is_empty() {
            serde_json::Value::Null
        } else {
            match serde_json::from_str::<serde_json::Value>(&raw) {
                Ok(v) => v,
                Err(_) => {
                    return fail(
                        env,
                        ErrorCode::Usage,
                        "the argument is one JSON value, like 40, '\"text\"' or '{\"level\": 40}'",
                    );
                }
            }
        };
        format!(
            "call {} {} {args}\n",
            inv.get("plugin").unwrap_or_default(),
            inv.get("function").unwrap_or_default()
        )
    } else {
        "status\n".to_owned()
    };
    let Some(rt) = env.var("XDG_RUNTIME_DIR") else {
        return fail(
            env,
            ErrorCode::Unavailable,
            "XDG_RUNTIME_DIR unset (no session)",
        );
    };
    let sock = Path::new(&rt).join(SHELL_SOCKET);
    let reply = match env.request(&sock, line.as_bytes()) {
        Ok(r) => r,
        Err(e) => {
            return fail(
                env,
                ErrorCode::Unavailable,
                &format!("shell not running ({e})"),
            );
        }
    };
    let reply = reply.lines().next().unwrap_or_default().trim().to_owned();
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&reply) else {
        // An older shell answers nothing to lines it does not know.
        return fail(
            env,
            ErrorCode::Unavailable,
            "the shell did not answer (is it older than this wol?)",
        );
    };
    let ok = v["ok"].as_bool() == Some(true);
    if inv.action == "shell.status" && ok && !json && env.stdout_tty() {
        let mut out = String::new();
        for p in v["panels"].as_array().into_iter().flatten() {
            let open = if p["open"].as_bool() == Some(true) {
                "open"
            } else {
                "closed"
            };
            out.push_str(&format!(
                "panel    {:<24} {open}\n",
                p["name"].as_str().unwrap_or_default()
            ));
        }
        for s in v["surfaces"].as_array().into_iter().flatten() {
            let shown = if s["visible"].as_bool() == Some(true) {
                "visible"
            } else {
                "hidden"
            };
            out.push_str(&format!(
                "surface  {:<24} {shown}\n",
                s["namespace"].as_str().unwrap_or_default()
            ));
        }
        for p in v["stopped"].as_array().into_iter().flatten() {
            out.push_str(&format!("stopped  {}\n", p.as_str().unwrap_or_default()));
        }
        env.out(&out);
        return 0;
    }
    env.out(&format!("{reply}\n"));
    if ok { 0 } else { ErrorCode::Failed.exit() }
}

/// Run the update check now through wol-admind (no password: the person at the computer) so
/// "up to date" is never read from a stale catalogue.
fn refresh_catalogue(env: &mut dyn Env) -> bool {
    let sock = env
        .var("WOL_ADMIN_SOCKET")
        .map_or_else(|| wire::SOCKET.into(), PathBuf::from);
    let req = AdminRequest {
        action: AdminAction::PkgRefresh,
        args: vec![],
    };
    // QA-STORE-14: wol-admind's own budget for the refresh (its reply wait), not a number of
    // our own: the Settings button waits `check::CHECK_SECS`, which is this plus the rest.
    env.request_wait(
        &sock,
        &wire::encode(&req, b""),
        wol_core::admin::reply_secs(&req),
    )
    .is_ok_and(|r| AdminReply::parse(&r) == AdminReply::Ok)
}

/// How long `pacman -Qu` may take before `wol update check` stops waiting for it.
const PACMAN_QU_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

/// Run `argv` with stdin closed: (exit status, stdout, stderr). `wol_core::system::KILLED` when a signal ended it;
/// None when it cannot start or does not finish within `timeout` (it is killed): a hung
/// `pacman -Qu` must not hang `wol update check`. One runner for every crate:
/// [`wol_core::system::run_full`].
pub(crate) fn run_status(
    argv: &[&str],
    timeout: std::time::Duration,
) -> Option<(i32, String, String)> {
    wol_core::system::run_full(argv, None, timeout).ok()
}

/// Free bytes where the check writes (`shared::update::CHECK_DISK`); None when unreadable.
fn var_free(env: &mut dyn Env) -> Option<u64> {
    shared::update::free_from_df(&env.capture(&shared::update::FREE_ARGV)?)
}

/// `1 update available: a` / `3 updates available: a, b, c`, with "WolOS 2026.12" among them as
/// the release it is (QA-STORE-15).
pub(crate) fn update_line(pending: &[String]) -> String {
    let n = pending.len();
    let what = if n == 1 { "update" } else { "updates" };
    format!("{n} {what} available: {}", pending.join(", "))
}

/// Pending updates: the check's own findings, never `pacman -Qu` on the running system. Its
/// database sits on the read-only seal (ADR 0016) and is never refreshed there, and `-Qu` exits 1
/// when nothing is pending (e2e D3: "pacman -Qu failed" on every installed system). wol-admind
/// runs the check now (`update-check now`, a temporary database); a stamp from before this request
/// means it could not refresh (offline).
fn update_check(env: &mut dyn Env, json: bool) -> i32 {
    let asked = env.now_secs();
    let refreshed = refresh_catalogue(env);
    let found = update_snapshot(env);
    let refreshed = refreshed && found.as_ref().is_some_and(|u| u.checked >= asked);
    let mut pending = found
        .as_ref()
        .map(|u| u.packages.clone())
        .unwrap_or_default();
    // A newer release the check found, until installed (`wol update apply`).
    if let Some(b) = found.and_then(|u| u.base) {
        pending.push(format!("WolOS {b}"));
    }
    if pending.is_empty() && !refreshed {
        // Never claim "up to date" from a catalogue that could not be refreshed. A full disk
        // is the cause worth naming (QA-FDAY F10): "are you online?" sends people to the router.
        if shared::update::no_room(var_free(env)) {
            return fail(
                env,
                ErrorCode::Unavailable,
                "there is no room left on the system disk (/var) to check for updates; free some space in Settings > Storage",
            );
        }
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
        env.out(&format!("{}\n", update_line(&pending)));
    }
    0
}

/// What `wol update install --json` reports: `applied` = the package updates the daily check had
/// listed (what Update Now installs), `restart` = one of them is a kernel.
/// `release` = the WolOS release that comes with it (the daily check's signed manifest, newer
/// than the running one; QA-STORE-2), null when none (frozen for the Store, C1).
pub(crate) fn update_install_result(env: &mut dyn Env) -> serde_json::Value {
    let found = update_snapshot(env);
    let release = found.as_ref().and_then(|u| u.base.clone());
    let pending = found.map(|u| u.packages).unwrap_or_default();
    let restart = pending
        .iter()
        .any(|p| p == "linux" || p.starts_with("linux-"));
    serde_json::json!({ "applied": pending.len(), "restart": restart, "release": release })
}

/// `wol update install`: every waiting package update through wol-admind (`pkg-upgrade`, the
/// request the shell's Update Now sends after its sheet), with the password like `wol admin`.
fn update_install(env: &mut dyn Env, json: bool) -> i32 {
    let result = update_install_result(env);
    let req = AdminRequest {
        action: AdminAction::PkgUpgrade,
        args: Vec::new(),
    };
    let code = admin_send(env, &req);
    if code != 0 {
        return code;
    }
    if json {
        emit_json(env, &result);
    } else {
        // QA-STORE-13: a computer with two root slots built the update in the other one: nothing
        // applies until the restart. One copy (ADR 0023) was updated in place.
        let one_copy = env
            .read_file(Path::new(wol_core::update::SINGLE_COPY))
            .is_some();
        let n = result["applied"].as_u64().unwrap_or(0);
        env.out(&installed_line(n, one_copy, result["restart"] == true));
    }
    0
}

/// What `wol update install` says when it is done.
pub fn installed_line(n: u64, one_copy: bool, kernel: bool) -> String {
    let what = if n == 1 { "update" } else { "updates" };
    let tail = match (one_copy, kernel) {
        (false, _) => " Restart to finish.",
        (true, true) => " Restart to use the new kernel.",
        (true, false) => "",
    };
    format!("Installed {n} {what}.{tail}\n")
}

/// The daily check's findings (wol_core::update::check), without a release installed since.
pub(crate) fn update_snapshot(env: &mut dyn Env) -> Option<wol_core::update::check::Updates> {
    use wol_core::update::check::{DIR, USERLAND_ARGV, Updates};
    let keys = wol_core::package::read_keys(Path::new(
        &env.var("WOL_KEYS_DIR")
            .unwrap_or_else(|| shared::package::KEYS_DIR.into()),
    ));
    let now = env.now_secs();
    let f = |n: &str| env.read_file(&Path::new(DIR).join(n)).unwrap_or_default();
    let u = Updates::from_files_at(f, &keys, now)?;
    // No release file in the image (v0.12.0): os-release names the running release.
    let userland = u.base.as_ref().and_then(|_| {
        env.capture(&USERLAND_ARGV)
            .or_else(|| env.read_file(Path::new(wol_core::update::check::OS_RELEASE)))
    });
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

/// Findings `wol base verify` prints as text and in `--json`; the totals are always complete.
const BASE_FINDINGS_TEXT: usize = 20;
const BASE_FINDINGS_JSON: usize = 500;

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
    // QA-LIVE3-25: 28 000 findings were 1.25 MB on one line. The count of each kind is always
    // there; the lines are the first few.
    let total = findings.len();
    let count = |state: &str| findings.iter().filter(|f| f.state == state).count();
    if json || !env.stdout_tty() {
        env.out(&format!(
            "{}\n",
            serde_json::json!({
                "files": entries.len(),
                "findings": findings.iter().take(BASE_FINDINGS_JSON).collect::<Vec<_>>(),
                "findings_total": total,
                "changed": count("changed"),
                "missing": count("missing"),
                "truncated": total > BASE_FINDINGS_JSON,
            })
        ));
    } else if findings.is_empty() {
        env.out(&format!("{} base files intact\n", entries.len()));
    } else {
        env.out(&format!(
            "{} of {} base files differ: {} changed, {} missing\n",
            total,
            entries.len(),
            count("changed"),
            count("missing")
        ));
        for f in findings.iter().take(BASE_FINDINGS_TEXT) {
            env.out(&format!("{} {}\n", f.state, f.path));
        }
        if total > BASE_FINDINGS_TEXT {
            env.out(&format!(
                "... and {} more; `wol base verify --json` lists up to {BASE_FINDINGS_JSON}\n",
                total - BASE_FINDINGS_TEXT
            ));
        }
    }
    if findings.is_empty() {
        0
    } else {
        ErrorCode::Failed.exit()
    }
}

/// The slot an update was building when it was cut off (`wol_core::update::interrupted`): its
/// mark is there and no build holds the lock (asked of `/proc/locks`: any account, and the lock
/// is never taken, not even for a moment). None while a build runs.
fn update_interrupted(env: &mut dyn Env) -> Option<String> {
    use wol_core::update as up;
    let mark = env.read_file(Path::new(up::BUILDING))?;
    up::interrupted(Some(&mark), env.lock_held(Path::new(up::LOCK))).map(str::to_owned)
}

/// `wol update status`: `{"checked":null,..}` / "not checked yet" before the first check.
fn update_status(env: &mut dyn Env, json: bool) -> i32 {
    let u = update_snapshot(env);
    let (base, kernel, wolos) = running_base(env);
    // QA-LIVE4-14: an update's mark with no build holding the lock is one that was cut off.
    let interrupted = update_interrupted(env);
    if json || !env.stdout_tty() {
        let v = u.map(|u| {
            let mut v = serde_json::json!(u);
            v["interrupted"] = interrupted.clone().into();
            v["count"] = u.count().into();
            v["base_version"] = base.clone().into();
            v["kernel"] = kernel.clone().into();
            v["kernel_is_wolos"] = wolos.into();
            v
        });
        // No check yet is an object, not `null`: a script can read `checked` either way.
        let none = serde_json::json!({
            "checked": null,
            "count": 0,
            "interrupted": interrupted,
            "base_version": base,
            "kernel": kernel,
            "kernel_is_wolos": wolos,
        });
        env.out(&format!("{}\n", v.unwrap_or(none)));
        return 0;
    }
    if interrupted.is_some() {
        env.out("The last update was interrupted before it finished; nothing of it is used. Update again to finish it.\n");
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

/// `wol update chroot ROOT -- CMD...`: one command of a slot build in the copy at ROOT. The slot
/// build runs it as `unshare --mount --propagation private wol update chroot ...`
/// (`wol_core::update::in_copy`), so every mount made here (the copy bound onto itself, which
/// pacman needs to find its root's file system; the shared package cache, QA-STORE-46; and
/// arch-chroot's own) is gone when it exits, even when it is killed or the power goes (RH-192).
fn update_chroot(env: &mut dyn Env, inv: &Invocation) -> i32 {
    use wol_core::update as up;
    let (Some(root), cmd) = (inv.get("root"), inv.all("args")) else {
        return fail(
            env,
            ErrorCode::Usage,
            "usage: wol update chroot ROOT -- COMMAND...",
        );
    };
    // Only the scratch copy a slot build makes: never a running slot, no other path.
    let scratch = format!("{}/@{}", up::TOP, up::SCRATCH);
    if cmd.is_empty() || root != scratch {
        return fail(
            env,
            ErrorCode::Usage,
            "usage: wol update chroot ROOT -- COMMAND... (ROOT is the slot build's copy)",
        );
    }
    if !env.is_root() {
        return fail(env, ErrorCode::Denied, "update chroot needs root");
    }
    // The copy itself, not a link to somewhere else.
    if env
        .capture(&["realpath", "-e", root])
        .as_deref()
        .map(str::trim)
        != Some(root)
    {
        return fail(
            env,
            ErrorCode::Failed,
            &format!("{root} is not the slot build's copy"),
        );
    }
    // Only in a mount namespace of its own (the build starts it under `unshare`): the binds
    // below must never reach the rest of the system (RH-192).
    let mut ns = |p: &str| env.capture(&["readlink", p]).map(|t| t.trim().to_owned());
    let (own, init) = (ns("/proc/self/ns/mnt"), ns("/proc/1/ns/mnt"));
    if own.is_none() || own == init {
        return fail(
            env,
            ErrorCode::Failed,
            "update chroot runs only in a private mount namespace (unshare --mount)",
        );
    }
    let cache = format!("{root}{}", up::CACHE);
    let s = |a: &[&str]| a.iter().map(|x| (*x).to_owned()).collect::<Vec<String>>();
    for a in [
        s(&["mount", "--bind", root, root]),
        s(&["mkdir", "-p", &cache]),
        s(&["mount", "--bind", up::CACHE, &cache]),
    ] {
        if env.exec(&a).unwrap_or(127) != 0 {
            return fail(
                env,
                ErrorCode::Failed,
                &format!("could not prepare the copy at {root}"),
            );
        }
    }
    let mut a = s(&["arch-chroot", root]);
    a.extend(cmd.iter().map(|c| (*c).to_owned()));
    env.exec(&a).unwrap_or(127)
}

/// `wol update channel`: the channel this computer follows (`/etc/wolos/channel`, read as one
/// this release offers: stable in 0.13; QA-STORE-9/68), and the ones it could follow.
fn update_channel(env: &mut dyn Env, json: bool) -> i32 {
    let text = env.read_file(Path::new(wol_core::admin::CHANNEL_FILE));
    let ch = shared::packages::channel_of(text.as_deref());
    if json {
        emit_json(
            env,
            &serde_json::json!({ "channel": ch, "offered": shared::packages::OFFERED }),
        );
    } else {
        env.out(&format!("{ch}\n"));
    }
    0
}

/// One step of `wol update plan` as it reads: an empty argument shows as `''` (QA-STORE-76:
/// the plan ended in a bare `bootctl set-default `).
fn plan_line(s: &wol_core::update::Step) -> String {
    match s {
        wol_core::update::Step::Run(a) => a
            .iter()
            .map(|x| if x.is_empty() { "''" } else { x.as_str() })
            .collect::<Vec<_>>()
            .join(" "),
        w => w.to_string(),
    }
}

/// The program a failed step ran, for the one line that says where it stopped.
fn step_name(s: &wol_core::update::Step) -> String {
    match s {
        wol_core::update::Step::Run(a) => {
            let i = usize::from(a.first().is_some_and(|x| x == "env" || x == "find"));
            let w = a
                .iter()
                .skip(i)
                .find(|x| !x.contains('='))
                .map_or("", String::as_str);
            Path::new(w)
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        }
        wol_core::update::Step::Write { .. } => "a file write".into(),
    }
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
        up::plan_from(&slot, from, date.as_deref(), cmdline.as_deref(), &[])
    };
    let Some(steps) = planned else {
        return fail(
            env,
            ErrorCode::Usage,
            &format!("bad slot '{slot}' or release date"),
        );
    };
    if inv.action == "update.plan" {
        env.out(
            &steps
                .iter()
                .map(|s| format!("{}\n", plan_line(s)))
                .collect::<String>(),
        );
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
    let _lock = match update_lock(env) {
        Ok(l) => l,
        Err(code) => return code,
    };
    // QA-STORE-55: the person reads what happens, not the commands (`wol update plan` lists them).
    env.err(&if one_copy {
        "wol update: updating this computer's only copy of WolOS in place\n".to_owned()
    } else {
        format!("wol update: building the update in {slot}; this takes a few minutes\n")
    });
    for (i, s) in steps.iter().enumerate() {
        let code = match s {
            Step::Run(argv) => env.exec(argv).unwrap_or(127),
            Step::Write { path, text } => i32::from(env.write_file(Path::new(path), text).is_err()),
        };
        if code == 0 {
            continue;
        }
        let what = step_name(s);
        // QA-STORE-50: one copy has no slot to roll back: nothing is deleted, the sentence says
        // what may already be installed.
        if one_copy {
            return fail(
                env,
                ErrorCode::Failed,
                &format!(
                    "{} ({what} stopped with {code})",
                    wol_core::update::pkg::IN_PLACE_FAILED
                ),
            );
        }
        // A failure after the swap no longer has the slot as it was (audit8 #4, QA-STORE-3).
        let undo = up::rollback_at(&slot, from, &steps, i);
        for r in &undo.argv {
            let _ = env.exec(r);
        }
        if undo.done {
            env.err(&format!(
                "wol update: {}\n",
                wol_core::update::pkg::INSTALLED_NOT_DEFAULT
            ));
            env.out(&format!("updated into {slot}; restart to use it\n"));
            return 0;
        }
        let said = if undo.kept {
            wol_core::update::pkg::SLOT_FAILED.to_owned()
        } else {
            format!(
                "The update did not finish and {slot} could not be kept, so it was removed. The \
                 running system is as it was."
            )
        };
        return fail(
            env,
            ErrorCode::Failed,
            &format!("{said} ({what} stopped with {code})"),
        );
    }
    env.out(&format!("updated into {slot}; restart to use it\n"));
    0
}

/// Verbs a helper may run although they are not helper verbs: they only describe `wol` (`help`,
/// `api`) or are the server an assistant starts (`mcp`, which gates each tool call itself).
const HELPER_OPEN: [&str; 3] = ["help", "api", "mcp"];

/// The variable Claude Code sets in the shell its Bash tool starts: such a `wol` is an assistant's.
pub const AGENT_ENV: &str = "CLAUDECODE";
/// The helper name an assistant's shell acts under (policy, sheet and audit log).
pub const AGENT_HELPER: &str = "claude-code";

/// Real side effects.
pub struct RealEnv;

/// How long `wol open` waits for the launcher to say it started the app, and how often it looks.
const LAUNCH_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
const LAUNCH_POLL: std::time::Duration = std::time::Duration::from_millis(20);
/// How often a wait on a socket looks whether the pinned client is still there.
const CLIENT_POLL: std::time::Duration = std::time::Duration::from_millis(200);
/// Why a wait ended early: the program that started `wol mcp` is gone (QA-PLUG-62).
pub const CLIENT_GONE: &str = "the program that asked is gone";
/// The parent pid `wol mcp` was started by (0 = not pinned).
static CLIENT_PARENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// `wol mcp`: remember the program that started this process, so a wait for an on-screen answer
/// ends once it is gone (its sheet then closes, and nothing runs for nobody).
pub fn pin_client() {
    CLIENT_PARENT.store(
        std::os::unix::process::parent_id(),
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// True when [`pin_client`] ran and this process has a different parent now (the client exited
/// and the process was handed to init or a subreaper).
pub fn client_gone() -> bool {
    let p = CLIENT_PARENT.load(std::sync::atomic::Ordering::Relaxed);
    p != 0 && std::os::unix::process::parent_id() != p
}

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
        s.write_all(req)?;
        s.shutdown(std::net::Shutdown::Write)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs.into());
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            // A short read timeout, so a pinned client that went away is noticed while waiting
            // (QA-PLUG-62); dropping `s` then closes the request on the other side.
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err(io::Error::from(io::ErrorKind::TimedOut));
            }
            s.set_read_timeout(Some(left.min(CLIENT_POLL)))?;
            match s.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    if client_gone() {
                        return Err(io::Error::new(
                            io::ErrorKind::ConnectionAborted,
                            CLIENT_GONE,
                        ));
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        String::from_utf8(out).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
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
    fn exec_input(&mut self, argv: &[String], input: &str) -> io::Result<i32> {
        use io::Write;
        let mut child = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            // A child that exits early closes the pipe; its exit code says what happened.
            let _ = stdin.write_all(input.as_bytes());
        }
        Ok(child.wait()?.code().unwrap_or(1))
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
    fn exec_detached(&mut self, argv: &[String]) -> io::Result<i32> {
        use std::os::unix::process::CommandExt;
        use std::process::Stdio;
        let mut child = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        let until = std::time::Instant::now() + LAUNCH_WAIT;
        loop {
            if let Some(st) = child.try_wait()? {
                return Ok(st.code().unwrap_or(1));
            }
            if std::time::Instant::now() >= until {
                // Still running: the launcher is the app (xdg-open handing a file to its viewer).
                return Ok(0);
            }
            std::thread::sleep(LAUNCH_POLL);
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
    fn exec_output(&mut self, argv: &[String]) -> io::Result<(i32, String)> {
        let out = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .output()?;
        Ok((
            out.status.code().unwrap_or(1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        ))
    }
    fn capture(&mut self, argv: &[&str]) -> Option<String> {
        wol_core::system::run_with_timeout(argv, std::time::Duration::from_secs(60))
    }
    fn capture_status(&mut self, argv: &[&str]) -> Option<(i32, String, String)> {
        run_status(argv, PACMAN_QU_WAIT)
    }
    fn stdin_line(&mut self) -> Option<String> {
        let mut l = String::new();
        io::stdin().read_line(&mut l).ok()?;
        Some(l.trim_end_matches(['\n', '\r']).to_owned())
    }
    fn read_password(&mut self) -> Option<String> {
        use std::os::fd::AsRawFd;
        let stdin = io::stdin();
        let fd = stdin.as_raw_fd();
        crate::secret::read_password(&mut stdin.lock(), &mut io::stderr(), fd)
    }
    fn read_file(&self, p: &Path) -> Option<String> {
        std::fs::read_to_string(p).ok()
    }
    fn patience(&self) -> shared::patience::Patience {
        use std::os::unix::fs::MetadataExt;
        // The owner of /proc/self is the process's effective user: the file must be that account's.
        let Ok(me) = std::fs::metadata("/proc/self") else {
            return shared::patience::Patience::Normal;
        };
        shared::patience::Patience::read_owned(
            &self.config_home().join(shared::patience::FILE),
            me.uid(),
        )
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
    /// Every argv run through `capture`, in order.
    pub captured_argvs: Vec<Vec<String>>,
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
    /// stdin given to `exec_input`, in order (the argv is in `execs`).
    pub inputs: Vec<String>,
    /// Per-command `capture_status` answers: (`argv joined by spaces`, exit status, stderr);
    /// stdout is the `captures` entry. Else `capture`'s answer with status 0.
    pub statuses: Vec<(String, i32, String)>,
    /// Lock files held through `try_lock` (share one between two fakes to interleave runs).
    pub locks: std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>>,
    /// Commands (`argv joined by spaces`) whose `capture` panics: a check that crashes, to prove
    /// the Doctor still reports (the other areas, and this one as a failed row).
    pub crash: Vec<String>,
}

/// A [`FakeEnv`] lock: gone from the shared list when dropped.
struct FakeLock(std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>>, PathBuf);

// Gives the path back to the shared list, so a second fake can take the lock again.
impl Drop for FakeLock {
    fn drop(&mut self) {
        let mut held = self.0.lock().unwrap_or_else(|p| p.into_inner());
        held.retain(|p| *p != self.1);
    }
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
        match if self.exec_codes.is_empty() {
            0
        } else {
            self.exec_codes.remove(0)
        } {
            // `i32::MIN` in `exec_codes`: the program could not be started at all.
            i32::MIN => Err(io::Error::from(io::ErrorKind::NotFound)),
            code => Ok(code),
        }
    }
    /// Recorded like `exec`; stdout is the `captures` entry for the argv, else `captured`.
    fn exec_output(&mut self, argv: &[String]) -> io::Result<(i32, String)> {
        let line = argv.join(" ");
        let text = match self.captures.iter().find(|c| c.0 == line) {
            Some(c) => c.1.clone(),
            None => self.captured.clone().unwrap_or_default(),
        };
        Ok((self.exec(argv)?, text))
    }
    fn exec_secret(&mut self, argv: &[String], var: &str, secret: &str) -> io::Result<i32> {
        self.secrets.push((var.into(), secret.into()));
        self.exec(argv)
    }
    fn exec_input(&mut self, argv: &[String], input: &str) -> io::Result<i32> {
        self.inputs.push(input.into());
        self.exec(argv)
    }
    fn keyring_read(&mut self, desc: &str) -> Option<Vec<u8>> {
        self.keys.iter().find(|k| k.0 == desc).map(|k| k.1.clone())
    }
    fn lock_held(&self, path: &Path) -> bool {
        let held = self.locks.lock().unwrap_or_else(|p| p.into_inner());
        held.iter().any(|p| p == path)
    }
    fn try_lock(&mut self, path: &Path) -> io::Result<Option<Lock>> {
        // As the real one: the lock file is root's, 0600.
        if !self.root {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        let mut held = self.locks.lock().unwrap_or_else(|p| p.into_inner());
        if held.iter().any(|p| p == path) {
            return Ok(None);
        }
        held.push(path.into());
        Ok(Some(Lock {
            _held: Box::new(FakeLock(std::sync::Arc::clone(&self.locks), path.into())),
        }))
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
        self.captured_argvs
            .push(argv.iter().map(|a| (*a).to_owned()).collect());
        let line = argv.join(" ");
        if self.crash.contains(&line) {
            panic!("FakeEnv: {line} crashed");
        }
        match self.captures.iter().find(|c| c.0 == line) {
            Some(c) => Some(c.1.clone()),
            None => self.captured.clone(),
        }
    }
    fn capture_status(&mut self, argv: &[&str]) -> Option<(i32, String, String)> {
        let line = argv.join(" ");
        let Some((_, code, err)) = self.statuses.iter().find(|s| s.0 == line).cloned() else {
            return self.capture(argv).map(|o| (0, o, String::new()));
        };
        // The fake's scripted output for this line; none scripted is an empty one.
        let out = self.capture(argv).unwrap_or_default();
        Some((code, out, err))
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
    fn file_exists(&self, p: &Path) -> bool {
        self.files.iter().any(|f| f.0 == p)
            || (!self.home.as_os_str().is_empty()
                && p.starts_with(&self.home)
                && std::fs::symlink_metadata(p).is_ok())
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
            stdin: Arc::new(|_, _| Ok(String::new())),
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
mod tests;
