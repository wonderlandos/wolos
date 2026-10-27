//! `wol games`: doctor, steam-setup into a temp HOME, and the one-click Steam launch.
use controller::cli::parse;
use controller::games::{LSU_WHEEL_FLAG, STEAM_ICON, lsu_bootstrap, lsu_steam};
use controller::run::{FakeEnv, execute};
use shared::errors::ErrorCode;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wol_core::games::notice::{self, Notice};
use wol_core::games::steam;
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

const MOUNTS: &str = "\
linprocfs /compat/linux/proc linprocfs rw 0 0
linsysfs /compat/linux/sys linsysfs rw 0 0
devfs /compat/linux/dev devfs rw 0 0
tmpfs /compat/linux/dev/shm tmpfs rw,mode=1777 0 0
fdescfs /compat/linux/dev/fd fdescfs rw,linrdlnk 0 0
";

/// FreeBSD with everything Steam and Windows games need; `groups` from `id -Gn`.
fn ready_host(groups: &str) -> FakeHost {
    let h = FakeHost::new(true);
    for f in [
        "/etc/machine-id",
        "/dev/dri/renderD128",
        "/usr/local/steam-utils/bin/steam",
        "/usr/local/wine-proton/bin/wine",
        "/usr/local/wine-proton/bin/wine64",
        "/usr/local/share/vulkan/icd.d/intel_icd.x86_64.json",
    ] {
        h.file(f, "");
    }
    h.file(
        "/compat/linux/etc/os-release",
        "ID=\"rocky\"\nVERSION_ID=\"9.8\"\n",
    );
    h.command("id -Gn", groups)
        .command("mount -p", MOUNTS)
        .command("kldstat -q -m nullfs", "")
        .command("/usr/local/wine-proton/bin/wine64 --version", "wine-11.0\n");
    for s in [
        "kern.features.linux",
        "kern.features.linux64",
        "security.bsd.unprivileged_chroot",
        "vfs.usermount",
        "compat.linux.use_real_ifnames",
    ] {
        h.int(s, 1);
    }
    h
}

fn bootstrapped(h: &FakeHost, home: &Path) {
    h.file(
        &home
            .join(".steam/steam/ubuntu12_32/steam")
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

fn tool(home: &Path) -> PathBuf {
    home.join(".steam/steam/compatibilitytools.d/WolOS-Proton")
}

#[test]
fn steam_setup_is_idempotent_in_a_temp_home() {
    let home = temp_home("setup");
    let mut e = env(&home, FakeHost::new(true), true);
    assert_eq!(run("games steam-setup", &mut e), 0, "{}", e.err);
    assert!(e.out.starts_with("Installed WolOS Proton in "), "{}", e.out);
    assert!(e.out.contains("Properties > Compatibility"));
    let dir = tool(&home);
    for (name, text, mode) in steam::tool_files(steam::WOL_RUN) {
        let p = dir.join(name);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), text, "{name}");
        let got = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(got, mode, "{name}");
    }
    e.out.clear();
    assert_eq!(run("games steam-setup", &mut e), 0);
    assert!(e.out.starts_with("WolOS Proton is up to date"), "{}", e.out);
    // A damaged launcher is repaired.
    std::fs::write(dir.join(steam::LAUNCHER), "#!/bin/sh\nexit 1\n").unwrap();
    e.out.clear();
    assert_eq!(run("games steam-setup", &mut e), 0);
    assert!(e.out.starts_with("Updated WolOS Proton"), "{}", e.out);
    assert!(steam::is_current(
        &|p| std::fs::read_to_string(p).ok(),
        &home.join(".steam/steam"),
        steam::WOL_RUN
    ));
    let _ = std::fs::remove_dir_all(&home);
}

/// linux-steam-utils links ~/.steam/root to the client; Steam reads tools from there.
#[test]
fn steam_setup_follows_the_steam_root() {
    let home = temp_home("root");
    std::fs::create_dir_all(home.join(".local/share/Steam")).unwrap();
    let mut e = env(&home, FakeHost::new(true), true);
    assert_eq!(run("games steam-setup", &mut e), 0);
    assert!(
        home.join(".local/share/Steam/compatibilitytools.d/WolOS-Proton/wolos-proton")
            .exists()
    );
    let mut e = FakeEnv::default();
    assert_eq!(
        run("games steam-setup", &mut e),
        ErrorCode::Usage.exit(),
        "no HOME"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn doctor_reports_ready_and_each_fix() {
    let home = Path::new("/home/ana");
    let mut e = env(home, ready_host("ana wheel video\n"), false);
    assert_eq!(run("games doctor", &mut e), 0, "{}", e.out);
    let v: serde_json::Value = serde_json::from_str(&e.out).expect("JSON when not a TTY");
    assert_eq!(v["ready"], true);
    assert_eq!(v["steam_ready"], true);
    assert!(v["checks"].as_array().unwrap().len() > 15);

    let broken = ready_host("ana wheel video\n");
    broken.int("vfs.usermount", 0);
    let mut e = env(home, broken, true);
    assert_eq!(run("games doctor", &mut e), ErrorCode::Failed.exit());
    assert!(
        e.out
            .contains("FAIL  usermount            vfs.usermount=0\n"),
        "{}",
        e.out
    );
    assert!(
        e.out
            .contains("fix: sudo sysctl vfs.usermount=1 && echo vfs.usermount=1 | sudo tee -a /etc/sysctl.conf\n"),
        "{}",
        e.out
    );
    assert!(
        e.out
            .ends_with("Not ready: 1 problem(s). Run the fix lines above, then check again.\n")
    );
    assert!(!e.out.contains("fix: \n"), "checks that pass print no fix");
}

/// From the Dock or Launchpad (no terminal): a notification while the client downloads, then
/// Steam's own updater window.
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
            vec![lsu_bootstrap(), LSU_WHEEL_FLAG.to_string()],
            vec![lsu_steam(), "steam://install/3658110".to_string()],
        ]
    );
    assert!(e.out.is_empty(), "{}", e.out);
    assert!(tool(&home).join(steam::LAUNCHER).exists(), "tool installed");
    let _ = std::fs::remove_dir_all(&home);

    // Not in wheel: no flag. Already downloaded: straight to Steam.
    let home = temp_home("again");
    let mut e = env(&home, ready_host("ana video\n"), true);
    assert_eq!(run("games steam", &mut e), 0);
    assert_eq!(
        e.execs[0],
        [lsu_bootstrap()],
        "in a terminal: no notification"
    );
    assert!(e.out.starts_with("Getting Steam ready"), "{}", e.out);
    let h = ready_host("ana wheel video\n");
    bootstrapped(&h, &home);
    let mut e = env(&home, h, true);
    assert_eq!(run("games steam", &mut e), 0);
    assert_eq!(e.execs, [[lsu_steam()]]);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn steam_stops_on_what_the_client_needs() {
    let home = temp_home("blocked");
    let h = ready_host("ana wheel video\n");
    h.int("compat.linux.use_real_ifnames", 0);
    let mut e = env(&home, h, false);
    assert_eq!(run("games steam", &mut e), ErrorCode::Unavailable.exit());
    assert!(
        e.err
            .contains("fix: sudo sysctl compat.linux.use_real_ifnames=1"),
        "{}",
        e.err
    );
    assert_eq!(e.execs, [note(notice::SETTINGS)], "only the notification");
    assert!(!tool(&home).exists(), "nothing written when blocked");

    let mut e = env(&home, FakeHost::new(true), false);
    assert_eq!(run("games steam", &mut e), ErrorCode::Unavailable.exit());
    assert_eq!(
        e.execs,
        [note(notice::MISSING)],
        "a bare system: packages first"
    );

    // A failed download does not start Steam.
    let mut e = env(&home, ready_host("ana\n"), false);
    e.exec_codes = vec![0, 1];
    assert_eq!(run("games steam", &mut e), ErrorCode::Unavailable.exit());
    assert_eq!(
        e.execs,
        [
            note(notice::PREPARING),
            vec![lsu_bootstrap()],
            note(notice::NO_DOWNLOAD)
        ]
    );
    assert!(e.err.contains("lsu-bootstrap exited 1"), "{}", e.err);

    let mut e = env(&home, ready_host("wheel\n"), true);
    e.root = true;
    assert_eq!(run("games steam", &mut e), ErrorCode::Denied.exit());
    assert!(e.execs.is_empty());
    let _ = std::fs::remove_dir_all(&home);
}
