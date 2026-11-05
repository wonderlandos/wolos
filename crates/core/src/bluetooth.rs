// SPDX-License-Identifier: BSD-2-Clause
//! Bluetooth for Settings: BlueZ through bluetoothctl (no root: the person at the computer may
//! pair, connect and forget). The hcsecd.conf helpers stay until wol-admind drops `bt-pair`.
use crate::system::Host;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BtKind {
    #[default]
    Other,
    Keyboard,
    Mouse,
    Audio,
    Phone,
    Computer,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BtDevice {
    /// "00:11:22:33:44:55", lowercase.
    pub addr: String,
    pub name: String,
    pub kind: BtKind,
    pub paired: bool,
    pub connected: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BtState {
    pub present: bool,
    pub powered: bool,
    pub devices: Vec<BtDevice>,
}

/// Six hex pairs separated by ':'.
pub fn is_valid_addr(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// (start, end) of each `device { ... }` block, end past the closing brace and its newline.
fn blocks(text: &str) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let mut i = 0;
    while let Some(rel) = text[i..].find("device") {
        let start = i + rel;
        let line_start = text[..start].rfind('\n').map_or(0, |n| n + 1);
        let Some(open) = text[start..].find('{').map(|o| start + o) else {
            break;
        };
        let commented = text[line_start..start].contains('#');
        let Some(close) = text[open..].find('}').map(|c| open + c + 1) else {
            break;
        };
        let end = if text[close..].starts_with('\n') {
            close + 1
        } else {
            close
        };
        if !commented && text[start + 6..open].trim().is_empty() {
            v.push((line_start, end));
        }
        i = end;
    }
    v
}

/// One `key value;` entry of a device block.
fn field(block: &str, key: &str) -> Option<String> {
    block.lines().find_map(|l| {
        let l = l.trim().trim_end_matches(';');
        let (k, v) = l.split_once(char::is_whitespace)?;
        (k == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

/// hcsecd.conf: every `device {}` block is a paired device.
pub fn parse_hcsecd(text: &str) -> Vec<BtDevice> {
    blocks(text)
        .into_iter()
        .filter_map(|(s, e)| {
            let b = &text[s..e];
            let addr = field(b, "bdaddr")?.to_ascii_lowercase();
            is_valid_addr(&addr).then(|| BtDevice {
                name: field(b, "name").unwrap_or_else(|| addr.clone()),
                addr,
                paired: true,
                ..Default::default()
            })
        })
        .collect()
}

/// `conf` without the block for `addr` (case-insensitive); everything else byte for byte.
pub fn remove_hcsecd(conf: Option<&str>, addr: &str) -> String {
    let text = conf.unwrap_or_default();
    let mut out = String::new();
    let mut i = 0;
    for (s, e) in blocks(text) {
        let mine = field(&text[s..e], "bdaddr").is_some_and(|a| a.eq_ignore_ascii_case(addr));
        if mine {
            out.push_str(&text[i..s]);
            i = e;
        }
    }
    out.push_str(&text[i..]);
    out
}

/// Add or replace the block for `addr`: no stored link key, no PIN (Secure Simple Pairing).
pub fn upsert_hcsecd(conf: Option<&str>, addr: &str, name: &str) -> String {
    let mut s = remove_hcsecd(conf, addr).trim_end_matches('\n').to_string();
    if !s.is_empty() {
        s.push_str("\n\n");
    }
    let name: String = name
        .chars()
        .filter(|c| *c != '"' && !c.is_control())
        .collect();
    s.push_str(&format!(
        "device {{\n\tbdaddr\t{};\n\tname\t\"{name}\";\n\tkey\tnokey;\n\tpin\tnopin;\n}}\n",
        addr.to_ascii_lowercase()
    ));
    s
}

/// bluetoothctl `devices [Paired|Connected]`: "Device AA:BB:.. Name".
pub fn parse_bluetoothctl_devices(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Device ")?;
            let (addr, name) = rest.split_once(' ').unwrap_or((rest, ""));
            let addr = addr.to_ascii_lowercase();
            is_valid_addr(&addr).then(|| {
                let name = if name.trim().is_empty() {
                    addr.clone()
                } else {
                    name.trim().to_string()
                };
                (addr, name)
            })
        })
        .collect()
}

/// Bluetooth adapters the kernel knows (`/sys/class/bluetooth/hciN`). Every `bluetoothctl` call is
/// gated on this: without an adapter bluetoothd never starts, org.bluez is never on the bus and the
/// client waits for it forever (QA-DEV2-3).
pub fn adapters(h: &dyn Host) -> Vec<String> {
    h.list_dir("/sys/class/bluetooth")
        .into_iter()
        .filter(|p| p.rsplit('/').next().is_some_and(|n| n.starts_with("hci")))
        .collect()
}

/// What Settings and Control Center can say about Bluetooth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BtLink {
    /// No adapter: "No Bluetooth hardware".
    None,
    /// An adapter, but bluetoothd is not running (crashed, stopped): offer Turn On.
    Down,
    /// An adapter and bluetoothd running.
    Up,
}

/// Whether there is an adapter and bluetoothd runs for it (Settings > Bluetooth's three states).
pub fn link(h: &dyn Host) -> BtLink {
    if adapters(h).is_empty() {
        return BtLink::None;
    }
    match h.run(&["systemctl", "is-active", "bluetooth.service"]) {
        Some(t) if t.trim() == "active" => BtLink::Up,
        _ => BtLink::Down,
    }
}

/// `Icon: audio-headset` of `bluetoothctl info`: the device's kind.
pub fn kind_from_icon(icon: &str) -> BtKind {
    let icon = icon.trim();
    if icon.starts_with("audio-") {
        BtKind::Audio
    } else if icon == "input-keyboard" {
        BtKind::Keyboard
    } else if icon == "input-mouse" || icon == "input-tablet" {
        BtKind::Mouse
    } else if icon == "phone" {
        BtKind::Phone
    } else if icon == "computer" {
        BtKind::Computer
    } else {
        BtKind::Other
    }
}

/// The `Icon:` line of `bluetoothctl info ADDR`.
pub fn parse_info_icon(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        l.trim()
            .strip_prefix("Icon:")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    })
}

/// Adapter, power and paired devices (fast; no scan). Without an adapter bluetoothctl is not run.
pub fn read(h: &dyn Host) -> BtState {
    if adapters(h).is_empty() {
        return BtState::default();
    }
    let Some(show) = h.run(&["bluetoothctl", "show"]) else {
        // An adapter, but bluetoothd does not answer (crashed, stopped): present and off, so
        // Settings offers Turn On instead of "No Bluetooth Hardware" (QA-SET-10).
        return BtState {
            present: true,
            ..Default::default()
        };
    };
    let connected: Vec<String> = h
        .run(&["bluetoothctl", "devices", "Connected"])
        .map(|t| {
            parse_bluetoothctl_devices(&t)
                .into_iter()
                .map(|d| d.0)
                .collect()
        })
        .unwrap_or_default();
    BtState {
        present: show.contains("Controller"),
        powered: crate::controls::parse_bluetoothctl(&show).unwrap_or(false),
        devices: h
            .run(&["bluetoothctl", "devices", "Paired"])
            .map(|t| parse_bluetoothctl_devices(&t))
            .unwrap_or_default()
            .into_iter()
            .map(|(addr, name)| BtDevice {
                connected: connected.contains(&addr),
                kind: h
                    .run(&["bluetoothctl", "info", &addr])
                    .and_then(|t| parse_info_icon(&t))
                    .map_or(BtKind::Other, |i| kind_from_icon(&i)),
                addr,
                name,
                paired: true,
            })
            .collect(),
    }
}

/// Nearby devices that are not paired yet (slow: the scan takes ~8 s; call off the UI thread).
/// Nameless devices show their address.
pub fn scan(h: &dyn Host) -> Vec<BtDevice> {
    if adapters(h).is_empty() {
        return Vec::new();
    }
    let _ = h.run_slow(&["bluetoothctl", "--timeout", "8", "scan", "on"]);
    let paired: Vec<String> = h
        .run(&["bluetoothctl", "devices", "Paired"])
        .map(|t| {
            parse_bluetoothctl_devices(&t)
                .into_iter()
                .map(|d| d.0)
                .collect()
        })
        .unwrap_or_default();
    h.run(&["bluetoothctl", "devices"])
        .map(|t| parse_bluetoothctl_devices(&t))
        .unwrap_or_default()
        .into_iter()
        .filter(|(addr, _)| !paired.contains(addr))
        .map(|(addr, name)| BtDevice {
            addr,
            name,
            ..Default::default()
        })
        .collect()
}

/// Pair, trust (reconnects by itself) and connect; or remove. Fixed argv, address checked.
/// This is the root-side plan of `AdminAction::BtPair`; the person at the computer pairs through
/// [`run_pair`], which also answers BlueZ's PIN and confirmation requests.
pub fn pair_argvs(addr: &str) -> Option<[[&str; 3]; 3]> {
    is_valid_addr(addr).then_some([
        ["bluetoothctl", "pair", addr],
        ["bluetoothctl", "trust", addr],
        ["bluetoothctl", "connect", addr],
    ])
}

pub fn forget_argv(addr: &str) -> Option<[&str; 3]> {
    is_valid_addr(addr).then_some(["bluetoothctl", "remove", addr])
}

/// Forget a paired device (BlueZ drops the link key).
pub fn forget(h: &dyn Host, addr: &str) -> bool {
    forget_argv(addr).is_some_and(|a| {
        h.run(&a)
            .is_some_and(|o| !o.contains("Failed") && !o.contains("not available"))
    })
}

/// The sentence for BlueZ's reason in `bluetoothctl` output ("Failed to connect:
/// org.bluez.Error.Failed br-connection-page-timeout"). None: no failure in `out`.
pub fn reason_sentence(out: &str) -> Option<&'static str> {
    let o = out.to_ascii_lowercase();
    if !(o.contains("failed") || o.contains("not available") || o.contains("error")) {
        return None;
    }
    Some(
        if o.contains("page-timeout") || o.contains("timeout") || o.contains("timed out") {
            "The device didn’t answer. Turn it on, bring it close, and try again."
        } else if o.contains("profile-unavailable") || o.contains("profile unavailable") {
            "This computer can’t use what the device offers. Forget it and pair it again."
        } else if o.contains("notready")
            || o.contains("not ready")
            || o.contains("not powered")
            || o.contains("powered off")
        {
            "Bluetooth is off. Turn it on first."
        } else if o.contains("key-missing") || o.contains("key missing") {
            "The pairing was lost. Forget the device and pair it again."
        } else if o.contains("authenticationfailed")
            || o.contains("authenticationrejected")
            || o.contains("authenticationcanceled")
            || o.contains("authenticationtimeout")
        {
            "The device didn’t accept the code. Try pairing again."
        } else if o.contains("not available") || o.contains("notavailable") {
            "That device isn’t in range."
        } else if o.contains("inprogress") || o.contains("in progress") {
            "Bluetooth is busy with another change. Try again in a moment."
        } else {
            "Couldn’t connect. Try again."
        },
    )
}

/// Connect or disconnect a paired device (slow: call off the UI thread). Err is a sentence.
pub fn connect(h: &dyn Host, addr: &str, on: bool) -> Result<(), &'static str> {
    if !is_valid_addr(addr) {
        return Err("That isn’t a Bluetooth device.");
    }
    if adapters(h).is_empty() {
        return Err("This computer has no Bluetooth adapter.");
    }
    let verb = if on { "connect" } else { "disconnect" };
    match h.run_slow(&["bluetoothctl", verb, addr]) {
        Some(o) if o.contains("AlreadyConnected") || o.contains("Already connected") => Ok(()),
        Some(o) => reason_sentence(&o).map_or(Ok(()), Err),
        None => Err("Couldn’t connect. Try again."),
    }
}

// ---- Pairing with a bluetoothctl agent ----

/// A request or result `bluetoothctl` prints while pairing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEvent {
    /// "Agent registered".
    AgentReady,
    /// "Default agent request successful".
    DefaultAgent,
    /// "Failed to register agent object" / "Failed to request default agent".
    AgentFailed,
    /// "Enter PIN code:" (legacy devices: the keyboard shows or expects 4-6 digits).
    NeedPin,
    /// "Enter passkey (number in 0-999999):".
    NeedPasskey,
    /// "Confirm passkey 123456 (yes/no):", or "Accept pairing (yes/no):" without a number.
    Confirm(Option<String>),
    /// "PIN code: 0000" / "Passkey: 123456 (entered 0)": type this on the device.
    Show(String),
    /// "Authorize service UUID (yes/no):" while pairing.
    Authorize,
    /// "Pairing successful" (the agent's word; the device's own `Paired: yes` decides).
    PairingDone,
    /// "[CHG] Device AA:.. Paired: yes" (address) or `Paired: yes` of `info` (None).
    PairedYes(Option<String>),
    /// "Failed to pair: org.bluez.Error.AuthenticationFailed".
    PairFailed(String),
    /// "Changing AA:.. trust succeeded" (address).
    Trusted(String),
    Connected,
    /// "Failed to connect: ..." after a good pairing.
    ConnectFailed(String),
}

/// The answer to an [`AgentEvent`] request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentAnswer {
    Text(String),
    Yes,
    No,
    /// Nothing typed (events that ask nothing).
    Ack,
    Cancel,
}

/// One output line without colour codes, readline markers, `\r` and the `[bluetooth]#` prompt.
pub fn clean_line(raw: &str) -> String {
    let mut out = String::new();
    let mut it = raw.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\x1b' => {
                if it.peek() == Some(&'[') {
                    it.next();
                    for n in it.by_ref() {
                        if n.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
            }
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    let mut s = out.trim();
    while let Some(rest) = s
        .strip_prefix("[bluetooth]#")
        .or_else(|| s.strip_prefix("[bluetoothctl]>"))
    {
        s = rest.trim_start();
    }
    s.to_string()
}

/// What a cleaned output line means for pairing (None: noise like `[CHG] Device ... RSSI`).
///
/// bluetoothctl 5.72 prints `Request confirmation` on its own line and THEN the prompt
/// `[agent] Confirm passkey 123456 (yes/no): `; only the prompt (the line ending in `(yes/no):`)
/// is a request, so the bare line is noise. A line is anchored at its start: a device named
/// "trust succeeded" arrives as `[CHG] Device AA:.. Name: trust succeeded` and means nothing.
pub fn parse_agent_line(line: &str) -> Option<AgentEvent> {
    let l = line.trim();
    let a = l.strip_prefix("[agent]").map_or(l, str::trim);
    let yes_no = a.ends_with("(yes/no):");
    if yes_no {
        if let Some(rest) = a.strip_prefix("Confirm passkey ") {
            let code = rest.split_whitespace().next().unwrap_or("");
            let code = (code.bytes().all(|b| b.is_ascii_digit()) && !code.is_empty())
                .then(|| code.to_string());
            return Some(AgentEvent::Confirm(code));
        }
        if a.starts_with("Accept pairing") {
            return Some(AgentEvent::Confirm(None));
        }
        if a.starts_with("Authorize service ") {
            return Some(AgentEvent::Authorize);
        }
        return None;
    }
    if a == "Enter PIN code:" {
        return Some(AgentEvent::NeedPin);
    }
    if a.starts_with("Enter passkey") && a.ends_with("):") {
        return Some(AgentEvent::NeedPasskey);
    }
    for key in ["PIN code:", "Passkey:"] {
        if let Some(rest) = a.strip_prefix(key)
            && let Some(code) = rest.split_whitespace().next()
        {
            return Some(AgentEvent::Show(code.to_string()));
        }
    }
    if l == "Agent registered" {
        return Some(AgentEvent::AgentReady);
    }
    if l == "Default agent request successful" {
        return Some(AgentEvent::DefaultAgent);
    }
    if l.starts_with("Failed to register agent") || l.starts_with("Failed to request default agent")
    {
        return Some(AgentEvent::AgentFailed);
    }
    if l == "Pairing successful" {
        return Some(AgentEvent::PairingDone);
    }
    if let Some(r) = l.strip_prefix("Failed to pair:") {
        return Some(AgentEvent::PairFailed(r.trim().to_string()));
    }
    if l == "Paired: yes" {
        return Some(AgentEvent::PairedYes(None));
    }
    if let Some(rest) = l.strip_prefix("[CHG] Device ")
        && let Some((addr, prop)) = rest.split_once(' ')
        && prop == "Paired: yes"
        && is_valid_addr(&addr.to_ascii_lowercase())
    {
        return Some(AgentEvent::PairedYes(Some(addr.to_ascii_lowercase())));
    }
    if let Some(rest) = l.strip_prefix("Changing ")
        && let Some(addr) = rest.strip_suffix(" trust succeeded")
        && is_valid_addr(&addr.to_ascii_lowercase())
    {
        return Some(AgentEvent::Trusted(addr.to_ascii_lowercase()));
    }
    if l == "Connection successful" {
        return Some(AgentEvent::Connected);
    }
    if let Some(r) = l.strip_prefix("Failed to connect:") {
        return Some(AgentEvent::ConnectFailed(r.trim().to_string()));
    }
    if l.starts_with("Device ") && l.ends_with(" not available") {
        return Some(AgentEvent::PairFailed("NotAvailable".into()));
    }
    None
}

/// Splits `bluetoothctl` output into lines. A prompt waits for input without a newline
/// ("... (yes/no): "), so a tail that looks like one is a line too.
#[derive(Default)]
pub struct LineSplitter {
    buf: Vec<u8>,
}

impl LineSplitter {
    /// Adds `bytes`; returns the lines they complete, a prompt without its newline included.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(i) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            out.push(String::from_utf8_lossy(&line).into_owned());
        }
        let tail = String::from_utf8_lossy(&self.buf).into_owned();
        let t = clean_line(&tail);
        if t.ends_with("(yes/no):") || t.ends_with("):") || t.ends_with("code:") {
            out.push(tail);
            self.buf.clear();
        }
        out
    }
}

/// An interactive `bluetoothctl` (or a script of one in tests).
pub trait Console {
    fn send(&mut self, line: &str) -> bool;
    /// The next output line; `Idle` after `wait` without one, `Closed` when it exited.
    fn recv(&mut self, wait: std::time::Duration) -> Recv;
}

/// What [`Console::recv`] got: a line, nothing within the wait, or the end of the process.
pub enum Recv {
    Line(String),
    Idle,
    Closed,
}

/// How a pairing ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairOutcome {
    /// Paired and trusted. `connected`: the first connection worked; `note` says why not.
    Paired {
        connected: bool,
        note: Option<&'static str>,
    },
    /// The person closed the sheet.
    Cancelled,
    Failed(&'static str),
}

/// How often [`run_pair`] looks for bluetoothctl's next line.
pub const PAIR_TICK: std::time::Duration = std::time::Duration::from_millis(200);

/// How long pairing waits for the device or the person before it gives up.
pub const PAIR_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

/// Pairs `addr` through a bluetoothctl agent that asks `ask` for PINs and confirmations. `ask`
/// blocks until the person answers (call this off the UI thread); it also gets `Show` events,
/// which need only `Ack`.
///
/// Order, as bluetoothctl 5.72 needs it: `agent KeyboardDisplay`, wait for "Agent registered",
/// `default-agent`, wait for "Default agent request successful", only then `pair` (an earlier
/// `pair` has no agent to ask). The device must report `Paired: yes` before it is trusted and
/// the pairing counts. The idle timer restarts only on lines that mean something, never on the
/// RSSI/property chatter of a device in range.
pub fn run_pair(
    c: &mut dyn Console,
    addr: &str,
    wait: std::time::Duration,
    ask: &mut dyn FnMut(&AgentEvent) -> AgentAnswer,
) -> PairOutcome {
    if !is_valid_addr(addr) {
        return PairOutcome::Failed("That isn’t a Bluetooth device.");
    }
    let addr = addr.to_ascii_lowercase();
    if !c.send("agent KeyboardDisplay") {
        return PairOutcome::Failed("Bluetooth didn’t answer. Try again.");
    }
    let tick = PAIR_TICK;
    let mut idle = std::time::Duration::ZERO;
    let (mut pair_sent, mut paired, mut info_sent) = (false, false, false);
    let mut trust_sent = false;
    let mut connect_sent = false;
    let finish = |c: &mut dyn Console, o: PairOutcome| {
        if matches!(o, PairOutcome::Cancelled | PairOutcome::Failed(_)) {
            c.send(&format!("cancel-pairing {addr}"));
        }
        c.send("quit");
        o
    };
    let mine = |a: &Option<String>| a.as_ref().is_none_or(|x| *x == addr);
    loop {
        let line = match c.recv(tick) {
            Recv::Line(l) => l,
            Recv::Idle => {
                idle += tick;
                if idle >= wait {
                    return finish(c, gave_up(paired, pair_sent));
                }
                continue;
            }
            Recv::Closed => return closed(paired),
        };
        let Some(ev) = parse_agent_line(&clean_line(&line)) else {
            continue;
        };
        idle = std::time::Duration::ZERO;
        match &ev {
            AgentEvent::AgentReady => {
                c.send("default-agent");
            }
            AgentEvent::DefaultAgent => {
                if !pair_sent {
                    pair_sent = true;
                    c.send(&format!("pair {addr}"));
                }
            }
            AgentEvent::AgentFailed => {
                return finish(
                    c,
                    PairOutcome::Failed("Bluetooth didn’t answer. Try again."),
                );
            }
            AgentEvent::NeedPin | AgentEvent::NeedPasskey => match ask(&ev) {
                AgentAnswer::Text(t) => {
                    let t: String = t.chars().filter(|c| !c.is_control()).collect();
                    c.send(&t);
                }
                _ => return finish(c, PairOutcome::Cancelled),
            },
            AgentEvent::Confirm(_) => match ask(&ev) {
                AgentAnswer::Yes => {
                    c.send("yes");
                }
                _ => {
                    c.send("no");
                    return finish(c, PairOutcome::Cancelled);
                }
            },
            AgentEvent::Show(_) => {
                if ask(&ev) == AgentAnswer::Cancel {
                    return finish(c, PairOutcome::Cancelled);
                }
            }
            AgentEvent::Authorize => {
                // Only while this session is the default agent, for the device just paired.
                c.send("yes");
            }
            AgentEvent::PairingDone => {
                // The agent says so; the device's own `Paired: yes` is what counts.
                if !paired && !info_sent {
                    info_sent = true;
                    c.send(&format!("info {addr}"));
                }
            }
            AgentEvent::PairedYes(a) if mine(a) => {
                paired = true;
                if !trust_sent {
                    trust_sent = true;
                    c.send(&format!("trust {addr}"));
                }
            }
            AgentEvent::PairedYes(_) => {}
            AgentEvent::Trusted(a) if *a == addr => {
                if !connect_sent {
                    connect_sent = true;
                    c.send(&format!("connect {addr}"));
                }
            }
            AgentEvent::Trusted(_) => {}
            AgentEvent::Connected => {
                if paired {
                    return finish(
                        c,
                        PairOutcome::Paired {
                            connected: true,
                            note: None,
                        },
                    );
                }
            }
            AgentEvent::ConnectFailed(r) => {
                if paired {
                    return finish(
                        c,
                        PairOutcome::Paired {
                            connected: false,
                            note: reason_sentence(&format!("Failed {r}")),
                        },
                    );
                }
            }
            AgentEvent::PairFailed(r) => {
                if r.contains("AlreadyExists") {
                    // Paired before: ask what the device says, then trust and connect it.
                    if !info_sent {
                        info_sent = true;
                        c.send(&format!("info {addr}"));
                    }
                    continue;
                }
                return finish(c, PairOutcome::Failed(pair_failure(r)));
            }
        }
    }
}

/// How a pairing ends when nothing meaningful came for the whole wait.
fn gave_up(paired: bool, pair_sent: bool) -> PairOutcome {
    if paired {
        PairOutcome::Paired {
            connected: false,
            note: Some("Paired, but it didn’t connect. Try Connect."),
        }
    } else if !pair_sent {
        PairOutcome::Failed("Bluetooth didn’t answer. Try again.")
    } else {
        PairOutcome::Failed("The device didn’t answer. Put it in pairing mode and try again.")
    }
}

/// How a pairing ends when bluetoothctl exited under it.
fn closed(paired: bool) -> PairOutcome {
    if paired {
        PairOutcome::Paired {
            connected: false,
            note: None,
        }
    } else {
        PairOutcome::Failed("Bluetooth stopped answering. Try again.")
    }
}

fn pair_failure(reason: &str) -> &'static str {
    let r = reason.to_ascii_lowercase();
    if r.contains("authenticationfailed") || r.contains("authenticationrejected") {
        "The device didn’t accept the code. Try pairing again."
    } else if r.contains("authenticationcanceled") {
        "Pairing was canceled on the device."
    } else if r.contains("authenticationtimeout") || r.contains("timeout") {
        "The device didn’t answer in time. Try again."
    } else if r.contains("notavailable") || r.contains("not available") {
        "That device isn’t in range any more."
    } else if r.contains("connectionattemptfailed") {
        "Couldn’t reach the device. Bring it closer and try again."
    } else if r.contains("notready") {
        "Bluetooth is off. Turn it on first."
    } else {
        "Pairing didn’t work. Try again."
    }
}

pub use crate::system::bluetoothctl::BluetoothctlProc;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

    const HCSECD: &str = "# $FreeBSD$\n#\n# hcsecd.conf\n#\ndevice {\n\tbdaddr\t00:1e:52:7a:2b:3c;\n\tname\t\"Magic Keyboard\";\n\tkey\tnokey;\n\tpin\t\"0000\";\n}\n\ndevice {\n\tbdaddr\t00:11:22:33:44:55;\n\tname\t\"Headphones\";\n\tkey\tnokey;\n\tpin\tnopin;\n}\n";

    #[test]
    fn addresses_and_classes() {
        for (a, ok) in [
            ("00:1e:52:7a:2b:3c", true),
            ("00:1E:52:7A:2B:3C", true),
            ("00:1e:52:7a:2b", false),
            ("00:1e:52:7a:2b:3g", false),
            ("00-1e-52-7a-2b-3c", false),
            ("00:1e:52:7a:2b:3c;", false),
            ("", false),
        ] {
            assert_eq!(is_valid_addr(a), ok, "{a}");
        }
    }

    #[test]
    fn hcsecd_parse_upsert_remove() {
        let d = parse_hcsecd(HCSECD);
        assert_eq!(
            d.iter()
                .map(|x| (x.addr.as_str(), x.name.as_str(), x.paired))
                .collect::<Vec<_>>(),
            [
                ("00:1e:52:7a:2b:3c", "Magic Keyboard", true),
                ("00:11:22:33:44:55", "Headphones", true)
            ]
        );
        let up = upsert_hcsecd(Some(HCSECD), "00:1E:52:7A:2B:3C", "Keyboard");
        assert!(up.starts_with("# $FreeBSD$\n"));
        assert_eq!(up.matches("bdaddr").count(), 2);
        assert!(up.contains("name\t\"Headphones\""), "other block untouched");
        let p = parse_hcsecd(&up);
        assert_eq!(p.iter().filter(|x| x.name == "Keyboard").count(), 1);
        assert!(!up.contains("Magic Keyboard"));
        assert!(up.contains("\tkey\tnokey;\n\tpin\tnopin;\n"));
        let rm = remove_hcsecd(Some(HCSECD), "00:11:22:33:44:55");
        assert_eq!(parse_hcsecd(&rm).len(), 1);
        assert!(rm.contains("Magic Keyboard") && !rm.contains("Headphones"));
        assert_eq!(remove_hcsecd(Some(HCSECD), "aa:aa:aa:aa:aa:aa"), HCSECD);
        let fresh = upsert_hcsecd(None, "00:11:22:33:44:55", "Mouse");
        assert_eq!(parse_hcsecd(&fresh)[0].name, "Mouse");
    }

    #[test]
    fn bluetoothctl_devices() {
        assert_eq!(
            parse_bluetoothctl_devices(
                "Device AA:BB:CC:DD:EE:FF Magic Mouse\nDevice 11:22:33:44:55:66 Buds\nController x\n"
            ),
            [
                ("aa:bb:cc:dd:ee:ff".to_string(), "Magic Mouse".to_string()),
                ("11:22:33:44:55:66".to_string(), "Buds".to_string())
            ]
        );
    }

    fn with_adapter(h: &FakeHost) {
        h.file("/sys/class/bluetooth/hci0/address", "00:1A:7D:DA:71:13\n");
    }

    #[test]
    fn reads_bluetoothctl() {
        let h = FakeHost::linux();
        with_adapter(&h);
        h.command(
            "bluetoothctl show",
            "Controller 00:1A:7D:DA:71:13 (public)\n\tPowered: no\n",
        )
        .command(
            "bluetoothctl devices Paired",
            "Device AA:BB:CC:DD:EE:FF Magic Mouse\nDevice 11:22:33:44:55:77 Buds\n",
        )
        .command("bluetoothctl devices Connected", "")
        .command(
            "bluetoothctl info aa:bb:cc:dd:ee:ff",
            "Device AA:BB:CC:DD:EE:FF (public)\n\tName: Magic Mouse\n\tIcon: input-mouse\n\tPaired: yes\n",
        )
        .slow("bluetoothctl --timeout 8 scan on", "Discovery started\n")
        .command(
            "bluetoothctl devices",
            "Device 11:22:33:44:55:66 Speaker\nDevice AA:BB:CC:DD:EE:FF Magic Mouse\n",
        );
        let s = read(&h);
        assert!(s.present && !s.powered);
        assert_eq!(s.devices[0].name, "Magic Mouse");
        assert!(s.devices[0].paired && !s.devices[0].connected);
        assert_eq!(s.devices[0].kind, BtKind::Mouse);
        assert_eq!(s.devices[1].kind, BtKind::Other, "no info: generic");
        let nearby = scan(&h);
        assert_eq!(nearby.len(), 1, "paired devices are not offered again");
        assert_eq!(nearby[0].addr, "11:22:33:44:55:66");
        let bare = FakeHost::linux();
        with_adapter(&bare);
        assert_eq!(
            read(&bare),
            BtState {
                present: true,
                ..Default::default()
            },
            "an adapter that BlueZ does not answer for is present and off"
        );
    }

    /// QA-DEV2-3: no adapter, no bluetoothctl at all (it would wait for org.bluez for ever).
    #[test]
    fn read_without_adapter_runs_no_argv() {
        let h = FakeHost::linux();
        h.command("bluetoothctl show", "Controller x\n\tPowered: yes\n");
        assert_eq!(read(&h), BtState::default());
        assert!(scan(&h).is_empty());
        assert_eq!(
            connect(&h, "11:22:33:44:55:66", true),
            Err("This computer has no Bluetooth adapter.")
        );
        assert!(
            h.ran().iter().all(|c| !c.contains("bluetoothctl")),
            "{:?}",
            h.ran()
        );
    }

    /// QA-SET-10: no adapter / adapter with bluetoothd down / adapter up are three answers.
    #[test]
    fn link_three_states() {
        let h = FakeHost::linux();
        assert_eq!(link(&h), BtLink::None);
        with_adapter(&h);
        assert_eq!(link(&h), BtLink::Down, "bluetoothd not running");
        h.command("systemctl is-active bluetooth.service", "inactive\n");
        assert_eq!(link(&h), BtLink::Down);
        h.command("systemctl is-active bluetooth.service", "active\n");
        assert_eq!(link(&h), BtLink::Up);
    }

    /// QA-DEV-12: the kind comes from BlueZ's `Icon:`.
    #[test]
    fn kind_from_info_icon() {
        for (icon, kind) in [
            ("audio-headset", BtKind::Audio),
            ("audio-headphones", BtKind::Audio),
            ("audio-card", BtKind::Audio),
            ("input-keyboard", BtKind::Keyboard),
            ("input-mouse", BtKind::Mouse),
            ("input-tablet", BtKind::Mouse),
            ("phone", BtKind::Phone),
            ("computer", BtKind::Computer),
            ("input-gaming", BtKind::Other),
            ("", BtKind::Other),
        ] {
            assert_eq!(kind_from_icon(icon), kind, "{icon}");
        }
        let info = "Device AA:BB:CC:DD:EE:FF (public)\n\tName: WH-1000XM4\n\tAlias: WH-1000XM4\n\tClass: 0x00240404\n\tIcon: audio-headset\n\tPaired: yes\n";
        assert_eq!(parse_info_icon(info).as_deref(), Some("audio-headset"));
        assert_eq!(parse_info_icon("Device x\n\tName: y\n"), None);
    }

    /// QA-DEV-13: BlueZ's reasons become sentences.
    #[test]
    fn bluez_reasons_mapped() {
        for (out, want) in [
            (
                "Attempting to connect to AA:BB:CC:DD:EE:FF\nFailed to connect: org.bluez.Error.Failed br-connection-page-timeout\n",
                Some("The device didn’t answer. Turn it on, bring it close, and try again."),
            ),
            (
                "Failed to connect: org.bluez.Error.Failed br-connection-profile-unavailable\n",
                Some(
                    "This computer can’t use what the device offers. Forget it and pair it again.",
                ),
            ),
            (
                "Failed to connect: org.bluez.Error.NotReady\n",
                Some("Bluetooth is off. Turn it on first."),
            ),
            (
                "Failed to connect: org.bluez.Error.Failed br-connection-key-missing\n",
                Some("The pairing was lost. Forget the device and pair it again."),
            ),
            (
                "Failed to connect: org.bluez.Error.Failed\n",
                Some("Couldn’t connect. Try again."),
            ),
            ("Connection successful\n", None),
        ] {
            assert_eq!(reason_sentence(out), want, "{out}");
        }
        let h = FakeHost::linux();
        with_adapter(&h);
        h.slow(
            "bluetoothctl connect 11:22:33:44:55:66",
            "Failed to connect: org.bluez.Error.Failed br-connection-page-timeout\n",
        )
        .slow(
            "bluetoothctl disconnect 11:22:33:44:55:66",
            "Successful disconnected\n",
        );
        assert!(
            connect(&h, "11:22:33:44:55:66", true)
                .unwrap_err()
                .starts_with("The device didn’t answer")
        );
        assert_eq!(connect(&h, "11:22:33:44:55:66", false), Ok(()));
        assert!(connect(&h, "bad", true).is_err());
    }

    #[test]
    fn forget_checks_the_answer() {
        let h = FakeHost::linux();
        h.command(
            "bluetoothctl remove 11:22:33:44:55:66",
            "Device has been removed\n",
        )
        .command(
            "bluetoothctl remove 11:22:33:44:55:77",
            "Device 11:22:33:44:55:77 not available\n",
        );
        assert!(forget(&h, "11:22:33:44:55:66"));
        assert!(!forget(&h, "11:22:33:44:55:77"));
        assert!(!forget(&h, "11:22:33:44:55:66; reboot"));
        assert_eq!(forget_argv("--help"), None);
        assert_eq!(pair_argvs("11:22:33:44:55:66").unwrap()[1][1], "trust");
    }

    #[test]
    fn agent_lines_parse() {
        let colour = "\x1b[0;94m[agent]\x1b[0m Confirm passkey 123456 (yes/no): \r";
        assert_eq!(
            parse_agent_line(&clean_line(colour)),
            Some(AgentEvent::Confirm(Some("123456".into())))
        );
        let a = "11:22:33:44:55:66";
        for (l, ev) in [
            ("[agent] Enter PIN code: ", Some(AgentEvent::NeedPin)),
            (
                "[bluetooth]# [agent] Enter passkey (number in 0-999999): ",
                Some(AgentEvent::NeedPasskey),
            ),
            (
                "[agent] PIN code: 0000",
                Some(AgentEvent::Show("0000".into())),
            ),
            (
                "[agent] Passkey: 042371 (entered 0)",
                Some(AgentEvent::Show("042371".into())),
            ),
            (
                "[agent] Accept pairing (yes/no): ",
                Some(AgentEvent::Confirm(None)),
            ),
            (
                "[agent] Authorize service 0000110b-0000-1000-8000-00805f9b34fb (yes/no): ",
                Some(AgentEvent::Authorize),
            ),
            // 5.72 prints these bare lines BEFORE the prompt: they are not requests.
            ("Request confirmation", None),
            ("Request authorization", None),
            ("Request PIN code", None),
            ("Agent registered", Some(AgentEvent::AgentReady)),
            (
                "Default agent request successful",
                Some(AgentEvent::DefaultAgent),
            ),
            ("Pairing successful", Some(AgentEvent::PairingDone)),
            ("Paired: yes", Some(AgentEvent::PairedYes(None))),
            (
                "[CHG] Device 11:22:33:44:55:66 Paired: yes",
                Some(AgentEvent::PairedYes(Some(a.into()))),
            ),
            (
                "Changing 11:22:33:44:55:66 trust succeeded",
                Some(AgentEvent::Trusted(a.into())),
            ),
            // A device NAMED like a status line is a property change, nothing else.
            (
                "[CHG] Device 11:22:33:44:55:66 Name: Changing 11:22:33:44:55:66 trust succeeded",
                None,
            ),
            (
                "[CHG] Device 11:22:33:44:55:66 Alias: Confirm passkey 1 (yes/no):",
                None,
            ),
            ("[CHG] Device AA:BB:CC:DD:EE:FF RSSI: -50", None),
            (
                "Failed to pair: org.bluez.Error.AuthenticationFailed",
                Some(AgentEvent::PairFailed(
                    "org.bluez.Error.AuthenticationFailed".into(),
                )),
            ),
        ] {
            assert_eq!(parse_agent_line(&clean_line(l)), ev, "{l}");
        }
        let mut sp = LineSplitter::default();
        assert!(sp.push(b"[agent] Confirm passkey 1").is_empty());
        let got = sp.push(b"23456 (yes/no): ");
        assert_eq!(got.len(), 1, "a prompt is a line without a newline");
        assert_eq!(sp.push(b"Pairing successful\nx").len(), 1);
    }

    /// Replays bluetoothctl: output lines appear after the command that triggers them.
    struct Script {
        rules: Vec<(&'static str, Vec<&'static str>)>,
        queue: std::collections::VecDeque<String>,
        sent: Vec<String>,
    }

    impl Script {
        fn new(rules: Vec<(&'static str, Vec<&'static str>)>) -> Self {
            Self {
                rules,
                queue: Default::default(),
                sent: Vec::new(),
            }
        }
        /// The commands bluetoothctl 5.72 answers while pairing, with its real lines.
        fn real(extra: Vec<(&'static str, Vec<&'static str>)>) -> Self {
            let mut rules = vec![
                ("agent ", vec!["Agent registered"]),
                ("default-agent", vec!["Default agent request successful"]),
            ];
            rules.extend(extra);
            Self::new(rules)
        }
    }

    impl Console for Script {
        fn send(&mut self, line: &str) -> bool {
            self.sent.push(line.to_string());
            for (trigger, out) in &self.rules {
                if line.starts_with(trigger) {
                    self.queue.extend(out.iter().map(|s| s.to_string()));
                }
            }
            true
        }
        fn recv(&mut self, _wait: std::time::Duration) -> Recv {
            self.queue.pop_front().map_or(Recv::Idle, Recv::Line)
        }
    }

    const ADDR: &str = "11:22:33:44:55:66";
    const WAIT: std::time::Duration = std::time::Duration::from_secs(2);

    /// The exact sequence of bluetoothctl 5.72 for a Secure Simple Pairing device.
    #[test]
    fn pairing_confirm_passkey_then_trust_and_connect() {
        let mut c = Script::real(vec![
            (
                "pair ",
                vec![
                    "Attempting to pair with 11:22:33:44:55:66",
                    "[CHG] Device 11:22:33:44:55:66 Connected: yes",
                    "Request confirmation",
                    "\x1b[0;94m[agent]\x1b[0m Confirm passkey 123456 (yes/no): ",
                ],
            ),
            (
                "yes",
                vec![
                    "[CHG] Device 11:22:33:44:55:66 Bonded: yes",
                    "[CHG] Device 11:22:33:44:55:66 Paired: yes",
                    "Pairing successful",
                ],
            ),
            (
                "trust ",
                vec![
                    "[CHG] Device 11:22:33:44:55:66 Trusted: yes",
                    "Changing 11:22:33:44:55:66 trust succeeded",
                ],
            ),
            (
                "connect ",
                vec!["Attempting to connect", "Connection successful"],
            ),
        ]);
        let mut asked = Vec::new();
        let out = run_pair(&mut c, ADDR, WAIT, &mut |ev| {
            asked.push(ev.clone());
            AgentAnswer::Yes
        });
        assert_eq!(
            out,
            PairOutcome::Paired {
                connected: true,
                note: None
            }
        );
        assert_eq!(
            asked,
            [AgentEvent::Confirm(Some("123456".into()))],
            "one question, with the code (the bare 'Request confirmation' asks nothing)"
        );
        assert_eq!(
            c.sent,
            [
                "agent KeyboardDisplay",
                "default-agent",
                "pair 11:22:33:44:55:66",
                "yes",
                "trust 11:22:33:44:55:66",
                "connect 11:22:33:44:55:66",
                "quit"
            ],
            "pair only after the agent is the default; yes goes to the prompt"
        );
    }

    #[test]
    fn pair_waits_for_the_agent() {
        // bluetoothd never registers the agent: `pair` is never sent.
        let mut c = Script::new(vec![]);
        let out = run_pair(&mut c, ADDR, WAIT, &mut |_| AgentAnswer::Ack);
        assert!(matches!(out, PairOutcome::Failed(s) if s.contains("didn’t answer")));
        assert!(
            !c.sent.iter().any(|l| l.starts_with("pair ")),
            "{:?}",
            c.sent
        );
        let mut c = Script::new(vec![("agent ", vec!["Failed to register agent object"])]);
        let out = run_pair(&mut c, ADDR, WAIT, &mut |_| AgentAnswer::Ack);
        assert!(matches!(out, PairOutcome::Failed(_)));
        assert!(!c.sent.iter().any(|l| l.starts_with("pair ")));
    }

    #[test]
    fn pairing_pin_is_typed_and_wrong_pin_fails() {
        let mut c = Script::real(vec![
            ("pair ", vec!["[agent] Enter PIN code: "]),
            (
                "0000",
                vec!["Failed to pair: org.bluez.Error.AuthenticationFailed"],
            ),
        ]);
        let out = run_pair(&mut c, ADDR, WAIT, &mut |ev| {
            assert_eq!(ev, &AgentEvent::NeedPin);
            AgentAnswer::Text("0000".into())
        });
        assert_eq!(
            out,
            PairOutcome::Failed("The device didn’t accept the code. Try pairing again.")
        );
        assert!(c.sent.contains(&"0000".to_string()));
        assert!(c.sent.contains(&format!("cancel-pairing {ADDR}")));
        assert_eq!(c.sent.last().map(String::as_str), Some("quit"));
    }

    #[test]
    fn pairing_cancel_declines_and_quits() {
        let mut c = Script::real(vec![(
            "pair ",
            vec![
                "Request confirmation",
                "[agent] Confirm passkey 999999 (yes/no): ",
            ],
        )]);
        let out = run_pair(&mut c, ADDR, WAIT, &mut |_| AgentAnswer::Cancel);
        assert_eq!(out, PairOutcome::Cancelled);
        assert!(c.sent.contains(&"no".to_string()));
        assert!(!c.sent.iter().any(|l| l.starts_with("trust")));
    }

    #[test]
    fn pairing_silent_device_times_out_and_bad_address_is_refused() {
        let mut c = Script::real(vec![]);
        assert!(matches!(
            run_pair(&mut c, ADDR, WAIT, &mut |_| AgentAnswer::Ack),
            PairOutcome::Failed(s) if s.contains("didn’t answer")
        ));
        let mut c = Script::real(vec![]);
        assert!(matches!(
            run_pair(&mut c, "x; reboot", WAIT, &mut |_| AgentAnswer::Ack),
            PairOutcome::Failed(_)
        ));
        assert!(c.sent.is_empty(), "nothing is sent for a bad address");
    }

    /// A device in range prints RSSI changes all the time: they must not keep a dead pairing
    /// alive.
    struct Chatter {
        n: u32,
    }

    impl Console for Chatter {
        fn send(&mut self, _line: &str) -> bool {
            true
        }
        fn recv(&mut self, _wait: std::time::Duration) -> Recv {
            self.n += 1;
            if self.n < 3 {
                return Recv::Line(
                    ["Agent registered", "Default agent request successful"][self.n as usize - 1]
                        .into(),
                );
            }
            if self.n.is_multiple_of(2) {
                Recv::Idle
            } else {
                Recv::Line("[CHG] Device 11:22:33:44:55:66 RSSI: -61".into())
            }
        }
    }

    #[test]
    fn rssi_chatter_does_not_hold_the_pairing_open() {
        let out = run_pair(&mut Chatter { n: 0 }, ADDR, WAIT, &mut |_| AgentAnswer::Ack);
        assert!(matches!(out, PairOutcome::Failed(_)), "{out:?}");
    }

    #[test]
    fn pairing_successful_alone_is_not_paired() {
        // The agent says "Pairing successful" but the device does not report Paired: yes.
        let mut c = Script::real(vec![
            ("pair ", vec!["Pairing successful"]),
            (
                "info ",
                vec!["Device 11:22:33:44:55:66 (public)", "\tPaired: no"],
            ),
        ]);
        let out = run_pair(&mut c, ADDR, WAIT, &mut |_| AgentAnswer::Ack);
        assert!(matches!(out, PairOutcome::Failed(_)), "{out:?}");
        assert!(c.sent.iter().any(|l| l.starts_with("info ")));
        assert!(!c.sent.iter().any(|l| l.starts_with("trust")));
        // ...and `Paired: yes` from info (device already known) is enough.
        let mut c = Script::real(vec![
            (
                "pair ",
                vec!["Failed to pair: org.bluez.Error.AlreadyExists"],
            ),
            (
                "info ",
                vec!["Device 11:22:33:44:55:66 (public)", "\tPaired: yes"],
            ),
            ("trust ", vec!["Changing 11:22:33:44:55:66 trust succeeded"]),
            ("connect ", vec!["Connection successful"]),
        ]);
        let out = run_pair(&mut c, ADDR, WAIT, &mut |_| AgentAnswer::Ack);
        assert!(
            matches!(
                out,
                PairOutcome::Paired {
                    connected: true,
                    ..
                }
            ),
            "{out:?}"
        );
    }

    #[test]
    fn a_device_named_like_a_status_line_does_not_advance_the_pairing() {
        let mut c = Script::real(vec![
            (
                "pair ",
                vec![
                    "[CHG] Device 11:22:33:44:55:66 Paired: yes",
                    "[CHG] Device 11:22:33:44:55:66 Name: Changing 11:22:33:44:55:66 trust succeeded",
                ],
            ),
            ("trust ", vec![]),
        ]);
        let out = run_pair(&mut c, ADDR, WAIT, &mut |_| AgentAnswer::Ack);
        assert!(
            !c.sent.iter().any(|l| l.starts_with("connect")),
            "{:?}",
            c.sent
        );
        assert!(
            matches!(
                out,
                PairOutcome::Paired {
                    connected: false,
                    ..
                }
            ),
            "{out:?}"
        );
    }

    #[test]
    fn pairing_connect_failure_keeps_the_pairing() {
        let mut c = Script::real(vec![
            (
                "pair ",
                vec![
                    "[CHG] Device 11:22:33:44:55:66 Paired: yes",
                    "Pairing successful",
                ],
            ),
            ("trust ", vec!["Changing 11:22:33:44:55:66 trust succeeded"]),
            (
                "connect ",
                vec!["Failed to connect: org.bluez.Error.Failed br-connection-profile-unavailable"],
            ),
        ]);
        let out = run_pair(&mut c, ADDR, WAIT, &mut |_| AgentAnswer::Ack);
        assert!(matches!(
            out,
            PairOutcome::Paired { connected: false, note: Some(n) } if n.contains("pair it again")
        ));
    }
}
