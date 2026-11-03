// SPDX-License-Identifier: GPL-2.0-or-later
//! The real backend on pipewire-rs 0.10 (feature `pw`). Two threads:
//! - `wol-audio-pw`: one PipeWire main loop. The registry gives sinks, sources, their devices'
//!   active `Route`s and the `default`/`settings` metadata; node `Props` params carry volumes,
//!   mute and `volumeBase`; the input meter is a capture stream that exists only while a page
//!   shows it. It owns every proxy and never runs a program. When the connection breaks (a core
//!   error on id 0: PipeWire restarted, for example by Reset audio) it drops the proxies, tells
//!   the service, and reconnects with [`Backoff`] (1, 2, 4, then every 8 s).
//! - `wol-audio`: the [`Service`] in [`serve`]. Programs (pactl, pw-cli, pw-metadata, pw-top,
//!   systemctl) and files run here with `std::process`/`std::fs`, so a slow one never stalls the
//!   loop; proxy calls (volume, mute, route, default) are posted to the loop.
//!
//! The UI talks to [`Handle`]: requests go in through a `std::sync::mpsc` channel, every change
//! comes back as a [`Snapshot`] on a receiver (drain it from a GLib idle).
use crate::model::{Command, Event, METER_PROPS, NodeInfo, Snapshot};
use crate::service::{Backend, Backoff, Ended, Input, MAX_RESTARTS, Service, serve_guarded};
use pipewire as pw;
use pw::device::{Device, DeviceListener};
use pw::metadata::{Metadata, MetadataListener};
use pw::node::{Node, NodeListener};
use pw::spa;
use pw::spa::param::ParamType;
use pw::spa::pod::deserialize::PodDeserializer;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Object, Pod, Property, Value, ValueArray};
use pw::spa::utils::SpaTypes;
use pw::stream::{StreamFlags, StreamListener, StreamRc};
use pw::types::ObjectType;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use wol_core::audio::{Action, DeviceKind};

pub use crate::service::Request;

/// What the service thread asks of the PipeWire thread.
enum LoopCmd {
    /// A proxy call: SetVolume, SetMute, SetRoute or SetDefault.
    Exec(Command),
    Meter(bool),
    Quit,
}

/// The service seen from the UI.
pub struct Handle {
    tx: mpsc::Sender<Input>,
    pub rx: mpsc::Receiver<Snapshot>,
    _thread: Option<JoinHandle<()>>,
}

impl Handle {
    /// Start the service and PipeWire threads for the user whose files live in `home`.
    pub fn spawn(home: PathBuf) -> Self {
        let (tx, rx_in) = mpsc::channel::<Input>();
        let (tx_snap, rx) = mpsc::channel::<Snapshot>();
        let events = tx.clone();
        let thread = std::thread::Builder::new()
            .name("wol-audio".into())
            .spawn(move || {
                // R2C-14: a panic in the service is logged and answered with a fresh service on
                // a fresh PipeWire connection (a few times); the old PipeWire thread is always
                // told to stop and joined first, so it never keeps a connection nobody reads.
                let mut restarts = 0;
                loop {
                    let (to_loop, from_service) = pw::channel::channel::<LoopCmd>();
                    let ev = events.clone();
                    let pw_thread = std::thread::Builder::new()
                        .name("wol-audio-pw".into())
                        .spawn(move || {
                            if let Err(e) = pw_loop(from_service, &ev) {
                                eprintln!("wol-audio: {e}");
                                let _ = ev.send(Input::Connected(false));
                            }
                        })
                        .ok();
                    let mut service = Service::new(
                        PwBackend {
                            to_loop: to_loop.clone(),
                        },
                        home.clone(),
                    );
                    let meter = to_loop.clone();
                    // 2 s: often enough to see a glitch while tuning a buffer; pw-top costs ~100 ms.
                    let ended = serve_guarded(
                        &mut service,
                        &rx_in,
                        &tx_snap,
                        Duration::from_secs(2),
                        &mut |on| {
                            let _ = meter.send(LoopCmd::Meter(on));
                        },
                    );
                    let _ = to_loop.send(LoopCmd::Quit);
                    if let Some(t) = pw_thread {
                        let _ = t.join();
                    }
                    if ended == Ended::Quit || restarts >= MAX_RESTARTS {
                        break;
                    }
                    restarts += 1;
                    std::thread::sleep(Duration::from_secs(1 << (restarts - 1)));
                }
            })
            .ok();
        Self {
            tx,
            rx,
            _thread: thread,
        }
    }

    pub fn request(&self, r: Request) {
        let _ = self.tx.send(Input::Ui(r));
    }
    pub fn act(&self, a: Action) {
        self.request(Request::Act(a));
    }
    /// The newest snapshot waiting, if any (older ones are dropped).
    pub fn latest(&self) -> Option<Snapshot> {
        let mut last = None;
        while let Ok(s) = self.rx.try_recv() {
            last = Some(s);
        }
        last
    }
}

impl Drop for Handle {
    /// Ask the threads to stop and do not wait: the service thread may be inside a program
    /// call for up to 10 s, and the UI thread that drops the Handle must not block on it.
    fn drop(&mut self) {
        let _ = self.tx.send(Input::Ui(Request::Quit));
    }
}

/// Live proxies of one connection (the registry listener fills them).
#[derive(Default)]
struct Proxies {
    nodes: HashMap<u32, (Node, NodeListener)>,
    /// `Audio/Device` objects: their `Route` param moves the hardware mixer.
    devices: HashMap<u32, (Device, DeviceListener)>,
    /// The `default` metadata: WirePlumber reads `default.configured.audio.*` from it.
    default_md: Option<(Metadata, MetadataListener)>,
    /// Kept bound so `clock.force-*` changes keep arriving; the service writes them via pw-metadata.
    _settings_md: Option<(Metadata, MetadataListener)>,
}

/// The service thread's backend: proxy calls go to the loop, programs and files run here.
pub struct PwBackend {
    to_loop: pw::channel::Sender<LoopCmd>,
}

/// Mode of a PipeWire or WirePlumber fragment: the person's own config, readable by its daemons.
const FRAGMENT_MODE: u32 = 0o644;

/// Replace a config fragment atomically (audit S4-10): a PipeWire or WirePlumber restart at that
/// moment reads the old file or the new one, never a truncated one (no sound).
fn write_fragment(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    shared::atomic::atomic_write(path, FRAGMENT_MODE, data)
}

impl Backend for PwBackend {
    fn execute(&mut self, cmd: &Command) -> Result<String, String> {
        match cmd {
            Command::SetVolume { .. }
            | Command::SetMute { .. }
            | Command::SetRoute { .. }
            | Command::SetDefault { .. } => self
                .to_loop
                .send(LoopCmd::Exec(cmd.clone()))
                .map(|()| String::new())
                .map_err(|_| "the sound connection is closed".to_string()),
            Command::Run(argv) => run_program(argv),
            Command::WriteFile(p, text) => {
                if let Some(dir) = p.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                write_fragment(p, text.as_bytes()).map_err(|e| e.to_string())?;
                Ok(String::new())
            }
            Command::RemovePath(p) => {
                let r = if p.is_dir() {
                    std::fs::remove_dir_all(p)
                } else {
                    std::fs::remove_file(p)
                };
                match r {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                    _ => Ok(String::new()),
                }
            }
            Command::AppendLine(p, line) => {
                use std::io::Write;
                if let Some(dir) = p.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                let mut f = std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(p)
                    .map_err(|e| e.to_string())?;
                writeln!(f, "{line}").map_err(|e| e.to_string())?;
                Ok(String::new())
            }
        }
    }
    fn read_file(&self, p: &Path) -> Option<String> {
        std::fs::read_to_string(p).ok()
    }
}

/// Run a program with a 10 s limit; stdout on success, else the first stderr line (or the exit
/// status) as the reason.
fn run_program(argv: &[String]) -> Result<String, String> {
    let (prog, args) = argv.split_first().ok_or("nothing to run")?;
    let mut child = std::process::Command::new(prog)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{prog}: {e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > Duration::from_secs(10) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{prog} did not answer within 10 s"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("{prog}: {e}")),
        }
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("{prog}: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(err
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map_or_else(
            || format!("{prog} exited with {}", out.status),
            str::to_owned,
        ))
}

/// A SPA object as pod bytes.
fn pod_bytes(obj: Object) -> Option<Vec<u8>> {
    PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &Value::Object(obj))
        .ok()
        .map(|(c, _)| c.into_inner())
}

/// `node.set_param(Props, { ...props })`.
fn set_props(node: &Node, props: Vec<Property>) {
    let Some(bytes) = pod_bytes(Object {
        type_: SpaTypes::ObjectParamProps.as_raw(),
        id: ParamType::Props.as_raw(),
        properties: props,
    }) else {
        return;
    };
    if let Some(pod) = Pod::from_bytes(&bytes) {
        node.set_param(ParamType::Props, 0, pod);
    }
}

/// `device.set_param(Route, { index, device, props: { ... }, save: true })`, as WirePlumber's
/// mixer-api (wpctl) and pipewire-pulse do it.
fn set_route(
    device: &Device,
    index: i32,
    route_device: i32,
    channel_volumes: Option<&[f32]>,
    mute: Option<bool>,
) {
    let mut props = Vec::new();
    if let Some(v) = channel_volumes {
        props.push(Property::new(
            spa::sys::SPA_PROP_channelVolumes,
            Value::ValueArray(ValueArray::Float(v.to_vec())),
        ));
    }
    if let Some(m) = mute {
        props.push(Property::new(spa::sys::SPA_PROP_mute, Value::Bool(m)));
    }
    let Some(bytes) = pod_bytes(Object {
        type_: SpaTypes::ObjectParamRoute.as_raw(),
        id: ParamType::Route.as_raw(),
        properties: vec![
            Property::new(spa::sys::SPA_PARAM_ROUTE_index, Value::Int(index)),
            Property::new(spa::sys::SPA_PARAM_ROUTE_device, Value::Int(route_device)),
            Property::new(
                spa::sys::SPA_PARAM_ROUTE_props,
                Value::Object(Object {
                    type_: SpaTypes::ObjectParamProps.as_raw(),
                    id: ParamType::Route.as_raw(),
                    properties: props,
                }),
            ),
            Property::new(spa::sys::SPA_PARAM_ROUTE_save, Value::Bool(true)),
        ],
    }) else {
        return;
    };
    if let Some(pod) = Pod::from_bytes(&bytes) {
        device.set_param(ParamType::Route, 0, pod);
    }
}

/// Carry out one proxy command on the loop thread; an object that is gone is skipped (its
/// removal is already on its way to the service).
fn exec(p: &Proxies, cmd: &Command) {
    match cmd {
        Command::SetVolume { id, linear } => {
            if let Some((node, _)) = p.nodes.get(id) {
                // Two is right for every stereo node and PipeWire spreads a shorter array over
                // the rest.
                set_props(
                    node,
                    vec![Property::new(
                        spa::sys::SPA_PROP_channelVolumes,
                        Value::ValueArray(ValueArray::Float(vec![*linear; 2])),
                    )],
                );
            }
        }
        Command::SetMute { id, muted } => {
            if let Some((node, _)) = p.nodes.get(id) {
                set_props(
                    node,
                    vec![Property::new(spa::sys::SPA_PROP_mute, Value::Bool(*muted))],
                );
            }
        }
        Command::SetRoute {
            device,
            index,
            route_device,
            channel_volumes,
            mute,
        } => {
            if let Some((dev, _)) = p.devices.get(device) {
                set_route(
                    dev,
                    *index,
                    *route_device,
                    channel_volumes.as_deref(),
                    *mute,
                );
            }
        }
        Command::SetDefault { kind, node_name } => {
            if let Some((md, _)) = p.default_md.as_ref() {
                let key = match kind {
                    DeviceKind::Output => "default.configured.audio.sink",
                    DeviceKind::Input => "default.configured.audio.source",
                };
                let value = serde_json::json!({ "name": node_name }).to_string();
                md.set_property(0, key, Some("Spa:String:JSON"), Some(&value));
            }
        }
        _ => {}
    }
}

/// A node's `Props` param as an event: linear channel gains, 0 dB position, mute.
fn props_event(id: u32, pod: &Pod) -> Option<Event> {
    let (_, value) = PodDeserializer::deserialize_from::<Value>(pod.as_bytes()).ok()?;
    let Value::Object(obj) = value else {
        return None;
    };
    let mut channel_volumes = Vec::new();
    let mut single = None;
    let mut volume_base = 1.0;
    let mut muted = false;
    let mut seen = false;
    for p in obj.properties {
        match (p.key, p.value) {
            (k, Value::ValueArray(ValueArray::Float(a)))
                if k == spa::sys::SPA_PROP_channelVolumes =>
            {
                channel_volumes = a;
                seen = true;
            }
            (k, Value::Float(f)) if k == spa::sys::SPA_PROP_volume => {
                single = Some(f);
                seen = true;
            }
            (k, Value::Float(f)) if k == spa::sys::SPA_PROP_volumeBase => volume_base = f,
            (k, Value::Bool(b)) if k == spa::sys::SPA_PROP_mute => muted = b,
            _ => {}
        }
    }
    if channel_volumes.is_empty() {
        channel_volumes.extend(single);
    }
    seen.then_some(Event::Props {
        id,
        channel_volumes,
        volume_base,
        muted,
    })
}

/// One active `Route` param of a device as an event; None without index or profile device.
fn route_event(device: u32, pod: &Pod) -> Option<Event> {
    let Ok((_, Value::Object(obj))) = PodDeserializer::deserialize_from::<Value>(pod.as_bytes())
    else {
        return None;
    };
    let (mut index, mut route_device) = (None, None);
    for p in obj.properties {
        match (p.key, p.value) {
            (k, Value::Int(i)) if k == spa::sys::SPA_PARAM_ROUTE_index => index = Some(i),
            (k, Value::Int(d)) if k == spa::sys::SPA_PARAM_ROUTE_device => route_device = Some(d),
            _ => {}
        }
    }
    Some(Event::Route {
        device,
        index: index?,
        route_device: route_device?,
    })
}

/// A sink or source from its properties; None for streams, filters' internals and video.
fn node_info(id: u32, props: &spa::utils::dict::DictRef) -> Option<NodeInfo> {
    let kind = match props.get("media.class")? {
        "Audio/Sink" => DeviceKind::Output,
        "Audio/Source" => DeviceKind::Input,
        _ => return None,
    };
    let node_name = props.get("node.name")?.to_owned();
    let description = props
        .get("node.description")
        .or_else(|| props.get("node.nick"))
        .unwrap_or(&node_name)
        .to_owned();
    Some(NodeInfo {
        id,
        node_name,
        description,
        kind,
        device_id: props.get("device.id").and_then(|d| d.parse().ok()),
        address: props.get("api.bluez5.address").map(str::to_owned),
        codec: props.get("api.bluez5.codec").map(str::to_owned),
        profile: props.get("device.profile.name").map(str::to_owned),
        route_device: props
            .get("card.profile.device")
            .and_then(|d| d.parse().ok()),
    })
}

/// `{ "name": "alsa_output.x" }` from the metadata value.
fn metadata_name(value: Option<&str>) -> String {
    value
        .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
        .and_then(|v| v["name"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

struct MeterData {
    last: Instant,
}

/// The capture stream behind the input level; lives only while a page shows the meter.
struct Meter {
    _stream: StreamRc,
    _listener: StreamListener<MeterData>,
}

type Events = mpsc::Sender<Input>;

fn send(tx: &Events, ev: Event) {
    let _ = tx.send(Input::Event(ev));
}

fn start_meter(core: &pw::core::CoreRc, tx: &Events) -> Option<Meter> {
    let mut props = pw::properties::PropertiesBox::new();
    for (k, v) in METER_PROPS {
        props.insert(k, v);
    }
    let stream = StreamRc::new(core.clone(), "wolos-meter", props).ok()?;
    let tx = tx.clone();
    let listener = stream
        .add_local_listener_with_user_data(MeterData {
            last: Instant::now(),
        })
        .process(move |stream, data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            let Some(d) = datas.first_mut() else { return };
            let n = (d.chunk().size() / 4) as usize;
            let Some(bytes) = d.data() else { return };
            let mut peak = 0f32;
            for s in bytes.as_chunks::<4>().0.iter().take(n) {
                let v = f32::from_le_bytes(*s).abs();
                peak = peak.max(v);
            }
            let now = Instant::now();
            let dt = now.duration_since(data.last).as_secs_f32();
            data.last = now;
            // The service names the device (the default input it knows).
            send(
                &tx,
                Event::Peak {
                    device: 0,
                    peak,
                    dt,
                },
            );
        })
        .register()
        .ok()?;
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    let bytes = pod_bytes(Object {
        type_: SpaTypes::ObjectParamFormat.as_raw(),
        id: ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    })?;
    let mut params = [Pod::from_bytes(&bytes)?];
    stream
        .connect(
            spa::utils::Direction::Input,
            None,
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .ok()?;
    Some(Meter {
        _stream: stream,
        _listener: listener,
    })
}

/// One PipeWire connection. Field order is drop order: the meter, listeners and proxies go
/// before the registry and the core they belong to.
struct Conn {
    meter: Option<Meter>,
    _registry_listener: pw::registry::Listener,
    _core_listener: pw::core::Listener,
    proxies: Rc<RefCell<Proxies>>,
    _registry: pw::registry::RegistryRc,
    core: pw::core::CoreRc,
}

/// Connect, and mirror the registry into the service as events. `broken` is set when the core
/// reports the connection gone.
fn connect(
    context: &pw::context::ContextRc,
    tx: &Events,
    broken: &Rc<Cell<bool>>,
) -> Result<Conn, pw::Error> {
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;
    let proxies: Rc<RefCell<Proxies>> = Rc::default();
    let settings: Rc<Cell<(u32, u32)>> = Rc::default();

    let core_listener = {
        let broken = broken.clone();
        core.add_listener_local()
            .error(move |id, _seq, _res, message| {
                eprintln!("wol-audio: pipewire error on {id}: {message}");
                if id == 0 {
                    broken.set(true);
                }
            })
            .register()
    };

    let registry_listener = {
        let registry_weak = registry.downgrade();
        let proxies_add = proxies.clone();
        let proxies_rm = proxies.clone();
        let tx = tx.clone();
        let tx_rm = tx.clone();
        registry
            .add_listener_local()
            .global(move |obj| {
                let Some(registry) = registry_weak.upgrade() else {
                    return;
                };
                let Some(props) = obj.props else { return };
                let id = obj.id;
                match obj.type_ {
                    ObjectType::Node => {
                        let Some(info) = node_info(id, props) else {
                            return;
                        };
                        let Ok(node) = registry.bind::<Node, _>(obj) else {
                            return;
                        };
                        let (t1, t2) = (tx.clone(), tx.clone());
                        let listener = node
                            .add_listener_local()
                            .info(move |i| {
                                if let Some(info) = i.props().and_then(|p| node_info(id, p)) {
                                    send(&t1, Event::Node(info));
                                }
                            })
                            .param(move |_seq, ty, _index, _next, param| {
                                if ty != ParamType::Props {
                                    return;
                                }
                                if let Some(ev) = param.and_then(|p| props_event(id, p)) {
                                    send(&t2, ev);
                                }
                            })
                            .register();
                        node.subscribe_params(&[ParamType::Props]);
                        node.enum_params(0, Some(ParamType::Props), 0, u32::MAX);
                        proxies_add.borrow_mut().nodes.insert(id, (node, listener));
                        send(&tx, Event::Node(info));
                    }
                    ObjectType::Device => {
                        if props.get("media.class") != Some("Audio/Device") {
                            return;
                        }
                        let Ok(device) = registry.bind::<Device, _>(obj) else {
                            return;
                        };
                        let t = tx.clone();
                        let listener = device
                            .add_listener_local()
                            .param(move |_seq, ty, _index, _next, param| {
                                if ty != ParamType::Route {
                                    return;
                                }
                                if let Some(ev) = param.and_then(|p| route_event(id, p)) {
                                    send(&t, ev);
                                }
                            })
                            .register();
                        device.subscribe_params(&[ParamType::Route]);
                        device.enum_params(0, Some(ParamType::Route), 0, u32::MAX);
                        proxies_add
                            .borrow_mut()
                            .devices
                            .insert(id, (device, listener));
                    }
                    ObjectType::Metadata => {
                        let name = props.get("metadata.name").unwrap_or("");
                        if name != "default" && name != "settings" {
                            return;
                        }
                        let Ok(md) = registry.bind::<Metadata, _>(obj) else {
                            return;
                        };
                        let t = tx.clone();
                        let settings = settings.clone();
                        let is_default = name == "default";
                        let listener = md
                            .add_listener_local()
                            .property(move |subject, key, _type, value| {
                                if subject != 0 {
                                    return 0;
                                }
                                let v = value.unwrap_or("");
                                let (rate, quantum) = settings.get();
                                match (is_default, key) {
                                    (true, Some("default.audio.sink")) => send(
                                        &t,
                                        Event::Default {
                                            kind: DeviceKind::Output,
                                            node_name: metadata_name(value),
                                        },
                                    ),
                                    (true, Some("default.audio.source")) => send(
                                        &t,
                                        Event::Default {
                                            kind: DeviceKind::Input,
                                            node_name: metadata_name(value),
                                        },
                                    ),
                                    (false, Some("clock.force-rate")) => {
                                        let rate = v.parse().unwrap_or(0);
                                        settings.set((rate, quantum));
                                        send(&t, Event::Settings { rate, quantum });
                                    }
                                    (false, Some("clock.force-quantum")) => {
                                        let quantum = v.parse().unwrap_or(0);
                                        settings.set((rate, quantum));
                                        send(&t, Event::Settings { rate, quantum });
                                    }
                                    _ => {}
                                }
                                0
                            })
                            .register();
                        let mut p = proxies_add.borrow_mut();
                        if is_default {
                            p.default_md = Some((md, listener));
                        } else {
                            p._settings_md = Some((md, listener));
                        }
                    }
                    _ => {}
                }
            })
            .global_remove(move |id| {
                let mut p = proxies_rm.borrow_mut();
                let node = p.nodes.remove(&id).is_some();
                let device = p.devices.remove(&id).is_some();
                if node || device {
                    send(&tx_rm, Event::Removed(id));
                }
            })
            .register()
    };

    Ok(Conn {
        meter: None,
        _registry_listener: registry_listener,
        _core_listener: core_listener,
        proxies,
        _registry: registry,
        core,
    })
}

/// The PipeWire thread: connect, serve proxy commands, reconnect after a drop, until Quit.
fn pw_loop(rx: pw::channel::Receiver<LoopCmd>, tx: &Events) -> Result<(), pw::Error> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let conn: Rc<RefCell<Option<Conn>>> = Rc::default();
    let broken: Rc<Cell<bool>> = Rc::default();
    let meter_on: Rc<Cell<bool>> = Rc::default();
    let backoff: Rc<RefCell<Backoff>> = Rc::default();

    // Try to connect when the backoff allows it; true when connected.
    let try_connect = {
        let (conn, broken, meter_on, backoff, tx) = (
            conn.clone(),
            broken.clone(),
            meter_on.clone(),
            backoff.clone(),
            tx.clone(),
        );
        Rc::new(move || {
            let now = Instant::now();
            if !backoff.borrow().due(now) {
                return false;
            }
            match connect(&context, &tx, &broken) {
                Ok(mut c) => {
                    if meter_on.get() {
                        c.meter = start_meter(&c.core, &tx);
                    }
                    *conn.borrow_mut() = Some(c);
                    backoff.borrow_mut().reset();
                    let _ = tx.send(Input::Connected(true));
                    true
                }
                Err(e) => {
                    let wait = backoff.borrow_mut().failed(now);
                    eprintln!("wol-audio: no PipeWire ({e}); trying again in {wait:?}");
                    false
                }
            }
        })
    };
    if !try_connect() {
        let _ = tx.send(Input::Connected(false));
    }

    // 500 ms: a restarted PipeWire is back within a second; the backoff spaces real retries.
    let tick = {
        let (conn, broken, backoff, tx) =
            (conn.clone(), broken.clone(), backoff.clone(), tx.clone());
        let try_connect = try_connect.clone();
        mainloop.loop_().add_timer(move |_| {
            if broken.replace(false) {
                // Dropped here, not inside the core's own error callback.
                conn.borrow_mut().take();
                backoff.borrow_mut().failed(Instant::now());
                let _ = tx.send(Input::Connected(false));
            }
            if conn.borrow().is_none() {
                try_connect();
            }
        })
    };
    let _ = tick.update_timer(
        Some(Duration::from_millis(500)),
        Some(Duration::from_millis(500)),
    );

    let _commands = {
        let weak = mainloop.downgrade();
        let (conn, meter_on, tx) = (conn.clone(), meter_on.clone(), tx.clone());
        rx.attach(mainloop.loop_(), move |cmd| match cmd {
            LoopCmd::Exec(c) => {
                if let Some(cn) = conn.borrow().as_ref() {
                    exec(&cn.proxies.borrow(), &c);
                }
            }
            LoopCmd::Meter(on) => {
                meter_on.set(on);
                if let Some(cn) = conn.borrow_mut().as_mut() {
                    if !on {
                        cn.meter = None;
                    } else if cn.meter.is_none() {
                        cn.meter = start_meter(&cn.core, &tx);
                    }
                }
            }
            LoopCmd::Quit => {
                if let Some(m) = weak.upgrade() {
                    m.quit();
                }
            }
        })
    };

    mainloop.run();
    conn.borrow_mut().take();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Audit S4-10: a fragment is replaced whole, keeps 0644 whatever the umask, and leaves no
    /// temporary behind; many writers at once each leave one whole version.
    #[test]
    fn fragments_are_replaced_atomically() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("wol-audio-frag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("51-wolos.conf");
        write_fragment(&p, b"a = 1").unwrap();
        write_fragment(&p, b"a = 2").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"a = 2");
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, FRAGMENT_MODE);
        let writers: Vec<_> = (0..8)
            .map(|i| {
                let p = p.clone();
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        write_fragment(&p, format!("writer = {i}").as_bytes()).unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        let last = std::fs::read_to_string(&p).unwrap();
        assert!(last.starts_with("writer = ") && last.len() == 10, "{last}");
        let names: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().collect();
        assert_eq!(names.len(), 1, "no temporary left");
        assert!(write_fragment(&d.join("missing/x.conf"), b"x").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
