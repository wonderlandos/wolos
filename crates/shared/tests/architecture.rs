// SPDX-License-Identifier: BSD-2-Clause
//! Workspace dependency rules (AGENTS.md): shared <- core <- {api, realtime, proxy, controller, wol-abi, wopkg, xguard, xguard-clam}.
//! libwopkg (ADR 0021): `wopkg` is the package core's stable C ABI, an adaptor like `wol-abi`.
//! libWolkit (ADR 0039): `wolkit` is the UI engine's stable C ABI, an adaptor like `wol-abi` that
//! depends on `shared` and `core` only (BSD-2-Clause), and no GPL crate may depend on it back.
//! libwol (ADR 0020): `wol-abi` is the stable C ABI; `wol-lua` (Lua widgets) builds on it and nothing else may.
//! Plugins (ADR 0031): `controller` may use `wol-lua` too, so `wol plugin check|run` runs plugin Lua headless.
//! Search by meaning: `wolsem` (BSD-2-Clause) is a leaf crate (no workspace dependency); `controller` and `shell` use it.
use std::collections::BTreeMap;
use std::process::Command;

const ADAPTORS: [&str; 8] = [
    "api",
    "realtime",
    "proxy",
    "controller",
    "wol-abi",
    "wolkit",
    "wopkg",
    "xguard",
];
const REQUIRED: [&str; 12] = [
    "shared",
    "core",
    "api",
    "realtime",
    "proxy",
    "controller",
    "wol-abi",
    "wolkit",
    "wol-lua",
    "wopkg",
    "xguard",
    "xguard-clam",
];

/// Rule violations for a package -> workspace-dependencies graph.
fn violations(graph: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let mut out = Vec::new();
    for req in REQUIRED {
        if !graph.contains_key(req) {
            out.push(format!("missing package {req}"));
        }
    }
    for (pkg, deps) in graph {
        for dep in deps {
            let ok = match pkg.as_str() {
                "shared" => false,
                "core" => dep == "shared",
                // `wol plugin check|run` loads and renders plugin Lua without a display (ADR 0031).
                // `wol help <words>` finds a command by meaning (wolsem, docs/modules/search.md).
                "controller" => ["shared", "core", "wol-lua", "wolsem"].contains(&dep.as_str()),
                p if ADAPTORS.contains(&p) => dep == "shared" || dep == "core",
                "wol-lua" => ["shared", "core", "wol-abi"].contains(&dep.as_str()),
                // Links only libc and libclamav: a GPL-2.0-only program stays alone (ADR 0028).
                "xguard-clam" => false,
                // wopkg checks what it is about to install (ADR 0024).
                "wopkg" => ["shared", "core", "xguard"].contains(&dep.as_str()),
                // The App Store links the package core directly (ADR 0021); the desktop runs Lua
                // widgets (ADR 0020); the Sound pane and the Control Center drive PipeWire through
                // the audio adaptor (wol_audio::pw::Handle).
                // Settings and the Search bar rank by meaning with wolsem (docs/modules/search.md).
                "shell" => ["shared", "core", "wopkg", "wol-lua", "audio", "wolsem"]
                    .contains(&dep.as_str()),
                // The web-app runtime (ADR 0042) paints into wolkit surface nodes and reads wolkit's
                // event records; BSD-2-Clause like wolkit, so the licence rule holds. It serves
                // fed:// through `federate` (ADR 0052), a BSD-2-Clause leaf with no workspace
                // dependency, which the browser's bridge links too.
                "webengine" => ["shared", "core", "wolkit", "federate"].contains(&dep.as_str()),
                // The Federate client (ADR 0052) uses wolcrypt (a BSD-2-Clause leaf, no workspace
                // dependency) for the hybrid TLS group and to seal its local state.
                "federate" => dep == "wolcrypt",
                // The render targets (wgpu, Vulkan, CPU) and the browser module (ADR 0060,
                // "wolkit on WebGPU and WASM"): BSD-2-Clause like wolkit; they build for
                // wasm32, where `core` does not, so they take `shared` and `wolkit` only.
                "wolkit-render" => ["shared", "wolkit"].contains(&dep.as_str()),
                "wolkit-web" => ["shared", "wolkit", "wolkit-render"].contains(&dep.as_str()),
                // other members (e.g. audio: shared::atomic for its config fragments) may use
                // core + shared only
                _ => dep == "shared" || dep == "core",
            };
            if !ok {
                out.push(format!("{pkg} -> {dep}"));
            }
        }
    }
    out
}

fn workspace_graph() -> BTreeMap<String, Vec<String>> {
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata");
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let pkgs = v["packages"].as_array().unwrap();
    let names: Vec<String> = pkgs
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_owned())
        .collect();
    pkgs.iter()
        .map(|p| {
            let deps = p["dependencies"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| d["name"].as_str().unwrap().to_owned())
                .filter(|d| names.contains(d))
                .collect();
            (p["name"].as_str().unwrap().to_owned(), deps)
        })
        .collect()
}

#[test]
fn workspace_follows_dependency_direction() {
    assert_eq!(violations(&workspace_graph()), Vec::<String>::new());
}

#[test]
fn violations_table() {
    let g = |edges: &[(&str, &[&str])]| -> BTreeMap<String, Vec<String>> {
        let mut m: BTreeMap<String, Vec<String>> =
            REQUIRED.iter().map(|r| (r.to_string(), vec![])).collect();
        for (p, ds) in edges {
            m.insert(p.to_string(), ds.iter().map(|d| d.to_string()).collect());
        }
        m
    };
    type Edges<'a> = &'a [(&'a str, &'a [&'a str])];
    let cases: &[(Edges, &[&str])] = &[
        (&[("core", &["shared"]), ("api", &["core", "shared"])], &[]),
        (&[("shared", &["core"])], &["shared -> core"]),
        (&[("core", &["api"])], &["core -> api"]),
        (&[("api", &["proxy"])], &["api -> proxy"]),
        (
            &[("realtime", &["controller"])],
            &["realtime -> controller"],
        ),
        (&[("shell", &["core", "shared"])], &[]),
        (&[("shell", &["api"])], &["shell -> api"]),
        (&[("wol-abi", &["core", "shared"])], &[]),
        (&[("wol-abi", &["api"])], &["wol-abi -> api"]),
        (&[("wolkit", &["core", "shared"])], &[]),
        (&[("wolkit", &["wol-abi"])], &["wolkit -> wol-abi"]),
        (&[("wolkit", &["api"])], &["wolkit -> api"]),
        (&[("shell", &["wolkit"])], &["shell -> wolkit"]),
        (&[("wol-abi", &["wolkit"])], &["wol-abi -> wolkit"]),
        (&[("wopkg", &["core", "shared"])], &[]),
        (&[("wopkg", &["api"])], &["wopkg -> api"]),
        (&[("wol-lua", &["wol-abi", "core", "shared"])], &[]),
        (&[("wol-lua", &["api"])], &["wol-lua -> api"]),
        (&[("shell", &["wol-abi"])], &["shell -> wol-abi"]),
        (&[("shell", &["wopkg", "core"])], &[]),
        (&[("shell", &["wol-lua"])], &[]),
        (&[("shell", &["audio"])], &[]),
        (&[("audio", &["shared", "core"])], &[]),
        (&[("audio", &["api"])], &["audio -> api"]),
        (&[("controller", &["wol-lua", "core", "shared"])], &[]),
        (&[("controller", &["wol-abi"])], &["controller -> wol-abi"]),
        (&[("controller", &["api"])], &["controller -> api"]),
        (&[("webengine", &["wolkit", "core", "shared"])], &[]),
        (&[("webengine", &["api"])], &["webengine -> api"]),
        (&[("wolkit-render", &["wolkit", "shared"])], &[]),
        (&[("wolkit-render", &["core"])], &["wolkit-render -> core"]),
        (
            &[("wolkit-web", &["wolkit-render", "wolkit", "shared"])],
            &[],
        ),
        (&[("wolkit-web", &["api"])], &["wolkit-web -> api"]),
        (
            &[("shell", &["wolkit-render"])],
            &["shell -> wolkit-render"],
        ),
    ];
    for (edges, want) in cases {
        assert_eq!(violations(&g(edges)), *want, "{edges:?}");
    }
    let mut missing = g(&[]);
    missing.remove("proxy");
    assert_eq!(violations(&missing), vec!["missing package proxy"]);
}
