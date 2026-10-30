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
            accent_rgb: 0x0a84ff,
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
