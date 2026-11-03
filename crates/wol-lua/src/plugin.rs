// SPDX-License-Identifier: GPL-2.0-or-later
//! Plugin scripts (ADR 0031, docs/modules/plugins.md): one Lua file per `[[menubar]]`, `[[widget]]`,
//! `[[panel]]` or `[[search]]` entry, run in the widget sandbox with API v1: the view parts, and
//! the service calls the manifest's permissions allow (`wol.notify`, `wol.run`, `wol.http_get`,
//! `wol.open`, `wol.toggle_get`/`wol.toggle_set`, `wol.clipboard.text`, `wol.show`, `wol.state`).
//! Loading never acts: services are added after the file has run, so only `render`, `query` and
//! handlers reach them. Every call answers `ok, message` (or `value, message`) instead of raising,
//! so a refused call shows a sentence on the card, not a crash.
use crate::{
    Deadlines, Handlers, Input, Meter, Services, arm, err, grant_notify, refresh_of, sandbox,
    to_node,
};
use mlua::{Function, Lua, Table, Value};
use serde_json::{Map, Value as Json};
use shared::plugins::{Kind, MAX_STATE, Permission};
use shared::view::Node;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

/// `wol.run` timeout bounds (seconds).
pub const DEFAULT_TIMEOUT: u32 = 5;
pub const MAX_TIMEOUT: u32 = 30;
/// Rows a search script may return per query.
pub const MAX_HITS: usize = 20;
/// Nesting of a `wol.state` or `wol.json` value.
const MAX_STATE_DEPTH: usize = 16;
/// Largest text `wol.json.decode` reads.
pub const MAX_JSON: usize = 256 * 1024;
const MAX_TITLE: usize = 64;

/// One search result row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// Handler id for [`Script::activate`].
    pub id: u32,
    pub title: String,
    pub subtitle: String,
    /// Icon name or an image file inside the plugin folder (absolute here).
    pub icon: String,
}

/// A loaded entry script: `wol.menubar { refresh, render, popover }`, `wol.widget { refresh, render }`,
/// `wol.panel { title, render }` or `wol.search { query }`.
pub struct Script {
    lua: Lua,
    kind: Kind,
    plugin: String,
    entry: String,
    dir: PathBuf,
    refresh: u32,
    title: String,
    render: Option<Function>,
    popover: Option<Function>,
    query: Option<Function>,
    handlers: Handlers,
    meter: Arc<Meter>,
    deadlines: Deadlines,
}

impl std::fmt::Debug for Script {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Script")
            .field("kind", &self.kind)
            .field("plugin", &self.plugin)
            .field("entry", &self.entry)
            .finish()
    }
}

fn marker(kind: Kind) -> &'static str {
    match kind {
        Kind::Menubar => "wol.menubar",
        Kind::Widget => "wol.widget",
        Kind::Panel => "wol.panel",
        Kind::Search => "wol.search",
        _ => "wol.widget",
    }
}

/// Which manifest entry a script belongs to.
#[derive(Clone, Copy, Debug)]
pub struct Entry<'a> {
    pub kind: Kind,
    /// Plugin id (audit log, state, toggles).
    pub plugin: &'a str,
    /// Entry id in the manifest.
    pub id: &'a str,
    /// Plugin folder: image files resolve inside it.
    pub dir: &'a Path,
}

impl Script {
    /// Runs `src` (the entry's file) and keeps the table it returns. `permissions` are the
    /// manifest's; they decide which `wol.*` calls exist.
    pub fn load(
        at: Entry<'_>,
        src: &str,
        permissions: &[Permission],
        services: Rc<dyn Services>,
    ) -> Result<Self, String> {
        Self::load_within(at, src, permissions, services, Deadlines::DEFAULT)
    }

    /// [`Script::load`] with the host's [`Deadlines`] (the load itself runs under `deadlines.load`).
    pub fn load_within(
        at: Entry<'_>,
        src: &str,
        permissions: &[Permission],
        services: Rc<dyn Services>,
        deadlines: Deadlines,
    ) -> Result<Self, String> {
        let Entry {
            kind,
            plugin,
            id: entry,
            dir,
        } = at;
        if !kind.has_script() {
            return Err(format!("A [[{}]] entry has no script.", kind.id()));
        }
        let lua = sandbox(&services).map_err(err)?;
        let meter = Meter::new();
        arm(&lua, &meter, deadlines.load).map_err(err)?;
        let v: Value = meter.settle(
            lua.load(src)
                .set_name(format!("={plugin}/{entry}"))
                .eval()
                .map_err(err),
        )?;
        let want = marker(kind);
        let Value::Table(t) = v else {
            return Err(format!("The script must end with return {want} {{ ... }}."));
        };
        let got: Option<String> = t.get("__wol_kind").ok().flatten();
        if got.as_deref() != Some(kind.id()) {
            return Err(format!("The script must end with return {want} {{ ... }}."));
        }
        let func = |key: &str| -> Result<Option<Function>, String> {
            match t.get::<Value>(key).map_err(err)? {
                Value::Function(f) => Ok(Some(f)),
                Value::Nil => Ok(None),
                _ => Err(format!("{key} is a function.")),
            }
        };
        let render = func("render")?;
        let query = func("query")?;
        let popover = func("popover")?;
        match kind {
            Kind::Search if query.is_none() => {
                return Err("A search script needs query = function(text) ... end.".into());
            }
            Kind::Search => {}
            _ if render.is_none() => {
                return Err(format!("A {} script needs a render function.", kind.id()));
            }
            _ => {}
        }
        let title = match t.get::<Value>(if kind == Kind::Widget {
            "name"
        } else {
            "title"
        }) {
            Ok(Value::String(s)) => s.to_str().map_err(err)?.trim().to_owned(),
            _ => String::new(),
        };
        if title.len() > MAX_TITLE || title.chars().any(char::is_control) {
            return Err(format!(
                "title is one line of up to {MAX_TITLE} characters."
            ));
        }
        let refresh = refresh_of(&t)?;
        let s = Self {
            lua,
            kind,
            plugin: plugin.to_owned(),
            entry: entry.to_owned(),
            dir: dir.to_owned(),
            refresh,
            title,
            render,
            popover,
            query,
            handlers: Handlers::default(),
            meter,
            deadlines,
        };
        s.grant(permissions, &services).map_err(err)?;
        Ok(s)
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn plugin(&self) -> &str {
        &self.plugin
    }

    pub fn entry(&self) -> &str {
        &self.entry
    }

    /// Seconds between renders; 0 = only after input.
    pub fn refresh(&self) -> u32 {
        self.refresh
    }

    /// Panel title or widget name; empty = the manifest's entry id.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The limits later renders, clicks and queries run under.
    pub fn set_deadlines(&mut self, deadlines: Deadlines) {
        self.deadlines = deadlines;
    }

    /// True once the script ran out of time or instructions: every call answers
    /// [`crate::TOO_LONG`] without running until it is loaded again. The host counts that answer
    /// as a failure (`wol_core::plugins::Health`) like any other.
    pub fn stopped(&self) -> bool {
        self.meter.alive().is_err()
    }

    fn call_node(&mut self, f: &Function) -> Result<Node, String> {
        self.meter.alive()?;
        arm(&self.lua, &self.meter, self.deadlines.render).map_err(err)?;
        let r = f.call(()).map_err(err).and_then(|v: Value| {
            let mut handlers = Handlers::default();
            let node = to_node(&v, &mut handlers, 1, Some(&self.dir))?;
            Ok((node, handlers))
        });
        let (node, handlers) = self.meter.settle(r)?;
        node.validate()?;
        self.handlers = handlers;
        Ok(node)
    }

    /// The tree to draw now (bar item, card or panel body). Parts are numbered in tree order.
    pub fn render(&mut self) -> Result<Node, String> {
        let Some(f) = self.render.clone() else {
            return Err("A search script has no render; call query.".into());
        };
        self.call_node(&f)
    }

    /// A menu bar item's popover, when the script has one. Replaces the handlers of the last render.
    pub fn popover(&mut self) -> Result<Option<Node>, String> {
        let Some(f) = self.popover.clone() else {
            return Ok(None);
        };
        self.call_node(&f).map(Some)
    }

    /// Run a button's `on_click` (ids from the last render or popover).
    pub fn click(&mut self, id: u32) -> Result<(), String> {
        self.meter.alive()?;
        let f = self.handlers.get(id)?;
        arm(&self.lua, &self.meter, self.deadlines.event).map_err(err)?;
        self.meter.settle(f.call::<()>(()).map_err(err))
    }

    /// A field, toggle or slider changed: run its `on_change(value)`.
    pub fn change(&mut self, id: u32, value: Input) -> Result<(), String> {
        self.meter.alive()?;
        let f = self.handlers.get(id)?;
        arm(&self.lua, &self.meter, self.deadlines.event).map_err(err)?;
        let r = match value {
            Input::Text(s) => f.call::<()>(s),
            Input::On(b) => f.call::<()>(b),
            Input::Number(n) => f.call::<()>(n),
        }
        .map_err(err);
        self.meter.settle(r)
    }

    /// Search: rows for `text` (a `command` entry gets the text after its prefix).
    pub fn query(&mut self, text: &str) -> Result<Vec<Hit>, String> {
        let Some(f) = self.query.clone() else {
            return Err("Only a search script answers queries.".into());
        };
        self.meter.alive()?;
        arm(&self.lua, &self.meter, self.deadlines.event).map_err(err)?;
        let r = self.rows(&f, text);
        self.meter.settle(r)
    }

    /// The rows `query` returned, checked; handlers replace those of the last query.
    fn rows(&mut self, f: &Function, text: &str) -> Result<Vec<Hit>, String> {
        let v: Value = f.call(text).map_err(err)?;
        let rows: Table = match v {
            Value::Table(t) => t,
            Value::Nil => return Ok(Vec::new()),
            _ => return Err("query returns a list of rows, like { { title = \"...\" } }.".into()),
        };
        let mut handlers = Handlers::default();
        let mut hits = Vec::new();
        for row in rows.sequence_values::<Value>() {
            let Value::Table(r) = row.map_err(err)? else {
                return Err("Each search row is a table with a title.".into());
            };
            if hits.len() == MAX_HITS {
                return Err(format!("A search returns at most {MAX_HITS} rows."));
            }
            let text = |k: &str| -> Result<String, String> {
                match r.get::<Value>(k).map_err(err)? {
                    Value::String(s) => Ok(s.to_str().map_err(err)?.to_owned()),
                    Value::Integer(n) => Ok(n.to_string()),
                    Value::Number(n) => Ok(n.to_string()),
                    Value::Nil => Ok(String::new()),
                    _ => Err(format!("A search row's {k} is text.")),
                }
            };
            let title = text("title")?;
            if title.trim().is_empty() || title.len() > MAX_TITLE {
                return Err(format!(
                    "Each search row needs a title of 1 to {MAX_TITLE} characters."
                ));
            }
            let on_activate = match r.get::<Value>("on_activate").map_err(err)? {
                Value::Function(f) => Some(f),
                Value::Nil => None,
                _ => return Err("A search row's on_activate is a function.".into()),
            };
            let icon = text("icon")?;
            let icon = if !icon.is_empty()
                && !icon.starts_with('/')
                && shared::plugins::is_icon_file(&icon)
            {
                if !shared::plugins::is_plugin_path(&icon) {
                    return Err("A row's icon file is inside the plugin folder.".into());
                }
                self.dir.join(icon).to_string_lossy().into_owned()
            } else {
                icon
            };
            hits.push(Hit {
                id: handlers.id(on_activate),
                title,
                subtitle: text("subtitle")?,
                icon,
            });
        }
        self.handlers = handlers;
        Ok(hits)
    }

    /// Run a search row's `on_activate` (ids from the last query).
    pub fn activate(&mut self, id: u32) -> Result<(), String> {
        self.click(id)
    }

    /// Adds the `wol.*` calls the manifest allows. Each returns `ok, message` or `value, message`.
    fn grant(&self, permissions: &[Permission], services: &Rc<dyn Services>) -> mlua::Result<()> {
        let lua = &self.lua;
        let wol = crate::api(lua)?;
        let plugin = self.plugin.clone();
        let has = |p: Permission| permissions.contains(&p);
        if has(Permission::Notifications) {
            grant_notify(lua, services)?;
        }
        if has(Permission::Run) {
            let (s, id) = (services.clone(), plugin.clone());
            wol.raw_set(
                "run",
                lua.create_function(move |_, t: Table| {
                    let mut argv = Vec::new();
                    for v in t.sequence_values::<Value>() {
                        match v? {
                            Value::String(x) => argv.push(x.to_str()?.to_owned()),
                            Value::Integer(n) => argv.push(n.to_string()),
                            _ => {
                                return Ok((
                                    false,
                                    Some(
                                        "wol.run takes the command as strings, like wol.run { \"wol\", \"focus\", \"on\" }."
                                            .to_owned(),
                                    ),
                                ));
                            }
                        }
                    }
                    if argv.is_empty() || argv.iter().any(|a| a.is_empty()) {
                        return Ok((false, Some("wol.run needs a program to run.".to_owned())));
                    }
                    let timeout = match t.get::<Value>("timeout")? {
                        Value::Nil => DEFAULT_TIMEOUT,
                        Value::Integer(n) if (1..=i64::from(MAX_TIMEOUT)).contains(&n) => n as u32,
                        _ => {
                            return Ok((
                                false,
                                Some(format!("timeout is 1 to {MAX_TIMEOUT} seconds.")),
                            ));
                        }
                    };
                    Ok(match s.run(&id, &argv, timeout) {
                        Ok(out) => (true, Some(out)),
                        Err(m) => (false, Some(m)),
                    })
                })?,
            )?;
        }
        if has(Permission::Http) {
            let (s, id) = (services.clone(), plugin.clone());
            wol.raw_set(
                "http_get",
                lua.create_function(move |_, url: String| {
                    if !url.starts_with("https://") && !url.starts_with("http://") {
                        return Ok((
                            None,
                            Some("wol.http_get takes an https:// or http:// address.".to_owned()),
                        ));
                    }
                    Ok(match s.http_get(&id, &url) {
                        Ok(body) => (Some(body), None),
                        Err(m) => (None, Some(m)),
                    })
                })?,
            )?;
        }
        if has(Permission::Open) {
            let (s, id) = (services.clone(), plugin.clone());
            wol.raw_set(
                "open",
                lua.create_function(move |_, target: String| {
                    if target.trim().is_empty() || target.chars().any(char::is_control) {
                        return Ok((
                            false,
                            Some("wol.open takes a file, folder, link or app id.".to_owned()),
                        ));
                    }
                    Ok(match s.open(&id, &target) {
                        Ok(()) => (true, None),
                        Err(m) => (false, Some(m)),
                    })
                })?,
            )?;
        }
        if has(Permission::Toggles) {
            let (s, id) = (services.clone(), plugin.clone());
            wol.raw_set(
                "toggle_set",
                lua.create_function(move |_, (toggle, on): (String, bool)| {
                    if !shared::plugins::is_entry_id(&toggle) {
                        return Ok((
                            false,
                            Some("wol.toggle_set takes a [[toggle]] id of this plugin.".to_owned()),
                        ));
                    }
                    Ok(match s.toggle_set(&id, &toggle, on) {
                        Ok(()) => (true, None),
                        Err(m) => (false, Some(m)),
                    })
                })?,
            )?;
        }
        // Reading its own toggles, showing a panel and keeping state need no permission.
        let (s, id) = (services.clone(), plugin.clone());
        wol.raw_set(
            "toggle_get",
            lua.create_function(move |_, toggle: String| {
                Ok(shared::plugins::is_entry_id(&toggle) && s.toggle_get(&id, &toggle))
            })?,
        )?;
        let (s, id) = (services.clone(), plugin.clone());
        wol.raw_set(
            "show",
            lua.create_function(move |_, panel: String| {
                if !shared::view::key_ok(&panel) {
                    return Ok((false, Some("wol.show takes a panel name, like \"control-center\" or a [[panel]] id.".to_owned())));
                }
                Ok(match s.show(&id, &panel) {
                    Ok(()) => (true, None),
                    Err(m) => (false, Some(m)),
                })
            })?,
        )?;
        if has(Permission::Clipboard) {
            let clipboard = lua.create_table()?;
            let (s, id) = (services.clone(), plugin.clone());
            clipboard.set(
                "text",
                lua.create_function(move |_, ()| {
                    Ok(match s.clipboard_text(&id) {
                        Ok(t) => (Some(t), None),
                        Err(m) => (None, Some(m)),
                    })
                })?,
            )?;
            wol.raw_set("clipboard", clipboard)?;
        }
        // JSON in and out (what `wol ... --json` and web services speak); no permission needed.
        let json = lua.create_table()?;
        json.set(
            "decode",
            lua.create_function(|lua, text: String| {
                if text.len() > MAX_JSON {
                    return Ok((
                        Value::Nil,
                        Some(format!("JSON is at most {} KB.", MAX_JSON / 1024)),
                    ));
                }
                Ok(match serde_json::from_str::<Json>(&text) {
                    Ok(v) => (to_lua(lua, &v)?, None),
                    Err(e) => (
                        Value::Nil,
                        Some(format!(
                            "Not JSON (line {}, column {}).",
                            e.line(),
                            e.column()
                        )),
                    ),
                })
            })?,
        )?;
        json.set(
            "encode",
            lua.create_function(|_, v: Value| {
                Ok(match to_json(&v, 0) {
                    Ok(j) => (Some(j.to_string()), None),
                    Err(e) => (None, Some(e)),
                })
            })?,
        )?;
        wol.raw_set("json", json)?;
        let state = lua.create_table()?;
        let cache: Rc<RefCell<Option<Map<String, Json>>>> = Rc::new(RefCell::new(None));
        let load = {
            let (s, id, cache) = (services.clone(), plugin.clone(), cache.clone());
            move || -> Map<String, Json> {
                if let Some(m) = cache.borrow().as_ref() {
                    return m.clone();
                }
                let m = s
                    .state_load(&id)
                    .and_then(|t| serde_json::from_str::<Json>(&t).ok())
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                *cache.borrow_mut() = Some(m.clone());
                m
            }
        };
        let load2 = load.clone();
        state.set(
            "get",
            lua.create_function(move |lua, key: String| {
                let m = load();
                match m.get(&key) {
                    Some(v) => to_lua(lua, v),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
        let (s, id) = (services.clone(), plugin.clone());
        state.set(
            "set",
            lua.create_function(move |_, (key, value): (String, Value)| {
                if key.is_empty() || key.len() > 64 {
                    return Ok((false, Some("A state key is 1 to 64 characters.".to_owned())));
                }
                let mut m = load2();
                match to_json(&value, 0) {
                    Ok(Json::Null) => {
                        m.remove(&key);
                    }
                    Ok(v) => {
                        m.insert(key, v);
                    }
                    Err(e) => return Ok((false, Some(e))),
                }
                let text = Json::Object(m.clone()).to_string();
                if text.len() > MAX_STATE {
                    return Ok((
                        false,
                        Some(format!("Plugin state is at most {} KB.", MAX_STATE / 1024)),
                    ));
                }
                Ok(match s.state_save(&id, &text) {
                    Ok(()) => {
                        *cache.borrow_mut() = Some(m);
                        (true, None)
                    }
                    Err(e) => (false, Some(e)),
                })
            })?,
        )?;
        wol.raw_set("state", state)?;
        Ok(())
    }
}

fn to_lua(lua: &Lua, v: &Json) -> mlua::Result<Value> {
    Ok(match v {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i),
            None => Value::Number(n.as_f64().unwrap_or(0.0)),
        },
        Json::String(s) => Value::String(lua.create_string(s)?),
        // A `null` in a list is skipped, so the list stays a sequence (`#` and `ipairs` see all).
        Json::Array(a) => {
            let t = lua.create_table()?;
            for (i, x) in a.iter().filter(|x| !x.is_null()).enumerate() {
                t.set(i + 1, to_lua(lua, x)?)?;
            }
            Value::Table(t)
        }
        Json::Object(o) => {
            let t = lua.create_table()?;
            for (k, x) in o {
                t.set(k.as_str(), to_lua(lua, x)?)?;
            }
            Value::Table(t)
        }
    })
}

/// Lua value to JSON: strings, numbers, booleans, sequences (arrays) and string-keyed tables.
fn to_json(v: &Value, depth: usize) -> Result<Json, String> {
    if depth > MAX_STATE_DEPTH {
        return Err("A state value is nested too deep.".into());
    }
    Ok(match v {
        Value::Nil => Json::Null,
        Value::Boolean(b) => Json::Bool(*b),
        Value::Integer(i) => Json::from(*i),
        Value::Number(n) => serde_json::Number::from_f64(*n)
            .map(Json::Number)
            .ok_or_else(|| "A state number must be finite.".to_owned())?,
        Value::String(s) => Json::String(
            s.to_str()
                .map_err(|_| "State text must be UTF-8.".to_owned())?
                .to_owned(),
        ),
        Value::Table(t) => {
            let len = t.raw_len();
            let mut keys = 0usize;
            let mut obj = Map::new();
            let mut arr = Vec::new();
            for pair in t.clone().pairs::<Value, Value>() {
                let (k, x) = pair.map_err(err)?;
                keys += 1;
                match k {
                    Value::Integer(i) if i >= 1 && (i as usize) <= len => {
                        arr.push((i as usize, to_json(&x, depth + 1)?));
                    }
                    Value::String(s) => {
                        obj.insert(
                            s.to_str()
                                .map_err(|_| "State keys must be UTF-8.".to_owned())?
                                .to_owned(),
                            to_json(&x, depth + 1)?,
                        );
                    }
                    _ => {
                        return Err(
                            "State tables are lists or have text keys (no functions or mixed keys).".into(),
                        );
                    }
                }
            }
            if keys == len && !arr.is_empty() {
                arr.sort_by_key(|(i, _)| *i);
                Json::Array(arr.into_iter().map(|(_, x)| x).collect())
            } else if arr.is_empty() {
                Json::Object(obj)
            } else {
                return Err("State tables are lists or have text keys, not both.".into());
            }
        }
        _ => return Err("State holds text, numbers, booleans and tables only.".into()),
    })
}

/// The Lua step of a plugin check (`wol plugin check`, Plugin Maker's problems list): every
/// script of `m`, read from `dir`, loaded with `deadlines` and run once the way the desktop runs
/// it (search: one query; the others: render, then popover). One sentence per script that does
/// not, starting with its file name; empty = every script runs. `services` makes a fresh set of
/// services for each script.
pub fn lua_problems(
    dir: &Path,
    m: &shared::plugins::Manifest,
    deadlines: Deadlines,
    services: &dyn Fn() -> Rc<dyn Services>,
) -> Vec<String> {
    let mut out = Vec::new();
    for (kind, id, file) in m.scripts() {
        let Ok(src) = std::fs::read_to_string(dir.join(file)) else {
            out.push(format!("{file}: the file could not be read as text."));
            continue;
        };
        let at = Entry {
            kind,
            plugin: &m.id,
            id,
            dir,
        };
        let mut script = match Script::load_within(at, &src, &m.permissions, services(), deadlines)
        {
            Ok(s) => s,
            Err(e) => {
                out.push(format!("{file}: {e}"));
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
