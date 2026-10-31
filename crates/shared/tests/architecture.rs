//! Workspace dependency rules (AGENTS.md): shared <- core <- {api, realtime, proxy, controller, wol-abi, wopkg, xguard}.
//! libwopkg (ADR 0021): `wopkg` is the package core's stable C ABI, an adaptor like `wol-abi`.
//! libwol (ADR 0020): `wol-abi` is the stable C ABI; `wol-lua` (Lua widgets) builds on it and nothing else may.
use std::collections::BTreeMap;
use std::process::Command;

const ADAPTORS: [&str; 7] = [
    "api",
    "realtime",
    "proxy",
    "controller",
    "wol-abi",
    "wopkg",
    "xguard",
];
const REQUIRED: [&str; 10] = [
    "shared",
    "core",
    "api",
    "realtime",
    "proxy",
    "controller",
    "wol-abi",
    "wol-lua",
    "wopkg",
    "xguard",
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
                p if ADAPTORS.contains(&p) => dep == "shared" || dep == "core",
                "wol-lua" => ["shared", "core", "wol-abi"].contains(&dep.as_str()),
                // wopkg checks what it is about to install (ADR 0024).
                "wopkg" => ["shared", "core", "xguard"].contains(&dep.as_str()),
                // The App Store links the package core directly (ADR 0021); the desktop runs Lua
                // widgets (ADR 0020).
                "shell" => ["shared", "core", "wopkg", "wol-lua"].contains(&dep.as_str()),
                // other members (e.g. shell) may use core + shared only
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
        (&[("wopkg", &["core", "shared"])], &[]),
        (&[("wopkg", &["api"])], &["wopkg -> api"]),
        (&[("wol-lua", &["wol-abi", "core", "shared"])], &[]),
        (&[("wol-lua", &["api"])], &["wol-lua -> api"]),
        (&[("shell", &["wol-abi"])], &["shell -> wol-abi"]),
        (&[("shell", &["wopkg", "core"])], &[]),
        (&[("shell", &["wol-lua"])], &[]),
    ];
    for (edges, want) in cases {
        assert_eq!(violations(&g(edges)), *want, "{edges:?}");
    }
    let mut missing = g(&[]);
    missing.remove("proxy");
    assert_eq!(violations(&missing), vec!["missing package proxy"]);
}
