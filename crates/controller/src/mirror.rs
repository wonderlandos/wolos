//! `wol mirror status|rank|gate|kernel` (ADR 0019): the WolOS mirrors and the `[wolos-cachy]` CPU
//! gate. Logic in `wol_core::mirror`; this reads the files, probes and writes.
use crate::run::{Env, fail};
use shared::errors::ErrorCode;
use std::path::Path;
use wol_core::mirror as m;

/// The file under `WOL_ROOT` (an installed disk, tests) or `/`.
fn at(env: &dyn Env, p: &str) -> String {
    format!("{}{p}", env.var("WOL_ROOT").unwrap_or_default())
}

fn read(env: &dyn Env, p: &str) -> String {
    env.read_file(Path::new(&at(env, p))).unwrap_or_default()
}

fn cpuinfo(env: &dyn Env) -> String {
    env.read_file(Path::new("/proc/cpuinfo"))
        .unwrap_or_default()
}

pub fn run(env: &mut dyn Env, action: &str, yes: bool, json: bool) -> i32 {
    match action {
        "mirror.status" => status(env, json),
        "mirror.rank" => rank(env, yes, json),
        "mirror.gate" => gate(env, yes, json),
        _ => kernel(env),
    }
}

fn status(env: &mut dyn Env, json: bool) -> i32 {
    let cpu = cpuinfo(env);
    let conf = read(env, m::PACMAN_CONF);
    let servers = m::servers(&read(env, m::MIRRORLIST));
    let v = serde_json::json!({
        "level": m::x86_64_level(&cpu),
        "cachy_supported": m::cachy_supported(&cpu),
        "cachy_enabled": m::cachy_enabled(&conf),
        "servers": servers,
    });
    if json || !env.stdout_tty() {
        env.out(&format!("{v}\n"));
        return 0;
    }
    let mut s = format!(
        "CPU x86-64-v{}; [wolos-cachy] {}{}\n",
        m::x86_64_level(&cpu),
        if m::cachy_enabled(&conf) { "on" } else { "off" },
        if m::cachy_supported(&cpu) {
            ""
        } else {
            " (needs x86-64-v3)"
        }
    );
    for sv in &servers {
        s.push_str(&format!("{sv}\n"));
    }
    env.out(&s);
    0
}

/// Probe every server in parallel; print fastest first; `--yes` (root) writes the new order.
fn rank(env: &mut dyn Env, yes: bool, json: bool) -> i32 {
    let list = read(env, m::MIRRORLIST);
    let servers = m::servers(&list);
    if servers.is_empty() {
        return fail(
            env,
            ErrorCode::NotFound,
            &format!("no servers in {}", m::MIRRORLIST),
        );
    }
    let urls: Vec<String> = servers.iter().map(|s| m::probe_url(s)).collect();
    let probe = env.prober();
    let ms = m::probe_all(&urls, &*probe);
    let ranked = m::rank(&list, &ms);
    let mut rows: Vec<(Option<u64>, &String)> = ms.iter().copied().zip(&servers).collect();
    rows.sort_by_key(|(t, _)| (t.is_none(), t.unwrap_or(0)));
    if json || !env.stdout_tty() {
        let v: Vec<_> = rows
            .iter()
            .map(|(t, s)| serde_json::json!({"server": s, "ms": t}))
            .collect();
        env.out(&format!("{}\n", serde_json::Value::from(v)));
    } else {
        for (t, s) in &rows {
            let t = t.map_or_else(|| "unreachable".into(), |t| format!("{t} ms"));
            env.out(&format!("{t:>12}  {s}\n"));
        }
    }
    if !yes {
        return 0;
    }
    if !env.is_root() {
        return fail(env, ErrorCode::Denied, "mirror rank --yes needs root");
    }
    let path = at(env, m::MIRRORLIST);
    match env.write_file(Path::new(&path), &ranked) {
        Ok(()) => 0,
        Err(e) => fail(env, ErrorCode::Failed, &format!("{}: {e}", m::MIRRORLIST)),
    }
}

/// `[wolos-cachy]` on exactly when the CPU runs x86-64-v3 builds (installer, first boot).
fn gate(env: &mut dyn Env, yes: bool, json: bool) -> i32 {
    let on = m::cachy_supported(&cpuinfo(env));
    let conf = read(env, m::PACMAN_CONF);
    if conf.is_empty() {
        return fail(env, ErrorCode::NotFound, &format!("no {}", m::PACMAN_CONF));
    }
    let new = m::gate(&conf, on);
    if new != conf {
        if !yes {
            return fail(
                env,
                ErrorCode::Usage,
                "mirror gate changes pacman.conf; pass --yes",
            );
        }
        if !env.is_root() {
            return fail(env, ErrorCode::Denied, "mirror gate needs root");
        }
        let path = at(env, m::PACMAN_CONF);
        if let Err(e) = env.write_file(Path::new(&path), &new) {
            return fail(env, ErrorCode::Failed, &format!("{}: {e}", m::PACMAN_CONF));
        }
    }
    let state = if on { "on" } else { "off" };
    if json || !env.stdout_tty() {
        env.out(&format!("{}\n", serde_json::json!({"wolos_cachy": state})));
    } else {
        env.out(&format!("[wolos-cachy] {state}\n"));
    }
    0
}

/// The system kernel package for this CPU (`linux-wolos-v3` when a repo offers it).
fn kernel(env: &mut dyn Env) -> i32 {
    let cpu = cpuinfo(env);
    let available =
        m::cachy_supported(&cpu) && env.capture(&["pacman", "-Si", m::KERNEL_V3]).is_some();
    env.out(&format!("{}\n", m::kernel_package(&cpu, available)));
    0
}

/// Real probe: time to the first byte of the server's database, curl, 5 s at most.
pub fn curl_probe(url: &str) -> Option<u64> {
    let out = wol_core::system::run_with_timeout(
        &[
            "curl",
            "-fsS",
            "-o",
            "/dev/null",
            "-r",
            "0-0",
            "--max-time",
            "5",
            "-w",
            "%{time_starttransfer}",
            url,
        ],
        std::time::Duration::from_secs(8),
    )?;
    let secs: f64 = out.trim().parse().ok()?;
    Some((secs * 1000.0).round() as u64)
}
