//! `wol hw`: list/status/fix/boot against the recording fake.
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use shared::hardware::{Device, HardwareSnapshot, Kind, SNAPSHOT_PATH, State};
use std::path::PathBuf;

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

fn dev(kind: Kind, name: &str, state: State, klds: &[&str], pkgs: &[&str]) -> Device {
    let v = |x: &[&str]| x.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    Device {
        kind,
        name: name.into(),
        state,
        missing_klds: v(klds),
        missing_packages: v(pkgs),
        hint: if state == State::Ready {
            String::new()
        } else {
            format!("fix {name}")
        },
        ..Default::default()
    }
}

fn laptop() -> HardwareSnapshot {
    HardwareSnapshot {
        devices: vec![
            Device {
                driver: Some("intel_pstate".into()),
                klds: vec!["coretemp".into()],
                ..dev(
                    Kind::Cpu,
                    "Intel Core i5",
                    State::NeedsDriver,
                    &["coretemp"],
                    &[],
                )
            },
            Device {
                driver: Some("iwlwifi".into()),
                ..dev(Kind::Wifi, "Intel AX211", State::Ready, &[], &[])
            },
            dev(
                Kind::Bluetooth,
                "Intel Bluetooth",
                State::NeedsFirmware,
                &[],
                &["linux-firmware-intel"],
            ),
        ],
    }
}

fn fixed() -> HardwareSnapshot {
    HardwareSnapshot {
        devices: vec![dev(Kind::Wifi, "Intel AX211", State::Ready, &[], &[])],
    }
}

#[test]
fn list_is_a_table_on_a_tty_and_json_otherwise() {
    let mut env = FakeEnv {
        tty: true,
        hw: vec![laptop()],
        ..Default::default()
    };
    assert_eq!(run("hw", &mut env), 0);
    let lines: Vec<&str> = env.out.lines().collect();
    assert!(lines[0].starts_with("KIND") && lines[0].contains("STATE"));
    assert!(lines[1].starts_with("cpu          needs-driver    intel_pstate     Intel Core i5"));
    assert_eq!(lines[2].trim(), "fix Intel Core i5");
    assert!(
        env.out
            .contains("wifi         ready           iwlwifi          Intel AX211\n")
    );
    let mut env = FakeEnv {
        hw: vec![laptop()],
        ..Default::default()
    };
    assert_eq!(run("hw list --json", &mut env), 0);
    let back: HardwareSnapshot = serde_json::from_str(&env.out).unwrap();
    assert_eq!(back, laptop());
}

#[test]
fn status_lists_problems_and_the_fix() {
    let mut env = FakeEnv {
        tty: true,
        hw: vec![laptop()],
        ..Default::default()
    };
    assert_eq!(run("hw status", &mut env), 0);
    assert_eq!(
        env.out,
        "cpu Intel Core i5: fix Intel Core i5\nbluetooth Intel Bluetooth: fix Intel Bluetooth\n"
    );
    let mut env = FakeEnv {
        hw: vec![laptop()],
        ..Default::default()
    };
    run("hw status --json", &mut env);
    let v: serde_json::Value = serde_json::from_str(&env.out).unwrap();
    assert_eq!(v["ready"], false);
    assert_eq!(v["problems"].as_array().unwrap().len(), 2);
    assert_eq!(v["fix"]["load"], serde_json::json!(["coretemp"]));
    assert_eq!(
        v["fix"]["install"],
        serde_json::json!(["linux-firmware-intel"])
    );
    let mut env = FakeEnv {
        tty: true,
        hw: vec![fixed()],
        ..Default::default()
    };
    run("hw status", &mut env);
    assert_eq!(env.out, "all 1 devices ready\n");
}

#[test]
fn fix_loads_modules_through_admind_and_installs_packages_only_with_yes() {
    let sock = PathBuf::from("/run/admin.sock");
    let mut env = FakeEnv {
        hw: vec![laptop(), fixed()],
        vars: vec![("WOL_ADMIN_SOCKET".into(), sock.display().to_string())],
        replies: vec!["ok\n".into()],
        ..Default::default()
    };
    assert_eq!(run("hw fix", &mut env), 0);
    assert_eq!(
        env.requests,
        vec![(sock.clone(), b"hw-load\x001\x00coretemp\x00".to_vec())]
    );
    assert!(env.out.contains("loaded coretemp\n"));
    assert!(
        env.out.contains(
            "missing packages: linux-firmware-intel\ninstall them with: wol hw fix --yes\n"
        )
    );
    assert!(env.execs.is_empty(), "root actions only through wol-admind");
    // --yes: packages too. Password on stdin when admind asks, never argv.
    let mut env = FakeEnv {
        hw: vec![laptop(), fixed()],
        vars: vec![("WOL_ADMIN_SOCKET".into(), sock.display().to_string())],
        replies: vec!["auth\n".into(), "ok\n".into(), "ok\n".into()],
        stdin: vec!["s3cret-pw".into()],
        ..Default::default()
    };
    assert_eq!(run("hw fix --yes", &mut env), 0);
    let sent: Vec<String> = env
        .requests
        .iter()
        .map(|r| String::from_utf8_lossy(&r.1).into_owned())
        .collect();
    assert_eq!(
        sent,
        [
            "hw-load\x001\0coretemp\0",
            "hw-load\x001\0coretemp\0s3cret-pw",
            "pkg-install\x001\0linux-firmware-intel\0",
        ]
    );
    assert!(env.out.contains("installed linux-firmware-intel\n"));
    // Nothing to do.
    let mut env = FakeEnv {
        hw: vec![fixed()],
        ..Default::default()
    };
    assert_eq!(run("hw fix", &mut env), 0);
    assert_eq!(env.out, "nothing to fix\n");
}

#[test]
fn boot_writes_module_files_loads_modules_and_publishes_the_snapshot() {
    let mut env = FakeEnv {
        root: false,
        hw: vec![laptop()],
        ..Default::default()
    };
    assert_ne!(run("hw boot", &mut env), 0);
    assert!(env.written.is_empty() && env.execs.is_empty());
    let mut env = FakeEnv {
        root: true,
        hw: vec![laptop(), fixed()],
        exec_codes: vec![0],
        ..Default::default()
    };
    assert_eq!(run("hw boot", &mut env), 0);
    assert_eq!(env.execs, vec![vec!["modprobe", "coretemp"]]);
    let paths: Vec<String> = env
        .written
        .iter()
        .map(|w| w.0.display().to_string())
        .collect();
    assert_eq!(paths, ["/etc/modules-load.d/wolos-hw.conf", SNAPSHOT_PATH]);
    assert!(env.written[0].1.ends_with("\ncoretemp\n"));
    // The published snapshot is the one read after loading modules.
    let published: HardwareSnapshot = serde_json::from_str(&env.written[1].1).unwrap();
    assert_eq!(published, fixed());
    assert!(env.out.contains("wolos_hw: modprobe coretemp\n"));
    assert!(env.out.ends_with("wolos_hw: 1 devices, 0 need attention\n"));
    // A failing modprobe is reported, boot goes on.
    let mut env = FakeEnv {
        root: true,
        hw: vec![laptop()],
        exec_codes: vec![1],
        ..Default::default()
    };
    assert_eq!(run("hw boot", &mut env), 0);
    assert!(env.out.contains("wolos_hw: modprobe coretemp failed\n"));
    assert!(
        env.out
            .contains("wolos_hw: bluetooth Intel Bluetooth: fix Intel Bluetooth\n")
    );
}

#[test]
fn boot_writes_modprobe_options_before_loading_the_module() {
    // nvidia_drm reads modeset only at load: the options file must exist before modprobe runs.
    let gpu = HardwareSnapshot {
        devices: vec![Device {
            loader: vec!["options nvidia_drm modeset=1 fbdev=1".into()],
            ..dev(
                Kind::Gpu,
                "NVIDIA GeForce",
                State::NeedsDriver,
                &["nvidia_modeset", "nvidia_drm"],
                &[],
            )
        }],
    };
    let mut env = FakeEnv {
        root: true,
        hw: vec![gpu, fixed()],
        exec_codes: vec![0, 0],
        ..Default::default()
    };
    assert_eq!(run("hw boot", &mut env), 0);
    assert_eq!(
        env.execs,
        vec![
            vec!["modprobe", "nvidia_modeset"],
            vec!["modprobe", "nvidia_drm"],
        ]
    );
    assert_eq!(
        env.written[0],
        (
            PathBuf::from("/etc/modprobe.d/wolos-hw.conf"),
            format!(
                "{}options nvidia_drm modeset=1 fbdev=1\n",
                wol_core::hardware::HEADER
            )
        )
    );
    let wrote = env.out.find("wrote /etc/modprobe.d").unwrap();
    assert!(wrote < env.out.find("modprobe nvidia_modeset").unwrap());
}

#[test]
fn first_boot_installs_missing_driver_packages_online_then_loads_them() {
    // First boot (network up): the catalogue names the NVIDIA driver packages.
    let need = HardwareSnapshot {
        devices: vec![dev(
            Kind::Gpu,
            "NVIDIA GeForce",
            State::NeedsDriver,
            &[],
            &["nvidia-open"],
        )],
    };
    let installed = HardwareSnapshot {
        devices: vec![dev(
            Kind::Gpu,
            "NVIDIA GeForce",
            State::NeedsDriver,
            &["nvidia_drm"],
            &[],
        )],
    };
    let mut env = FakeEnv {
        root: true,
        hw: vec![need.clone(), installed, fixed()],
        exec_codes: vec![0, 0],
        ..Default::default()
    };
    assert_eq!(run("hw boot --online", &mut env), 0);
    assert_eq!(
        env.execs,
        vec![
            vec!["pacman", "-S", "--needed", "--noconfirm", "nvidia-open"],
            vec!["modprobe", "nvidia_drm"],
        ]
    );
    // Offline or a failing install: reported, boot goes on with what is there.
    let mut env = FakeEnv {
        root: true,
        hw: vec![need.clone(), need],
        exec_codes: vec![1],
        ..Default::default()
    };
    assert_eq!(run("hw boot --online", &mut env), 0);
    assert!(env.out.contains("wolos_hw: pacman -S nvidia-open failed\n"));
    // Without --online (every boot): never touches the network.
    let mut env = FakeEnv {
        root: true,
        hw: vec![HardwareSnapshot {
            devices: vec![dev(
                Kind::Gpu,
                "NVIDIA GeForce",
                State::NeedsDriver,
                &[],
                &["nvidia-open"],
            )],
        }],
        ..Default::default()
    };
    assert_eq!(run("hw boot", &mut env), 0);
    assert!(env.execs.is_empty());
}

#[test]
fn helpers_may_read_hardware_but_not_fix_it() {
    use controller::catalog::{Access, find_tool};
    for t in ["hw", "hw_list", "hw_status"] {
        assert_eq!(
            find_tool(t).and_then(|v| v.access()),
            Some(Access::Read),
            "{t}"
        );
    }
    // Loading modules and installing packages stays with people (wol hw fix) and the boot.
    assert!(find_tool("hw_fix").is_none());
    assert!(find_tool("hw_boot").is_none());
}

/// Network up (NetworkManager dispatcher, root): an online-only driver missing since first boot is
/// installed once per boot, without loading it (the next boot does); the session reads the result.
#[test]
fn network_up_installs_a_pending_driver_once_and_leaves_the_result() {
    use controller::run::Env;
    use shared::hardware::{ONLINE_PATH, OnlineInstall};
    let need = HardwareSnapshot {
        devices: vec![dev(
            Kind::Gpu,
            "NVIDIA GeForce",
            State::NeedsDriver,
            &[],
            &["nvidia-open"],
        )],
    };
    let result = |env: &FakeEnv| -> Option<OnlineInstall> {
        env.written
            .iter()
            .find(|(p, _)| p == &PathBuf::from(ONLINE_PATH))
            .and_then(|(_, t)| wol_core::hardware::online::parse(t))
    };
    for (code, ok) in [(0, true), (1, false)] {
        let mut env = FakeEnv {
            root: true,
            hw: vec![need.clone()],
            exec_codes: vec![code],
            ..Default::default()
        };
        assert_eq!(run("hw online", &mut env), 0);
        assert_eq!(
            env.execs,
            vec![vec![
                "pacman",
                "-S",
                "--needed",
                "--noconfirm",
                "nvidia-open"
            ]]
        );
        let r = result(&env).expect("result written");
        assert_eq!(
            r,
            OnlineInstall {
                at: env.now_secs(),
                packages: vec!["nvidia-open".into()],
                ok,
                graphics: true,
            }
        );
        assert!(
            env.written
                .iter()
                .any(|(p, _)| p == &PathBuf::from(SNAPSHOT_PATH)),
            "Settings sees the new state"
        );
    }
    // Already tried this boot (/run is cleared at boot), first boot still running (its own
    // `wol hw boot --online` installs), or nothing pending: nothing runs, nothing is written.
    for (files, hw) in [
        (
            vec![(PathBuf::from(ONLINE_PATH), "{}".to_owned())],
            need.clone(),
        ),
        (
            vec![(PathBuf::from("/firstboot"), String::new())],
            need.clone(),
        ),
        (vec![], fixed()),
    ] {
        let mut env = FakeEnv {
            root: true,
            hw: vec![hw],
            files,
            ..Default::default()
        };
        assert_eq!(run("hw online", &mut env), 0);
        assert!(
            env.execs.is_empty() && env.written.is_empty(),
            "{:?}",
            env.execs
        );
    }
    let mut env = FakeEnv {
        hw: vec![need],
        ..Default::default()
    };
    assert_ne!(run("hw online", &mut env), 0, "root only");
    assert!(env.execs.is_empty());
}
