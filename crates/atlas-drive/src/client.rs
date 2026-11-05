// SPDX-License-Identifier: GPL-2.0-or-later
//! The client library: one connection to the Drive socket, requests and replies by id. Used by the
//! CLI, the MCP bridge, the benchmark and the tests.
use crate::cdp::{Framer, MAX_BROWSER_MSG};
use serde_json::{Value, json};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;

pub struct Client {
    w: UnixStream,
    r: Framer<UnixStream>,
    next: u64,
}

impl Client {
    pub fn connect(path: &Path) -> std::io::Result<Client> {
        let s = UnixStream::connect(path)?;
        let w = s.try_clone()?;
        Ok(Client {
            w,
            r: Framer::new(s, b'\n', MAX_BROWSER_MSG),
            next: 1,
        })
    }

    /// Send a request and wait for its reply; streamed lines (batch events, raw CDP events) go to `on_line`.
    pub fn request(
        &mut self,
        method: &str,
        params: Value,
        on_line: &mut dyn FnMut(&Value),
    ) -> Result<Value, String> {
        let id = self.next;
        self.next += 1;
        let mut req = json!({"id": id, "method": method});
        if !params.is_null() {
            req["params"] = params;
        }
        let mut line = req.to_string();
        line.push('\n');
        self.w
            .write_all(line.as_bytes())
            .map_err(|e| format!("cannot write to the daemon: {e}"))?;
        loop {
            let raw = self
                .r
                .next_msg()
                .map_err(|e| format!("cannot read from the daemon: {e}"))?
                .ok_or("the daemon closed the connection")?;
            let v: Value = serde_json::from_slice(&raw).map_err(|e| format!("bad reply: {e}"))?;
            if v.get("id") == Some(&json!(id))
                && (v.get("result").is_some() || v.get("error").is_some())
            {
                if let Some(e) = v.get("error") {
                    return Err(e["message"].as_str().unwrap_or("error").to_string());
                }
                return Ok(v["result"].clone());
            }
            if v.get("id").is_none() && v.get("error").is_some() {
                return Err(v["error"]["message"]
                    .as_str()
                    .unwrap_or("error")
                    .to_string());
            }
            on_line(&v);
        }
    }

    pub fn verb(&mut self, verb: &str, params: Value) -> Result<Value, String> {
        self.request(&format!("Drive.{verb}"), params, &mut |_| {})
    }

    /// Raw CDP on the same socket.
    pub fn cdp(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.request(method, params, &mut |_| {})
    }
}
