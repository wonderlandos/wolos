// SPDX-License-Identifier: GPL-2.0-or-later
//! The Drive socket. One thread per connection reads `\n`-framed JSON; a request is either a
//! `Drive.<verb>` call (answered with a short result, `tokens`, and streamed `event` lines for
//! batches) or a raw CDP command (ids rewritten, passed through to the browser). Peer identity is
//! SO_PEERCRED with a pinned `/proc/<pid>` (peer.rs); only this account's processes are served,
//! and an agent needs the grant for the scope a verb belongs to (perms.rs).
use crate::browser::RawSink;
use crate::cdp::{Framer, MAX_CLIENT_LINE};
use crate::daemon::Daemon;
use crate::peer::Identity;
use crate::perms::{self, Decision, Grants};
use crate::spec;
use crate::util::{DriveError, tokens};
use crate::verbs::{Ctx, call};
use serde_json::{Value, json};
use std::io::Write;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

pub const MAX_CONNECTIONS: usize = 32;
pub const MAX_IN_FLIGHT: usize = 32;
const WRITE_WAIT: Duration = Duration::from_secs(10);

pub struct ServerCfg {
    pub grants_path: PathBuf,
    /// CI mode (ATLAS_DRIVE_CI=1): no consent, for the forge and the wolvm guest.
    pub ci: bool,
    pub consent_cmd: Option<String>,
    pub own_exe: Option<PathBuf>,
    pub attached_file: Option<PathBuf>,
}

struct Store {
    path: PathBuf,
    cache: Mutex<(Option<SystemTime>, Grants)>,
}

impl Store {
    fn current(&self) -> Grants {
        let m = std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok();
        let mut c = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if c.0 != m || m.is_none() {
            *c = (m, perms::load(&self.path));
        }
        c.1.clone()
    }
    fn grant(&self, exe: &str, scopes: &[String]) {
        let mut g = perms::load(&self.path);
        g.grant(exe, scopes, perms::now());
        let _ = perms::save(&self.path, &g);
        *self.cache.lock().unwrap_or_else(PoisonError::into_inner) = (None, Grants::default());
    }
}

struct Shared {
    d: Arc<Daemon>,
    cfg: ServerCfg,
    store: Store,
    conns: AtomicUsize,
    attached: Mutex<Vec<(u64, String, i32)>>,
    next_conn: std::sync::atomic::AtomicU64,
}

/// A client connection as an output: lines are written whole under a lock, slow clients are dropped.
pub struct Out {
    stream: Mutex<UnixStream>,
    pub alive: std::sync::atomic::AtomicBool,
}

impl Out {
    pub fn new(s: UnixStream) -> Arc<Out> {
        let _ = s.set_write_timeout(Some(WRITE_WAIT));
        Arc::new(Out {
            stream: Mutex::new(s),
            alive: true.into(),
        })
    }
}

impl RawSink for Out {
    fn send(&self, mut line: String) -> bool {
        line.push('\n');
        let mut s = self.stream.lock().unwrap_or_else(PoisonError::into_inner);
        let ok = s.write_all(line.as_bytes()).is_ok();
        if !ok {
            self.alive.store(false, Ordering::Relaxed);
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        ok
    }
}

/// The name an agent is known by. The Drive CLI is the person when it runs on a terminal; run by a
/// program without one (an agent's shell tool) it is that program: its parent's executable.
pub fn agent_name(id: &Identity, own_exe: Option<&std::path::Path>) -> (String, bool) {
    let exe = id.name();
    if own_exe.is_some_and(|o| id.exe.as_deref() == Some(o))
        && let Some((_, tty)) = id.stat()
    {
        if tty != 0 {
            return ("terminal".into(), true);
        }
        if let Some(pe) = id.parent_exe() {
            return (pe, false);
        }
    }
    (exe, false)
}

/// Add the result's token estimate; cut `text` to fit a budget.
pub fn finalize(mut v: Value, budget: Option<usize>) -> Value {
    if !v.is_object() {
        v = json!({ "value": v });
    }
    if let Some(b) = budget {
        fit_budget(&mut v, b);
    }
    let n = tokens(v.to_string().len());
    v["tokens"] = n.into();
    v
}

/// `--quiet`: only the answer. A verb's result keeps its value or text (or `fields`, comma-separated
/// or a list); `ok: true` and the change report go. A batch keeps the answers of the steps that have
/// one, in order (`values`), and its failure if any. Scripts that already know their locators pay
/// for what they asked, nothing else.
pub fn terse(v: Value, fields: Option<&Value>) -> Value {
    let wanted: Vec<String> = match fields {
        Some(Value::String(s)) => s
            .split(',')
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty())
            .collect(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect(),
        _ => vec![],
    };
    let keep = |o: &serde_json::Map<String, Value>| -> Option<Value> {
        if !wanted.is_empty() {
            let m: serde_json::Map<String, Value> = o
                .iter()
                .filter(|(k, _)| wanted.iter().any(|w| w == *k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            return (!m.is_empty()).then_some(Value::Object(m));
        }
        ["value", "text", "view", "values"]
            .iter()
            .find_map(|k| o.get(*k).cloned())
    };
    let Value::Object(o) = v else { return v };
    if let Some(Value::Array(steps)) = o.get("results") {
        let mut values = vec![];
        for st in steps {
            if let Some(r) = st.get("r").and_then(Value::as_object)
                && let Some(x) = keep(r)
            {
                values.push(x);
            }
        }
        // one answer is the answer itself
        let mut out = if values.len() == 1 && wanted.is_empty() {
            json!({ "value": values.remove(0) })
        } else {
            json!({ "values": values })
        };
        if let Some(f) = o.get("failed") {
            out["failed"] = f.clone();
            if let Some(e) = steps.iter().find_map(|s| s.get("error")) {
                out["error"] = e.clone();
            }
        }
        return out;
    }
    match keep(&o) {
        Some(x) if wanted.is_empty() => json!({ "value": x }),
        Some(x) => x,
        None => json!({}),
    }
}

/// Deterministic truncation: cut the longest text field at a line boundary so the whole fits, and say so.
pub fn fit_budget(v: &mut Value, budget: usize) {
    let total = tokens(v.to_string().len());
    if total <= budget {
        return;
    }
    let key = ["text", "view", "body", "script"]
        .iter()
        .find(|k| v[**k].is_string())
        .copied();
    if let Some(k) = key {
        let text = v[k].as_str().unwrap_or("").to_string();
        v[k] = "".into();
        let other = tokens(v.to_string().len());
        let room = budget.saturating_sub(other + 12) * 4;
        let mut out = String::new();
        let mut cut = 0usize;
        for l in text.split_inclusive('\n') {
            if out.len() + l.len() <= room {
                out.push_str(l);
            } else {
                cut += 1;
            }
        }
        out.push_str(&format!(
            "… {cut} more lines cut to fit --budget {budget}\n"
        ));
        v[k] = out.into();
        v["truncated"] = true.into();
    } else if let Some(a) = v["results"].as_array().cloned() {
        // a batch: keep whole steps while they fit
        let mut keep = vec![];
        let mut used = 40usize;
        for r in &a {
            let c = tokens(r.to_string().len());
            if used + c > budget {
                break;
            }
            used += c;
            keep.push(r.clone());
        }
        let dropped = a.len() - keep.len();
        v["results"] = Value::Array(keep);
        v["truncated"] = true.into();
        v["omitted_steps"] = dropped.into();
    } else {
        v["over_budget"] = true.into();
    }
}

/// The scopes a request needs: the verb's own, those of every step of a batch, `files` for any call that
/// writes where the caller says (`out`, `dir`), and every scope for `cdp` (raw CDP can do anything).
pub fn scopes_needed(verb: &str, params: &Value, steps: &[(String, Value)]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = vec![];
    let mut add = |v: &str, p: &Value| {
        let Some(sp) = spec::find(v) else { return };
        if v == "cdp" {
            out.extend(perms::SCOPES.iter().copied());
        }
        out.push(sp.scope);
        if p.get("out").is_some_and(Value::is_string) || p.get("dir").is_some_and(Value::is_string)
        {
            out.push("files");
        }
    };
    add(verb, params);
    for (n, p) in steps {
        add(n, p);
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn err_json(id: &Value, e: &DriveError) -> Value {
    json!({"id": id, "error": {"code": -32000, "message": e.line(), "data": {"code": e.code}}})
}

pub fn serve(listener: UnixListener, d: Arc<Daemon>, cfg: ServerCfg) -> std::io::Result<()> {
    let shared = Arc::new(Shared {
        store: Store {
            path: cfg.grants_path.clone(),
            cache: Mutex::new((None, Grants::default())),
        },
        d,
        cfg,
        conns: AtomicUsize::new(0),
        attached: Mutex::new(vec![]),
        next_conn: 1.into(),
    });
    for conn in listener.incoming() {
        let Ok(conn) = conn else {
            std::thread::sleep(crate::util::WAIT_100MS);
            continue;
        };
        if shared.conns.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            shared.conns.fetch_sub(1, Ordering::SeqCst);
            let _ = (&conn).write_all(
                b"{\"error\":{\"code\":-32000,\"message\":\"too many connections; close one\"}}\n",
            );
            continue;
        }
        let sh = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("conn".into())
            .stack_size(1024 * 1024)
            .spawn(move || {
                handle(conn, &sh);
                sh.conns.fetch_sub(1, Ordering::SeqCst);
            });
        if spawned.is_err() {
            shared.conns.fetch_sub(1, Ordering::SeqCst);
        }
    }
    Ok(())
}

fn publish_attached(sh: &Shared) {
    if let Some(f) = &sh.cfg.attached_file {
        let list: Vec<Value> = sh
            .attached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(_, n, p)| json!({"agent": n, "pid": p}))
            .collect();
        if list.is_empty() {
            let _ = std::fs::remove_file(f);
        } else {
            let _ = std::fs::write(f, json!({"attached": list}).to_string());
        }
    }
}

fn authorize(
    sh: &Shared,
    id: &Identity,
    name: &str,
    person: bool,
    scope: &str,
    site: Option<&str>,
) -> Result<(), DriveError> {
    if sh.cfg.ci || person {
        return Ok(());
    }
    let g = sh.store.current();
    match g.decide(name, scope, perms::now(), site) {
        Decision::Allow | Decision::Auto => Ok(()),
        Decision::Ask => {
            if let Some(cmd) = &sh.cfg.consent_cmd
                && perms::ask(cmd, name, perms::SCOPES)
            {
                sh.store.grant(
                    name,
                    &perms::SCOPES
                        .iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>(),
                );
                if sh.store.current().decide(name, scope, perms::now(), site) != Decision::Ask {
                    return Ok(());
                }
            }
            let _ = id;
            Err(DriveError::new(
                "not_allowed",
                format!("{name} is not allowed to use '{scope}' on Atlas"),
                format!(
                    "the person can allow it once: atlas-drive permissions allow {name} (or turn AUTO on: atlas-drive permissions auto on --for 2h)"
                ),
            ))
        }
    }
}

fn handle(conn: UnixStream, sh: &Arc<Shared>) {
    let Some(id) = Identity::capture(&conn) else {
        return;
    };
    if !id.is_owner() {
        return;
    }
    let (name, person) = agent_name(&id, sh.cfg.own_exe.as_deref());
    let Ok(wr) = conn.try_clone() else { return };
    let out = Out::new(wr);
    let cid = sh.next_conn.fetch_add(1, Ordering::Relaxed);
    if !person {
        sh.attached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((cid, name.clone(), id.pid));
        publish_attached(sh);
    }
    let in_flight = Arc::new(AtomicUsize::new(0));
    let mut fr = Framer::new(conn, b'\n', MAX_CLIENT_LINE);
    while let Ok(Some(raw)) = fr.next_msg() {
        if !out.alive.load(Ordering::Relaxed) {
            break;
        }
        let Ok(req) = serde_json::from_slice::<Value>(&raw) else {
            out.send(
                json!({"error": {"code": -32700, "message": "not JSON; one JSON object per line"}})
                    .to_string(),
            );
            continue;
        };
        let rid = req.get("id").cloned().unwrap_or(Value::Null);
        if !id.still() {
            out.send(
                err_json(
                    &rid,
                    &DriveError::new(
                        "identity",
                        "the client program changed after connecting",
                        "reconnect",
                    ),
                )
                .to_string(),
            );
            break;
        }
        let Some(method) = req.get("method").and_then(Value::as_str).map(String::from) else {
            out.send(
                err_json(
                    &rid,
                    &DriveError::new(
                        "args",
                        "a request needs a method",
                        "{\"id\":1,\"method\":\"Drive.snapshot\"}",
                    ),
                )
                .to_string(),
            );
            continue;
        };
        let site =
            sh.d.running()
                .and_then(|b| b.current())
                .and_then(|p| perms::site_of(&p.lock().url));
        if method == "Drive.shutdown" {
            if let Err(e) = authorize(sh, &id, &name, person, "act", site.as_deref()) {
                out.send(err_json(&rid, &e).to_string());
                continue;
            }
            sh.d.close_browser();
            out.send(json!({"id": rid, "result": {"ok": true}}).to_string());
            std::process::exit(0);
        }
        if let Some(verb) = method.strip_prefix("Drive.") {
            let params = req.get("params").cloned().unwrap_or(json!({}));
            let verbs: Vec<(String, Value)> = if verb == "batch" {
                crate::batch::plan(&params)
                    .map(|s| s.into_iter().map(|s| (s.verb, s.params)).collect())
                    .unwrap_or_default()
            } else {
                vec![]
            };
            let scopes = scopes_needed(verb, &params, &verbs);
            // AUTO for a site covers where the call goes: a navigation names its destination
            let dest = std::iter::once((verb.to_string(), params.clone()))
                .chain(verbs.iter().cloned())
                .find_map(|(v, p)| {
                    (v == "goto" || v == "tab")
                        .then(|| {
                            p.get("url")
                                .and_then(Value::as_str)
                                .and_then(perms::site_of)
                        })
                        .flatten()
                });
            let site = dest.or(site.clone());
            let mut denied = None;
            for s in &scopes {
                if let Err(e) = authorize(sh, &id, &name, person, s, site.as_deref()) {
                    denied = Some(e);
                    break;
                }
            }
            if let Some(e) = denied {
                out.send(err_json(&rid, &e).to_string());
                continue;
            }
            if in_flight.fetch_add(1, Ordering::SeqCst) >= MAX_IN_FLIGHT {
                in_flight.fetch_sub(1, Ordering::SeqCst);
                out.send(
                    err_json(
                        &rid,
                        &DriveError::new("busy", "too many requests in flight", "wait for answers"),
                    )
                    .to_string(),
                );
                continue;
            }
            let (sh2, out2, verb, inf) =
                (sh.clone(), out.clone(), verb.to_string(), in_flight.clone());
            let spawned = std::thread::Builder::new()
                .name("verb".into())
                .stack_size(8 * 1024 * 1024)
                .spawn(move || {
                    let budget = params
                        .get("budget")
                        .and_then(Value::as_u64)
                        .map(|b| b as usize);
                    let emit_out = out2.clone();
                    let emit_id = rid.clone();
                    let emit = move |ev: Value| {
                        emit_out.send(json!({"id": emit_id, "event": ev}).to_string());
                    };
                    let ctx = Ctx {
                        d: &sh2.d,
                        sink: Some(out2.clone() as Arc<dyn RawSink>),
                        emit: &emit,
                    };
                    let r = call(&ctx, &verb, &params);
                    let quiet = params.get("quiet").is_some_and(|q| q == true || q == "true");
                    let resp = match r {
                        Ok(v) if quiet => {
                            json!({"id": rid, "result": finalize(terse(v, params.get("fields")), budget)})
                        }
                        Ok(v) => json!({"id": rid, "result": finalize(v, budget)}),
                        Err(e) => err_json(&rid, &e),
                    };
                    out2.send(resp.to_string());
                    inf.fetch_sub(1, Ordering::SeqCst);
                });
            if spawned.is_err() {
                in_flight.fetch_sub(1, Ordering::SeqCst);
            }
            continue;
        }
        // raw CDP: a full-power command, so every scope
        let mut denied = None;
        for s in perms::SCOPES {
            if let Err(e) = authorize(sh, &id, &name, person, s, site.as_deref()) {
                denied = Some(e);
                break;
            }
        }
        if let Some(e) = denied {
            out.send(
                err_json(
                    &rid,
                    &DriveError::new(
                        "not_allowed",
                        format!("raw CDP needs every scope: {}", e.message),
                        e.hint,
                    ),
                )
                .to_string(),
            );
            continue;
        }
        let b = match sh.d.browser() {
            Ok(b) => b,
            Err(e) => {
                out.send(err_json(&rid, &e).to_string());
                continue;
            }
        };
        if let Err(e) = b.forward(out.clone() as Arc<dyn RawSink>, req) {
            out.send(err_json(&rid, &e).to_string());
        }
    }
    let mut a = sh.attached.lock().unwrap_or_else(PoisonError::into_inner);
    a.retain(|(c, _, _)| *c != cid);
    drop(a);
    publish_attached(sh);
}

/// Bind the socket: directory 0700, socket 0600, a stale socket file replaced only if nothing answers on it.
/// The directory is made 0700 when missing and refused when it is not this account's own private
/// directory: with `XDG_RUNTIME_DIR` unset it is `/tmp/atlas-<uid>`, a name any account can make
/// first, and whoever owns the directory owns the socket in it (QA-A10-CS-3).
pub fn bind(path: &std::path::Path) -> std::io::Result<UnixListener> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        private_dir(dir)?;
    }
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "another atlas-drive already serves this socket",
            ));
        }
        let _ = std::fs::remove_file(path);
    }
    let l = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(l)
}

/// `Ok` when `dir` itself (not through a symlink) is a directory of this effective uid with no
/// group or other bits, as `DirBuilder` makes it; otherwise a refusal that names the directory.
/// A directory in sticky `/tmp` or under our own one cannot be swapped by another account after
/// this check.
fn private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(dir)?;
    let me = shared::sys::euid();
    if m.file_type().is_dir() && m.uid() == me && m.mode() & 0o077 == 0 {
        return Ok(());
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!(
            "{} is not a private directory of this account ({}, owner uid {}, mode {:o}; uid {me} \
             and mode 700 needed): not serving from it; remove it or set XDG_RUNTIME_DIR",
            dir.display(),
            if m.file_type().is_symlink() {
                "a symlink"
            } else if m.file_type().is_dir() {
                "a directory"
            } else {
                "not a directory"
            },
            m.uid(),
            m.mode() & 0o7777,
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_adds_the_token_estimate() {
        let v = finalize(json!({"ok": true}), None);
        assert_eq!(v["tokens"], tokens(json!({"ok": true}).to_string().len()));
    }

    #[test]
    fn a_budget_cuts_text_at_line_boundaries_deterministically() {
        let text: String = (0..200).map(|i| format!("line number {i}\n")).collect();
        let a = finalize(json!({"text": text}), Some(60));
        let b = finalize(json!({"text": text}), Some(60));
        assert_eq!(a, b);
        assert_eq!(a["truncated"], true);
        let t = a["text"].as_str().unwrap();
        assert!(t.contains("more lines cut to fit --budget 60"));
        assert!(t.starts_with("line number 0\n"));
        assert!(a["tokens"].as_u64().unwrap() <= 64, "{}", a["tokens"]);
    }

    #[test]
    fn a_batch_keeps_whole_steps_within_budget() {
        let steps: Vec<Value> = (0..20)
            .map(|i| json!({"id": i.to_string(), "ok": true, "r": {"text": "x".repeat(200)}}))
            .collect();
        let v = finalize(json!({"ok": true, "results": steps}), Some(200));
        let kept = v["results"].as_array().unwrap().len();
        assert!(kept > 0 && kept < 20);
        assert_eq!(v["omitted_steps"], 20 - kept);
    }

    #[test]
    fn scopes_follow_the_verb_and_where_it_writes() {
        assert_eq!(scopes_needed("snapshot", &json!({}), &[]), vec!["read"]);
        assert_eq!(
            scopes_needed("screenshot", &json!({"out": "/home/x/.bashrc"}), &[]),
            vec!["files", "read"],
            "a read-only agent cannot write a file where it likes"
        );
        assert_eq!(
            scopes_needed("cdp", &json!({"method": "Page.reload"}), &[]).len(),
            5,
            "raw CDP is every scope"
        );
        let steps = vec![
            ("click".to_string(), json!({})),
            ("network".to_string(), json!({"op": "har", "out": "/tmp/x"})),
        ];
        assert_eq!(
            scopes_needed("batch", &json!({}), &steps),
            vec!["act", "files", "network"]
        );
        assert!(scopes_needed("nope", &json!({}), &[]).is_empty());
    }

    #[test]
    fn quiet_keeps_only_the_answer() {
        let batch = json!({"ok": true, "results": [
            {"id": "0", "verb": "fill", "ok": true, "r": {"ok": true, "focus": "input#email"}},
            {"id": "1", "verb": "click", "ok": true, "r": {"ok": true, "clicked": "button \"Save\""}},
            {"id": "2", "verb": "eval", "ok": true, "r": {"value": "Saved!"}}
        ]});
        assert_eq!(terse(batch, None), json!({"value": "Saved!"}));
        let one = json!({"ok": true, "changed": {"url": "x"}, "text": "hi"});
        assert_eq!(terse(one.clone(), None), json!({"value": "hi"}));
        assert_eq!(
            terse(one, Some(&json!("changed"))),
            json!({"changed": {"url": "x"}})
        );
        let failed = json!({"ok": false, "failed": "1", "results": [{"id": "1", "verb": "click", "error": "not_found: x"}]});
        assert_eq!(
            terse(failed, None),
            json!({"values": [], "failed": "1", "error": "not_found: x"})
        );
    }

    /// QA-A10-CS-3: the socket's directory is this account's own and private, or nothing is
    /// served from it: a directory someone made first (wider mode, a symlink, another owner such
    /// as `/tmp` itself) is refused, never adopted.
    #[test]
    fn bind_refuses_a_fallback_directory_it_does_not_own() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("ad-a10-cs3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        // made by someone else first, open to all (a pre-made /tmp/atlas-<uid>)
        let open = root.join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
        let e = bind(&open.join("drive.sock")).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{e}");
        assert!(e.to_string().contains(&*open.to_string_lossy()), "{e}");
        assert!(!open.join("drive.sock").exists());
        // a group-readable one of ours is not private either
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o750)).unwrap();
        assert!(bind(&open.join("drive.sock")).is_err());
        // a symlink to a directory of ours is not that directory
        let real = root.join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(bind(&link.join("drive.sock")).is_err());
        // another owner: /tmp is root's (1777)
        if shared::sys::euid() != 0 {
            let p = std::env::temp_dir().join(format!("ad-a10-cs3-{}.sock", std::process::id()));
            assert!(bind(&p).is_err());
            assert!(!p.exists());
        }
        // ours and 0700, made now or before: served
        assert!(bind(&real.join("drive.sock")).is_ok());
        assert!(bind(&root.join("fresh/drive.sock")).is_ok());
        let mode = std::fs::metadata(root.join("fresh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn errors_are_one_line() {
        let e = err_json(
            &json!(3),
            &DriveError::new("stale_ref", "unknown ref e9", "run snapshot"),
        );
        assert_eq!(e["error"]["message"], "unknown ref e9; run snapshot");
        assert_eq!(e["error"]["data"]["code"], "stale_ref");
        assert_eq!(e["id"], 3);
    }
}
