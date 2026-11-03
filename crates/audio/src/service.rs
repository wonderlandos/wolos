// SPDX-License-Identifier: GPL-2.0-or-later
//! The rules: events in, commands out. Backends ([`crate::fake::FakeBackend`], [`crate::pw`])
//! only move bytes; everything decided here is tested in `fake`.
//!
//! Threads (A6/A7): the service runs on its own thread ([`serve`]), so a `pactl` or `systemctl`
//! that takes seconds never stalls the PipeWire main loop (the meter, the registry); the PipeWire
//! thread only owns the proxies and reconnects with [`Backoff`] when PipeWire goes away (Reset
//! audio restarts it under our feet).
use crate::model::{Command, Event, Model, Node, Snapshot};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use wol_core::audio::{
    AGC_SOURCE, ALLOWED_RATES, APPMIX_SINK, Action, DeviceKind, MICMIX_SOURCE, PRO_AUDIO_CONF,
    QUANTA, RNNOISE_SOURCE, SNAPPED_FILE, appmix_argv, echo_cancel_argv, force_argv, linear_for,
    modules_matching, parse_pw_top_xruns, pro_audio_conf, reset_plan, rnnoise_argv, snap_to_base,
};

/// What a backend must do for the service. `Run` returns the program's stdout, `Err` the reason
/// it did not work (stderr, exit status, timeout); the other commands return `Ok("")` once sent.
pub trait Backend {
    fn execute(&mut self, cmd: &Command) -> Result<String, String>;
    fn read_file(&self, p: &Path) -> Option<String>;
}

/// The nodes our on-demand modules create (never the user's own devices).
const VIRTUAL: [&str; 4] = [AGC_SOURCE, RNNOISE_SOURCE, APPMIX_SINK, MICMIX_SOURCE];

/// A filter source that was loaded but has not shown up yet: the default input moves to it only
/// once its node exists (a default naming a missing node leaves the user with no input at all).
#[derive(Clone, Debug, PartialEq)]
struct PendingDefault {
    node_name: String,
    /// Ids that carried this name before the reload (the unloaded module's node, until its
    /// removal arrives); they do not count as the new node.
    stale: Vec<u32>,
}

pub struct Service<B: Backend> {
    pub model: Model,
    backend: B,
    home: PathBuf,
    snapped: Vec<String>,
    boost: bool,
    auto_input_level: bool,
    noise_suppression: bool,
    /// The user's real microphone (node name) while filters stand in front of it.
    physical_mic: Option<String>,
    pending_default: Option<PendingDefault>,
    /// PipeWire went away while a filter was on: the pipewire-pulse modules died with it, so the
    /// filters are loaded again when the microphone is back in the registry (A6-4).
    refilter: bool,
    connected: bool,
    /// The last action's problem, shown by the page until the next action.
    error: Option<String>,
}

impl<B: Backend> Service<B> {
    pub fn new(backend: B, home: PathBuf) -> Self {
        let snapped = backend
            .read_file(&home.join(SNAPPED_FILE))
            .map(|s| {
                s.lines()
                    .map(str::to_owned)
                    .filter(|l| !l.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let pro = backend
            .read_file(&home.join(PRO_AUDIO_CONF))
            .map(|t| wol_core::audio::parse_pro_audio_conf(&t))
            .unwrap_or_default();
        let model = Model {
            pro,
            ..Model::default()
        };
        Self {
            model,
            backend,
            home,
            snapped,
            boost: false,
            auto_input_level: false,
            noise_suppression: false,
            physical_mic: None,
            pending_default: None,
            refilter: false,
            connected: true,
            error: None,
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// "Allow louder": sliders may pass 0 dB (a UI toggle, not persisted here).
    pub fn set_boost(&mut self, on: bool) {
        self.boost = on;
    }

    /// The PipeWire connection went down (`false`: PipeWire restarts; the graph is forgotten
    /// until the registry sends it again) or came back (`true`). Always worth a redraw.
    pub fn set_connected(&mut self, up: bool) {
        self.connected = up;
        if !up {
            let pro = self.model.pro;
            self.model = Model {
                pro,
                ..Model::default()
            };
            self.pending_default = None;
            // The filters are pipewire-pulse modules: they are gone with the sound server, while
            // the switches (and the saved default naming the filter) still say "on". The
            // registry has to send the microphone again before they can be loaded (`handle`).
            self.refilter = self.auto_input_level || self.noise_suppression;
        }
    }

    /// Apply one backend event. Returns true when the snapshot changed.
    pub fn handle(&mut self, ev: Event) -> bool {
        let mic_removed =
            match &ev {
                Event::Removed(id) => self.model.nodes.get(id).is_some_and(|n| {
                    self.physical_mic.as_deref() == Some(n.info.node_name.as_str())
                }),
                _ => false,
            };
        let mut changed = self.model.apply(&ev);
        match &ev {
            Event::Props { id, .. } => self.snap_once(*id),
            Event::Route {
                device,
                route_device,
                ..
            } => {
                // A microphone whose `Props` came before its card's `Route` waited for it.
                let waiting: Vec<u32> = self
                    .model
                    .nodes
                    .values()
                    .filter(|n| {
                        n.seen_props
                            && n.info.device_id == Some(*device)
                            && n.info.route_device == Some(*route_device)
                    })
                    .map(|n| n.info.id)
                    .collect();
                for id in waiting {
                    self.snap_once(id);
                }
            }
            Event::Node(info) => {
                let arrived = self
                    .pending_default
                    .as_ref()
                    .is_some_and(|p| p.node_name == info.node_name && !p.stale.contains(&info.id));
                if arrived {
                    self.pending_default = None;
                    if let Err(e) = self.backend.execute(&Command::SetDefault {
                        kind: DeviceKind::Input,
                        node_name: info.node_name.clone(),
                    }) {
                        self.error =
                            Some(format!("Could not switch to the filtered microphone: {e}"));
                        changed = true;
                    }
                }
                if self.refilter && self.physical_mic.as_deref() == Some(info.node_name.as_str()) {
                    self.refilter = false;
                    self.error = self.rebuild_filters().err();
                    changed = true;
                }
            }
            _ => {}
        }
        if mic_removed {
            self.mic_unplugged();
            changed = true;
        }
        changed
    }

    /// The microphone the filters were fed from went away: move them to another real input, or
    /// switch them off (a filter with no microphone behind it is silence posing as an input).
    fn mic_unplugged(&mut self) {
        self.physical_mic = None;
        if !self.auto_input_level && !self.noise_suppression {
            return;
        }
        let other = self
            .model
            .nodes
            .values()
            .find(|n| {
                n.info.kind == DeviceKind::Input && !VIRTUAL.contains(&n.info.node_name.as_str())
            })
            .map(|n| n.info.node_name.clone());
        match other {
            Some(m) => {
                self.physical_mic = Some(m);
                self.error = self.rebuild_filters().err();
            }
            None => {
                self.refilter = false;
                self.auto_input_level = false;
                self.noise_suppression = false;
                self.pending_default = None;
                self.unload(&[AGC_SOURCE, RNNOISE_SOURCE]);
                self.error = Some(
                    "The microphone was unplugged, so Automatic input level and Reduce \
                     background noise are off."
                        .into(),
                );
            }
        }
    }

    /// The command that sets a node's volume: the device `Route` when the node has one (so the
    /// hardware mixer moves, like wpctl), else the node's own `Props` (software volume).
    fn volume_command(&self, id: u32, linear: f32) -> Command {
        match self.model.route_of(id) {
            Some((device, index, route_device)) => {
                let channels = self.model.nodes.get(&id).map_or(2, Node::channels);
                Command::SetRoute {
                    device,
                    index,
                    route_device,
                    channel_volumes: Some(vec![linear; channels]),
                    mute: None,
                }
            }
            None => Command::SetVolume { id, linear },
        }
    }

    /// Mute through the device `Route` when the node has one, else the node's `Props`.
    fn mute_command(&self, id: u32, muted: bool) -> Command {
        match self.model.route_of(id) {
            Some((device, index, route_device)) => Command::SetRoute {
                device,
                index,
                route_device,
                channel_volumes: None,
                mute: Some(muted),
            },
            None => Command::SetMute { id, muted },
        }
    }

    /// A microphone seen for the first time above 0 dB goes to 0 dB, once.
    fn snap_once(&mut self, id: u32) {
        let Some(n) = self.model.nodes.get(&id) else {
            return;
        };
        if VIRTUAL.contains(&n.info.node_name.as_str()) {
            return;
        }
        // The card has not said which route carries this node yet (`Props` can come before
        // `Route`): setting the node's own volume now would skip the hardware mixer and mark the
        // microphone done. The `Route` event calls this again.
        if n.info.device_id.is_some()
            && n.info.route_device.is_some()
            && self.model.route_of(id).is_none()
        {
            return;
        }
        let dev = self.model.device(n);
        let key = n.info.node_name.clone();
        let Some(target) = snap_to_base(
            &wol_core::audio::Device {
                name: key.clone(),
                ..dev
            },
            &self.snapped,
        ) else {
            return;
        };
        let cmd = self.volume_command(id, target);
        if let Err(e) = self.backend.execute(&cmd) {
            // Not remembered: the next Props event tries again.
            self.error = Some(format!("Could not set the microphone to 0 dB: {e}"));
            return;
        }
        self.snapped.push(key.clone());
        // A lost line only means the mic is snapped once more next session.
        let _ = self
            .backend
            .execute(&Command::AppendLine(self.home.join(SNAPPED_FILE), key));
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut devices: Vec<_> = self
            .model
            .nodes
            .values()
            .map(|n| self.model.device(n))
            .collect();
        devices.sort_by(|a, b| {
            (a.kind == DeviceKind::Input, a.name.to_lowercase())
                .cmp(&(b.kind == DeviceKind::Input, b.name.to_lowercase()))
        });
        Snapshot {
            devices,
            meter: self.model.meter,
            pro: self.model.pro,
            auto_input_level: self.auto_input_level,
            noise_suppression: self.noise_suppression,
            boost: self.boost,
            connected: self.connected,
            error: self.error.clone(),
        }
    }

    /// Carry out one action. Errors are for the UI's toast, in plain words; the last one also
    /// rides in [`Snapshot::error`] until the next action.
    pub fn act(&mut self, action: &Action) -> Result<(), String> {
        let r = self.act_inner(action);
        self.error = r.as_ref().err().cloned();
        r
    }

    fn act_inner(&mut self, action: &Action) -> Result<(), String> {
        match action {
            Action::Input(id) => {
                let n = self.model.nodes.get(id).ok_or("that input is gone")?;
                if n.info.kind != DeviceKind::Input {
                    return Err("not an input".into());
                }
                let name = n.info.node_name.clone();
                if !VIRTUAL.contains(&name.as_str()) {
                    self.physical_mic = Some(name.clone());
                    // With a filter on, the new microphone gets the filters, not a bypass.
                    if self.auto_input_level || self.noise_suppression {
                        return self.rebuild_filters();
                    }
                }
                self.pending_default = None;
                self.backend
                    .execute(&Command::SetDefault {
                        kind: DeviceKind::Input,
                        node_name: name,
                    })
                    .map_err(|e| format!("Could not switch the input: {e}"))?;
                Ok(())
            }
            Action::MicVolume(p) => {
                let n = self.default_input()?;
                let p = if self.boost { *p } else { (*p).min(100) };
                let cmd = self.volume_command(n.info.id, linear_for(p, n.volume_base));
                self.backend
                    .execute(&cmd)
                    .map_err(|e| format!("Could not set the input volume: {e}"))?;
                Ok(())
            }
            Action::OutputVolume(p) => {
                let n = self
                    .model
                    .default_node(DeviceKind::Output)
                    .cloned()
                    .ok_or("no output device")?;
                let p = if self.boost { *p } else { (*p).min(100) };
                let cmd = self.volume_command(n.info.id, linear_for(p, n.volume_base));
                self.backend
                    .execute(&cmd)
                    .map_err(|e| format!("Could not set the volume: {e}"))?;
                Ok(())
            }
            Action::MicMuted(m) => {
                let n = self.default_input()?;
                let cmd = self.mute_command(n.info.id, *m);
                self.backend
                    .execute(&cmd)
                    .map_err(|e| format!("Could not mute the input: {e}"))?;
                Ok(())
            }
            Action::AutoInputLevel(on) => {
                self.auto_input_level = *on;
                self.rebuild_filters()
            }
            Action::NoiseSuppression(on) => {
                self.noise_suppression = *on;
                self.rebuild_filters()
            }
            Action::BtCodec(addr, codec) => {
                let dev = self
                    .model
                    .nodes
                    .values()
                    .find(|n| n.info.address.as_deref() == Some(addr.as_str()))
                    .and_then(|n| n.info.device_id)
                    .ok_or("that Bluetooth device is not connected")?;
                if !codec.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    return Err("unknown codec".into());
                }
                self.backend
                    .execute(&Command::Run(wol_core::audio::bt_codec_argv(dev, codec)))
                    .map_err(|e| format!("The headphones did not take that codec: {e}"))?;
                Ok(())
            }
            Action::ProAudio { rate, quantum } => {
                if *rate != 0 && !ALLOWED_RATES.contains(rate) {
                    return Err(format!("rate {rate} is not one of {ALLOWED_RATES:?}"));
                }
                if *quantum != 0 && !QUANTA.contains(quantum) {
                    return Err(format!("buffer {quantum} is not one of {QUANTA:?}"));
                }
                let file = self.home.join(PRO_AUDIO_CONF);
                let text = pro_audio_conf(*rate, *quantum);
                let saved = if text.is_empty() {
                    self.backend.execute(&Command::RemovePath(file))
                } else {
                    self.backend.execute(&Command::WriteFile(file, text))
                };
                saved.map_err(|e| format!("Could not save the Pro Audio settings: {e}"))?;
                self.model.pro.rate = *rate;
                self.model.pro.quantum = *quantum;
                for (key, v) in [
                    ("clock.force-rate", *rate),
                    ("clock.force-quantum", *quantum),
                ] {
                    self.backend
                        .execute(&Command::Run(force_argv(key, v)))
                        .map_err(|e| format!("Saved; it applies after the next restart: {e}"))?;
                }
                Ok(())
            }
            Action::ResetAudio => {
                self.refilter = false;
                self.auto_input_level = false;
                self.noise_suppression = false;
                self.physical_mic = None;
                self.pending_default = None;
                self.unload(&VIRTUAL);
                // Audit S4-22: WirePlumber's state is under $XDG_STATE_HOME; never a relative path.
                let state = std::env::var_os("XDG_STATE_HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .unwrap_or_else(|| self.home.join(".local/state"));
                let Some(plan) = reset_plan(&self.home, &state) else {
                    return Err("Sound could not be reset: the home folder is unknown.".into());
                };
                // Every step runs even when one did not work: half a reset helps nobody.
                let mut first_err = None;
                for p in &plan.remove {
                    if let Err(e) = self.backend.execute(&Command::RemovePath(p.clone())) {
                        first_err.get_or_insert(e);
                    }
                }
                for c in &plan.commands {
                    if let Err(e) = self.backend.execute(&Command::Run(c.clone())) {
                        first_err.get_or_insert(e);
                    }
                }
                self.snapped.clear();
                self.model.pro = Default::default();
                match first_err {
                    Some(e) => Err(format!("Sound was reset, but one step did not work: {e}")),
                    None => Ok(()),
                }
            }
        }
    }

    fn default_input(&self) -> Result<Node, String> {
        self.model
            .default_node(DeviceKind::Input)
            .cloned()
            .ok_or_else(|| "no input device".to_string())
    }

    /// The microphone the filters feed on: the remembered one, else the current default when it
    /// is a real device.
    fn mic(&self) -> Option<String> {
        if let Some(m) = &self.physical_mic {
            return Some(m.clone());
        }
        let d = self.model.default_node(DeviceKind::Input)?;
        let name = d.info.node_name.clone();
        (!VIRTUAL.contains(&name.as_str())).then_some(name)
    }

    /// Tear down the filter modules and load the ones switched on: mic -> [AGC] -> [RNNoise].
    /// The last stage that loaded becomes the default input once its node exists; a stage that
    /// does not load is switched off and named in the error, and the rest of the chain still
    /// works (both failing leaves the real microphone as the default).
    fn rebuild_filters(&mut self) -> Result<(), String> {
        let mic = self.mic().ok_or("no input device")?;
        if self.model.node_by_name(&mic).is_none() {
            self.physical_mic = None;
            return Err("that input is gone".into());
        }
        self.physical_mic = Some(mic.clone());
        self.pending_default = None;
        self.unload(&[AGC_SOURCE, RNNOISE_SOURCE]);
        let mut head = mic.clone();
        let mut err = None;
        if self.auto_input_level {
            let ran = echo_cancel_argv(&head).and_then(|a| self.backend.execute(&Command::Run(a)));
            match ran {
                Ok(_) => head = AGC_SOURCE.into(),
                Err(e) => {
                    self.auto_input_level = false;
                    err = Some(format!("Automatic input level could not start: {e}"));
                }
            }
        }
        if self.noise_suppression {
            let ran = rnnoise_argv(&head).and_then(|a| self.backend.execute(&Command::Run(a)));
            match ran {
                Ok(_) => head = RNNOISE_SOURCE.into(),
                Err(e) => {
                    self.noise_suppression = false;
                    err = Some(format!(
                        "Reduce background noise could not start; it needs the \
                         noise-suppression-for-voice package: {e}"
                    ));
                }
            }
        }
        if head == mic {
            if self.model.default_source.as_deref() != Some(mic.as_str()) {
                self.backend
                    .execute(&Command::SetDefault {
                        kind: DeviceKind::Input,
                        node_name: mic,
                    })
                    .map_err(|e| format!("Could not switch the input: {e}"))?;
            }
        } else {
            let stale = self
                .model
                .nodes
                .values()
                .filter(|n| n.info.node_name == head)
                .map(|n| n.info.id)
                .collect();
            self.pending_default = Some(PendingDefault {
                node_name: head,
                stale,
            });
        }
        err.map_or(Ok(()), Err)
    }

    /// `wol audio appmix on`: the App Mix sink and the Mic + App Mix source for OBS. A module
    /// that does not load takes the others down again.
    pub fn appmix(&mut self, on: bool) -> Result<(), String> {
        let r = self.appmix_inner(on);
        self.error = r.as_ref().err().cloned();
        r
    }

    fn appmix_inner(&mut self, on: bool) -> Result<(), String> {
        self.unload(&[APPMIX_SINK, MICMIX_SOURCE]);
        if on {
            let mic = self
                .model
                .default_node(DeviceKind::Input)
                .map(|n| n.info.node_name.clone())
                .ok_or("no input device")?;
            // R2C-13: the name is spliced into a pactl argument.
            wol_core::audio::node_name_ok(&mic)?;
            for argv in appmix_argv(&mic) {
                if let Err(e) = self.backend.execute(&Command::Run(argv)) {
                    self.unload(&[APPMIX_SINK, MICMIX_SOURCE]);
                    return Err(format!("The App Mix could not start: {e}"));
                }
            }
        }
        Ok(())
    }

    /// Unload every pipewire-pulse module whose arguments name one of `markers`.
    fn unload(&mut self, markers: &[&str]) {
        let list = self
            .backend
            .execute(&Command::Run(sv(&["pactl", "list", "short", "modules"])))
            .unwrap_or_default();
        for id in modules_matching(&list, markers) {
            // A module that is already gone is what we wanted.
            let _ = self.backend.execute(&Command::Run(vec![
                "pactl".into(),
                "unload-module".into(),
                id.to_string(),
            ]));
        }
    }
}

fn sv(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// What the UI asks of the service thread.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Act(Action),
    /// `wol audio appmix on|off` from the UI side.
    AppMix(bool),
    /// "Allow louder".
    Boost(bool),
    /// Show (true) or hide (false) the input level: starts/stops the capture stream and the
    /// xrun poll.
    Meter(bool),
    /// A fresh snapshot now.
    Snapshot,
    Quit,
}

/// What the service thread receives: the UI's requests and the PipeWire thread's news.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Ui(Request),
    Event(Event),
    /// The PipeWire connection went down (false) or is up again (true).
    Connected(bool),
}

/// How [`serve_guarded`] ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// [`Request::Quit`], or every sender is gone.
    Quit,
    /// The service panicked; the caller starts a fresh one.
    Panicked,
}

/// What Settings shows after the audio service stopped on a bug (R2C-14).
pub const SERVICE_STOPPED: &str = "The sound service stopped on a problem and is starting again.";

/// [`serve`] with a panic stopped at the thread boundary (R2C-14): it is logged and the UI gets a
/// last snapshot that says so (`connected` false, an error), never stale state with no word.
pub fn serve_guarded<B: Backend>(
    service: &mut Service<B>,
    rx: &mpsc::Receiver<Input>,
    tx: &mpsc::Sender<Snapshot>,
    poll: Duration,
    meter: &mut dyn FnMut(bool),
) -> Ended {
    let run = std::panic::AssertUnwindSafe(|| serve(service, rx, tx, poll, meter));
    match std::panic::catch_unwind(run) {
        Ok(()) => Ended::Quit,
        Err(p) => {
            let why = p
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            eprintln!("wol-audio: the service stopped on a bug: {why}");
            let _ = tx.send(Snapshot {
                connected: false,
                error: Some(SERVICE_STOPPED.into()),
                ..Snapshot::default()
            });
            Ended::Panicked
        }
    }
}

/// Fresh services after a panic before the thread gives up (each waits 1, 2, 4 s first).
pub const MAX_RESTARTS: u32 = 3;

/// The service thread: apply inputs in order, send a snapshot after every change, and while the
/// meter is shown read the xruns every `poll` (`pw-top`, which takes a moment: it runs here, not
/// on the PipeWire loop). `meter` tells the PipeWire thread to start or stop the capture stream.
/// Returns on [`Request::Quit`] or when every sender is gone.
pub fn serve<B: Backend>(
    service: &mut Service<B>,
    rx: &mpsc::Receiver<Input>,
    tx: &mpsc::Sender<Snapshot>,
    poll: Duration,
    meter: &mut dyn FnMut(bool),
) {
    let push = |s: &Service<B>| {
        let _ = tx.send(s.snapshot());
    };
    let mut meter_on = false;
    let mut next_poll = Instant::now() + poll;
    loop {
        let input = if meter_on {
            let wait = next_poll.saturating_duration_since(Instant::now());
            match rx.recv_timeout(wait) {
                Ok(i) => Some(i),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Ok(i) => Some(i),
                Err(_) => return,
            }
        };
        if meter_on && Instant::now() >= next_poll {
            next_poll = Instant::now() + poll;
            let out = service
                .backend_mut()
                .execute(&Command::Run(sv(&["pw-top", "-b", "-n", "1"])))
                .unwrap_or_default();
            if service.handle(Event::Xruns(parse_pw_top_xruns(&out))) {
                push(service);
            }
        }
        let Some(input) = input else { continue };
        match input {
            Input::Event(mut ev) => {
                // The capture stream follows the default input; name it here, where the model is.
                if let Event::Peak { device, .. } = &mut ev {
                    *device = service
                        .model
                        .default_node(DeviceKind::Input)
                        .map_or(0, |n| n.info.id);
                }
                if service.handle(ev) {
                    push(service);
                }
            }
            Input::Connected(up) => {
                service.set_connected(up);
                push(service);
            }
            Input::Ui(Request::Act(a)) => {
                // The error rides in the snapshot (Snapshot::error).
                let _ = service.act(&a);
                push(service);
            }
            Input::Ui(Request::AppMix(on)) => {
                let _ = service.appmix(on);
                push(service);
            }
            Input::Ui(Request::Boost(on)) => {
                service.set_boost(on);
                push(service);
            }
            Input::Ui(Request::Meter(on)) => {
                meter_on = on;
                next_poll = Instant::now() + poll;
                meter(on);
            }
            Input::Ui(Request::Snapshot) => push(service),
            Input::Ui(Request::Quit) => return,
        }
    }
}

/// When to try PipeWire again after the connection broke: 1 s, 2 s, 4 s, then every 8 s, back to
/// 1 s after a success. Pure, so the schedule is tested without a sound server.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Backoff {
    fails: u32,
    next: Option<Instant>,
}

impl Backoff {
    /// Record a failure (or the drop itself) at `now`; returns the wait before the next try.
    pub fn failed(&mut self, now: Instant) -> Duration {
        let d = Duration::from_secs(1 << self.fails.min(3));
        self.fails = self.fails.saturating_add(1);
        self.next = Some(now + d);
        d
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// True when a connection attempt is allowed at `now`.
    pub fn due(&self, now: Instant) -> bool {
        self.next.is_none_or(|t| now >= t)
    }
}
