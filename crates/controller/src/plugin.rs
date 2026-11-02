// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol plugin` (ADR 0031, docs/modules/plugins.md): the agent-first way to make, check, run and
//! ship a shell plugin without a display. Folders, grants, state and toggles go through
//! `wol_core::plugins`, the same code the desktop uses; scripts run in `wol_lua::plugin` with a
//! headless [`CliServices`] that prints side effects (notifications, panels) as `events`.
use crate::cli::Invocation;
use crate::run::{Env, SHELL_SOCKET, emit_json, fail};
use serde_json::{Value, json};
use shared::errors::ErrorCode;
use shared::plugins::{Kind, MANIFEST_FILE, Manifest, Permission};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
use wol_core::plugins as cp;
use wol_lua::plugin::{Entry, Script};
use wol_lua::{Services, Stamp};

/// Comma-separated hosts `wol.http_get` may contact in `wol plugin run` and `check`
/// (there is no consent sheet on the command line).
pub const HOSTS_VAR: &str = "WOL_PLUGIN_HOSTS";
/// Output cap for `wol.run`.
const RUN_OUTPUT: usize = 64 * 1024;

pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let as_json = inv.flag("json") || !env.stdout_tty();
    let get = |k: &str| inv.get(k).unwrap_or_default().to_owned();
    match inv.action {
        "plugin.list" => list(env, as_json),
        "plugin.new" => new(env, &get("id"), inv.get("kinds"), as_json),
        "plugin.check" => check(env, &get("plugin"), as_json),
        "plugin.run" => run(
            env,
            &get("plugin"),
            &get("kind"),
            inv.get("entry"),
            inv.get("input"),
        ),
        "plugin.enable" => set_enabled(env, &get("id"), true, as_json),
        "plugin.disable" => set_enabled(env, &get("id"), false, as_json),
        "plugin.remove" => remove(env, &get("id")),
        "plugin.pack" => pack(env, &get("plugin"), inv.get("out"), as_json),
        other => fail(env, ErrorCode::Failed, &format!("no handler for {other}")),
    }
}

/// `~/.local/share/wolos/plugins`.
fn user_dir(env: &dyn Env) -> PathBuf {
    env.data_home().join(shared::plugins::USER_DIR)
}

fn plugins(env: &dyn Env) -> Vec<cp::Plugin> {
    cp::load_all(&cp::dirs(&env.data_home()))
}

/// A folder path (has a `plugin.toml`) or an installed plugin id: (folder, system).
fn resolve(env: &dyn Env, what: &str) -> Result<(PathBuf, bool), String> {
    if what.is_empty() {
        return Err("Give a plugin id or folder.".into());
    }
    let p = Path::new(what);
    if p.join(MANIFEST_FILE).is_file() {
        // A folder outside the user plugin dir is a checkout (it may use org.wolos.*); installing
        // into the user dir re-checks it as a user plugin.
        let dir = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let user = user_dir(env);
        let user = user.canonicalize().unwrap_or(user);
        return Ok((p.to_path_buf(), !dir.starts_with(user)));
    }
    for (dir, system) in cp::dirs(&env.data_home()) {
        let d = dir.join(what);
        if shared::plugins::is_plugin_id(what) && d.join(MANIFEST_FILE).is_file() {
            return Ok((d, system));
        }
    }
    Err(format!(
        "No plugin called \"{what}\" is installed and no folder {what}/{MANIFEST_FILE} exists. Start one: wol plugin new {what}"
    ))
}

/// Asks a running desktop to reread plugins; false when none runs.
fn reload(env: &mut dyn Env) -> bool {
    let Some(rt) = env.var("XDG_RUNTIME_DIR") else {
        return false;
    };
    env.request(&Path::new(&rt).join(SHELL_SOCKET), b"reload\n")
        .is_ok()
}

fn perm_labels(perms: &[Permission]) -> String {
    if perms.is_empty() {
        return "nothing beyond drawing".into();
    }
    perms
        .iter()
        .map(|p| p.label())
        .collect::<Vec<_>>()
        .join("; ")
}

// ---- Headless services for `check` and `run` ----

/// Side effects become lines in `events` (printed after the run); commands run limited as the
/// user; state and toggles use the real stores under `$XDG_STATE_HOME`.
pub struct CliServices {
    pub state_home: PathBuf,
    pub manifest: Manifest,
    pub dir: PathBuf,
    pub hosts: Vec<String>,
    pub events: RefCell<Vec<String>>,
}

impl CliServices {
    fn event(&self, s: String) {
        self.events.borrow_mut().push(s);
    }
}

impl Services for CliServices {
    fn now(&self) -> Stamp {
        wol_lua::Live.now()
    }
    fn system(&self) -> wol_lua::SystemInfo {
        wol_lua::Live.system()
    }
    fn notify(&self, title: &str, body: &str) -> Result<(), String> {
        self.event(format!("notify: {title} — {body}"));
        Ok(())
    }
    fn run(&self, plugin: &str, argv: &[String], timeout_secs: u32) -> Result<String, String> {
        // `wol` verbs see the plugin as a helper: policy, approval and the audit log apply.
        let mut cmd: Vec<String> = Vec::new();
        if argv[0] == "wol" {
            cmd.extend(["env".to_owned(), format!("WOL_HELPER={plugin}")]);
        }
        cmd.extend(argv.iter().cloned());
        self.event(format!("run: {}", argv.join(" ")));
        wol_core::widgets::run_limited(
            &cmd,
            &self.dir,
            Duration::from_secs(u64::from(timeout_secs)),
            RUN_OUTPUT,
        )
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .map_err(|e| e.message())
    }
    fn http_get(&self, _plugin: &str, url: &str) -> Result<String, String> {
        let host = shared::widgets::manifest::url_host(url).unwrap_or_default();
        if !self.hosts.contains(&host) {
            return Err(format!(
                "Allow this plugin to read data from {host} ({HOSTS_VAR}={host} on the command line)."
            ));
        }
        self.event(format!("http_get: {url}"));
        wol_core::widgets::http_get(url, wol_core::widgets::HTTP_TIMEOUT, RUN_OUTPUT)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .map_err(|e| e.message())
    }
    fn open(&self, _plugin: &str, target: &str) -> Result<(), String> {
        self.event(format!("open: {target}"));
        Ok(())
    }
    fn toggle_get(&self, plugin: &str, toggle: &str) -> bool {
        cp::toggle_get(&self.state_home, plugin, toggle)
    }
    fn toggle_set(&self, plugin: &str, toggle: &str, on: bool) -> Result<(), String> {
        let Some(t) = self.manifest.toggle.iter().find(|t| t.id == toggle) else {
            return Err(format!("This plugin has no toggle \"{toggle}\"."));
        };
        let argv = if on { &t.on } else { &t.off };
        self.event(format!(
            "toggle {toggle} {}: {}",
            if on { "on" } else { "off" },
            argv.join(" ")
        ));
        cp::toggle_set(&self.state_home, plugin, toggle, on)
    }
    fn show(&self, _plugin: &str, panel: &str) -> Result<(), String> {
        let own = self.manifest.panel.iter().any(|p| p.id == panel);
        if !own && !shared::shell::PANELS.contains(&panel) {
            return Err(format!("There is no panel called \"{panel}\"."));
        }
        self.event(format!("show: {panel}"));
        Ok(())
    }
    fn state_load(&self, plugin: &str) -> Option<String> {
        cp::load_state(&self.state_home, plugin)
    }
    fn state_save(&self, plugin: &str, json: &str) -> Result<(), String> {
        cp::save_state(&self.state_home, plugin, json)
    }
}

fn services(env: &dyn Env, dir: &Path, m: &Manifest) -> Rc<CliServices> {
    let hosts = env
        .var(HOSTS_VAR)
        .unwrap_or_default()
        .split(',')
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty())
        .collect();
    Rc::new(CliServices {
        state_home: env.state_home(),
        manifest: m.clone(),
        dir: dir.to_path_buf(),
        hosts,
        events: RefCell::new(Vec::new()),
    })
}

fn load_script(
    dir: &Path,
    m: &Manifest,
    kind: Kind,
    id: &str,
    file: &str,
    s: Rc<CliServices>,
) -> Result<Script, String> {
    let src = std::fs::read_to_string(dir.join(file))
        .map_err(|_| format!("{file}: the file could not be read as text."))?;
    Script::load(
        Entry {
            kind,
            plugin: &m.id,
            id,
            dir,
        },
        &src,
        &m.permissions,
        s,
    )
    .map_err(|e| format!("{file}: {e}"))
}

/// Loads every script and renders each once (search: one query), the way the desktop will.
fn lua_problems(env: &dyn Env, dir: &Path, m: &Manifest) -> Vec<String> {
    let mut out = Vec::new();
    for (kind, id, file) in m.scripts() {
        let s = services(env, dir, m);
        let mut script = match load_script(dir, m, kind, id, file, s) {
            Ok(s) => s,
            Err(e) => {
                out.push(e);
                continue;
            }
        };
        let r = if kind == Kind::Search {
            script.query("test").map(|_| ())
        } else {
            script.render().and_then(|_| script.popover().map(|_| ()))
        };
        if let Err(e) = r {
            out.push(format!("{file}: {e}"));
        }
    }
    out
}

/// The full check: folder (`wol_core::plugins::check`) plus Lua loads and renders.
pub fn full_check(env: &dyn Env, dir: &Path, system: bool) -> cp::Report {
    let mut r = cp::check(dir, system);
    if let Some(m) = r.manifest.clone()
        && r.problems.is_empty()
    {
        r.problems.extend(lua_problems(env, dir, &m));
    }
    r
}

fn report_json(r: &cp::Report, dir: &Path) -> Value {
    json!({
        "id": r.id,
        "dir": dir,
        "valid": r.problems.is_empty(),
        "problems": r.problems,
        "files": r.files,
        "size": r.size,
        "manifest": r.manifest.as_ref().map(|m| serde_json::to_value(m).unwrap_or(Value::Null)),
    })
}

fn kinds_line(m: &Manifest) -> String {
    m.entries()
        .iter()
        .map(|(k, id)| format!("{} {id}", k.id()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn print_report(env: &mut dyn Env, r: &cp::Report, dir: &Path, as_json: bool) -> i32 {
    if as_json {
        emit_json(env, &report_json(r, dir));
        return if r.problems.is_empty() {
            0
        } else {
            ErrorCode::Invalid.exit()
        };
    }
    match (&r.manifest, r.problems.is_empty()) {
        (Some(m), true) => {
            env.out(&format!(
                "OK {} {} ({} KB): {}\nPermissions: {}\n",
                m.id,
                m.version,
                r.size.div_ceil(1024),
                kinds_line(m),
                perm_labels(&m.permissions)
            ));
            0
        }
        _ => {
            let mut out = format!("{}: {} problem(s)\n", dir.display(), r.problems.len());
            for p in &r.problems {
                out.push_str(&format!("  {p}\n"));
            }
            env.out(&out);
            ErrorCode::Invalid.exit()
        }
    }
}

// ---- Verbs ----

fn list(env: &mut dyn Env, as_json: bool) -> i32 {
    let enabled = cp::load_enabled(&env.config_home());
    let all = plugins(env);
    if as_json {
        let items: Vec<Value> = all
            .iter()
            .map(|p| {
                let (valid, problem) = match &p.manifest {
                    Ok(_) => (true, Value::Null),
                    Err(e) => (false, Value::String(e.clone())),
                };
                json!({
                    "id": p.id, "name": p.name(), "dir": p.dir, "system": p.system,
                    "enabled": enabled.is_enabled(&p.id), "valid": valid, "problem": problem,
                    "version": p.manifest.as_ref().map(|m| m.version.clone()).unwrap_or_default(),
                    "kinds": p.manifest.as_ref().map(|m| m.kinds().iter().map(|k| k.id()).collect::<Vec<_>>()).unwrap_or_default(),
                    "permissions": p.manifest.as_ref().map(|m| m.permissions.iter().map(|x| x.id()).collect::<Vec<_>>()).unwrap_or_default(),
                    "needs_grant": p.manifest.as_ref().map(|m| !enabled.missing(m).is_empty()).unwrap_or(false),
                })
            })
            .collect();
        let dir = user_dir(env);
        emit_json(env, &json!({"plugins": items, "user_dir": dir}));
        return 0;
    }
    if all.is_empty() {
        env.out(&format!(
            "No plugins in {} or {}. Make one: wol plugin new <id>\n",
            user_dir(env).display(),
            shared::plugins::SYSTEM_DIR
        ));
        return 0;
    }
    let mut out = String::new();
    for p in &all {
        let state = match (&p.manifest, enabled.is_enabled(&p.id)) {
            (Err(e), _) => format!("problem: {e}"),
            (Ok(m), true) if !enabled.missing(m).is_empty() => {
                "on, needs a new grant (wol plugin enable)".into()
            }
            (Ok(_), true) => "on".into(),
            (Ok(_), false) => "off".into(),
        };
        let detail = p
            .manifest
            .as_ref()
            .map(|m| format!("{} - {}", m.version, kinds_line(m)))
            .unwrap_or_default();
        out.push_str(&format!(
            "{:<28} {:<20} {}{}\n  {}\n",
            p.id,
            p.name(),
            state,
            if p.system { " (system)" } else { "" },
            detail
        ));
    }
    env.out(&out);
    0
}

fn new(env: &mut dyn Env, id: &str, kinds: Option<&str>, as_json: bool) -> i32 {
    let mut ks = Vec::new();
    for k in kinds
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        match Kind::parse(k) {
            Some(k) => ks.push(k),
            None => {
                return fail(
                    env,
                    ErrorCode::Usage,
                    &format!(
                        "unknown kind '{k}'; kinds are {}",
                        Kind::ALL.map(Kind::id).join(", ")
                    ),
                );
            }
        }
    }
    let name = {
        let last = id.rsplit('.').next().unwrap_or(id);
        let mut c = last.chars();
        c.next()
            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
            .unwrap_or_default()
            .replace('-', " ")
    };
    let files = match cp::scaffold(id, &name, &ks) {
        Ok(f) => f,
        Err(e) => return fail(env, ErrorCode::Usage, &e),
    };
    let dir = user_dir(env).join(id);
    if dir.exists() {
        return fail(
            env,
            ErrorCode::Invalid,
            &format!(
                "{} exists already; pick another id or remove it first",
                dir.display()
            ),
        );
    }
    for (rel, text) in &files {
        let p = dir.join(rel);
        if let Some(parent) = p.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return fail(
                env,
                ErrorCode::Failed,
                &format!("{}: {e}", parent.display()),
            );
        }
        if let Err(e) = std::fs::write(&p, text) {
            return fail(env, ErrorCode::Failed, &format!("{}: {e}", p.display()));
        }
    }
    let r = full_check(env, &dir, false);
    if as_json {
        emit_json(env, &report_json(&r, &dir));
    } else {
        env.out(&format!(
            "Made {}:\n{}\nNext: edit it, then wol plugin check {id} && wol plugin enable {id}\n",
            dir.display(),
            files
                .iter()
                .map(|(f, _)| format!("  {f}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
        if !r.problems.is_empty() {
            print_report(env, &r, &dir, false);
        }
    }
    if r.problems.is_empty() {
        0
    } else {
        ErrorCode::Failed.exit()
    }
}

fn check(env: &mut dyn Env, what: &str, as_json: bool) -> i32 {
    let (dir, system) = match resolve(env, what) {
        Ok(x) => x,
        Err(e) => return fail(env, ErrorCode::NotFound, &e),
    };
    let r = full_check(env, &dir, system);
    print_report(env, &r, &dir, as_json)
}

fn run(env: &mut dyn Env, what: &str, kind: &str, entry: Option<&str>, input: Option<&str>) -> i32 {
    let (dir, system) = match resolve(env, what) {
        Ok(x) => x,
        Err(e) => return fail(env, ErrorCode::NotFound, &e),
    };
    let Some(kind) = Kind::parse(kind).filter(|k| k.has_script()) else {
        return fail(
            env,
            ErrorCode::Usage,
            "<kind> is menubar, widget, panel or search",
        );
    };
    let r = cp::check(&dir, system);
    let Some(m) = r.manifest.filter(|_| r.problems.is_empty()) else {
        return fail(
            env,
            ErrorCode::Invalid,
            &r.problems
                .first()
                .cloned()
                .unwrap_or_else(|| "the plugin does not load".into()),
        );
    };
    let scripts: Vec<(Kind, &str, &str)> = m
        .scripts()
        .into_iter()
        .filter(|(k, id, _)| *k == kind && entry.is_none_or(|e| e == *id))
        .collect();
    let Some((_, id, file)) = scripts.first().copied() else {
        return fail(
            env,
            ErrorCode::NotFound,
            &format!(
                "no [[{}]] entry{} in {}; it has: {}",
                kind.id(),
                entry.map(|e| format!(" \"{e}\"")).unwrap_or_default(),
                m.id,
                kinds_line(&m)
            ),
        );
    };
    let s = services(env, &dir, &m);
    let mut script = match load_script(&dir, &m, kind, id, file, s.clone()) {
        Ok(x) => x,
        Err(e) => return fail(env, ErrorCode::Invalid, &e),
    };
    let mut out =
        json!({"plugin": m.id, "kind": kind.id(), "entry": id, "refresh": script.refresh()});
    let result: Result<(), String> = if kind == Kind::Search {
        script.query(input.unwrap_or("test")).map(|hits| {
            out["hits"] = json!(hits
                .iter()
                .map(|h| json!({"id": h.id, "title": h.title, "subtitle": h.subtitle, "icon": h.icon}))
                .collect::<Vec<_>>());
        })
    } else {
        script.render().and_then(|_| {
            if let Some(n) = input.and_then(|i| i.parse::<u32>().ok()) {
                script.click(n)?;
                out["clicked"] = json!(n);
            }
            let node = script.render()?;
            out["node"] = serde_json::to_value(&node).unwrap_or(Value::Null);
            out["popover"] = match script.popover()? {
                Some(p) => serde_json::to_value(&p).unwrap_or(Value::Null),
                None => Value::Null,
            };
            Ok(())
        })
    };
    out["events"] = json!(s.events.borrow().clone());
    match result {
        Ok(()) => {
            emit_json(env, &out);
            0
        }
        Err(e) => {
            out["error"] = json!(e);
            emit_json(env, &out);
            ErrorCode::Invalid.exit()
        }
    }
}

fn set_enabled(env: &mut dyn Env, id: &str, on: bool, as_json: bool) -> i32 {
    let p = match plugins(env).into_iter().find(|p| p.id == id) {
        Some(p) => p,
        None => {
            return fail(
                env,
                ErrorCode::NotFound,
                &format!("no plugin called \"{id}\" is installed; see wol plugin list"),
            );
        }
    };
    let config = env.config_home();
    let mut enabled = cp::load_enabled(&config);
    let granted = if on {
        let m = match &p.manifest {
            Ok(m) => m,
            Err(e) => return fail(env, ErrorCode::Invalid, &format!("{id} does not load: {e}")),
        };
        enabled.enable(m);
        m.permissions.clone()
    } else {
        enabled.disable(id);
        Vec::new()
    };
    if let Err(e) = cp::save_enabled(&config, &enabled) {
        return fail(env, ErrorCode::Failed, &e);
    }
    let reloaded = reload(env);
    if as_json {
        emit_json(
            env,
            &json!({"id": id, "enabled": on, "granted": granted.iter().map(|p| p.id()).collect::<Vec<_>>(), "reloaded": reloaded}),
        );
    } else if on {
        env.out(&format!(
            "{id} is on. Granted: {}.\n",
            perm_labels(&granted)
        ));
    } else {
        env.out(&format!("{id} is off.\n"));
    }
    0
}

fn remove(env: &mut dyn Env, id: &str) -> i32 {
    let dir = user_dir(env).join(id);
    if !shared::plugins::is_plugin_id(id) || !dir.join(MANIFEST_FILE).is_file() {
        let system = Path::new(shared::plugins::SYSTEM_DIR).join(id);
        if system.join(MANIFEST_FILE).is_file() {
            return fail(
                env,
                ErrorCode::Denied,
                &format!("{id} is a system plugin; turn it off with wol plugin disable {id}"),
            );
        }
        let msg = format!(
            "no user plugin called \"{id}\" in {}",
            user_dir(env).display()
        );
        return fail(env, ErrorCode::NotFound, &msg);
    }
    let config = env.config_home();
    let mut enabled = cp::load_enabled(&config);
    enabled.disable(id);
    if let Err(e) = cp::save_enabled(&config, &enabled) {
        return fail(env, ErrorCode::Failed, &e);
    }
    cp::clear_state(&env.state_home(), id);
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        return fail(env, ErrorCode::Failed, &format!("{}: {e}", dir.display()));
    }
    reload(env);
    env.out(&format!("Removed {}.\n", dir.display()));
    0
}

fn pack(env: &mut dyn Env, what: &str, out: Option<&str>, as_json: bool) -> i32 {
    let (dir, system) = match resolve(env, what) {
        Ok(x) => x,
        Err(e) => return fail(env, ErrorCode::NotFound, &e),
    };
    let r = full_check(env, &dir, system);
    let Some(m) = r.manifest.filter(|_| r.problems.is_empty()) else {
        return fail(
            env,
            ErrorCode::Invalid,
            &format!(
                "fix the plugin first: {}",
                r.problems.first().cloned().unwrap_or_default()
            ),
        );
    };
    let p = match cp::pack(&dir, &m) {
        Ok(p) => p,
        Err(e) => return fail(env, ErrorCode::Failed, &e),
    };
    let target = match out {
        Some(o) if Path::new(o).is_dir() => Path::new(o).join(&p.name),
        Some(o) => PathBuf::from(o),
        None => dir
            .parent()
            .map_or_else(|| PathBuf::from(&p.name), |d| d.join(&p.name)),
    };
    let tar = target.with_extension("tar");
    if let Err(e) = std::fs::write(&tar, &p.tar) {
        return fail(env, ErrorCode::Failed, &format!("{}: {e}", tar.display()));
    }
    // zstd (the system's own, as every image has it): level 19, removes the tar.
    let (to, from) = (
        target.to_string_lossy().into_owned(),
        tar.to_string_lossy().into_owned(),
    );
    let argv: Vec<String> = ["zstd", "-q", "-f", "-19", "--rm", "-o", &to, &from]
        .iter()
        .map(|s| s.to_string())
        .collect();
    match env.exec(&argv) {
        Ok(0) => {}
        Ok(c) => return fail(env, ErrorCode::Failed, &format!("zstd exited with {c}")),
        Err(e) => return fail(env, ErrorCode::Unavailable, &format!("zstd: {e}")),
    }
    if as_json {
        emit_json(
            env,
            &json!({"file": target, "id": m.id, "version": m.version, "sha256": p.sha256, "signed": false}),
        );
    } else {
        env.out(&format!(
            "{}\n{}Unsigned: the Store marks it unreviewed until the index signs it.\n",
            target.display(),
            p.sha256
        ));
    }
    0
}
