//! A C program on libwol (ADR 0020): `tests/c/hello.c` is compiled with `cc` against `include/wol.h`,
//! linked against the built `libwol.so.1` (by soname), and run against a fake wold and a fake Kit
//! host speaking the real `shared` wires. Proves the C ABI end to end: strings, errors, system
//! info, a permission-checked service call, subscriptions, a window with a button and its callback.
use shared::events::Event;
use shared::portal::NoteAction;
use shared::portal::{Answer, Call, Code};
use shared::view::{FromHost, Node, Role, ToHost};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

mod common;

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-abi-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Fake wold: first call allowed, the rest denied; `subscribe` gets one theme event.
fn fake_wold(path: &Path, calls: Arc<Mutex<Vec<Call>>>) {
    let l = UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        for conn in l.incoming().flatten() {
            let calls = calls.clone();
            std::thread::spawn(move || {
                let mut line = String::new();
                BufReader::new(&conn).read_line(&mut line).unwrap();
                let mut w = &conn;
                if line.trim() == "subscribe" {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    let e = Event::Theme {
                        id: "wol-dark-blue".into(),
                    };
                    w.write_all(e.to_line().as_bytes()).unwrap();
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    return;
                }
                let json = line.trim().strip_prefix("call ").unwrap();
                let call = Call::decode(json.as_bytes()).unwrap();
                let mut c = calls.lock().unwrap();
                c.push(call);
                let a = if c.len() == 1 {
                    let mut a = Answer::ok();
                    a.id = Some(1);
                    a
                } else {
                    Answer::err(Code::PermissionDenied, "Notifications are off for hello.")
                };
                w.write_all(a.to_line().as_bytes()).unwrap();
            });
        }
    });
}

/// Fake Kit host: records the window, clicks the first button it shows.
fn fake_kit(path: &Path, shown: Arc<Mutex<Vec<ToHost>>>) {
    let l = UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        let conn = l.incoming().next().unwrap().unwrap();
        let mut r = BufReader::new(&conn);
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        assert_eq!(
            ToHost::parse(&line),
            Some(ToHost::Hello {
                app: "org.example.hello".into()
            }),
            "the app says who it is first"
        );
        line.clear();
        r.read_line(&mut line).unwrap();
        let m = ToHost::parse(&line).expect("valid view message");
        let ToHost::Show {
            window,
            content: Node::Stack { children, .. },
            ..
        } = &m
        else {
            panic!("{line}")
        };
        let Some(Node::Button { id, .. }) = children.get(1) else {
            panic!("{line}")
        };
        let click = FromHost::Clicked {
            window: *window,
            widget: *id,
        };
        shown.lock().unwrap().push(m.clone());
        let mut w = &conn;
        w.write_all(click.to_line().as_bytes()).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(5));
    });
}

/// Compile `tests/c/<name>.c` against `wol.h` and the built `libwol.so.1`.
fn build(name: &str, dir: &Path) -> PathBuf {
    let lib = common::lib_dir();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exe = dir.join(name);
    let cc = std::process::Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()))
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-pedantic"])
        .arg("-I")
        .arg(manifest.join("include"))
        .arg(manifest.join(format!("tests/c/{name}.c")))
        .arg(format!("-L{}", lib.display()))
        .arg("-lwol")
        .arg(format!("-Wl,-rpath,{}", lib.display()))
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("cc");
    assert!(
        cc.status.success(),
        "cc: {}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let needed = std::process::Command::new("readelf")
        .env("LC_ALL", "C")
        .args(["-d"])
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&needed.stdout).contains("[libwol.so.1]"),
        "apps record the soname"
    );
    exe
}

#[test]
fn hello_from_c() {
    let dir = temp("hello");
    let exe = build("hello", &dir);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let shown = Arc::new(Mutex::new(Vec::new()));
    fake_wold(&dir.join("wold.sock"), calls.clone());
    fake_kit(&dir.join("kit.sock"), shown.clone());
    let config = dir.join("config");
    std::fs::create_dir_all(config.join("wolos")).unwrap();
    std::fs::write(config.join("wolos/theme"), "wol-dark-blue\n").unwrap();

    let out = std::process::Command::new(&exe)
        .env("WOL_SOCKET", dir.join("wold.sock"))
        .env("WOL_KIT_SOCKET", dir.join("kit.sock"))
        .env("XDG_CONFIG_HOME", &config)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines: Vec<&str> = stdout.lines().collect();
    let os = lines.iter().find(|l| l.starts_with("os: ")).unwrap();
    assert!(os.len() > 4, "{os}");
    let want = [
        "version: 1.1.0 65792",
        "appearance: dark",
        "accent: blue",
        "accent-rgb: 0a84ff",
        "posted",
        "denied: Notifications are off for hello.",
        "invalid argument",
        "button has no children",
        "fd: yes",
        "clicked",
        "event: theme",
        r#"json: {"event":"theme","id":"wol-dark-blue"}"#,
        "clicks: 1 themes: 1",
    ];
    let got: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.starts_with("os: "))
        .collect();
    let mut sorted_got = got.clone();
    let mut sorted_want = want.to_vec();
    // Click and theme event order depends on which socket answers first.
    sorted_got.sort_unstable();
    sorted_want.sort_unstable();
    assert_eq!(sorted_got, sorted_want, "{stdout}");
    assert_eq!(got.last(), Some(&"clicks: 1 themes: 1"));

    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            Call::NotificationPost {
                title: "Hello".into(),
                body: "from C".into(),
                actions: Vec::new(),
            },
            Call::NotificationPost {
                title: "Again".into(),
                body: String::new(),
                actions: Vec::new(),
            },
        ],
        "the invalid call never left the library"
    );
    let shown = shown.lock().unwrap();
    let ToHost::Show { title, content, .. } = &shown[0] else {
        panic!()
    };
    assert_eq!(title, "Hello");
    let Node::Stack { spacing, children } = content else {
        panic!()
    };
    assert_eq!(*spacing, 8);
    assert_eq!(
        children[0],
        Node::Label {
            text: "Hello, WolOS".into(),
            role: Role::Body
        }
    );
    assert!(matches!(&children[1], Node::Button { label, .. } if label == "Click Me"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Fake wold for parts.c: a post gets id 7; the subscription then reports button "open" pressed.
fn fake_wold_actions(path: &Path, calls: Arc<Mutex<Vec<Call>>>) {
    let l = UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        for conn in l.incoming().flatten() {
            let calls = calls.clone();
            std::thread::spawn(move || {
                let mut line = String::new();
                BufReader::new(&conn).read_line(&mut line).unwrap();
                let mut w = &conn;
                if line.trim() == "subscribe" {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    for id in [6, 7] {
                        let e = Event::NotificationAction {
                            uid: 1000,
                            pid: 1,
                            id,
                            key: "open".into(),
                        };
                        w.write_all(e.to_line().as_bytes()).unwrap();
                    }
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    return;
                }
                let json = line.trim().strip_prefix("call ").unwrap();
                calls
                    .lock()
                    .unwrap()
                    .push(Call::decode(json.as_bytes()).unwrap());
                let mut a = Answer::ok();
                a.id = Some(7);
                w.write_all(a.to_line().as_bytes()).unwrap();
            });
        }
    });
}

/// Fake Kit host for parts.c: records two shows, then edits, toggles, slides and closes.
fn fake_kit_parts(path: &Path, shown: Arc<Mutex<Vec<ToHost>>>) {
    let l = UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        let conn = l.incoming().next().unwrap().unwrap();
        let mut r = BufReader::new(&conn);
        let mut msgs = Vec::new();
        for _ in 0..3 {
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            msgs.push(ToHost::parse(&line).unwrap_or_else(|| panic!("{line}")));
        }
        let ToHost::Show {
            window,
            content: Node::List { children },
            ..
        } = &msgs[2]
        else {
            panic!("{msgs:?}")
        };
        let id = |i: usize| match &children[i] {
            Node::TextField { id, .. } | Node::Toggle { id, .. } | Node::Slider { id, .. } => *id,
            n => panic!("{n:?}"),
        };
        let window = *window;
        let events = [
            FromHost::Edited {
                window,
                widget: id(1),
                text: "Ada".into(),
            },
            FromHost::Toggled {
                window,
                widget: id(2),
                on: false,
            },
            FromHost::Slid {
                window,
                widget: id(3),
                value: 75.0,
            },
            FromHost::Closed { window },
        ];
        shown.lock().unwrap().extend(msgs);
        let mut w = &conn;
        for e in events {
            w.write_all(e.to_line().as_bytes()).unwrap();
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    });
}

#[test]
fn one_one_parts_from_c() {
    let dir = temp("parts");
    let exe = build("parts", &dir);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let shown = Arc::new(Mutex::new(Vec::new()));
    fake_wold_actions(&dir.join("wold.sock"), calls.clone());
    fake_kit_parts(&dir.join("kit.sock"), shown.clone());
    let out = std::process::Command::new(&exe)
        .env("WOL_SOCKET", dir.join("wold.sock"))
        .env("WOL_KIT_SOCKET", dir.join("kit.sock"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut got: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        got.last(),
        Some(&"changes: 3 closed: 1 pressed: 1"),
        "{stdout}"
    );
    got.sort_unstable();
    let mut want = vec![
        "bad image refused",
        "out of range refused",
        "name: Ada",
        "wifi: 0",
        "volume: 75.0",
        "closed",
        "action: open",
        "changes: 3 closed: 1 pressed: 1",
    ];
    want.sort_unstable();
    assert_eq!(got, want, "only the posted note's button runs the handler");

    assert_eq!(
        *calls.lock().unwrap(),
        vec![Call::NotificationPost {
            title: "Download done".into(),
            body: "parts.iso".into(),
            actions: vec![NoteAction {
                key: "open".into(),
                label: "Open".into()
            }],
        }]
    );
    let shown = shown.lock().unwrap();
    assert_eq!(
        shown[0],
        ToHost::Hello {
            app: "org.example.parts".into()
        }
    );
    let label = |m: &ToHost| match m {
        ToHost::Show {
            content: Node::List { children },
            ..
        } => match &children[0] {
            Node::Row { children, .. } => children.clone(),
            n => panic!("{n:?}"),
        },
        m => panic!("{m:?}"),
    };
    let first = label(&shown[1]);
    assert_eq!(
        first[0],
        Node::Image {
            source: "weather-clear-symbolic".into(),
            size: 24
        }
    );
    assert!(matches!(&first[1], Node::Label { text, .. } if text == "Sunny"));
    assert_eq!(first[2], Node::Spacer);
    assert!(
        matches!(&label(&shown[2])[1], Node::Label { text, .. } if text == "Cloudy"),
        "set_text redraws the shown window"
    );
    let _ = std::fs::remove_dir_all(dir);
}
