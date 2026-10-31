//! Lua desktop widgets (ADR 0020, archicture/modules/abi.md): a widget is one `.lua` file that
//! returns `wol.widget { name, refresh, permissions, render }`. `render` returns a Kit view tree
//! (`shared::view::Node`) built with `wol.stack`, `wol.row`, `wol.label`, `wol.button`, `wol.spacer`;
//! button `on_click` functions run on click, and the next render shows the change.
//!
//! Sandbox: Lua 5.4 with only `string`, `table`, `math`, `utf8` and the safe base functions. No `io`,
//! `os`, `require`, `debug`, loaders or bytecode. Services come from libwol and only for the
//! permissions the widget declares (wold still checks each call). Memory and run time are bounded.
use mlua::{Function, HookTriggers, Lua, LuaOptions, StdLib, Table, Value, VmState};
use shared::view::{Node, Role, SPACING};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// What [`Services::system`] returns (libwol's system facts).
pub use wol::system::Info as SystemInfo;

pub const DEFAULT_REFRESH: u32 = 60;
pub const MAX_REFRESH: u32 = 86_400;
/// Permissions a widget may declare (libwol services that need one).
pub const PERMISSIONS: [&str; 1] = ["notifications"];
pub const MEMORY_LIMIT: usize = 16 << 20;
/// Lua VM instructions per load, render or click.
pub const INSTRUCTION_BUDGET: u64 = 5_000_000;
pub const MAX_FILE: u64 = 256 * 1024;
const MAX_NAME: usize = 40;

/// Local wall clock. weekday: 1 = Monday ..= 7 = Sunday.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// What a widget may reach. Live: [`Live`]; tests: a fake.
pub trait Services {
    fn now(&self) -> Stamp;
    fn system(&self) -> wol::system::Info;
    /// Through wold (permission-checked there too).
    fn notify(&self, title: &str, body: &str) -> Result<(), String>;
}

/// This machine: local time, libwol system facts, wold for services.
pub struct Live;

impl Services for Live {
    fn now(&self) -> Stamp {
        // SAFETY: time(NULL) and localtime_r write only into the zeroed `tm`.
        let t = unsafe { libc::time(std::ptr::null_mut()) };
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        unsafe { libc::localtime_r(&t, &mut tm) };
        Stamp {
            year: tm.tm_year + 1900,
            month: (tm.tm_mon + 1).clamp(1, 12) as u8,
            day: tm.tm_mday.clamp(1, 31) as u8,
            weekday: if tm.tm_wday == 0 {
                7
            } else {
                tm.tm_wday.clamp(1, 6) as u8
            },
            hour: tm.tm_hour.clamp(0, 23) as u8,
            minute: tm.tm_min.clamp(0, 59) as u8,
            second: tm.tm_sec.clamp(0, 60) as u8,
        }
    }

    fn system(&self) -> wol::system::Info {
        wol::system::read(&wol::system::config_home())
    }

    fn notify(&self, title: &str, body: &str) -> Result<(), String> {
        let a = wol::client::call(
            &wol::client::wold_socket(),
            &shared::portal::Call::NotificationPost {
                title: title.into(),
                body: body.into(),
                actions: Vec::new(),
            },
        );
        if a.ok {
            Ok(())
        } else {
            Err(a.message.unwrap_or_default())
        }
    }
}

/// Node constructors, run before the widget (our code, same sandbox).
const PRELUDE: &str = r#"
local function container(kind)
  return function(t)
    t = t or {}
    local children = {}
    for i, v in ipairs(t) do children[i] = v end
    return { type = kind, spacing = t.spacing or 0, children = children }
  end
end
wol.stack = container("stack")
wol.row = container("row")
wol.label = function(t)
  if type(t) ~= "table" then t = { t } end
  return { type = "label", text = tostring(t.text or t[1] or ""), role = t.role }
end
wol.button = function(t)
  return { type = "button", label = t.label or t[1], on_click = t.on_click }
end
wol.spacer = function() return { type = "spacer" } end
wol.widget = function(t)
  if type(t) == "table" then t.__wol_widget = true end
  return t
end
"#;

/// Globals the base library brings that a widget must not have.
const REMOVED: [&str; 5] = ["load", "loadfile", "dofile", "collectgarbage", "print"];

pub struct Widget {
    lua: Lua,
    name: String,
    refresh: u32,
    render: Function,
    handlers: BTreeMap<u32, Function>,
    budget: Arc<AtomicU64>,
}

fn err(e: mlua::Error) -> String {
    let s = e.to_string();
    if s.contains("budget") {
        return "This widget took too long and was stopped.".into();
    }
    if s.contains("not enough memory") {
        return "This widget used too much memory and was stopped.".into();
    }
    // First line: the message; Lua's traceback is for the log.
    s.lines().next().unwrap_or_default().to_owned()
}

fn sandbox(services: &Rc<dyn Services>) -> mlua::Result<Lua> {
    let lua = Lua::new_with(
        StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8,
        LuaOptions::default(),
    )?;
    lua.set_memory_limit(MEMORY_LIMIT)?;
    let g = lua.globals();
    for name in REMOVED {
        g.raw_set(name, Value::Nil)?;
    }
    let string: Table = g.get("string")?;
    string.raw_set("dump", Value::Nil)?;
    let wol = lua.create_table()?;
    let s = services.clone();
    wol.set(
        "now",
        lua.create_function(move |lua, ()| {
            let t = s.now();
            let o = lua.create_table()?;
            o.set("year", t.year)?;
            o.set("month", t.month)?;
            o.set("day", t.day)?;
            o.set("weekday", t.weekday)?;
            o.set("hour", t.hour)?;
            o.set("minute", t.minute)?;
            o.set("second", t.second)?;
            Ok(o)
        })?,
    )?;
    let s = services.clone();
    wol.set(
        "system",
        lua.create_function(move |lua, ()| {
            let i = s.system();
            let o = lua.create_table()?;
            o.set("os_name", i.os_name)?;
            o.set("os_version", i.os_version)?;
            o.set("kernel", i.kernel)?;
            let dark = i.appearance == wol::system::Appearance::Dark;
            o.set("appearance", if dark { "dark" } else { "light" })?;
            o.set("accent", i.accent_name)?;
            o.set("accent_rgb", i.accent_rgb)?;
            Ok(o)
        })?,
    )?;
    g.set("wol", wol)?;
    lua.load(PRELUDE).set_name("=wol").exec()?;
    Ok(lua)
}

/// Services for declared permissions, added once the file has loaded: a widget reaches them from
/// `render` and click handlers only, so loading one never acts.
fn grant(lua: &Lua, services: &Rc<dyn Services>, permissions: &[String]) -> mlua::Result<()> {
    let wol: Table = lua.globals().get("wol")?;
    if permissions.iter().any(|p| p == "notifications") {
        let s = services.clone();
        wol.set(
            "notify",
            lua.create_function(move |_, (title, body): (String, Option<String>)| {
                Ok(match s.notify(&title, body.as_deref().unwrap_or("")) {
                    Ok(()) => (true, None),
                    Err(m) => (false, Some(m)),
                })
            })?,
        )?;
    }
    Ok(())
}

fn arm(lua: &Lua, budget: &Arc<AtomicU64>) -> mlua::Result<()> {
    budget.store(INSTRUCTION_BUDGET, Ordering::Relaxed);
    let b = budget.clone();
    lua.set_global_hook(
        HookTriggers::new().every_nth_instruction(1000),
        move |_, _| {
            let left = b.load(Ordering::Relaxed);
            if left < 1000 {
                return Err(mlua::Error::runtime("instruction budget spent"));
            }
            b.store(left - 1000, Ordering::Relaxed);
            Ok(VmState::Continue)
        },
    )
}

fn permissions(t: &Table) -> Result<Vec<String>, String> {
    let bad = || "permissions is a list of names, like { \"notifications\" }.".to_owned();
    let perms: Vec<String> = match t.get::<Value>("permissions").map_err(err)? {
        Value::Nil => Vec::new(),
        Value::Table(p) => p
            .sequence_values::<String>()
            .collect::<mlua::Result<_>>()
            .map_err(|_| bad())?,
        _ => return Err(bad()),
    };
    if let Some(p) = perms.iter().find(|p| !PERMISSIONS.contains(&p.as_str())) {
        return Err(format!(
            "Widgets cannot ask for the permission \"{p}\". Known: {}.",
            PERMISSIONS.join(", ")
        ));
    }
    Ok(perms)
}

impl std::fmt::Debug for Widget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Widget")
            .field("name", &self.name)
            .field("refresh", &self.refresh)
            .finish()
    }
}

impl Widget {
    /// Run the script and keep its widget. `name` is for messages (the file id).
    pub fn load(name: &str, src: &str, services: Rc<dyn Services>) -> Result<Self, String> {
        let lua = sandbox(&services).map_err(err)?;
        let budget = Arc::new(AtomicU64::new(0));
        arm(&lua, &budget).map_err(err)?;
        let v: Value = lua
            .load(src)
            .set_name(format!("={name}"))
            .eval()
            .map_err(err)?;
        let Value::Table(t) = v else {
            return Err("The script must end with return wol.widget { ... }.".into());
        };
        if !t.get::<bool>("__wol_widget").unwrap_or(false) {
            return Err("The script must end with return wol.widget { ... }.".into());
        }
        let title = match t.get::<Value>("name").map_err(err)? {
            Value::String(s) => s.to_str().map_err(err)?.trim().to_owned(),
            _ => String::new(),
        };
        if title.is_empty() || title.len() > MAX_NAME || title.chars().any(char::is_control) {
            return Err(format!(
                "A widget needs a name of 1 to {MAX_NAME} characters."
            ));
        }
        let refresh = match t.get::<Value>("refresh").map_err(err)? {
            Value::Nil => DEFAULT_REFRESH,
            Value::Integer(n) if (0..=MAX_REFRESH as i64).contains(&n) => n as u32,
            _ => {
                return Err(format!(
                    "refresh is seconds between renders, 0 to {MAX_REFRESH}."
                ));
            }
        };
        let Value::Function(render) = t.get::<Value>("render").map_err(err)? else {
            return Err("A widget needs a render function.".into());
        };
        grant(&lua, &services, &permissions(&t)?).map_err(err)?;
        Ok(Self {
            lua,
            name: title,
            refresh,
            render,
            handlers: BTreeMap::new(),
            budget,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Seconds between renders; 0 = only after a click.
    pub fn refresh(&self) -> u32 {
        self.refresh
    }

    /// The tree to draw now. Buttons are numbered in tree order.
    pub fn render(&mut self) -> Result<Node, String> {
        arm(&self.lua, &self.budget).map_err(err)?;
        let v: Value = self.render.call(()).map_err(err)?;
        let mut handlers = BTreeMap::new();
        let node = to_node(&v, &mut handlers, 1)?;
        node.validate()?;
        self.handlers = handlers;
        Ok(node)
    }

    /// Run a button's handler (ids from the last render).
    pub fn click(&mut self, id: u32) -> Result<(), String> {
        let f = self
            .handlers
            .get(&id)
            .cloned()
            .ok_or_else(|| "That button is not on the widget any more.".to_owned())?;
        arm(&self.lua, &self.budget).map_err(err)?;
        f.call::<()>(()).map_err(err)
    }
}

fn to_node(
    v: &Value,
    handlers: &mut BTreeMap<u32, Function>,
    depth: usize,
) -> Result<Node, String> {
    if depth > shared::view::MAX_DEPTH {
        return Err("The widget's parts are nested too deep.".into());
    }
    let Value::Table(t) = v else {
        return Err("render must return a part, like wol.label { \"Hi\" }.".into());
    };
    let kind: String = t
        .get::<Option<String>>("type")
        .ok()
        .flatten()
        .ok_or("render must return a part, like wol.label { \"Hi\" }.")?;
    let text = |key: &str| -> Result<String, String> {
        match t.get::<Value>(key).map_err(err)? {
            Value::String(s) => Ok(s.to_str().map_err(err)?.to_owned()),
            Value::Integer(n) => Ok(n.to_string()),
            Value::Number(n) => Ok(n.to_string()),
            Value::Nil => Ok(String::new()),
            _ => Err(format!("A {kind}'s {key} is text.")),
        }
    };
    Ok(match kind.as_str() {
        "stack" | "row" => {
            let spacing: i64 = t.get("spacing").unwrap_or(0);
            let spacing = u8::try_from(spacing)
                .ok()
                .filter(|s| SPACING.contains(s))
                .ok_or_else(|| format!("Spacing is one of {SPACING:?}."))?;
            let list: Table = t.get("children").map_err(err)?;
            let mut children = Vec::new();
            for c in list.sequence_values::<Value>() {
                children.push(to_node(&c.map_err(err)?, handlers, depth + 1)?);
            }
            if kind == "stack" {
                Node::Stack { spacing, children }
            } else {
                Node::Row { spacing, children }
            }
        }
        "label" => {
            let role = match t.get::<Option<String>>("role").map_err(err)? {
                None => Role::Body,
                Some(r) => Role::parse(&r).ok_or_else(|| {
                    format!("Unknown label role \"{r}\": display, large, title, body, secondary or caption.")
                })?,
            };
            Node::Label {
                text: text("text")?,
                role,
            }
        }
        "button" => {
            let Value::Function(f) = t.get::<Value>("on_click").map_err(err)? else {
                return Err("A button needs on_click = function() ... end.".into());
            };
            let id = handlers.len() as u32 + 1;
            handlers.insert(id, f);
            Node::Button {
                id,
                label: text("label")?,
            }
        }
        "spacer" => Node::Spacer,
        other => {
            return Err(format!(
                "Unknown part \"{other}\": stack, row, label, button or spacer."
            ));
        }
    })
}

/// What [`Library::poll`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// New or changed, and it loads.
    Loaded(String),
    /// New or changed, and it does not load: show the message on the card.
    Failed(String, String),
    Removed(String),
}

/// `~/.config/wolos/widgets/`.
pub fn user_dir() -> PathBuf {
    wol::system::config_home().join("wolos/widgets")
}

/// Widget id from a file name: `clock.lua` -> `clock` (lowercase letters, digits, `-`).
pub fn id_of(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let ok = path.extension().is_some_and(|e| e == "lua")
        && !stem.is_empty()
        && stem.len() <= 64
        && stem
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    ok.then(|| stem.to_owned())
}

struct Entry {
    stamp: Option<std::time::SystemTime>,
    widget: Option<Widget>,
}

/// A folder of `.lua` widgets, reloaded when a file changes.
pub struct Library {
    dir: PathBuf,
    services: Rc<dyn Services>,
    entries: BTreeMap<String, Entry>,
}

impl Library {
    pub fn new(dir: PathBuf, services: Rc<dyn Services>) -> Self {
        Self {
            dir,
            services,
            entries: BTreeMap::new(),
        }
    }

    /// Look at the folder again (cheap: one stat per file). Call on a timer or a file watch.
    pub fn poll(&mut self) -> Vec<Change> {
        let mut seen = BTreeMap::new();
        for e in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            let p = e.path();
            if let Some(id) = id_of(&p) {
                seen.insert(id, p);
            }
        }
        let mut changes = Vec::new();
        let gone: Vec<String> = self
            .entries
            .keys()
            .filter(|k| !seen.contains_key(*k))
            .cloned()
            .collect();
        for id in gone {
            self.entries.remove(&id);
            changes.push(Change::Removed(id));
        }
        for (id, path) in seen {
            let meta = std::fs::metadata(&path).ok();
            let stamp = meta.as_ref().and_then(|m| m.modified().ok());
            if self.entries.get(&id).is_some_and(|e| e.stamp == stamp) {
                continue;
            }
            let loaded = match meta {
                Some(m) if m.len() > MAX_FILE => {
                    Err(format!("The file is larger than {} KB.", MAX_FILE / 1024))
                }
                _ => std::fs::read_to_string(&path)
                    .map_err(|_| "The file could not be read as text.".to_owned())
                    .and_then(|src| Widget::load(&id, &src, self.services.clone())),
            };
            let (widget, change) = match loaded {
                Ok(w) => (Some(w), Change::Loaded(id.clone())),
                Err(m) => (None, Change::Failed(id.clone(), m)),
            };
            self.entries.insert(id, Entry { stamp, widget });
            changes.push(change);
        }
        changes
    }

    pub fn widget(&mut self, id: &str) -> Option<&mut Widget> {
        self.entries.get_mut(id)?.widget.as_mut()
    }

    pub fn ids(&self) -> Vec<&str> {
        self.entries.keys().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_table() {
        for (p, want) in [
            ("/x/clock.lua", Some("clock")),
            ("/x/my-cpu2.lua", Some("my-cpu2")),
            ("/x/Clock.lua", None),
            ("/x/clock.txt", None),
            ("/x/.lua", None),
            ("/x/a b.lua", None),
        ] {
            assert_eq!(id_of(Path::new(p)).as_deref(), want, "{p}");
        }
    }
}
