// SPDX-License-Identifier: GPL-2.0-or-later
//! Lua widgets, headless: a script is loaded, rendered into a Kit view tree, clicked, reloaded.
use shared::view::{Node, Role};
use std::cell::RefCell;
use std::rc::Rc;
use wol_lua::{Change, Library, Services, Stamp, Widget};

#[derive(Default)]
struct Fake {
    posted: RefCell<Vec<(String, String)>>,
}

impl Services for Fake {
    fn now(&self) -> Stamp {
        // Monday 26 October 2026, 14:05:09.
        Stamp {
            year: 2026,
            month: 10,
            day: 26,
            weekday: 1,
            hour: 14,
            minute: 5,
            second: 9,
        }
    }
    fn system(&self) -> wol::system::Info {
        wol::system::Info {
            os_name: "WolOS".into(),
            os_version: "0.11".into(),
            kernel: "WolKernel".into(),
            appearance: wol::system::Appearance::Dark,
            accent_name: "blue".into(),
            accent_rgb: 0x6aaef2,
        }
    }
    fn notify(&self, title: &str, body: &str) -> Result<(), String> {
        self.posted.borrow_mut().push((title.into(), body.into()));
        Ok(())
    }
}

fn load(src: &str) -> Result<Widget, String> {
    Widget::load("test", src, Rc::new(Fake::default()))
}

fn label(text: &str, role: Role) -> Node {
    Node::Label {
        text: text.into(),
        role,
    }
}

#[test]
fn shipped_clock_renders_the_time_and_date() {
    let src = include_str!("../widgets/clock.lua");
    let mut w = load(src).unwrap();
    assert_eq!((w.name(), w.refresh()), ("Clock", 60));
    assert_eq!(
        w.render().unwrap(),
        Node::Stack {
            spacing: 4,
            children: vec![
                label("14:05", Role::Display),
                label("Monday, 26 October", Role::Secondary),
            ],
        }
    );
}

const COUNTER: &str = r#"
local n = 0
return wol.widget {
  name = "Counter",
  render = function()
    return wol.row { spacing = 8,
      wol.label { "Count: " .. n },
      wol.spacer(),
      wol.button { label = "+1", on_click = function() n = n + 1 end },
    }
  end,
}
"#;

#[test]
fn clicks_run_the_handler_and_the_next_render_shows_it() {
    let mut w = load(COUNTER).unwrap();
    assert_eq!(w.refresh(), wol_lua::DEFAULT_REFRESH);
    let Node::Row { children, .. } = w.render().unwrap() else {
        panic!()
    };
    let Node::Button { id, label: l } = &children[2] else {
        panic!("{children:?}")
    };
    assert_eq!(l, "+1");
    w.click(*id).unwrap();
    w.click(*id).unwrap();
    let Node::Row { children, .. } = w.render().unwrap() else {
        panic!()
    };
    assert_eq!(children[0], label("Count: 2", Role::Body));
    assert!(w.click(999).is_err(), "no such button");
}

#[test]
fn scripts_get_no_files_processes_or_loaders() {
    for global in [
        "io",
        "os",
        "require",
        "package",
        "debug",
        "load",
        "loadfile",
        "dofile",
        "collectgarbage",
    ] {
        let src = format!(
            "return wol.widget {{ name = 'x', render = function() return wol.label {{ tostring({global}) }} end }}"
        );
        let mut w = load(&src).unwrap();
        assert_eq!(w.render().unwrap(), label("nil", Role::Body), "{global}");
    }
    let mut w = load("return wol.widget { name = 'x', render = function() return wol.label { tostring(string.dump) } end }").unwrap();
    assert_eq!(w.render().unwrap(), label("nil", Role::Body), "no bytecode");
}

#[test]
fn services_need_a_declared_permission() {
    let src = |perms: &str| {
        format!(
            "return wol.widget {{ name = 'n', permissions = {{ {perms} }}, render = function() \
             return wol.label {{ tostring(wol.notify) }} end }}"
        )
    };
    let mut w = load(&src("")).unwrap();
    assert_eq!(w.render().unwrap(), label("nil", Role::Body));
    let fake = Rc::new(Fake::default());
    let posting = "return wol.widget { name = 'n', permissions = { 'notifications' }, render = function() \
                   wol.notify('Hi', 'there'); return wol.label { wol.system().accent } end }";
    let mut w = Widget::load("n", posting, fake.clone()).unwrap();
    assert_eq!(w.render().unwrap(), label("blue", Role::Body));
    assert_eq!(
        *fake.posted.borrow(),
        vec![("Hi".to_string(), "there".to_string())]
    );
    let e = load(&src("'camera'")).err().unwrap();
    assert!(e.contains("camera"), "{e}");
}

/// The instruction hook does not run inside C string functions (audit A21).
#[test]
fn string_bombs_are_stopped_quickly() {
    let start = std::time::Instant::now();
    let run = |body: &str| {
        let src = format!("return wol.widget {{ name = 'x', render = function() {body} end }}");
        load(&src).and_then(|mut w| w.render().map(|_| ()))
    };
    let e = run("local s = string.rep('x', 2^30)").unwrap_err();
    assert!(e.contains("too large"), "{e}");
    let e = run("local s = ('x'):rep(2^30)").unwrap_err();
    assert!(e.contains("too large"), "{e}");
    // Backtracking bomb: many '-' items over a long subject.
    let e = run(
        "local s = string.rep('a', 100000) local p = string.rep('a-', 12) .. 'b' string.find(s, p)",
    )
    .unwrap_err();
    assert!(e.contains("too costly"), "{e}");
    let e = run("local s = string.rep('a', 2^20) s = s .. s string.gsub(s, 'a', 'b')").unwrap_err();
    assert!(!e.is_empty());
    // Ordinary use still works.
    let ok = run(
        "local s = string.rep('ab', 10, ',') assert(#s == 29) assert(s:gsub('%s+', '') == s)         assert(string.find(s, 'b,a', 1, true) == 2) local n = 0 for w in s:gmatch('%a+') do n = n + 1 end assert(n == 10)         return wol.label('ok')",
    );
    assert!(ok.is_ok(), "{ok:?}");
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}

/// Audit S9 and L3: `?` is a quantifier; sets and a quantifier that ends the pattern are not.
#[test]
fn optional_items_count_and_sets_do_not() {
    let start = std::time::Instant::now();
    let run = |body: &str| {
        let src = format!("return wol.widget {{ name = 'x', render = function() {body} end }}");
        load(&src).and_then(|mut w| w.render().map(|_| ()))
    };
    let e = run("string.find(string.rep('a', 64), string.rep('a?', 40) .. string.rep('a', 40))")
        .unwrap_err();
    assert!(e.contains("too costly"), "{e}");
    let e = run("string.match(string.rep('a', 64), string.rep('%a?', 40) .. 'b')").unwrap_err();
    assert!(e.contains("too costly"), "{e}");
    let e = run("string.find(string.rep('(', 5000), '%b()%b()%b()x')").unwrap_err();
    assert!(e.contains("too costly"), "{e}");
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
    let ok = run(
        "assert(('a'):rep(1100):match('[a-z]+') == ('a'):rep(1100))          assert(('x'):rep(2000):gsub('[%w%-]+', '') == '')          assert(('x'):rep(40000):match('^(x+)$'))          assert(('k=v'):rep(5000):match('(%w+)=(%w*)') == 'k')          assert(string.find('a-b', '[*+?-]') == 2)          return wol.label('ok')",
    );
    assert!(ok.is_ok(), "{ok:?}");
}

/// Audit L2: C loops over a range or a table the hook never sees.
#[test]
fn table_bombs_are_stopped_quickly() {
    let start = std::time::Instant::now();
    let run = |body: &str| {
        let src = format!("return wol.widget {{ name = 'x', render = function() {body} end }}");
        load(&src).and_then(|mut w| w.render().map(|_| ()))
    };
    let e = run("table.move({}, 1, 2^31 - 2, 2)").unwrap_err();
    assert!(e.contains("too large"), "{e}");
    let e = run("table.move({}, '1', '2147483646', 2)").unwrap_err();
    assert!(e.contains("too large"), "{e}");
    let e = run("table.concat({}, '', 1, 2^31)").unwrap_err();
    assert!(e.contains("too large"), "{e}");
    let e = run("local t = {} for i = 1, 70000 do t[i] = i end table.sort(t)").unwrap_err();
    assert!(e.contains("too large"), "{e}");
    // A big sort under the cap, in a loop, ends on the instruction budget.
    let e = run("local t = {} for i = 1, 60000 do t[i] = -i end while true do table.sort(t) end")
        .unwrap_err();
    assert!(e.contains("too long"), "{e}");
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    let ok = run(
        "local t = { 3, 1, 2 } table.sort(t) assert(t[1] == 1 and t[3] == 3)          table.sort(t, function(a, b) return a > b end) assert(t[1] == 3)          assert(table.concat(table.move({ 'a', 'b' }, 1, 2, 2, { 'z' }), ',') == 'z,a,b')          return wol.label('ok')",
    );
    assert!(ok.is_ok(), "{ok:?}");
}

/// Audit L1: `pcall` and `xpcall` cannot swallow a spent budget or a full heap.
#[test]
fn pcall_cannot_outlive_the_budget() {
    let start = std::time::Instant::now();
    let run = |body: &str| {
        let src = format!("return wol.widget {{ name = 'x', render = function() {body} end }}");
        load(&src).and_then(|mut w| w.render().map(|_| ()))
    };
    let e = run("while true do pcall(function() while true do end end) end").unwrap_err();
    assert!(e.contains("too long"), "{e}");
    let e =
        run("while true do xpcall(function() while true do end end, function(m) return m end) end")
            .unwrap_err();
    assert!(e.contains("too long"), "{e}");
    let e = run(
        "while true do pcall(function() local t = {} while true do t[#t + 1] = ('x'):rep(1000) end end) end",
    )
    .unwrap_err();
    assert!(e.contains("was stopped"), "{e}");
    assert!(
        load("while true do pcall(function() while true do end end) end")
            .unwrap_err()
            .contains("too long")
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    // Ordinary errors still come back to the script, and the widget stays usable.
    let ok = run(
        "local ok, m = pcall(error, 'boom') assert(not ok and m == 'boom')          local ok2, m2 = xpcall(function() error('x', 0) end, function(e) return e .. '!' end)          assert(not ok2 and m2 == 'x!') return wol.label('ok')",
    );
    assert!(ok.is_ok(), "{ok:?}");
}

#[test]
fn runaway_and_greedy_scripts_are_stopped() {
    let start = std::time::Instant::now();
    let mut w = load("return wol.widget { name = 'x', render = function() while true do end end }")
        .unwrap();
    assert!(w.render().unwrap_err().contains("too long"));
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    let mut w = load(
        "return wol.widget { name = 'x', render = function() local t = {} for i = 1, 1e9 do t[i] = string.rep('x', 1000) end end }",
    )
    .unwrap();
    assert!(w.render().is_err());
    assert!(load("while true do end").unwrap_err().contains("too long"));
}

#[test]
fn bad_scripts_and_trees_say_what_is_wrong() {
    let cases = [
        ("return 1", "wol.widget"),
        ("return wol.widget { render = function() end }", "name"),
        ("return wol.widget { name = 'x' }", "render"),
        (
            "return wol.widget { name = 'x', refresh = -1, render = function() end }",
            "refresh",
        ),
        ("syntax error here", "syntax"),
    ];
    for (src, want) in cases {
        let e = load(src).err().unwrap_or_default();
        assert!(e.contains(want), "{src}: {e}");
    }
    let renders = [
        ("return 5", "part"),
        ("return wol.stack { spacing = 5 }", "Spacing"),
        ("return { type = 'iframe' }", "iframe"),
        ("return wol.label { role = 'huge', 'x' }", "huge"),
        ("return wol.button { label = 'x' }", "on_click"),
    ];
    for (body, want) in renders {
        let src = format!("return wol.widget {{ name = 'x', render = function() {body} end }}");
        let e = load(&src).unwrap().render().unwrap_err();
        assert!(e.contains(want), "{body}: {e}");
    }
}

#[test]
fn library_hot_reloads_a_folder() {
    let dir = std::env::temp_dir().join(format!("wol-lua-lib-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut lib = Library::new(dir.clone(), Rc::new(Fake::default()));
    assert!(lib.poll().is_empty());
    let file = dir.join("count.lua");
    std::fs::write(&file, COUNTER).unwrap();
    std::fs::write(dir.join("Not An Id.lua"), COUNTER).unwrap();
    std::fs::write(dir.join("notes.txt"), "x").unwrap();
    assert_eq!(lib.poll(), vec![Change::Loaded("count".into())]);
    assert!(lib.poll().is_empty(), "unchanged files are not reloaded");
    let old = lib.widget("count").unwrap().render().unwrap();

    let bump = |text: &str, secs: u64| {
        std::fs::write(&file, text).unwrap();
        let t = std::time::SystemTime::now() + std::time::Duration::from_secs(secs);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(t)
            .unwrap();
    };
    bump(&COUNTER.replace("Count: ", "Total: "), 10);
    assert_eq!(lib.poll(), vec![Change::Loaded("count".into())]);
    let new = lib.widget("count").unwrap().render().unwrap();
    assert_ne!(old, new);

    bump("return nope(", 20);
    let changes = lib.poll();
    assert!(
        matches!(&changes[..], [Change::Failed(id, _)] if id == "count"),
        "{changes:?}"
    );
    assert!(lib.widget("count").is_none());

    std::fs::remove_file(&file).unwrap();
    assert_eq!(lib.poll(), vec![Change::Removed("count".into())]);
    let _ = std::fs::remove_dir_all(dir);
}

/// R2C-2: the cost of `string.rep` (and of the C loops over `#t`) is the count, so a count of
/// 2^40 over an empty piece, or an empty table whose `__len` says 2^40, is refused at once.
#[test]
fn zero_size_pieces_with_a_huge_count_are_refused() {
    let run = |body: &str| {
        let src = format!("return wol.widget {{ name = 'x', render = function() {body} end }}");
        load(&src).and_then(|mut w| w.render().map(|_| ()))
    };
    let huge = "setmetatable({}, { __len = function() return 2^40 end })";
    for (body, want) in [
        ("string.rep('', 2^40)", "count too large"),
        ("('').rep('', 2^40)", "count too large"),
        ("string.rep('', '1099511627776')", "count too large"),
        ("string.rep('', 2^40, '')", "count too large"),
        (&format!("table.concat({huge}, '')"), "range too large"),
        (&format!("table.insert({huge}, 1, 'x')"), "too large"),
        (&format!("table.remove({huge}, 1)"), "too large"),
        (&format!("table.sort({huge})"), "too large"),
    ] {
        let start = std::time::Instant::now();
        let e = run(body).unwrap_err();
        assert!(e.contains(want), "{body}: {e}");
        assert!(
            start.elapsed() < std::time::Duration::from_millis(100),
            "{body}: {:?}",
            start.elapsed()
        );
    }
    let ok = run(
        "assert(string.rep('', 3) == '' and ('ab'):rep(2) == 'abab' and string.rep('x', '3') == 'xxx')          local t = { 'a', 'c' } table.insert(t, 2, 'b') assert(table.remove(t, 1) == 'a')          assert(table.concat(t) == 'bc') return wol.label('ok')",
    );
    assert!(ok.is_ok(), "{ok:?}");
}

/// R2C-3: a loop of capped C calls ends at the wall-clock deadline (well before the instruction
/// budget), in render and in a click, even through pcall; the widget then stays stopped.
#[test]
fn loops_of_capped_c_calls_stop_at_the_deadline() {
    let d = wol_lua::Deadlines::DEFAULT;
    let bombs = [
        "local t = {} while true do table.move(t, 1, 2^20 - 1, 2) end",
        "while true do pcall(table.move, {}, 1, 2^20 - 1, 2) end",
        "while true do pcall(string.rep, '', 2^40) local s = string.rep('', 2^20) end",
        "local s = ('x'):rep(2^20) while true do s:upper() s:reverse() end",
        "local s = ('ab'):rep(2^19) while true do local n = select('#', s:byte(1, 100000)) end",
        "local t = {} for i = 1, 60000 do t[i] = -i end while true do table.unpack(t) end",
        "while true do math.floor(1.5) string.len('x') end",
    ];
    for body in bombs {
        let src = format!(
            "return wol.widget {{ name = 'x', render = function() {body} end,              refresh = 0 }}"
        );
        let mut w = load(&src).unwrap();
        let start = std::time::Instant::now();
        let e = w.render().unwrap_err();
        let took = start.elapsed();
        assert_eq!(e, wol_lua::TOO_LONG, "{body}");
        assert!(took < 2 * d.render, "{body}: {took:?}");
        assert!(w.stopped(), "{body}");
        // The next render does not run the script again.
        let again = std::time::Instant::now();
        assert_eq!(w.render().unwrap_err(), wol_lua::TOO_LONG);
        assert!(again.elapsed() < std::time::Duration::from_millis(5));
    }
    // A click has the event deadline, which the host may change.
    let src = "return wol.widget { name = 'x', render = function() return wol.button { 'go',      on_click = function() local t = {} while true do table.move(t, 1, 2^20 - 1, 2) end end } end }";
    let mut w = load(src).unwrap();
    let fast = wol_lua::Deadlines {
        event: std::time::Duration::from_millis(80),
        ..d
    };
    w.set_deadlines(fast);
    w.render().unwrap();
    let start = std::time::Instant::now();
    assert_eq!(w.click(1).unwrap_err(), wol_lua::TOO_LONG);
    let took = start.elapsed();
    assert!(took >= fast.event && took < 2 * fast.event, "{took:?}");
    assert_eq!(w.render().unwrap_err(), wol_lua::TOO_LONG);
    // Loading has its own deadline.
    let start = std::time::Instant::now();
    let e = load("local t = {} while true do table.move(t, 1, 2^20 - 1, 2) end").unwrap_err();
    assert_eq!(e, wol_lua::TOO_LONG);
    assert!(start.elapsed() < 2 * d.load, "{:?}", start.elapsed());
}

/// The deadline is not the instruction budget's sentinel text: a script's own error that says
/// "budget" is shown as written, and a widget that errors for another reason keeps running.
#[test]
fn ordinary_errors_are_not_stops() {
    let mut w = load(
        "local n = 0 return wol.widget { name = 'x', render = function() n = n + 1          if n == 1 then error('over budget', 0) end return wol.label('ok') end }",
    )
    .unwrap();
    let e = w.render().unwrap_err();
    assert!(e.contains("over budget") && e != wol_lua::TOO_LONG, "{e}");
    assert!(!w.stopped());
    assert!(w.render().is_ok());
}
