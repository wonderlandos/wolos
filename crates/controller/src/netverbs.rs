// SPDX-License-Identifier: GPL-2.0-or-later
//! Network verbs (NET-3, docs/modules/wifi.md): `wol network ip`, `wol vpn …`, `wol proxy …`,
//! `wol dns …`, `wol airplane …`. The work is `wol_core::controls` (`controls/net.rs`); the
//! proxy and DNS changes are wol-admind's (`proxy`, `dns-over-tls`), the administrator password
//! on stdin like every root change of `wol`.
use crate::cli::Invocation;
use crate::run::{Env, fail};
use crate::verbs::{control_error, show, with_password};
use serde_json::{Value, json};
use shared::errors::ErrorCode;
use wol_core::controls::ControlError;
use wol_core::netconf::{self, DotConfig, DotMode, IpApplied, Ipv4Config, ProxyConfig, ProxyMode};
use wol_core::vpn::VpnKind;

/// Handles `inv` when it is a network verb.
pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let json = inv.flag("json") || !env.stdout_tty();
    let a = |k: &str| inv.get(k).unwrap_or_default().to_string();
    match inv.action {
        "network.ip" => network_ip(env, inv, json),
        "vpn.list" => vpn_list(env, json),
        "vpn.import" => vpn_import(env, &a("file"), json),
        "vpn.connect" => vpn_change(env, &a("name"), Change::Connect, json),
        "vpn.disconnect" => vpn_change(env, &a("name"), Change::Disconnect, json),
        "vpn.forget" => vpn_change(env, &a("name"), Change::Forget, json),
        "proxy.get" => {
            let v = proxy_state(env);
            show(env, v, json)
        }
        "proxy.off" => proxy_set(env, ProxyConfig::default(), json),
        "proxy.auto" => match proxy_options(&inv.all("options")) {
            Ok(mut c) => {
                c.mode = ProxyMode::Auto;
                c.pac_url = a("url");
                proxy_set(env, c, json)
            }
            Err(e) => fail(env, ErrorCode::Usage, &e),
        },
        "proxy.manual" => match proxy_options(&inv.all("options")) {
            Ok(mut c) => {
                c.mode = ProxyMode::Manual;
                proxy_set(env, c, json)
            }
            Err(e) => fail(env, ErrorCode::Usage, &e),
        },
        "dns.get" => {
            let v = dns_state(env);
            show(env, v, json)
        }
        "dns.tls" => dns_tls(env, &a("mode"), &inv.all("servers"), json),
        "airplane.get" => {
            let v = airplane_state(env);
            show(env, v, json)
        }
        "airplane.on" | "airplane.off" => airplane_set(env, inv.action == "airplane.on", json),
        other => fail(env, ErrorCode::Failed, &format!("no handler for {other}")),
    }
}

// ---- wol network ip ----

fn ip_json(iface: &str, c: &Ipv4Config) -> Value {
    json!({
        "interface": iface,
        "automatic": !c.manual,
        "address": (c.manual && !c.address.is_empty()).then(|| c.address.clone()),
        "router": (c.manual && !c.router.is_empty()).then(|| c.router.clone()),
        "dns": if c.manual { c.dns.clone() } else { Vec::new() },
    })
}

/// `wol network ip <iface> [auto | manual <address/prefix> [router] [dns…]]` (QA-LIVE3-14).
fn network_ip(env: &mut dyn Env, inv: &Invocation, json: bool) -> i32 {
    let iface = inv.get("interface").unwrap_or_default().to_string();
    let c = env.controls();
    let Some(now) = c.ipv4(&iface) else {
        return fail(
            env,
            ErrorCode::Unavailable,
            &format!(
                "{iface} is not connected to a network (no NetworkManager profile is active on it)"
            ),
        );
    };
    let want = match inv.get("mode") {
        None => return show(env, ip_json(&iface, &now), json),
        Some("auto") => Ipv4Config::default(),
        Some(_) => {
            let dns = inv.all("dns").join(",");
            let Some(dns) = netconf::parse_dns_list(&dns) else {
                return fail(env, ErrorCode::Invalid, netconf::IpProblem::Dns.sentence());
            };
            Ipv4Config {
                manual: true,
                address: inv.get("address").unwrap_or_default().to_string(),
                router: inv.get("router").unwrap_or_default().to_string(),
                dns,
            }
        }
    };
    if let Err(p) = netconf::validate_ipv4(&want) {
        return fail(env, ErrorCode::Invalid, p.sentence());
    }
    match c.set_ipv4(&iface, &want) {
        Ok(IpApplied::Kept) => {
            let after = c.ipv4(&iface).unwrap_or(want);
            show(env, ip_json(&iface, &after), json)
        }
        Ok(IpApplied::RolledBack) => fail(
            env,
            ErrorCode::Failed,
            &format!(
                "{iface} did not reach the network with the new address within {} seconds, so it went back to how it was",
                netconf::IP_TRIAL_SECS
            ),
        ),
        Err(e) => control_error(env, e),
    }
}

// ---- wol vpn ----

fn vpn_list(env: &mut dyn Env, json: bool) -> i32 {
    match env.controls().vpns() {
        Some(v) => show(env, json!({ "vpn": v }), json),
        None => fail(env, ErrorCode::Unavailable, "NetworkManager is not running"),
    }
}

fn vpn_error(env: &mut dyn Env, e: ControlError) -> i32 {
    match e {
        ControlError::Unsupported => fail(
            env,
            ErrorCode::Unavailable,
            "OpenVPN files need NetworkManager's OpenVPN plugin (networkmanager-openvpn), which is not installed",
        ),
        ControlError::Invalid => fail(
            env,
            ErrorCode::Invalid,
            "that is not a WireGuard (.conf) or OpenVPN (.ovpn) file NetworkManager can read",
        ),
        ControlError::NotFound => fail(env, ErrorCode::Failed, "no VPN of that name"),
        ControlError::WrongPassword => fail(
            env,
            ErrorCode::Denied,
            "the VPN needs its password or key (give it on stdin)",
        ),
        e => control_error(env, e),
    }
}

/// `wol vpn import <file>`: a WireGuard `.conf` or OpenVPN `.ovpn` file.
fn vpn_import(env: &mut dyn Env, file: &str, json: bool) -> i32 {
    let p = if let Some(rest) = file.strip_prefix("~/") {
        env.home().join(rest)
    } else {
        std::path::PathBuf::from(file)
    };
    let p = std::path::absolute(&p).unwrap_or(p);
    match env.controls().vpn_import(&p) {
        Ok(v) => show(env, json!({ "imported": v }), json),
        // nmcli's own reason after the sentence (net3-vpn: it was dropped).
        Err(e @ (ControlError::Invalid | ControlError::Failed)) => {
            match wol_core::vpn::take_import_reason() {
                Some(why) => fail(
                    env,
                    if e == ControlError::Invalid {
                        ErrorCode::Invalid
                    } else {
                        ErrorCode::Failed
                    },
                    &format!("NetworkManager could not import {file}: {why}"),
                ),
                None => vpn_error(env, e),
            }
        }
        Err(e) => vpn_error(env, e),
    }
}

#[derive(Clone, Copy)]
enum Change {
    Connect,
    Disconnect,
    Forget,
}

/// The profile named `name` (or with that UUID): its UUID and kind.
fn vpn_uuid(env: &mut dyn Env, name: &str) -> Result<(String, VpnKind), ControlError> {
    let list = env.controls().vpns().ok_or(ControlError::Unavailable)?;
    list.into_iter()
        .find(|v| v.name == name || v.uuid == name)
        .map(|v| (v.uuid, v.kind))
        .ok_or(ControlError::NotFound)
}

fn vpn_change(env: &mut dyn Env, name: &str, how: Change, json: bool) -> i32 {
    let (uuid, kind) = match vpn_uuid(env, name) {
        Ok(u) => u,
        Err(e) => return vpn_error(env, e),
    };
    let c = env.controls();
    let r = match how {
        Change::Connect => match c.vpn_connect(&uuid, None) {
            // A WireGuard profile has no password to give: its key left the keyring (QA-A7-19).
            Err(ControlError::WrongPassword) if !kind.has_password() => {
                return fail(
                    env,
                    ErrorCode::Failed,
                    "the VPN's key is missing from this computer; import its file again (wol vpn import)",
                );
            }
            // OpenVPN with a password: asked once on stdin, remembered in the keyring.
            Err(ControlError::WrongPassword) => match env.stdin_line().filter(|p| !p.is_empty()) {
                Some(pw) => c.vpn_connect(&uuid, Some(&pw)),
                None => Err(ControlError::WrongPassword),
            },
            r => r,
        },
        Change::Disconnect => c.vpn_disconnect(&uuid),
        Change::Forget => c.vpn_forget(&uuid),
    };
    match r {
        Ok(()) => vpn_list(env, json),
        Err(e) => vpn_error(env, e),
    }
}

// ---- wol proxy ----

fn proxy_state(env: &mut dyn Env) -> Value {
    serde_json::to_value(env.controls().proxy()).unwrap_or_default()
}

/// `http=host:port https=… socks=… no-proxy=a,b`.
fn proxy_options(opts: &[&str]) -> Result<ProxyConfig, String> {
    let mut c = ProxyConfig::default();
    for o in opts {
        let Some((k, v)) = o.split_once('=') else {
            return Err(format!(
                "{o}: write it as key=value (http, https, socks, no-proxy)"
            ));
        };
        match k {
            "http" => c.http = v.into(),
            "https" => c.https = v.into(),
            "socks" => c.socks = v.into(),
            "no-proxy" => {
                c.no_proxy = netconf::parse_no_proxy(v)
                    .ok_or_else(|| format!("{v}: not a list of names, domains or networks"))?
            }
            _ => return Err(format!("{k}: unknown (http, https, socks, no-proxy)")),
        }
    }
    Ok(c)
}

fn proxy_set(env: &mut dyn Env, c: ProxyConfig, json: bool) -> i32 {
    if let Some(p) = netconf::proxy_problem(&c) {
        return fail(env, ErrorCode::Invalid, p);
    }
    with_password(env, json, proxy_state, &move |ctl, pw| {
        ctl.set_proxy(&c, pw)
    })
}

// ---- wol dns ----

fn dns_state(env: &mut dyn Env) -> Value {
    serde_json::to_value(env.controls().dns_over_tls()).unwrap_or_default()
}

/// `wol dns tls <off|opportunistic|strict> [server…]`.
fn dns_tls(env: &mut dyn Env, mode: &str, servers: &[&str], json: bool) -> i32 {
    let Some(mode) = DotMode::parse(mode) else {
        return fail(env, ErrorCode::Usage, "mode: off, opportunistic or strict");
    };
    let Some(servers) = netconf::parse_dot_servers(&servers.join(" ")) else {
        return fail(
            env,
            ErrorCode::Invalid,
            "each server is an IP address, optionally with its name after #, like 9.9.9.9#dns.quad9.net",
        );
    };
    let c = DotConfig { mode, servers };
    if let Some(p) = netconf::dot_problem(&c) {
        return fail(env, ErrorCode::Invalid, p);
    }
    with_password(env, json, dns_state, &move |ctl, pw| {
        ctl.set_dns_over_tls(&c, pw)
    })
}

// ---- wol airplane ----

fn airplane_file(env: &dyn Env) -> std::path::PathBuf {
    netconf::airplane_path(&env.state_home())
}

fn airplane_state(env: &mut dyn Env) -> Value {
    let saved = env.read_file(&airplane_file(env));
    let c = env.controls();
    json!({
        "on": c.airplane(saved.as_deref()),
        "radios": c.radios().unwrap_or_default().iter().map(|r| json!({"kind": r.kind, "on": !r.blocked})).collect::<Vec<_>>(),
    })
}

/// `wol airplane on|off` (NET-21): every radio off, then back to how they were.
fn airplane_set(env: &mut dyn Env, on: bool, json: bool) -> i32 {
    let path = airplane_file(env);
    let saved = env.read_file(&path);
    match env.controls().set_airplane(on, saved.as_deref()) {
        Ok(keep) => {
            let w = match keep {
                Some(text) => {
                    if let Some(d) = path.parent() {
                        let _ = env.make_dir(d);
                    }
                    env.write_file(&path, &text)
                }
                None => env.remove_file(&path),
            };
            if let Err(e) = w {
                return fail(env, ErrorCode::Failed, &format!("{}: {e}", path.display()));
            }
            let v = airplane_state(env);
            show(env, v, json)
        }
        Err(ControlError::Unavailable) => fail(
            env,
            ErrorCode::Unavailable,
            "this computer has no radio to switch off (rfkill lists none)",
        ),
        Err(e) => control_error(env, e),
    }
}
