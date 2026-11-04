// SPDX-License-Identifier: GPL-2.0-or-later
//! The Tor switches of the OS and what turning one means (ADR 0056).
//!
//! Three scopes, each its own switch: Tor for Atlas tabs (default on, no privilege), Tor for chosen
//! apps (a namespace per app), and Tor for the whole machine. [`plan`] turns a change of switches
//! into the ordered actions the privileged daemon runs. The order is the safety property: rules that
//! drop are loaded before tor starts (a redirect to a tor that is not up yet fails closed) and
//! unloaded after it stops only when the person turned the scope off, never because tor died.
use crate::transparent::{self, AppNs, PrivStep, SystemGateway};
use std::collections::BTreeMap;

/// The switches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switches {
    /// Tor for Atlas tabs (the Tor group of ADR 0053). Default on: it only enables the menu item.
    pub atlas: bool,
    /// Apps that run in their own Tor namespace, with the index that fixes the app's address
    /// (assigned when the app is added and kept, so a running app never changes address).
    pub apps: BTreeMap<String, u8>,
    /// Every user's traffic through tor.
    pub system: bool,
    /// With `system`: the private ranges stay direct ([`transparent::lan_ranges`]).
    pub lan: bool,
    /// Keep Tor ready ([`crate::ready`]); default off.
    pub keep_ready: bool,
}

impl Default for Switches {
    fn default() -> Self {
        Self {
            atlas: true,
            apps: BTreeMap::new(),
            system: false,
            lan: false,
            keep_ready: false,
        }
    }
}

/// One thing the privileged daemon does. The variants are in the order they can be run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Load the system rules (redirect plus drop-by-default).
    LoadSystemRules,
    /// Start the machine-wide tor (the `wol-tor-gateway` unit).
    StartGateway,
    /// Set up the app bridge and its rules.
    GatewayUp,
    /// Create an app's namespace.
    AddApp(String),
    /// Re-read the gateway's torrc: the set of addresses tor must listen on changed while it ran.
    ReloadGateway,
    /// Remove an app's namespace.
    RemoveApp(String),
    /// Remove the app bridge and its rules.
    GatewayDown,
    /// Stop the machine-wide tor.
    StopGateway,
    /// Unload the system rules.
    UnloadSystemRules,
}

impl Switches {
    /// Whether the gateway tor must run.
    pub fn needs_gateway(&self) -> bool {
        self.system || !self.apps.is_empty()
    }

    /// Whether `app`'s traffic goes through tor (its own switch, or the machine's).
    pub fn covers(&self, app: &str) -> bool {
        self.system || self.apps.contains_key(app)
    }

    /// Adds an app with the lowest free index; `Err` when the name is not usable or the bridge
    /// has no room left. Adding an app that is there changes nothing.
    pub fn add_app(&mut self, name: &str) -> Result<u8, transparent::BadApp> {
        if let Some(i) = self.apps.get(name) {
            return Ok(*i);
        }
        let taken: Vec<u8> = self.apps.values().copied().collect();
        let free = (1..=transparent::MAX_APPS).find(|i| !taken.contains(i));
        let idx =
            free.ok_or_else(|| transparent::BadApp("no free address on the app bridge".into()))?;
        AppNs::new(name, idx)?;
        self.apps.insert(name.to_string(), idx);
        Ok(idx)
    }

    /// The text of a setting file: one `key value` per line.
    pub fn render(&self) -> String {
        let onoff = |b| if b { "on" } else { "off" };
        let mut o = format!(
            "atlas {}\nsystem {}\nlan {}\nkeep-ready {}\n",
            onoff(self.atlas),
            onoff(self.system),
            onoff(self.lan),
            onoff(self.keep_ready)
        );
        for (a, i) in &self.apps {
            o.push_str(&format!("app {a} {i}\n"));
        }
        o
    }

    /// Reads a setting file. Unknown or malformed lines are ignored, which can only turn a scope
    /// off (the defaults are the safe ones), never on by accident.
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for l in text.lines().map(str::trim) {
            let (k, v) = l.split_once(' ').unwrap_or((l, ""));
            match (k, v.trim()) {
                ("atlas", v) => s.atlas = v == "on",
                ("system", v) => s.system = v == "on",
                ("lan", v) => s.lan = v == "on",
                ("keep-ready", v) => s.keep_ready = v == "on",
                ("app", rest) => {
                    let (name, idx) = rest.split_once(' ').unwrap_or((rest, ""));
                    let taken = s.apps.values().any(|i| idx.trim().parse() == Ok(*i));
                    match idx.trim().parse::<u8>() {
                        Ok(i) if !taken && AppNs::new(name, i).is_ok() => {
                            s.apps.insert(name.to_string(), i);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        s
    }
}

/// The actions that take the machine from `from` to `to`, in the order to run them.
///
/// On: system rules, then the bridge (tor listens on its address, so it must exist first), then
/// tor, then the namespaces. Off: namespaces, then tor, then the bridge, then the system rules.
pub fn plan(from: &Switches, to: &Switches) -> Vec<Action> {
    let mut out = Vec::new();
    let app_names = |s: &Switches| -> Vec<String> { s.apps.keys().cloned().collect() };
    let (new_apps, old_apps): (Vec<String>, Vec<String>) = (
        app_names(to)
            .into_iter()
            .filter(|a| !from.apps.contains_key(a))
            .collect(),
        app_names(from)
            .into_iter()
            .filter(|a| !to.apps.contains_key(a))
            .collect(),
    );
    let listeners_changed = from.needs_gateway()
        && to.needs_gateway()
        && (from.system != to.system || from.apps.is_empty() != to.apps.is_empty());
    let adds_listener =
        (to.system && !from.system) || (!to.apps.is_empty() && from.apps.is_empty());
    if (to.system && !from.system) || (to.system && from.system && to.lan != from.lan) {
        out.push(Action::LoadSystemRules);
    }
    if !to.apps.is_empty() && from.apps.is_empty() {
        out.push(Action::GatewayUp);
    }
    if to.needs_gateway() && !from.needs_gateway() {
        out.push(Action::StartGateway);
    }
    out.extend(new_apps.into_iter().map(Action::AddApp));
    if listeners_changed && adds_listener {
        out.push(Action::ReloadGateway);
    }
    out.extend(old_apps.into_iter().map(Action::RemoveApp));
    if listeners_changed && !adds_listener {
        out.push(Action::ReloadGateway);
    }
    if !to.needs_gateway() && from.needs_gateway() {
        out.push(Action::StopGateway);
    }
    if to.apps.is_empty() && !from.apps.is_empty() {
        out.push(Action::GatewayDown);
    }
    if !to.system && from.system {
        out.push(Action::UnloadSystemRules);
    }
    out
}

/// What the steps need from the machine.
#[derive(Clone, Debug)]
pub struct Ctx {
    /// The user id of tor (exempt from the system rules).
    pub tor_uid: u32,
}

fn systemctl(args: &[&str]) -> PrivStep {
    let mut argv = vec!["systemctl".to_string()];
    argv.extend(args.iter().map(|s| s.to_string()));
    PrivStep {
        argv,
        stdin: None,
        write_file: None,
    }
}

fn nft_stdin(text: String) -> PrivStep {
    PrivStep {
        argv: vec!["nft".into(), "-f".into(), "-".into()],
        stdin: Some(text),
        write_file: None,
    }
}

fn system_rules(sw: &Switches, ctx: &Ctx) -> PrivStep {
    let lan = if sw.lan {
        transparent::lan_ranges()
    } else {
        vec![]
    };
    nft_stdin(
        transparent::system_ruleset(&SystemGateway {
            tor_uid: ctx.tor_uid,
            lan,
        })
        .render(),
    )
}

/// The privileged work of one action.
pub fn steps(action: &Action, from: &Switches, to: &Switches, ctx: &Ctx) -> Vec<PrivStep> {
    let ns = |name: &str, sw: &Switches| sw.apps.get(name).and_then(|i| AppNs::new(name, *i).ok());
    match action {
        Action::LoadSystemRules => vec![system_rules(to, ctx)],
        Action::UnloadSystemRules => vec![PrivStep {
            argv: ["nft", "destroy", "table", "inet", transparent::SYSTEM_TABLE]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            stdin: None,
            write_file: None,
        }],
        Action::StartGateway => vec![systemctl(&["start", GATEWAY_UNIT])],
        Action::StopGateway => vec![systemctl(&["stop", GATEWAY_UNIT])],
        // The listeners are written into the torrc at start, so a change is a restart; the rules
        // that are loaded keep the machine closed meanwhile.
        Action::ReloadGateway => vec![systemctl(&["restart", GATEWAY_UNIT])],
        Action::GatewayUp => transparent::gateway_up_steps(),
        Action::GatewayDown => transparent::gateway_down_steps(),
        Action::AddApp(n) => ns(n, to)
            .map(|n| transparent::netns_steps(&n))
            .unwrap_or_default(),
        Action::RemoveApp(n) => ns(n, from)
            .map(|n| transparent::netns_down_steps(&n))
            .unwrap_or_default(),
    }
}

/// The unit of the machine-wide tor.
pub const GATEWAY_UNIT: &str = "wol-tor-gateway.service";

/// The rule loads alone, for a firewall reload: system rules when that scope is on, and the app
/// bridge's host rules when apps are in a Tor scope. Idempotent (each ruleset replaces its table).
pub fn reload_rules_steps(sw: &Switches, ctx: &Ctx) -> Vec<PrivStep> {
    let mut v = Vec::new();
    if sw.system {
        v.push(system_rules(sw, ctx));
    }
    if !sw.apps.is_empty() {
        v.push(nft_stdin(transparent::app_host_ruleset().render()));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps(names: &[&str]) -> BTreeMap<String, u8> {
        let mut s = Switches::default();
        for n in names {
            s.add_app(n).unwrap();
        }
        s.apps
    }

    #[test]
    fn defaults_are_atlas_on_everything_else_off() {
        let d = Switches::default();
        assert!(d.atlas && !d.system && d.apps.is_empty() && !d.keep_ready);
        assert!(!d.needs_gateway());
        assert_eq!(Switches::parse(""), d);
    }

    #[test]
    fn nothing_changes_nothing_is_done() {
        let s = Switches {
            system: true,
            apps: apps(&["discord"]),
            ..Switches::default()
        };
        assert!(plan(&s, &s).is_empty());
        assert!(plan(&Switches::default(), &Switches::default()).is_empty());
    }

    #[test]
    fn system_on_loads_the_rules_before_tor_starts() {
        let to = Switches {
            system: true,
            ..Switches::default()
        };
        assert_eq!(
            plan(&Switches::default(), &to),
            [Action::LoadSystemRules, Action::StartGateway],
            "a redirect to a tor that is not up yet fails closed; the other order leaks while it boots"
        );
    }

    #[test]
    fn system_off_stops_tor_and_then_unloads_the_rules() {
        let from = Switches {
            system: true,
            ..Switches::default()
        };
        assert_eq!(
            plan(&from, &Switches::default()),
            [Action::StopGateway, Action::UnloadSystemRules]
        );
    }

    #[test]
    fn an_app_gets_the_bridge_then_its_namespace_and_loses_them_in_reverse() {
        let on = Switches {
            apps: apps(&["discord"]),
            ..Switches::default()
        };
        assert_eq!(
            plan(&Switches::default(), &on),
            [
                Action::GatewayUp,
                Action::StartGateway,
                Action::AddApp("discord".into())
            ],
            "tor listens on the bridge address, so the bridge comes first"
        );
        assert_eq!(
            plan(&on, &Switches::default()),
            [
                Action::RemoveApp("discord".into()),
                Action::StopGateway,
                Action::GatewayDown
            ],
            "tor stops before the address it listens on goes away"
        );
    }

    #[test]
    fn a_second_app_reuses_the_gateway() {
        let one = Switches {
            apps: apps(&["a"]),
            ..Switches::default()
        };
        let two = Switches {
            apps: apps(&["a", "b"]),
            ..Switches::default()
        };
        assert_eq!(plan(&one, &two), [Action::AddApp("b".into())]);
        assert_eq!(plan(&two, &one), [Action::RemoveApp("b".into())]);
    }

    #[test]
    fn system_on_while_apps_run_does_not_restart_the_gateway() {
        let from = Switches {
            apps: apps(&["a"]),
            ..Switches::default()
        };
        let to = Switches {
            apps: apps(&["a"]),
            system: true,
            ..Switches::default()
        };
        assert_eq!(
            plan(&from, &to),
            [Action::LoadSystemRules, Action::ReloadGateway],
            "tor must also listen on 127.0.0.1 now, and is not restarted"
        );
        assert_eq!(
            plan(&to, &from),
            [Action::ReloadGateway, Action::UnloadSystemRules],
            "the apps keep their gateway; tor stops listening before the rules go"
        );
    }

    #[test]
    fn atlas_and_keep_ready_are_not_the_daemons_business() {
        let from = Switches::default();
        let to = Switches {
            atlas: false,
            keep_ready: true,
            ..Switches::default()
        };
        assert!(
            plan(&from, &to).is_empty(),
            "Atlas tabs need no privilege; the cache is a user unit"
        );
    }

    #[test]
    fn coverage_follows_the_switches() {
        let s = Switches {
            apps: apps(&["discord"]),
            ..Switches::default()
        };
        assert!(s.covers("discord") && !s.covers("firefox"));
        let all = Switches {
            system: true,
            ..Switches::default()
        };
        assert!(all.covers("firefox"));
    }

    #[test]
    fn the_file_round_trips_and_junk_cannot_turn_a_scope_on() {
        let s = Switches {
            atlas: false,
            apps: apps(&["a", "b-1"]),
            system: true,
            lan: true,
            keep_ready: true,
        };
        assert_eq!(Switches::parse(&s.render()), s);
        let junk = Switches::parse(
            "system yes\nsystem\napp Bad Name 1\napp ../x 2\napp ok 0\napp ok 201\napp ok\nsystem ON\nfoo on\n",
        );
        assert_eq!(junk, Switches::default());
        let dup = Switches::parse("app a 1\napp b 1\n");
        assert_eq!(dup.apps.len(), 1, "two apps never share an address");
    }

    #[test]
    fn an_app_keeps_its_address_when_others_come_and_go() {
        let mut s = Switches::default();
        assert_eq!(s.add_app("a").unwrap(), 1);
        assert_eq!(s.add_app("b").unwrap(), 2);
        assert_eq!(s.add_app("a").unwrap(), 1, "adding again changes nothing");
        s.apps.remove("a");
        assert_eq!(s.add_app("c").unwrap(), 1, "lowest free index");
        assert_eq!(s.apps["b"], 2);
        assert!(s.add_app("Bad Name").is_err());
    }

    #[test]
    fn system_rules_and_namespaces_are_the_steps_of_their_actions() {
        let ctx = Ctx { tor_uid: 105 };
        let mut to = Switches {
            system: true,
            lan: true,
            ..Switches::default()
        };
        to.add_app("discord").unwrap();
        let from = Switches::default();
        let load = steps(&Action::LoadSystemRules, &from, &to, &ctx);
        let text = load[0].stdin.clone().unwrap();
        assert!(text.contains("meta skuid 105 return") && text.contains("allowed LAN"));
        let add = steps(&Action::AddApp("discord".into()), &from, &to, &ctx);
        assert!(
            add.iter()
                .any(|s| s.argv.contains(&"wolt-discord".to_string()))
        );
        let rm = steps(&Action::RemoveApp("discord".into()), &to, &from, &ctx);
        assert_eq!(
            rm.len(),
            2,
            "the app's index comes from the state it was added in"
        );
        assert_eq!(
            steps(&Action::StartGateway, &from, &to, &ctx)[0].argv,
            ["systemctl", "start", "wol-tor-gateway.service"]
        );
        assert_eq!(
            steps(&Action::ReloadGateway, &from, &to, &ctx)[0].argv[1],
            "restart"
        );
        let un = steps(&Action::UnloadSystemRules, &to, &from, &ctx);
        assert_eq!(un[0].argv, ["nft", "destroy", "table", "inet", "wol_tor"]);
        assert!(reload_rules_steps(&Switches::default(), &ctx).is_empty());
        assert_eq!(reload_rules_steps(&to, &ctx).len(), 2);
    }

    #[test]
    fn changing_only_the_lan_option_reloads_the_system_rules() {
        let on = Switches {
            system: true,
            ..Switches::default()
        };
        let lan = Switches {
            system: true,
            lan: true,
            ..Switches::default()
        };
        assert_eq!(plan(&on, &lan), [Action::LoadSystemRules]);
    }
}
