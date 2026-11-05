// SPDX-License-Identifier: GPL-2.0-or-later
//! `batch`: many verbs in one request. Sequential by default (each step after the previous one,
//! stop at the first failure). A step with `after: [ids]`, or any step when `parallel` is set, is
//! a fork/join node: independent steps (screenshot + extract + links) run concurrently, each result
//! is streamed as it finishes, and a failure stops unstarted steps unless `continue` is set.
use crate::util::{DriveError, Res, err};
use crate::verbs::{Ctx, call, known, pb};
use serde_json::{Value, json};
use std::sync::mpsc::channel;

pub const MAX_STEPS: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub id: String,
    pub verb: String,
    pub params: Value,
    pub after: Vec<String>,
}

/// Parse and validate the steps (ids unique, dependencies exist and point backwards or at known ids,
/// no cycles, no nested batch).
pub fn plan(p: &Value) -> Res<Vec<Step>> {
    let arr = p
        .get("steps")
        .and_then(|s| {
            if let Value::String(t) = s {
                serde_json::from_str::<Value>(t).ok()
            } else {
                Some(s.clone())
            }
        })
        .and_then(|v| v.as_array().cloned())
        .ok_or_else(|| {
            DriveError::new(
                "args",
                "batch needs steps",
                "pass an array: [{\"verb\":\"goto\",\"params\":{\"url\":\"…\"}},…]",
            )
        })?;
    if arr.is_empty() || arr.len() > MAX_STEPS {
        return err(
            "args",
            format!("batch has {} steps", arr.len()),
            format!("1 to {MAX_STEPS} steps"),
        );
    }
    let parallel = pb(p, "parallel").unwrap_or(false);
    let mut steps: Vec<Step> = vec![];
    for (i, s) in arr.iter().enumerate() {
        let verb = s.get("verb").and_then(Value::as_str).ok_or_else(|| {
            DriveError::new(
                "args",
                format!("step {i} has no verb"),
                "each step is {\"verb\":…}",
            )
        })?;
        if verb == "batch" {
            return err("args", "batches do not nest", "flatten the steps");
        }
        if !known(verb) {
            return err(
                "unknown_verb",
                format!("step {i}: unknown verb {verb}"),
                "run atlas-drive --help-agent",
            );
        }
        let id = s
            .get("id")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| i.to_string());
        if steps.iter().any(|x| x.id == id) {
            return err(
                "args",
                format!("step id {id} is used twice"),
                "ids must be unique",
            );
        }
        let after = match s.get("after") {
            Some(a) => a
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default(),
            None if parallel || i == 0 => vec![],
            None => vec![steps[i - 1].id.clone()],
        };
        steps.push(Step {
            id,
            verb: verb.into(),
            params: s.get("params").cloned().unwrap_or(json!({})),
            after,
        });
    }
    for s in &steps {
        for a in &s.after {
            if !steps.iter().any(|x| &x.id == a) {
                return err(
                    "args",
                    format!("step {} waits for unknown step {a}", s.id),
                    "after: [ids of other steps]",
                );
            }
            if a == &s.id {
                return err("args", format!("step {} waits for itself", s.id), "");
            }
        }
    }
    // cycle check by repeated removal of ready nodes
    let mut left: Vec<&Step> = steps.iter().collect();
    let mut done: Vec<&str> = vec![];
    while !left.is_empty() {
        let (ready, rest): (Vec<&Step>, Vec<&Step>) = left
            .into_iter()
            .partition(|s| s.after.iter().all(|a| done.contains(&a.as_str())));
        if ready.is_empty() {
            return err(
                "args",
                "the steps wait for each other in a cycle",
                "fix the after lists",
            );
        }
        done.extend(ready.iter().map(|s| s.id.as_str()));
        left = rest;
    }
    Ok(steps)
}

pub fn run(ctx: &Ctx, p: &Value) -> Res<Value> {
    let steps = plan(p)?;
    let cont = pb(p, "continue").unwrap_or(false);
    let n = steps.len();
    let (tx, rx) = channel::<(usize, Result<Value, DriveError>)>();
    let mut state: Vec<u8> = vec![0; n]; // 0 waiting, 1 running, 2 ok, 3 failed, 4 skipped
    let mut results: Vec<Value> = vec![Value::Null; n];
    let mut failed: Option<String> = None;
    let mut running = 0;
    std::thread::scope(|sc| {
        loop {
            for i in 0..n {
                if state[i] != 0 {
                    continue;
                }
                let ready = steps[i].after.iter().all(|a| {
                    let j = steps.iter().position(|x| &x.id == a).unwrap_or(0);
                    state[j] >= 2
                });
                if !ready {
                    continue;
                }
                let dep_bad = steps[i].after.iter().any(|a| {
                    let j = steps.iter().position(|x| &x.id == a).unwrap_or(0);
                    state[j] >= 3
                });
                if (failed.is_some() && !cont) || dep_bad {
                    state[i] = 4;
                    results[i] = json!({"id": steps[i].id, "verb": steps[i].verb, "skipped": true});
                    continue;
                }
                state[i] = 1;
                running += 1;
                let txi = tx.clone();
                let step = steps[i].clone();
                sc.spawn(move || {
                    let r = call(ctx, &step.verb, &step.params);
                    let _ = txi.send((i, r));
                });
            }
            if running == 0 {
                break;
            }
            let Ok((i, r)) = rx.recv() else { break };
            running -= 1;
            match r {
                Ok(v) => {
                    state[i] = 2;
                    results[i] =
                        json!({"id": steps[i].id, "verb": steps[i].verb, "ok": true, "r": v});
                }
                Err(e) => {
                    state[i] = 3;
                    if failed.is_none() {
                        failed = Some(steps[i].id.clone());
                    }
                    results[i] =
                        json!({"id": steps[i].id, "verb": steps[i].verb, "error": e.line()});
                }
            }
            (ctx.emit)(json!({"event": "step", "step": results[i]}));
        }
    });
    let ok = failed.is_none();
    let mut out = if pb(p, "stream_only").unwrap_or(false) {
        // the caller already got every step as an event
        json!({"ok": ok, "steps": n, "ran": state.iter().filter(|s| **s >= 2 && **s != 4).count()})
    } else {
        json!({"ok": ok, "results": results})
    };
    if let Some(f) = failed {
        out["failed"] = f.into();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_by_default_and_parallel_on_request() {
        let s = plan(&json!({"steps": [{"verb":"goto"},{"verb":"snapshot"},{"verb":"links"}]}))
            .unwrap();
        assert_eq!(s[1].after, vec!["0"]);
        assert_eq!(s[2].after, vec!["1"]);
        let s = plan(&json!({"parallel": true, "steps": [{"verb":"screenshot"},{"verb":"extract"},{"verb":"links","after":["0","1"]}]})).unwrap();
        assert!(s[0].after.is_empty() && s[1].after.is_empty());
        assert_eq!(s[2].after, vec!["0", "1"]);
    }

    #[test]
    fn bad_plans_are_one_line_errors() {
        assert!(
            plan(&json!({"steps": []}))
                .unwrap_err()
                .message
                .contains("0 steps")
        );
        assert!(
            plan(&json!({"steps": [{"verb":"nope"}]}))
                .unwrap_err()
                .message
                .contains("unknown verb")
        );
        assert!(
            plan(&json!({"steps": [{"verb":"batch"}]}))
                .unwrap_err()
                .message
                .contains("nest")
        );
        assert!(
            plan(&json!({"steps": [{"id":"a","verb":"links"},{"id":"a","verb":"forms"}]}))
                .unwrap_err()
                .message
                .contains("twice")
        );
        assert!(plan(&json!({"steps": [{"id":"a","verb":"links","after":["b"]},{"id":"b","verb":"forms","after":["a"]}]})).unwrap_err().message.contains("cycle"));
        assert!(
            plan(&json!({"steps": [{"id":"a","verb":"links","after":["zz"]}]}))
                .unwrap_err()
                .message
                .contains("unknown step")
        );
    }

    #[test]
    fn steps_may_arrive_as_a_json_string() {
        let s = plan(&json!({"steps": "[{\"verb\":\"tabs\"}]"})).unwrap();
        assert_eq!(s.len(), 1);
    }
}
