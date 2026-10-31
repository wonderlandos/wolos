// SPDX-License-Identifier: GPL-2.0-or-later
//! Machine facts for libwol's service calls (ADR 0020), said in the app API's own words. An app
//! gets what it needs to adapt, and no more: no MAC address, IP address or network name.
use serde_json::{Value, json};
use std::process::{Child, Command, Stdio};
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
        let child = Command::new("systemd-inhibit")
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
    use wol_core::system::fake::FakeHost;

    #[test]
    fn a_desktop_has_no_battery() {
        let v = power(&FakeHost::linux());
        assert_eq!(v, json!({"on_battery": false, "low_power": false}));
    }
}
