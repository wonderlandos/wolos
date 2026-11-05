// SPDX-License-Identifier: GPL-2.0-or-later
//! atlas-drive: `serve` (the daemon), every Drive verb as a subcommand, `mcp` (stdio MCP server),
//! `permissions`, `--help-agent`. See docs/modules/atlas-drive.md.
#![forbid(unsafe_code)]
use atlas_drive::{browser::LaunchConfig, cli, client::Client, daemon, mcp, perms, server, spec};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

fn usage() -> &'static str {
    "usage: atlas-drive [--socket PATH] [--no-spawn] <command>\n  serve [--browser PATH] [--headed] [--profile DIR] [--no-sandbox] [--disable-gpu] [--log FILE] [--launch]\n  <verb> [args]      any Drive verb (see --help-agent)\n  mcp                MCP server on stdio\n  mcp-manifest       the tool manifest as JSON\n  permissions list | allow <exe> [--scopes a,b] | revoke <exe> | auto on [--for 2h] [--site S] | auto off\n  stop               close the browser and the daemon\n  --help-agent       one page for a model's context"
}

fn die(msg: &str) -> ExitCode {
    eprintln!("atlas-drive: {msg}");
    ExitCode::from(2)
}

/// glibc's allocator keeps one arena per thread and raises its mmap threshold as big blocks come and go:
/// after one heavy page (tens of MB of accessibility JSON) a daemon would stay several times larger than
/// it needs to be. Two arenas and a fixed mmap threshold return big blocks at once. The variables are read
/// at process start, so `serve` starts itself once more with them set (`exec`: same pid, nothing is lost).
fn tune_allocator() {
    use std::os::unix::process::CommandExt;
    if std::env::var_os("MALLOC_ARENA_MAX").is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let e = std::process::Command::new(exe)
        .args(std::env::args().skip(1))
        .env("MALLOC_ARENA_MAX", "2")
        .env("MALLOC_MMAP_THRESHOLD_", "262144")
        .env("MALLOC_TRIM_THRESHOLD_", "262144")
        .exec();
    eprintln!("atlas-drive: could not restart with the allocator settings: {e}");
}

fn serve(args: &[String]) -> ExitCode {
    tune_allocator();
    let mut c = LaunchConfig::default();
    let mut socket = daemon::socket_path();
    let mut launch = false;
    let mut i = 0;
    let next = |i: &mut usize| -> Option<String> {
        *i += 1;
        args.get(*i).cloned()
    };
    while i < args.len() {
        match args[i].as_str() {
            "--browser" => c.binary = next(&mut i).map(PathBuf::from).unwrap_or(c.binary),
            "--headed" => c.headless = false,
            "--profile" => c.profile = next(&mut i).map(PathBuf::from),
            "--no-sandbox" => c.no_sandbox = true,
            "--disable-gpu" => c.disable_gpu = true,
            "--log" => c.log = next(&mut i).map(PathBuf::from),
            "--socket" => socket = next(&mut i).map(PathBuf::from).unwrap_or(socket),
            "--launch" => launch = true,
            "--arg" => {
                if let Some(a) = next(&mut i) {
                    c.extra_args.push(a);
                }
            }
            other => return die(&format!("serve: unknown option {other}")),
        }
        i += 1;
    }
    if let Err(e) = atlas_drive::browser::forbid_debug_port(&c.extra_args) {
        return die(&e.line());
    }
    let listener = match server::bind(&socket) {
        Ok(l) => l,
        Err(e) => return die(&format!("cannot bind {}: {e}", socket.display())),
    };
    let state_dir = socket
        .parent()
        .map(|p| p.join("drive"))
        .unwrap_or_else(|| PathBuf::from("/tmp/atlas-drive"));
    let d = daemon::Daemon::new(c, state_dir);
    if launch && let Err(e) = d.browser() {
        return die(&e.line());
    }
    let cfg = server::ServerCfg {
        grants_path: perms::config_path(),
        ci: std::env::var("ATLAS_DRIVE_CI").is_ok_and(|v| v == "1"),
        consent_cmd: std::env::var("ATLAS_DRIVE_CONSENT_CMD")
            .ok()
            .filter(|s| !s.is_empty()),
        own_exe: std::env::current_exe()
            .ok()
            .and_then(|p| p.canonicalize().ok()),
        attached_file: socket.parent().map(|p| p.join("drive.attached")),
    };
    eprintln!("atlas-drive: serving {}", socket.display());
    match server::serve(listener, d, cfg) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => die(&e.to_string()),
    }
}

fn connect(socket: &std::path::Path, spawn: bool) -> Result<Client, String> {
    if let Ok(c) = Client::connect(socket) {
        return Ok(c);
    }
    if !spawn {
        return Err(format!(
            "no daemon on {}; start it: atlas-drive serve &",
            socket.display()
        ));
    }
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("serve").arg("--socket").arg(socket);
    for a in std::env::var("ATLAS_DRIVE_SERVE_ARGS")
        .unwrap_or_default()
        .split_whitespace()
    {
        cmd.arg(a);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0);
    cmd.spawn()
        .map_err(|e| format!("cannot start the daemon: {e}"))?;
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        if let Ok(c) = Client::connect(socket) {
            return Ok(c);
        }
    }
    Err("the daemon did not come up".into())
}

fn permissions(args: &[String]) -> ExitCode {
    let path = perms::config_path();
    let mut g = perms::load(&path);
    let now = perms::now();
    let save = |g: &perms::Grants| -> ExitCode {
        match perms::save(&path, g) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => die(&format!("cannot write {}: {e}", path.display())),
        }
    };
    match args.first().map(String::as_str) {
        None | Some("list") => {
            print!("{}", g.list(now));
            ExitCode::SUCCESS
        }
        Some("allow") => {
            let Some(exe) = args.get(1) else {
                return die("allow needs the agent's executable path (see: permissions list)");
            };
            let scopes: Vec<String> = match args.iter().position(|a| a == "--scopes") {
                Some(i) => args
                    .get(i + 1)
                    .map(|s| s.split(',').map(String::from).collect())
                    .unwrap_or_default(),
                None => perms::SCOPES.iter().map(|s| s.to_string()).collect(),
            };
            g.grant(exe, &scopes, now);
            save(&g)
        }
        Some("revoke") => {
            let Some(exe) = args.get(1) else {
                return die("revoke needs the agent's executable path");
            };
            if !g.revoke(exe) {
                return die(&format!("{exe} had no grant"));
            }
            save(&g)
        }
        Some("auto") => match args.get(1).map(String::as_str) {
            Some("on") => {
                let until = args
                    .iter()
                    .position(|a| a == "--for")
                    .and_then(|i| args.get(i + 1))
                    .and_then(|s| perms::parse_span(s))
                    .map(|ms| now + ms);
                let sites: Vec<String> = args
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| *a == "--site")
                    .filter_map(|(i, _)| args.get(i + 1).cloned())
                    .collect();
                g.auto = Some(perms::Auto { until, sites });
                save(&g)
            }
            Some("off") => {
                g.auto = None;
                save(&g)
            }
            _ => die("auto on [--for 2h] [--site example.com] | auto off"),
        },
        Some(o) => die(&format!("unknown permissions command {o}")),
    }
}

fn mcp_stdio(socket: &std::path::Path, spawn: bool) -> ExitCode {
    let mut client: Option<Client> = None;
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let mut call = |verb: &str, params: Value| -> Result<Value, String> {
            if client.is_none() {
                client = Some(connect(socket, spawn)?);
            }
            client
                .as_mut()
                .map(|c| c.verb(verb, params))
                .unwrap_or_else(|| Err("not connected".into()))
        };
        if let Some(reply) = mcp::handle(&req, &mut call) {
            let _ = writeln!(out, "{reply}");
            let _ = out.flush();
        }
    }
    ExitCode::SUCCESS
}

fn verb_help(v: &spec::Verb) -> String {
    let mut s = format!("atlas-drive {}: {}\n", v.name.replace('_', "-"), v.doc);
    for a in v.args {
        s.push_str(&format!(
            "  {}{} ({}) {}\n",
            if a.required { "<" } else { "--" },
            a.name.replace('_', "-"),
            a.ty,
            a.doc
        ));
    }
    s.push_str(&format!("  scope: {}", v.scope));
    s
}

fn run_verb(socket: &std::path::Path, spawn: bool, name: &str, args: &[String]) -> ExitCode {
    let name = name.replace('-', "_");
    let Some(verb) = spec::find(&name) else {
        return die(&format!(
            "unknown command {name}; run atlas-drive --help-agent"
        ));
    };
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", verb_help(verb));
        return ExitCode::SUCCESS;
    }
    let mut params = match cli::parse(verb, args) {
        Ok(p) => p,
        Err(e) => return die(&e),
    };
    if name == "batch" {
        params["stream_only"] = true.into();
    }
    let mut c = match connect(socket, spawn) {
        Ok(c) => c,
        Err(e) => return die(&e),
    };
    let mut on_line = |v: &Value| {
        if let Some(ev) = v.get("event") {
            match ev.get("step") {
                Some(st) => {
                    let head = format!(
                        "# {} {}",
                        st["id"].as_str().unwrap_or(""),
                        st["verb"].as_str().unwrap_or("")
                    );
                    match st.get("error").and_then(Value::as_str) {
                        Some(e) => println!("{head} ERROR: {e}"),
                        None => print!("{head}\n{}", cli::render(&st["r"])),
                    }
                }
                None => println!("{ev}"),
            }
        } else if v.get("method").is_some() {
            println!("{v}");
        }
    };
    match c.request(&format!("Drive.{name}"), params, &mut on_line) {
        Ok(r) => {
            print!("{}", cli::render(&r));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut socket = daemon::socket_path();
    let mut spawn = true;
    while let Some(first) = args.first() {
        match first.as_str() {
            "--socket" if args.len() > 1 => {
                socket = PathBuf::from(args[1].clone());
                args.drain(..2);
            }
            "--no-spawn" => {
                spawn = false;
                args.remove(0);
            }
            _ => break,
        }
    }
    let Some(cmd) = args.first().cloned() else {
        println!("{}", usage());
        return ExitCode::from(2);
    };
    let rest = &args[1..];
    match cmd.as_str() {
        "--help-agent" | "help-agent" => {
            print!("{}", spec::help_agent());
            ExitCode::SUCCESS
        }
        "-h" | "--help" | "help" => {
            println!("{}", usage());
            ExitCode::SUCCESS
        }
        "mcp-manifest" => {
            println!("{}", spec::mcp_tools());
            ExitCode::SUCCESS
        }
        "serve" => {
            let mut a = rest.to_vec();
            if !a.iter().any(|x| x == "--socket") {
                a.push("--socket".into());
                a.push(socket.display().to_string());
            }
            serve(&a)
        }
        "mcp" => mcp_stdio(&socket, spawn),
        "permissions" => permissions(rest),
        "stop" => match connect(&socket, false) {
            Ok(mut c) => {
                let _ = c.verb("shutdown", json!({}));
                ExitCode::SUCCESS
            }
            Err(_) => ExitCode::SUCCESS,
        },
        verb => run_verb(&socket, spawn, verb, rest),
    }
}
