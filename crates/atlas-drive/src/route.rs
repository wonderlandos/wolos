// SPDX-License-Identifier: GPL-2.0-or-later
//! Network interception through `Fetch`: mock a response, block a request, delay it. Routes are
//! kept by the daemon; one worker thread answers `Fetch.requestPaused` events.
use crate::browser::Browser;
use crate::investigate::chan_sub;
use crate::util::{Res, b64_encode, err, glob};
use crate::verbs::{Ctx, need, ps, pu};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Route {
    pub pattern: String,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Mock {
        status: u16,
        body: String,
        content_type: String,
    },
    Block,
    Delay(u64),
}

static STARTED: AtomicBool = AtomicBool::new(false);

/// What to do with a URL: the first route that matches, else None (continue untouched).
pub fn decide<'a>(routes: &'a [Route], url: &str) -> Option<&'a Action> {
    routes
        .iter()
        .find(|r| glob(&r.pattern, url))
        .map(|r| &r.action)
}

pub fn verb(ctx: &Ctx, p: &Value) -> Res<Value> {
    let (b, page) = ctx.d.page(p)?;
    match ps(p, "op").unwrap_or("add") {
        "list" => {
            let r = ctx.d.routes.lock().unwrap_or_else(PoisonError::into_inner);
            Ok(
                json!({"text": r.iter().map(|x| format!("{} -> {:?}\n", x.pattern, x.action)).collect::<String>()}),
            )
        }
        "clear" => {
            ctx.d
                .routes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
            let _ = b.call(Some(&page.session), "Fetch.disable", Value::Null);
            Ok(json!({"ok": true}))
        }
        "add" => {
            let pattern = need(p, "pattern")?.to_string();
            let action = match need(p, "action")? {
                "mock" => Action::Mock {
                    status: pu(p, "status").unwrap_or(200) as u16,
                    body: ps(p, "body").unwrap_or("").to_string(),
                    content_type: ps(p, "content_type")
                        .unwrap_or("application/json")
                        .to_string(),
                },
                "block" => Action::Block,
                "delay" => Action::Delay(pu(p, "ms").unwrap_or(1000)),
                o => {
                    return err(
                        "args",
                        format!("unknown action {o}"),
                        "mock | block | delay",
                    );
                }
            };
            let patterns: Vec<Value> = {
                let mut r = ctx.d.routes.lock().unwrap_or_else(PoisonError::into_inner);
                r.push(Route { pattern, action });
                r.iter()
                    .map(|x| json!({"urlPattern": x.pattern, "requestStage": "Request"}))
                    .collect()
            };
            b.call(
                Some(&page.session),
                "Fetch.enable",
                json!({"patterns": patterns}),
            )?;
            start_worker(ctx.d.clone(), b);
            Ok(json!({"ok": true, "routes": patterns.len()}))
        }
        o => err(
            "args",
            format!("unknown route op {o}"),
            "add | list | clear",
        ),
    }
}

fn start_worker(d: Arc<crate::daemon::Daemon>, b: Browser) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let (sub, rx, _sink) = chan_sub(vec!["Fetch.requestPaused".into()], 512, 0);
    b.subscribe(sub);
    let spawned = std::thread::Builder::new().name("fetch-routes".into()).spawn(move || {
        while let Ok(line) = rx.recv() {
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            let sess = v["sessionId"].as_str().map(String::from);
            let id = v["params"]["requestId"].clone();
            let url = v["params"]["request"]["url"].as_str().unwrap_or("").to_string();
            let action = {
                let r = d.routes.lock().unwrap_or_else(PoisonError::into_inner);
                decide(&r, &url).cloned()
            };
            let (b2, sess2, id2) = (b.clone(), sess.clone(), id.clone());
            let b = b.clone();
            let one = std::thread::Builder::new().name("fetch-route".into()).spawn(move || {
                let s = sess.as_deref();
                match action {
                    Some(Action::Mock { status, body, content_type }) => {
                        let _ = b.call(s, "Fetch.fulfillRequest", json!({"requestId": id, "responseCode": status, "responseHeaders": [{"name": "Content-Type", "value": content_type}, {"name": "Access-Control-Allow-Origin", "value": "*"}], "body": b64_encode(body.as_bytes())}));
                    }
                    Some(Action::Block) => {
                        let _ = b.call(s, "Fetch.failRequest", json!({"requestId": id, "errorReason": "BlockedByClient"}));
                    }
                    Some(Action::Delay(ms)) => {
                        std::thread::sleep(Duration::from_millis(ms.min(30_000)));
                        let _ = b.call(s, "Fetch.continueRequest", json!({"requestId": id}));
                    }
                    None => {
                        let _ = b.call(s, "Fetch.continueRequest", json!({"requestId": id}));
                    }
                }
            });
            if one.is_err() {
                // no thread for it: let the request go on unrouted rather than hang the page
                b2.fire(sess2.as_deref(), "Fetch.continueRequest", json!({"requestId": id2}));
            }
        }
    });
    if spawned.is_err() {
        // a later `route add` tries again
        STARTED.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_matching_route_wins() {
        let r = vec![
            Route {
                pattern: "https://a.test/api/*".into(),
                action: Action::Block,
            },
            Route {
                pattern: "*".into(),
                action: Action::Delay(5),
            },
        ];
        assert_eq!(decide(&r, "https://a.test/api/x"), Some(&Action::Block));
        assert_eq!(decide(&r, "https://b.test/"), Some(&Action::Delay(5)));
        assert_eq!(decide(&r[..1], "https://b.test/"), None);
    }
}
