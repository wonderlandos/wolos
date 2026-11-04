// SPDX-License-Identifier: BSD-2-Clause
//! System web apps (ADR 0048): every `webapps/*/app.toml` and its permissions against the catalog,
//! the kit's catalog copy, fixtures and tokens against their Rust and design sources, every method
//! against the `wol` verbs, and (with `CHROME` set to a headless browser) the starter app in
//! development mode, screenshotted light and dark into `WOLKIT_SHOTS` (default: a temp folder).
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use webengine::bridge::catalog::{self, PERMISSIONS};
use webengine::bridge::manifest::{KIT, Manifest};
use webengine::bridge::mock::Fixtures;
use webengine::bridge::{AppInfo, Bridge, Step, Theme, Wold};
use webengine::sandbox::{Gate, Load};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn webapps() -> PathBuf {
    repo().join("webapps")
}
fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}
/// Every app folder (not the kit).
fn apps() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(webapps())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir() && p.file_name().unwrap() != KIT)
        .collect();
    v.sort();
    assert!(!v.is_empty(), "no web apps");
    v
}
fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            files(&p, out);
        } else {
            out.push(p);
        }
    }
}
fn kit_fixtures() -> Fixtures {
    Fixtures::parse(&read(&webapps().join(KIT).join("fixtures.js"))).unwrap()
}

#[test]
fn every_manifest_is_valid_and_names_only_catalog_permissions() {
    let mut ids = BTreeSet::new();
    for dir in apps() {
        let m = Manifest::load(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        assert!(ids.insert(m.id.clone()), "{} twice", m.id);
        for p in &m.permissions {
            assert!(catalog::permission(p).is_some(), "{}: {p}", m.id);
        }
    }
}

#[test]
fn every_method_is_a_wol_verb() {
    // controller is GPL and not a dependency: its command tree is read as text (`cli::TREE`).
    let cli = read(&repo().join("crates/controller/src/cli.rs"));
    let actions: BTreeSet<&str> = cli
        .split("\"")
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|w| w[0].trim_end().ends_with("],") || w[0].contains("action: "))
        .map(|w| w[1])
        .collect();
    assert!(actions.len() > 100 && actions.contains("audio.doctor") && !actions.contains("audio"));
    for p in PERMISSIONS {
        for m in p.methods {
            assert!(
                actions.contains(m),
                "{}: {m} is no `wol` verb action",
                p.name
            );
        }
    }
}

#[test]
fn the_mock_catalog_is_the_rust_catalog() {
    let js = read(&webapps().join(KIT).join("mock-bridge.js"));
    let a = js.find("/*catalog*/").unwrap() + "/*catalog*/".len();
    let b = js.find("/*end*/").unwrap();
    let mock: BTreeMap<String, Vec<String>> = serde_json::from_str(&js[a..b]).unwrap();
    let rust: BTreeMap<String, Vec<String>> = PERMISSIONS
        .iter()
        .map(|p| {
            (
                p.name.to_owned(),
                p.methods.iter().map(|m| m.to_string()).collect(),
            )
        })
        .collect();
    assert_eq!(mock, rust);
    for t in catalog::TOPICS {
        assert!(js.contains(&format!("\"{t}\"")), "mock topic {t}");
    }
}

#[test]
fn fixtures_cover_the_catalog_and_match_each_manifest() {
    let kit = kit_fixtures();
    for p in PERMISSIONS {
        for m in p.methods {
            assert!(kit.calls.contains_key(*m), "kit fixtures lack {m}");
        }
    }
    for k in kit.calls.keys() {
        assert!(
            catalog::permission_of(k).is_some(),
            "fixture {k} is no method"
        );
    }
    assert!(kit.permissions.is_none(), "the kit declares no permissions");
    for dir in apps() {
        let f = dir.join("fixtures.js");
        if !f.exists() {
            continue;
        }
        let own = Fixtures::parse(&read(&f)).unwrap();
        let m = Manifest::load(&dir).unwrap();
        for k in own.calls.keys() {
            assert!(catalog::permission_of(k).is_some(), "{}: fixture {k}", m.id);
        }
        if let Some(p) = &own.permissions {
            assert_eq!(p, &m.permissions, "{}: fixtures.js permissions", m.id);
        }
    }
}

#[test]
fn a_bridge_over_the_fixtures_answers_like_the_mock() {
    let dir = webapps().join("hello");
    let m = Manifest::load(&dir).unwrap();
    let app = AppInfo {
        id: m.id.clone(),
        version: m.version.clone(),
        locale: "en".into(),
        theme: Theme::Dark,
        accent: "violet".into(),
    };
    let granted = m.permissions.clone();
    let mut b = Bridge::new(m, app, &granted);
    let wold = kit_fixtures();
    let Step::Call(c) = b.handle(r#"{"id":7,"call":"audio.doctor","params":{}}"#) else {
        panic!("audio.doctor refused")
    };
    let r: Value = serde_json::from_str(&c.run(&b.app().id, &wold)).unwrap();
    assert!(r["ok"]["checks"].as_array().is_some_and(|c| !c.is_empty()));
    let Step::Reply(r) = b.handle(r#"{"id":8,"call":"audio.reset"}"#) else {
        panic!("audio.reset passed undeclared")
    };
    assert!(r.contains("\"denied\""));
    assert!(wold.call("x", "sys", &Default::default()).is_ok());
}

#[test]
fn the_policy_lets_the_app_reach_its_folder_and_the_kit_only() {
    let dir = Path::new("/usr/share/wolos/webapps/hello");
    let m = Manifest::parse(&read(&webapps().join("hello/app.toml"))).unwrap();
    let p = m.policy(dir, Path::new("/usr/share/wolos/webapps/kit"));
    let g = Gate::new(
        &p,
        Path::new("/home/a/.local/share/wolos/webapps/profiles/hello"),
    )
    .unwrap();
    for ok in [
        "file:///usr/share/wolos/webapps/hello/index.html",
        "file:///usr/share/wolos/webapps/kit/wolkit.css",
    ] {
        assert!(g.allows(ok, Load::Resource), "{ok}");
    }
    for no in [
        "file:///usr/share/wolos/webapps/other/index.html",
        "file:///etc/passwd",
        "https://example.com/",
    ] {
        assert!(!g.allows(no, Load::Resource), "{no}");
    }
}

#[test]
fn the_kit_is_small_plain_and_self_contained() {
    let mut all = Vec::new();
    files(&webapps(), &mut all);
    let mut kit = 0;
    for f in &all {
        let text = read(f).replace("http://www.w3.org/2000/svg", "");
        let rel = f.strip_prefix(webapps()).unwrap().display().to_string();
        if rel.starts_with("kit/") {
            kit += text.len();
        }
        // The kit is BSD-2-Clause (any app may ship it); an app names its own licence. JSON (a
        // recorded answer under fixtures/) has no comments: its app's fixtures.js carries it.
        let spdx = if rel.starts_with("kit/") {
            "SPDX-License-Identifier: BSD-2-Clause"
        } else {
            "SPDX-License-Identifier: "
        };
        assert!(
            text.contains(spdx) || rel.ends_with(".svg") || rel.ends_with(".json"),
            "{rel}: SPDX"
        );
        // Nothing is fetched from the network: no remote script, style, font or image. An app's
        // `test/` folder is a development file (node and a browser), never installed.
        if rel.split('/').nth(1) == Some("test") {
            continue;
        }
        for bad in [
            "http://", "https://", "@import", "import(", " from \"", "fetch(",
        ] {
            assert!(!text.contains(bad), "{rel}: {bad}");
        }
    }
    assert!(kit < 60 * 1024, "kit is {kit} bytes");
}

#[test]
fn colours_are_the_golden_tokens_and_every_accent_is_themed() {
    let css = read(&webapps().join(KIT).join("wolkit.css"));
    let tokens = repo().join("design/tokens/colors.json");
    if let Ok(text) = std::fs::read_to_string(&tokens) {
        let v: Value = serde_json::from_str(&text).unwrap();
        let block = |sel: &str| {
            let a = css.find(sel).unwrap_or_else(|| panic!("{sel}"));
            css[a..a + css[a..].find('}').unwrap()].to_owned()
        };
        let light = block(":root {");
        let dark = block(":root[data-theme=\"dark\"] {");
        for k in [
            "bg",
            "surface",
            "surface-2",
            "border",
            "text",
            "text-2",
            "red",
            "orange",
            "yellow",
            "green",
            "blue",
            "accent",
            "selection",
        ] {
            for (scheme, b) in [("light", &light), ("dark", &dark)] {
                let want = format!("--wol-{k}: {};", v[scheme][k].as_str().unwrap());
                assert!(b.contains(&want), "{scheme}: {want}");
            }
        }
    }
    for (id, _, light, dark) in wol_core::theme::accent::ACCENTS {
        if id == "violet" {
            continue;
        }
        assert!(
            css.contains(&format!(
                ":root[data-accent=\"{id}\"] {{ --wol-accent: {light};"
            )),
            "{id} light"
        );
        assert!(
            css.contains(&format!(
                ":root[data-theme=\"dark\"][data-accent=\"{id}\"] {{ --wol-accent: {dark};"
            )),
            "{id} dark"
        );
    }
}

/// Keys of one strings table (`"key": "text",` lines).
fn keys(js: &str) -> BTreeSet<String> {
    js.lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix('"')?
                .split_once("\":")
                .map(|(k, _)| k.to_owned())
        })
        .collect()
}

#[test]
fn every_string_an_app_shows_is_in_its_table() {
    let kit = keys(&read(&webapps().join(KIT).join("wolkit.js")));
    for dir in apps() {
        let own = keys(&read(&dir.join("strings.js")));
        let mut used = BTreeSet::new();
        let html = read(&dir.join("index.html"));
        for part in html.split("data-t").skip(1) {
            if let Some(k) = part.split('"').nth(1) {
                used.insert(k.to_owned());
            }
        }
        let js = read(&dir.join("app.js"));
        let calls: Vec<&str> = js.split("t(\"").collect();
        for w in calls.windows(2) {
            // `t("key")`, not `createElement("code")`.
            if w[0].ends_with(|c: char| !c.is_alphanumeric() && c != '_' && c != '.') {
                used.insert(w[1].split('"').next().unwrap().to_owned());
            }
        }
        for k in used {
            // `t("area." + v)`: a prefix, so some key starts with it.
            if k.ends_with('.') {
                assert!(
                    own.iter().chain(&kit).any(|o| o.starts_with(&k)),
                    "{}: no string starts with {k:?}",
                    dir.display()
                );
                continue;
            }
            assert!(
                own.contains(&k) || kit.contains(&k),
                "{}: string {k:?} has no entry",
                dir.display()
            );
        }
    }
}

/// The starter app in a headless browser, development mode: rendered from the fixtures, a missing
/// grant shown in words, light and dark screenshots. Skipped without `CHROME`.
#[test]
fn hello_runs_in_a_browser_against_the_mock_bridge() {
    let Ok(browser) = std::env::var("CHROME") else {
        eprintln!("CHROME not set: skipped");
        return;
    };
    let out = std::env::var_os("WOLKIT_SHOTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("wolkit-shots"));
    std::fs::create_dir_all(&out).unwrap();
    let page = webapps().join("hello/index.html").canonicalize().unwrap();
    let url = |q: &str| format!("file://{}?dev&delay=0{q}", page.display());
    let run = |args: &[String]| {
        let o = Command::new(&browser)
            .args([
                "--headless",
                "--password-store=basic",
                "--use-mock-keychain",
                "--virtual-time-budget=3000",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        (
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    let (dom, log) = run(&["--dump-dom".into(), url("#sound")]);
    assert!(
        dom.contains("Speakers and a microphone are ready"),
        "sound checks not drawn"
    );
    assert!(
        dom.contains("Plugged in") && dom.contains("System disk"),
        "overview or storage not drawn"
    );
    assert!(!dom.contains("Could not load this"), "a load failed");
    assert!(dom.contains("data-theme=\"light\""));
    assert!(!log.contains("Uncaught"), "page error: {log}");
    let (dom, _) = run(&[
        "--dump-dom".into(),
        url("&deny=audio.read&theme=dark#sound"),
    ]);
    assert!(
        dom.contains("This app is not allowed to do that. Turn it on in Settings &gt; Privacy.")
    );
    assert!(dom.contains("data-theme=\"dark\""));
    let mut shots = Vec::new();
    for theme in ["light", "dark"] {
        for pg in ["overview", "storage", "sound"] {
            let f = out.join(format!("hello-{pg}-{theme}.png"));
            let _ = std::fs::remove_file(&f);
            run(&[
                "--force-device-scale-factor=2".into(),
                "--window-size=960,640".into(),
                format!("--screenshot={}", f.display()),
                url(&format!("&theme={theme}#{pg}")),
            ]);
            let png = std::fs::read(&f).unwrap();
            assert!(
                png.starts_with(b"\x89PNG") && png.len() > 10_000,
                "{}",
                f.display()
            );
            shots.push(png);
        }
    }
    assert_ne!(shots[0], shots[3], "light and dark are the same picture");
}
