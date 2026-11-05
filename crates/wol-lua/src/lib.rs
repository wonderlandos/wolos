// SPDX-License-Identifier: GPL-2.0-or-later
//! Lua desktop widgets (ADR 0020, docs/modules/abi.md) and plugin scripts (ADR 0031,
//! docs/modules/plugins.md, [`plugin`]). A widget is one `.lua` file that returns
//! `wol.widget { name, refresh, permissions, render }`. `render` returns a Kit view tree
//! (`shared::view::Node`) built with `wol.stack`, `wol.row`, `wol.label`, `wol.button`, `wol.spacer`,
//! `wol.image`, `wol.toggle`, `wol.slider`, `wol.text_field`, `wol.list`, `wol.item`; `on_click` and
//! `on_change` functions run on input, and the next render shows the change.
//!
//! Sandbox: Lua 5.4 with only `string`, `table`, `math`, `utf8` and the safe base functions. No `io`,
//! `os`, `require`, `debug`, loaders or bytecode. Services come from the host ([`Services`]) and only
//! for the permissions the widget or plugin declares. Memory and run time are bounded.
use mlua::{Function, HookTriggers, Lua, LuaOptions, StdLib, Table, Value, VmState};
use shared::view::{IMAGE_SIZES, Node, Role, SPACING};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub mod plugin;

/// What [`Services::system`] returns (libwol's system facts), for hosts without libwol.
pub use wol::system::{Appearance, Info as SystemInfo};

pub const DEFAULT_REFRESH: u32 = 60;
pub const MAX_REFRESH: u32 = 86_400;
/// Permissions a stand-alone widget may declare (plugins declare theirs in `plugin.toml`).
pub const PERMISSIONS: [&str; 1] = ["notifications"];
pub const MEMORY_LIMIT: usize = 16 << 20;
/// Lua VM instructions per load, render or click.
pub const INSTRUCTION_BUDGET: u64 = 5_000_000;
/// Instructions between two looks of the hook at the budget and the clock.
const HOOK_STEP: u32 = 250;
pub const MAX_FILE: u64 = 256 * 1024;
const MAX_NAME: usize = 40;
/// Default image size in px.
pub const DEFAULT_IMAGE: u16 = 24;

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

/// What a widget or plugin script may reach. Live: [`Live`]; tests: a fake. The plugin calls have
/// defaults that refuse with a sentence, so a host adds only what it offers; the shell and
/// `wol plugin run` wire them to `wol_core` (commands, state, toggles, grants).
pub trait Services {
    fn now(&self) -> Stamp;
    fn system(&self) -> wol::system::Info;
    /// Through wold (permission-checked there too).
    fn notify(&self, title: &str, body: &str) -> Result<(), String>;
    /// `wol.run`: argv as the user, limited, stdout back. `plugin` is for the audit log.
    fn run(&self, plugin: &str, argv: &[String], timeout_secs: u32) -> Result<String, String> {
        let _ = (plugin, argv, timeout_secs);
        Err("Running commands is not available here.".into())
    }
    /// `wol.http_get`: the host checks the per-host grant and answers with the consent sentence.
    fn http_get(&self, plugin: &str, url: &str) -> Result<String, String> {
        let _ = (plugin, url);
        Err("Reading from the web is not available here.".into())
    }
    /// `wol.open`: a file, folder, URL or app id, with the default handler.
    fn open(&self, plugin: &str, target: &str) -> Result<(), String> {
        let _ = (plugin, target);
        Err("Opening things is not available here.".into())
    }
    /// `wol.toggle_get`: the plugin's own flag file.
    fn toggle_get(&self, plugin: &str, toggle: &str) -> bool {
        let _ = (plugin, toggle);
        false
    }
    /// `wol.toggle_set`: flips the flag and runs the toggle's on/off command.
    fn toggle_set(&self, plugin: &str, toggle: &str, on: bool) -> Result<(), String> {
        let _ = (plugin, toggle, on);
        Err("Toggles are not available here.".into())
    }
    /// `wol.clipboard.text`.
    fn clipboard_text(&self, plugin: &str) -> Result<String, String> {
        let _ = plugin;
        Err("The clipboard is not available here.".into())
    }
    /// `wol.show`: a shell panel name or one of the plugin's `[[panel]]` ids.
    fn show(&self, plugin: &str, panel: &str) -> Result<(), String> {
        let _ = (plugin, panel);
        Err("Panels are not available here.".into())
    }
    /// `wol.state`: the plugin's JSON object as text; None = nothing saved yet.
    fn state_load(&self, plugin: &str) -> Option<String> {
        let _ = plugin;
        None
    }
    fn state_save(&self, plugin: &str, json: &str) -> Result<(), String> {
        let _ = (plugin, json);
        Err("Saving state is not available here.".into())
    }
}

/// The host's [`Services`] as a script sees them: each call runs off the script's clock
/// ([`Meter::off_clock`]), so a render that waits on `wol.run` or a state write is not stopped as
/// "took too long" (QA-PLUG-2). Every load wraps the host's services in this before the sandbox
/// exists, so no `wol.*` call can reach the host around it; `off_clock_wraps_every_service`
/// checks that each [`Services`] method is forwarded here.
pub(crate) struct OffClock {
    inner: Rc<dyn Services>,
    meter: Arc<Meter>,
}

impl OffClock {
    pub(crate) fn wrap(inner: Rc<dyn Services>, meter: &Arc<Meter>) -> Rc<dyn Services> {
        Rc::new(OffClock {
            inner,
            meter: meter.clone(),
        })
    }
}

impl Services for OffClock {
    fn now(&self) -> Stamp {
        self.meter.off_clock(|| self.inner.now())
    }
    fn system(&self) -> wol::system::Info {
        self.meter.off_clock(|| self.inner.system())
    }
    fn notify(&self, title: &str, body: &str) -> Result<(), String> {
        self.meter.off_clock(|| self.inner.notify(title, body))
    }
    fn run(&self, plugin: &str, argv: &[String], timeout_secs: u32) -> Result<String, String> {
        self.meter
            .off_clock(|| self.inner.run(plugin, argv, timeout_secs))
    }
    fn http_get(&self, plugin: &str, url: &str) -> Result<String, String> {
        self.meter.off_clock(|| self.inner.http_get(plugin, url))
    }
    fn open(&self, plugin: &str, target: &str) -> Result<(), String> {
        self.meter.off_clock(|| self.inner.open(plugin, target))
    }
    fn toggle_get(&self, plugin: &str, toggle: &str) -> bool {
        self.meter
            .off_clock(|| self.inner.toggle_get(plugin, toggle))
    }
    fn toggle_set(&self, plugin: &str, toggle: &str, on: bool) -> Result<(), String> {
        self.meter
            .off_clock(|| self.inner.toggle_set(plugin, toggle, on))
    }
    fn clipboard_text(&self, plugin: &str) -> Result<String, String> {
        self.meter.off_clock(|| self.inner.clipboard_text(plugin))
    }
    fn show(&self, plugin: &str, panel: &str) -> Result<(), String> {
        self.meter.off_clock(|| self.inner.show(plugin, panel))
    }
    fn state_load(&self, plugin: &str) -> Option<String> {
        self.meter.off_clock(|| self.inner.state_load(plugin))
    }
    fn state_save(&self, plugin: &str, json: &str) -> Result<(), String> {
        self.meter.off_clock(|| self.inner.state_save(plugin, json))
    }
}

/// This machine: local time, libwol system facts, wold for notifications and the clipboard, the
/// shell socket for panels. Commands, state and toggles need a host with `wol_core`.
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

    fn clipboard_text(&self, _plugin: &str) -> Result<String, String> {
        let a = wol::client::call(
            &wol::client::wold_socket(),
            &shared::portal::Call::ClipboardRead {},
        );
        if !a.ok {
            return Err(a
                .message
                .unwrap_or_else(|| "The clipboard could not be read.".into()));
        }
        Ok(a.data
            .as_ref()
            .and_then(|d| d.get("text").or(Some(d)))
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_owned())
    }

    fn show(&self, _plugin: &str, panel: &str) -> Result<(), String> {
        use std::io::Write;
        if !shared::shell::PANELS.contains(&panel) {
            return Err(format!("There is no panel called \"{panel}\"."));
        }
        let rt = std::env::var_os("XDG_RUNTIME_DIR")
            .ok_or_else(|| "The desktop is not running.".to_owned())?;
        let sock = PathBuf::from(rt).join("wol-shell.sock");
        let mut s = std::os::unix::net::UnixStream::connect(sock)
            .map_err(|_| "The desktop is not running.".to_owned())?;
        s.write_all(format!("show {panel}\n").as_bytes())
            .map_err(|_| "The desktop did not answer.".to_owned())
    }
}

/// Largest `string.rep` result.
pub const MAX_REP: u64 = 1 << 20;
/// Longest subject a pattern function takes.
pub const MAX_SUBJECT: u64 = 1 << 20;
/// Longest pattern.
pub const MAX_PATTERN: u64 = 256;
/// Upper bound on the work of one pattern call: subject length ^ (backtracking items + 1).
pub const PATTERN_WORK: u64 = 1_000_000_000;
/// Longest range `table.move` and `table.concat` walk in one call (audit L2).
pub const MAX_ITEMS: u64 = 1 << 20;
/// Largest table `table.sort` takes (audit L2).
pub const MAX_SORT: u64 = 1 << 16;

/// The instruction hook does not run inside C library functions, so the ones whose time grows
/// with their input are bounded here (audit A21, S9, L2, L3, R2C-2): `string.rep` by count and
/// result size;
/// `find`, `match`, `gmatch`, `gsub` by subject and pattern length and by the backtracking a
/// pattern can do (`len(s) ^ (items + 1)` at most [`PATTERN_WORK`], where items are the `*`, `+`,
/// `-`, `?` and `%b` items outside sets, less a quantifier that ends the pattern); `table.move`
/// and `table.concat` by range; `table.insert`/`remove` and `table.sort` by size (through `__len`,
/// as C reads it), sort through a Lua comparator the hook sees. Every wrapper first calls `tick`,
/// which raises once the entry point is out of instructions or past its deadline (R2C-3).
/// `pcall` and `xpcall` re-raise a spent budget or a full heap (audit L1). Plain `find` is linear
/// and only pays the subject cap. The originals stay in upvalues the script cannot reach.
const STRING_GUARD: &str = r#"
local rep, find, match, gmatch, gsub, len, byte = string.rep, string.find, string.match,
  string.gmatch, string.gsub, string.len, string.byte
local tmove, tsort, tconcat = table.move, table.sort, table.concat
local tinsert, tremove = table.insert, table.remove
local type, error, tostring, tonumber, rawlen, log = type, error, tostring, tonumber, rawlen, math.log
local rawpcall, rawxpcall = pcall, xpcall
local MAX_REP, MAX_SUBJECT, MAX_PATTERN, WORK, MAX_ITEMS, MAX_SORT, dead, tick = ...
-- The index after one single-character class at i: %x, [set] or a plain character. Mirrors
-- classEnd in lstrlib.c, so a quantifier character inside a set is never mistaken for one.
local function class_end(p, i, n)
  local c = byte(p, i)
  if c == 37 then return i + 2 end -- %x
  if c ~= 91 then return i + 1 end -- plain character
  i = i + 1
  if byte(p, i) == 94 then i = i + 1 end -- [^
  repeat -- the first character after [ or [^ is a member, even ']'
    if i > n then return i end
    local d = byte(p, i)
    i = i + 1
    if d == 37 then i = i + 1 end -- %] and friends
  until byte(p, i) == 93
  return i + 1
end
-- Items that can make the matcher go back: quantified classes (* + - ?) and %bxy (each scans
-- to the end of the subject when unbalanced). A quantifier that ends the pattern (only closing
-- captures after it) never backtracks: once it is reached the match has succeeded. In a pattern
-- anchored at both ends the last one tries at most its own run against the final $, once.
local function backtracking(p)
  local q, i, n, tail = 0, 1, len(p), nil
  local anchored = byte(p, 1) == 94
  if anchored then i = 2 end -- ^ anchor
  while i <= n do
    local c = byte(p, i)
    if c == 40 or c == 41 then -- ( ) and the position capture ()
      i = i + 1
    elseif c == 36 and i == n then -- $ anchor
      i = i + 1
    elseif c == 37 and byte(p, i + 1) == 98 then -- %bxy
      q, i, tail = q + 1, i + 4, nil
    elseif c == 37 and byte(p, i + 1) == 102 then -- %f[set]: one look, no backtracking
      i = class_end(p, i + 2, n)
    else
      i = class_end(p, i, n)
      local d = byte(p, i)
      if d == 42 or d == 43 or d == 45 or d == 63 then -- * + - ?
        q, i, tail = q + 1, i + 1, i + 1
      end
    end
  end
  if tail then
    local only_closing = true
    for k = tail, n do
      local c = byte(p, k)
      if c ~= 41 and not (anchored and c == 36 and k == n) then only_closing = false end
    end
    if only_closing then q = q - 1 end
  end
  return q
end
local function check(s, p, plain)
  if type(s) == "string" or type(s) == "number" then
    local n = len(s)
    if n > MAX_SUBJECT then error("string too long for a pattern (more than " .. MAX_SUBJECT .. " bytes)", 3) end
    if plain or (type(p) ~= "string" and type(p) ~= "number") then return end
    p = tostring(p)
    if len(p) > MAX_PATTERN then error("pattern too long (more than " .. MAX_PATTERN .. " bytes)", 3) end
    local q = backtracking(p)
    if n > 1 and q > 0 and (q + 1) * log(n) > log(WORK) then
      error("pattern too costly for a string of " .. n .. " bytes; use fewer * + - ? or a shorter string", 3)
    end
  end
end
-- The cost of rep is its count, not its bytes: an empty piece copies nothing but C still loops
-- n times (R2C-2). n is read as C reads it (tonumber), so "1e12" is a count too.
string.rep = function(s, n, sep, ...)
  tick()
  local k = tonumber(n)
  if k and k > MAX_REP then error("string.rep count too large (more than " .. MAX_REP .. ")", 2) end
  if k and k > 0 and (type(s) == "string" or type(s) == "number") then
    local total = (len(s) + (sep ~= nil and len(sep) or 0)) * k
    if total > MAX_REP then error("string.rep result too large (more than " .. MAX_REP .. " bytes)", 2) end
  end
  return rep(s, n, sep, ...)
end
string.find = function(s, p, init, plain, ...)
  tick()
  check(s, p, plain)
  return find(s, p, init, plain, ...)
end
string.match = function(s, p, ...)
  tick()
  check(s, p, false)
  return match(s, p, ...)
end
string.gmatch = function(s, p, ...)
  tick()
  check(s, p, false)
  return gmatch(s, p, ...)
end
string.gsub = function(s, p, ...)
  tick()
  check(s, p, false)
  return gsub(s, p, ...)
end
-- table.move and table.concat loop in C over a range the caller names, not over what exists.
local function range(f, e, what)
  local nf, ne = tonumber(f), tonumber(e)
  if nf and ne and ne - nf >= MAX_ITEMS then
    error(what .. " range too large (more than " .. MAX_ITEMS .. " items)", 3)
  end
end
table.move = function(a1, f, e, ...)
  tick()
  range(f, e, "table.move")
  return tmove(a1, f, e, ...)
end
-- C takes the length through __len (luaL_len), so the guards do too: an empty table whose __len
-- says 2^40 is a count of 2^40 (R2C-2).
table.concat = function(list, sep, i, j, ...)
  tick()
  if type(list) == "table" then range(i or 1, j or #list, "table.concat") end
  return tconcat(list, sep, i, j, ...)
end
-- insert at a position and remove shift every item from there to #t, in C.
local function shifts(t, what)
  if type(t) == "table" then
    local n = tonumber(#t)
    if n and n > MAX_ITEMS then error(what .. " table too large (more than " .. MAX_ITEMS .. " items)", 3) end
  end
end
table.insert = function(t, ...)
  tick()
  shifts(t, "table.insert")
  return tinsert(t, ...)
end
table.remove = function(t, ...)
  tick()
  shifts(t, "table.remove")
  return tremove(t, ...)
end
-- table.sort compares in C; a Lua comparator lets the instruction budget see every comparison.
local function less(a, b) return a < b end
table.sort = function(t, comp, ...)
  tick()
  local n = type(t) == "table" and tonumber(#t)
  if n and n > MAX_SORT then
    error("table.sort table too large (more than " .. MAX_SORT .. " items)", 2)
  end
  if comp ~= nil and type(comp) ~= "function" then return tsort(t, comp, ...) end
  local c = comp or less
  return tsort(t, function(a, b) return c(a, b) end, ...)
end
-- A spent budget or a full heap stops the whole script: pcall and xpcall pass every other error
-- back as usual, but re-raise these, so `while true do pcall(f) end` cannot keep the shell busy.
local function fatal(e)
  return dead() or (type(e) == "string" and find(e, "not enough memory", 1, true) ~= nil)
end
local function pass(ok, ...)
  if not ok and fatal((...)) then error((...), 0) end
  return ok, ...
end
pcall = function(f, ...) return pass(rawpcall(f, ...)) end
xpcall = function(f, h, ...) return pass(rawxpcall(f, h, ...)) end
-- The remaining C functions whose time grows with their input are each bounded by the heap or the
-- stack, but a loop around one runs many calls between two hook ticks; each call looks at the
-- clock first, so a loop of them stops at the deadline (R2C-3).
local function timed(lib, names)
  for _, name in ipairs(names) do
    local f = lib[name]
    lib[name] = function(...) tick() return f(...) end
  end
end
timed(string, { "upper", "lower", "reverse", "sub", "format", "byte", "char", "pack", "unpack", "packsize" })
timed(table, { "unpack" })
timed(utf8, { "char", "codepoint", "len", "offset" })
"#;

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
wol.list = function(t)
  t = t or {}
  local children = {}
  for i, v in ipairs(t) do children[i] = v end
  return { type = "list", children = children }
end
wol.label = function(t)
  if type(t) ~= "table" then t = { t } end
  return { type = "label", text = tostring(t.text or t[1] or ""), role = t.role }
end
wol.button = function(t)
  return { type = "button", label = t.label or t[1], on_click = t.on_click }
end
wol.spacer = function() return { type = "spacer" } end
wol.image = function(t)
  if type(t) ~= "table" then t = { t } end
  return { type = "image", source = t.source or t[1], size = t.size }
end
wol.toggle = function(t)
  return { type = "toggle", label = t.label or t[1], on = t.on, on_change = t.on_change }
end
wol.slider = function(t)
  return { type = "slider", value = t.value, min = t.min, max = t.max, on_change = t.on_change }
end
wol.text_field = function(t)
  t = t or {}
  return { type = "text_field", text = t.text or t[1], placeholder = t.placeholder, on_change = t.on_change }
end
wol.item = function(t)
  t = t or {}
  local row = { type = "row", spacing = 8, children = {} }
  if t.icon then row.children[#row.children + 1] = wol.image { t.icon, size = t.size or 24 } end
  local text = { type = "stack", spacing = 0, children = { wol.label { t.title or t[1] or "" } } }
  if t.subtitle then text.children[2] = wol.label { t.subtitle, role = "secondary" } end
  row.children[#row.children + 1] = text
  if t.button then
    row.children[#row.children + 1] = wol.spacer()
    row.children[#row.children + 1] = wol.button(t.button)
  end
  return row
end
local function marker(kind)
  return function(t)
    if type(t) == "table" then t.__wol_kind = kind end
    return t
  end
end
wol.widget = marker("widget")
wol.menubar = marker("menubar")
wol.panel = marker("panel")
wol.search = marker("search")
"#;

/// Globals the base library brings that a widget must not have.
/// `warn` too: it wrote to the host's journal with no cap (QA-PLUG-13).
const REMOVED: [&str; 6] = [
    "load",
    "loadfile",
    "dofile",
    "collectgarbage",
    "print",
    "warn",
];

/// Handlers of the last render, by part id (buttons: `on_click`; fields, toggles, sliders:
/// `on_change`). Ids are given in tree order, so the host's events name a part of this tree.
#[derive(Default)]
pub(crate) struct Handlers {
    map: BTreeMap<u32, Function>,
    next: u32,
}

impl Handlers {
    fn id(&mut self, f: Option<Function>) -> u32 {
        self.next += 1;
        if let Some(f) = f {
            self.map.insert(self.next, f);
        }
        self.next
    }

    fn get(&self, id: u32) -> Result<Function, String> {
        self.map
            .get(&id)
            .cloned()
            .ok_or_else(|| "That part is not on the view any more.".to_owned())
    }
}

pub struct Widget {
    lua: Lua,
    name: String,
    refresh: u32,
    render: Function,
    handlers: Handlers,
    meter: Arc<Meter>,
    deadlines: Deadlines,
}

/// What a script that ran out of time or instructions shows, then and on every later call.
pub const TOO_LONG: &str = "This widget took too long and was stopped.";

/// The error the hook and `tick` raise; [`err`] turns it into [`TOO_LONG`].
const SPENT: &str = "wol: time or instruction budget spent";

pub(crate) fn err(e: mlua::Error) -> String {
    let s = e.to_string();
    if s.contains(SPENT) {
        return TOO_LONG.into();
    }
    if s.contains("not enough memory") {
        return "This widget used too much memory and was stopped.".into();
    }
    // First line: the message; Lua's traceback is for the log.
    s.lines().next().unwrap_or_default().to_owned()
}

pub(crate) fn sandbox(services: &Rc<dyn Services>) -> mlua::Result<Lua> {
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
    // True once this entry point's budget is spent: the guard's pcall/xpcall re-raise then.
    let dead = lua.create_function(|lua, ()| Ok(spent(lua)))?;
    // Called by every guard wrapper before its C function: raises once the entry point is spent.
    let tick = lua.create_function(|lua, ()| {
        if spent(lua) {
            return Err(mlua::Error::runtime(SPENT));
        }
        Ok(())
    })?;
    lua.load(STRING_GUARD).set_name("=wol.string").call::<()>((
        MAX_REP,
        MAX_SUBJECT,
        MAX_PATTERN,
        PATTERN_WORK,
        MAX_ITEMS,
        MAX_SORT,
        dead,
        tick,
    ))?;
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
    lua.set_named_registry_value(API, &wol)?;
    g.raw_set("wol", wol)?;
    lua.load(PRELUDE).set_name("=wol").exec()?;
    Ok(lua)
}

/// Registry key of the `wol` table [`sandbox`] made.
const API: &str = "wol.api";

/// The `wol` table [`sandbox`] made, put back as the global: grants go into it with `raw_set`, so
/// a script that replaced `wol` or gave it (or `_G`) a `__newindex` runs nothing while it loads
/// (audit L5).
pub(crate) fn api(lua: &Lua) -> mlua::Result<Table> {
    let wol: Table = lua.named_registry_value(API)?;
    lua.globals().raw_set("wol", &wol)?;
    Ok(wol)
}

/// Services for declared permissions, added once the file has loaded: a widget reaches them from
/// `render` and click handlers only, so loading one never acts.
fn grant(lua: &Lua, services: &Rc<dyn Services>, permissions: &[String]) -> mlua::Result<()> {
    if permissions.iter().any(|p| p == "notifications") {
        grant_notify(lua, services)?;
    }
    Ok(())
}

/// `wol.notify` calls one script may make per [`NOTIFY_WINDOW`] (audit C8-4: a loop in `render`
/// otherwise posted thousands of notifications, and the shell spawned a thread for each).
pub const NOTIFY_PER_WINDOW: usize = 10;
pub const NOTIFY_WINDOW: Duration = Duration::from_secs(60);
/// Longest notification title and body, in characters; longer text is cut.
pub const NOTIFY_TITLE: usize = 120;
pub const NOTIFY_BODY: usize = 1000;
/// What `wol.notify` answers past [`NOTIFY_PER_WINDOW`].
pub const TOO_MANY_NOTIFICATIONS: &str = "Too many notifications: a script may post 10 a minute.";

fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

pub(crate) fn grant_notify(lua: &Lua, services: &Rc<dyn Services>) -> mlua::Result<()> {
    let wol = api(lua)?;
    let s = services.clone();
    let sent: std::cell::RefCell<std::collections::VecDeque<Instant>> = Default::default();
    wol.raw_set(
        "notify",
        lua.create_function(move |_, (title, body): (String, Option<String>)| {
            let now = Instant::now();
            let mut sent = sent.borrow_mut();
            while sent
                .front()
                .is_some_and(|t| now.duration_since(*t) >= NOTIFY_WINDOW)
            {
                sent.pop_front();
            }
            if sent.len() >= NOTIFY_PER_WINDOW {
                return Ok((false, Some(TOO_MANY_NOTIFICATIONS.to_owned())));
            }
            sent.push_back(now);
            let (title, body) = (
                cut(&title, NOTIFY_TITLE),
                cut(body.as_deref().unwrap_or(""), NOTIFY_BODY),
            );
            Ok(match s.notify(&title, &body) {
                Ok(()) => (true, None),
                Err(m) => (false, Some(m)),
            })
        })?,
    )
}

/// Wall-clock limit of each kind of entry point (R2C-3). The instruction budget does not charge
/// for work done inside C functions, so a loop around a capped C call could otherwise run for
/// hours on the shell's GTK thread. The hook and every guard wrapper look at the clock; past the
/// deadline the script is stopped with [`TOO_LONG`] and stays stopped until it is loaded again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deadlines {
    /// Running the file once.
    pub load: Duration,
    /// `render` and `popover`.
    pub render: Duration,
    /// Input: `on_click`, `on_change`, `on_activate`, and a search `query`.
    pub event: Duration,
    /// The most time spent inside host [`Services`] (a `wol.run`, a state write, a D-Bus call)
    /// that one entry point gets back on its deadline (QA-PLUG-2): the deadline bounds the
    /// script's own work, and this bounds a loop of slow service calls.
    pub services: Duration,
}

/// Service time one entry point gets back on its deadline ([`Deadlines::services`]).
pub const SERVICE_CREDIT: Duration = Duration::from_secs(2);

impl Deadlines {
    /// 200 ms to load, 50 ms per render, 200 ms per click, change or query; up to 2 s of
    /// service time given back per entry point.
    pub const DEFAULT: Deadlines = Deadlines {
        load: Duration::from_millis(200),
        render: Duration::from_millis(50),
        event: Duration::from_millis(200),
        services: SERVICE_CREDIT,
    };
}

impl Default for Deadlines {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What the entry point running now may still spend: instructions and time. Shared by the hook,
/// the guard's `dead` and `tick`, and the entry points (which turn a spent meter into a stop).
pub(crate) struct Meter {
    left: AtomicU64,
    origin: Instant,
    /// Nanoseconds after `origin` at which the running entry point is late.
    until: AtomicU64,
    /// Ran out once: every later entry point answers [`TOO_LONG`] without running.
    stopped: AtomicBool,
    /// Service time (ns) this entry point may still get back on its deadline ([`OffClock`]).
    credit: AtomicU64,
}

impl Meter {
    pub(crate) fn new() -> Arc<Meter> {
        Arc::new(Meter {
            left: AtomicU64::new(0),
            origin: Instant::now(),
            until: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            credit: AtomicU64::new(0),
        })
    }

    /// Runs one host call and moves the deadline on by the time it took, up to the credit left.
    fn off_clock<T>(&self, f: impl FnOnce() -> T) -> T {
        let t0 = self.now();
        let r = f();
        let took = self.now().saturating_sub(t0);
        let back = took.min(self.credit.load(Ordering::Relaxed));
        self.credit.fetch_sub(back, Ordering::Relaxed);
        self.until.fetch_add(back, Ordering::Relaxed);
        r
    }

    fn now(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    /// Out of instructions or past the deadline; stays true until the next [`arm`].
    fn spent(&self) -> bool {
        if self.left.load(Ordering::Relaxed) == 0 {
            return true;
        }
        if self.now() >= self.until.load(Ordering::Relaxed) {
            self.left.store(0, Ordering::Relaxed);
            return true;
        }
        false
    }

    /// Err([`TOO_LONG`]) for a script stopped before, without running it.
    pub(crate) fn alive(&self) -> Result<(), String> {
        if self.stopped.load(Ordering::Relaxed) {
            return Err(TOO_LONG.into());
        }
        Ok(())
    }

    /// The outcome of one entry point: an error while spent stops the script for good.
    pub(crate) fn settle<T>(&self, r: Result<T, String>) -> Result<T, String> {
        match r {
            Err(_) if self.spent() => {
                self.stopped.store(true, Ordering::Relaxed);
                Err(TOO_LONG.into())
            }
            r => r,
        }
    }
}

/// The meter of the entry point running now, for the guard's `dead` and `tick`.
struct Budget(Arc<Meter>);

fn spent(lua: &Lua) -> bool {
    lua.app_data_ref::<Budget>().is_some_and(|b| b.0.spent())
}

/// Starts an entry point: a fresh instruction budget, a deadline `limit` from now and the
/// service credit of `d` ([`Deadlines::services`]).
pub(crate) fn arm(
    lua: &Lua,
    meter: &Arc<Meter>,
    limit: Duration,
    d: Deadlines,
) -> mlua::Result<()> {
    meter.left.store(INSTRUCTION_BUDGET, Ordering::Relaxed);
    meter.credit.store(
        u64::try_from(d.services.as_nanos()).unwrap_or(u64::MAX),
        Ordering::Relaxed,
    );
    let until = meter
        .now()
        .saturating_add(u64::try_from(limit.as_nanos()).unwrap_or(u64::MAX));
    meter.until.store(until, Ordering::Relaxed);
    lua.set_app_data(Budget(meter.clone()));
    let m = meter.clone();
    lua.set_global_hook(
        HookTriggers::new().every_nth_instruction(HOOK_STEP),
        move |_, _| {
            // Once spent, every later tick raises again until the next entry point.
            if m.spent() {
                return Err(mlua::Error::runtime(SPENT));
            }
            let left = m.left.load(Ordering::Relaxed);
            if left < u64::from(HOOK_STEP) {
                m.left.store(0, Ordering::Relaxed);
                return Err(mlua::Error::runtime(SPENT));
            }
            m.left.store(left - u64::from(HOOK_STEP), Ordering::Relaxed);
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

/// `refresh` of a script table: seconds between renders, 0 = only after input.
pub(crate) fn refresh_of(t: &Table) -> Result<u32, String> {
    match t.get::<Value>("refresh").map_err(err)? {
        Value::Nil => Ok(DEFAULT_REFRESH),
        Value::Integer(n) if (0..=MAX_REFRESH as i64).contains(&n) => Ok(n as u32),
        _ => Err(format!(
            "refresh is seconds between renders, 0 to {MAX_REFRESH}."
        )),
    }
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
        Self::load_within(name, src, services, Deadlines::DEFAULT)
    }

    /// [`Widget::load`] with the host's [`Deadlines`] (the load itself runs under `deadlines.load`).
    pub fn load_within(
        name: &str,
        src: &str,
        services: Rc<dyn Services>,
        deadlines: Deadlines,
    ) -> Result<Self, String> {
        let meter = Meter::new();
        let services = OffClock::wrap(services, &meter);
        let lua = sandbox(&services).map_err(err)?;
        arm(&lua, &meter, deadlines.load, deadlines).map_err(err)?;
        let v: Value = meter.settle(
            lua.load(src)
                .set_name(format!("={name}"))
                // Text only: a precompiled chunk is refused by Lua itself, not by luck of UTF-8.
                .set_mode(mlua::chunk::ChunkMode::Text)
                .eval()
                .map_err(err),
        )?;
        let Value::Table(t) = v else {
            return Err("The script must end with return wol.widget { ... }.".into());
        };
        if t.get::<Option<String>>("__wol_kind")
            .ok()
            .flatten()
            .as_deref()
            != Some("widget")
        {
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
        let refresh = refresh_of(&t)?;
        let Value::Function(render) = t.get::<Value>("render").map_err(err)? else {
            return Err("A widget needs a render function.".into());
        };
        grant(&lua, &services, &permissions(&t)?).map_err(err)?;
        Ok(Self {
            lua,
            name: title,
            refresh,
            render,
            handlers: Handlers::default(),
            meter,
            deadlines,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The limits later renders and clicks run under.
    pub fn set_deadlines(&mut self, deadlines: Deadlines) {
        self.deadlines = deadlines;
    }

    /// True once the widget ran out of time or instructions: it answers [`TOO_LONG`] without
    /// running until its file is loaded again.
    pub fn stopped(&self) -> bool {
        self.meter.alive().is_err()
    }

    /// Seconds between renders; 0 = only after a click.
    pub fn refresh(&self) -> u32 {
        self.refresh
    }

    /// The tree to draw now. Interactive parts are numbered in tree order.
    pub fn render(&mut self) -> Result<Node, String> {
        self.meter.alive()?;
        arm(
            &self.lua,
            &self.meter,
            self.deadlines.render,
            self.deadlines,
        )
        .map_err(err)?;
        let r = self.render.call(()).map_err(err).and_then(|v: Value| {
            let mut handlers = Handlers::default();
            let node = to_node(&v, &mut handlers, 1, None)?;
            Ok((node, handlers))
        });
        let (node, handlers) = self.meter.settle(r)?;
        node.validate()?;
        self.handlers = handlers;
        Ok(node)
    }

    /// Run a button's handler (ids from the last render).
    pub fn click(&mut self, id: u32) -> Result<(), String> {
        self.meter.alive()?;
        let f = self.handlers.get(id)?;
        arm(&self.lua, &self.meter, self.deadlines.event, self.deadlines).map_err(err)?;
        self.meter.settle(f.call::<()>(()).map_err(err))
    }

    /// A field, toggle or slider changed: run its `on_change(value)`.
    pub fn change(&mut self, id: u32, value: Input) -> Result<(), String> {
        self.meter.alive()?;
        let f = self.handlers.get(id)?;
        arm(&self.lua, &self.meter, self.deadlines.event, self.deadlines).map_err(err)?;
        let r = match value {
            Input::Text(s) => f.call::<()>(s),
            Input::On(b) => f.call::<()>(b),
            Input::Number(n) => f.call::<()>(n),
        }
        .map_err(err);
        self.meter.settle(r)
    }
}

/// What the person entered (`FromHost::Edited`, `Toggled`, `Slid`).
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Text(String),
    On(bool),
    Number(f64),
}

/// A Lua part table to a node. `dir`: plugin folder, so `wol.image { "icon.png" }` resolves inside it.
/// What a plugin hears for an image outside its folder.
pub(crate) const IMAGE_INSIDE: &str =
    "An image file is inside the plugin folder, like \"icon.png\".";

/// A plugin's image or icon (QA-PLUG-18): an icon name as it is; anything that looks like a file
/// (a `/`, an image extension) only inside the plugin folder `dir`, never an absolute path or
/// `..`, so a card cannot show a picture of the person's own files. `refused` is the sentence.
pub(crate) fn plugin_image(dir: &Path, source: &str, refused: &str) -> Result<String, String> {
    let file = source.contains('/')
        || [".png", ".svg", ".jpg", ".jpeg"]
            .iter()
            .any(|e| source.to_ascii_lowercase().ends_with(e));
    if !file {
        return Ok(source.to_owned());
    }
    if shared::plugins::is_plugin_path(source) {
        Ok(dir.join(source).to_string_lossy().into_owned())
    } else {
        Err(refused.into())
    }
}

pub(crate) fn to_node(
    v: &Value,
    handlers: &mut Handlers,
    depth: usize,
    dir: Option<&Path>,
) -> Result<Node, String> {
    if depth > shared::view::MAX_DEPTH {
        return Err("The view's parts are nested too deep.".into());
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
    let number = |key: &str, default: f64| -> Result<f64, String> {
        match t.get::<Value>(key).map_err(err)? {
            Value::Integer(n) => Ok(n as f64),
            Value::Number(n) => Ok(n),
            Value::Nil => Ok(default),
            _ => Err(format!("A {kind}'s {key} is a number.")),
        }
    };
    let handler = |key: &str| -> Result<Option<Function>, String> {
        match t.get::<Value>(key).map_err(err)? {
            Value::Function(f) => Ok(Some(f)),
            Value::Nil => Ok(None),
            _ => Err(format!("A {kind}'s {key} is a function.")),
        }
    };
    let children = |handlers: &mut Handlers| -> Result<Vec<Node>, String> {
        let list: Table = t.get("children").map_err(err)?;
        let mut out = Vec::new();
        for c in list.sequence_values::<Value>() {
            out.push(to_node(&c.map_err(err)?, handlers, depth + 1, dir)?);
        }
        Ok(out)
    };
    Ok(match kind.as_str() {
        "stack" | "row" => {
            let spacing: i64 = t.get("spacing").unwrap_or(0);
            let spacing = u8::try_from(spacing)
                .ok()
                .filter(|s| SPACING.contains(s))
                .ok_or_else(|| format!("Spacing is one of {SPACING:?}."))?;
            let children = children(handlers)?;
            if kind == "stack" {
                Node::Stack { spacing, children }
            } else {
                Node::Row { spacing, children }
            }
        }
        "list" => Node::List {
            children: children(handlers)?,
        },
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
            let Some(f) = handler("on_click")? else {
                return Err("A button needs on_click = function() ... end.".into());
            };
            Node::Button {
                id: handlers.id(Some(f)),
                label: text("label")?,
            }
        }
        "spacer" => Node::Spacer,
        "image" => {
            let source = text("source")?;
            if source.is_empty() {
                return Err("An image needs a source: an icon name or an image file.".into());
            }
            let size = number("size", f64::from(DEFAULT_IMAGE))?;
            let size = u16::try_from(size as i64)
                .ok()
                .filter(|s| IMAGE_SIZES.contains(s))
                .ok_or_else(|| format!("Image size is one of {IMAGE_SIZES:?}."))?;
            let source = match dir {
                Some(d) => plugin_image(d, &source, IMAGE_INSIDE)?,
                None => source,
            };
            Node::Image { source, size }
        }
        "toggle" => Node::Toggle {
            id: handlers.id(handler("on_change")?),
            on: t.get::<Option<bool>>("on").map_err(err)?.unwrap_or(false),
            label: text("label")?,
        },
        "slider" => Node::Slider {
            id: handlers.id(handler("on_change")?),
            value: number("value", 0.0)?,
            min: number("min", 0.0)?,
            max: number("max", 100.0)?,
        },
        "text_field" => Node::TextField {
            id: handlers.id(handler("on_change")?),
            text: text("text")?,
            placeholder: text("placeholder")?,
        },
        other => {
            return Err(format!(
                "Unknown part \"{other}\": stack, row, list, label, button, spacer, image, toggle, slider, text_field or item."
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
    deadlines: Deadlines,
}

impl Library {
    pub fn new(dir: PathBuf, services: Rc<dyn Services>) -> Self {
        Self {
            dir,
            services,
            entries: BTreeMap::new(),
            deadlines: Deadlines::DEFAULT,
        }
    }

    /// The limits every widget of the folder runs under, now and after a reload.
    pub fn set_deadlines(&mut self, deadlines: Deadlines) {
        self.deadlines = deadlines;
        for w in self.entries.values_mut().filter_map(|e| e.widget.as_mut()) {
            w.set_deadlines(deadlines);
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
                    .and_then(|src| {
                        Widget::load_within(&id, &src, self.services.clone(), self.deadlines)
                    }),
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

    /// Method names in the block that starts with `head` (one level of braces).
    fn methods(src: &str, head: &str) -> Vec<String> {
        let at = src.find(head).expect(head);
        let body = &src[at..];
        let mut depth = 0i32;
        let mut end = body.len();
        for (i, c) in body.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let mut names: Vec<String> = body[..end]
            .lines()
            .filter_map(|l| l.trim().strip_prefix("fn "))
            .map(|l| l.split('(').next().unwrap_or_default().to_owned())
            .collect();
        names.sort();
        names
    }

    /// QA-PLUG-2 standard: every host service a script can reach runs off its clock, so a new
    /// [`Services`] method (whose default would otherwise slip past the wrapper) must be forwarded
    /// by [`OffClock`], and both loaders wrap the host's services before building the sandbox.
    #[test]
    fn off_clock_wraps_every_service() {
        let lib = include_str!("lib.rs");
        let want = methods(lib, "pub trait Services {");
        assert!(want.len() >= 12, "{want:?}");
        assert_eq!(methods(lib, "impl Services for OffClock {"), want);
        let plugin = include_str!("plugin.rs");
        for (file, src) in [("lib.rs", lib), ("plugin.rs", plugin)] {
            let sandboxes = src.matches("= sandbox(&services)").count();
            let wraps = src.matches("OffClock::wrap(services, &meter);").count();
            assert!(
                sandboxes >= 1 && wraps == sandboxes,
                "{file}: {sandboxes} sandboxes, {wraps} wraps"
            );
        }
    }
}
