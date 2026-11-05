// SPDX-License-Identifier: BSD-2-Clause
//! Network settings (NET-3): a link's IPv4 address with a 15 s trial (QA-LIVE3-14), VPN profiles
//! with their secrets in the login keyring, the system proxy, DNS over TLS and airplane mode.
//! Pure parts in [`crate::netconf`] and [`crate::vpn`]. NetworkManager and rfkill let the person
//! at the computer make the link, VPN and radio changes (polkit, uaccess); the proxy and DNS
//! files are root's (wol-admind `proxy`, `dns-over-tls`). Every change runs through
//! [`Controls::stdin`] (secrets on stdin, nmcli's stderr kept for the cause).
use super::{ControlError, Controls};
use crate::netconf::{self, DotConfig, IpApplied, Ipv4Config, ProxyConfig, Radio};
use crate::netinfo::{self, Connectivity};
use crate::vpn::{self, Vpn, VpnKind};
use shared::admin::AdminAction;
use std::path::Path;
use std::time::Duration;

/// A link name nmcli may take as an argument.
fn iface_ok(s: &str) -> bool {
    (1..=15).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.@".contains(&b))
        && !s.starts_with('-')
}

impl Controls {
    fn exec(&self, argv: &[String], input: &str) -> Result<String, String> {
        let a: Vec<&str> = argv.iter().map(String::as_str).collect();
        (self.stdin)(&a, input)
    }

    fn read(&self, argv: &[String]) -> Option<String> {
        let a: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.host.run(&a)
    }

    // ---- IPv4 of one link (QA-LIVE3-14) ----

    /// The profile active on `iface`; None when it has none (unplugged) or nmcli does not answer.
    pub fn link_uuid(&self, iface: &str) -> Option<String> {
        if !iface_ok(iface) {
            return None;
        }
        self.host
            .run(&netconf::device_uuid_argv(iface))
            .map(|s| s.trim().to_string())
            .filter(|u| netconf::is_uuid(u))
    }

    /// How `iface`'s profile gets its IPv4 address now.
    pub fn ipv4(&self, iface: &str) -> Option<Ipv4Config> {
        let u = self.link_uuid(iface)?;
        self.read(&netconf::ipv4_read_argv(&u))
            .map(|t| netconf::parse_ipv4(&t))
    }

    /// [`Controls::set_ipv4_with`] waiting a real second between checks (slow: off the UI thread).
    pub fn set_ipv4(&self, iface: &str, c: &Ipv4Config) -> Result<IpApplied, ControlError> {
        self.set_ipv4_with(iface, c, &std::thread::sleep)
    }

    /// Saves `c` on `iface`'s profile and brings it up again. A Manual address then gets
    /// [`netconf::IP_TRIAL_SECS`]: when the link is not back on the network by then, or reaches
    /// less than before (full internet before, none after), the profile goes back to what it was
    /// (its earlier Manual address, else Automatic/DHCP) and the answer is
    /// [`IpApplied::RolledBack`]. `wait` sleeps between the checks.
    pub fn set_ipv4_with(
        &self,
        iface: &str,
        c: &Ipv4Config,
        wait: &dyn Fn(Duration),
    ) -> Result<IpApplied, ControlError> {
        if netconf::validate_ipv4(c).is_err() {
            return Err(ControlError::Invalid);
        }
        let uuid = self.link_uuid(iface).ok_or(ControlError::Unavailable)?;
        let previous = self.ipv4(iface).unwrap_or_default();
        let before = netconf::reach(
            self.host
                .run(&netinfo::CONNECTIVITY_ARGV)
                .map(|t| netinfo::parse_connectivity(&t))
                .unwrap_or_default(),
        );
        self.exec(&netconf::ipv4_modify_argv(&uuid, c), "")
            .map_err(|_| ControlError::Failed)?;
        let up = self.exec(&netconf::up_uuid_argv(&uuid), "");
        if !c.manual {
            return up
                .map(|_| IpApplied::Kept)
                .map_err(|_| ControlError::Failed);
        }
        if up.is_ok() {
            let start = self.host.now();
            for _ in 0..netconf::IP_TRIAL_SECS {
                wait(netconf::IP_TRIAL_STEP);
                if self.link_reaches(iface, before) {
                    return Ok(IpApplied::Kept);
                }
                if self.host.now() - start >= netconf::IP_TRIAL_SECS as f64 {
                    break;
                }
            }
        }
        // Back to what it was: the address that was typed cut this computer off. A link that
        // had a working Manual address (a network with no DHCP server) gets that one back, not
        // Automatic, which would leave it offline (QA-A7-6).
        self.exec(&netconf::ipv4_modify_argv(&uuid, &previous), "")
            .and_then(|_| self.exec(&netconf::up_uuid_argv(&uuid), ""))
            .map(|_| IpApplied::RolledBack)
            .map_err(|_| ControlError::Failed)
    }

    /// `iface` is connected and reaches at least as far as `before` ([`netconf::reach`]).
    fn link_reaches(&self, iface: &str, before: u8) -> bool {
        let on = self
            .host
            .run(&netconf::device_state_argv(iface))
            .is_some_and(|s| netconf::device_connected(&s));
        on && netconf::reach(
            self.host
                .run(&netconf::CONNECTIVITY_CHECK_ARGV)
                .map(|t| netinfo::parse_connectivity(&t))
                .unwrap_or(Connectivity::Unknown),
        ) >= before
    }

    // ---- VPN ----

    /// The VPN profiles NetworkManager has; None when it does not answer.
    pub fn vpns(&self) -> Option<Vec<Vpn>> {
        let mut v = vpn::parse_list(&self.host.run(&vpn::LIST_ARGV)?);
        for p in v.iter_mut().filter(|p| p.kind == VpnKind::Other) {
            if let Some(s) = self.read(&vpn::service_type_argv(&p.uuid)) {
                p.kind = vpn::kind_of_service(&s);
            }
        }
        Some(v)
    }

    fn vpn_by_uuid(&self, uuid: &str) -> Result<Vpn, ControlError> {
        if !netconf::is_uuid(uuid) {
            return Err(ControlError::Invalid);
        }
        self.vpns()
            .ok_or(ControlError::Unavailable)?
            .into_iter()
            .find(|v| v.uuid == uuid)
            .ok_or(ControlError::NotFound)
    }

    /// Imports a WireGuard `.conf` or OpenVPN `.ovpn` file. A WireGuard file is copied under a
    /// name NetworkManager takes as the tunnel's interface ([`vpn::interface_name`]) first. Its
    /// private key then moves into the login keyring and out of the profile; when no keyring
    /// answers it stays in NetworkManager's root-only profile file. Unsupported: OpenVPN's plugin
    /// is not installed.
    pub fn vpn_import(&self, path: &Path) -> Result<Vpn, ControlError> {
        let p = path.to_str().ok_or(ControlError::Invalid)?;
        let kind = vpn::file_kind(p).ok_or(ControlError::Invalid)?;
        if kind == VpnKind::OpenVpn && self.host.read_text(vpn::OPENVPN_PLUGIN).is_none() {
            return Err(ControlError::Unsupported);
        }
        let staged = match kind {
            VpnKind::WireGuard => Some(
                crate::system::stage_copy(path, &format!("{}.conf", vpn::interface_name(p)))
                    .ok_or(ControlError::NotFound)?,
            ),
            _ => None,
        };
        let from = staged.as_ref().map_or(p, |(_, f)| f.to_str().unwrap_or(p));
        let r = vpn::import_argv(kind, from)
            .ok_or(ControlError::Invalid)
            .and_then(|a| {
                self.exec(&a, "")
                    .map_err(|e| match vpn::classify_import_error(&e) {
                        vpn::ImportFailure::NoPlugin => ControlError::Unsupported,
                        vpn::ImportFailure::BadFile => ControlError::Invalid,
                        vpn::ImportFailure::Other => ControlError::Failed,
                    })
            });
        if let Some((dir, file)) = &staged {
            crate::system::remove_staged(dir, file);
        }
        let uuid = vpn::parse_import(&r?).ok_or(ControlError::Failed)?;
        let name = self
            .vpns()
            .and_then(|l| l.into_iter().find(|v| v.uuid == uuid))
            .map_or_else(|| vpn::interface_name(p), |v| v.name);
        if kind == VpnKind::WireGuard {
            self.secret_to_keyring(&uuid, &name, kind);
        }
        Ok(Vpn {
            name,
            uuid,
            kind,
            active: false,
        })
    }

    /// Moves the profile's secret into the keyring; the profile keeps it when the keyring
    /// does not take it. True when it moved.
    fn secret_to_keyring(&self, uuid: &str, name: &str, kind: VpnKind) -> bool {
        let Some(key) = kind.secret_key() else {
            return false;
        };
        let secret = self
            .read(&vpn::read_secret_argv(uuid, key))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let Some(secret) = secret else {
            return false;
        };
        self.exec(&vpn::keyring_store_argv(uuid, name, key), &secret)
            .is_ok()
            && vpn::forget_secret_argv(uuid, kind).is_some_and(|a| self.exec(&a, "").is_ok())
    }

    /// Connects. The secret comes from `password` (OpenVPN asks for one), else the keyring, and
    /// goes on stdin; a typed password that worked is remembered in the keyring. WrongPassword:
    /// NetworkManager needs a secret it did not get (Settings asks for it).
    pub fn vpn_connect(&self, uuid: &str, password: Option<&str>) -> Result<(), ControlError> {
        let v = self.vpn_by_uuid(uuid)?;
        let key = v.kind.secret_key();
        let secret = password.map(String::from).or_else(|| {
            key.and_then(|k| self.read(&vpn::keyring_lookup_argv(uuid, k)))
                .map(|s| s.trim_end_matches('\n').to_string())
                .filter(|s| !s.is_empty())
        });
        let r = match (key, &secret) {
            (Some(k), Some(s)) => {
                self.exec(&vpn::up_with_secret_argv(uuid), &vpn::secret_line(k, s))
            }
            _ => self.exec(&netconf::up_uuid_argv(uuid), ""),
        };
        match r {
            Ok(_) => {
                if let (Some(k), Some(p)) = (key, password) {
                    let _ = self.exec(&vpn::keyring_store_argv(uuid, &v.name, k), p);
                }
                Ok(())
            }
            Err(e) => Err(
                match crate::wifi::classify_join_error(&e, secret.is_some()) {
                    crate::wifi::JoinFailure::WrongPassword => ControlError::WrongPassword,
                    _ if e.contains("Secrets were required") || e.contains("no secrets") => {
                        ControlError::WrongPassword
                    }
                    _ => ControlError::Failed,
                },
            ),
        }
    }

    /// Disconnects; the profile stays for next time.
    pub fn vpn_disconnect(&self, uuid: &str) -> Result<(), ControlError> {
        self.vpn_by_uuid(uuid)?;
        self.exec(&vpn::down_argv(uuid), "")
            .map(|_| ())
            .map_err(|_| ControlError::Failed)
    }

    /// Deletes the profile and its secrets in the keyring.
    pub fn vpn_forget(&self, uuid: &str) -> Result<(), ControlError> {
        self.vpn_by_uuid(uuid)?;
        self.exec(&vpn::delete_argv(uuid), "")
            .map_err(|_| ControlError::Failed)?;
        let _ = self.exec(&vpn::keyring_clear_argv(uuid), "");
        Ok(())
    }

    // ---- Proxy ----

    /// The proxy as wol-admind last saved it (Off when never set).
    pub fn proxy(&self) -> ProxyConfig {
        self.host
            .read_text(netconf::PROXY_CONF)
            .map(|t| netconf::parse_proxy_conf(&t))
            .unwrap_or_default()
    }

    /// Saves the proxy for the whole computer (wol-admind writes /etc/wolos/proxy.conf and the
    /// login environment), then sets it for this session: the desktop's proxy keys and
    /// NetworkManager's PAC setting on the link that carries the default route.
    pub fn set_proxy(&self, c: &ProxyConfig, pw: &str) -> Result<(), ControlError> {
        if netconf::proxy_problem(c).is_some() {
            return Err(ControlError::Invalid);
        }
        let args = netconf::proxy_args(c);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.admin(AdminAction::Proxy, &args, pw)?;
        self.apply_proxy_session(c)
    }

    /// The person's side of [`Controls::set_proxy`] (no password).
    pub fn apply_proxy_session(&self, c: &ProxyConfig) -> Result<(), ControlError> {
        let mut ok = true;
        for a in netconf::gsettings_argvs(c) {
            ok &= self.exec(&a, "").is_ok();
        }
        let route = self
            .host
            .run(&["ip", "route", "show", "default"])
            .and_then(|t| netinfo::parse_route_dev(&t));
        if let Some(u) = route.and_then(|d| self.link_uuid(&d)) {
            ok &= self.exec(&netconf::nm_proxy_argv(&u, c), "").is_ok();
        }
        if ok {
            Ok(())
        } else {
            Err(ControlError::Failed)
        }
    }

    // ---- DNS over TLS ----

    /// systemd-resolved's DNS over TLS as Settings last saved it.
    pub fn dns_over_tls(&self) -> DotConfig {
        self.host
            .read_text(netconf::DOT_DROPIN)
            .map(|t| netconf::parse_dot(&t))
            .unwrap_or_default()
    }

    /// Saves DNS over TLS for the whole computer (wol-admind `dns-over-tls`).
    pub fn set_dns_over_tls(&self, c: &DotConfig, pw: &str) -> Result<(), ControlError> {
        if netconf::dot_problem(c).is_some() {
            return Err(ControlError::Invalid);
        }
        let args = netconf::dot_args(c);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.admin(AdminAction::DnsOverTls, &args, pw)
    }

    // ---- Airplane mode ----

    /// Every radio (rfkill); None when rfkill does not answer.
    pub fn radios(&self) -> Option<Vec<Radio>> {
        self.host
            .run(&netconf::RFKILL_LIST_ARGV)
            .and_then(|t| netconf::parse_rfkill(&t))
    }

    /// Airplane mode is on ([`netconf::airplane_on`]); None on a computer with no radio.
    /// `saved` is the text of `<state home>/`[`netconf::AIRPLANE_FILE`] (None: no file).
    pub fn airplane(&self, saved: Option<&str>) -> Option<bool> {
        let r = self.radios().filter(|r| !r.is_empty())?;
        Some(netconf::airplane_on(saved, &r))
    }

    /// On: switches every radio off (Wi-Fi, Bluetooth, mobile broadband) and answers the list
    /// of the ones that were on, for the caller to save (`Some(text)`; the saved one is kept
    /// when airplane mode was on already). Off: switches back on only the ones in `saved`
    /// (every radio without a list) and answers None: the file goes.
    pub fn set_airplane(
        &self,
        on: bool,
        saved: Option<&str>,
    ) -> Result<Option<String>, ControlError> {
        let r = self
            .radios()
            .filter(|r| !r.is_empty())
            .ok_or(ControlError::Unavailable)?;
        let run = |a: &[&str]| self.exec(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "");
        if on {
            let keep = match saved {
                Some(t) if netconf::airplane_on(saved, &r) => t.to_string(),
                _ => netconf::airplane_file_text(&r),
            };
            return run(&["rfkill", "block", "all"])
                .map(|_| Some(keep))
                .map_err(|_| ControlError::Failed);
        }
        let kinds = match saved {
            Some(t) => netconf::parse_airplane_file(t),
            None => {
                let mut k: Vec<String> = Vec::new();
                for x in &r {
                    if !k.contains(&x.kind) {
                        k.push(x.kind.clone());
                    }
                }
                k
            }
        };
        let mut ok = true;
        for k in &kinds {
            ok &= run(&["rfkill", "unblock", k]).is_ok();
        }
        if ok {
            Ok(None)
        } else {
            Err(ControlError::Failed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;
    use shared::admin::AdminReply;
    use std::sync::{Arc, Mutex};

    const U: &str = "6a3f0c2e-1111-4a5b-9c8d-0123456789ab";

    /// Controls on `h` whose every change is logged; `fail` lists command prefixes that fail
    /// with the given stderr.
    fn ctl(
        h: Arc<FakeHost>,
        fail: Vec<(&'static str, &'static str)>,
    ) -> (Controls, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (l1, l2) = (log.clone(), log.clone());
        let c = Controls {
            host: h,
            stdin: Arc::new(move |argv, input| {
                let line = argv.join(" ");
                l1.lock().unwrap().push(if input.is_empty() {
                    line.clone()
                } else {
                    format!("{line} <{}", input.trim())
                });
                match fail.iter().find(|(p, _)| line.starts_with(p)) {
                    Some((_, e)) => Err((*e).into()),
                    None if line.starts_with("nmcli connection import") => {
                        Ok(format!("Connection 'wg0' ({U}) successfully added.\n"))
                    }
                    None => Ok(String::new()),
                }
            }),
            admin: Arc::new(move |req, pw| {
                l2.lock().unwrap().push(format!(
                    "admin {} {}",
                    req.action.name(),
                    req.args.join(" ")
                ));
                Some(if pw.is_empty() {
                    AdminReply::NeedsAuth
                } else {
                    AdminReply::Ok
                })
            }),
            wlan: "wlan0".into(),
        };
        (c, log)
    }

    fn manual() -> Ipv4Config {
        Ipv4Config {
            manual: true,
            address: "10.0.2.50/24".into(),
            router: "10.0.2.2".into(),
            dns: vec!["10.0.2.3".into()],
        }
    }

    fn link(h: &FakeHost) {
        h.command(
            "nmcli -g GENERAL.CON-UUID device show enp0s1",
            &format!("{U}\n"),
        )
        .command("nmcli -t -g CONNECTIVITY general status", "full\n");
    }

    /// QA-LIVE3-14: a Manual address that keeps the computer online stays.
    #[test]
    fn static_ip_kept_when_online() {
        let h = Arc::new(FakeHost::linux());
        link(&h);
        h.command(
            "nmcli -g GENERAL.STATE device show enp0s1",
            "100 (connected)\n",
        )
        .command("nmcli networking connectivity check", "full\n");
        let (c, log) = ctl(h, vec![]);
        let waits = Mutex::new(0);
        let r = c.set_ipv4_with("enp0s1", &manual(), &|_| *waits.lock().unwrap() += 1);
        assert_eq!(r, Ok(IpApplied::Kept));
        assert_eq!(*waits.lock().unwrap(), 1, "kept at the first check");
        let log = log.lock().unwrap();
        assert_eq!(
            *log,
            [
                format!(
                    "nmcli connection modify uuid {U} ipv4.method manual ipv4.addresses 10.0.2.50/24 ipv4.gateway 10.0.2.2 ipv4.dns 10.0.2.3"
                ),
                format!("nmcli connection up uuid {U}"),
            ]
        );
    }

    /// QA-LIVE3-14: a Manual address that cuts the computer off goes back to Automatic after
    /// the 15 s trial.
    #[test]
    fn static_ip_rolls_back_when_cut_off() {
        let h = Arc::new(FakeHost::linux());
        link(&h);
        h.command(
            "nmcli -g GENERAL.STATE device show enp0s1",
            "100 (connected)\n",
        )
        .command("nmcli networking connectivity check", "limited\n");
        let (c, log) = ctl(h, vec![]);
        let waits = Mutex::new(0);
        let r = c.set_ipv4_with("enp0s1", &manual(), &|_| *waits.lock().unwrap() += 1);
        assert_eq!(r, Ok(IpApplied::RolledBack));
        assert_eq!(*waits.lock().unwrap(), 15, "15 one-second checks");
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 4, "{log:?}");
        assert!(log[2].contains("ipv4.method auto"), "{log:?}");
        assert_eq!(log[3], format!("nmcli connection up uuid {U}"));
    }

    /// QA-A7-6: a link that already had a working Manual address (no DHCP server on that
    /// network) goes back to THAT address when the edited one cuts it off, not to Automatic.
    #[test]
    fn static_ip_rolls_back_to_the_earlier_manual_address() {
        let h = Arc::new(FakeHost::linux());
        link(&h);
        h.command(
            &format!("nmcli -t -f ipv4.method,ipv4.addresses,ipv4.gateway,ipv4.dns connection show uuid {U}"),
            "ipv4.method:manual\nipv4.addresses:192.168.1.20/24\nipv4.gateway:192.168.1.1\nipv4.dns:192.168.1.1\n",
        )
        .command(
            "nmcli -g GENERAL.STATE device show enp0s1",
            "100 (connected)\n",
        )
        .command("nmcli networking connectivity check", "limited\n");
        let (c, log) = ctl(h, vec![]);
        let r = c.set_ipv4_with("enp0s1", &manual(), &|_| ());
        assert_eq!(r, Ok(IpApplied::RolledBack));
        let log = log.lock().unwrap();
        assert!(
            log[2].contains(
                "ipv4.method manual ipv4.addresses 192.168.1.20/24 ipv4.gateway 192.168.1.1 ipv4.dns 192.168.1.1"
            ),
            "{log:?}"
        );
    }

    /// The profile does not come up with the new address: undone at once.
    #[test]
    fn static_ip_rolls_back_when_up_fails() {
        let h = Arc::new(FakeHost::linux());
        link(&h);
        let (c, log) = ctl(
            h,
            vec![("nmcli connection up", "Error: Connection activation failed")],
        );
        let r = c.set_ipv4_with("enp0s1", &manual(), &|_| panic!("no trial"));
        // The rollback's own `up` fails too here: said so.
        assert_eq!(r, Err(ControlError::Failed));
        assert!(log.lock().unwrap()[2].contains("ipv4.method auto"));
    }

    #[test]
    fn static_ip_refusals_and_reads() {
        let h = Arc::new(FakeHost::linux());
        let (c, _) = ctl(h.clone(), vec![]);
        let mut bad = manual();
        bad.router = "192.168.9.1".into();
        assert_eq!(
            c.set_ipv4_with("enp0s1", &bad, &|_| ()),
            Err(ControlError::Invalid)
        );
        assert_eq!(
            c.set_ipv4_with("enp0s1", &manual(), &|_| ()),
            Err(ControlError::Unavailable),
            "no active profile"
        );
        assert_eq!(c.link_uuid("en p0;rm"), None);
        link(&h);
        h.command(
            &format!("nmcli -t -f ipv4.method,ipv4.addresses,ipv4.gateway,ipv4.dns connection show uuid {U}"),
            "ipv4.method:auto\nipv4.addresses:\nipv4.gateway:--\nipv4.dns:\n",
        );
        assert_eq!(c.ipv4("enp0s1"), Some(Ipv4Config::default()));
        assert_eq!(
            c.set_ipv4_with("enp0s1", &Ipv4Config::default(), &|_| panic!("no trial")),
            Ok(IpApplied::Kept)
        );
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wolos-net3-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// NET-21: a WireGuard file is imported under a valid interface name, its key moves into
    /// the keyring, and connecting hands the key back on stdin.
    #[test]
    fn wireguard_import_connect_forget() {
        let d = temp("wg");
        let f = d.join("My Office.conf");
        std::fs::write(&f, "[Interface]\nPrivateKey = abc=\n").unwrap();
        let h = Arc::new(FakeHost::linux());
        h.command(
            "nmcli -t -f NAME,UUID,TYPE,ACTIVE connection show",
            &format!("My-Office:{U}:wireguard:no\n"),
        )
        .command(
            &format!("nmcli -s -g wireguard.private-key connection show uuid {U}"),
            "abc=\n",
        )
        .command(
            &format!("secret-tool lookup application wolos vpn {U} setting wireguard.private-key"),
            "abc=",
        );
        let (c, log) = ctl(h.clone(), vec![]);
        let v = c.vpn_import(&f).unwrap();
        assert_eq!((v.name.as_str(), v.kind), ("My-Office", VpnKind::WireGuard));
        {
            let log = log.lock().unwrap();
            assert!(
                log[0].starts_with("nmcli connection import type wireguard file /")
                    && log[0].ends_with("/My-Office.conf"),
                "{log:?}"
            );
            assert_eq!(
                log[1],
                format!(
                    "secret-tool store --label VPN My-Office application wolos vpn {U} setting wireguard.private-key <abc="
                )
            );
            assert!(log[2].contains("wireguard.private-key-flags 2"), "{log:?}");
        }
        c.vpn_connect(U, None).unwrap();
        assert_eq!(
            log.lock().unwrap()[3],
            format!(
                "nmcli connection up uuid {U} passwd-file /dev/stdin <wireguard.private-key:abc="
            )
        );
        c.vpn_disconnect(U).unwrap();
        c.vpn_forget(U).unwrap();
        let log = log.lock().unwrap();
        assert_eq!(log[4], format!("nmcli connection down uuid {U}"));
        assert_eq!(log[5], format!("nmcli connection delete uuid {U}"));
        assert_eq!(
            log[6],
            format!("secret-tool clear application wolos vpn {U}")
        );
        assert_eq!(
            c.vpn_connect("not-a-uuid", None),
            Err(ControlError::Invalid)
        );
        assert_eq!(
            c.vpn_connect("11111111-2222-3333-4444-555555555555", None),
            Err(ControlError::NotFound)
        );
        let _ = std::fs::remove_dir_all(d);
    }

    /// OpenVPN needs its plugin; a password it asks for is given on stdin and remembered.
    #[test]
    fn openvpn_plugin_and_password() {
        let h = Arc::new(FakeHost::linux());
        let (c, _) = ctl(h.clone(), vec![]);
        assert_eq!(
            c.vpn_import(Path::new("/tmp/office.ovpn")),
            Err(ControlError::Unsupported)
        );
        assert_eq!(
            c.vpn_import(Path::new("/tmp/notes.txt")),
            Err(ControlError::Invalid)
        );
        h.command(
            "nmcli -t -f NAME,UUID,TYPE,ACTIVE connection show",
            &format!("office:{U}:vpn:no\n"),
        )
        .command(
            &format!("nmcli -g vpn.service-type connection show uuid {U}"),
            "org.freedesktop.NetworkManager.openvpn\n",
        );
        assert_eq!(c.vpns().unwrap()[0].kind, VpnKind::OpenVpn);
        let (c, log) = ctl(
            h.clone(),
            vec![(
                "nmcli connection up uuid",
                "Error: Connection activation failed: Secrets were required, but not provided.",
            )],
        );
        assert_eq!(c.vpn_connect(U, None), Err(ControlError::WrongPassword));
        let (c, log2) = ctl(h, vec![]);
        c.vpn_connect(U, Some("s3cret")).unwrap();
        let l = log2.lock().unwrap();
        assert_eq!(
            l[0],
            format!(
                "nmcli connection up uuid {U} passwd-file /dev/stdin <vpn.secrets.password:s3cret"
            )
        );
        assert!(
            l[1].starts_with("secret-tool store --label VPN office") && l[1].ends_with("<s3cret")
        );
        assert_eq!(log.lock().unwrap().len(), 1);
    }

    #[test]
    fn proxy_and_dot_go_through_wol_admind() {
        let h = Arc::new(FakeHost::linux());
        h.command("ip route show default", "default via 10.0.2.2 dev enp0s1\n");
        link(&h);
        let (c, log) = ctl(h.clone(), vec![]);
        let p = ProxyConfig {
            mode: netconf::ProxyMode::Manual,
            http: "proxy.example:3128".into(),
            ..Default::default()
        };
        assert_eq!(c.set_proxy(&p, ""), Err(ControlError::NeedsAuth));
        c.set_proxy(&p, "pw").unwrap();
        let l = log.lock().unwrap().clone();
        assert_eq!(l[1], "admin proxy manual proxy.example:3128 - - -");
        assert_eq!(l[2], "gsettings set org.gnome.system.proxy mode manual");
        assert!(l.last().unwrap().contains("proxy.method none"), "{l:?}");
        assert_eq!(c.proxy(), ProxyConfig::default(), "nothing saved yet");
        h.file(netconf::PROXY_CONF, &netconf::render_proxy_conf(&p));
        assert_eq!(c.proxy(), p);
        let bad = ProxyConfig {
            mode: netconf::ProxyMode::Manual,
            ..Default::default()
        };
        assert_eq!(c.set_proxy(&bad, "pw"), Err(ControlError::Invalid));
        let d = DotConfig {
            mode: netconf::DotMode::Strict,
            servers: vec!["9.9.9.9#dns.quad9.net".into()],
        };
        c.set_dns_over_tls(&d, "pw").unwrap();
        assert_eq!(
            log.lock().unwrap().last().unwrap(),
            "admin dns-over-tls strict 9.9.9.9#dns.quad9.net"
        );
        h.file(netconf::DOT_DROPIN, &netconf::render_dot(&d));
        assert_eq!(c.dns_over_tls(), d);
    }

    /// NET-21 airplane: on saves which radios were on and blocks all; off brings back only those.
    #[test]
    fn airplane_restores_the_radios_that_were_on() {
        let h = Arc::new(FakeHost::linux());
        let (c, log) = ctl(h.clone(), vec![]);
        assert_eq!(c.airplane(None), None, "no rfkill answer");
        assert_eq!(c.set_airplane(true, None), Err(ControlError::Unavailable));
        h.command(
            "rfkill -J -o TYPE,SOFT",
            r#"{"rfkilldevices":[{"type":"wlan","soft":"unblocked"},{"type":"bluetooth","soft":"blocked"}]}"#,
        );
        assert_eq!(c.airplane(None), Some(false));
        let saved = c.set_airplane(true, None).unwrap();
        assert_eq!(saved.as_deref(), Some("wlan\n"));
        h.command(
            "rfkill -J -o TYPE,SOFT",
            r#"{"rfkilldevices":[{"type":"wlan","soft":"blocked"},{"type":"bluetooth","soft":"blocked"}]}"#,
        );
        assert_eq!(c.airplane(saved.as_deref()), Some(true));
        // Turned on twice: the list of what was on stays.
        assert_eq!(c.set_airplane(true, saved.as_deref()).unwrap(), saved);
        assert_eq!(c.set_airplane(false, saved.as_deref()), Ok(None));
        assert_eq!(
            *log.lock().unwrap(),
            [
                "rfkill block all",
                "rfkill block all",
                "rfkill unblock wlan"
            ],
            "Bluetooth was off before: it stays off"
        );
        log.lock().unwrap().clear();
        c.set_airplane(false, None).unwrap();
        assert_eq!(
            *log.lock().unwrap(),
            ["rfkill unblock wlan", "rfkill unblock bluetooth"],
            "no list: every radio"
        );
    }
}
