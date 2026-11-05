// SPDX-License-Identifier: GPL-2.0-or-later
//! Benchmark of Atlas Drive for docs/bench/atlas-drive.md: one run, one JSON object on stdout.
//! `bench --bin <atlas-drive> --browser <atlas> --pages name=url,name=url --app <name>`
//! Same sequence as tools/bench/atlas-drive/pw.mjs (Playwright), so the orchestrator can compare.
use atlas_drive::client::Client;
use serde_json::{Value, json};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn ms(t: Instant) -> f64 {
    (t.elapsed().as_secs_f64() * 1000.0 * 100.0).round() / 100.0
}

fn tokens(bytes: usize) -> usize {
    bytes.div_ceil(4)
}

fn ctx_switches(pid: u32) -> u64 {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(format!("/proc/{pid}/task")) {
        for e in rd.flatten() {
            if let Ok(s) = std::fs::read_to_string(e.path().join("status")) {
                for l in s.lines() {
                    if l.starts_with("voluntary_ctxt_switches:")
                        || l.starts_with("nonvoluntary_ctxt_switches:")
                    {
                        n += l
                            .split_whitespace()
                            .nth(1)
                            .and_then(|v| v.parse::<u64>().ok())
                            .unwrap_or(0);
                    }
                }
            }
        }
    }
    n
}

fn rss_kb(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

fn ref_of(text: &str, needle: &str) -> Option<String> {
    text.lines()
        .find(|l| l.contains(needle))
        .and_then(|l| l.trim_start().strip_prefix("[e"))
        .and_then(|r| r.split(']').next())
        .map(|n| format!("e{n}"))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_default()
    };
    let (bin, browser, app) = (arg("--bin"), arg("--browser"), arg("--app"));
    let pages: Vec<(String, String)> = arg("--pages")
        .split(',')
        .filter_map(|p| p.split_once('='))
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
    let sock = std::env::temp_dir().join(format!("adb-{}/drive.sock", std::process::id()));
    // the daemon makes the directory 0700 and refuses one that is not private (QA-A10-CS-3)
    {
        use std::os::unix::fs::DirBuilderExt;
        let _ = std::fs::DirBuilder::new()
            .mode(0o700)
            .create(sock.parent().unwrap());
    }
    let mut out = json!({});

    // cold start: process start -> first command answered
    let t0 = Instant::now();
    // ATLAS_DRIVE_BENCH_ARGS: extra browser flags for experiments (each passed as `--arg F`)
    let extra: Vec<String> = std::env::var("ATLAS_DRIVE_BENCH_ARGS")
        .unwrap_or_default()
        .split_whitespace()
        .flat_map(|a| ["--arg".to_string(), a.to_string()])
        .collect();
    let mut daemon = Command::new(&bin)
        .args([
            "serve",
            "--socket",
            &sock.display().to_string(),
            "--browser",
            &browser,
            "--no-sandbox",
            "--disable-gpu",
        ])
        .args(&extra)
        .env("ATLAS_DRIVE_CI", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(
            std::env::var_os("ATLAS_DRIVE_BENCH_LOG")
                .and_then(|p| std::fs::File::create(p).ok())
                .map(Stdio::from)
                .unwrap_or_else(Stdio::null),
        )
        .spawn()
        .expect("daemon");
    let mut c = loop {
        match Client::connect(&sock) {
            Ok(c) => break c,
            Err(_) => std::thread::sleep(Duration::from_millis(1)),
        }
    };
    c.verb("goto", json!({"url": "about:blank"}))
        .expect("first goto");
    out["cold_ms"] = json!(ms(t0));
    let dpid = daemon.id();
    let bpid = c
        .verb("status", json!({}))
        .ok()
        .and_then(|s| s["browser"]["pid"].as_u64())
        .unwrap_or(0) as u32;

    let mut per_page = serde_json::Map::new();
    for (name, url) in &pages {
        let mut m = serde_json::Map::new();
        let t = Instant::now();
        match c.verb("goto", json!({"url": url})) {
            Ok(r) => {
                if r["retried"] == true {
                    // the browser aborted the first attempt (not the driver): counted apart
                    m.insert("goto_retried".into(), json!(1));
                }
                m.insert("goto_ms".into(), json!(ms(t)))
            }
            Err(e) => {
                m.insert("error".into(), json!(e));
                per_page.insert(name.clone(), Value::Object(m));
                continue;
            }
        };
        let t = Instant::now();
        let s = c.verb("snapshot", json!({})).expect("snapshot");
        m.insert("snap_ms".into(), json!(ms(t)));
        let text = s["text"].as_str().unwrap_or("");
        m.insert("snap_bytes".into(), json!(text.len()));
        m.insert("snap_tokens".into(), json!(tokens(text.len())));
        let t = Instant::now();
        let _ = c.verb("snapshot", json!({"safe": false}));
        m.insert("snap_unsafe_ms".into(), json!(ms(t)));
        let t = Instant::now();
        let s = c
            .verb("snapshot", json!({"interactive_only": true}))
            .expect("snapshot io");
        m.insert("snap_io_ms".into(), json!(ms(t)));
        m.insert(
            "snap_io_bytes".into(),
            json!(s["text"].as_str().unwrap_or("").len()),
        );
        let t = Instant::now();
        let _ = c.verb("snapshot", json!({"max_tokens": 1000}));
        m.insert("snap_1000_ms".into(), json!(ms(t)));
        per_page.insert(name.clone(), Value::Object(m));
    }
    out["pages"] = Value::Object(per_page);
    // round 2: a navigation once the browser has idled a second (its spare renderer is ready), the
    // same for both tools; the first visits above pay the renderer start of a just-launched browser
    if let Some((_, url)) = pages.iter().find(|(n, _)| *n == app) {
        std::thread::sleep(Duration::from_millis(1000));
        let t = Instant::now();
        c.verb("goto", json!({"url": format!("{url}?warm=1")}))
            .expect("goto warm");
        out["goto_warm_ms"] = json!(ms(t));
    }

    // the app page: click round trip, 100 clicks, screenshot, agent task
    if let Some((_, url)) = pages.iter().find(|(n, _)| *n == app) {
        c.verb("goto", json!({"url": url})).expect("goto app");
        let s = c
            .verb("snapshot", json!({"interactive_only": true}))
            .expect("snap");
        let text = s["text"].as_str().unwrap_or("").to_string();
        let bottom = ref_of(&text, "Bottom button").expect("bottom ref");
        let mut one = vec![];
        for _ in 0..10 {
            let t = Instant::now();
            c.verb("click", json!({"target": bottom})).expect("click");
            one.push(ms(t));
        }
        out["click_ms"] = json!(one);
        let mut loc = vec![];
        for _ in 0..10 {
            let t = Instant::now();
            c.verb(
                "click",
                json!({"target": "role=button name=\"Bottom button\""}),
            )
            .expect("click loc");
            loc.push(ms(t));
        }
        out["click_locator_ms"] = json!(loc);
        let t = Instant::now();
        for _ in 0..100 {
            c.verb("click", json!({"target": bottom})).expect("click");
        }
        out["clicks100_ms"] = json!(ms(t));
        let shot = std::env::temp_dir().join(format!("adb-shot-{}.png", std::process::id()));
        let t = Instant::now();
        let r = c
            .verb(
                "screenshot",
                json!({"full_page": true, "out": shot.display().to_string()}),
            )
            .expect("shot");
        out["shot_ms"] = json!(ms(t));
        out["shot_bytes"] = r["bytes"].clone();
        let _ = std::fs::remove_file(shot);

        // agent task: find and press Save, read the toast
        c.verb("goto", json!({"url": url})).expect("goto app");
        let t = Instant::now();
        let mut toks = 0;
        let s = c
            .verb("snapshot", json!({"interactive_only": true}))
            .expect("snap");
        toks += s["tokens"].as_u64().unwrap_or(0);
        let text = s["text"].as_str().unwrap_or("").to_string();
        let (email, save) = (
            ref_of(&text, "textbox \"Email\"").expect("email"),
            ref_of(&text, "button \"Save\"").expect("save"),
        );
        let r = c
            .verb(
                "batch",
                json!({"steps": [
                    {"verb": "fill", "params": {"target": email, "text": "me@example.com"}},
                    {"verb": "click", "params": {"target": save, "then": "diff"}},
                ]}),
            )
            .expect("batch");
        toks += r["tokens"].as_u64().unwrap_or(0);
        let toast_seen = r.to_string().contains("Saved!");
        out["task"] = json!({"ms": ms(t), "tokens": toks, "toast_seen": toast_seen, "calls": 2});

        // the same task when the script already knows what to press and what to read (nothing is re-read)
        c.verb("goto", json!({"url": url})).expect("goto app");
        let t = Instant::now();
        let r = c
            .verb("batch", json!({"quiet": true, "steps": [
                {"verb": "fill", "params": {"target": "label~=Email", "text": "me@example.com"}},
                {"verb": "click", "params": {"target": "role=button name=Save"}},
                {"verb": "wait", "params": {"for": "selector", "value": "[role=status]"}},
                {"verb": "eval", "params": {"expr": "document.querySelector('[role=status]').innerText"}},
            ]}))
            .expect("batch known");
        out["task_known"] = json!({"ms": ms(t), "tokens": r["tokens"], "toast_seen": r.to_string().contains("Saved!"), "calls": 1, "quiet": true});

        // memory and idle wakeups
        std::thread::sleep(Duration::from_millis(1500));
        let (brss, _) = (0, 0);
        let _ = brss;
        let tree = c.verb("memory", json!({})).expect("memory");
        out["browser_rss_mb"] = tree["browser_rss_mb"].clone();
        out["driver_rss_mb"] = json!((rss_kb(dpid) as f64 / 1024.0 * 10.0).round() / 10.0);
        let a = ctx_switches(dpid);
        std::thread::sleep(Duration::from_secs(10));
        let b = ctx_switches(dpid);
        // a thread that ends takes its count with it: never let the difference go below zero
        out["idle_wakeups_per_s"] =
            json!((b.saturating_sub(a) as f64 / 10.0 * 100.0).round() / 100.0);
        out["driver_rss_after_idle_mb"] =
            json!((rss_kb(dpid) as f64 / 1024.0 * 10.0).round() / 10.0);
        let _ = bpid;
    }
    // the first command of a new client when the daemon and its browser already run (pre-warmed)
    let t = Instant::now();
    if let Ok(mut c2) = Client::connect(&sock)
        && c2.verb("goto", json!({"url": "about:blank"})).is_ok()
    {
        out["warm_first_ms"] = json!(ms(t));
    }
    let _ = c.verb("close", json!({}));
    let _ = daemon.kill();
    let _ = daemon.wait();
    println!("{out}");
}
