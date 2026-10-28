//! `wol games`: doctor and the one-click Steam launch.
use controller::cli::parse;
use controller::games::STEAM_ICON;
use controller::run::{FakeEnv, execute};
use shared::errors::ErrorCode;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wol_core::games::notice::{self, Notice};
use wol_core::system::fake::FakeHost;

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

/// A fresh HOME per test (cleared first, so reruns start empty).
fn temp_home(name: &str) -> PathBuf {
    let h = std::env::temp_dir().join(format!("wol-games-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&h);
    std::fs::create_dir_all(&h).unwrap();
    h
}

const STEAM: &str = wol_core::games::doctor::STEAM_BIN;

/// Arch with everything Steam and Windows games need; `groups` from `id -Gn`.
fn ready_host(groups: &str) -> FakeHost {
    let h = FakeHost::new(false);
    for f in [
        STEAM,
        "/dev/dri/renderD128",
        "/usr/lib32/libvulkan.so.1",
        "/usr/lib32/libGLX_mesa.so.0",
        "/usr/bin/wine",
        "/usr/share/vulkan/icd.d/intel_icd.x86_64.json",
    ] {
        h.file(f, "");
    }
    h.file(
        "/etc/pacman.conf",
        "[multilib]\nInclude = /etc/pacman.d/mirrorlist\n",
    )
    .file("/proc/sys/kernel/osrelease", "6.17.3-wolos1-1\n");
    h.command("id -Gn", groups)
        .command("/usr/bin/wine --version", "wine-10.0\n");
    h
}

fn bootstrapped(h: &FakeHost, home: &Path) {
    h.file(
        &home
            .join(".local/share/Steam/ubuntu12_32/steam")
            .to_string_lossy(),
        "",
    );
}

fn env(home: &Path, host: FakeHost, tty: bool) -> FakeEnv {
    FakeEnv {
        home: home.into(),
        tty,
        vars: vec![("USER".into(), "ana".into())],
        host: Some(Arc::new(host)),
        ..Default::default()
    }
}

fn note((title, body): Notice) -> Vec<String> {
    let icon = format!("--icon={STEAM_ICON}");
    [
        "notify-send",
        "--app-name=Steam",
        icon.as_str(),
        "--",
        title,
        body,
    ]
    .map(String::from)
    .to_vec()
}

#[test]
fn doctor_reports_ready_and_each_fix() {
    let home = Path::new("/home/ana");
    let mut e = env(home, ready_host("ana wheel video\n"), false);
    assert_eq!(run("games doctor", &mut e), 0, "{}", e.out);
    let v: serde_json::Value = serde_json::from_str(&e.out).expect("JSON when not a TTY");
    assert_eq!(v["ready"], true);
    assert_eq!(v["steam_ready"], true);
    assert!(v["checks"].as_array().unwrap().len() >= 9);

    let broken = ready_host("ana wheel video\n");
    broken.file("/etc/pacman.conf", "#[multilib]\n");
    let mut e = env(home, broken, true);
    assert_eq!(run("games doctor", &mut e), ErrorCode::Failed.exit());
    assert!(e.out.contains("FAIL  multilib "), "{}", e.out);
    assert!(
        e.out
            .contains("fix: enable [multilib] in /etc/pacman.conf, then sudo pacman -Sy\n"),
        "{}",
        e.out
    );
    assert!(
        e.out
            .ends_with("Not ready: 1 problem(s). Run the fix lines above, then check again.\n")
    );
    assert!(!e.out.contains("fix: \n"), "checks that pass print no fix");
}

/// From the Dock or Launchpad (no terminal): a notification on the first start, then Steam's
/// own updater window.
#[test]
fn steam_first_start_downloads_the_client_then_starts_it() {
    let home = temp_home("first");
    let mut e = env(&home, ready_host("ana wheel video\n"), false);
    assert_eq!(
        run("games steam steam://install/3658110", &mut e),
        0,
        "{}",
        e.err
    );
    assert_eq!(
        e.execs,
        [
            note(notice::PREPARING),
            vec![STEAM.to_string(), "steam://install/3658110".to_string()],
        ]
    );
    assert!(e.out.is_empty(), "{}", e.out);
    let h = ready_host("ana wheel video\n");
    bootstrapped(&h, &home);
    let mut e = env(&home, h, true);
    assert_eq!(run("games steam", &mut e), 0);
    assert_eq!(e.execs, [[STEAM]]);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn steam_stops_on_what_the_client_needs() {
    let home = temp_home("blocked");
    let h = ready_host("ana wheel video\n");
    h.file("/etc/pacman.conf", "");
    let mut e = env(&home, h, false);
    assert_eq!(run("games steam", &mut e), ErrorCode::Unavailable.exit());
    assert!(e.err.contains("fix: enable [multilib]"), "{}", e.err);
    assert_eq!(e.execs, [note(notice::MISSING)], "only the notification");

    let mut e = env(&home, FakeHost::new(false), false);
    assert_eq!(run("games steam", &mut e), ErrorCode::Unavailable.exit());
    assert_eq!(
        e.execs,
        [note(notice::MISSING)],
        "a bare system: packages first"
    );

    let mut e = env(&home, ready_host("wheel\n"), true);
    e.root = true;
    assert_eq!(run("games steam", &mut e), ErrorCode::Denied.exit());
    assert!(e.execs.is_empty());
    let _ = std::fs::remove_dir_all(&home);
}
