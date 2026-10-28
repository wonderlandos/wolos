//! Admin actions + wire (ADR 0026 in IaraOS). Wire on [`SOCKET`]:
//! `action NUL count NUL arg1 NUL ... argN NUL password`; reply one ASCII line.

pub const SOCKET: &str = "/var/run/wol-admin.sock";
pub const MAX_REQUEST: usize = 8192;
pub const MAX_ARGS: usize = 4;
pub const MAX_PASSWORD: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AdminAction {
    WifiJoin,
    WifiPower,
    Bluetooth,
    AudioDefault,
    Appearance,
    Keybinds,
    Idle,
    UserAdd,
    UserRemove,
    TimeZone,
    PkgUpgrade,
    /// Refresh the package catalogue (`pkg update`) so an update check reads current data.
    PkgRefresh,
    Revoke,
    /// App Store: one catalog id or pkg name (`wol_core::store`).
    PkgInstall,
    /// App Store: remove one catalog id or package name with its unused dependencies.
    PkgRemove,
    /// Low Power Mode: `low` or `auto` (powerd flags).
    PowerMode,
    /// Boot this boot environment next (restore point; `wol_core::update::be`).
    BootEnvironment,
    /// `kldload -n` one module from the hardware catalogue (`wol hw fix`).
    HwLoad,
    /// Install a `.wol` file: `<path> signed|unsigned` (ADR 0011; `unsigned` needs a fresh password).
    WolInstall,
    /// Remove an installed `.wol` app by id.
    WolRemove,
    /// Network time: `on` or `off` (ntpd).
    Ntp,
    /// Computer name: one lowercase RFC 1123 label.
    Hostname,
    /// Remote Login: `on` or `off` (sshd).
    RemoteLogin,
    /// Firewall: `on` or `off` (pf with the WolOS ruleset).
    Firewall,
    /// Lid closed: `sleep` or `nothing`.
    Lid,
    /// `<queue> <device uri>`: driverless CUPS queue.
    PrinterAdd,
    /// `<queue>`.
    PrinterRemove,
    /// `<queue>`.
    PrinterDefault,
    /// `<app path> <capability> allow|deny`: compositor consent database.
    Consent,
    /// `<addr> <name>`: remember a Bluetooth device (hcsecd.conf).
    BtPair,
    /// `<addr>`.
    BtForget,
    /// `<iface> <ssid>`: drop a saved Wi-Fi network.
    WifiForget,
    /// `sleep`, `restart` or `shutdown` from the session menu.
    SessionPower,
    /// System-wide Flatpak app: `<app-id> <permission> allow|deny`, or `<app-id> reset`.
    FlatpakOverride,
    /// Install one Flathub app for every account: `<app-id>`.
    FlatpakInstall,
    /// Remove one system-wide Flatpak app: `<app-id>`.
    FlatpakRemove,
    /// Automatic updates: download pending packages in the background, `on` or `off`.
    AutoUpdate,
}

/// Length inferred: adding an action is one row here and one variant above.
const NAMES: &[(AdminAction, &str)] = &[
    (AdminAction::WifiJoin, "wifi-join"),
    (AdminAction::WifiPower, "wifi-power"),
    (AdminAction::Bluetooth, "bluetooth"),
    (AdminAction::AudioDefault, "audio-default"),
    (AdminAction::Appearance, "appearance"),
    (AdminAction::Keybinds, "keybinds"),
    (AdminAction::Idle, "idle"),
    (AdminAction::UserAdd, "user-add"),
    (AdminAction::UserRemove, "user-remove"),
    (AdminAction::TimeZone, "timezone"),
    (AdminAction::PkgUpgrade, "pkg-upgrade"),
    (AdminAction::PkgRefresh, "pkg-refresh"),
    (AdminAction::Revoke, "revoke"),
    (AdminAction::PkgInstall, "pkg-install"),
    (AdminAction::PkgRemove, "pkg-remove"),
    (AdminAction::PowerMode, "power-mode"),
    (AdminAction::BootEnvironment, "boot-environment"),
    (AdminAction::HwLoad, "hw-load"),
    (AdminAction::WolInstall, "wol-install"),
    (AdminAction::WolRemove, "wol-remove"),
    (AdminAction::Ntp, "ntp"),
    (AdminAction::Hostname, "hostname"),
    (AdminAction::RemoteLogin, "remote-login"),
    (AdminAction::Firewall, "firewall"),
    (AdminAction::Lid, "lid"),
    (AdminAction::PrinterAdd, "printer-add"),
    (AdminAction::PrinterRemove, "printer-remove"),
    (AdminAction::PrinterDefault, "printer-default"),
    (AdminAction::Consent, "consent"),
    (AdminAction::BtPair, "bt-pair"),
    (AdminAction::BtForget, "bt-forget"),
    (AdminAction::WifiForget, "wifi-forget"),
    (AdminAction::SessionPower, "session-power"),
    (AdminAction::FlatpakOverride, "flatpak-override"),
    (AdminAction::FlatpakInstall, "flatpak-install"),
    (AdminAction::FlatpakRemove, "flatpak-remove"),
    (AdminAction::AutoUpdate, "auto-update"),
];

impl AdminAction {
    pub const ALL: [AdminAction; NAMES.len()] = {
        let mut a = [AdminAction::Revoke; NAMES.len()];
        let mut i = 0;
        while i < NAMES.len() {
            a[i] = NAMES[i].0;
            i += 1;
        }
        a
    };

    pub fn name(self) -> &'static str {
        NAMES
            .iter()
            .find(|n| n.0 == self)
            .map(|n| n.1)
            .unwrap_or("")
    }

    pub fn parse(name: &str) -> Option<Self> {
        NAMES.iter().find(|n| n.1 == name).map(|n| n.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminRequest {
    pub action: AdminAction,
    pub args: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdminReply {
    Ok,
    NeedsAuth,
    Denied,
    NotAdmin,
    Invalid,
    Failed,
}

const LINES: [(AdminReply, &str); 6] = [
    (AdminReply::Ok, "ok"),
    (AdminReply::NeedsAuth, "auth"),
    (AdminReply::Denied, "denied"),
    (AdminReply::NotAdmin, "notadmin"),
    (AdminReply::Invalid, "invalid"),
    (AdminReply::Failed, "failed"),
];

impl AdminReply {
    pub fn line(self) -> &'static str {
        LINES.iter().find(|l| l.0 == self).map_or("failed", |l| l.1)
    }

    /// Unknown or missing line = Failed.
    pub fn parse(line: &str) -> Self {
        LINES
            .iter()
            .find(|l| l.1 == line.trim())
            .map_or(AdminReply::Failed, |l| l.0)
    }

    pub fn message(self) -> &'static str {
        match self {
            AdminReply::Ok => "",
            AdminReply::NotAdmin => "Only administrators can make this change",
            AdminReply::Denied => "Incorrect password",
            AdminReply::Invalid => "The change was refused (invalid request)",
            AdminReply::NeedsAuth => "Administrator password required",
            AdminReply::Failed => "The change failed",
        }
    }
}

pub fn encode(req: &AdminRequest, password: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    for f in [req.action.name(), &req.args.len().to_string()]
        .into_iter()
        .chain(req.args.iter().map(String::as_str))
    {
        b.extend_from_slice(f.as_bytes());
        b.push(0);
    }
    b.extend_from_slice(password);
    b
}

/// Request and the byte range of the password; None if malformed.
pub fn decode(data: &[u8]) -> Option<(AdminRequest, std::ops::Range<usize>)> {
    if data.is_empty() || data.len() > MAX_REQUEST {
        return None;
    }
    let mut pos = 0;
    let mut field = || -> Option<&str> {
        let nul = data[pos..].iter().position(|b| *b == 0)?;
        let s = std::str::from_utf8(&data[pos..pos + nul]).ok()?;
        pos += nul + 1;
        (!s.chars().any(|c| c < ' ' || c == '\x7f')).then_some(s)
    };
    let action = AdminAction::parse(field()?)?;
    let count = field()?;
    if count.len() != 1 || !count.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    let count = (count.as_bytes()[0] - b'0') as usize;
    if count > MAX_ARGS {
        return None;
    }
    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        args.push(field()?.to_owned());
    }
    let pw = &data[pos..];
    if pw.len() > MAX_PASSWORD || pw.contains(&0) {
        return None;
    }
    Some((AdminRequest { action, args }, pos..data.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_roundtrip() {
        for a in AdminAction::ALL {
            assert_eq!(AdminAction::parse(a.name()), Some(a));
        }
        assert_eq!(AdminAction::parse("rm-rf"), None);
    }

    /// Wire names System Settings sends (compatibility: never renamed).
    #[test]
    fn settings_wire_names() {
        for n in [
            "ntp",
            "hostname",
            "remote-login",
            "firewall",
            "lid",
            "printer-add",
            "printer-remove",
            "printer-default",
            "consent",
            "bt-pair",
            "bt-forget",
            "wifi-forget",
            "session-power",
            "auto-update",
        ] {
            assert_eq!(AdminAction::parse(n).map(AdminAction::name), Some(n));
        }
    }

    #[test]
    fn wire_roundtrip() {
        let r = AdminRequest {
            action: AdminAction::WifiJoin,
            args: vec!["wlan0".into(), "Home".into(), "pw".into()],
        };
        let b = encode(&r, b"secret");
        assert_eq!(&b[..], b"wifi-join\x003\x00wlan0\x00Home\x00pw\x00secret");
        let (d, pw) = decode(&b).unwrap();
        assert_eq!(d, r);
        assert_eq!(&b[pw], b"secret");
        let (d, pw) = decode(&encode(
            &AdminRequest {
                action: AdminAction::Revoke,
                args: vec![],
            },
            b"",
        ))
        .unwrap();
        assert_eq!((d.action, pw.len()), (AdminAction::Revoke, 0));
    }

    #[test]
    fn malformed_table() {
        let big = vec![b'a'; MAX_REQUEST + 1];
        let cases: &[&[u8]] = &[
            b"",
            &big,
            b"nope\x000\x00",
            b"revoke\x00",
            b"revoke\x00x\x00",
            b"revoke\x0010\x00",
            b"idle\x005\x00a\x00b\x00c\x00d\x00e\x00",
            b"idle\x001\x00",
            b"idle\x001\x001\n2\x00",
            b"idle\x001\x00\xff\x00",
            b"idle\x001\x0030\x00pw\x00x",
        ];
        for c in cases {
            assert!(decode(c).is_none(), "{:?}", String::from_utf8_lossy(c));
        }
    }

    #[test]
    fn replies() {
        for (r, l) in LINES {
            assert_eq!(r.line(), l);
            assert_eq!(AdminReply::parse(&format!("{l}\n")), r);
        }
        assert_eq!(AdminReply::parse("garbage"), AdminReply::Failed);
    }
}
