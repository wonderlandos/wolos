//! A C program on libwol (ADR 0020): `tests/c/hello.c` is compiled with `cc` against `include/wol.h`,
//! linked against the built `libwol.so.1` (by soname), and run against a fake wold and a fake Kit
//! host speaking the real `shared` wires. Proves the C ABI end to end: strings, errors, system
//! info, a permission-checked service call, subscriptions, a window with a button and its callback.
use shared::events::Event;
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
                    Answer::ok()
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

#[test]
fn hello_from_c() {
    let dir = temp("hello");
    let lib = common::lib_dir();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exe = dir.join("hello");
    let cc = std::process::Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()))
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-pedantic"])
        .arg("-I")
        .arg(manifest.join("include"))
        .arg(manifest.join("tests/c/hello.c"))
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
        "version: 1.0.0 65536",
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
                body: "from C".into()
            },
            Call::NotificationPost {
                title: "Again".into(),
                body: String::new()
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
