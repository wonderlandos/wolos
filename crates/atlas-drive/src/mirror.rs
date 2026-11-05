// SPDX-License-Identifier: GPL-2.0-or-later
//! The in-page accessibility mirror (ADR 0063, round 2). `Accessibility.getFullAXTree` costs the
//! browser about 0.9 s on a 2300-link article every time it is asked (17 386 nodes, 6.9 MB, a third
//! of them inline text boxes and list markers), and the Accessibility domain's update events only
//! cover nodes fetched one level at a time, throttled to 250 ms. So the tree an agent reads is
//! built in the page instead: one walk in Drive's isolated world (`pagejs::SNAP`, the page cannot
//! see or change it) returns compact rows, which become AX-shaped nodes here so `agent::lines_of`,
//! the refs, the diff and the budget are unchanged. Elements keep one id while the document lives
//! (a WeakMap in the isolated world), so refs are stable across snapshots; an id becomes a real
//! backend node id only when a verb acts on it (two round trips, cached per document).
//! `snapshot --ax` still reads the browser's own tree.
use crate::agent::AgentState;
use crate::browser::Browser;
use crate::page::Page;
use crate::util::{DriveError, Res, err};
use crate::verbs::{exec, unwrap_eval};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

const T: Duration = Duration::from_secs(30);

/// Mirror keys are negative so they can never be taken for a backend node id by mistake.
pub fn key_of(element_id: i64) -> i64 {
    -element_id
}

/// What one walk gives: AX-shaped nodes, in-viewport flags by key (when asked), withheld count.
pub struct Tree {
    pub nodes: Vec<Value>,
    pub in_view: HashMap<i64, bool>,
    pub withheld: usize,
    pub title: String,
}

/// Rows of 8 from `pagejs::SNAP` to the `AXNode` shape `agent::lines_of` reads. Pure (tested).
pub fn rows_to_tree(v: &Value) -> Tree {
    let empty = vec![];
    let rows = v["rows"].as_array().unwrap_or(&empty);
    let n = rows.len() / 8;
    let mut children: Vec<Vec<Value>> = vec![vec![]; n];
    for i in 0..n {
        let par = rows[i * 8 + 1].as_i64().unwrap_or(-1);
        if par >= 0 && (par as usize) < i {
            children[par as usize].push(Value::String(i.to_string()));
        }
    }
    let mut nodes = Vec::with_capacity(n);
    let mut in_view = HashMap::new();
    let mut title = String::new();
    for (i, kids) in children.into_iter().enumerate() {
        let r = &rows[i * 8..i * 8 + 8];
        let id = r[0].as_i64().unwrap_or(0);
        let par = r[1].as_i64().unwrap_or(-1);
        let role = r[2].as_str().unwrap_or("");
        let name = r[3].as_str().unwrap_or("");
        let flags = r[4].as_str().unwrap_or("");
        let val = r[5].as_str().unwrap_or("");
        let url = r[6].as_str().unwrap_or("");
        let level = r[7].as_i64().unwrap_or(0);
        if i == 0 {
            title = name.to_string();
        }
        let mut props: Vec<Value> = vec![];
        let mut flag = |k: &str, v: Value| props.push(json!({"name": k, "value": {"value": v}}));
        for c in flags.chars() {
            match c {
                'd' => flag("disabled", json!(true)),
                'c' => flag("checked", json!("true")),
                'm' => flag("checked", json!("mixed")),
                'u' => flag("checked", json!("false")),
                's' => flag("selected", json!(true)),
                'e' => flag("expanded", json!(true)),
                'x' => flag("expanded", json!(false)),
                'r' => flag("required", json!(true)),
                'o' => flag("readonly", json!(true)),
                'i' => flag("invalid", json!("true")),
                'p' => flag("pressed", json!("true")),
                'M' => flag("modal", json!(true)),
                'f' => flag("focused", json!(true)),
                _ => {}
            }
        }
        if level > 0 {
            flag("level", json!(level));
        }
        if !url.is_empty() {
            flag("url", json!(url));
        }
        let mut node = json!({
            "nodeId": i.to_string(),
            "role": {"value": role},
            "name": {"value": name},
            "properties": props,
            "childIds": kids,
        });
        if par >= 0 {
            node["parentId"] = json!(par.to_string());
        }
        if id > 0 {
            node["backendDOMNodeId"] = json!(key_of(id));
            // a text row carries its element's id (for `find`), not a box of its own
            if role != "StaticText" {
                in_view.insert(key_of(id), flags.contains('v'));
            }
        }
        if !val.is_empty() || matches!(role, "textbox" | "searchbox" | "combobox") {
            node["value"] = json!({"value": val});
        }
        nodes.push(node);
    }
    Tree {
        nodes,
        in_view,
        withheld: v["withheld"].as_u64().unwrap_or(0) as usize,
        title,
    }
}

/// Drive's isolated world in the current document (created once per document).
pub fn world(b: &Browser, page: &Page, ag: &mut AgentState) -> Res<i64> {
    if let Some(c) = ag.world {
        return Ok(c);
    }
    let r = exec(
        b,
        page,
        "Page.createIsolatedWorld",
        json!({"frameId": page.target_id, "worldName": "atlas-drive", "grantUniveralAccess": false}),
        T,
    )?;
    let c = r["executionContextId"]
        .as_i64()
        .ok_or_else(|| DriveError::new("world", "the browser made no isolated world", "retry"))?;
    ag.world = Some(c);
    Ok(c)
}

/// Run an expression in Drive's isolated world (recreated once if the document changed under it).
pub fn eval_in_world(
    b: &Browser,
    page: &Page,
    ag: &mut AgentState,
    expr: &str,
    by_value: bool,
) -> Res<Value> {
    for attempt in 0..2 {
        let ctx = world(b, page, ag)?;
        match exec(
            b,
            page,
            "Runtime.evaluate",
            json!({"expression": expr, "returnByValue": by_value, "contextId": ctx}),
            T,
        ) {
            Ok(r) => return Ok(r),
            Err(e) if attempt == 0 => {
                let _ = e;
                ag.world = None;
            }
            Err(e) => return Err(e),
        }
    }
    err("world", "the isolated world is gone", "retry")
}

/// One walk of the page. `geo`: also say which elements are in the viewport. `safe`: withhold traps.
pub fn tree(b: &Browser, page: &Page, ag: &mut AgentState, geo: bool, safe: bool) -> Res<Tree> {
    let js = crate::pagejs::with_args(crate::pagejs::SNAP, &json!({"geo": geo, "safe": safe}));
    let r = eval_in_world(b, page, ag, &js, true)?;
    let v = unwrap_eval(&r)?;
    if v["rows"].as_array().is_none() {
        return err(
            "ax",
            "the page gave no tree",
            "retry after the page loaded, or snapshot --ax",
        );
    }
    Ok(rows_to_tree(&v))
}

/// A mirror key to the browser's backend node id (cached per document); other ids pass through.
pub fn real_backend(b: &Browser, page: &Page, key: i64) -> Res<i64> {
    if key >= 0 {
        return Ok(key);
    }
    let mut ag = page
        .agent
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(r) = ag.real.get(&key) {
        return Ok(*r);
    }
    let r = eval_in_world(
        b,
        page,
        &mut ag,
        &format!("globalThis.__ad&&globalThis.__ad.els[{}]", -key),
        false,
    )?;
    let Some(oid) = r["result"]["objectId"].as_str() else {
        return err(
            "stale_ref",
            "that element is gone from the page",
            "run snapshot for new refs",
        );
    };
    let d = exec(b, page, "DOM.describeNode", json!({"objectId": oid}), T)?;
    let bk = d["node"]["backendNodeId"].as_i64().ok_or_else(|| {
        DriveError::new(
            "stale_ref",
            "that element is gone from the page",
            "run snapshot",
        )
    })?;
    ag.real.insert(key, bk);
    Ok(bk)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Options, lines_of};

    #[test]
    fn rows_become_a_tree_that_renders_like_the_browsers() {
        let v = json!({"rows": [
            0, -1, "RootWebArea", "Shop", "", "", "", 0,
            1, 0, "navigation", "Main", "", "", "", 0,
            2, 1, "link", "", "v", "", "http://x/a", 0,
            2, 2, "StaticText", "Home", "", "", "", 0,
            3, 0, "heading", "", "", "", "", 1,
            3, 4, "StaticText", "Welcome", "", "", "", 0,
            4, 0, "textbox", "Email", "rf", "me@x", "", 0,
            5, 0, "checkbox", "News", "u", "", "", 0,
            6, 0, "button", "Save", "d", "", "", 0
        ], "withheld": 2});
        let t = rows_to_tree(&v);
        assert_eq!(t.title, "Shop");
        assert_eq!(t.withheld, 2);
        assert_eq!(t.in_view.get(&-2), Some(&true));
        assert_eq!(t.in_view.get(&-6), Some(&false));
        let lines = lines_of(&t.nodes, &Options::default());
        let bodies: Vec<&str> = lines.iter().map(|l| l.body.as_str()).collect();
        assert_eq!(
            bodies,
            vec![
                "navigation \"Main\"",
                "link \"Home\"",
                "h1 \"Welcome\"",
                "textbox \"Email\" value=\"me@x\" required focused",
                "checkbox \"News\"",
                "button \"Save\" disabled",
            ]
        );
        assert_eq!(lines[1].backend, Some(-2));
        assert_eq!(lines[1].href, "http://x/a");
    }

    #[test]
    fn short_or_malformed_rows_do_not_panic() {
        for v in [
            json!({}),
            json!({"rows": [1, 2, 3]}),
            json!({"rows": [0, 5, null, 1, "", "", "", "x"]}),
        ] {
            let t = rows_to_tree(&v);
            let _ = lines_of(&t.nodes, &Options::default());
        }
    }
}
