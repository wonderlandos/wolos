// SPDX-License-Identifier: BSD-2-Clause
//! The `bluetoothctl` process behind a pairing ([`crate::bluetooth::run_pair`]): the OS side of
//! [`crate::bluetooth::Console`], so the pairing logic stays testable without a machine.
use crate::bluetooth::{Console, LineSplitter, Recv};

/// The real `bluetoothctl`, one process per pairing, killed when dropped.
pub struct BluetoothctlProc {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    rx: std::sync::mpsc::Receiver<String>,
}

impl BluetoothctlProc {
    /// Starts `bluetoothctl` with its output read on a named thread; None when it cannot start.
    pub fn spawn() -> Option<Self> {
        use std::io::Read;
        use std::process::{Command, Stdio};
        let mut child = Command::new("bluetoothctl")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take();
        let mut out = child.stdout.take()?;
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::Builder::new().name("bluetoothctl-out".into());
        let started = reader.spawn(move || {
            let mut split = LineSplitter::default();
            let mut buf = [0u8; 4096];
            while let Ok(n) = out.read(&mut buf) {
                if n == 0 {
                    break;
                }
                for l in split.push(&buf[..n]) {
                    if tx.send(l).is_err() {
                        return;
                    }
                }
            }
        });
        if started.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        Some(Self { child, stdin, rx })
    }
}

impl Console for BluetoothctlProc {
    fn send(&mut self, line: &str) -> bool {
        use std::io::Write;
        self.stdin
            .as_mut()
            .is_some_and(|s| writeln!(s, "{line}").and_then(|_| s.flush()).is_ok())
    }
    fn recv(&mut self, wait: std::time::Duration) -> Recv {
        match self.rx.recv_timeout(wait) {
            Ok(l) => Recv::Line(l),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Recv::Idle,
            Err(_) => Recv::Closed,
        }
    }
}

// Closing stdin ends the session; the kill and wait reap a bluetoothctl that did not quit.
impl Drop for BluetoothctlProc {
    fn drop(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
