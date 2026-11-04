// SPDX-License-Identifier: GPL-2.0-or-later
//! The web-app benchmark (tools/bench/webapps/apps.json) measures every web-based app WolOS ships
//! or offers: each Store app is a measured row or classified as not web-based, a module that
//! brings a measured app names the module in that row, and the default browser and the web-app
//! launcher (`wol webapp add`) have rows. A new app therefore cannot ship unmeasured.
#[path = "support/repo.rs"]
mod repo;

use repo::read;
use serde_json::Value;
use std::collections::BTreeSet;
use wol_core::modules::{IDS, Module};
use wol_core::store::CATALOG;
use wol_core::theme::helium::HELIUM;
use wol_core::webapps;

const APPS: &str = "tools/bench/webapps/apps.json";

fn spec() -> Value {
    serde_json::from_str(&read(APPS)).unwrap_or_else(|e| panic!("{APPS}: {e}"))
}

fn rows(v: &Value) -> Vec<Value> {
    v["apps"].as_array().expect("apps is a list").clone()
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// `store:<id>` origins of the measured rows.
fn measured_store_ids(v: &Value) -> BTreeSet<String> {
    rows(v)
        .iter()
        .flat_map(|r| strs(&r["origin"]))
        .filter_map(|o| o.strip_prefix("store:").map(String::from))
        .collect()
}

#[test]
fn every_store_app_is_measured_or_classified() {
    let v = spec();
    let measured = measured_store_ids(&v);
    let not_web: BTreeSet<String> = v["not_web"]
        .as_object()
        .expect("not_web is a map of id to reason")
        .keys()
        .cloned()
        .collect();
    let catalog: BTreeSet<String> = CATALOG.iter().map(|a| a.id.to_string()).collect();
    for id in &catalog {
        assert!(
            measured.contains(id) ^ not_web.contains(id),
            "Store app `{id}`: add a row to {APPS} (web-based) or a `not_web` entry with the reason, not both"
        );
    }
    for id in measured.union(&not_web) {
        assert!(
            catalog.contains(id),
            "{APPS} names `{id}`, which the Store no longer offers"
        );
    }
}

#[test]
fn modules_that_bring_a_measured_app_are_named_in_its_row() {
    let v = spec();
    let rows = rows(&v);
    for id in IDS {
        let m = Module::parse(&read(&format!("files/usr/share/wolos/modules/{id}.json")))
            .unwrap_or_else(|e| panic!("module {id}: {e}"));
        for r in &rows {
            let pk = strs(&r["packages"]);
            let brings = m.packages.iter().any(|p| pk.contains(p));
            let named = strs(&r["origin"]).contains(&format!("module:{id}"));
            // A row is web-based (or a native comparison app): only web rows must name modules.
            if r["kind"] == "web" {
                assert_eq!(
                    brings, named,
                    "row `{}` and module `{id}`: origin must list module:{id} exactly when the module installs one of {pk:?}",
                    r["id"]
                );
            }
        }
    }
}

#[test]
fn the_default_browser_and_the_web_app_launcher_are_measured() {
    let v = spec();
    let rows = rows(&v);
    let browser = HELIUM.bin.rsplit('/').next().unwrap();
    let base_browser = rows.iter().find(|r| {
        strs(&r["origin"]).contains(&"base".to_string())
            && r["argv"][0] == browser
            && !strs(&r["origin"]).contains(&"webapp".to_string())
    });
    assert!(
        base_browser.is_some(),
        "{APPS}: no base row runs the default browser `{browser}`"
    );
    let wa = rows
        .iter()
        .find(|r| strs(&r["origin"]).contains(&"webapp".to_string()))
        .expect("a row for the web-app launcher (origin webapp)");
    let argv = strs(&wa["argv"]);
    assert_eq!(argv[0], browser, "web apps open in the default browser");
    // The same window kind `wol webapp add` writes (webapps::launch_argv), and the row finds the
    // window by the class the engine really gives it (webapps::wm_class, from the address since
    // 0.12.1: the engine ignores `--class` for app windows on Wayland).
    assert!(argv.iter().any(|a| a.starts_with("--app=")), "{argv:?}");
    let page = argv
        .iter()
        .find_map(|a| a.strip_prefix("--app={url}"))
        .expect("--app={url}<path>");
    let app = webapps::WebApp::new(
        "Bench",
        &format!("http://127.0.0.1:8080{page}"),
        webapps::Profile::Shared,
    )
    .unwrap();
    let class = format!("^{}$", webapps::wm_class(&app).replace('.', "\\."));
    let pattern = wa["class"].as_str().expect("class pattern");
    assert!(
        pattern.split('|').any(|p| p == class),
        "{pattern} does not accept {class}"
    );
}

#[test]
fn rows_are_well_formed() {
    let v = spec();
    let mut ids = BTreeSet::new();
    let mut kinds = BTreeSet::new();
    for r in rows(&v) {
        let id = r["id"].as_str().expect("id");
        assert!(ids.insert(id.to_string()), "duplicate row `{id}`");
        let kind = r["kind"].as_str().unwrap_or("");
        assert!(
            matches!(kind, "web" | "native" | "candidate"),
            "`{id}`: kind {kind}"
        );
        kinds.insert(kind.to_string());
        assert!(
            matches!(
                r["interact"].as_str(),
                Some("scroll+type" | "type" | "none")
            ),
            "`{id}`: interact"
        );
        assert!(!strs(&r["argv"]).is_empty(), "`{id}`: argv");
        assert!(
            r["class"].as_str().is_some_and(|c| !c.is_empty()),
            "`{id}`: class"
        );
        for o in strs(&r["origin"]) {
            assert!(
                o == "base"
                    || o == "webapp"
                    || o == "example"
                    || o.starts_with("store:")
                    || o.starts_with("module:"),
                "`{id}`: origin {o}"
            );
        }
    }
    assert!(
        kinds.contains("native"),
        "the gap analysis needs native comparison rows"
    );
}
