//! No args: Hyprland events -> frontmost QoS boost + event bus (wold).
//! `gpu env` / `gpu offload` / `gpu hyprconf [--vrr]`: session GPU env, one app's dGPU env, Hyprland fragment.
//! `gpu caps` / `gpu sample`: ADR 0007 JSON (root: debugfs, cpuctl). `hypr stats`: frame ring of
//! wol-hypr (hyprctl `wolstats`). `power show` / `power set [--pl1 W] [--pl2 W]`: Intel RAPL package
//! limits (root, cpuctl). Thin composition root.
use realtime::{
    bus::Publisher, glass::GlassConfig, gpu_env, gpu_probe, gpu_select, gpu_tune, hypr, power,
    qos::Qos,
};
use shared::errors::ErrorCode;

const GLASS: &str = "/etc/wolos/glass.json";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] => events(),
        ["gpu", what @ ("caps" | "sample")] => {
            let wol_gpu = std::env::var("WOL_GPU").ok();
            let gpus = gpu_probe::gpus();
            let Some(g) = gpu_select::choose(&gpus, wol_gpu.as_deref()) else {
                eprintln!("wol-realtime: no GPU");
                std::process::exit(ErrorCode::NotFound.exit());
            };
            let dir = gpu_tune::debugfs_dir(&g.node).unwrap_or_default();
            let b = gpu_tune::backend(&g.driver, dir);
            println!(
                "{}",
                if *what == "caps" {
                    b.caps().to_json()
                } else {
                    b.sample().to_json()
                }
            );
        }
        ["power", "show"] => match read_limits() {
            Some((_, _, l)) => println!("{}", l.to_json()),
            None => fail("no RAPL MSR access (root, kldload cpuctl, Intel CPU)"),
        },
        ["power", "set", rest @ ..] => {
            let Some((pl1, pl2)) = power::parse_set(rest) else {
                return usage();
            };
            let Some((units, msr, _)) = read_limits() else {
                return fail("no RAPL MSR access (root, kldload cpuctl, Intel CPU)");
            };
            match power::with_watts(units, msr, pl1, pl2) {
                Ok(v) => {
                    let ok = std::process::Command::new("cpucontrol")
                        .args([
                            "-m",
                            &format!("{:#x}={v:#x}", power::MSR_PKG_POWER_LIMIT),
                            "/dev/cpuctl0",
                        ])
                        .status()
                        .is_ok_and(|s| s.success());
                    match (ok, read_limits()) {
                        (true, Some((_, _, l))) => println!("{}", l.to_json()),
                        _ => fail("cpucontrol write failed"),
                    }
                }
                Err(e) => fail(&format!("refused: {e:?}")),
            }
        }
        ["hypr", "stats"] => match hypr::request("wolstats") {
            Ok(j) => println!("{}", j.trim()),
            Err(e) => {
                eprintln!("wol-realtime: hyprctl wolstats: {e} (wol-hypr loaded?)");
                std::process::exit(ErrorCode::Unavailable.exit());
            }
        },
        ["gpu", rest @ ..] => {
            let glass = GlassConfig::parse(&std::fs::read_to_string(GLASS).unwrap_or_default());
            let wol_gpu = std::env::var("WOL_GPU").ok();
            let gpus = gpu_probe::gpus();
            match gpu_env::gpu_command(
                rest,
                &gpus,
                wol_gpu.as_deref(),
                &gpu_env::Userland::probe(),
                &glass,
                gpu_env::on_battery(),
            ) {
                Some(out) => print!("{out}"),
                None => usage(),
            }
        }
        _ => usage(),
    }
}

fn read_limits() -> Option<(u64, u64, power::PkgLimits)> {
    let units = gpu_tune::cpuctl_msr(power::MSR_RAPL_POWER_UNIT)?;
    let msr = gpu_tune::cpuctl_msr(power::MSR_PKG_POWER_LIMIT)?;
    Some((units, msr, power::decode(units, msr)))
}

fn fail(msg: &str) {
    eprintln!("wol-realtime: {msg}");
    std::process::exit(ErrorCode::Unavailable.exit());
}

fn usage() {
    eprintln!(
        "usage: wol-realtime [gpu env | gpu offload | gpu hyprconf [--vrr] | gpu caps | gpu sample | hypr stats | power show | power set [--pl1 W] [--pl2 W]]"
    );
    std::process::exit(ErrorCode::Usage.exit());
}

fn events() {
    let events = match hypr::events() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("wol-realtime: hyprland event socket: {e}");
            std::process::exit(ErrorCode::Unavailable.exit());
        }
    };
    let mut qos = Qos::default();
    let mut bus = Publisher::new(shared::jobs::SOCKET, std::io::stdout());
    let lookup = || {
        hypr::request("j/activewindow")
            .ok()
            .and_then(|j| hypr::active_pid(&j))
    };
    hypr::run(
        events,
        lookup,
        |pid| {
            qos.foreground(pid);
        },
        |e| {
            bus.send(e);
        },
    );
}
