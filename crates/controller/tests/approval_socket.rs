// SPDX-License-Identifier: GPL-2.0-or-later
//! End to end through a real Unix socket: `wol mcp` (the built binary) -> fake wol-shell on
//! `$XDG_RUNTIME_DIR/wol-shell.sock` -> answer -> the change runs (or not) -> audit log.
//! The child gets a temp home and an empty PATH: nothing touches this machine's desktop.
use serde_json::{Value, json};
use shared::shell::{Answer, Approval, ApprovalRequest};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-approval-sock-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for sub in ["run", "config", "state", "bin"] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
    }
    d
}

/// Fake wol-shell: answers each `approve` line with `answer` and records it; other lines (a
/// `reload`) get no reply. Serves until stopped; reads time out, so it never hangs.
type Shell = (Arc<AtomicBool>, std::thread::JoinHandle<Vec<Approval>>);

fn fake_shell(dir: &Path, answer: Answer) -> Shell {
    let l = UnixListener::bind(dir.join("run/wol-shell.sock")).unwrap();
    l.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let st = stop.clone();
    let h = std::thread::spawn(move || {
        let mut seen = Vec::new();
        while !st.load(Ordering::SeqCst) {
            let Ok((mut s, _)) = l.accept() else {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            };
            s.set_nonblocking(false).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut line = String::new();
            let _ = BufReader::new(&s).read_line(&mut line);
            if let Some(j) = line.strip_prefix("approve ") {
                seen.push(Approval::parse(j).expect("approve json"));
                s.write_all(answer.line().as_bytes()).unwrap();
            }
        }
        seen
    });
    (stop, h)
}

/// Stops the fake shell; what it was asked.
fn finish(shell: Shell) -> Vec<Approval> {
    shell.0.store(true, Ordering::SeqCst);
    shell.1.join().unwrap()
}

/// Runs `wol mcp` with these messages on stdin; the responses, in order.
fn mcp(dir: &Path, calls: &[Value]) -> Vec<Value> {
    let mut msgs = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"Claude Code","version":"2"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    ];
    msgs.extend_from_slice(calls);
    let mut child = Command::new(env!("CARGO_BIN_EXE_wol"))
        .arg("mcp")
        .env_clear()
        .env("HOME", dir)
        .env("PATH", dir.join("bin"))
        .env("XDG_RUNTIME_DIR", dir.join("run"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for m in msgs {
        writeln!(stdin, "{m}").unwrap();
    }
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn dark(id: u32) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"appearance_set","arguments":{"mode":"dark"}}})
}

fn audit(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join("state/wolos/actions.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn allowed_once_runs_and_is_logged() {
    let d = temp("allow");
    let shell = fake_shell(&d, Answer::AllowOnce);
    let r = mcp(&d, &[dark(2)]);
    assert_eq!(r.len(), 2, "{r:?}");
    assert_eq!(r[1]["result"]["isError"], false, "{}", r[1]);
    assert_eq!(r[1]["result"]["structuredContent"]["mode"], "dark");
    assert_eq!(
        std::fs::read_to_string(d.join("config/wolos/theme")).unwrap(),
        "wol-dark\n"
    );
    let seen = finish(shell);
    assert_eq!(
        seen,
        [Approval {
            // C10-6: the declared name, bound to the program that started `wol mcp` (this test).
            helper: format!(
                "Claude Code ({})",
                std::env::current_exe().unwrap().display()
            ),
            tool: "appearance set".into(),
            request: ApprovalRequest::Change {
                capability: "appearance".into(),
                what: "switch to Dark appearance".into()
            }
        }]
    );
    let a = audit(&d);
    assert_eq!(a.len(), 1);
    // QA-PLUG-25: the audit names the program too, as the policy and the sheet do.
    assert_eq!(
        (a[0]["helper"].as_str(), a[0]["result"].as_str()),
        (Some(seen[0].helper.as_str()), Some("approved"))
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn denied_changes_nothing_and_is_logged() {
    let d = temp("deny");
    let shell = fake_shell(&d, Answer::Deny);
    let r = mcp(&d, &[dark(2)]);
    assert_eq!(r[1]["result"]["isError"], true, "{}", r[1]);
    assert!(!d.join("config/wolos/theme").exists());
    assert_eq!(finish(shell).len(), 1);
    assert_eq!(audit(&d)[0]["result"], "declined");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn no_shell_listening_is_refused() {
    let d = temp("noshell");
    let r = mcp(&d, &[dark(2)]);
    assert_eq!(r[1]["result"]["isError"], true);
    assert!(
        r[1]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("No WolOS session is showing")
    );
    assert!(!d.join("config/wolos/theme").exists());
    assert_eq!(audit(&d)[0]["result"], "refused");
    let _ = std::fs::remove_dir_all(&d);
}

/// QA-PLUG-62: the MCP client goes away while its sheet is on screen (here: the `sh` that started
/// `wol mcp` exits after 1 s); a later Allow Once must not run the change. `wol mcp` drops the
/// request when the program that started it is gone, closes the sheet's connection (the shell
/// sees the hang-up) and logs a refusal.
#[test]
fn eof_cancels_pending_request() {
    let d = temp("gone");
    let l = UnixListener::bind(d.join("run/wol-shell.sock")).unwrap();
    // The sheet is on screen and nobody answers; the shell sees whether the requester hangs up
    // (poll reports HUP once the peer has closed both directions; a half-close is not one).
    let shell = std::thread::spawn(move || {
        use std::os::fd::AsRawFd;
        let (s, _) = l.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line).unwrap();
        assert!(line.starts_with("approve "), "{line}");
        let mut p = libc::pollfd {
            fd: s.as_raw_fd(),
            events: 0,
            revents: 0,
        };
        // SAFETY: one valid pollfd, one entry.
        unsafe { libc::poll(&mut p, 1, 20_000) };
        p.revents & (libc::POLLHUP | libc::POLLERR) != 0
    });
    let msgs = [
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                       "clientInfo": {"name": "Gone-client", "version": "1"}}
        }),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        dark(2),
    ];
    let input: String = msgs.iter().map(|m| format!("{m}\n")).collect();
    std::fs::write(d.join("in.jsonl"), input).unwrap();
    // The client: starts `wol mcp` with an open pipe on stdin, then exits after 1 s.
    let start = std::time::Instant::now();
    let mut client = Command::new("sh")
        .arg("-c")
        .arg("(cat \"$1\"; sleep 30) | \"$0\" mcp & sleep 1")
        .arg(env!("CARGO_BIN_EXE_wol"))
        .arg(d.join("in.jsonl"))
        .env_clear()
        .env("HOME", &d)
        .env("PATH", "/usr/bin:/bin")
        .env("XDG_RUNTIME_DIR", d.join("run"))
        .env("XDG_CONFIG_HOME", d.join("config"))
        .env("XDG_STATE_HOME", d.join("state"))
        .env("XDG_DATA_HOME", d.join("data"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    client.wait().unwrap();
    // `wol mcp` inherited the pipe: its end of file is its exit, after it logged the refusal.
    let mut reply = String::new();
    std::io::Read::read_to_string(&mut client.stdout.take().unwrap(), &mut reply).unwrap();
    assert!(reply.contains("\"isError\":true"), "{reply}");
    assert!(
        shell.join().unwrap(),
        "wol mcp closed the sheet's connection"
    );
    assert!(
        !d.join("config/wolos/theme").exists(),
        "the change ran after the client was gone"
    );
    let a = audit(&d);
    assert_eq!(a.len(), 1, "{a:?}");
    assert_eq!(a[0]["result"], "refused", "{a:?}");
    assert!(
        a[0]["detail"].as_str().unwrap_or_default().contains("gone")
            || a[0].to_string().contains("gone"),
        "{a:?}"
    );
    assert!(start.elapsed() < Duration::from_secs(20));
    let _ = std::fs::remove_dir_all(&d);
}
