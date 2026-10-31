//! A C program on libwol (ADR 0020): `tests/c/hello.c` is compiled with `cc` against `include/wol.h`,
//! linked against the built `libwol.so.1` (by soname), and run against a fake wold and a fake Kit
//! host speaking the real `shared` wires. Proves the C ABI end to end: strings, errors, system
//! info, a permission-checked service call, subscriptions, a window with a button and its callback.
use shared::portal::Call;
use shared::view::{Node, Role, ToHost};
use std::sync::{Arc, Mutex};

mod common;
use common::fakes::{fake_kit, fake_wold};

#[test]
fn hello_from_c() {
    let dir = common::temp("hello");
    let exe = common::build("hello", &dir);
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
    let (a, b, c) = wol::VERSION;
    let version = format!("version: {a}.{b}.{c} {}", (a << 16) | (b << 8) | c);
    let want = [
        version.as_str(),
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

#[test]
fn one_one_parts_from_c() {
    let dir = common::temp("parts");
    let exe = common::build("parts", &dir);
    common::fakes::run_parts(std::process::Command::new(&exe), &dir);
    let _ = std::fs::remove_dir_all(dir);
}
