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
        "plugin.add" => add(env, &get("plugin"), inv.flag("yes"), as_json),
        "plugin.pack" => pack(
            env,
            &get("plugin"),
            inv.get("out"),
            inv.flag("yes"),
            as_json,
        ),
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

/// The permissions as sentences. `[[toggle]]`, `[[dock]]` and `[[hotkey]]` commands that are not
/// `wol` verbs need `run` (audit C8-1), so "nothing beyond drawing" is true for `[]`; the
/// commands themselves are listed by [`commands_lines`].
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

/// Each command a `[[toggle]]`, `[[dock]]` or `[[hotkey]]` runs as the person (not a `wol`
/// verb), one line each, so `check` shows what "Run commands as you" means here.
fn commands_lines(m: &Manifest) -> String {
    m.commands()
        .iter()
        .map(|(k, id, argv)| format!("Runs ({} {id}): {}\n", k.id(), argv.join(" ")))
        .collect()
}

// ---- Headless services for `check` and `run` ----

/// Side effects become lines in `events` (printed after the run). For `wol plugin run`
/// commands run limited as the user and state and toggles use the real stores under
/// `$XDG_STATE_HOME`. With `inert` (`check`, `new`, `pack`, the same as Plugin Maker's check)
/// nothing runs, nothing is read from the web and nothing is written: each call is only an event
/// with an empty answer, because the plugin was never granted anything.
pub struct CliServices {
    pub inert: bool,
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
        if self.inert {
            return Ok(String::new());
        }
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
        if self.inert {
            self.event(format!("http_get: {url}"));
            return Ok(String::new());
        }
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
        if self.inert {
            return Ok(());
        }
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
        if self.inert {
            return None;
        }
        cp::load_state(&self.state_home, plugin)
    }
    fn state_save(&self, plugin: &str, json: &str) -> Result<(), String> {
        if self.inert {
            self.event("state saved".to_owned());
            return Ok(());
        }
        cp::save_state(&self.state_home, plugin, json)
    }
}

fn services(env: &dyn Env, dir: &Path, m: &Manifest, inert: bool) -> Rc<CliServices> {
    let hosts = env
        .var(HOSTS_VAR)
        .unwrap_or_default()
        .split(',')
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty())
        .collect();
    Rc::new(CliServices {
        inert,
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

/// The full check: folder (`wol_core::plugins::check`) plus every script loaded and run once
/// (`wol_lua::plugin::lua_problems`), with inert services (nothing runs or is written); the same
/// function as Plugin Maker's problems list.
pub fn full_check(env: &dyn Env, dir: &Path, system: bool) -> cp::Report {
    cp::check_full(dir, system, |d, m| {
        wol_lua::plugin::lua_problems(
            d,
            m,
            wol_lua::Deadlines::DEFAULT,
            &|| -> Rc<dyn Services> { services(env, d, m, true) },
        )
    })
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
                "OK {} {} ({} KB): {}\nPermissions: {}\n{}",
                m.id,
                m.version,
                r.size.div_ceil(1024),
                kinds_line(m),
                perm_labels(&m.permissions),
                commands_lines(m)
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
                        Kind::ALL
                            .into_iter()
                            .filter(|k| !k.is_planned())
                            .map(Kind::id)
                            .collect::<Vec<_>>()
                            .join(", ")
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
    let s = services(env, &dir, &m, false);
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
    let manifest = if on {
        let m = match &p.manifest {
            Ok(m) => m,
            Err(e) => return fail(env, ErrorCode::Invalid, &format!("{id} does not load: {e}")),
        };
        enabled.enable(m);
        Some(m)
    } else {
        enabled.disable(id);
        None
    };
    let granted = manifest.map(|m| m.permissions.clone()).unwrap_or_default();
    if let Err(e) = cp::save_enabled(&config, &enabled) {
        return fail(env, ErrorCode::Failed, &e);
    }
    // Audit X6-1: `[[webapp]]` entries are launchers while the plugin is on.
    let kept = match cp::sync_webapps(&env.data_home(), id, manifest.map(|m| (m, p.dir.as_path())))
    {
        Ok(k) => k,
        Err(e) => return fail(env, ErrorCode::Failed, &e),
    };
    let reloaded = reload(env);
    if as_json {
        emit_json(
            env,
            &json!({"id": id, "enabled": on, "granted": granted.iter().map(|p| p.id()).collect::<Vec<_>>(), "reloaded": reloaded, "kept": kept}),
        );
    } else if let Some(m) = manifest {
        env.out(&format!(
            "{id} is on. Granted: {}.\n{}{}",
            perm_labels(&granted),
            commands_lines(m),
            kept_lines(&kept)
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
    if let Err(e) = cp::sync_webapps(&env.data_home(), id, None) {
        return fail(env, ErrorCode::Failed, &e);
    }
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        return fail(env, ErrorCode::Failed, &format!("{}: {e}", dir.display()));
    }
    reload(env);
    env.out(&format!("Removed {}.\n", dir.display()));
    0
}

/// Web app launchers left alone because a file of that name is the person's own.
fn kept_lines(kept: &[String]) -> String {
    kept.iter()
        .map(|f| format!("Kept your own {f}; that web app of the plugin is not added.\n"))
        .collect()
}

/// `wol plugin add <folder|file.wolplug> [--yes]` (audit X6-2): the Store's install step
/// (`wol_core::package::verify_wolplug`, then `install_plugin` into this account's plugin folder)
/// for a file or folder on this machine. Unsigned (every folder, a `.wolplug` without the index
/// signature) needs `--yes`. Installing never turns the plugin on.
fn add(env: &mut dyn Env, what: &str, yes: bool, as_json: bool) -> i32 {
    let p = Path::new(what);
    let v = if p.join(MANIFEST_FILE).is_file() {
        verified_folder(p)
    } else if p.is_file() {
        verified_wolplug(p)
    } else if shared::plugins::is_plugin_id(what) {
        return fail(
            env,
            ErrorCode::Unavailable,
            &format!(
                "Plugins by id come from App Store > Plugins for now; wol plugin add takes a .{} file or a plugin folder.",
                cp::PACK_EXT
            ),
        );
    } else {
        return fail(
            env,
            ErrorCode::NotFound,
            &format!(
                "{what} is neither a .{} file nor a folder with a {MANIFEST_FILE}.",
                cp::PACK_EXT
            ),
        );
    };
    let v = match v {
        Ok(v) => v,
        Err(e) => return fail(env, ErrorCode::Invalid, &e),
    };
    let signed = matches!(v.trust, wol_core::package::Trust::Signed(_));
    if !signed && !yes {
        return fail(
            env,
            ErrorCode::Usage,
            &format!(
                "{} is not signed by the WolOS plugin index: read it (wol plugin check {what}), then pass --yes to install it anyway",
                v.manifest.id
            ),
        );
    }
    let m = v.manifest.clone();
    let dir = match wol_core::package::install_plugin(&v, &user_dir(env)) {
        Ok(d) => d,
        Err(e) => return fail(env, ErrorCode::Invalid, &e),
    };
    let enabled = cp::load_enabled(&env.config_home());
    let on = enabled.is_enabled(&m.id);
    // An update of a plugin that is on: its web app launchers follow the new manifest.
    let kept = if on {
        cp::sync_webapps(&env.data_home(), &m.id, Some((&m, &dir))).unwrap_or_default()
    } else {
        Vec::new()
    };
    let needs_grant = on && !enabled.missing(&m).is_empty();
    let reloaded = on && reload(env);
    if as_json {
        emit_json(
            env,
            &json!({"id": m.id, "version": m.version, "dir": dir, "signed": signed, "enabled": on, "needs_grant": needs_grant, "reloaded": reloaded, "kept": kept}),
        );
    } else {
        let next = if !on {
            format!("Turn it on: wol plugin enable {}\n", m.id)
        } else if needs_grant {
            format!("It asks for more now: wol plugin enable {}\n", m.id)
        } else {
            String::new()
        };
        env.out(&format!(
            "Installed {} {} in {}.\nAsks to: {}\n{}{}{next}",
            m.id,
            m.version,
            dir.display(),
            perm_labels(&m.permissions),
            commands_lines(&m),
            kept_lines(&kept)
        ));
    }
    0
}

/// A plugin folder as the Store's verified form: checked as a user plugin, unsigned.
fn verified_folder(dir: &Path) -> Result<wol_core::package::VerifiedPlugin, String> {
    let r = cp::check(dir, false);
    let m = match (r.manifest, r.problems.first()) {
        (Some(m), None) => m,
        (_, Some(p)) => return Err(p.clone()),
        (None, None) => return Err("The plugin does not load.".into()),
    };
    let mut files = Vec::new();
    for (rel, path) in cp::walk(dir)? {
        let b = std::fs::read(&path).map_err(|e| format!("{rel}: {e}"))?;
        files.push((rel, b));
    }
    Ok(wol_core::package::VerifiedPlugin {
        manifest: m,
        trust: wol_core::package::Trust::Unsigned,
        files,
    })
}

/// A `.wolplug` file: unpacked by the system's zstd (as `pack` makes it) to a bounded pipe, so
/// a bomb stops one byte past `WOLPLUG_MAX_TAR` and nothing is written; then the same checks as
/// a Store download (`verify_wolplug`: size, shape, signature, hashes, manifest).
fn verified_wolplug(file: &Path) -> Result<wol_core::package::VerifiedPlugin, String> {
    use wol_core::package::WOLPLUG_MAX_TAR;
    let size = std::fs::metadata(file).map_err(|e| format!("{}: {e}", file.display()))?;
    if size.len() > WOLPLUG_MAX_TAR {
        return Err("The plugin file is too large.".into());
    }
    let argv = [
        "zstd".to_owned(),
        "-q".to_owned(),
        "-d".to_owned(),
        "-c".to_owned(),
        "--".to_owned(),
        file.to_string_lossy().into_owned(),
    ];
    let tar = wol_core::widgets::run_limited(
        &argv,
        Path::new("/"),
        Duration::from_secs(30),
        usize::try_from(WOLPLUG_MAX_TAR).unwrap_or(usize::MAX),
    )
    .map_err(|e| match e {
        wol_core::widgets::SourceError::TooLarge => "The plugin file is too large.".to_owned(),
        wol_core::widgets::SourceError::NotFound(_) => {
            "zstd is missing, so the plugin file cannot be read.".to_owned()
        }
        _ => "The plugin file is damaged (zstd could not read it).".to_owned(),
    })?;
    let keys = wol_core::package::read_keys(Path::new(shared::package::KEYS_DIR));
    wol_core::package::verify_wolplug(&tar, &keys)
}

fn pack(env: &mut dyn Env, what: &str, out: Option<&str>, yes: bool, as_json: bool) -> i32 {
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
    // Audit S4-23: only the target is ever replaced, and only when asked; the tar is a fresh,
    // unique name next to it (never `with_extension`, which named a neighbour).
    if target.symlink_metadata().is_ok() && !yes {
        return fail(
            env,
            ErrorCode::Usage,
            &format!(
                "{} already exists: pass --yes to replace it",
                target.display()
            ),
        );
    }
    let nonce = shared::jobs::new_nonce()
        .unwrap_or_else(|_| format!("{}-{}", std::process::id(), shared::sys::monotonic_ns()));
    let mut tar_name = target.file_name().unwrap_or_default().to_os_string();
    tar_name.push(format!(".{nonce}.tar"));
    let tar = target.with_file_name(tar_name);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tar)
        .and_then(|mut f| std::io::Write::write_all(&mut f, &p.tar));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tar);
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
        Ok(c) => {
            let _ = std::fs::remove_file(&tar);
            return fail(env, ErrorCode::Failed, &format!("zstd exited with {c}"));
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tar);
            return fail(env, ErrorCode::Unavailable, &format!("zstd: {e}"));
        }
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
