// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol mcp`: Model Context Protocol server on stdin/stdout (JSON-RPC 2.0, one message per line;
//! no network listener). Tools are the helper catalogue ([`catalog::tools`]); a call builds the same
//! argv the CLI takes and runs the same handlers, so CLI and MCP cannot drift. Every call passes
//! the helper gate ([`approve`]: policy, then the approval sheet for changes) and lands in the audit
//! log once. Administrator verbs never run here: wol-shell asks for the password and runs them.
use crate::approve::{self, Verdict};
use crate::catalog;
use crate::run::Env;
use serde_json::{Value, json};
use std::io;
use std::path::{Path, PathBuf};
use wol_core::automation::{Outcome, audit};

/// Newest first; an unknown requested version gets the newest.
pub const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "WolOS system tools. Read state with the read-only tools before changing it. \
Every change asks for the user's approval on screen first (Allow Once, Always Allow, Deny; administrator changes \
also need their password), so a call that changes something waits up to a minute for the answer. \
A declined or unanswered request changes nothing: tell the user, do not retry on your own. \
Settings changes are journaled: `history` lists them and `undo` reverts one. `files_tidy`: call \
it without a plan first (dry run), show the moves to the user, then call it with that plan id.";

pub struct Server {
    client: String,
    /// The executable that started `wol mcp`, as the kernel reports it (audit C10-6); None only
    /// for a server built without one ([`Server::default`], tests of the protocol itself).
    program: Option<String>,
    pub version: &'static str,
    pub initialized: bool,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            client: "MCP client".into(),
            program: None,
            version: VERSIONS[0],
            initialized: false,
        }
    }
}

/// Shown for a client whose starter could not be read (it exited, or is not this account's):
/// no grant made for a known program ever matches it.
pub const UNKNOWN_PROGRAM: &str = "unknown program";

/// The executable of the process that started `wol mcp` (the MCP client), as the kernel reports
/// it; None when it cannot be told. STANDARDS 7.1 "Identity": the parent's `/proc/<ppid>` is
/// pinned as a directory first and the executable read through that descriptor. The pid is
/// checked to still be this process's parent after the open: while it is, the parent is alive,
/// so the pid cannot have been reused and the directory is the client, not a newcomer. A parent
/// that already exited (`wol mcp` handed to init or a subreaper) is unknown, never the reaper.
pub fn parent_program() -> Option<String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let ppid = std::os::unix::process::parent_id();
    if ppid <= 1 {
        return None;
    }
    let dir = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(format!("/proc/{ppid}"))
        .ok()?;
    if std::os::unix::process::parent_id() != ppid {
        return None;
    }
    std::fs::read_link(format!("/proc/self/fd/{}/exe", dir.as_raw_fd()))
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

fn ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Tool-level failure: the model sees it and can correct itself.
fn tool_error(text: &str) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": true})
}

/// Client name as shown in Settings and the audit log: printable, at most 64 characters.
fn client_name(v: &Value) -> String {
    let n: String = v
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect();
    if n.trim().is_empty() {
        "MCP client".into()
    } else {
        n.trim().to_string()
    }
}

impl Server {
    /// A server for a client started by `program` (its executable path, or None when it could
    /// not be read, which is [`UNKNOWN_PROGRAM`]).
    pub fn started_by(program: Option<String>) -> Self {
        Self {
            program: Some(program.unwrap_or_else(|| UNKNOWN_PROGRAM.into())),
            ..Self::default()
        }
    }

    pub fn client(&self) -> &str {
        &self.client
    }

    /// Who the policy and the approval sheet name (audit C10-6): the name the client declares
    /// for itself, bound to the program that started it, `Assistant (/usr/bin/assistant)`. An
    /// Always Allow given to one program is not inherited by another that copies its name. At
    /// most [`shared::portal::MAX_HELPER_TEXT`] bytes (wold relays it): a long path keeps its end.
    pub fn helper(&self) -> String {
        let Some(program) = &self.program else {
            return self.client.clone();
        };
        let program: String = program.chars().filter(|c| !c.is_control()).collect();
        let room = shared::portal::MAX_HELPER_TEXT.saturating_sub(self.client.len() + 3);
        let shown = if program.len() <= room {
            program
        } else {
            let mut tail: Vec<char> = Vec::new();
            let mut used = '…'.len_utf8();
            for c in program.chars().rev() {
                if used + c.len_utf8() > room {
                    break;
                }
                used += c.len_utf8();
                tail.push(c);
            }
            std::iter::once('…').chain(tail.into_iter().rev()).collect()
        };
        format!("{} ({shown})", self.client)
    }

    /// One JSON-RPC message in; a response for requests, None for notifications and responses.
    pub fn handle(&mut self, line: &str, env: &mut dyn Env) -> Option<Value> {
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return Some(error(Value::Null, -32700, "Parse error"));
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str);
        let Some(method) = method.filter(|_| msg["jsonrpc"] == "2.0") else {
            // We send no requests, so a result/error from the client needs no answer.
            if msg.get("result").is_some() || msg.get("error").is_some() {
                return None;
            }
            return Some(error(id.unwrap_or(Value::Null), -32600, "Invalid Request"));
        };
        let Some(id) = id else {
            if method == "notifications/initialized" {
                self.initialized = true;
            }
            return None;
        };
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
        Some(match method {
            "initialize" => ok(id, self.initialize(&params)),
            "ping" => ok(id, json!({})),
            "tools/list" => {
                let tools: Vec<Value> = catalog::tools().iter().map(catalog::tool_json).collect();
                ok(id, json!({"tools": tools}))
            }
            "tools/call" => match self.call(&params, env) {
                Ok(r) => ok(id, r),
                Err((code, m)) => error(id, code, &m),
            },
            _ => error(id, -32601, &format!("Method not found: {method}")),
        })
    }

    fn initialize(&mut self, p: &Value) -> Value {
        let asked = p["protocolVersion"].as_str().unwrap_or("");
        self.version = VERSIONS
            .iter()
            .find(|v| **v == asked)
            .copied()
            .unwrap_or(VERSIONS[0]);
        self.client = client_name(&p["clientInfo"]["name"]);
        json!({
            "protocolVersion": self.version,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "wolos", "title": "WolOS", "version": env!("CARGO_PKG_VERSION")},
            "instructions": INSTRUCTIONS,
        })
    }

    fn call(&self, p: &Value, env: &mut dyn Env) -> Result<Value, (i64, String)> {
        let name = p["name"]
            .as_str()
            .ok_or((-32602, "missing tool name".to_string()))?;
        let verb = catalog::find_tool(name).ok_or((-32602, format!("Unknown tool: {name}")))?;
        let rule = verb.helper.expect("tools are helper verbs");
        let tool = verb.cmd.path.join(" ");
        let log = |env: &mut dyn Env, args: &[&str], o: Outcome, detail: &str| {
            self.log(env, &tool, args, o, detail)
        };
        let args = match &p["arguments"] {
            Value::Object(m) => m.clone(),
            Value::Null => serde_json::Map::new(),
            _ => return Ok(tool_error("arguments must be an object")),
        };
        let (argv, secret) = match catalog::argv(&verb, &args) {
            Ok(x) => x,
            Err(e) => return Ok(tool_error(&e)),
        };
        let inv = match crate::cli::parse(&argv) {
            Ok(i) => i,
            Err(e) => return Ok(tool_error(&e.to_string())),
        };
        let shown: Vec<&str> = inv.args.iter().map(|a| a.1.as_str()).collect();
        let helper = self.helper();
        let (approved, note) = match approve::decide(env, &helper, &tool, rule, &inv, secret) {
            Verdict::Stop(o, msg) => {
                log(env, &shown, o, &msg);
                return Ok(tool_error(&msg));
            }
            Verdict::Done(what) => {
                log(env, &shown, Outcome::Approved, &what);
                let text = format!("Done: the user allowed it and WolOS did it ({what}).");
                return Ok(json!({
                    "content": [{"type": "text", "text": text}],
                    "structuredContent": {"status": "done", "request": what},
                    "isError": false,
                }));
            }
            Verdict::Run { approved, note } => (approved, note),
        };
        let actor = format!("mcp:{}", self.client);
        let mut te = ToolEnv::new(env, actor);
        let code = crate::run::dispatch(&inv, &mut te);
        let (out, err) = (te.out, te.err);
        if code != 0 {
            let msg = err.trim().trim_start_matches("wol: ").trim().to_string();
            let msg = if msg.is_empty() {
                format!("failed (exit {code})")
            } else {
                msg
            };
            log(env, &shown, Outcome::Failed, &msg);
            return Ok(tool_error(&msg));
        }
        let o = if approved {
            Outcome::Approved
        } else {
            Outcome::Ok
        };
        log(env, &shown, o, note);
        let text = out.trim_end().to_string();
        let mut r = json!({"content": [{"type": "text", "text": text}], "isError": false});
        match serde_json::from_str::<Value>(&text) {
            Ok(v @ Value::Object(_)) => r["structuredContent"] = v,
            Ok(v @ Value::Array(_)) => r["structuredContent"] = json!({"items": v}),
            _ => {}
        }
        Ok(r)
    }

    fn log(&self, env: &mut dyn Env, tool: &str, args: &[&str], o: Outcome, detail: &str) {
        let r = crate::verbs::record(&self.client, "mcp", tool, args, o, detail, env.now_secs());
        let _ = audit::append(&env.state_home(), &r);
    }
}

/// Serves until stdin closes.
pub fn serve(env: &mut dyn Env) -> i32 {
    let mut s = Server::started_by(parent_program());
    while let Some(line) = env.read_line() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(r) = s.handle(l, env) {
            env.out(&format!(
                "{}\n",
                serde_json::to_string(&r).unwrap_or_default()
            ));
        }
    }
    0
}

/// Env for one tool call: output captured (stdout is the protocol), no stdin, actor = the client,
/// children's stdout kept off the protocol stream.
pub struct ToolEnv<'a> {
    inner: &'a mut dyn Env,
    actor: String,
    pub out: String,
    pub err: String,
}

impl<'a> ToolEnv<'a> {
    pub fn new(inner: &'a mut dyn Env, actor: String) -> Self {
        Self {
            inner,
            actor,
            out: String::new(),
            err: String::new(),
        }
    }
}

impl Env for ToolEnv<'_> {
    fn out(&mut self, s: &str) {
        self.out.push_str(s);
    }
    fn err(&mut self, s: &str) {
        self.err.push_str(s);
    }
    fn stdout_tty(&self) -> bool {
        false
    }
    fn var(&self, name: &str) -> Option<String> {
        self.inner.var(name)
    }
    fn is_root(&self) -> bool {
        self.inner.is_root()
    }
    fn request(&mut self, socket: &Path, req: &[u8]) -> io::Result<String> {
        self.inner.request(socket, req)
    }
    fn request_wait(&mut self, socket: &Path, req: &[u8], secs: u32) -> io::Result<String> {
        self.inner.request_wait(socket, req, secs)
    }
    fn exec(&mut self, argv: &[String]) -> io::Result<i32> {
        self.inner.exec_quiet(argv)
    }
    fn exec_quiet(&mut self, argv: &[String]) -> io::Result<i32> {
        self.inner.exec_quiet(argv)
    }
    fn exec_output(&mut self, argv: &[String]) -> io::Result<(i32, String)> {
        self.inner.exec_output(argv)
    }
    fn capture(&mut self, argv: &[&str]) -> Option<String> {
        self.inner.capture(argv)
    }
    fn try_lock(&mut self, path: &Path) -> io::Result<Option<crate::run::Lock>> {
        self.inner.try_lock(path)
    }
    fn stdin_line(&mut self) -> Option<String> {
        None
    }
    fn read_line(&mut self) -> Option<String> {
        None
    }
    fn read_file(&self, p: &Path) -> Option<String> {
        self.inner.read_file(p)
    }
    fn sys_topic(&mut self, topic: &str) -> Option<Value> {
        self.inner.sys_topic(topic)
    }
    fn now_secs(&self) -> u64 {
        self.inner.now_secs()
    }
    fn config_home(&self) -> PathBuf {
        self.inner.config_home()
    }
    fn controls(&self) -> wol_core::controls::Controls {
        self.inner.controls()
    }
    fn state_home(&self) -> PathBuf {
        self.inner.state_home()
    }
    fn home(&self) -> PathBuf {
        self.inner.home()
    }
    fn actor(&self) -> String {
        self.actor.clone()
    }
}
