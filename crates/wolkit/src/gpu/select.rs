// SPDX-License-Identifier: BSD-2-Clause
//! Which Vulkan device draws, and which devices never do. Pure: device facts in, choice out.
//!
//! The rule is the compositor's (`crates/realtime/src/gpu_select.rs`, restated here because
//! that crate is GPL and this one is BSD): draw on the GPU that drives the display, so a frame
//! never crosses between GPUs. The compositor tells a Wayland client its main device through
//! linux-dmabuf feedback; when that is known it wins. Without it, prefer integrated, then
//! discrete, then virtual. A software device (lavapipe) is used only when nothing else exists
//! or when it is asked for (CI goldens). `WOL_GPU=<render node>` overrides everything, as for
//! the compositor. A device on the blocklist is never chosen.

/// What the renderer knows about one physical device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceFacts {
    /// Index in the instance's enumeration order.
    pub index: usize,
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    pub driver_version: u32,
    pub kind: DeviceKind,
    /// `(major, minor)` of the render node (VK_EXT_physical_device_drm), when the driver says.
    pub render_node: Option<(u32, u32)>,
    /// `(major, minor)` of the primary (KMS) node, when the driver says.
    pub primary_node: Option<(u32, u32)>,
    /// The device supports what the uber-pipeline needs (graphics queue, formats, limits).
    pub suitable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeviceKind {
    Software,
    Other,
    Virtual,
    Discrete,
    Integrated,
}

/// What the caller adds to the device facts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preference {
    /// `WOL_GPU`, e.g. `/dev/dri/renderD129`.
    pub override_node: Option<String>,
    /// The compositor's main device (`dev_t` major, minor) from dmabuf feedback.
    pub display_device: Option<(u32, u32)>,
    /// Accept a software device even when a hardware one exists (CI goldens).
    pub prefer_software: bool,
}

/// The render node path for a `(major, minor)` pair as the kernel names it.
pub fn node_path(minor: u32) -> String {
    format!("/dev/dri/renderD{minor}")
}

/// Picks the device to draw on, or `None` when no device may draw.
pub fn choose<'a>(
    devices: &'a [DeviceFacts],
    pref: &Preference,
    blocklist: &Blocklist,
) -> Option<&'a DeviceFacts> {
    let usable = |d: &&DeviceFacts| d.suitable && !blocklist.blocks(d);
    if let Some(node) = pref.override_node.as_deref() {
        return devices
            .iter()
            .filter(usable)
            .find(|d| d.render_node.is_some_and(|(_, m)| node_path(m) == node));
    }
    if pref.prefer_software
        && let Some(d) = devices
            .iter()
            .filter(usable)
            .find(|d| d.kind == DeviceKind::Software)
    {
        return Some(d);
    }
    devices.iter().filter(usable).max_by_key(|d| {
        let drives = pref
            .display_device
            .is_some_and(|dev| d.render_node == Some(dev) || d.primary_node == Some(dev));
        (
            drives,
            d.kind != DeviceKind::Software,
            d.kind,
            std::cmp::Reverse(d.index),
        )
    })
}

/// One blocklist rule: a vendor, optionally a device id and a driver version ceiling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub vendor: u32,
    pub device: Option<u32>,
    /// Blocks only drivers whose packed `driverVersion` is below this.
    pub below: Option<u32>,
    pub reason: String,
}

/// Devices that must not draw. Text format, one rule per line, `#` starts a comment:
/// `<vendor hex>:<device hex | *> [below <driverVersion hex>] [# reason]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blocklist {
    pub rules: Vec<Rule>,
}

/// Rules that ship with the engine. Empty today: every driver the image ships passes the GPU
/// goldens. A rule lands here with the reason and the golden or bug that proved it.
pub const BUILTIN: &str = "# vendor:device [below driverVersion] # reason\n";

/// Longest blocklist file the renderer reads.
pub const MAX_BLOCKLIST_BYTES: usize = 64 * 1024;

fn hex(s: &str) -> Option<u32> {
    let s = s.trim();
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u32::from_str_radix(s, 16).ok()
}

impl Blocklist {
    /// Parses rules; a malformed line is reported with its number and skipped, never fatal.
    pub fn parse(text: &str) -> (Blocklist, Vec<usize>) {
        let mut out = Blocklist::default();
        let mut bad = Vec::new();
        let text = text
            .get(..MAX_BLOCKLIST_BYTES.min(text.len()))
            .unwrap_or("");
        for (n, line) in text.lines().enumerate() {
            let (body, reason) = match line.split_once('#') {
                Some((b, r)) => (b.trim(), r.trim()),
                None => (line.trim(), ""),
            };
            if body.is_empty() {
                continue;
            }
            let mut words = body.split_whitespace();
            let ids = words.next().unwrap_or("");
            let parsed = ids.split_once(':').and_then(|(v, d)| {
                let vendor = hex(v)?;
                let device = if d.trim() == "*" { None } else { Some(hex(d)?) };
                Some((vendor, device))
            });
            let below = match (words.next(), words.next()) {
                (None, _) => Some(None),
                (Some("below"), Some(v)) => hex(v).map(Some),
                _ => None,
            };
            match (parsed, below, words.next()) {
                (Some((vendor, device)), Some(below), None) => out.rules.push(Rule {
                    vendor,
                    device,
                    below,
                    reason: reason.to_string(),
                }),
                _ => bad.push(n + 1),
            }
        }
        (out, bad)
    }

    /// The built-in rules plus the rules of `extra` (a file the package or the admin ships).
    pub fn with_extra(extra: Option<&str>) -> Blocklist {
        let (mut list, _) = Blocklist::parse(BUILTIN);
        if let Some(t) = extra {
            list.rules.extend(Blocklist::parse(t).0.rules);
        }
        list
    }

    /// The rule that blocks `d`, if any.
    pub fn rule_for(&self, d: &DeviceFacts) -> Option<&Rule> {
        self.rules.iter().find(|r| {
            r.vendor == d.vendor
                && r.device.is_none_or(|id| id == d.device)
                && r.below.is_none_or(|v| d.driver_version < v)
        })
    }

    pub fn blocks(&self, d: &DeviceFacts) -> bool {
        self.rule_for(d).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(index: usize, kind: DeviceKind, node: u32) -> DeviceFacts {
        DeviceFacts {
            index,
            name: format!("gpu{index}"),
            vendor: 0x8086,
            device: 0x1000 + u32::try_from(index).unwrap_or(0),
            driver_version: 100,
            kind,
            render_node: Some((226, node)),
            primary_node: Some((226, node - 128)),
            suitable: true,
        }
    }

    #[test]
    fn integrated_wins_over_discrete_without_display_facts() {
        let d = [
            dev(0, DeviceKind::Discrete, 129),
            dev(1, DeviceKind::Integrated, 128),
        ];
        let got = choose(&d, &Preference::default(), &Blocklist::default());
        assert_eq!(got.map(|g| g.index), Some(1));
    }

    #[test]
    fn the_device_driving_the_display_wins() {
        let d = [
            dev(0, DeviceKind::Integrated, 128),
            dev(1, DeviceKind::Discrete, 129),
        ];
        let pref = Preference {
            display_device: Some((226, 1)),
            ..Preference::default()
        };
        assert_eq!(
            choose(&d, &pref, &Blocklist::default()).map(|g| g.index),
            Some(1)
        );
    }

    #[test]
    fn software_is_last_unless_asked_for() {
        let d = [
            dev(0, DeviceKind::Software, 130),
            dev(1, DeviceKind::Virtual, 128),
        ];
        let none = Blocklist::default();
        assert_eq!(
            choose(&d, &Preference::default(), &none).map(|g| g.index),
            Some(1)
        );
        let ci = Preference {
            prefer_software: true,
            ..Preference::default()
        };
        assert_eq!(choose(&d, &ci, &none).map(|g| g.index), Some(0));
        let only = [dev(0, DeviceKind::Software, 130)];
        assert_eq!(
            choose(&only, &Preference::default(), &none).map(|g| g.index),
            Some(0)
        );
    }

    #[test]
    fn override_node_picks_that_device_or_nothing() {
        let d = [
            dev(0, DeviceKind::Integrated, 128),
            dev(1, DeviceKind::Discrete, 129),
        ];
        let pref = Preference {
            override_node: Some("/dev/dri/renderD129".into()),
            ..Preference::default()
        };
        let none = Blocklist::default();
        assert_eq!(choose(&d, &pref, &none).map(|g| g.index), Some(1));
        let missing = Preference {
            override_node: Some("/dev/dri/renderD140".into()),
            ..Preference::default()
        };
        assert!(choose(&d, &missing, &none).is_none());
    }

    #[test]
    fn blocklisted_and_unsuitable_devices_are_skipped() {
        let mut bad = dev(0, DeviceKind::Integrated, 128);
        bad.suitable = false;
        let d = [bad, dev(1, DeviceKind::Discrete, 129)];
        let none = Blocklist::default();
        assert_eq!(
            choose(&d, &Preference::default(), &none).map(|g| g.index),
            Some(1)
        );
        let (list, errs) = Blocklist::parse("8086:1001 # hangs on resize\n");
        assert!(errs.is_empty());
        assert!(choose(&d, &Preference::default(), &list).is_none());
    }

    #[test]
    fn blocklist_parses_wildcards_versions_and_reports_bad_lines() {
        let text =
            "# comment\n0x1002:* below 0x5000 # old firmware\n10de:2204\nnonsense\n8086:zz\n";
        let (list, bad) = Blocklist::parse(text);
        assert_eq!(list.rules.len(), 2);
        assert_eq!(bad, vec![4, 5]);
        let mut amd = dev(0, DeviceKind::Discrete, 128);
        amd.vendor = 0x1002;
        amd.driver_version = 0x4fff;
        assert_eq!(
            list.rule_for(&amd).map(|r| r.reason.as_str()),
            Some("old firmware")
        );
        amd.driver_version = 0x5000;
        assert!(!list.blocks(&amd));
        let builtin = Blocklist::with_extra(None);
        assert!(builtin.rules.is_empty());
    }
}
