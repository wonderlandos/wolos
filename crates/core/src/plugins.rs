// SPDX-License-Identifier: BSD-2-Clause
//! Shell plugins (ADR 0031, docs/modules/plugins.md), the part without Lua: plugin folders and
//! their loading and checking, the enable list with grants, the per-plugin state store, toggle
//! flag files, the render failure counter, the `wol plugin new` scaffold and the `.wolplug` pack.
//! The shell and `wol plugin` both go through here, so they behave the same; scripts run in
//! `wol_lua::plugin` (the shell and the CLI wire its `Services` to the stores below).
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared::plugins::{
    Kind, MANIFEST_FILE, MAX_FAILURES, MAX_STATE, MAX_TOTAL_SIZE, Manifest, Permission, STATE_DIR,
    SYSTEM_DIR, TOGGLES_DIR, USER_DIR, check_hypr, is_plugin_id,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Under `$XDG_CONFIG_HOME`: which plugins are on and what each was granted.
pub const ENABLED_FILE: &str = "wolos/plugins.json";
/// Largest single file in a plugin folder (scripts, themes, images).
pub const MAX_FILE: u64 = 256 * 1024;
/// Folder depth under the plugin root.
const MAX_DEPTH: usize = 4;
/// `.wolplug` file name: `<id>-<version>.wolplug`.
pub const PACK_EXT: &str = "wolplug";
/// Hash list member of a `.wolplug` (ADR 0011 text).
pub const PACK_MANIFEST: &str = "SHA256";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plugin {
    /// The folder name.
    pub id: String,
    pub dir: PathBuf,
    /// Under [`SYSTEM_DIR`] (shipped or installed by wopkg): may use `org.wolos.*`.
    pub system: bool,
    /// The manifest, or the first sentence to fix.
    pub manifest: Result<Manifest, String>,
}

impl Plugin {
    pub fn name(&self) -> &str {
        self.manifest.as_ref().map_or(&self.id, |m| &m.name)
    }
}

/// Plugin dirs, user first (a user plugin overrides a system one with the same id).
pub fn dirs(data_home: &Path) -> Vec<(PathBuf, bool)> {
    vec![
        (data_home.join(USER_DIR), false),
        (PathBuf::from(SYSTEM_DIR), true),
    ]
}

/// Regular files under `dir` as (relative path, absolute path), sorted. A symlink, device,
/// hidden entry or a folder deeper than [`MAX_DEPTH`] is an error: a plugin is plain files.
pub fn walk(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    fn go(
        root: &Path,
        dir: &Path,
        depth: usize,
        out: &mut Vec<(String, PathBuf)>,
    ) -> Result<(), String> {
        let rd = std::fs::read_dir(dir)
            .map_err(|_| format!("The folder {} could not be read.", dir.display()))?;
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let rel = p
                .strip_prefix(root)
                .map_err(|_| "A file is outside the plugin folder.".to_owned())?
                .to_string_lossy()
                .replace('\\', "/");
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            if name.as_deref().is_some_and(|n| n.starts_with('.')) {
                return Err(format!(
                    "Remove the hidden entry {rel}; a plugin is plain files."
                ));
            }
            let meta = std::fs::symlink_metadata(&p)
                .map_err(|_| format!("The file {rel} could not be read."))?;
            if meta.file_type().is_symlink() {
                return Err(format!("{rel} is a symlink; a plugin has no symlinks."));
            }
            if meta.is_dir() {
                if depth >= MAX_DEPTH {
                    return Err(format!(
                        "The folder {rel} is nested too deep (at most {MAX_DEPTH} levels)."
                    ));
                }
                go(root, &p, depth + 1, out)?;
            } else if meta.is_file() {
                if meta.len() > MAX_FILE {
                    return Err(format!("{rel} is larger than {} KB.", MAX_FILE / 1024));
                }
                out.push((rel, p));
            } else {
                return Err(format!("{rel} is not a regular file."));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    go(dir, dir, 1, &mut out)?;
    Ok(out)
}

/// What `wol plugin check` and the loader know about a folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub id: String,
    pub manifest: Option<Manifest>,
    /// Sentences, in the order found; empty = the folder loads.
    pub problems: Vec<String>,
    /// Every regular file, relative.
    pub files: Vec<String>,
    pub size: u64,
}

/// Reads and checks one folder without running Lua: manifest, id, reserved prefix, files named
/// exist, no symlinks, size, compositor whitelist, theme tokens and contrast, Looks presets.
pub fn check(dir: &Path, system: bool) -> Report {
    let id = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut r = Report {
        id: id.clone(),
        ..Report::default()
    };
    let text = match std::fs::read_to_string(dir.join(MANIFEST_FILE)) {
        Ok(t) => t,
        Err(_) => {
            r.problems.push(format!(
                "The folder has no {MANIFEST_FILE}; start with wol plugin new <id>."
            ));
            return r;
        }
    };
    let m = match shared::plugins::parse(&text) {
        Ok(m) => m,
        Err(e) => {
            r.problems.push(e);
            return r;
        }
    };
    if !is_plugin_id(&id) {
        r.problems.push(format!(
            "The folder is called \"{id}\"; name it after the plugin id, {}.",
            m.id
        ));
    } else if m.id != id {
        r.problems.push(format!(
            "The folder is called \"{id}\" but plugin.toml says id = \"{}\"; they must match.",
            m.id
        ));
    }
    if m.is_reserved() && !system {
        r.problems.push(format!(
            "The id {} and ids starting with {} are reserved for WolOS; use a reverse-DNS or user.name id.",
            shared::plugins::RESERVED_PREFIX.trim_end_matches('.'),
            shared::plugins::RESERVED_PREFIX
        ));
    }
    match walk(dir) {
        Err(e) => r.problems.push(e),
        Ok(files) => {
            r.size = files
                .iter()
                .map(|(_, p)| std::fs::metadata(p).map_or(0, |m| m.len()))
                .sum();
            if r.size > MAX_TOTAL_SIZE {
                r.problems.push(format!(
                    "The plugin is {} KB; the limit is {} KB.",
                    r.size / 1024,
                    MAX_TOTAL_SIZE / 1024
                ));
            }
            r.files = files.into_iter().map(|(rel, _)| rel).collect();
        }
    }
    for f in m.files() {
        if !r.files.iter().any(|x| x == f) {
            r.problems.push(format!(
                "plugin.toml names {f}, but the folder has no such file."
            ));
        }
    }
    // Audit X6-1: these kinds parse, but nothing applies them yet; a check that passed them
    // promised a plugin that does nothing.
    for k in m.kinds().into_iter().filter(|k| k.is_planned()) {
        r.problems.push(k.planned_sentence());
    }
    let read = |rel: &str| std::fs::read_to_string(dir.join(rel)).ok();
    for e in &m.hypr {
        if let Some(t) = read(&e.file)
            && let Err(p) = check_hypr(&t)
        {
            r.problems.push(format!("{}: {p}", e.file));
        }
    }
    for e in &m.theme {
        if let Some(t) = read(&e.file) {
            for p in check_theme(&t, dir) {
                r.problems.push(format!("{}: {p}", e.file));
            }
        }
    }
    for e in &m.look {
        if let Some(t) = read(&e.file)
            && let Err(p) = crate::looks::Preset::parse(&e.id, &t)
        {
            r.problems.push(format!("{}: {p}", e.file));
        }
    }
    r.manifest = Some(m);
    r
}

/// The full check of `wol plugin check` and Plugin Maker: [`check`], then, when the folder is
/// clean, the Lua step `lua` (given the folder and the manifest; `wol_lua::plugin::lua_problems`
/// loads and runs every script once). Core runs no Lua itself, so the step is passed in. The bad
/// path is the report's `problems`, folder problems first; Lua is not run on a folder with any.
pub fn check_full(
    dir: &Path,
    system: bool,
    lua: impl FnOnce(&Path, &shared::plugins::Manifest) -> Vec<String>,
) -> Report {
    let mut r = check(dir, system);
    if let Some(m) = &r.manifest
        && r.problems.is_empty()
    {
        r.problems = lua(dir, m);
    }
    r
}

/// A plugin theme file: parses as `theme.json`; without `extends` every token must be there and
/// readable. (A theme that extends another is resolved by the catalogue at switch time.)
pub fn check_theme(json: &str, dir: &Path) -> Vec<String> {
    let spec = match crate::theme::ThemeSpec::parse(json, dir) {
        Ok(s) => s,
        Err(e) => return vec![format!("not a theme ({e})")],
    };
    if spec.extends.is_some() {
        return Vec::new();
    }
    shared::theme::validate(|t| spec.tokens.get(t).cloned())
}

/// One folder; the folder name is the id. The manifest is kept only when the folder checks out.
pub fn load(dir: &Path, system: bool) -> Plugin {
    let r = check(dir, system);
    Plugin {
        id: r.id,
        dir: dir.to_path_buf(),
        system,
        manifest: match (r.manifest, r.problems.first()) {
            (Some(m), None) => Ok(m),
            (_, Some(p)) => Err(p.clone()),
            (None, None) => Err("The plugin could not be read.".into()),
        },
    }
}

/// Every folder with a `plugin.toml` in `dirs`, sorted by name; first dir wins on equal ids,
/// except that a broken entry never hides a later good one of the same id (audit S4-25: a user
/// folder `org.wolos.stats` refused as reserved does not take the shipped plugin's place).
pub fn load_all(dirs: &[(PathBuf, bool)]) -> Vec<Plugin> {
    let mut out: Vec<Plugin> = Vec::new();
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
            let pkg = load(&p, *system);
            match out.iter_mut().find(|q| q.id == pkg.id) {
                None => out.push(pkg),
                Some(q) if q.manifest.is_err() && pkg.manifest.is_ok() => *q = pkg,
                Some(_) => {}
            }
        }
    }
    out.sort_by_key(|a| a.name().to_lowercase());
    out
}

// ---- Enabled plugins and grants (`plugins.json`) ----

/// What one enabled plugin was granted when it was turned on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grant {
    pub permissions: Vec<Permission>,
    /// Hosts `wol.http_get` may contact (each asked once).
    pub hosts: Vec<String>,
}

/// `{"enabled": {"<id>": {"permissions": [...], "hosts": [...]}}}`. No file = nothing on; a
/// malformed file = nothing on (fail closed, like the helper policy). Enabling grants exactly the
/// permissions the manifest declared at that moment; a manifest that later asks for more is shown
/// as needing a new grant.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Enabled {
    pub enabled: BTreeMap<String, Grant>,
}

impl Enabled {
    pub fn parse(text: Option<&str>) -> Self {
        text.and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.enabled.contains_key(id)
    }

    /// Turn on and grant what the manifest declares.
    pub fn enable(&mut self, m: &Manifest) {
        let g = self.enabled.entry(m.id.clone()).or_default();
        g.permissions = m.permissions.clone();
    }

    pub fn disable(&mut self, id: &str) {
        self.enabled.remove(id);
    }

    /// Permissions the manifest asks for that were not granted (a newer manifest).
    pub fn missing(&self, m: &Manifest) -> Vec<Permission> {
        let got = self.enabled.get(&m.id);
        m.permissions
            .iter()
            .copied()
            .filter(|p| !got.is_some_and(|g| g.permissions.contains(p)))
            .collect()
    }

    /// An enabled plugin may use `p` now.
    pub fn allows(&self, id: &str, p: Permission) -> bool {
        self.enabled
            .get(id)
            .is_some_and(|g| g.permissions.contains(&p))
    }

    pub fn allows_host(&self, id: &str, host: &str) -> bool {
        self.enabled
            .get(id)
            .is_some_and(|g| g.hosts.iter().any(|h| h == host))
    }

    /// Remember a host grant (consent sheet). Nothing for a plugin that is off.
    pub fn allow_host(&mut self, id: &str, host: &str) {
        if let Some(g) = self.enabled.get_mut(id)
            && !g.hosts.iter().any(|h| h == host)
        {
            g.hosts.push(host.to_ascii_lowercase());
            g.hosts.sort();
        }
    }
}

pub fn enabled_path(config_home: &Path) -> PathBuf {
    config_home.join(ENABLED_FILE)
}

pub fn load_enabled(config_home: &Path) -> Enabled {
    Enabled::parse(
        std::fs::read_to_string(enabled_path(config_home))
            .ok()
            .as_deref(),
    )
}

pub fn save_enabled(config_home: &Path, e: &Enabled) -> Result<(), String> {
    write_atomic(&enabled_path(config_home), &e.to_json())
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| "The file has no folder.".to_owned())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    shared::atomic::atomic_write(path, 0o644, text.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}

// ---- Per-plugin state (`wol.state`) ----

/// `$XDG_STATE_HOME/wolos/plugins/<id>/state.json`, the only place a plugin writes.
pub fn state_path(state_home: &Path, id: &str) -> PathBuf {
    state_home.join(STATE_DIR).join(id).join("state.json")
}

/// The saved JSON object as text; None when nothing was saved or the file is not an object.
pub fn load_state(state_home: &Path, id: &str) -> Option<String> {
    let text = std::fs::read_to_string(state_path(state_home, id)).ok()?;
    serde_json::from_str::<Value>(&text)
        .ok()
        .filter(Value::is_object)
        .map(|_| text)
}

/// Saves a JSON object of at most [`MAX_STATE`] bytes, atomically.
pub fn save_state(state_home: &Path, id: &str, json: &str) -> Result<(), String> {
    if !is_plugin_id(id) {
        return Err("Not a plugin id.".into());
    }
    if json.len() > MAX_STATE {
        return Err(format!("Plugin state is at most {} KB.", MAX_STATE / 1024));
    }
    if !serde_json::from_str::<Value>(json).is_ok_and(|v| v.is_object()) {
        return Err("Plugin state is a JSON object.".into());
    }
    write_atomic(&state_path(state_home, id), json)
}

pub fn clear_state(state_home: &Path, id: &str) {
    if is_plugin_id(id) {
        let _ = std::fs::remove_dir_all(state_home.join(STATE_DIR).join(id));
    }
}

// ---- Toggles: a flag file per toggle ----

/// `$XDG_STATE_HOME/wolos/toggles/<plugin id>.<toggle id>`; present = on.
pub fn toggle_path(state_home: &Path, plugin: &str, toggle: &str) -> PathBuf {
    state_home
        .join(TOGGLES_DIR)
        .join(format!("{plugin}.{toggle}"))
}

pub fn toggle_get(state_home: &Path, plugin: &str, toggle: &str) -> bool {
    toggle_path(state_home, plugin, toggle).is_file()
}

/// Sets the flag. Running the toggle's `on`/`off` command is the caller's (the host checks the
/// grant, `run` for anything but a `wol` verb, runs it limited, then calls this on success).
pub fn toggle_set(state_home: &Path, plugin: &str, toggle: &str, on: bool) -> Result<(), String> {
    if !is_plugin_id(plugin) || !shared::plugins::is_entry_id(toggle) {
        return Err("Not a toggle of a plugin.".into());
    }
    let p = toggle_path(state_home, plugin, toggle);
    if on {
        write_atomic(&p, "on\n")
    } else {
        match std::fs::remove_file(&p) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {e}", p.display())),
        }
    }
}

// ---- Web apps (`[[webapp]]`) ----

/// Key in a plugin web app launcher: the plugin it came from (next to the web app mark of
/// `crate::webapps`, so `wol webapp list` shows it and the plugin's turn-off removes it).
pub const WEBAPP_PLUGIN_KEY: &str = "X-WolOS-Plugin";

/// The launchers of `m`'s `[[webapp]]` entries (audit X6-1): the same `.desktop` files `wol webapp
/// add` writes (`crate::webapps`), id `plugin-<plugin>-<entry>`, icon a file of the plugin
/// folder `dir` or an icon name, each marked with [`WEBAPP_PLUGIN_KEY`]. (file, text); an entry
/// whose name or address does not check is left out.
pub fn webapp_launchers(m: &Manifest, dir: &Path, data_home: &Path) -> Vec<(PathBuf, String)> {
    use crate::webapps as wa;
    m.webapp
        .iter()
        .filter_map(|e| {
            let profile = match e.profile {
                shared::plugins::Profile::Shared => wa::Profile::Shared,
                shared::plugins::Profile::Own => wa::Profile::Own,
            };
            let mut app = wa::WebApp::new(&e.name, &e.url, profile).ok()?;
            app.id = wa::slug(&format!("plugin-{}-{}", m.id, e.id))?;
            let icon = if shared::plugins::is_icon_file(&e.icon) {
                dir.join(&e.icon).to_string_lossy().into_owned()
            } else {
                e.icon.clone()
            };
            if wa::check_icon(&icon) {
                app.icon = icon;
            }
            let text = format!(
                "{}{WEBAPP_PLUGIN_KEY}={}\n",
                wa::desktop_entry(&app, data_home),
                m.id
            );
            Some((wa::desktop_path(data_home, &app.id), text))
        })
        .collect()
}

/// Whether the launcher text came from plugin `id`.
fn is_launcher_of(text: &str, id: &str) -> bool {
    text.lines().any(|l| {
        l.trim()
            .strip_prefix(WEBAPP_PLUGIN_KEY)
            .and_then(|r| r.strip_prefix('='))
            == Some(id)
    })
}

/// Puts plugin `id`'s web app launchers in `<data home>/applications`: removes every launcher
/// of `id` there, then, when `m` is given (the plugin is on), writes [`webapp_launchers`]. A file
/// of the same name that is not this plugin's (a web app of the person's own) is never
/// replaced; the names of those skipped are returned.
pub fn sync_webapps(
    data_home: &Path,
    id: &str,
    m: Option<(&Manifest, &Path)>,
) -> Result<Vec<String>, String> {
    let apps = data_home.join("applications");
    if let Ok(rd) = std::fs::read_dir(&apps) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if crate::webapps::id_of_file(&name).is_none() {
                continue;
            }
            let p = e.path();
            if std::fs::read_to_string(&p).is_ok_and(|t| is_launcher_of(&t, id)) {
                std::fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            }
        }
    }
    let mut skipped = Vec::new();
    let Some((m, dir)) = m else {
        return Ok(skipped);
    };
    for (file, text) in webapp_launchers(m, dir, data_home) {
        if file.symlink_metadata().is_ok() {
            skipped.push(
                file.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
            continue;
        }
        write_atomic(&file, &text)?;
    }
    Ok(skipped)
}

// ---- Render failures ----

/// Counts failed renders per plugin; [`MAX_FAILURES`] in a row disables it with a toast.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Health {
    fails: BTreeMap<String, u32>,
}

impl Health {
    /// A render or click failed with `why`. Some(toast) when the plugin must be turned off now.
    pub fn failed(&mut self, id: &str, name: &str, why: &str) -> Option<String> {
        let n = {
            let n = self.fails.entry(id.to_owned()).or_default();
            *n += 1;
            *n
        };
        if n < MAX_FAILURES {
            return None;
        }
        self.fails.remove(id);
        Some(format!(
            "{name} was turned off after {MAX_FAILURES} failures in a row: {why}"
        ))
    }

    /// A render worked: the streak ends.
    pub fn ok(&mut self, id: &str) {
        self.fails.remove(id);
    }

    pub fn count(&self, id: &str) -> u32 {
        self.fails.get(id).copied().unwrap_or(0)
    }
}

// ---- `.wolplug`: tar of the folder + SHA256, compressed with zstd by the caller ----

/// What `wol plugin pack` writes before compression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pack {
    /// `<id>-<version>.wolplug`.
    pub name: String,
    /// ustar: `<id>/<file>` for every file, then `SHA256`.
    pub tar: Vec<u8>,
    /// The `SHA256` member's text (ADR 0011 lines, sorted).
    pub sha256: String,
}

pub fn pack_name(m: &Manifest) -> String {
    format!("{}-{}.{PACK_EXT}", m.id, m.version)
}

/// Archives a checked folder. Members are `<id>/<relative path>`; `SHA256` lists each of them.
pub fn pack(dir: &Path, m: &Manifest) -> Result<Pack, String> {
    let files = walk(dir)?;
    let mut members: Vec<(String, Vec<u8>)> = Vec::new();
    for (rel, p) in files {
        let bytes = std::fs::read(&p).map_err(|_| format!("The file {rel} could not be read."))?;
        members.push((format!("{}/{rel}", m.id), bytes));
    }
    let refs: Vec<(&str, &[u8])> = members
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    let sha256 = crate::package::manifest(&refs);
    let mut all = refs;
    all.push((PACK_MANIFEST, sha256.as_bytes()));
    if all.iter().any(|(n, _)| n.len() > 99) {
        return Err("A file path inside the plugin is longer than 99 characters.".into());
    }
    Ok(Pack {
        name: pack_name(m),
        tar: crate::package::ustar(&all),
        sha256,
    })
}

// ---- `wol plugin new` ----

const LUA_MENUBAR: &str = r#"-- Menu bar item: rendered every `refresh` seconds; `popover` opens on click.
local clicks = 0
return wol.menubar {
  refresh = 60,
  render = function()
    local t = wol.now()
    return wol.row { spacing = 4,
      wol.image { "emblem-default-symbolic", size = 16 },
      wol.label { string.format("%02d:%02d", t.hour, t.minute) },
    }
  end,
  popover = function()
    return wol.stack { spacing = 8,
      wol.label { "Hello from __ID__", role = "title" },
      wol.label { "Clicked " .. clicks .. " times", role = "secondary" },
      wol.button { label = "Click", on_click = function() clicks = clicks + 1 end },
    }
  end,
}
"#;

const LUA_WIDGET: &str = r#"-- Desktop card: `render` returns the view; buttons re-render after their handler runs.
-- Loading never acts: `wol.state` and the other services exist from the first render on.
local n
return wol.widget {
  refresh = 0,
  render = function()
    n = n or wol.state.get("count") or 0
    return wol.stack { spacing = 8,
      wol.label { "Count: " .. n, role = "display" },
      wol.row { spacing = 8,
        wol.button { label = "+1", on_click = function() n = n + 1; wol.state.set("count", n) end },
        wol.button { label = "Reset", on_click = function() n = 0; wol.state.set("count", nil) end },
      },
    }
  end,
}
"#;

const LUA_PANEL: &str = r#"-- Panel: a sheet the shell shows on `wol.show("__ENTRY__")` or a hotkey.
return wol.panel {
  title = "__NAME__",
  render = function()
    return wol.list {
      wol.item { title = "First row", subtitle = "Rows are wol.item { title, subtitle, icon }", icon = "emblem-default-symbolic" },
      wol.item { title = "Second row", button = { label = "Open", on_click = function() end } },
    }
  end,
}
"#;

const LUA_SEARCH: &str = r#"-- Search: `query(text)` returns up to 20 rows; `on_activate` runs when one is chosen.
return wol.search {
  query = function(text)
    if text == "" then return {} end
    return {
      { title = "Echo: " .. text, subtitle = "__NAME__", icon = "edit-find-symbolic", on_activate = function() end },
    }
  end,
}
"#;

const LUA_IPC: &str = r#"-- IPC: `wol shell call __ID__ ping '{"hello": 1}'` runs `ping` and prints what it returns.
-- One function per [[ipc]] entry, named like the entry; `return nil, "sentence"` refuses.
return wol.ipc {
  ping = function(args)
    return { pong = args }
  end,
}
"#;

const THEME_JSON: &str = r##"{
  "id": "__ENTRY__",
  "name": "__NAME__",
  "variant": "dark",
  "tokens": {
    "surface": "#1b1d24",
    "sidebar": "rgba(27,29,36,0.92)",
    "card": "#252834",
    "line": "rgba(255,255,255,0.08)",
    "text": "#f2f3f7",
    "dim": "#a9adbb",
    "accent": "#8ab4ff",
    "accentText": "#0b1020",
    "glass": "rgba(27,29,36,0.84)",
    "control": "#2e3242",
    "success": "#7fc08a",
    "warning": "#e0b25a",
    "danger": "#e07070"
  }
}
"##;

/// Files for a new plugin: `plugin.toml` with one entry per kind in `kinds`, and the scripts and
/// files they name. The result passes `wol plugin check` as is.
pub fn scaffold(id: &str, name: &str, kinds: &[Kind]) -> Result<Vec<(String, String)>, String> {
    if !is_plugin_id(id) {
        return Err(format!(
            "A plugin id is a reverse-DNS or user.name id of lowercase labels, like \"alice.{}\".",
            id.rsplit('.').next().unwrap_or("demo").to_ascii_lowercase()
        ));
    }
    if let Some(k) = kinds.iter().find(|k| k.is_planned()) {
        return Err(k.planned_sentence());
    }
    let mut kinds: Vec<Kind> = kinds.to_vec();
    kinds.dedup();
    if kinds.is_empty() {
        kinds = vec![Kind::Menubar, Kind::Widget];
    }
    if kinds.contains(&Kind::Tile) && !kinds.contains(&Kind::Toggle) {
        kinds.push(Kind::Toggle);
    }
    let fill = |t: &str, entry: &str| {
        t.replace("__ID__", id)
            .replace("__NAME__", name)
            .replace("__ENTRY__", entry)
    };
    let mut toml = format!(
        "schema = 1\nid = \"{id}\"\nname = \"{name}\"\nversion = \"0.1\"\nauthor = \"\"\nlicense = \"MIT\"\ndescription = \"What it does, in one line.\"\nmin_wolos = \"0.12\"\n"
    );
    let mut perms: Vec<&str> = Vec::new();
    if kinds.contains(&Kind::Dock) {
        // The Dock entry runs `foot`, not a `wol` verb.
        perms.push("run");
    }
    if kinds.contains(&Kind::Toggle) {
        perms.push("toggles");
    }
    if !perms.is_empty() {
        toml.push_str(&format!(
            "permissions = [{}]\n",
            perms
                .iter()
                .map(|p| format!("\"{p}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut files: Vec<(String, String)> = Vec::new();
    for k in &kinds {
        let section = match k {
            Kind::Menubar => {
                files.push(("bar.lua".into(), fill(LUA_MENUBAR, "bar")));
                "\n[[menubar]]\nid = \"bar\"\nscript = \"bar.lua\"\nside = \"right\"\n".to_owned()
            }
            Kind::Dock => format!(
                "\n[[dock]]\nid = \"launch\"\nname = \"{name}\"\nicon = \"utilities-terminal-symbolic\"\nexec = [\"foot\"]\n"
            ),
            Kind::Widget => {
                files.push(("card.lua".into(), fill(LUA_WIDGET, "card")));
                "\n[[widget]]\nid = \"card\"\nscript = \"card.lua\"\nsizes = [\"small\", \"medium\"]\n"
                    .to_owned()
            }
            Kind::Panel => {
                files.push(("panel.lua".into(), fill(LUA_PANEL, "panel")));
                "\n[[panel]]\nid = \"panel\"\nscript = \"panel.lua\"\nanchor = \"top\"\n".to_owned()
            }
            Kind::Search => {
                files.push(("search.lua".into(), fill(LUA_SEARCH, "search")));
                "\n[[search]]\nid = \"search\"\nscript = \"search.lua\"\nkind = \"command\"\nprefix = \"?\"\n"
                    .to_owned()
            }
            Kind::Tile => format!(
                "\n[[tile]]\nid = \"tile\"\nicon = \"weather-clear-night-symbolic\"\nlabel = \"{name}\"\ntoggle = \"switch\"\n"
            ),
            Kind::Toggle => {
                "\n[[toggle]]\nid = \"switch\"\non = [\"wol\", \"focus\", \"on\"]\noff = [\"wol\", \"focus\", \"off\"]\n"
                    .to_owned()
            }
            Kind::Hotkey => format!(
                "\n[[hotkey]]\nid = \"open\"\ndefault = \"super+shift+d\"\nexec = [\"wol\", \"shell\", \"toggle\", \"control-center\"]\ngroup = \"{name}\"\n"
            ),
            Kind::Webapp => format!(
                "\n[[webapp]]\nid = \"web\"\nname = \"{name}\"\nurl = \"https://example.com/\"\nicon = \"web-browser-symbolic\"\nprofile = \"shared\"\n"
            ),
            Kind::Theme => {
                files.push(("theme.json".into(), fill(THEME_JSON, "theme")));
                "\n[[theme]]\nid = \"theme\"\nfile = \"theme.json\"\n".to_owned()
            }
            Kind::Ipc => {
                files.push(("ipc.lua".into(), fill(LUA_IPC, "ping")));
                "\n[[ipc]]\nid = \"ping\"\nscript = \"ipc.lua\"\nargs = \"any\"\n".to_owned()
            }
            // Refused above (audit X6-1).
            Kind::Look | Kind::Hypr => continue,
        };
        toml.push_str(&section);
    }
    files.insert(0, (MANIFEST_FILE.into(), toml));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_full_check_runs_the_lua_step_only_on_a_clean_folder() {
        let d = tmp("full");
        let dir = d.join("alice.full");
        std::fs::create_dir_all(&dir).unwrap();
        for (rel, text) in scaffold("alice.full", "Full", &[Kind::Menubar]).unwrap() {
            std::fs::write(dir.join(rel), text).unwrap();
        }
        let r = check_full(&dir, false, |at, m| {
            assert_eq!(at, dir.as_path());
            vec![format!("{}: ran", m.id)]
        });
        assert_eq!(r.problems, ["alice.full: ran"]);
        std::fs::remove_file(dir.join("bar.lua")).unwrap();
        let r = check_full(&dir, false, |_, _| panic!("no Lua on a broken folder"));
        assert_eq!(
            r.problems,
            ["plugin.toml names bar.lua, but the folder has no such file."]
        );
        let _ = std::fs::remove_dir_all(d);
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wol-plugins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_all(dir: &Path, files: &[(String, String)]) {
        for (rel, text) in files {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
    }

    /// Every scaffold (each kind alone and all together) passes check: the agent's first run is green.
    #[test]
    fn scaffolds_check_clean_and_name_their_files() {
        let root = tmp("scaffold");
        let offered: Vec<Kind> = Kind::ALL.into_iter().filter(|k| !k.is_planned()).collect();
        let mut sets: Vec<Vec<Kind>> = offered.iter().map(|k| vec![*k]).collect();
        sets.push(offered.clone());
        sets.push(Vec::new());
        // Audit X6-1: planned kinds are not scaffolded.
        for k in Kind::PLANNED {
            assert_eq!(
                scaffold("alice.x", "X", &[k]).unwrap_err(),
                k.planned_sentence()
            );
        }
        for (i, kinds) in sets.iter().enumerate() {
            let id = format!("alice.demo{i}");
            let files = scaffold(&id, "Demo", kinds).unwrap();
            let dir = root.join(&id);
            write_all(&dir, &files);
            let r = check(&dir, false);
            assert_eq!(
                r.problems,
                Vec::<String>::new(),
                "{kinds:?}\n{}",
                files[0].1
            );
            let m = r.manifest.unwrap();
            assert_eq!(m.id, id);
            let want: Vec<Kind> = if kinds.is_empty() {
                vec![Kind::Menubar, Kind::Widget]
            } else {
                let mut k = kinds.clone();
                if k.contains(&Kind::Tile) && !k.contains(&Kind::Toggle) {
                    k.push(Kind::Toggle);
                }
                k.sort();
                k
            };
            assert_eq!(m.kinds(), want, "{kinds:?}");
            for f in m.files() {
                assert!(r.files.iter().any(|x| x == f), "{f} missing for {kinds:?}");
            }
        }
        assert!(
            scaffold("Demo", "Demo", &[])
                .unwrap_err()
                .contains("alice.demo")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Audit X6-1: an enabled plugin's `[[webapp]]` is a launcher next to the person's own web
    /// apps; turning it off removes it and never touches a web app of theirs.
    #[test]
    fn webapp_entries_become_launchers_and_go_away() {
        let root = tmp("webapp");
        let data = root.join("data");
        let dir = root.join("alice.web");
        write_all(
            &dir,
            &scaffold("alice.web", "Web", &[Kind::Webapp]).unwrap(),
        );
        let m = check(&dir, false).manifest.unwrap();
        let l = webapp_launchers(&m, &dir, &data);
        assert_eq!(l.len(), 1);
        let (file, text) = &l[0];
        assert_eq!(
            file,
            &data.join("applications/wolos-webapp-plugin-alice-web-web.desktop")
        );
        assert!(text.contains("--app=https://example.com/"), "{text}");
        assert!(text.ends_with("X-WolOS-Plugin=alice.web\n"), "{text}");
        let app = crate::webapps::parse_entry("plugin-alice-web-web", text).unwrap();
        assert_eq!(
            (app.name.as_str(), app.icon.as_str()),
            ("Web", "web-browser-symbolic")
        );

        // The person's own web app, and one of another plugin.
        let apps = data.join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        let mine = apps.join("wolos-webapp-mail.desktop");
        std::fs::write(&mine, "[Desktop Entry]\nName=Mail\n").unwrap();
        let other = apps.join("wolos-webapp-plugin-bob-x-web.desktop");
        std::fs::write(&other, "[Desktop Entry]\nX-WolOS-Plugin=bob.x\n").unwrap();

        assert!(
            sync_webapps(&data, &m.id, Some((&m, &dir)))
                .unwrap()
                .is_empty()
        );
        assert_eq!(std::fs::read_to_string(file).unwrap(), *text);
        // Off: only this plugin's launcher goes.
        sync_webapps(&data, &m.id, None).unwrap();
        assert!(!file.exists());
        assert!(mine.is_file() && other.is_file());
        // A file of the same name that is not ours is kept and reported.
        std::fs::write(file, "[Desktop Entry]\nName=Mine\n").unwrap();
        let skipped = sync_webapps(&data, &m.id, Some((&m, &dir))).unwrap();
        assert_eq!(skipped, ["wolos-webapp-plugin-alice-web-web.desktop"]);
        assert_eq!(
            std::fs::read_to_string(file).unwrap(),
            "[Desktop Entry]\nName=Mine\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn check_names_folder_problems() {
        let root = tmp("check");
        let files = scaffold("alice.demo", "Demo", &[Kind::Menubar, Kind::Theme]).unwrap();
        let dir = root.join("alice.demo");
        write_all(&dir, &files);
        assert!(check(&dir, false).problems.is_empty());
        // Folder name must be the id.
        let other = root.join("alice.other");
        std::fs::rename(&dir, &other).unwrap();
        let r = check(&other, false);
        assert!(
            r.problems[0].contains("they must match"),
            "{:?}",
            r.problems
        );
        std::fs::rename(&other, &dir).unwrap();
        // A file the manifest names is missing.
        std::fs::remove_file(dir.join("bar.lua")).unwrap();
        let r = check(&dir, false);
        assert!(
            r.problems.iter().any(|p| p.contains("no such file")),
            "{:?}",
            r.problems
        );
        std::fs::write(dir.join("bar.lua"), "return 1").unwrap();
        // Audit X6-1: [[hypr]] and [[look]] parse but nothing applies them, so check refuses
        // them (the compositor whitelist still runs: shared::plugins::check_hypr).
        let toml = std::fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap();
        std::fs::write(
            dir.join(MANIFEST_FILE),
            format!("{toml}\n[[hypr]]\nid = \"rules\"\nfile = \"rules.conf\"\n"),
        )
        .unwrap();
        std::fs::write(dir.join("rules.conf"), "bind = SUPER, X, exec, rm\n").unwrap();
        let r = check(&dir, false);
        assert!(
            r.problems.contains(&Kind::Hypr.planned_sentence()),
            "{:?}",
            r.problems
        );
        assert!(
            r.problems
                .iter()
                .any(|p| p.starts_with("rules.conf:") && p.contains("\"bind\"")),
            "{:?}",
            r.problems
        );
        std::fs::write(dir.join(MANIFEST_FILE), toml).unwrap();
        std::fs::remove_file(dir.join("rules.conf")).unwrap();
        let theme = std::fs::read_to_string(dir.join("theme.json")).unwrap();
        std::fs::write(dir.join("theme.json"), theme.replace("#f2f3f7", "#1b1d24")).unwrap();
        let r = check(&dir, false);
        assert!(
            r.problems.iter().any(|p| p.contains("text/surface")),
            "{:?}",
            r.problems
        );
        std::fs::write(dir.join("theme.json"), "{").unwrap();
        assert!(
            check(&dir, false)
                .problems
                .iter()
                .any(|p| p.contains("not a theme"))
        );
        std::fs::write(dir.join("theme.json"), theme).unwrap();
        // Hidden files, symlinks, size, reserved ids.
        std::fs::write(dir.join(".secret"), "x").unwrap();
        assert!(check(&dir, false).problems[0].contains("hidden"));
        std::fs::remove_file(dir.join(".secret")).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", dir.join("link.lua")).unwrap();
        assert!(check(&dir, false).problems[0].contains("symlink"));
        std::fs::remove_file(dir.join("link.lua")).unwrap();
        std::fs::write(dir.join("big.bin"), vec![0u8; MAX_FILE as usize + 1]).unwrap();
        assert!(check(&dir, false).problems[0].contains("larger than"));
        std::fs::remove_file(dir.join("big.bin")).unwrap();
        let reserved = root.join("org.wolos.x");
        write_all(
            &reserved,
            &scaffold("org.wolos.x", "X", &[Kind::Dock]).unwrap(),
        );
        assert!(check(&reserved, false).problems[0].contains("reserved"));
        assert!(
            check(&reserved, true).problems.is_empty(),
            "system folders may ship org.wolos.*"
        );
        let bare = root.join("org.wolos");
        write_all(&bare, &scaffold("org.wolos", "X", &[Kind::Toggle]).unwrap());
        assert!(check(&bare, false).problems[0].contains("reserved"));
        std::fs::remove_dir_all(&bare).unwrap();
        assert!(check(&root.join("nope"), false).problems[0].contains("plugin.toml"));
        // load keeps the manifest only for a clean folder; load_all orders by name, user first.
        assert!(load(&dir, false).manifest.is_ok());
        assert!(load(&reserved, false).manifest.is_err());
        let all = load_all(&[(root.clone(), false)]);
        assert_eq!(
            all.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["alice.demo", "org.wolos.x"]
        );
        assert_eq!(
            dirs(Path::new("/h/.local/share"))[0].0,
            Path::new("/h/.local/share/wolos/plugins")
        );
        // Audit S4-25: the user's refused org.wolos.x does not shadow the shipped one.
        let system = tmp("shipped");
        write_all(
            &system.join("org.wolos.x"),
            &scaffold("org.wolos.x", "X", &[Kind::Dock]).unwrap(),
        );
        let all = load_all(&[(root.clone(), false), (system.clone(), true)]);
        let x: Vec<&Plugin> = all.iter().filter(|p| p.id == "org.wolos.x").collect();
        assert_eq!(x.len(), 1);
        assert!(x[0].system && x[0].manifest.is_ok(), "{:?}", x[0].manifest);
        // A good user plugin still wins over the same id shipped.
        let all = load_all(&[
            (root.clone(), false),
            (system.clone(), true),
            (root.clone(), true),
        ]);
        let demo: Vec<&Plugin> = all.iter().filter(|p| p.id == "alice.demo").collect();
        assert_eq!(demo.len(), 1);
        assert!(!demo[0].system);
        std::fs::remove_dir_all(system).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn enabled_grants_round_trip_and_fail_closed() {
        let m = shared::plugins::parse(
            "schema = 1\nid = \"alice.demo\"\nname = \"Demo\"\nversion = \"1\"\nlicense = \"MIT\"\npermissions = [\"run\", \"http\"]\n[[dock]]\nid = \"d\"\nname = \"D\"\nicon = \"x\"\nexec = [\"x\"]\n",
        )
        .unwrap();
        let mut e = Enabled::parse(None);
        assert!(!e.is_enabled("alice.demo"));
        assert_eq!(e.missing(&m), [Permission::Run, Permission::Http]);
        e.enable(&m);
        assert!(e.is_enabled("alice.demo"));
        assert!(e.allows("alice.demo", Permission::Run));
        assert!(!e.allows("alice.demo", Permission::Clipboard));
        assert!(e.missing(&m).is_empty());
        assert!(!e.allows_host("alice.demo", "api.example.com"));
        e.allow_host("alice.demo", "API.example.com");
        e.allow_host("alice.demo", "api.example.com");
        assert!(e.allows_host("alice.demo", "api.example.com"));
        assert_eq!(e.enabled["alice.demo"].hosts, ["api.example.com"]);
        assert_eq!(Enabled::parse(Some(&e.to_json())), e);
        assert_eq!(
            Enabled::parse(Some("{nope")),
            Enabled::default(),
            "malformed = nothing on"
        );
        e.allow_host("bob.x", "h");
        assert!(!e.is_enabled("bob.x"), "hosts only for enabled plugins");
        e.disable("alice.demo");
        assert!(!e.is_enabled("alice.demo"));
        let home = tmp("enabled");
        save_enabled(&home, &e).unwrap();
        assert_eq!(load_enabled(&home), e);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn state_and_toggles_live_under_state_home() {
        let sh = tmp("state");
        assert_eq!(load_state(&sh, "alice.demo"), None);
        save_state(&sh, "alice.demo", r#"{"count": 3}"#).unwrap();
        assert_eq!(
            load_state(&sh, "alice.demo").as_deref(),
            Some(r#"{"count": 3}"#)
        );
        assert_eq!(
            state_path(&sh, "alice.demo"),
            sh.join("wolos/plugins/alice.demo/state.json")
        );
        assert!(
            save_state(&sh, "alice.demo", "[1]")
                .unwrap_err()
                .contains("object")
        );
        assert!(save_state(&sh, "../x", "{}").is_err());
        let big = format!("{{\"k\": \"{}\"}}", "x".repeat(MAX_STATE));
        assert!(
            save_state(&sh, "alice.demo", &big)
                .unwrap_err()
                .contains("64 KB")
        );
        clear_state(&sh, "alice.demo");
        assert_eq!(load_state(&sh, "alice.demo"), None);

        assert!(!toggle_get(&sh, "alice.demo", "awake"));
        toggle_set(&sh, "alice.demo", "awake", true).unwrap();
        assert!(toggle_get(&sh, "alice.demo", "awake"));
        assert_eq!(
            toggle_path(&sh, "alice.demo", "awake"),
            sh.join("wolos/toggles/alice.demo.awake")
        );
        toggle_set(&sh, "alice.demo", "awake", false).unwrap();
        toggle_set(&sh, "alice.demo", "awake", false).unwrap();
        assert!(!toggle_get(&sh, "alice.demo", "awake"));
        assert!(toggle_set(&sh, "alice.demo", "../x", true).is_err());
        std::fs::remove_dir_all(sh).unwrap();
    }

    #[test]
    fn three_failures_in_a_row_disable_with_a_sentence() {
        let mut h = Health::default();
        assert_eq!(h.failed("a.b", "Demo", "boom"), None);
        assert_eq!(h.failed("a.b", "Demo", "boom"), None);
        h.ok("a.b");
        assert_eq!(h.count("a.b"), 0, "a good render ends the streak");
        for _ in 0..2 {
            assert_eq!(h.failed("a.b", "Demo", "boom"), None);
        }
        assert_eq!(
            h.failed("a.b", "Demo", "The widget took too long."),
            Some("Demo was turned off after 3 failures in a row: The widget took too long.".into())
        );
        assert_eq!(h.count("a.b"), 0);
    }

    #[test]
    fn pack_is_a_ustar_of_the_folder_plus_sha256() {
        let root = tmp("pack");
        let files = scaffold("alice.demo", "Demo", &[Kind::Menubar]).unwrap();
        let dir = root.join("alice.demo");
        write_all(&dir, &files);
        let m = check(&dir, false).manifest.unwrap();
        let p = pack(&dir, &m).unwrap();
        assert_eq!(p.name, "alice.demo-0.1.wolplug");
        let lines: Vec<&str> = p.sha256.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("SHA256 (alice.demo/bar.lua) = "));
        assert!(lines[1].starts_with("SHA256 (alice.demo/plugin.toml) = "));
        let tar_path = root.join("x.tar");
        std::fs::write(&tar_path, &p.tar).unwrap();
        let members = crate::package::members(&tar_path).unwrap();
        let names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            ["alice.demo/bar.lua", "alice.demo/plugin.toml", "SHA256"]
        );
        assert_eq!(p.tar.len() % 512, 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
