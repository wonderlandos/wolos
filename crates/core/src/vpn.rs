// SPDX-License-Identifier: BSD-2-Clause
//! VPN profiles (NET-3, QA-NET-21): WireGuard `.conf` and OpenVPN `.ovpn` files imported into
//! NetworkManager, listed, connected, disconnected and forgotten as the person at the computer
//! (NetworkManager's polkit rules allow it). Secrets live in the login keyring (`secret-tool`,
//! libsecret) and reach `nmcli connection up` on stdin, never argv or the profile file. Pure
//! parsers and fixed argv here; [`crate::controls`] runs them (`controls/net.rs`).

/// What a VPN profile speaks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VpnKind {
    WireGuard,
    OpenVpn,
    /// A VPN profile of another plugin (made with nmcli or nm-connection-editor).
    #[default]
    Other,
}

impl VpnKind {
    /// nmcli's `import type` name.
    pub fn import_type(self) -> Option<&'static str> {
        match self {
            Self::WireGuard => Some("wireguard"),
            Self::OpenVpn => Some("openvpn"),
            Self::Other => None,
        }
    }
    /// What Settings writes under the name.
    pub fn label(self) -> &'static str {
        match self {
            Self::WireGuard => "WireGuard",
            Self::OpenVpn => "OpenVPN",
            Self::Other => "VPN",
        }
    }
    /// The secret NetworkManager asks for when the profile does not keep it.
    pub fn secret_key(self) -> Option<&'static str> {
        match self {
            Self::WireGuard => Some("wireguard.private-key"),
            Self::OpenVpn => Some("vpn.secrets.password"),
            Self::Other => None,
        }
    }
}

/// One VPN profile of NetworkManager.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Vpn {
    pub name: String,
    pub uuid: String,
    pub kind: VpnKind,
    /// Connected (or connecting) now.
    pub active: bool,
}

/// Every profile with its type and whether it is active.
pub const LIST_ARGV: [&str; 6] = [
    "nmcli",
    "-t",
    "-f",
    "NAME,UUID,TYPE,ACTIVE",
    "connection",
    "show",
];

/// [`LIST_ARGV`]'s output -> the VPN profiles (WireGuard and NetworkManager's `vpn` type). The
/// name comes first and may hold an escaped `\:`; the three fields after it never do.
pub fn parse_list(text: &str) -> Vec<Vpn> {
    let mut v = Vec::new();
    for l in text.lines() {
        let mut f = l.rsplitn(4, ':');
        let (Some(active), Some(ty), Some(uuid), Some(name)) =
            (f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        let kind = match ty {
            "wireguard" => VpnKind::WireGuard,
            "vpn" => VpnKind::Other,
            _ => continue,
        };
        if !crate::netconf::is_uuid(uuid) {
            continue;
        }
        v.push(Vpn {
            name: crate::wifi::unescape(name),
            uuid: uuid.into(),
            kind,
            active: active == "yes",
        });
    }
    v
}

/// The plugin of a `vpn` profile: `nmcli -g vpn.service-type connection show uuid <uuid>`.
pub fn service_type_argv(uuid: &str) -> Vec<String> {
    sv(&[
        "nmcli",
        "-g",
        "vpn.service-type",
        "connection",
        "show",
        "uuid",
        uuid,
    ])
}

/// OpenVPN's plugin answers to `org.freedesktop.NetworkManager.openvpn`.
pub fn kind_of_service(service: &str) -> VpnKind {
    if service.trim().ends_with(".openvpn") {
        VpnKind::OpenVpn
    } else {
        VpnKind::Other
    }
}

/// The kind of file by its name: `.conf` is WireGuard, `.ovpn` is OpenVPN.
pub fn file_kind(path: &str) -> Option<VpnKind> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    match ext.as_str() {
        "conf" => Some(VpnKind::WireGuard),
        "ovpn" => Some(VpnKind::OpenVpn),
        _ => None,
    }
}

/// NetworkManager names an imported WireGuard profile after the file, and that name is the
/// tunnel's interface: at most 15 letters, digits, `-` or `_`. "My Office VPN.conf" -> "My-Office-VPN".
pub fn interface_name(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let mut n: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    n = n.trim_matches('-').chars().take(15).collect();
    if n.is_empty() { "wg0".into() } else { n }
}

/// OpenVPN's plugin is installed (`networkmanager-openvpn`).
pub const OPENVPN_PLUGIN: &str = "/usr/lib/NetworkManager/VPN/nm-openvpn-service.name";

/// `nmcli connection import type <wireguard|openvpn> file <path>`.
pub fn import_argv(kind: VpnKind, path: &str) -> Option<Vec<String>> {
    Some(sv(&[
        "nmcli",
        "connection",
        "import",
        "type",
        kind.import_type()?,
        "file",
        path,
    ]))
}

/// nmcli's "Connection 'wg0' (6a3f…) successfully added." -> the UUID.
pub fn parse_import(out: &str) -> Option<String> {
    let start = out.find('(')? + 1;
    let end = start + out[start..].find(')')?;
    let u = &out[start..end];
    crate::netconf::is_uuid(u).then(|| u.to_string())
}

/// The secret as the profile keeps it (`-s`: secrets shown), to move it into the keyring.
pub fn read_secret_argv(uuid: &str, key: &str) -> Vec<String> {
    sv(&["nmcli", "-s", "-g", key, "connection", "show", "uuid", uuid])
}

/// After the move: NetworkManager keeps the secret nowhere (flag 2, "not saved") and the tunnel
/// starts only when asked (it needs the keyring's copy).
pub fn forget_secret_argv(uuid: &str, kind: VpnKind) -> Option<Vec<String>> {
    let flags = match kind {
        VpnKind::WireGuard => "wireguard.private-key-flags",
        VpnKind::OpenVpn => "+vpn.data",
        VpnKind::Other => return None,
    };
    let mut v = sv(&["nmcli", "connection", "modify", "uuid", uuid]);
    match kind {
        VpnKind::OpenVpn => v.extend(sv(&[flags, "password-flags=2"])),
        _ => v.extend(sv(&[flags, "2"])),
    }
    v.extend(sv(&["connection.autoconnect", "no"]));
    Some(v)
}

/// `secret-tool` attributes of one profile's secret.
fn attrs(uuid: &str, key: &str) -> Vec<String> {
    sv(&["application", "wolos", "vpn", uuid, "setting", key])
}

/// Store: the secret goes on stdin (`secret-tool store` reads it there).
pub fn keyring_store_argv(uuid: &str, name: &str, key: &str) -> Vec<String> {
    let mut v = sv(&["secret-tool", "store", "--label", &format!("VPN {name}")]);
    v.extend(attrs(uuid, key));
    v
}

/// The secret back from the keyring (stdout).
pub fn keyring_lookup_argv(uuid: &str, key: &str) -> Vec<String> {
    let mut v = sv(&["secret-tool", "lookup"]);
    v.extend(attrs(uuid, key));
    v
}

/// Every secret of the profile (Forget).
pub fn keyring_clear_argv(uuid: &str) -> Vec<String> {
    sv(&["secret-tool", "clear", "application", "wolos", "vpn", uuid])
}

/// `nmcli connection up uuid <uuid> passwd-file /dev/stdin`: the secret line on stdin.
pub fn up_with_secret_argv(uuid: &str) -> Vec<String> {
    sv(&[
        "nmcli",
        "connection",
        "up",
        "uuid",
        uuid,
        "passwd-file",
        "/dev/stdin",
    ])
}

/// The `passwd-file` line: `<key>:<secret>`.
pub fn secret_line(key: &str, secret: &str) -> String {
    format!("{key}:{secret}\n")
}

/// Disconnect: the profile stays.
pub fn down_argv(uuid: &str) -> Vec<String> {
    sv(&["nmcli", "connection", "down", "uuid", uuid])
}

/// Forget: the profile goes.
pub fn delete_argv(uuid: &str) -> Vec<String> {
    sv(&["nmcli", "connection", "delete", "uuid", uuid])
}

/// Why an import failed, from nmcli's stderr.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportFailure {
    /// The OpenVPN plugin is not installed.
    NoPlugin,
    /// The file is not a configuration NetworkManager can read.
    BadFile,
    Other,
}

/// [`ImportFailure`] from what `nmcli connection import` wrote on stderr.
pub fn classify_import_error(stderr: &str) -> ImportFailure {
    let e = stderr.to_ascii_lowercase();
    if e.contains("plugin") || e.contains("unknown vpn") {
        ImportFailure::NoPlugin
    } else if e.contains("failed to import") || e.contains("invalid") || e.contains("not a valid") {
        ImportFailure::BadFile
    } else {
        ImportFailure::Other
    }
}

fn sv(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const U: &str = "6a3f0c2e-1111-4a5b-9c8d-0123456789ab";

    #[test]
    fn lists_vpn_profiles_only() {
        let t = format!(
            concat!(
                "Home:11111111-2222-3333-4444-555555555555:802-11-wireless:yes\nOffice\\: Lisbon:{U}:wireguard:no\n",
                "work:22222222-2222-3333-4444-555555555555:vpn:yes\nlo:33333333-2222-3333-4444-555555555555:loopback:yes\n",
                "odd:not-a-uuid:wireguard:no\n"
            ),
            U = U
        );
        let v = parse_list(&t);
        assert_eq!(v.len(), 2);
        assert_eq!(
            v[0],
            Vpn {
                name: "Office: Lisbon".into(),
                uuid: U.into(),
                kind: VpnKind::WireGuard,
                active: false
            }
        );
        assert!(v[1].active && v[1].kind == VpnKind::Other);
        assert_eq!(
            kind_of_service("org.freedesktop.NetworkManager.openvpn\n"),
            VpnKind::OpenVpn
        );
        assert_eq!(
            kind_of_service("org.freedesktop.NetworkManager.vpnc"),
            VpnKind::Other
        );
    }

    #[test]
    fn import_names_and_argv() {
        assert_eq!(file_kind("/home/a/wg0.conf"), Some(VpnKind::WireGuard));
        assert_eq!(file_kind("/home/a/Office.OVPN"), Some(VpnKind::OpenVpn));
        assert_eq!(file_kind("/home/a/notes.txt"), None);
        assert_eq!(file_kind("noext"), None);
        assert_eq!(interface_name("/x/My Office VPN.conf"), "My-Office-VPN");
        assert_eq!(
            interface_name("/x/a-very-long-tunnel-name.conf"),
            "a-very-long-tun"
        );
        assert_eq!(interface_name("/x/ .conf"), "wg0");
        assert_eq!(
            import_argv(VpnKind::WireGuard, "/tmp/x/wg0.conf")
                .unwrap()
                .join(" "),
            "nmcli connection import type wireguard file /tmp/x/wg0.conf"
        );
        assert_eq!(import_argv(VpnKind::Other, "/x"), None);
        assert_eq!(
            parse_import(&format!("Connection 'wg0' ({U}) successfully added.\n")),
            Some(U.into())
        );
        assert_eq!(parse_import("Connection added."), None);
        assert_eq!(
            classify_import_error("Error: failed to find VPN plugin for openvpn."),
            ImportFailure::NoPlugin
        );
        assert_eq!(
            classify_import_error("Error: failed to import '/x': Invalid argument"),
            ImportFailure::BadFile
        );
    }

    #[test]
    fn secrets_go_through_the_keyring() {
        assert_eq!(
            keyring_store_argv(U, "Office", "wireguard.private-key").join(" "),
            format!(
                "secret-tool store --label VPN Office application wolos vpn {U} setting wireguard.private-key"
            )
        );
        assert_eq!(
            keyring_lookup_argv(U, "vpn.secrets.password").join(" "),
            format!("secret-tool lookup application wolos vpn {U} setting vpn.secrets.password")
        );
        assert_eq!(
            forget_secret_argv(U, VpnKind::WireGuard).unwrap().join(" "),
            format!(
                "nmcli connection modify uuid {U} wireguard.private-key-flags 2 connection.autoconnect no"
            )
        );
        assert_eq!(
            forget_secret_argv(U, VpnKind::OpenVpn).unwrap()[5..7],
            ["+vpn.data", "password-flags=2"]
        );
        assert_eq!(
            secret_line("wireguard.private-key", "abc="),
            "wireguard.private-key:abc=\n"
        );
        assert!(up_with_secret_argv(U).ends_with(&["passwd-file".into(), "/dev/stdin".into()]));
    }
}
