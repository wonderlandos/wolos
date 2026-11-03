// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol modules` and `wol hw compute` against the recording fake (ADR 0032): a fake registry
//! directory, pacman's answer as a capture, wol-admind replies for install/remove, the user-side
//! enable step as recorded execs, and the shipped module records.
use controller::cli::parse;
use controller::hw::{parse_vram_mb, pick_gpu};
use controller::run::{FakeEnv, execute};
use serde_json::Value;
use shared::hardware::{Device, HardwareSnapshot, Kind};
use std::path::PathBuf;
use wol_core::modules::{IDS, Module};

const DIR: &str = "/fake/modules";

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

fn temp_home(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-modules-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn record(id: &str, name: &str, post: &[&str], recipes: &[&str]) -> (PathBuf, String) {
    let text = serde_json::json!({
        "id": id, "name": name, "summary": format!("{name} stack"), "category": "developer",
        "size_mb": 42, "packages": ["git"], "recipes": recipes, "post": post, "needs": ["network"],
    })
    .to_string();
    (PathBuf::from(format!("{DIR}/{id}.json")), text)
}

/// Two modules on disk, one broken record; pacman knows `wolos-module-dev`.
fn env(tag: &str, pacman: &str) -> FakeEnv {
    FakeEnv {
        home: temp_home(tag),
        tty: true,
        vars: vec![
            (controller::modules::DIR_VAR.into(), DIR.into()),
            (
                controller::modules::PENDING_VAR.into(),
                "/fake/modules-pending".into(),
            ),
        ],
        files: vec![
            record(
                "dev",
                "Developer",
                &["/usr/lib/wolos/modules-enable-dev"],
                &[],
            ),
            record("console", "Console", &[], &["powershell"]),
            (PathBuf::from(format!("{DIR}/broken.json")), "{".into()),
            (PathBuf::from("/fake/modules-pending"), "console\n".into()),
        ],
        captures: vec![("pacman -Qq".into(), pacman.into())],
        ..Default::default()
    }
}

fn json_out(env: &FakeEnv) -> Value {
    serde_json::from_str(&env.out).unwrap_or_else(|e| panic!("{e}: {}", env.out))
}

#[test]
fn list_is_a_table_on_a_tty_and_json_otherwise() {
    let mut e = env("list", "git\nwolos-module-dev\n");
    assert_eq!(run("modules", &mut e), 0);
    let lines: Vec<&str> = e.out.lines().collect();
    assert!(
        lines[0].starts_with("ID") && lines[0].contains("STATE"),
        "{}",
        e.out
    );
    assert!(lines[1].starts_with("console      pending"), "{}", e.out);
    assert!(lines[2].starts_with("dev          installed"), "{}", e.out);
    assert!(lines[3].contains("wol modules enable dev"), "{}", e.out);
    assert!(!e.out.contains("broken"));

    let mut e = env("listjson", "git\nwolos-module-dev\n");
    e.tty = false;
    assert_eq!(run("modules list", &mut e), 0);
    let v = json_out(&e);
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 2, "sorted by id, broken record skipped");
    assert_eq!(rows[0]["id"], "console");
    assert_eq!(
        (rows[0]["pending"].as_bool(), rows[0]["installed"].as_bool()),
        (Some(true), Some(false))
    );
    assert_eq!(rows[1]["id"], "dev");
    assert_eq!(rows[1]["package"], "wolos-module-dev");
    assert_eq!(
        (rows[1]["installed"].as_bool(), rows[1]["enabled"].as_bool()),
        (Some(true), Some(false))
    );
    assert_eq!(rows[1]["needs_enable"], true);
    assert_eq!(rows[1]["size_mb"], 42);
}

#[test]
fn status_of_one_module_and_unknown_ids() {
    let mut e = env("status", "wolos-module-dev\n");
    assert_eq!(run("modules status dev --json", &mut e), 0);
    let v = json_out(&e);
    assert_eq!(
        (v["id"].as_str(), v["installed"].as_bool()),
        (Some("dev"), Some(true))
    );
    assert_eq!(v["needs"], serde_json::json!(["network"]));

    let mut e = env("statustext", "");
    assert_eq!(run("modules status console", &mut e), 0);
    assert!(
        e.out.starts_with("Console (console): pending\n"),
        "{}",
        e.out
    );
    assert!(e.out.contains("recipes: powershell\n"));

    let mut e = env("statusall", "");
    assert_eq!(run("modules status --json", &mut e), 0);
    assert_eq!(
        json_out(&e).as_array().unwrap().len(),
        2,
        "no id = the list"
    );

    let mut e = env("unknown", "");
    assert_eq!(run("modules status nope", &mut e), 66);
    assert!(e.err.contains("no module 'nope'"), "{}", e.err);
    let mut e = env("badid", "");
    assert_eq!(run("modules install Bad", &mut e), 64);
    assert!(e.err.contains("bad module id"), "{}", e.err);
}

/// Install goes through wol-admind as `pkg-install wolos-module-<id>`, asking the password when the
/// daemon needs it; the user-side step is left to `enable` and named.
#[test]
fn install_sends_pkg_install_through_admind() {
    let mut e = env("install", "git\n");
    e.replies = vec!["auth\n".into(), "ok\n".into()];
    e.stdin = vec!["hunter22".into()];
    assert_eq!(run("modules install dev", &mut e), 0, "{}", e.err);
    assert_eq!(e.requests.len(), 2);
    let req = String::from_utf8_lossy(&e.requests[1].1).into_owned();
    assert!(
        req.starts_with("pkg-install\u{0}1\u{0}wolos-module-dev\u{0}"),
        "{req:?}"
    );
    assert!(
        req.ends_with("hunter22"),
        "the password goes to the daemon only"
    );
    assert!(
        e.out.contains("installed Developer (wolos-module-dev)"),
        "{}",
        e.out
    );
    assert!(
        e.out.contains("finish setup: wol modules enable dev"),
        "{}",
        e.out
    );
    assert!(e.execs.is_empty(), "no pacman from the user side");

    let mut e = env("installagain", "wolos-module-dev\n");
    assert_eq!(run("modules install dev", &mut e), 0);
    assert!(e.requests.is_empty(), "already installed: nothing asked");
    assert!(e.out.contains("already installed"));

    let mut e = env("installdenied", "");
    e.replies = vec!["auth\n".into(), "denied\n".into()];
    e.stdin = vec!["wrong".into()];
    assert_ne!(run("modules install dev", &mut e), 0);
    assert!(!e.out.contains("installed Developer"));

    let mut e = env("noadmind", "");
    assert_eq!(run("modules install dev", &mut e), 69, "{}", e.err);
    assert!(e.err.contains("wol-admind"));
}

/// Enable runs the user-side steps in order, records the module as enabled, and is repeatable.
#[test]
fn enable_runs_recipes_and_post_as_the_user_and_records_it() {
    let mut e = env("enable", "wolos-module-dev\nwolos-module-console\n");
    assert_eq!(run("modules enable dev --json", &mut e), 0, "{}", e.err);
    assert_eq!(e.execs, [["/usr/lib/wolos/modules-enable-dev"]]);
    let v = json_out(&e);
    assert_eq!(v["enabled"], true);
    assert_eq!(
        v["ran"],
        serde_json::json!(["/usr/lib/wolos/modules-enable-dev"])
    );
    let state = e.home.join("state/wolos/modules.json");
    assert_eq!(e.written.len(), 1);
    assert_eq!(e.written[0].0, state);
    assert!(e.written[0].1.contains("\"dev\""), "{}", e.written[0].1);
    assert!(e.requests.is_empty(), "never wol-admind");

    // A recipe module: wopkg as the user, and the state file now lists both.
    e.files.push((state.clone(), e.written[0].1.clone()));
    e.out.clear();
    e.execs.clear();
    e.written.clear();
    assert_eq!(run("modules enable console", &mut e), 0, "{}", e.err);
    assert_eq!(e.execs, [["wopkg", "install", "powershell"]]);
    assert!(e.out.contains("Console is set up"), "{}", e.out);
    let saved: Value = serde_json::from_str(&e.written[0].1).unwrap();
    assert_eq!(saved["enabled"], serde_json::json!(["console", "dev"]));

    // Already enabled: steps run again (idempotent by contract), nothing rewritten.
    e.files.retain(|f| f.0 != state);
    e.files.push((state, e.written[0].1.clone()));
    e.execs.clear();
    e.written.clear();
    assert_eq!(run("modules enable console", &mut e), 0);
    assert_eq!(e.execs.len(), 1);
    assert!(e.written.is_empty(), "state unchanged");
}

#[test]
fn enable_refuses_what_is_not_installed_and_reports_a_failed_step() {
    let mut e = env("notinstalled", "git\n");
    assert_eq!(run("modules enable dev", &mut e), 66);
    assert!(e.err.contains("wol modules install dev"), "{}", e.err);
    assert!(e.execs.is_empty() && e.written.is_empty());

    let mut e = env("stepfails", "wolos-module-console\n");
    e.exec_codes = vec![3];
    assert_eq!(run("modules enable console", &mut e), 1);
    assert!(
        e.err.contains("wopkg install powershell") && e.err.contains("exit 3"),
        "{}",
        e.err
    );
    assert!(
        e.written.is_empty(),
        "a failed step is not recorded as enabled"
    );
}

/// Remove: `pkg-remove` through wol-admind, then the user-side parts and the enablement record.
#[test]
fn remove_sends_pkg_remove_and_forgets_the_user_side() {
    let mut e = env("remove", "wolos-module-console\n");
    let state = e.home.join("state/wolos/modules.json");
    e.files
        .push((state.clone(), r#"{"enabled":["console","dev"]}"#.into()));
    e.replies = vec!["ok\n".into()];
    assert_eq!(run("modules remove console --json", &mut e), 0, "{}", e.err);
    let req = String::from_utf8_lossy(&e.requests[0].1).into_owned();
    assert!(
        req.starts_with("pkg-remove\u{0}1\u{0}wolos-module-console\u{0}"),
        "{req:?}"
    );
    assert_eq!(e.execs, [["wopkg", "remove", "powershell"]]);
    let saved: Value = serde_json::from_str(&e.written[0].1).unwrap();
    assert_eq!(saved["enabled"], serde_json::json!(["dev"]));
    assert_eq!(json_out(&e)["removed"], true);

    let mut e = env("removeabsent", "git\n");
    assert_eq!(run("modules remove console", &mut e), 0);
    assert!(e.requests.is_empty() && e.execs.is_empty());
    assert!(e.out.contains("was not installed"), "{}", e.out);
}

/// Audit S4-9: a recipe that `wopkg remove` cannot take off fails the verb, says so in JSON and
/// keeps the enablement record so the removal can be run again.
#[test]
fn a_failed_recipe_removal_fails_and_keeps_the_record() {
    let mut e = env("removefails", "wolos-module-console\n");
    let state = e.home.join("state/wolos/modules.json");
    e.files
        .push((state, r#"{"enabled":["console","dev"]}"#.into()));
    e.replies = vec!["ok\n".into()];
    e.exec_codes = vec![1];
    assert_eq!(run("modules remove console --json", &mut e), 1, "{}", e.err);
    let v = json_out(&e);
    assert_eq!(v["removed"], false);
    assert_eq!(v["failed"], serde_json::json!(["powershell"]));
    assert!(e.written.is_empty(), "the record stays: {:?}", e.written);

    // wopkg missing (exec error) is the same failure, in text too.
    let mut e = env("removenowopkg", "");
    let state = e.home.join("state/wolos/modules.json");
    e.files.push((state, r#"{"enabled":["console"]}"#.into()));
    e.exec_codes = vec![127];
    assert_eq!(run("modules remove console", &mut e), 1);
    assert!(e.out.contains("only partly removed"), "{}", e.out);
}

/// Audit S4-9: `modules install --json`, and any run whose stdout is not a terminal, prints JSON.
#[test]
fn install_honours_json_and_non_tty() {
    let mut e = env("installjson", "git\n");
    e.replies = vec!["ok\n".into()];
    assert_eq!(run("modules install dev --json", &mut e), 0, "{}", e.err);
    let v = json_out(&e);
    assert_eq!(v["id"], "dev");
    assert_eq!(v["installed"], true);
    assert_eq!(v["was_installed"], false);
    assert_eq!(v["needs_enable"], true);

    let mut e = env("installpipe", "wolos-module-dev\n");
    e.tty = false;
    assert_eq!(run("modules install dev", &mut e), 0, "{}", e.err);
    let v = json_out(&e);
    assert_eq!(v["was_installed"], true);
    assert!(e.requests.is_empty());
}

fn gpu(name: &str, id: &str, klds: &[&str], location: &str) -> Device {
    Device {
        kind: Kind::Gpu,
        name: name.into(),
        id: id.into(),
        location: location.into(),
        klds: klds.iter().map(|s| (*s).to_owned()).collect(),
        ..Default::default()
    }
}

fn snap(devices: Vec<Device>) -> HardwareSnapshot {
    HardwareSnapshot { devices }
}

/// The catalogue decides: NVIDIA with the open driver = cuda, AMD Vega+ = rocm, GCN 1-4 and Intel =
/// vulkan, a virtual or no GPU = cpu. The discrete card wins over an integrated one.
#[test]
fn compute_backend_from_the_catalogue() {
    let nv = gpu(
        "NVIDIA GeForce",
        "10de:2684",
        &["nvidia", "nvidia_drm"],
        "0000:01:00.0",
    );
    let old_nv = gpu(
        "NVIDIA GeForce (Maxwell or older)",
        "10de:1c82",
        &["nouveau"],
        "0000:01:00.0",
    );
    let vega = gpu("AMD Radeon", "1002:687f", &["amdgpu"], "0000:03:00.0");
    let polaris = gpu("AMD Radeon", "1002:67df", &["amdgpu"], "0000:03:00.0");
    let apu = gpu("AMD Radeon", "1002:1681", &["amdgpu"], "0000:04:00.0");
    let intel = gpu(
        "Intel Iris Xe (Tiger Lake)",
        "8086:9a49",
        &["i915"],
        "0000:00:02.0",
    );
    let virt = gpu(
        "QEMU standard VGA",
        "1234:1111",
        &["bochs-drm"],
        "0000:00:01.0",
    );
    let cpu = Device {
        kind: Kind::Cpu,
        klds: vec!["nvidia".into()],
        ..Default::default()
    };
    for (devs, want) in [
        (vec![nv.clone()], Some("cuda")),
        (vec![intel.clone(), nv.clone()], Some("cuda")),
        (vec![old_nv], Some("vulkan")),
        (vec![vega], Some("rocm")),
        (vec![apu], Some("rocm")),
        (vec![polaris], Some("vulkan")),
        (vec![intel.clone()], Some("vulkan")),
        (vec![virt], None),
        (vec![cpu], None),
        (vec![], None),
    ] {
        let s = snap(devs);
        assert_eq!(
            pick_gpu(&s).map(|x| x.1),
            want,
            "{:?}",
            s.devices.iter().map(|d| &d.name).collect::<Vec<_>>()
        );
    }
    assert_eq!(
        pick_gpu(&snap(vec![intel, nv])).unwrap().0.name,
        "NVIDIA GeForce"
    );
    assert_eq!(parse_vram_mb(Some("8192\n"), None), 8192);
    assert_eq!(parse_vram_mb(None, Some("17163091968\n")), 16368);
    assert_eq!(parse_vram_mb(Some("N/A"), Some("x")), 0);
    assert_eq!(parse_vram_mb(None, None), 0);
}

#[test]
fn hw_compute_json_reads_vram_from_nvidia_smi_or_sysfs() {
    let mut e = FakeEnv {
        hw: vec![snap(vec![gpu(
            "NVIDIA GeForce",
            "10de:2684",
            &["nvidia"],
            "0000:01:00.0",
        )])],
        captures: vec![(
            "nvidia-smi --query-gpu=memory.total --format=csv,noheader,nounits".into(),
            "24564\n".into(),
        )],
        ..Default::default()
    };
    assert_eq!(run("hw compute --json", &mut e), 0);
    assert_eq!(
        json_out(&e),
        serde_json::json!({"backend": "cuda", "vram_mb": 24564, "gpu": "NVIDIA GeForce"})
    );

    let mut e = FakeEnv {
        hw: vec![snap(vec![gpu(
            "AMD Radeon",
            "1002:73bf",
            &["amdgpu"],
            "0000:03:00.0",
        )])],
        files: vec![(
            PathBuf::from("/sys/bus/pci/devices/0000:03:00.0/mem_info_vram_total"),
            "17163091968\n".into(),
        )],
        ..Default::default()
    };
    assert_eq!(run("hw compute --json", &mut e), 0);
    let v = json_out(&e);
    assert_eq!(
        (v["backend"].as_str(), v["vram_mb"].as_u64()),
        (Some("rocm"), Some(16368))
    );

    let mut e = FakeEnv {
        tty: true,
        ..Default::default()
    };
    assert_eq!(run("hw compute", &mut e), 0);
    assert_eq!(e.out, "backend: cpu\nvram_mb: 0\ngpu: \n");
}

/// The shipped records (wol_core::modules::IDS) parse, carry the ADR's fields and match their file names.
#[test]
fn shipped_module_records_are_valid() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../files/usr/share/wolos/modules");
    let mut seen = Vec::new();
    for id in IDS {
        let path = dir.join(format!("{id}.json"));
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let m = Module::parse(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(m.id, id);
        assert!(
            !m.summary.is_empty() && !m.category.is_empty() && !m.icon.is_empty(),
            "{id}"
        );
        assert!(m.size_mb > 0, "{id}: size_mb");
        assert!(
            !m.packages.is_empty(),
            "{id}: a module is a meta package with dependencies"
        );
        for p in m.packages.iter().chain(&m.recipes).chain(&m.flatpaks) {
            assert!(
                p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_+@".contains(&b)),
                "{id}: {p}"
            );
        }
        if !m.post.is_empty() {
            assert!(
                m.post[0].starts_with("/usr/lib/wolos/") && !m.post[0].contains(".."),
                "{id}: post is a WolOS script"
            );
            // M1: the script ships in files/ (wolos-system copies it and makes it 755) and follows
            // docs/STANDARDS.md section 2.
            let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../files")
                .join(m.post[0].trim_start_matches('/'));
            let body = std::fs::read_to_string(&script)
                .unwrap_or_else(|e| panic!("{id}: post {} not shipped: {e}", m.post[0]));
            let mut lines = body.lines();
            assert_eq!(lines.next(), Some("#!/bin/sh"), "{id}: {}", m.post[0]);
            assert_eq!(
                lines.next(),
                Some("# SPDX-License-Identifier: GPL-2.0-or-later"),
                "{id}: {}",
                m.post[0]
            );
            assert!(
                body.lines().any(|l| l == "set -eu"),
                "{id}: {} lacks set -eu",
                m.post[0]
            );
        }
        assert!(
            ["ai", "developer", "creative", "gaming", "console"].contains(&m.category.as_str()),
            "{id}"
        );
        seen.push(id);
    }
    let on_disk: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter_map(|e| {
            e.path()
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .collect();
    assert_eq!(on_disk.len(), seen.len(), "{on_disk:?}");
    let ai = Module::parse(&std::fs::read_to_string(dir.join("ai-claude.json")).unwrap()).unwrap();
    assert_eq!(ai.recipes, ["claude-code"]);
    let console =
        Module::parse(&std::fs::read_to_string(dir.join("console.json")).unwrap()).unwrap();
    assert!(console.recipes.contains(&"powershell".to_string()));
    for r in ai.recipes.iter().chain(&console.recipes) {
        let recipe = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../wopkg/recipes/{r}/recipe.toml"));
        assert!(recipe.is_file(), "{}", recipe.display());
    }
    let local =
        Module::parse(&std::fs::read_to_string(dir.join("ai-local.json")).unwrap()).unwrap();
    assert!(local.packages.contains(&"ollama".to_string()));
    assert!(
        local.packages.iter().all(|p| !p.starts_with("ollama-")),
        "backend chosen per machine from wol hw compute, never a depends="
    );
    assert_eq!(local.backend.as_deref(), Some("ollama"));
    assert_eq!(
        local.backend_package("rocm").as_deref(),
        Some("ollama-rocm")
    );
    // Gaming adds what the base lacks; Steam and GameMode are already in the image.
    let gaming = Module::parse(&std::fs::read_to_string(dir.join("gaming.json")).unwrap()).unwrap();
    assert_eq!(gaming.category, "gaming");
    assert!(gaming.packages.iter().any(|p| p == "wine"));
    assert!(
        !gaming
            .packages
            .iter()
            .any(|p| p == "steam" || p == "gamemode"),
        "base packages are not module dependencies"
    );
}

/// A record with a `backend` (ai-local): the per-machine package goes with the meta package, or
/// alone when only it is missing; a CPU-only machine adds nothing.
#[test]
fn install_adds_the_records_backend_package_for_this_machine() {
    let local = |e: &mut FakeEnv| {
        let text = serde_json::json!({
            "id": "ai-local", "name": "Local AI", "summary": "Ollama", "category": "ai",
            "size_mb": 1, "packages": ["ollama"], "backend": "ollama", "needs": ["network"],
        })
        .to_string();
        e.files
            .push((PathBuf::from(format!("{DIR}/ai-local.json")), text));
        e.hw = vec![snap(vec![gpu(
            "AMD Radeon",
            "1002:687f",
            &["amdgpu"],
            "0000:03:00.0",
        )])];
    };
    let sent = |e: &FakeEnv| -> Vec<String> {
        e.requests
            .iter()
            .map(|r| String::from_utf8_lossy(&r.1).into_owned())
            .collect()
    };
    let mut e = env("backendboth", "git\n");
    local(&mut e);
    e.replies = vec!["ok\n".into(), "ok\n".into()];
    assert_eq!(
        run("modules install ai-local --json", &mut e),
        0,
        "{}",
        e.err
    );
    let r = sent(&e);
    assert!(
        r[0].starts_with("pkg-install\u{0}1\u{0}wolos-module-ai-local\u{0}"),
        "{r:?}"
    );
    assert!(
        r[1].starts_with("pkg-install\u{0}1\u{0}ollama-rocm\u{0}"),
        "{r:?}"
    );
    assert_eq!(json_out(&e)["backend"], "ollama-rocm");

    let mut e = env("backendonly", "wolos-module-ai-local\n");
    local(&mut e);
    e.replies = vec!["ok\n".into()];
    assert_eq!(run("modules install ai-local", &mut e), 0, "{}", e.err);
    let r = sent(&e);
    assert_eq!(r.len(), 1, "{r:?}");
    assert!(
        r[0].starts_with("pkg-install\u{0}1\u{0}ollama-rocm\u{0}"),
        "{r:?}"
    );
    assert!(
        e.out.contains("installed ollama-rocm for Local AI"),
        "{}",
        e.out
    );

    let mut e = env("backendhave", "wolos-module-ai-local\nollama-rocm\n");
    local(&mut e);
    assert_eq!(run("modules install ai-local", &mut e), 0, "{}", e.err);
    assert!(
        e.requests.is_empty() && e.out.contains("already installed"),
        "{}",
        e.out
    );

    let mut e = env("backendcpu", "git\n");
    local(&mut e);
    e.hw = vec![snap(vec![])];
    e.replies = vec!["ok\n".into()];
    assert_eq!(run("modules install ai-local", &mut e), 0, "{}", e.err);
    assert_eq!(sent(&e).len(), 1, "CPU only: the meta package alone");
}

/// Settings installs the record's backend package with the meta package, so `list --json` and
/// `status --json` name it for this machine; a CPU-only machine and a record without one say
/// nothing (the key is absent).
#[test]
fn list_and_status_json_name_the_backend_package_for_this_machine() {
    let text = serde_json::json!({
        "id": "ai-local", "name": "Local AI", "summary": "Ollama", "category": "ai",
        "size_mb": 1, "packages": ["ollama"], "backend": "ollama", "needs": ["network"],
    })
    .to_string();
    let setup = |tag: &str, devices: Vec<Device>| {
        let mut e = env(tag, "git\n");
        e.files
            .push((PathBuf::from(format!("{DIR}/ai-local.json")), text.clone()));
        e.hw = vec![snap(devices)];
        e
    };
    let amd = || vec![gpu("AMD Radeon", "1002:687f", &["amdgpu"], "0000:03:00.0")];
    let mut e = setup("listbackend", amd());
    assert_eq!(run("modules list --json", &mut e), 0, "{}", e.err);
    let rows = json_out(&e);
    let local = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "ai-local")
        .unwrap();
    assert_eq!(local["backend"], "ollama-rocm");
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .filter(|r| r["id"] != "ai-local")
            .all(|r| r.get("backend").is_none()),
        "{rows}"
    );

    let mut e = setup("statusbackend", amd());
    assert_eq!(
        run("modules status ai-local --json", &mut e),
        0,
        "{}",
        e.err
    );
    assert_eq!(json_out(&e)["backend"], "ollama-rocm");

    let mut e = setup("cpubackend", vec![]);
    assert_eq!(
        run("modules status ai-local --json", &mut e),
        0,
        "{}",
        e.err
    );
    assert!(json_out(&e).get("backend").is_none());
}

/// Audit C8-6: `remove` undoes all of `enable` and `install`. The user side runs in reverse
/// (`unpost`, Flatpaks, mise tools, recipes), then the installed backend package (installed
/// explicitly next to the meta package) and the meta package come off: two PkgRemove requests.
#[test]
fn remove_takes_off_the_backend_and_undoes_every_enable_step() {
    let mut e = env("removeall", "wolos-module-ai-local\nollama-cuda\ngit\n");
    let text = serde_json::json!({
        "id": "ai-local", "name": "Local AI", "summary": "Ollama", "category": "ai",
        "size_mb": 1, "packages": ["ollama"], "backend": "ollama", "needs": ["network"],
        "recipes": ["r1"], "mise": ["node@22"], "flatpaks": ["org.example.App"],
        "post": ["/usr/lib/wolos/ai-local"], "unpost": ["/usr/lib/wolos/ai-local-off"],
    })
    .to_string();
    e.files
        .push((PathBuf::from(format!("{DIR}/ai-local.json")), text));
    let state = e.home.join("state/wolos/modules.json");
    e.files.push((state, r#"{"enabled":["ai-local"]}"#.into()));
    e.replies = vec!["ok\n".into(), "ok\n".into()];
    assert_eq!(
        run("modules remove ai-local --json", &mut e),
        0,
        "{}",
        e.err
    );
    let sent: Vec<String> = e
        .requests
        .iter()
        .map(|r| String::from_utf8_lossy(&r.1).into_owned())
        .collect();
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(
        sent[0].starts_with("pkg-remove\u{0}1\u{0}ollama-cuda\u{0}"),
        "{sent:?}"
    );
    assert!(
        sent[1].starts_with("pkg-remove\u{0}1\u{0}wolos-module-ai-local\u{0}"),
        "{sent:?}"
    );
    let want: Vec<Vec<&str>> = vec![
        vec!["/usr/lib/wolos/ai-local-off"],
        vec![
            "flatpak",
            "uninstall",
            "--user",
            "-y",
            "--noninteractive",
            "org.example.App",
        ],
        vec!["mise", "unuse", "-g", "node@22"],
        vec!["wopkg", "remove", "r1"],
    ];
    assert_eq!(e.execs, want);
    let v = json_out(&e);
    assert_eq!(v["removed"], true);
    assert_eq!(v["backends"], serde_json::json!(["ollama-cuda"]));
    let saved: Value = serde_json::from_str(&e.written[0].1).unwrap();
    assert_eq!(saved["enabled"], serde_json::json!([]));

    // The meta package already gone, the backend still there: remove still takes it off.
    let mut e = env("removebackend", "ollama-vulkan\n");
    let text = serde_json::json!({
        "id": "ai-local", "name": "Local AI", "summary": "Ollama", "category": "ai",
        "size_mb": 1, "packages": ["ollama"], "backend": "ollama", "needs": ["network"],
    })
    .to_string();
    e.files
        .push((PathBuf::from(format!("{DIR}/ai-local.json")), text));
    e.replies = vec!["ok\n".into()];
    assert_eq!(run("modules remove ai-local", &mut e), 0, "{}", e.err);
    assert_eq!(e.requests.len(), 1);
    assert!(e.out.contains("removed ollama-vulkan"), "{}", e.out);
}

/// The shipped records that `post` something also undo it (`unpost`), and the undo script exists.
#[test]
fn shipped_records_with_side_effects_have_an_unpost() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (id, script) in [
        ("ai-claude", "modules-disable-ai-claude"),
        ("dev", "modules-disable-dev"),
    ] {
        let p = root.join(format!("files/usr/share/wolos/modules/{id}.json"));
        let m = Module::parse(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(m.unpost, [format!("/usr/lib/wolos/{script}")], "{id}");
        assert!(root.join("files/usr/lib/wolos").join(script).is_file());
    }
}
