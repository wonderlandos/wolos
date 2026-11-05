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

/// `Ok` when the process serving the socket runs as `me`: whoever answers on a path is taken as
/// the browser's truth, and a directory another account made first (`/tmp/atlas-<uid>` with
/// `XDG_RUNTIME_DIR` unset) would let that account answer (QA-A10-CS-3).
fn same_account(peer: Option<shared::sys::Peer>, me: u32, path: &Path) -> std::io::Result<()> {
    match peer {
        Some(p) if p.uid == me => Ok(()),
        other => Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "the process on {} is {}, not this account (uid {me}): not talking to it",
                path.display(),
                other.map_or_else(|| "an unknown account".into(), |p| format!("uid {}", p.uid)),
            ),
        )),
    }
}

impl Client {
    pub fn connect(path: &Path) -> std::io::Result<Client> {
        let s = UnixStream::connect(path)?;
        same_account(shared::sys::peer(&s), shared::sys::euid(), path)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use shared::sys::Peer;

    /// QA-A10-CS-3: only a daemon of this account is the daemon.
    #[test]
    fn a_daemon_of_another_account_is_refused() {
        let p = Path::new("/tmp/atlas-1000/drive.sock");
        assert!(same_account(Some(Peer { pid: 7, uid: 1000 }), 1000, p).is_ok());
        for peer in [
            Some(Peer { pid: 7, uid: 1001 }),
            Some(Peer { pid: 7, uid: 0 }),
            None,
        ] {
            let e = same_account(peer, 1000, p).unwrap_err();
            assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{peer:?}");
            assert!(e.to_string().contains("/tmp/atlas-1000/drive.sock"), "{e}");
        }
    }
}
