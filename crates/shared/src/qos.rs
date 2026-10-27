//! QoS classes. Raw values are ABI (kernel + wire); never renumber.

/// macOS-style QoS class. `as u8` is the raw ABI value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum QosClass {
    UserInteractive = 0,
    UserInitiated = 1,
    Default = 2,
    Utility = 3,
    Background = 4,
}

/// FreeBSD scheduling policy a class maps to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedPolicy {
    /// rtprio(1) realtime priority.
    RtPrio(u8),
    /// nice(1) value.
    Nice(i8),
    /// idprio(1) idle priority.
    IdPrio(u8),
}

impl QosClass {
    pub const ALL: [QosClass; 5] = [
        QosClass::UserInteractive,
        QosClass::UserInitiated,
        QosClass::Default,
        QosClass::Utility,
        QosClass::Background,
    ];

    pub fn from_raw(raw: u8) -> Option<Self> {
        Self::ALL.get(raw as usize).copied()
    }

    pub fn policy(self) -> SchedPolicy {
        match self {
            QosClass::UserInteractive => SchedPolicy::RtPrio(8),
            QosClass::UserInitiated => SchedPolicy::Nice(-10),
            QosClass::Default => SchedPolicy::Nice(0),
            QosClass::Utility => SchedPolicy::Nice(10),
            QosClass::Background => SchedPolicy::IdPrio(16),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            QosClass::UserInteractive => "user-interactive",
            QosClass::UserInitiated => "user-initiated",
            QosClass::Default => "default",
            QosClass::Utility => "utility",
            QosClass::Background => "background",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|q| q.name() == name)
    }
}

/// Compositor/realtime -> wold: one ASCII line `foreground <pid>`.
pub const SOCKET: &str = "/var/run/wol-qos.sock";

/// Parse `foreground <pid>` (trailing CR/LF/space allowed). Does not allocate.
pub fn parse_foreground(line: &[u8]) -> Option<i32> {
    let line = line.trim_ascii();
    let digits = line.strip_prefix(b"foreground ")?;
    if digits.is_empty() || digits.len() > 10 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let pid = digits
        .iter()
        .try_fold(0i64, |a, d| Some(a * 10 + i64::from(d - b'0')))?;
    (pid > 0 && pid <= i64::from(i32::MAX)).then_some(pid as i32)
}

/// Write the wire line into `buf`; returns its length. Does not allocate.
pub fn format_foreground(pid: i32, buf: &mut [u8; 24]) -> usize {
    use std::io::Write;
    let mut w = &mut buf[..];
    let _ = writeln!(w, "foreground {pid}");
    24 - w.len()
}

/// Frontmost app gets UserInteractive; the one it replaces returns to Default.
#[derive(Default, Debug)]
pub struct ForegroundTracker {
    current: i32,
}

impl ForegroundTracker {
    /// (demoted, promoted); 0 = none. Allocation-free.
    pub fn switch(&mut self, pid: i32) -> (i32, i32) {
        if pid == self.current {
            return (0, 0);
        }
        let demoted = self.current;
        self.current = pid;
        (demoted, pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_abi_and_policy_table() {
        let table = [
            (0, QosClass::UserInteractive, SchedPolicy::RtPrio(8)),
            (1, QosClass::UserInitiated, SchedPolicy::Nice(-10)),
            (2, QosClass::Default, SchedPolicy::Nice(0)),
            (3, QosClass::Utility, SchedPolicy::Nice(10)),
            (4, QosClass::Background, SchedPolicy::IdPrio(16)),
        ];
        for (raw, class, policy) in table {
            assert_eq!(class as u8, raw);
            assert_eq!(QosClass::from_raw(raw), Some(class));
            assert_eq!(class.policy(), policy);
            assert_eq!(QosClass::parse(class.name()), Some(class));
        }
        assert_eq!(QosClass::from_raw(5), None);
    }

    #[test]
    fn foreground_wire_table() {
        let cases: &[(&[u8], Option<i32>)] = &[
            (b"foreground 42\n", Some(42)),
            (b"foreground 42", Some(42)),
            (b"foreground 42\r\n", Some(42)),
            (b"foreground 0", None),
            (b"foreground -3", None),
            (b"foreground x", None),
            (b"foreground ", None),
            (b"foreground 99999999999", None),
            (b"background 42", None),
            (b"", None),
        ];
        for (line, want) in cases {
            assert_eq!(
                parse_foreground(line),
                *want,
                "{:?}",
                String::from_utf8_lossy(line)
            );
        }
        let mut buf = [0u8; 24];
        let n = format_foreground(1234, &mut buf);
        assert_eq!(&buf[..n], b"foreground 1234\n");
        assert_eq!(parse_foreground(&buf[..n]), Some(1234));
    }

    #[test]
    fn tracker_demotes_previous() {
        let mut t = ForegroundTracker::default();
        assert_eq!(t.switch(10), (0, 10));
        assert_eq!(t.switch(10), (0, 0));
        assert_eq!(t.switch(20), (10, 20));
    }
}
