// SPDX-License-Identifier: GPL-2.0-or-later
//! `record`: frames of a run (Page.startScreencast) written to a directory as numbered JPEGs with
//! a `frames.jsonl` index (timestamp per frame). Assemble a video with any encoder, e.g.
//! `ffmpeg -framerate 10 -i frame-%05d.jpg run.webm`. With `cursor show` the pointer is in the frames.
use crate::browser::Browser;
use crate::investigate::chan_sub;
use crate::util::{DriveError, Res, b64_decode, err};
use crate::verbs::{Ctx, need, ps, pu};
use serde_json::{Value, json};
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, PoisonError};
use std::thread::JoinHandle;

pub struct Recorder {
    pub dir: PathBuf,
    pub session: String,
    pub sub: Arc<crate::browser::Sub>,
    pub worker: Option<JoinHandle<u64>>,
}

pub fn verb(ctx: &Ctx, p: &Value) -> Res<Value> {
    let (b, page) = ctx.d.page(p)?;
    match need(p, "op")? {
        "start" => {
            let mut g = ctx
                .d
                .recorder
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if g.is_some() {
                return err(
                    "record",
                    "a recording is already running",
                    "stop it first: record stop",
                );
            }
            let dir = ps(p, "dir")
                .map(PathBuf::from)
                .unwrap_or_else(|| ctx.d.state_dir.join(format!("rec-{}", ctx.d.next_shot())));
            std::fs::create_dir_all(&dir).map_err(|e| {
                DriveError::new(
                    "io",
                    format!("cannot create {}: {e}", dir.display()),
                    "choose a writable --dir",
                )
            })?;
            let (sub, rx, sink) = chan_sub(vec!["Page.screencastFrame".into()], 32, 0);
            let sub = Arc::new(crate::browser::Sub {
                sink: sink.clone(),
                patterns: sub.patterns.clone(),
                session: Some(page.session.clone()),
                sent: Default::default(),
                limit: 0,
                done: Default::default(),
                all_pages: false,
            });
            b.subscribe(sub.clone());
            let worker = spawn_writer(b.clone(), rx, dir.clone());
            b.call(Some(&page.session), "Page.startScreencast", json!({"format": "jpeg", "quality": 70, "everyNthFrame": pu(p, "every").unwrap_or(1)}))?;
            *g = Some(Recorder {
                dir: dir.clone(),
                session: page.session.clone(),
                sub,
                worker: Some(worker),
            });
            Ok(json!({"ok": true, "dir": dir.display().to_string()}))
        }
        "stop" => {
            let rec = ctx
                .d
                .recorder
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            let Some(mut rec) = rec else {
                return err("record", "no recording is running", "");
            };
            let _ = b.call(Some(&rec.session), "Page.stopScreencast", Value::Null);
            b.unsubscribe(&rec.sub);
            drop(rec.sub.clone());
            // dropping the last sender ends the worker
            let sub = rec.sub.clone();
            drop(sub);
            let n = rec
                .worker
                .take()
                .map(|w| w.join().unwrap_or(0))
                .unwrap_or(0);
            Ok(
                json!({"ok": true, "frames": n, "dir": rec.dir.display().to_string(), "video": "ffmpeg -framerate 10 -i frame-%05d.jpg run.webm"}),
            )
        }
        o => err("args", format!("unknown record op {o}"), "start | stop"),
    }
}

fn spawn_writer(b: Browser, rx: Receiver<String>, dir: PathBuf) -> JoinHandle<u64> {
    std::thread::spawn(move || {
        let mut n = 0u64;
        let mut index = std::fs::File::create(dir.join("frames.jsonl")).ok();
        // recv_timeout so the worker also ends when the sender stays alive in a dropped subscription
        while let Ok(line) = rx.recv_timeout(crate::util::WAIT_1500MS) {
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let sess = v["sessionId"].as_str().map(String::from);
            let p = &v["params"];
            if let Some(bytes) = p["data"].as_str().and_then(b64_decode) {
                n += 1;
                if n > 20_000 {
                    break;
                }
                let _ = std::fs::write(dir.join(format!("frame-{n:05}.jpg")), &bytes);
                if let Some(f) = index.as_mut() {
                    let _ = writeln!(
                        f,
                        "{}",
                        json!({"n": n, "t": p["metadata"]["timestamp"], "w": p["metadata"]["deviceWidth"], "h": p["metadata"]["deviceHeight"]})
                    );
                }
            }
            if let Some(sid) = p["sessionId"].as_i64() {
                b.fire(
                    sess.as_deref(),
                    "Page.screencastFrameAck",
                    json!({"sessionId": sid}),
                );
            }
        }
        n
    })
}
