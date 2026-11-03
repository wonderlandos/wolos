// SPDX-License-Identifier: GPL-2.0-or-later
//! Plugin scripts, headless (ADR 0031): every entry kind loads, renders and answers input; the
//! service calls follow the manifest's permissions and refuse with a sentence; state and JSON
//! round-trip; runaway or greedy scripts are stopped in every entry point.
use shared::plugins::{Kind, Permission};
use shared::view::{Node, Role};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::Path;
use std::rc::Rc;
use wol_lua::plugin::{Entry, Hit, Script};
use wol_lua::{Input, Services, Stamp};

/// Records everything; grants `api.example.com`.
#[derive(Default)]
struct Fake {
    runs: RefCell<Vec<Vec<String>>>,
    posted: RefCell<Vec<(String, String)>>,
    opened: RefCell<Vec<String>>,
    shown: RefCell<Vec<String>>,
    toggles: RefCell<BTreeSet<String>>,
    state: RefCell<Option<String>>,
    saves: RefCell<u32>,
    clipboard: Option<String>,
}

fn info() -> wol::system::Info {
    wol::system::Info {
        os_name: "WolOS".into(),
        os_version: "0.12".into(),
        kernel: "WolKernel".into(),
        appearance: wol::system::Appearance::Dark,
        accent_name: "blue".into(),
        accent_rgb: 0x0a84ff,
    }
}

fn stamp() -> Stamp {
    Stamp {
        year: 2026,
        month: 11,
        day: 1,
        weekday: 7,
        hour: 9,
        minute: 30,
        second: 0,
    }
}

impl Services for Fake {
    fn now(&self) -> Stamp {
        stamp()
    }
    fn system(&self) -> wol::system::Info {
        info()
    }
    fn notify(&self, title: &str, body: &str) -> Result<(), String> {
        self.posted.borrow_mut().push((title.into(), body.into()));
        Ok(())
    }
    fn run(&self, plugin: &str, argv: &[String], timeout_secs: u32) -> Result<String, String> {
        assert_eq!(plugin, "alice.demo");
        self.runs.borrow_mut().push(argv.to_vec());
        Ok(format!("ran {} in {timeout_secs}s", argv.join(" ")))
    }
    fn http_get(&self, _plugin: &str, url: &str) -> Result<String, String> {
        let host = shared::widgets::manifest::url_host(url).unwrap_or_default();
        if host == "api.example.com" {
            Ok(r#"{"text": "hello", "n": [1, 2]}"#.into())
        } else {
            Err(format!("Allow this plugin to read data from {host}."))
        }
    }
    fn open(&self, _plugin: &str, target: &str) -> Result<(), String> {
        self.opened.borrow_mut().push(target.into());
        Ok(())
    }
    fn toggle_get(&self, _plugin: &str, toggle: &str) -> bool {
        self.toggles.borrow().contains(toggle)
    }
    fn toggle_set(&self, _plugin: &str, toggle: &str, on: bool) -> Result<(), String> {
        if on {
            self.toggles.borrow_mut().insert(toggle.into());
        } else {
            self.toggles.borrow_mut().remove(toggle);
        }
        Ok(())
    }
    fn clipboard_text(&self, _plugin: &str) -> Result<String, String> {
        self.clipboard
            .clone()
            .ok_or_else(|| "The clipboard is empty.".to_owned())
    }
    fn show(&self, _plugin: &str, panel: &str) -> Result<(), String> {
        self.shown.borrow_mut().push(panel.into());
        Ok(())
    }
    fn state_load(&self, _plugin: &str) -> Option<String> {
        self.state.borrow().clone()
    }
    fn state_save(&self, _plugin: &str, json: &str) -> Result<(), String> {
        *self.saves.borrow_mut() += 1;
        *self.state.borrow_mut() = Some(json.into());
        Ok(())
    }
}

/// Only the three required calls: every plugin service refuses with its default sentence.
struct Bare;

impl Services for Bare {
    fn now(&self) -> Stamp {
        stamp()
    }
    fn system(&self) -> wol::system::Info {
        info()
    }
    fn notify(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
}

fn entry(kind: Kind) -> Entry<'static> {
    Entry {
        kind,
        plugin: "alice.demo",
        id: "x",
        dir: Path::new("/plug/alice.demo"),
    }
}

fn load_with(
    kind: Kind,
    src: &str,
    perms: &[Permission],
    s: Rc<dyn Services>,
) -> Result<Script, String> {
    Script::load(entry(kind), src, perms, s)
}

fn load(kind: Kind, src: &str, perms: &[Permission]) -> Result<Script, String> {
    load_with(kind, src, perms, Rc::new(Fake::default()))
}

fn label(text: &str, role: Role) -> Node {
    Node::Label {
        text: text.into(),
        role,
    }
}

/// `render = function() return wol.label { tostring(<expr>) } end` as a widget.
fn shows(expr: &str, perms: &[Permission], s: Rc<dyn Services>) -> String {
    let src = format!(
        "return wol.widget {{ render = function() return wol.label {{ tostring({expr}) }} end }}"
    );
    let mut w = load_with(Kind::Widget, &src, perms, s).unwrap();
    match w.render().unwrap() {
        Node::Label { text, .. } => text,
        other => panic!("{other:?}"),
    }
}

const BAR: &str = r#"
local n = 0
return wol.menubar {
  refresh = 5,
  render = function() return wol.row { spacing = 4, wol.image { "emblem-default-symbolic", size = 16 }, wol.label { "n=" .. n } } end,
  popover = function() return wol.stack { spacing = 8, wol.label { "Popover", role = "title" }, wol.button { label = "+1", on_click = function() n = n + 1 end } } end,
}
"#;

#[test]
fn menubar_renders_pops_over_and_clicks() {
    let mut s = load(Kind::Menubar, BAR, &[]).unwrap();
    assert_eq!(
        (s.kind(), s.plugin(), s.entry(), s.refresh()),
        (Kind::Menubar, "alice.demo", "x", 5)
    );
    assert_eq!(s.title(), "");
    let Node::Row { children, .. } = s.render().unwrap() else {
        panic!()
    };
    assert_eq!(
        children[0],
        Node::Image {
            source: "emblem-default-symbolic".into(),
            size: 16
        }
    );
    assert_eq!(children[1], label("n=0", Role::Body));
    let Some(Node::Stack { children, .. }) = s.popover().unwrap() else {
        panic!()
    };
    let Node::Button { id, label: l } = &children[1] else {
        panic!("{children:?}")
    };
    assert_eq!(l, "+1");
    s.click(*id).unwrap();
    let Node::Row { children, .. } = s.render().unwrap() else {
        panic!()
    };
    assert_eq!(children[1], label("n=1", Role::Body));
    assert!(
        s.click(*id)
            .unwrap_err()
            .contains("not on the view any more"),
        "render replaced the popover's handlers"
    );
    let mut plain = load(
        Kind::Widget,
        "return wol.widget { name = 'Card', render = function() return wol.spacer() end }",
        &[],
    )
    .unwrap();
    assert_eq!(plain.popover().unwrap(), None);
    assert_eq!(plain.title(), "Card");
}

#[test]
fn kinds_must_match_and_search_answers_queries() {
    let widget = "return wol.widget { render = function() return wol.spacer() end }";
    let e = load(Kind::Menubar, widget, &[]).unwrap_err();
    assert_eq!(e, "The script must end with return wol.menubar { ... }.");
    assert!(
        load(Kind::Search, widget, &[])
            .unwrap_err()
            .contains("wol.search")
    );
    assert!(
        load(Kind::Dock, widget, &[])
            .unwrap_err()
            .contains("no script")
    );
    assert!(
        load(Kind::Panel, "return wol.panel { title = 'P' }", &[])
            .unwrap_err()
            .contains("render")
    );
    assert!(
        load(Kind::Search, "return wol.search { }", &[])
            .unwrap_err()
            .contains("query")
    );
    assert!(
        load(
            Kind::Panel,
            "return wol.panel { title = 'x', render = 5 }",
            &[]
        )
        .unwrap_err()
        .contains("render is a function")
    );
    assert!(
        load(Kind::Widget, "return 1", &[])
            .unwrap_err()
            .contains("wol.widget")
    );
    assert!(
        load(Kind::Widget, "nope(", &[])
            .unwrap_err()
            .contains("syntax")
            || load(Kind::Widget, "nope(", &[]).is_err()
    );

    let search = r#"
local picked = nil
return wol.search {
  query = function(text)
    if text == "" then return nil end
    return {
      { title = "Echo " .. text, subtitle = "sub", icon = "pic.png", on_activate = function() picked = text end },
      { title = "Plain" },
    }
  end,
}
"#;
    let mut s = load(Kind::Search, search, &[]).unwrap();
    assert!(s.render().unwrap_err().contains("query"));
    assert_eq!(s.query("").unwrap(), Vec::<Hit>::new());
    let hits = s.query("hi").unwrap();
    assert_eq!(
        hits,
        vec![
            Hit {
                id: 1,
                title: "Echo hi".into(),
                subtitle: "sub".into(),
                icon: "/plug/alice.demo/pic.png".into()
            },
            Hit {
                id: 2,
                title: "Plain".into(),
                subtitle: String::new(),
                icon: String::new()
            },
        ]
    );
    s.activate(1).unwrap();
    assert!(s.activate(2).is_err(), "no on_activate");
    for (q, want) in [
        (
            "return wol.search { query = function() return 5 end }",
            "list of rows",
        ),
        (
            "return wol.search { query = function() return { 5 } end }",
            "table with a title",
        ),
        (
            "return wol.search { query = function() return { { title = '' } } end }",
            "needs a title",
        ),
        (
            "return wol.search { query = function() return { { title = 'x', icon = '../x.png' } } end }",
            "inside the plugin folder",
        ),
        (
            "return wol.search { query = function() return { { title = 'x', on_activate = 1 } } end }",
            "on_activate is a function",
        ),
    ] {
        let e = load(Kind::Search, q, &[]).unwrap().query("t").unwrap_err();
        assert!(e.contains(want), "{q}: {e}");
    }
}

#[test]
fn services_exist_only_with_their_permission_and_refuse_with_a_sentence() {
    let none: &[Permission] = &[];
    let fake = Rc::new(Fake::default());
    for call in [
        "wol.run",
        "wol.http_get",
        "wol.open",
        "wol.toggle_set",
        "wol.clipboard",
        "wol.notify",
    ] {
        assert_eq!(
            shows(call, none, fake.clone()),
            "nil",
            "{call} without permission"
        );
    }
    for call in [
        "wol.toggle_get",
        "wol.show",
        "wol.state.get",
        "wol.json.decode",
        "wol.now",
        "wol.system",
    ] {
        // tostring() of a function carries its address; type() names what exists.
        assert_eq!(
            shows(&format!("type({call})"), none, fake.clone()),
            "function",
            "{call} needs no permission"
        );
    }
    let all = Permission::ALL;
    assert_eq!(
        shows(
            "select(2, wol.run { 'echo', 'hi', timeout = 3 })",
            &all,
            fake.clone()
        ),
        "ran echo hi in 3s"
    );
    assert_eq!(fake.runs.borrow()[0], ["echo", "hi"]);
    assert_eq!(
        shows("select(2, wol.run { 'echo', 1 })", &all, fake.clone()),
        "ran echo 1 in 5s",
        "numbers become text, default timeout"
    );
    assert!(shows("select(2, wol.run { })", &all, fake.clone()).contains("needs a program"));
    assert!(
        shows(
            "select(2, wol.run { 'x', timeout = 99 })",
            &all,
            fake.clone()
        )
        .contains("1 to 30")
    );
    assert!(
        shows("select(2, wol.run { function() end })", &all, fake.clone()).contains("as strings")
    );
    assert_eq!(
        shows("wol.open('https://wolos.org')", &all, fake.clone()),
        "true"
    );
    assert_eq!(fake.opened.borrow()[0], "https://wolos.org");
    assert!(shows("select(2, wol.open(''))", &all, fake.clone()).contains("takes a file"));
    assert_eq!(
        shows("wol.notify('Hi', 'there')", &all, fake.clone()),
        "true"
    );
    assert_eq!(
        fake.posted.borrow()[0],
        ("Hi".to_string(), "there".to_string())
    );
    assert_eq!(shows("wol.clipboard.text()", &all, fake.clone()), "nil");
    assert_eq!(
        shows("select(2, wol.clipboard.text())", &all, fake.clone()),
        "The clipboard is empty."
    );
    let full = Rc::new(Fake {
        clipboard: Some("pasted".into()),
        ..Fake::default()
    });
    assert_eq!(shows("wol.clipboard.text()", &all, full), "pasted");

    // A host that offers none of them: every call answers false and the default sentence.
    let bare: Rc<dyn Services> = Rc::new(Bare);
    for (call, want) in [
        (
            "select(2, wol.run { 'ls' })",
            "Running commands is not available here.",
        ),
        (
            "select(2, wol.http_get('https://a.b/'))",
            "Reading from the web is not available here.",
        ),
        (
            "select(2, wol.open('x'))",
            "Opening things is not available here.",
        ),
        (
            "select(2, wol.toggle_set('a', true))",
            "Toggles are not available here.",
        ),
        (
            "select(2, wol.clipboard.text())",
            "The clipboard is not available here.",
        ),
        (
            "select(2, wol.show('control-center'))",
            "Panels are not available here.",
        ),
        (
            "select(2, wol.state.set('k', 1))",
            "Saving state is not available here.",
        ),
    ] {
        assert_eq!(shows(call, &all, bare.clone()), want, "{call}");
    }
    assert_eq!(shows("wol.toggle_get('a')", &all, bare.clone()), "false");
    assert_eq!(shows("wol.state.get('k')", &all, bare), "nil");
}

#[test]
fn http_needs_a_host_grant_and_json_decodes() {
    let all = Permission::ALL;
    let fake = Rc::new(Fake::default());
    assert_eq!(
        shows(
            "select(2, wol.http_get('https://other.example.org/x'))",
            &all,
            fake.clone()
        ),
        "Allow this plugin to read data from other.example.org."
    );
    assert!(shows("select(2, wol.http_get('ftp://x'))", &all, fake.clone()).contains("https://"));
    assert_eq!(
        shows(
            "(function() local b = wol.http_get('https://api.example.com/q'); local v = wol.json.decode(b); return v.text .. v.n[2] end)()",
            &all,
            fake.clone()
        ),
        "hello2"
    );
    assert_eq!(
        shows(
            "(function() local v = wol.json.decode('{\"a\": [1, 2.5, true, null, \"s\"], \"o\": {\"k\": \"v\"}}'); return #v.a .. v.a[2] .. tostring(v.a[3]) .. v.o.k end)()",
            &all,
            fake.clone()
        ),
        "42.5truev"
    );
    assert!(
        shows("select(2, wol.json.decode('{nope'))", &all, fake.clone()).starts_with("Not JSON")
    );
    assert_eq!(
        shows(
            "wol.json.encode({ a = { 1, 2 }, b = 'x', c = true })",
            &all,
            fake.clone()
        ),
        r#"{"a":[1,2],"b":"x","c":true}"#
    );
    assert_eq!(shows("wol.json.encode({})", &all, fake.clone()), "{}");
    assert!(
        shows(
            "select(2, wol.json.encode({ f = wol.now }))",
            &all,
            fake.clone()
        )
        .contains("text, numbers")
    );
    assert!(
        shows(
            "select(2, wol.json.encode({ 1, x = 2 }))",
            &all,
            fake.clone()
        )
        .contains("not both")
    );
    assert!(shows("select(2, wol.json.encode(0/0))", &all, fake.clone()).contains("finite"));
}

#[test]
fn state_round_trips_and_refuses_bombs() {
    let fake = Rc::new(Fake::default());
    let none: &[Permission] = &[];
    assert_eq!(shows("wol.state.get('count')", none, fake.clone()), "nil");
    assert_eq!(
        shows("wol.state.set('count', 3)", none, fake.clone()),
        "true"
    );
    assert_eq!(fake.state.borrow().as_deref(), Some(r#"{"count":3}"#));
    assert_eq!(
        shows("wol.state.set('list', { 'a', 'b' })", none, fake.clone()),
        "true"
    );
    assert_eq!(
        shows(
            "wol.state.set('obj', { k = { deep = true } })",
            none,
            fake.clone()
        ),
        "true"
    );
    assert_eq!(
        fake.state.borrow().as_deref(),
        Some(r#"{"count":3,"list":["a","b"],"obj":{"k":{"deep":true}}}"#)
    );
    // A fresh script sees what the last one saved.
    assert_eq!(
        shows(
            "wol.state.get('list')[2] .. tostring(wol.state.get('obj').k.deep) .. wol.state.get('count')",
            none,
            fake.clone()
        ),
        "btrue3"
    );
    assert_eq!(
        shows("wol.state.set('count', nil)", none, fake.clone()),
        "true"
    );
    assert!(!fake.state.borrow().as_deref().unwrap().contains("count"));
    let saves = *fake.saves.borrow();
    assert!(shows("select(2, wol.state.set('', 1))", none, fake.clone()).contains("1 to 64"));
    assert!(
        shows(
            "select(2, wol.state.set('f', function() end))",
            none,
            fake.clone()
        )
        .contains("text, numbers")
    );
    assert!(
        shows(
            "select(2, wol.state.set('big', string.rep('x', 70000)))",
            none,
            fake.clone()
        )
        .contains("64 KB")
    );
    let deep = "(function() local t = {} local c = t for i = 1, 40 do c.n = {} c = c.n end return select(2, wol.state.set('deep', t)) end)()";
    assert!(shows(deep, none, fake.clone()).contains("nested too deep"));
    assert_eq!(*fake.saves.borrow(), saves, "refused sets save nothing");
    // A broken file on disk is an empty state, not a crash.
    *fake.state.borrow_mut() = Some("{broken".into());
    assert_eq!(shows("wol.state.get('list')", none, fake.clone()), "nil");
}

#[test]
fn new_parts_resolve_images_in_the_folder_and_take_input() {
    let src = r#"
local on, level, name = false, 10, ""
return wol.panel {
  title = "Sheet",
  render = function()
    return wol.stack { spacing = 12,
      wol.image { "icon.png" },
      wol.toggle { label = "On", on = on, on_change = function(v) on = v end },
      wol.slider { value = level, min = 0, max = 100, on_change = function(v) level = v end },
      wol.text_field { placeholder = "Name", text = name, on_change = function(t) name = t end },
      wol.list { wol.item { title = "Row", subtitle = "Sub", icon = "folder-symbolic", button = { label = "Go", on_click = function() end } } },
      wol.label { tostring(on) .. "/" .. level .. "/" .. name, role = "caption" },
    }
  end,
}
"#;
    let mut s = load(Kind::Panel, src, &[]).unwrap();
    assert_eq!(s.title(), "Sheet");
    let Node::Stack { children, .. } = s.render().unwrap() else {
        panic!()
    };
    assert_eq!(
        children[0],
        Node::Image {
            source: "/plug/alice.demo/icon.png".into(),
            size: 24
        }
    );
    assert_eq!(
        children[1],
        Node::Toggle {
            id: 1,
            on: false,
            label: "On".into()
        }
    );
    assert_eq!(
        children[2],
        Node::Slider {
            id: 2,
            value: 10.0,
            min: 0.0,
            max: 100.0
        }
    );
    assert_eq!(
        children[3],
        Node::TextField {
            id: 3,
            text: String::new(),
            placeholder: "Name".into()
        }
    );
    let Node::List { children: rows } = &children[4] else {
        panic!("{children:?}")
    };
    let Node::Row { children: row, .. } = &rows[0] else {
        panic!()
    };
    assert_eq!(
        row[0],
        Node::Image {
            source: "folder-symbolic".into(),
            size: 24
        }
    );
    assert_eq!(
        row[1],
        Node::Stack {
            spacing: 0,
            children: vec![label("Row", Role::Body), label("Sub", Role::Secondary)]
        }
    );
    assert_eq!(row[2], Node::Spacer);
    assert_eq!(
        row[3],
        Node::Button {
            id: 4,
            label: "Go".into()
        }
    );
    s.change(1, Input::On(true)).unwrap();
    s.change(2, Input::Number(55.0)).unwrap();
    s.change(3, Input::Text("Ada".into())).unwrap();
    s.click(4).unwrap();
    let Node::Stack { children, .. } = s.render().unwrap() else {
        panic!()
    };
    assert_eq!(children[5], label("true/55.0/Ada", Role::Caption));
    for (body, want) in [
        ("wol.image { '../x.png' }", "inside the plugin folder"),
        ("wol.image { 'x.png', size = 20 }", "Image size"),
        ("wol.image { '' }", "needs a source"),
        ("{ type = 'frame' }", "Unknown part \"frame\""),
        ("wol.slider { value = 5, min = 10, max = 1 }", "min < max"),
        (
            "wol.toggle { on = true, on_change = 5 }",
            "on_change is a function",
        ),
    ] {
        let src = format!("return wol.widget {{ render = function() return {body} end }}");
        let e = load(Kind::Widget, &src, &[]).unwrap().render().unwrap_err();
        assert!(e.contains(want), "{body}: {e}");
    }
}

/// Audit L5: a script that replaces `wol`, or puts `__newindex` on it or on `_G`, cannot make
/// the host's grants run its code while it loads.
#[test]
fn loading_never_acts_even_through_metatables() {
    let tricks = [
        r#"wol = setmetatable({}, { __newindex = function(t, k, v) if k == "run" then v { "wol", "notify", "x" } end rawset(t, k, v) end, __index = wol })
           return wol.widget { render = function() return wol.spacer() end }"#,
        r#"local real = wol setmetatable(real, { __newindex = function(t, k, v) if k == "run" then v { "wol", "notify", "x" } end rawset(t, k, v) end })
           return wol.widget { render = function() return wol.spacer() end }"#,
        r#"local real = wol wol = nil setmetatable(_G, { __index = function(_, k) if k == "wol" then return setmetatable({}, { __newindex = function(t, k2, v) if k2 == "run" then v { "wol", "notify", "x" } end end }) end end })
           return real.widget { render = function() return real.spacer() end }"#,
    ];
    for src in tricks {
        let fake = Rc::new(Fake::default());
        let s = load_with(Kind::Widget, src, &[Permission::Run], fake.clone());
        assert!(s.is_ok(), "{s:?}");
        assert!(fake.runs.borrow().is_empty(), "{src}");
    }
    // The real table is the global again, with the grant in it.
    let fake = Rc::new(Fake::default());
    let mut s = load_with(Kind::Widget, tricks[0], &[Permission::Run], fake.clone()).unwrap();
    s.render().unwrap();
    assert!(fake.runs.borrow().is_empty());
    assert_eq!(
        shows(
            "type(wol.run)",
            &[Permission::Run],
            Rc::new(Fake::default())
        ),
        "function"
    );
}

#[test]
fn runaway_and_greedy_scripts_are_stopped_in_every_entry_point() {
    let start = std::time::Instant::now();
    let mut s = load(
        Kind::Search,
        "return wol.search { query = function() while true do end end }",
        &[],
    )
    .unwrap();
    assert!(s.query("x").unwrap_err().contains("too long"));
    let mut s = load(Kind::Menubar, "return wol.menubar { render = function() return wol.spacer() end, popover = function() while true do end end }", &[]).unwrap();
    s.render().unwrap();
    assert!(s.popover().unwrap_err().contains("too long"));
    let mut s = load(Kind::Panel, "return wol.panel { render = function() local t = {} for i = 1, 1e9 do t[i] = string.rep('x', 1000) end end }", &[]).unwrap();
    assert!(s.render().is_err());
    assert!(start.elapsed() < std::time::Duration::from_secs(15));
    // Too many rows, too many parts.
    let mut s = load(Kind::Search, "return wol.search { query = function() local r = {} for i = 1, 21 do r[i] = { title = 'r' .. i } end return r end }", &[]).unwrap();
    assert!(s.query("x").unwrap_err().contains("at most 20 rows"));
    let mut s = load(Kind::Widget, "return wol.widget { render = function() local t = wol.stack {} for i = 1, 201 do t.children[i] = wol.spacer() end return t end }", &[]).unwrap();
    assert!(s.render().unwrap_err().contains("at most 200 parts"));
    let mut s = load(Kind::Widget, "return wol.widget { render = function() local t = wol.stack {} local c = t for i = 1, 13 do c.children[1] = wol.stack {} c = c.children[1] end return t end }", &[]).unwrap();
    assert!(s.render().unwrap_err().contains("nested too deep"));
    // A handler that loops is stopped too, and the script stays stopped until it is loaded again
    // (R2C-3).
    let mut s = load(Kind::Widget, "return wol.widget { render = function() return wol.button { label = 'x', on_click = function() while true do end end } end }", &[]).unwrap();
    s.render().unwrap();
    assert!(s.click(1).unwrap_err().contains("too long"));
    assert!(s.render().unwrap_err().contains("too long"));
    // The sandbox is the widget one: no files, processes or loaders.
    for g in [
        "io",
        "os",
        "require",
        "debug",
        "load",
        "dofile",
        "string.dump",
    ] {
        assert_eq!(shows(g, &[], Rc::new(Fake::default())), "nil", "{g}");
    }
}

#[test]
fn toggles_panels_and_show_go_through_the_host() {
    let all = Permission::ALL;
    let fake = Rc::new(Fake::default());
    assert_eq!(
        shows("wol.toggle_get('awake')", &all, fake.clone()),
        "false"
    );
    assert_eq!(
        shows("wol.toggle_set('awake', true)", &all, fake.clone()),
        "true"
    );
    assert_eq!(shows("wol.toggle_get('awake')", &all, fake.clone()), "true");
    assert!(
        shows(
            "select(2, wol.toggle_set('Not An Id', true))",
            &all,
            fake.clone()
        )
        .contains("[[toggle]] id")
    );
    assert_eq!(
        shows("wol.toggle_get('Not An Id')", &all, fake.clone()),
        "false"
    );
    assert_eq!(
        shows("wol.show('control-center')", &all, fake.clone()),
        "true"
    );
    assert_eq!(
        shows("wol.show('sheet')", &all, fake.clone()),
        "true",
        "the host decides which panels exist"
    );
    assert!(shows("select(2, wol.show('Not A Panel'))", &all, fake.clone()).contains("panel name"));
    assert_eq!(*fake.shown.borrow(), ["control-center", "sheet"]);
    assert_eq!(
        fake.toggles.borrow().iter().cloned().collect::<Vec<_>>(),
        ["awake"]
    );
}

/// R2C-3: every plugin entry point has a wall-clock deadline, and a stopped script stays stopped.
#[test]
fn every_entry_point_stops_at_its_deadline() {
    let d = wol_lua::Deadlines::DEFAULT;
    let bomb = "local t = {} while true do table.move(t, 1, 2^20 - 1, 2) end";
    let timed = |what: &str, limit: std::time::Duration, f: &mut dyn FnMut() -> String| {
        let start = std::time::Instant::now();
        let e = f();
        let took = start.elapsed();
        assert_eq!(e, wol_lua::TOO_LONG, "{what}");
        assert!(took < 2 * limit, "{what}: {took:?}");
    };
    let mut s = load(
        Kind::Search,
        &format!("return wol.search {{ query = function() {bomb} end }}"),
        &[],
    )
    .unwrap();
    timed("query", d.event, &mut || s.query("x").unwrap_err());
    assert!(s.stopped());
    assert_eq!(s.query("x").unwrap_err(), wol_lua::TOO_LONG);
    let mut s = load(
        Kind::Menubar,
        &format!(
            "return wol.menubar {{ render = function() return wol.spacer() end, popover = function() {bomb} end }}"
        ),
        &[],
    )
    .unwrap();
    s.render().unwrap();
    timed("popover", d.render, &mut || s.popover().unwrap_err());
    assert_eq!(s.render().unwrap_err(), wol_lua::TOO_LONG);
    let mut s = load(
        Kind::Panel,
        &format!("return wol.panel {{ render = function() return wol.button {{ 'go', on_click = function() {bomb} end }} end }}"),
        &[],
    )
    .unwrap();
    s.render().unwrap();
    timed("click", d.event, &mut || s.click(1).unwrap_err());
    timed("load", d.load, &mut || {
        load(Kind::Widget, bomb, &[]).unwrap_err()
    });
}

/// Audit C8-4: a loop of `wol.notify` in one render reaches the host at most
/// `NOTIFY_PER_WINDOW` times; the rest answer `false, "Too many ..."`, and long text is cut.
#[test]
fn notify_is_rate_limited_and_capped() {
    let fake = Rc::new(Fake::default());
    let src = r#"
local refused, why = 0, ""
return wol.widget { render = function()
  local long = string.rep("x", 5000)
  for i = 1, 10000 do
    local t = i == 1 and long or "t"
    local ok, m = wol.notify(t, t)
    if not ok then refused = refused + 1; why = m end
  end
  return wol.label { refused .. "|" .. why }
end }
"#;
    let mut w = load_with(
        Kind::Widget,
        src,
        &[Permission::Notifications],
        fake.clone(),
    )
    .unwrap();
    let text = match w.render().unwrap() {
        Node::Label { text, .. } => text,
        other => panic!("{other:?}"),
    };
    let posted = fake.posted.borrow();
    assert_eq!(posted.len(), wol_lua::NOTIFY_PER_WINDOW);
    assert_eq!(
        text,
        format!(
            "{}|{}",
            10000 - wol_lua::NOTIFY_PER_WINDOW,
            wol_lua::TOO_MANY_NOTIFICATIONS
        )
    );
    assert_eq!(posted[0].0.chars().count(), wol_lua::NOTIFY_TITLE);
    assert_eq!(posted[0].1.chars().count(), wol_lua::NOTIFY_BODY);
}
