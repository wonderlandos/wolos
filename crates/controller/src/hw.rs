//! `wol hw`: hardware list, status, fix, and the boot step (rc.d `wolos_hw`). Logic in
//! `wol_core::hardware`; root changes through wol-admind (`hw-load`, `pkg-install`).
use crate::cli::Invocation;
use crate::run::{Env, admin_send};
use shared::admin::{AdminAction, AdminRequest};
use shared::errors::ErrorCode;
use shared::hardware::{Device, HardwareSnapshot, ONLINE_PATH, OnlineInstall, SNAPSHOT_PATH};
use std::path::Path;
use wol_core::hardware::{boot_plan, fixes};

pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let json = inv.flag("json") || !env.stdout_tty();
    match inv.action {
        "hw.list" => {
            let s = env.hardware();
            if json {
                out_json(env, &serde_json::to_value(&s).unwrap_or_default());
            } else {
                env.out(&table(&s));
            }
            0
        }
        "hw.status" => {
            let s = env.hardware();
            if json {
                let (load, install) = fixes(&s);
                let problems: Vec<&Device> = s.problems().collect();
                out_json(
                    env,
                    &serde_json::json!({
                        "ready": problems.is_empty(),
                        "problems": problems,
                        "fix": {"load": load, "install": install},
                    }),
                );
            } else {
                env.out(&status(&s));
            }
            0
        }
        "hw.fix" => fix(env, inv.flag("yes")),
        "hw.boot" => boot(env, inv.flag("online")),
        "hw.online" => online(env),
        other => {
            env.err(&format!("wol: no handler for {other}\n"));
            ErrorCode::Failed.exit()
        }
    }
}

fn out_json(env: &mut dyn Env, v: &serde_json::Value) {
    env.out(&format!(
        "{}\n",
        serde_json::to_string_pretty(v).unwrap_or_default()
    ));
}

fn table(s: &HardwareSnapshot) -> String {
    let mut t = format!(
        "{:<12} {:<15} {:<16} {}\n",
        "KIND", "STATE", "DRIVER", "NAME"
    );
    for d in &s.devices {
        t.push_str(&format!(
            "{:<12} {:<15} {:<16} {}\n",
            d.kind.name(),
            d.state.name(),
            d.driver.as_deref().unwrap_or("-"),
            d.name
        ));
        if !d.hint.is_empty() {
            t.push_str(&format!("{:<12} {}\n", "", d.hint));
        }
    }
    t
}

fn status(s: &HardwareSnapshot) -> String {
    let lines: String = s
        .problems()
        .map(|d| format!("{} {}: {}\n", d.kind.name(), d.name, d.hint))
        .collect();
    if lines.is_empty() {
        format!("all {} devices ready\n", s.devices.len())
    } else {
        lines
    }
}

fn req(action: AdminAction, arg: &str) -> AdminRequest {
    AdminRequest {
        action,
        args: vec![arg.to_owned()],
    }
}

/// Load missing modules (wol-admind `hw-load`); with `--yes` also install missing packages.
fn fix(env: &mut dyn Env, yes: bool) -> i32 {
    let (load, install) = fixes(&env.hardware());
    if load.is_empty() && install.is_empty() {
        env.out("nothing to fix\n");
        return 0;
    }
    let mut code = 0;
    for k in &load {
        match admin_send(env, &req(AdminAction::HwLoad, k)) {
            0 => env.out(&format!("loaded {k}\n")),
            c => code = c,
        }
    }
    if !install.is_empty() && !yes {
        env.out(&format!(
            "missing packages: {}\ninstall them with: wol hw fix --yes\n",
            install.join(" ")
        ));
    } else {
        for p in &install {
            match admin_send(env, &req(AdminAction::PkgInstall, p)) {
                0 => env.out(&format!("installed {p}\n")),
                c => code = c,
            }
        }
    }
    let left = env.hardware().problems().count();
    if left > 0 {
        env.out(&format!(
            "{left} device(s) still need attention: wol hw status\n"
        ));
    }
    code
}

/// `/etc/dhclient-exit-hooks` on a new lease (root, background, one at a time via lockf): install
/// what the catalogue still misses (an NVIDIA driver first boot had no network for), once per boot,
/// without loading it: the next boot does. [`ONLINE_PATH`] tells the session (restart, or where
/// to fix it). Not while first boot runs: its own `wol hw boot --online` installs.
fn online(env: &mut dyn Env) -> i32 {
    if !env.is_root() {
        env.err("wol: hw online needs root\n");
        return ErrorCode::Denied.exit();
    }
    let done = |p: &str| env.read_file(Path::new(p)).is_some();
    if done(ONLINE_PATH) || done("/firstboot") {
        return 0;
    }
    let snap = env.hardware();
    let Some((packages, graphics)) = wol_core::hardware::online::pending(&snap) else {
        return 0;
    };
    let mut argv: Vec<String> = ["pkg", "install", "-y"].map(String::from).to_vec();
    argv.extend(packages.iter().cloned());
    let ok = matches!(env.exec(&argv), Ok(0));
    let what = packages.join(" ");
    env.out(&format!(
        "wolos_hw: pkg install {what}{}\n",
        if ok { "" } else { " failed" }
    ));
    let r = OnlineInstall {
        at: env.now_secs(),
        packages,
        ok,
        graphics,
    };
    let snap = env.hardware();
    for (path, text) in [
        (ONLINE_PATH, serde_json::to_string(&r).unwrap_or_default()),
        (
            SNAPSHOT_PATH,
            serde_json::to_string(&snap).unwrap_or_default(),
        ),
    ] {
        if let Err(e) = env.write_file(Path::new(path), &(text + "\n")) {
            env.out(&format!("wolos_hw: {path}: {e}\n"));
        }
    }
    0
}

/// rc.d `wolos_hw`, every boot (a disk moves between machines): set the tunables of modules about to
/// load (kenv), load the modules the catalogue names for the hardware present, write next-boot
/// tunables, publish the snapshot. Never fails boot.
/// `online` (wolos_firstboot, network up): first install the packages the catalogue says are missing
/// (NVIDIA drivers are not in the offline repo), so this boot already loads them.
fn boot(env: &mut dyn Env, online: bool) -> i32 {
    if !env.is_root() {
        env.err("wol: hw boot needs root\n");
        return ErrorCode::Denied.exit();
    }
    let mut snap = env.hardware();
    let (_, install) = fixes(&snap);
    if online && !install.is_empty() {
        let mut argv: Vec<String> = ["pkg", "install", "-y"].map(String::from).to_vec();
        argv.extend(install.iter().cloned());
        let what = install.join(" ");
        match env.exec(&argv) {
            Ok(0) => env.out(&format!("wolos_hw: pkg install {what}\n")),
            _ => env.out(&format!("wolos_hw: pkg install {what} failed\n")),
        }
        snap = env.hardware();
    }
    let plan = boot_plan(&snap, &|p| env.read_file(Path::new(p)));
    for (k, v) in &plan.kenv {
        let argv = ["kenv".to_string(), format!("{k}={v}")];
        match env.exec(&argv) {
            Ok(0) => env.out(&format!("wolos_hw: kenv {k}={v}\n")),
            _ => env.out(&format!("wolos_hw: kenv {k}={v} failed\n")),
        }
    }
    for k in &plan.kldload {
        let argv = ["kldload".to_string(), "-n".into(), k.clone()];
        match env.exec(&argv) {
            Ok(0) => env.out(&format!("wolos_hw: kldload {k}\n")),
            _ => env.out(&format!("wolos_hw: kldload {k} failed\n")),
        }
    }
    for (path, text) in &plan.writes {
        match env.write_file(Path::new(path), text) {
            Ok(()) => env.out(&format!("wolos_hw: wrote {path}\n")),
            Err(e) => env.out(&format!("wolos_hw: {path}: {e}\n")),
        }
    }
    let snap = if plan.kldload.is_empty() {
        snap
    } else {
        env.hardware()
    };
    let json = serde_json::to_string(&snap).unwrap_or_default() + "\n";
    if let Err(e) = env.write_file(Path::new(SNAPSHOT_PATH), &json) {
        env.out(&format!("wolos_hw: {SNAPSHOT_PATH}: {e}\n"));
    }
    for d in snap.problems() {
        env.out(&format!(
            "wolos_hw: {} {}: {}\n",
            d.kind.name(),
            d.name,
            d.hint
        ));
    }
    env.out(&format!(
        "wolos_hw: {} devices, {} need attention\n",
        snap.devices.len(),
        snap.problems().count()
    ));
    0
}
