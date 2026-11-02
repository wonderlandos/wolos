// SPDX-License-Identifier: GPL-2.0-or-later
//! The service's view of the graph: events a backend reports, commands it carries out, and the
//! snapshot the UI draws. Pure data; the rules live in `service`.
use std::collections::BTreeMap;
use std::path::PathBuf;
use wol_core::audio::{Device, DeviceKind, InputMeter, ProAudio};

/// What the backend saw. Node ids are PipeWire global ids.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// An audio sink or source appeared (or its properties changed).
    Node(NodeInfo),
    /// The node's `Props` param: linear gains, mute, 0 dB position.
    Props {
        id: u32,
        channel_volumes: Vec<f32>,
        volume_base: f32,
        muted: bool,
    },
    Removed(u32),
    /// An active `Route` param of a device (`device.id`): the route `index` that currently
    /// serves the profile device `route_device` (a node's `card.profile.device`). The hardware
    /// mixer (Capture, Mic Boost, Master) sits behind it.
    Route {
        device: u32,
        index: i32,
        route_device: i32,
    },
    /// `default.audio.sink` / `default.audio.source` from the `default` metadata.
    Default {
        kind: DeviceKind,
        node_name: String,
    },
    /// One captured buffer's linear peak, `dt` seconds after the previous one.
    Peak {
        device: u32,
        peak: f32,
        dt: f32,
    },
    /// `clock.force-rate` / `clock.force-quantum` from the `settings` metadata (0 = not forced).
    Settings {
        rate: u32,
        quantum: u32,
    },
    /// Graph xruns since the service started.
    Xruns(u32),
}

/// Node properties the model keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeInfo {
    pub id: u32,
    /// `node.name`, what metadata and pactl take.
    pub node_name: String,
    /// `node.description`, else `node.nick`, else `node.name`.
    pub description: String,
    pub kind: DeviceKind,
    /// `device.id`: the card the node belongs to (Bluetooth codec changes go there).
    pub device_id: Option<u32>,
    /// `api.bluez5.address`.
    pub address: Option<String>,
    pub codec: Option<String>,
    pub profile: Option<String>,
    /// `card.profile.device`: which of the device's routes carries this node's volume.
    pub route_device: Option<i32>,
}

impl Default for NodeInfo {
    fn default() -> Self {
        Self {
            id: 0,
            node_name: String::new(),
            description: String::new(),
            kind: DeviceKind::Output,
            device_id: None,
            address: None,
            codec: None,
            profile: None,
            route_device: None,
        }
    }
}

/// What the service asks the backend to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Set every channel of the node to this linear gain.
    SetVolume {
        id: u32,
        linear: f32,
    },
    SetMute {
        id: u32,
        muted: bool,
    },
    /// Set the device's `Route` param (what wpctl and pipewire-pulse do for a node that has a
    /// route): the device moves its hardware mixer and pushes the rest back into the node as
    /// software volume. `None` leaves that property alone. Saved, like wpctl.
    SetRoute {
        device: u32,
        index: i32,
        route_device: i32,
        channel_volumes: Option<Vec<f32>>,
        mute: Option<bool>,
    },
    /// Write the `default` metadata key for this kind.
    SetDefault {
        kind: DeviceKind,
        node_name: String,
    },
    /// Run a program (pactl, pw-cli, pw-metadata, systemctl) and capture stdout.
    Run(Vec<String>),
    WriteFile(PathBuf, String),
    /// Remove a file or a whole directory; missing is fine.
    RemovePath(PathBuf),
    AppendLine(PathBuf, String),
}

/// One node as the model keeps it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub info: NodeInfo,
    pub channel_volumes: Vec<f32>,
    pub volume_base: f32,
    pub muted: bool,
    /// `Props` arrived at least once (volumes are meaningful).
    pub seen_props: bool,
}

impl Node {
    pub fn volume(&self) -> f32 {
        if self.channel_volumes.is_empty() {
            return 0.0;
        }
        self.channel_volumes.iter().sum::<f32>() / self.channel_volumes.len() as f32
    }
    pub fn channels(&self) -> usize {
        self.channel_volumes.len().max(1)
    }
}

/// The graph as the model knows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub nodes: BTreeMap<u32, Node>,
    /// Active routes: (device id, profile device) -> route index.
    pub routes: BTreeMap<(u32, i32), i32>,
    pub default_sink: Option<String>,
    pub default_source: Option<String>,
    pub meter: InputMeter,
    pub pro: ProAudio,
}

impl Model {
    /// Apply one event; true when something the UI shows changed.
    pub fn apply(&mut self, ev: &Event) -> bool {
        match ev {
            Event::Node(info) => {
                let n = self.nodes.entry(info.id).or_default();
                let changed = n.info != *info;
                n.info = info.clone();
                changed
            }
            Event::Props {
                id,
                channel_volumes,
                volume_base,
                muted,
            } => {
                let Some(n) = self.nodes.get_mut(id) else {
                    return false;
                };
                let before = (
                    n.channel_volumes.clone(),
                    n.volume_base,
                    n.muted,
                    n.seen_props,
                );
                n.channel_volumes.clone_from(channel_volumes);
                n.volume_base = if *volume_base > 0.0 {
                    *volume_base
                } else {
                    1.0
                };
                n.muted = *muted;
                n.seen_props = true;
                before != (n.channel_volumes.clone(), n.volume_base, n.muted, true)
            }
            Event::Removed(id) => {
                // Device and node ids share PipeWire's global id space.
                self.routes.retain(|k, _| k.0 != *id);
                self.nodes.remove(id).is_some()
            }
            Event::Route {
                device,
                index,
                route_device,
            } => {
                self.routes.insert((*device, *route_device), *index);
                false
            }
            Event::Default { kind, node_name } => {
                let slot = match kind {
                    DeviceKind::Output => &mut self.default_sink,
                    DeviceKind::Input => &mut self.default_source,
                };
                let v = (!node_name.is_empty()).then(|| node_name.clone());
                let changed = *slot != v;
                *slot = v;
                changed
            }
            Event::Peak { device, peak, dt } => {
                self.meter.device = *device;
                self.meter.feed(*peak, *dt);
                true
            }
            Event::Settings { rate, quantum } => {
                let changed = (self.pro.rate, self.pro.quantum) != (*rate, *quantum);
                self.pro.rate = *rate;
                self.pro.quantum = *quantum;
                changed
            }
            Event::Xruns(n) => {
                let changed = self.pro.xruns != *n;
                self.pro.xruns = *n;
                changed
            }
        }
    }

    /// The device route behind a node's volume: (device, route index, profile device), or None
    /// for nodes without one (virtual filters, nodes whose device has not reported its routes).
    pub fn route_of(&self, id: u32) -> Option<(u32, i32, i32)> {
        let info = &self.nodes.get(&id)?.info;
        let (dev, rd) = (info.device_id?, info.route_device?);
        let index = *self.routes.get(&(dev, rd))?;
        Some((dev, index, rd))
    }

    pub fn node_by_name(&self, node_name: &str) -> Option<&Node> {
        self.nodes.values().find(|n| n.info.node_name == node_name)
    }

    /// The default node of a kind, when known and present.
    pub fn default_node(&self, kind: DeviceKind) -> Option<&Node> {
        let name = match kind {
            DeviceKind::Output => self.default_sink.as_deref(),
            DeviceKind::Input => self.default_source.as_deref(),
        }?;
        self.node_by_name(name).filter(|n| n.info.kind == kind)
    }

    pub fn is_default(&self, n: &Node) -> bool {
        let d = match n.info.kind {
            DeviceKind::Output => &self.default_sink,
            DeviceKind::Input => &self.default_source,
        };
        d.as_deref() == Some(n.info.node_name.as_str())
    }

    /// The frozen `Device` for a node.
    pub fn device(&self, n: &Node) -> Device {
        Device {
            id: n.info.id,
            name: n.info.description.clone(),
            kind: n.info.kind,
            default: self.is_default(n),
            volume: n.volume(),
            volume_base: n.volume_base,
            muted: n.muted,
            codec: n.info.codec.clone(),
            profile: n.info.profile.clone(),
        }
    }
}

/// What the UI draws: devices (outputs first, then inputs, each by name), the input meter, the
/// Pro Audio state and the two input switches.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct Snapshot {
    pub devices: Vec<Device>,
    pub meter: InputMeter,
    pub pro: ProAudio,
    pub auto_input_level: bool,
    pub noise_suppression: bool,
    /// "Allow louder": sliders may pass 0 dB.
    pub boost: bool,
    /// The PipeWire connection is up (false while PipeWire restarts; the list is then empty).
    pub connected: bool,
    /// The last action's problem, in plain words, for the page's notice; None when it worked.
    pub error: Option<String>,
}

impl Snapshot {
    pub fn default_input(&self) -> Option<&Device> {
        self.devices
            .iter()
            .find(|d| d.kind == DeviceKind::Input && d.default)
    }
    pub fn default_output(&self) -> Option<&Device> {
        self.devices
            .iter()
            .find(|d| d.kind == DeviceKind::Output && d.default)
    }
}
