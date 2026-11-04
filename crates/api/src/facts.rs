// SPDX-License-Identifier: GPL-2.0-or-later
//! Machine facts for libwol's service calls (ADR 0020), said in the app API's own words. An app
//! gets what it needs to adapt, and no more: no MAC address, IP address or network name.
use serde_json::{Value, json};
use std::process::{Child, Stdio};
use wol_core::netinfo::{self, IfKind};
use wol_core::power::{self, Charge, Profile};
use wol_core::system::Host;

pub fn power(h: &dyn Host) -> Value {
    let battery = power::battery(h);
    let profile = power::profile(h);
    let mut v = json!({
        "on_battery": battery.as_ref().is_some_and(|b| b.charge == Charge::Discharging),
        "low_power": profile == Some(Profile::LowPower),
    });
    if let Some(b) = battery {
        v["battery"] = json!({
            "percent": b.percent,
            "charging": b.charge == Charge::Charging,
            "full": b.charge == Charge::Full,
        });
        if let Some(m) = b.minutes_left {
            v["battery"]["minutes_left"] = json!(m);
        }
    }
    v
}

/// The account `uid` from a passwd file: its login and the full name, nothing else of the line.
pub fn account(passwd: &str, uid: u32) -> Option<Value> {
    let line = passwd.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() >= 5 && f[2].parse() == Ok(uid)).then_some(f)
    })?;
    let full = line[4].split(',').next().unwrap_or_default();
    let full = if full.is_empty() { line[0] } else { full };
    Some(json!({ "name": line[0], "full_name": full }))
}

/// `online`: a default route (IPv4 or IPv6) and NetworkManager does not say "no network";
/// `limited`: the network does not reach the internet; `portal`: a sign-in page holds it (QA-NET-12,
/// NET2-3); `metered`: the connection costs by the byte (so apps hold big downloads).
pub fn network(h: &dyn Host) -> Value {
    let n = netinfo::read(h);
    let kind = |k: IfKind| match k {
        IfKind::Ethernet => "ethernet",
        IfKind::WiFi => "wifi",
        IfKind::Vpn => "vpn",
        IfKind::Other => "other",
    };
    let interfaces: Vec<Value> = n
        .interfaces
        .iter()
        .map(|i| json!({"name": i.name, "kind": kind(i.kind), "up": i.up}))
        .collect();
    json!({
        "online": n.online(),
        "limited": n.limited(),
        "portal": n.portal(),
        "metered": n.metered,
        "interfaces": interfaces,
    })
}

/// Keeps the machine awake while the value lives.
pub trait Inhibit: Send + Sync {
    fn hold(&self, app: &str, why: &str) -> Option<Box<dyn Send>>;
}

/// logind's inhibitor lock, held by a child process.
pub struct Logind;

struct Lock(Child);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Inhibit for Logind {
    fn hold(&self, app: &str, why: &str) -> Option<Box<dyn Send>> {
        let child = crate::privileged::privileged_command("systemd-inhibit")
            .args(["--what=idle:sleep", "--mode=block"])
            .arg(format!("--who={app}"))
            .arg(format!("--why={why}"))
            .args(["sleep", "infinity"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        Some(Box::new(Lock(child)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_is_named_by_its_uid_alone() {
        let t = "root:x:0:0:root:/root:/bin/bash\n\
                 alice:x:1000:1000:Alice Liddell,Room 1,,:/home/alice:/bin/zsh\n\
                 bob:x:1001:1001::/home/bob:/bin/zsh\n";
        assert_eq!(
            account(t, 1000),
            Some(json!({"name": "alice", "full_name": "Alice Liddell"}))
        );
        assert_eq!(account(t, 1001).unwrap()["full_name"], "bob");
        assert_eq!(account(t, 7), None);
        assert_eq!(account("not a line\n", 0), None);
    }
    use wol_core::system::fake::FakeHost;

    #[test]
    fn network_says_how_not_where() {
        let h = FakeHost::linux();
        h.command(
            "ip -j addr",
            r#"[{"ifname":"eth0","operstate":"UP","address":"aa:bb:cc:dd:ee:ff","link_type":"ether",
                "addr_info":[{"family":"inet","local":"192.168.1.5","prefixlen":24}]}]"#,
        );
        h.command(
            "ip route show default",
            "default via 192.168.1.1 dev eth0\n",
        );
        let v = network(&h);
        assert_eq!(v["online"], true);
        assert_eq!(v["interfaces"][0]["name"], "eth0");
        let text = v.to_string();
        assert!(
            !text.contains("aa:bb") && !text.contains("192.168"),
            "{text}"
        );
    }

    /// QA-NET-12 / NET2-3: `{online, limited, portal}` from NetworkManager; IPv6-only is online.
    #[test]
    fn limited_state() {
        let h = FakeHost::linux();
        h.command(
            "ip route show default",
            "default via 10.99.0.1 dev enp0s2\n",
        )
        .command("nmcli -t -g CONNECTIVITY general status", "portal\n");
        let v = network(&h);
        assert_eq!(
            (&v["online"], &v["limited"], &v["portal"]),
            (&json!(true), &json!(false), &json!(true))
        );
        h.command("nmcli -t -g CONNECTIVITY general status", "limited\n");
        assert_eq!(network(&h)["limited"], true);
        let h6 = FakeHost::linux();
        h6.command(
            "ip -6 route show default",
            "default via fe80::1 dev enp0s1 proto ra metric 100\n",
        )
        .command("nmcli -t -g CONNECTIVITY general status", "full\n")
        .command(
            "busctl get-property org.freedesktop.NetworkManager /org/freedesktop/NetworkManager org.freedesktop.NetworkManager Metered",
            "u 3\n",
        );
        let v = network(&h6);
        assert_eq!((&v["online"], &v["metered"]), (&json!(true), &json!(true)));
        assert!(!v.to_string().contains("fe80"), "{v}");
    }

    #[test]
    fn a_desktop_has_no_battery() {
        let v = power(&FakeHost::linux());
        assert_eq!(v, json!({"on_battery": false, "low_power": false}));
    }
}
