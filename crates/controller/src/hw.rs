// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol hw`: hardware list, status, fix, and the boot step (`wolos-hw.service`). Logic in
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
        "hw.compute" => {
            let s = env.hardware();
            let c = compute(env, &s);
            if json {
                out_json(env, &serde_json::to_value(&c).unwrap_or_default());
            } else {
                env.out(&format!(
                    "backend: {}\nvram_mb: {}\ngpu: {}\n",
                    c.backend, c.vram_mb, c.gpu
                ));
            }
            0
        }
        "hw.report" => {
            // Always JSON: the file a person sends to the hardware list (`wol hw report > r.json`).
            let os = env
                .read_file(Path::new("/etc/os-release"))
                .and_then(|t| wol_core::base::version(&t))
                .unwrap_or_default();
            let s = env.hardware();
            out_json(env, &wol_core::hardware::catalog::report(&s, &os));
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

/// `wol hw compute --json` (ADR 0032): which local-AI backend package fits this machine.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Compute {
    /// `cuda` | `rocm` | `vulkan` | `cpu`.
    pub backend: &'static str,
    /// Dedicated video memory in MiB; 0 when unknown (integrated, or no driver yet).
    pub vram_mb: u64,
    /// Catalogue name of the GPU, empty for `cpu`.
    pub gpu: String,
}

/// AMD GPUs older than Vega (GCN 1-4, by PCI device id) have no ROCm support: Vulkan instead.
/// Vega 10 (0x6860-0x687f), Vega 12 (0x69a0-0x69af), Vega 20 (0x66a0-0x66af) and every APU from
/// Raven on (0x15xx, 0x16xx) sit outside these ranges.
const AMD_PRE_VEGA: &[(u16, u16)] = &[
    (0x1300, 0x131f),
    (0x6600, 0x669f),
    (0x66b0, 0x685f),
    (0x6880, 0x699f),
    (0x7300, 0x730f),
    (0x9830, 0x98ff),
];

/// Rank of the backend a device gives: higher wins when a machine has several GPUs.
fn backend_of(d: &Device) -> Option<(&'static str, u8)> {
    if d.kind != shared::hardware::Kind::Gpu {
        return None;
    }
    let has = |k: &str| d.klds.iter().any(|m| m == k);
    // The catalogue gives `nvidia` to Turing and newer only; older cards get nouveau.
    if has("nvidia") {
        return Some(("cuda", 3));
    }
    if has("amdgpu") {
        let dev =
            d.id.split(':')
                .nth(1)
                .and_then(|h| u16::from_str_radix(h, 16).ok());
        let pre_vega =
            dev.is_some_and(|x| AMD_PRE_VEGA.iter().any(|(a, b)| (*a..=*b).contains(&x)));
        return Some(if pre_vega { ("vulkan", 1) } else { ("rocm", 2) });
    }
    // Real Vulkan drivers; virtual GPUs (bochs, vmwgfx, virtio) would only give llvmpipe.
    ["i915", "xe", "nouveau", "radeon"]
        .iter()
        .any(|k| has(k))
        .then_some(("vulkan", 1))
}

/// The GPU that decides, and its backend: the best-ranked GPU device, else `cpu`.
pub fn pick_gpu(s: &HardwareSnapshot) -> Option<(&Device, &'static str)> {
    s.devices
        .iter()
        .filter_map(|d| backend_of(d).map(|(b, rank)| (d, b, rank)))
        .max_by_key(|x| x.2)
        .map(|(d, b, _)| (d, b))
}

/// MiB from `nvidia-smi --query-gpu=memory.total` ("8192") or sysfs `mem_info_vram_total` (bytes).
pub fn parse_vram_mb(nvidia_smi: Option<&str>, sysfs_bytes: Option<&str>) -> u64 {
    let smi = nvidia_smi
        .and_then(|t| t.lines().next())
        .and_then(|l| l.trim().parse::<u64>().ok());
    if let Some(n) = smi {
        return n;
    }
    sysfs_bytes
        .and_then(|t| t.trim().parse::<u64>().ok())
        .map_or(0, |b| b / (1024 * 1024))
}

/// Backend, VRAM and GPU name for the machine in `s`. VRAM: nvidia-smi for NVIDIA (the open driver
/// has no sysfs size), `/sys/bus/pci/devices/<addr>/mem_info_vram_total` for amdgpu; else 0.
pub fn compute(env: &mut dyn Env, s: &HardwareSnapshot) -> Compute {
    let Some((d, backend)) = pick_gpu(s) else {
        return Compute {
            backend: "cpu",
            vram_mb: 0,
            gpu: String::new(),
        };
    };
    let (location, gpu) = (d.location.clone(), d.name.clone());
    let smi = (backend == "cuda")
        .then(|| {
            env.capture(&[
                "nvidia-smi",
                "--query-gpu=memory.total",
                "--format=csv,noheader,nounits",
            ])
        })
        .flatten();
    let sysfs = env.read_file(
        &Path::new("/sys/bus/pci/devices")
            .join(&location)
            .join("mem_info_vram_total"),
    );
    Compute {
        backend,
        vram_mb: parse_vram_mb(smi.as_deref(), sysfs.as_deref()),
        gpu,
    }
}

/// pacman, non-interactive, skipping what is already installed.
fn pacman_install(pkgs: &[String]) -> Vec<String> {
    let mut argv: Vec<String> = ["pacman", "-S", "--needed", "--noconfirm"]
        .map(String::from)
        .to_vec();
    argv.extend(pkgs.iter().cloned());
    argv
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

/// NetworkManager dispatcher on `up` (root, background, one at a time): install
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
    let argv = pacman_install(&packages);
    let ok = matches!(env.exec(&argv), Ok(0));
    let what = packages.join(" ");
    env.out(&format!(
        "wolos_hw: pacman -S {what}{}\n",
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

/// `wolos-hw.service`, every boot (a disk moves between machines): write modprobe options and the
/// boot module list, load the modules the catalogue names for the hardware present, publish the
/// snapshot. Never fails boot.
/// `online` (first boot, network up): first install the packages the catalogue says are missing
/// (NVIDIA drivers are not in the offline repo), so this boot already loads them.
fn boot(env: &mut dyn Env, online: bool) -> i32 {
    if !env.is_root() {
        env.err("wol: hw boot needs root\n");
        return ErrorCode::Denied.exit();
    }
    let mut snap = env.hardware();
    let (_, install) = fixes(&snap);
    // audit8 stability2 #10: a failed online install is the exit code (the rest still runs), so
    // firstboot sees it and the network-up hook retries.
    let mut code = 0;
    if online && !install.is_empty() {
        let what = install.join(" ");
        match env.exec(&pacman_install(&install)) {
            Ok(0) => env.out(&format!("wolos_hw: pacman -S {what}\n")),
            _ => {
                env.out(&format!("wolos_hw: pacman -S {what} failed\n"));
                code = 1;
            }
        }
        snap = env.hardware();
    }
    let plan = boot_plan(&snap, &|p| env.read_file(Path::new(p)));
    // Options first: a module reads modprobe.d only when it loads.
    for (path, text) in &plan.writes {
        match env.write_file(Path::new(path), text) {
            Ok(()) => env.out(&format!("wolos_hw: wrote {path}\n")),
            Err(e) => env.out(&format!("wolos_hw: {path}: {e}\n")),
        }
    }
    for k in &plan.modprobe {
        let argv = ["modprobe".to_string(), k.clone()];
        match env.exec(&argv) {
            Ok(0) => env.out(&format!("wolos_hw: modprobe {k}\n")),
            _ => env.out(&format!("wolos_hw: modprobe {k} failed\n")),
        }
    }
    let snap = if plan.modprobe.is_empty() {
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
    code
}
