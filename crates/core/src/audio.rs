// SPDX-License-Identifier: BSD-2-Clause
//! Sound model shared by Settings, Control Center, `wol audio` and the wol-audio service
//! (crates/audio): the frozen types of the audio audit and the pure rules behind them. Nothing here
//! touches PipeWire; backends feed [`Device`]s in and take the numbers out (docs/modules/audio.md).
//!
//! The one rule: **100% on a slider is 0 dB** (`volume_base`), not the hardware maximum. Codecs
//! merge Capture and Mic Boost into one ALSA slider, so the maximum is +20..+30 dB and a voice
//! clips; the historical PipeWire default put every new microphone there. Louder than 0 dB needs
//! the explicit "Allow louder" toggle ([`percent_of`] with `boost`).
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Graph rates Pro Audio offers (70-wolos-latency.conf `default.clock.allowed-rates`).
pub const ALLOWED_RATES: [u32; 4] = [44100, 48000, 88200, 96000];
/// Buffer sizes Pro Audio offers (frames); 64 is the graph minimum (70-wolos-latency.conf).
pub const QUANTA: [u32; 6] = [64, 128, 256, 512, 1024, 2048];
/// The user file Pro Audio writes (relative to `$HOME`); `wol audio reset` removes it.
pub const PRO_AUDIO_CONF: &str = ".config/pipewire/pipewire.conf.d/50-wolos-pro-audio.conf";
/// Per-user WirePlumber state (default device, stored volumes); `wol audio reset` removes it.
pub const WIREPLUMBER_STATE: &str = ".local/state/wireplumber";
/// Devices already snapped to 0 dB once (one node name per line, relative to `$HOME`).
pub const SNAPPED_FILE: &str = ".local/state/wolos/audio-snapped";
/// Node names of the on-demand virtual devices (pipewire-pulse modules, never static config).
pub const AGC_SOURCE: &str = "wolos_mic_agc";
pub const RNNOISE_SOURCE: &str = "wolos_mic_rnnoise";
pub const APPMIX_SINK: &str = "wolos_appmix";
pub const MICMIX_SOURCE: &str = "wolos_mic_mix";
/// Peak meter decay: 20 dB per second (what a VU needle does), fed from the capture stream.
pub const METER_DECAY_DB_PER_S: f32 = 20.0;
/// Above this linear peak the input clips (-0.3 dBFS).
pub const CLIP_LEVEL: f32 = 0.966;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Output,
    Input,
}

/// One sound device as the UI lists it. `volume` and `volume_base` are PipeWire's linear gains
/// (`channelVolumes` mean, `volumeBase`); the slider percent comes from [`percent_of`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Device {
    /// PipeWire node id (what setters take).
    pub id: u32,
    /// `node.description`, else `node.nick`, else `node.name`.
    pub name: String,
    pub kind: DeviceKind,
    pub default: bool,
    pub volume: f32,
    /// Linear gain at 0 dB; 1.0 when the device has no boost range.
    pub volume_base: f32,
    pub muted: bool,
    /// Bluetooth codec in use (`api.bluez5.codec`), e.g. `ldac`.
    pub codec: Option<String>,
    /// Card profile (`device.profile.name`), e.g. `pro-audio`, `a2dp-sink`.
    pub profile: Option<String>,
}

impl Default for Device {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            kind: DeviceKind::Output,
            default: false,
            volume: 0.0,
            volume_base: 1.0,
            muted: false,
            codec: None,
            profile: None,
        }
    }
}

/// Live input level: `peak` is the newest buffer's peak, `hold` decays at [`METER_DECAY_DB_PER_S`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InputMeter {
    pub device: u32,
    pub peak: f32,
    pub hold: f32,
    pub clipping: bool,
}

impl InputMeter {
    /// Feed one buffer's linear peak after `dt` seconds since the last one.
    pub fn feed(&mut self, peak: f32, dt: f32) -> &mut Self {
        let peak = peak.clamp(0.0, 1.0);
        self.hold = decay(self.hold, dt).max(peak);
        self.peak = peak;
        self.clipping = peak >= CLIP_LEVEL;
        self
    }
}

/// Settings > Sound > Pro Audio: 0 means "not forced" (the graph's own rate / quantum).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProAudio {
    pub rate: u32,
    pub quantum: u32,
    /// Xruns summed over the graph since the service started (`pw-top` ERR column).
    pub xruns: u32,
}

/// What the UI and `wol audio` can ask for. The service turns these into PipeWire calls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "action", content = "value")]
pub enum Action {
    /// Make this node the default input.
    Input(u32),
    /// Default input slider, 0..100 (100 = 0 dB; above 100 only with boost).
    MicVolume(u16),
    MicMuted(bool),
    /// Default output slider, same scale as [`Action::MicVolume`] (100 = 0 dB; above 100 only
    /// with boost). The service converts with [`linear_for`], so the slider and the hardware
    /// agree on where 0 dB is (`wpctl set-volume N%` counts from the hardware maximum instead).
    OutputVolume(u16),
    /// "Automatic input level": module-echo-cancel with WebRTC gain control -> `WolOS Microphone`.
    AutoInputLevel(bool),
    /// "Reduce background noise": RNNoise LADSPA source on the default input.
    NoiseSuppression(bool),
    /// Bluetooth codec for a device (`addr` is the `api.bluez5.address`), e.g. `ldac`.
    BtCodec(String, String),
    /// Force the graph rate and/or buffer (0 = let the graph choose); persisted in a conf.d file.
    ProAudio {
        rate: u32,
        quantum: u32,
    },
    /// `wol audio reset`: forget stored defaults and volumes, remove the pro-audio file, restart.
    ResetAudio,
}

// ---- 0 dB rule: cubic slider math ----

/// Slider percent for a linear gain: 100 is `volume_base` (0 dB). Without `boost` the slider
/// stops at 100; with it the hardware maximum (linear 1.0) is reachable.
pub fn percent_of(linear: f32, volume_base: f32, boost: bool) -> u16 {
    let base = if volume_base > 0.0 { volume_base } else { 1.0 };
    let p = (linear.max(0.0) / base).cbrt() * 100.0;
    let cap = if boost { max_percent(base) } else { 100 };
    (p.round() as i64).clamp(0, i64::from(cap)) as u16
}

/// Linear gain for a slider percent (cubic), never above the hardware maximum.
pub fn linear_for(percent: u16, volume_base: f32) -> f32 {
    let base = if volume_base > 0.0 { volume_base } else { 1.0 };
    let f = f32::from(percent) / 100.0;
    (base * f * f * f).clamp(0.0, 1.0)
}

/// What `pactl set-sink-volume N%` and `wpctl set-volume N%` call the slider position `percent`
/// (100 = 0 dB) on a device whose 0 dB sits at `volume_base`: both count from the hardware
/// maximum, cubic like the slider, so a base below 1.0 makes the numbers differ.
pub fn hardware_percent(percent: u16, volume_base: f32) -> u16 {
    percent_of(linear_for(percent, volume_base), 1.0, false)
}

/// The slider's end when boost is allowed: the hardware maximum in percent of 0 dB.
pub fn max_percent(volume_base: f32) -> u16 {
    if volume_base <= 0.0 || volume_base >= 1.0 {
        return 100;
    }
    ((1.0 / volume_base).cbrt() * 100.0)
        .round()
        .clamp(100.0, 1000.0) as u16
}

/// Decibels relative to 0 dB for a linear gain (`-inf` at silence).
pub fn db_of(linear: f32, volume_base: f32) -> f32 {
    let base = if volume_base > 0.0 { volume_base } else { 1.0 };
    if linear <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * (linear / base).log10()
    }
}

/// Decay a meter level over `dt` seconds at [`METER_DECAY_DB_PER_S`].
pub fn decay(level: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        return level;
    }
    level * 10f32.powf(-METER_DECAY_DB_PER_S * dt / 20.0)
}

/// Snap-once policy: a microphone that appears for the first time louder than 0 dB is put at
/// 0 dB; the name goes into [`SNAPPED_FILE`] so a user's later choice is kept. Returns the new
/// linear gain to set, or None when nothing is to do.
pub fn snap_to_base(dev: &Device, snapped: &[String]) -> Option<f32> {
    if dev.kind != DeviceKind::Input || snapped.iter().any(|n| n == &dev.name) {
        return None;
    }
    // No boost range: 0 dB is the maximum and 40% (0.064) from WirePlumber is already safe.
    if dev.volume_base >= 1.0 || dev.volume_base <= 0.0 {
        return None;
    }
    (dev.volume > dev.volume_base + 1e-4).then_some(dev.volume_base)
}

// ---- Pro Audio file ----

/// The user fragment Pro Audio writes; empty when nothing is forced (the caller removes the file).
///
/// A quantum pins the graph: quantum = min-quantum = max-quantum = N. In a VM PipeWire's own
/// `context.properties.rules` (pipewire.conf) raise default.clock.min-quantum to
/// [`VM_MIN_QUANTUM`], and whether that rule lands before or after this file is PipeWire's load
/// order; a pinned N below 1024 would then leave min (1024) above max (N). So for N < 1024 the
/// file carries the same `cpu.vm.name` match and pins 1024 there: N on hardware, 1024 in a VM,
/// min never above max in either order. The live `clock.force-quantum` still gives N in a VM.
pub fn pro_audio_conf(rate: u32, quantum: u32) -> String {
    let mut s = String::from(
        "# Written by Settings > Sound > Pro Audio (WolOS). `wol audio reset` removes it.\n",
    );
    if rate == 0 && quantum == 0 {
        return String::new();
    }
    s.push_str("context.properties = {\n");
    if rate != 0 {
        s.push_str(&format!("    default.clock.rate = {rate}\n"));
        s.push_str(&format!("    default.clock.allowed-rates = [ {rate} ]\n"));
    }
    if quantum != 0 {
        s.push_str(&format!("    default.clock.quantum = {quantum}\n"));
        s.push_str(&format!("    default.clock.min-quantum = {quantum}\n"));
        s.push_str(&format!("    default.clock.max-quantum = {quantum}\n"));
    }
    s.push_str("}\n");
    s.push_str(&pro_audio_vm_block(quantum));
    s
}

/// The buffer of the "low-latency" profile (`wol audio profile low-latency`, ADR 0032): 128 frames,
/// 2.7 ms at 48 kHz. Monitoring through a DAW wants it; 64 frames (Pro Audio) is the graph's floor
/// and crackles on a busy machine. It is a Pro Audio buffer, so the file, the live force and
/// `wol audio reset` are the ones Pro Audio already has.
pub const LOW_LATENCY_QUANTUM: u32 = 128;

/// The name of the profile a Pro Audio setting amounts to: `default` (the graph chooses its
/// buffer), `low-latency` ([`LOW_LATENCY_QUANTUM`]), else `custom` (Settings > Sound > Pro Audio).
pub fn profile_name(quantum: u32) -> &'static str {
    match quantum {
        0 => "default",
        LOW_LATENCY_QUANTUM => "low-latency",
        _ => "custom",
    }
}

/// The VM block of [`pro_audio_conf`] for `quantum`: pins [`VM_MIN_QUANTUM`] in a VM when the
/// quantum is below it; empty otherwise.
fn pro_audio_vm_block(quantum: u32) -> String {
    if quantum == 0 || quantum >= VM_MIN_QUANTUM {
        return String::new();
    }
    let quantum = VM_MIN_QUANTUM;
    format!(
        "{PRO_AUDIO_VM_RULES}\n    {{\n        matches = [ {{ cpu.vm.name = !null }} ]\n        \
         actions = {{\n            update-props = {{\n                \
         default.clock.quantum = {quantum}\n                \
         default.clock.min-quantum = {quantum}\n                \
         default.clock.max-quantum = {quantum}\n            }}\n        }}\n    }}\n]\n"
    )
}

/// The floor PipeWire's own rule sets when `cpu.vm.name` is present (pipewire.conf).
pub const VM_MIN_QUANTUM: u32 = 1024;

/// Opens the VM block of [`pro_audio_conf`]; [`parse_pro_audio_conf`] stops reading there.
const PRO_AUDIO_VM_RULES: &str = "context.properties.rules = [";

/// `key = value` or `key=value` (the spaces are the writer's, not the format's).
fn key_value(line: &str) -> Option<(&str, &str)> {
    let (k, v) = line.trim().split_once('=')?;
    Some((k.trim(), v.trim()))
}

/// Read back what [`pro_audio_conf`] wrote (a hand-edited file may leave out the spaces).
pub fn parse_pro_audio_conf(text: &str) -> ProAudio {
    let mut p = ProAudio::default();
    for l in text.lines() {
        if l.trim() == PRO_AUDIO_VM_RULES {
            break;
        }
        match key_value(l) {
            Some(("default.clock.rate", v)) => p.rate = v.parse().unwrap_or(0),
            Some(("default.clock.quantum", v)) => p.quantum = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    p
}

/// The keys [`pro_audio_conf`] writes at the top level: the only lines [`rewrite_pro_audio_conf`]
/// touches.
const PRO_AUDIO_KEYS: [&str; 5] = [
    "default.clock.rate",
    "default.clock.allowed-rates",
    "default.clock.quantum",
    "default.clock.min-quantum",
    "default.clock.max-quantum",
];

/// `old` (the Pro Audio file as it is) with its rate and quantum lines rewritten for `rate` and
/// `quantum` and every other line and comment kept where it was. A file that has no such block
/// gets one at its end. The result is empty when nothing but those two settings (and the writer's
/// own header comment) was in the file: the caller then removes it instead of keeping a shell.
pub fn rewrite_pro_audio_conf(old: &str, rate: u32, quantum: u32) -> String {
    let mut lines: Vec<String> = old.lines().map(str::to_owned).collect();
    // The writer's VM block (only that one: it names the VM match and the clock quantum).
    if let Some(i) = lines.iter().position(|l| l.trim() == PRO_AUDIO_VM_RULES)
        && let Some(len) = lines[i..].iter().position(|l| l.trim() == "]")
        && lines[i..=i + len].iter().any(|l| l.contains("cpu.vm.name"))
    {
        lines.drain(i..=i + len);
    }
    let squash = |l: &str| l.split_whitespace().collect::<String>();
    let open = lines
        .iter()
        .position(|l| squash(l) == "context.properties={");
    // (key, line) of what the file should hold for `rate` and `quantum`.
    let wanted = |rate: u32, quantum: u32| -> Vec<(&'static str, String)> {
        let mut v = Vec::new();
        if rate != 0 {
            v.push((
                PRO_AUDIO_KEYS[0],
                format!("    default.clock.rate = {rate}"),
            ));
            v.push((
                PRO_AUDIO_KEYS[1],
                format!("    default.clock.allowed-rates = [ {rate} ]"),
            ));
        }
        if quantum != 0 {
            v.push((
                PRO_AUDIO_KEYS[2],
                format!("    default.clock.quantum = {quantum}"),
            ));
            v.push((
                PRO_AUDIO_KEYS[3],
                format!("    default.clock.min-quantum = {quantum}"),
            ));
            v.push((
                PRO_AUDIO_KEYS[4],
                format!("    default.clock.max-quantum = {quantum}"),
            ));
        }
        v
    };
    match open {
        Some(open) => {
            let close = lines[open..]
                .iter()
                .position(|l| l.trim() == "}")
                .map_or(lines.len(), |c| open + c);
            // Each of our lines is replaced where it stands (or dropped); the ones the file did not
            // have yet go last in the block.
            let mut todo = wanted(rate, quantum);
            let mut kept = Vec::new();
            for l in &lines[open + 1..close] {
                match key_value(l)
                    .map(|(k, _)| k)
                    .filter(|k| PRO_AUDIO_KEYS.contains(k))
                {
                    Some(k) => {
                        if let Some(i) = todo.iter().position(|(w, _)| *w == k) {
                            kept.push(todo.remove(i).1);
                        }
                    }
                    None => kept.push(l.clone()),
                }
            }
            kept.extend(todo.into_iter().map(|(_, l)| l));
            let tail: Vec<String> = lines.split_off(close.min(lines.len()));
            lines.truncate(open + 1);
            lines.extend(kept);
            lines.extend(tail);
        }
        None if rate != 0 || quantum != 0 => {
            lines.push("context.properties = {".into());
            lines.extend(wanted(rate, quantum).into_iter().map(|(_, l)| l));
            lines.push("}".into());
        }
        None => {}
    }
    // An empty block says nothing.
    if let Some(open) = lines
        .iter()
        .position(|l| squash(l) == "context.properties={")
        && lines.get(open + 1).is_some_and(|l| l.trim() == "}")
    {
        lines.drain(open..=open + 1);
    }
    let bare = lines
        .iter()
        .all(|l| l.trim().is_empty() || l.starts_with("# Written by Settings > Sound > Pro Audio"));
    if bare && pro_audio_vm_block(quantum).is_empty() {
        return String::new();
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out.push_str(&pro_audio_vm_block(quantum));
    out
}

/// Live overrides as `pw-metadata -n settings 0` prints them (`clock.force-rate`,
/// `clock.force-quantum`); 0 when not forced.
pub fn parse_settings_metadata(text: &str) -> (u32, u32) {
    let mut rate = 0;
    let mut quantum = 0;
    for l in text.lines() {
        let Some((key, rest)) = l.split_once("key:'").and_then(|(_, r)| r.split_once('\'')) else {
            continue;
        };
        let value = rest
            .split_once("value:'")
            .and_then(|(_, r)| r.split_once('\''))
            .map(|(v, _)| v.trim().parse::<u32>().unwrap_or(0))
            .unwrap_or(0);
        match key {
            "clock.force-rate" => rate = value,
            "clock.force-quantum" => quantum = value,
            _ => {}
        }
    }
    (rate, quantum)
}

/// `pw-metadata -n settings 0 <key> <value>`: apply a rate/quantum now (0 clears).
pub fn force_argv(key: &str, value: u32) -> Vec<String> {
    vec![
        "pw-metadata".into(),
        "-n".into(),
        "settings".into(),
        "0".into(),
        key.into(),
        value.to_string(),
    ]
}

/// Buffer length in milliseconds.
pub fn quantum_ms(quantum: u32, rate: u32) -> f32 {
    if rate == 0 {
        return 0.0;
    }
    quantum as f32 * 1000.0 / rate as f32
}

/// Sum of the ERR column of `pw-top -b -n 1` (one batch); the header names the columns.
pub fn parse_pw_top_xruns(text: &str) -> u32 {
    let mut col = None;
    let mut sum = 0u32;
    for l in text.lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        // The header names the columns; rows start with a state letter (`S` suspended too).
        if f.contains(&"ERR") && f.contains(&"QUANT") {
            col = f.iter().position(|c| *c == "ERR");
            continue;
        }
        if let Some(v) = col.and_then(|c| f.get(c)) {
            sum = sum.saturating_add(v.parse().unwrap_or(0));
        }
    }
    sum
}

// ---- pw-dump (the CLI's view; the service gets the same from the registry) ----

/// Audio sinks and sources from `pw-dump` JSON, with the defaults from the metadata object.
pub fn parse_pw_dump(json: &str) -> Vec<Device> {
    let Ok(serde_json::Value::Array(objs)) = serde_json::from_str::<serde_json::Value>(json) else {
        return vec![];
    };
    let mut def_sink = None;
    let mut def_source = None;
    for o in &objs {
        if o["type"] != "PipeWire:Interface:Metadata" {
            continue;
        }
        for m in o["metadata"].as_array().into_iter().flatten() {
            let name = m["value"]["name"].as_str().map(str::to_owned);
            match m["key"].as_str() {
                Some("default.audio.sink") => def_sink = name,
                Some("default.audio.source") => def_source = name,
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for o in &objs {
        if o["type"] != "PipeWire:Interface:Node" {
            continue;
        }
        let props = &o["info"]["props"];
        let kind = match props["media.class"].as_str() {
            Some("Audio/Sink") => DeviceKind::Output,
            Some("Audio/Source") => DeviceKind::Input,
            _ => continue,
        };
        let node_name = props["node.name"].as_str().unwrap_or_default();
        let name = ["node.description", "node.nick", "node.name"]
            .iter()
            .find_map(|k| props[*k].as_str())
            .unwrap_or_default()
            .to_owned();
        let p = o["info"]["params"]["Props"]
            .as_array()
            .and_then(|a| a.iter().find(|p| p.get("volume").is_some()))
            .cloned()
            .unwrap_or_default();
        let volume = p["channelVolumes"]
            .as_array()
            .filter(|a| !a.is_empty())
            .map(|a| a.iter().filter_map(|v| v.as_f64()).sum::<f64>() / a.len() as f64)
            .or_else(|| p["volume"].as_f64())
            .unwrap_or(0.0) as f32;
        let def = match kind {
            DeviceKind::Output => def_sink.as_deref() == Some(node_name),
            DeviceKind::Input => def_source.as_deref() == Some(node_name),
        };
        out.push(Device {
            id: o["id"].as_u64().unwrap_or(0) as u32,
            name,
            kind,
            default: def,
            volume,
            volume_base: p["volumeBase"].as_f64().unwrap_or(1.0) as f32,
            muted: p["mute"].as_bool().unwrap_or(false),
            codec: props["api.bluez5.codec"].as_str().map(str::to_owned),
            profile: props["device.profile.name"].as_str().map(str::to_owned),
        });
    }
    out
}

// ---- On-demand modules (pipewire-pulse hosts them; nothing is loaded statically) ----

/// `pactl load-module ...` for "Automatic input level": echo cancel with WebRTC gain control and
/// noise suppression in front of `source` (a node name); the result is "WolOS Microphone".
///
/// pactl joins its arguments with spaces and pipewire-pulse splits them again, so a value with a
/// space must be quoted inside the argument (`key="a b"`, `key='x="a b"'`); an unquoted value ends
/// at the first space and the rest becomes a module argument of its own, which PipeWire 1.6 refuses
/// (`module_args_check`). `aec_args` names the PipeWire WebRTC keys (`webrtc.*`,
/// spa/plugins/aec/aec-webrtc.cpp); `webrtc.gain_control` defaults to false there, and it is the
/// whole point of this filter.
pub fn echo_cancel_argv(source: &str) -> Result<Vec<String>, String> {
    node_name_ok(source)?;
    Ok(vec![
        "pactl".into(),
        "load-module".into(),
        "module-echo-cancel".into(),
        format!("source_name={AGC_SOURCE}"),
        format!("source_master={source}"),
        "aec_method=webrtc".into(),
        format!("aec_args=\"{AEC_ARGS}\""),
        "source_properties='device.description=\"WolOS Microphone\"'".into(),
        "sink_properties='device.description=\"WolOS Microphone (echo reference)\" \
         node.passive=true'"
            .into(),
    ])
}

/// A PipeWire node name spliced into a pactl module argument (R2C-13): pactl joins arguments
/// with spaces and pipewire-pulse splits them again, so only `[A-Za-z0-9._:-]` is let through
/// (WirePlumber's names: `alsa_input.pci-0000_00_1f.3.analog-stereo`, `bluez_input.00:11:..`).
pub fn node_name_ok(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 256
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b));
    if ok {
        Ok(())
    } else {
        Err("that input's name cannot be used".into())
    }
}

/// The WebRTC settings behind "Automatic input level", as PipeWire's `aec.args` keys.
pub const AEC_ARGS: &str = "webrtc.gain_control=true webrtc.noise_suppression=true \
     webrtc.voice_detection=true webrtc.high_pass_filter=true webrtc.extended_filter=true";

/// `pactl load-module ...` for "Reduce background noise": the RNNoise LADSPA plugin
/// (noise-suppression-for-voice) as a filter in front of `source`.
pub fn rnnoise_argv(source: &str) -> Result<Vec<String>, String> {
    node_name_ok(source)?;
    Ok(vec![
        "pactl".into(),
        "load-module".into(),
        "module-ladspa-source".into(),
        format!("source_name={RNNOISE_SOURCE}"),
        format!("master={source}"),
        "plugin=librnnoise_ladspa".into(),
        "label=noise_suppressor_mono".into(),
        "control=50".into(),
        "source_properties='device.description=\"WolOS Microphone (noise reduced)\"'".into(),
    ])
}

/// The App Mix for OBS: a "WolOS App Mix" sink apps can be sent to, played on to the default
/// output so the user still hears it, and a "Mic + App Mix" source combining it with `mic`.
pub fn appmix_argv(mic: &str) -> Vec<Vec<String>> {
    let pactl = |rest: &[&str]| -> Vec<String> {
        let mut v = vec!["pactl".to_string(), "load-module".to_string()];
        v.extend(rest.iter().map(|s| s.to_string()));
        v
    };
    vec![
        pactl(&[
            "module-null-sink",
            &format!("sink_name={APPMIX_SINK}"),
            "sink_properties='device.description=\"WolOS App Mix\"'",
        ]),
        pactl(&[
            "module-loopback",
            &format!("source={APPMIX_SINK}.monitor"),
            "sink=@DEFAULT_SINK@",
            "latency_msec=20",
            "source_dont_move=true",
        ]),
        pactl(&[
            "module-null-sink",
            &format!("sink_name={MICMIX_SOURCE}"),
            "channel_map=front-left,front-right",
            // media.class is a node property, not a module argument (refused by PipeWire 1.6).
            "sink_properties='device.description=\"Mic + App Mix\" media.class=Audio/Source/Virtual'",
        ]),
        pactl(&[
            "module-loopback",
            &format!("source={APPMIX_SINK}.monitor"),
            &format!("sink={MICMIX_SOURCE}"),
            "latency_msec=20",
            "source_dont_move=true",
            "sink_dont_move=true",
        ]),
        pactl(&[
            "module-loopback",
            &format!("source={mic}"),
            &format!("sink={MICMIX_SOURCE}"),
            "latency_msec=20",
            "sink_dont_move=true",
        ]),
    ]
}

/// Module ids from `pactl list short modules` whose arguments mention one of `markers`
/// (node names), oldest first; `pactl unload-module <id>` takes them down.
pub fn modules_matching(list: &str, markers: &[&str]) -> Vec<u32> {
    list.lines()
        .filter_map(|l| {
            let mut f = l.splitn(3, '\t');
            let id: u32 = f.next()?.trim().parse().ok()?;
            let _name = f.next()?;
            let args = f.next().unwrap_or("");
            markers.iter().any(|m| args.contains(m)).then_some(id)
        })
        .collect()
}

/// `pw-cli set-param <device> Props { bluetoothAudioCodec: <codec> }`: switch a headset's codec.
pub fn bt_codec_argv(device_id: u32, codec: &str) -> Vec<String> {
    vec![
        "pw-cli".into(),
        "set-param".into(),
        device_id.to_string(),
        "Props".into(),
        format!("{{ bluetoothAudioCodec: \"{codec}\" }}"),
    ]
}

// ---- Reset ----

/// What `wol audio reset` does, in order; shown as the dry run, executed with `--yes`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ResetPlan {
    pub remove: Vec<PathBuf>,
    pub commands: Vec<Vec<String>>,
}

/// The reset for one account: WirePlumber's state under `state_home` (`$XDG_STATE_HOME`, and the
/// default place under `home` when that differs), WolOS's own files under `home`. None when `home`
/// or `state_home` is empty or relative (audit S4-22): nothing is ever removed relative to the
/// working directory.
pub fn reset_plan(home: &Path, state_home: &Path) -> Option<ResetPlan> {
    if !home.is_absolute() || !state_home.is_absolute() {
        return None;
    }
    let sv = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut remove = vec![state_home.join("wireplumber")];
    if home.join(WIREPLUMBER_STATE) != remove[0] {
        remove.push(home.join(WIREPLUMBER_STATE));
    }
    remove.extend([home.join(PRO_AUDIO_CONF), home.join(SNAPPED_FILE)]);
    Some(ResetPlan {
        remove,
        commands: vec![
            sv(&["wpctl", "clear-default"]),
            sv(&[
                "pw-metadata",
                "-n",
                "settings",
                "0",
                "clock.force-rate",
                "0",
            ]),
            sv(&[
                "pw-metadata",
                "-n",
                "settings",
                "0",
                "clock.force-quantum",
                "0",
            ]),
            sv(&[
                "systemctl",
                "--user",
                "restart",
                "pipewire.service",
                "pipewire-pulse.service",
                "wireplumber.service",
            ]),
        ],
    })
}

impl ResetPlan {
    /// One line per step, for the dry run.
    pub fn describe(&self) -> String {
        let mut s = String::new();
        for p in &self.remove {
            s.push_str(&format!("remove {}\n", p.display()));
        }
        for c in &self.commands {
            s.push_str(&format!("run {}\n", c.join(" ")));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mic(volume: f32, base: f32) -> Device {
        Device {
            id: 42,
            name: "Built-in Audio Analog Stereo".into(),
            kind: DeviceKind::Input,
            volume,
            volume_base: base,
            ..Default::default()
        }
    }

    #[test]
    fn a_pro_audio_file_is_rewritten_line_by_line_and_nothing_else_is_lost() {
        let old = "# mine\nfoo.bar = 1\ncontext.properties = {\n    default.clock.rate=44100\n    \
                   default.clock.allowed-rates = [ 44100 ]\n    # keep me\n    default.clock.quantum=256\n    \
                   default.clock.min-quantum = 256\n    default.clock.max-quantum = 256\n    log.level = 2\n}\nlast = 1\n";
        assert_eq!(
            parse_pro_audio_conf(old),
            ProAudio {
                rate: 44100,
                quantum: 256,
                xruns: 0
            },
            "key=value without spaces"
        );
        let new = rewrite_pro_audio_conf(old, 44100, LOW_LATENCY_QUANTUM);
        for kept in [
            "# mine",
            "foo.bar = 1",
            "    # keep me",
            "    log.level = 2",
            "last = 1",
        ] {
            assert!(new.lines().any(|l| l == kept), "{kept} lost:\n{new}");
        }
        assert!(
            new.contains("default.clock.quantum = 128")
                && !new.contains("= 256")
                && !new.contains("=256"),
            "{new}"
        );
        assert!(new.contains("default.clock.rate = 44100"), "{new}");
        // The quantum lines stay where they were, between the comment and log.level.
        let at = |s: &str| new.lines().position(|l| l.contains(s)).unwrap();
        assert!(
            at("# keep me") < at("default.clock.quantum")
                && at("default.clock.quantum") < at("log.level")
        );
        assert!(
            new.contains("cpu.vm.name"),
            "below 1024 the VM block comes with it"
        );
        // Back to the default keeps what was not ours, and the file stays.
        let back = rewrite_pro_audio_conf(&new, 44100, 0);
        assert!(
            !back.contains("quantum") && !back.contains("cpu.vm.name"),
            "{back}"
        );
        assert!(back.contains("log.level = 2") && back.contains("default.clock.rate = 44100"));
    }

    #[test]
    fn the_file_goes_only_when_nothing_but_rate_and_quantum_was_in_it() {
        assert_eq!(rewrite_pro_audio_conf(&pro_audio_conf(0, 128), 0, 0), "");
        assert_eq!(
            rewrite_pro_audio_conf(&pro_audio_conf(44100, 128), 0, 0),
            ""
        );
        assert_ne!(
            rewrite_pro_audio_conf(&pro_audio_conf(44100, 128), 44100, 0),
            ""
        );
        // Another key, or a comment of the person's, keeps the file.
        let own = format!("{}# note\n", pro_audio_conf(0, 128));
        assert!(rewrite_pro_audio_conf(&own, 0, 0).contains("# note"));
        let other =
            "context.properties = {\n    default.clock.quantum = 64\n    link.max-buffers = 8\n}\n";
        let new = rewrite_pro_audio_conf(other, 0, 0);
        assert!(
            new.contains("link.max-buffers = 8") && !new.contains("quantum"),
            "{new}"
        );
        // A file with no block gets one; an empty one is the writer's own output.
        assert!(rewrite_pro_audio_conf("# x\n", 0, 128).contains("context.properties = {"));
        assert_eq!(rewrite_pro_audio_conf("", 0, 0), "");
    }

    #[test]
    fn the_low_latency_profile_is_a_pro_audio_buffer_that_reads_back() {
        let conf = pro_audio_conf(0, LOW_LATENCY_QUANTUM);
        assert!(conf.contains("default.clock.quantum = 128"), "{conf}");
        assert!(
            !conf.contains("default.clock.rate"),
            "the rate stays automatic: {conf}"
        );
        let back = parse_pro_audio_conf(&conf);
        assert_eq!((back.rate, back.quantum), (0, 128));
        assert_eq!(profile_name(back.quantum), "low-latency");
        assert_eq!(profile_name(0), "default");
        assert_eq!(profile_name(256), "custom");
        assert!(
            QUANTA.contains(&LOW_LATENCY_QUANTUM),
            "Pro Audio offers it too"
        );
    }

    #[test]
    fn hundred_percent_is_zero_db_not_hardware_max() {
        // +30 dB of Mic Boost: 0 dB sits at 0.0316 linear.
        let base = 10f32.powf(-30.0 / 20.0);
        assert_eq!(percent_of(base, base, false), 100);
        assert_eq!(percent_of(1.0, base, false), 100, "capped without boost");
        assert_eq!(percent_of(1.0, base, true), max_percent(base));
        assert_eq!(max_percent(base), 316);
        assert_eq!(max_percent(1.0), 100);
        assert!((linear_for(100, base) - base).abs() < 1e-6);
        assert_eq!(linear_for(100, 1.0), 1.0);
        assert_eq!(linear_for(0, base), 0.0);
        assert!((db_of(base, base)).abs() < 1e-4);
        assert!((db_of(1.0, base) - 30.0).abs() < 1e-3);
        assert_eq!(db_of(0.0, base), f32::NEG_INFINITY);
    }

    /// A6-2: the same position means the same sound on both scales; only a base below 1.0 makes
    /// the numbers differ, and 100 stays 0 dB, never the hardware maximum.
    #[test]
    fn hardware_percent_counts_from_the_hardware_maximum() {
        assert_eq!(hardware_percent(100, 1.0), 100);
        assert_eq!(hardware_percent(40, 1.0), 40);
        let base = 0.125; // 0 dB is half of the maximum on the cubic scale
        assert_eq!(hardware_percent(100, base), 50, "0 dB is not the maximum");
        assert_eq!(hardware_percent(0, base), 0);
        assert_eq!(
            hardware_percent(max_percent(base), base),
            100,
            "louder tops out"
        );
    }

    #[test]
    fn cubic_slider_round_trips() {
        for p in [0u16, 10, 37, 50, 73, 100] {
            for base in [1.0f32, 0.5, 0.0316] {
                assert_eq!(
                    percent_of(linear_for(p, base), base, false),
                    p,
                    "{p}% @ {base}"
                );
            }
        }
        // WirePlumber's 0.064 default is 40% on a no-boost device.
        assert_eq!(percent_of(0.064, 1.0, false), 40);
        // A bad base never divides by zero.
        assert_eq!(percent_of(0.5, 0.0, false), percent_of(0.5, 1.0, false));
    }

    #[test]
    fn a_new_microphone_above_zero_db_snaps_once() {
        let base = 0.0316;
        assert_eq!(snap_to_base(&mic(1.0, base), &[]), Some(base));
        assert_eq!(
            snap_to_base(&mic(0.02, base), &[]),
            None,
            "already below 0 dB"
        );
        assert_eq!(snap_to_base(&mic(base, base), &[]), None, "at 0 dB");
        let seen = vec!["Built-in Audio Analog Stereo".to_string()];
        assert_eq!(
            snap_to_base(&mic(1.0, base), &seen),
            None,
            "the user's choice stays"
        );
        assert_eq!(snap_to_base(&mic(1.0, 1.0), &[]), None, "no boost range");
        let mut out = mic(1.0, base);
        out.kind = DeviceKind::Output;
        assert_eq!(
            snap_to_base(&out, &[]),
            None,
            "outputs keep WirePlumber's 40%"
        );
    }

    #[test]
    fn meter_holds_the_peak_and_decays_twenty_db_per_second() {
        let mut m = InputMeter::default();
        m.feed(0.5, 0.0);
        assert_eq!((m.peak, m.hold, m.clipping), (0.5, 0.5, false));
        m.feed(0.01, 1.0);
        assert!((m.hold - 0.05).abs() < 1e-4, "-20 dB after 1 s: {}", m.hold);
        assert_eq!(m.peak, 0.01);
        m.feed(0.99, 0.01);
        assert!(m.clipping && m.hold == 0.99);
        assert_eq!(decay(0.8, 0.0), 0.8);
        assert_eq!(decay(0.8, -1.0), 0.8);
        m.feed(2.0, 0.0);
        assert_eq!(m.peak, 1.0, "clamped");
    }

    /// The clock keys of one block of a fragment: `context.properties = {..}` (props) or the
    /// update-props of `context.properties.rules` (VM-only), as PipeWire would read them.
    fn clock_keys(text: &str, rules: bool) -> Vec<(String, u32)> {
        let mut inside = false;
        let mut out = vec![];
        for l in text.lines().map(str::trim) {
            if l.starts_with("context.properties") {
                inside = (l == PRO_AUDIO_VM_RULES) == rules;
                continue;
            }
            if let Some((k, v)) = l.split_once(" = ")
                && inside
                && k.starts_with("default.clock.")
                && k.ends_with("quantum")
            {
                out.push((k.to_string(), v.parse().unwrap()));
            }
        }
        out
    }

    /// (quantum, min, max) after a reboot: the shipped fragment (1024 / 64 / 2048), PipeWire's
    /// VM rule (min 1024) when `vm`, and the Pro Audio file, with the VM rules in either order.
    fn boot_clock(user: &str, vm: bool, upstream_rule_last: bool) -> (u32, u32, u32) {
        let mut m: std::collections::HashMap<String, u32> = [
            ("default.clock.quantum", 1024),
            ("default.clock.min-quantum", 64),
            ("default.clock.max-quantum", 2048),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        m.extend(clock_keys(user, false));
        if vm {
            let upstream = vec![("default.clock.min-quantum".to_string(), 1024)];
            let ours = clock_keys(user, true);
            let order = if upstream_rule_last {
                [ours, upstream]
            } else {
                [upstream, ours]
            };
            for rule in order {
                m.extend(rule);
            }
        }
        let g = |k: &str| m[&format!("default.clock.{k}")];
        (g("quantum"), g("min-quantum"), g("max-quantum"))
    }

    #[test]
    fn a_saved_quantum_never_leaves_min_above_max_on_hardware_or_in_a_vm() {
        for n in QUANTA.iter().copied().chain([2048]) {
            let t = pro_audio_conf(0, n);
            assert_eq!(
                boot_clock(&t, false, false),
                (n, n, n),
                "hardware, quantum {n}\n{t}"
            );
            let vm = n.max(VM_MIN_QUANTUM);
            for last in [false, true] {
                let (q, min, max) = boot_clock(&t, true, last);
                assert!(
                    min <= q && q <= max,
                    "VM, quantum {n}: {min}/{q}/{max}\n{t}"
                );
                let want = if n < VM_MIN_QUANTUM {
                    (vm, vm, vm)
                } else {
                    (n, VM_MIN_QUANTUM.min(n), n)
                };
                assert_eq!(
                    (q, min, max),
                    want,
                    "VM, quantum {n}, upstream rule last {last}\n{t}"
                );
            }
            assert_eq!(
                t.contains("matches = [ { cpu.vm.name = !null } ]"),
                n < VM_MIN_QUANTUM,
                "{t}"
            );
            assert_eq!(parse_pro_audio_conf(&t).quantum, n);
        }
        // Rate only: no rule, the shipped floor (and the VM's 1024) stay in charge.
        let r = pro_audio_conf(96000, 0);
        assert!(!r.contains("context.properties.rules"), "{r}");
        assert_eq!(boot_clock(&r, true, true), (1024, 1024, 2048));
        assert_eq!(boot_clock(&r, false, true), (1024, 64, 2048));
    }

    #[test]
    fn pro_audio_file_round_trips_and_is_empty_when_nothing_is_forced() {
        assert_eq!(pro_audio_conf(0, 0), "");
        let t = pro_audio_conf(96000, 128);
        for want in [
            "default.clock.rate = 96000",
            "default.clock.allowed-rates = [ 96000 ]",
            "default.clock.quantum = 128",
            "default.clock.min-quantum = 128",
            "default.clock.max-quantum = 128",
        ] {
            assert!(t.lines().any(|l| l.trim() == want), "{want}\n{t}");
        }
        assert_eq!(
            parse_pro_audio_conf(&t),
            ProAudio {
                rate: 96000,
                quantum: 128,
                xruns: 0
            }
        );
        let only_q = pro_audio_conf(0, 256);
        assert!(!only_q.contains("clock.rate"));
        assert_eq!(parse_pro_audio_conf(&only_q).quantum, 256);
        assert_eq!(quantum_ms(256, 48000), 256.0 * 1000.0 / 48000.0);
        assert_eq!(quantum_ms(256, 0), 0.0);
        assert!(QUANTA.contains(&64) && ALLOWED_RATES.contains(&96000));
        assert_eq!(
            force_argv("clock.force-quantum", 256).join(" "),
            "pw-metadata -n settings 0 clock.force-quantum 256"
        );
    }

    #[test]
    fn settings_metadata_and_pw_top_parse() {
        let md = "Found \"settings\" metadata 31\n\
                  update: id:0 key:'log.level' value:'2' type:''\n\
                  update: id:0 key:'clock.force-quantum' value:'256' type:''\n\
                  update: id:0 key:'clock.force-rate' value:'0' type:''\n";
        assert_eq!(parse_settings_metadata(md), (0, 256));
        assert_eq!(parse_settings_metadata(""), (0, 0));
        let top = "S   ID  QUANT   RATE    WAIT    BUSY   W/Q   B/Q  ERR FORMAT           NAME\n\
                   S   31      0      0    ---     ---   ---   ---    0        Dummy-Driver\n\
                   R   45    256  48000  21.3us  40.1us  0.00  0.01    2    S32LE 2 48000 alsa_output.pci-0000_00_1f.3.analog-stereo\n\
                   R   64    256  48000   5.2us  11.0us  0.00  0.00    1    F32LE 2 48000  + pw-cat\n";
        assert_eq!(parse_pw_top_xruns(top), 3);
        assert_eq!(parse_pw_top_xruns("garbage\n"), 0);
    }

    #[test]
    fn pw_dump_lists_sinks_and_sources_with_volume_base_and_defaults() {
        let dump = r#"[
          { "id": 31, "type": "PipeWire:Interface:Metadata", "metadata": [
              { "subject": 0, "key": "default.audio.sink", "type": "Spa:String:JSON", "value": { "name": "alsa_output.hdmi" } },
              { "subject": 0, "key": "default.audio.source", "type": "Spa:String:JSON", "value": { "name": "alsa_input.mic" } } ] },
          { "id": 45, "type": "PipeWire:Interface:Node", "info": {
              "props": { "node.name": "alsa_output.hdmi", "node.description": "HDMI Audio", "media.class": "Audio/Sink", "device.profile.name": "output:hdmi-stereo" },
              "params": { "Props": [ { "volume": 1.0, "mute": false, "channelVolumes": [ 0.064, 0.064 ], "volumeBase": 1.0 } ] } } },
          { "id": 46, "type": "PipeWire:Interface:Node", "info": {
              "props": { "node.name": "alsa_input.mic", "node.nick": "Mic", "media.class": "Audio/Source" },
              "params": { "Props": [ { "volume": 1.0, "mute": true, "channelVolumes": [ 1.0 ], "volumeBase": 0.0316 } ] } } },
          { "id": 47, "type": "PipeWire:Interface:Node", "info": {
              "props": { "node.name": "bluez_output.AA", "node.description": "WH-1000XM4", "media.class": "Audio/Sink", "api.bluez5.codec": "ldac", "device.profile.name": "a2dp-sink" },
              "params": { "Props": [ { "volume": 0.3, "mute": false, "volumeBase": 1.0 } ] } } },
          { "id": 50, "type": "PipeWire:Interface:Node", "info": { "props": { "node.name": "pw-cat", "media.class": "Stream/Output/Audio" } } },
          { "id": 51, "type": "PipeWire:Interface:Port" }
        ]"#;
        let d = parse_pw_dump(dump);
        assert_eq!(d.len(), 3);
        assert_eq!(
            (d[0].id, d[0].kind, d[0].default, d[0].name.as_str()),
            (45, DeviceKind::Output, true, "HDMI Audio")
        );
        assert!(
            (d[0].volume - 0.064).abs() < 1e-6
                && d[0].profile.as_deref() == Some("output:hdmi-stereo")
        );
        assert_eq!(
            (d[1].kind, d[1].default, d[1].muted, d[1].name.as_str()),
            (DeviceKind::Input, true, true, "Mic")
        );
        assert!((d[1].volume_base - 0.0316).abs() < 1e-6);
        assert_eq!(
            snap_to_base(&d[1], &[]),
            Some(d[1].volume_base),
            "the audit's mic at 100%"
        );
        assert_eq!((d[2].codec.as_deref(), d[2].default), (Some("ldac"), false));
        assert!(
            (d[2].volume - 0.3).abs() < 1e-6,
            "volume falls back to `volume`"
        );
        assert!(parse_pw_dump("not json").is_empty());
        assert!(parse_pw_dump("{}").is_empty());
    }

    #[test]
    fn on_demand_modules_go_through_pipewire_pulse_and_are_findable() {
        // R2C-13: a node name that would split into extra module arguments is refused.
        for bad in ["a b", "x'y", "m\"q", "", "n;rm", "a\nb"] {
            assert!(echo_cancel_argv(bad).is_err(), "{bad:?}");
            assert!(rnnoise_argv(bad).is_err(), "{bad:?}");
        }
        assert!(echo_cancel_argv("bluez_input.00:11:22:33:44:55").is_ok());
        assert!(echo_cancel_argv("alsa_input.pci-0000_00_1f.3.analog-stereo").is_ok());
        let ec = echo_cancel_argv("alsa_input.mic").unwrap();
        assert_eq!(&ec[..3], &["pactl", "load-module", "module-echo-cancel"]);
        assert!(ec.iter().any(|a| a == "source_master=alsa_input.mic"));
        assert!(
            ec.iter()
                .any(|a| a.starts_with("aec_args=") && a.contains("webrtc.gain_control=true"))
        );
        assert!(ec.iter().any(|a| a == &format!("source_name={AGC_SOURCE}")));
        let rn = rnnoise_argv("alsa_input.mic").unwrap();
        assert_eq!(rn[2], "module-ladspa-source");
        assert!(
            rn.iter().any(|a| a == "plugin=librnnoise_ladspa"),
            "LADSPA name, found in /usr/lib/ladspa"
        );
        assert!(rn.iter().any(|a| a == "label=noise_suppressor_mono"));
        let mix = appmix_argv("alsa_input.mic");
        assert_eq!(mix.len(), 5);
        assert!(
            mix[0]
                .iter()
                .any(|a| a == &format!("sink_name={APPMIX_SINK}"))
        );
        assert!(
            mix[2].iter().any(|a| a.starts_with("sink_properties=")
                && a.contains("media.class=Audio/Source/Virtual"))
        );
        assert!(mix[4].iter().any(|a| a == "source=alsa_input.mic"));
        let list = "536870912\tmodule-null-sink\tsink_name=wolos_appmix sink_properties=...\n\
                    536870913\tmodule-loopback\tsource=wolos_appmix.monitor sink=@DEFAULT_SINK@\n\
                    536870914\tmodule-echo-cancel\tsource_name=wolos_mic_agc source_master=x\n\
                    536870915\tmodule-null-sink\tsink_name=other\n";
        assert_eq!(
            modules_matching(list, &[APPMIX_SINK]),
            vec![536870912, 536870913]
        );
        assert_eq!(
            modules_matching(list, &[AGC_SOURCE, RNNOISE_SOURCE]),
            vec![536870914]
        );
        assert!(modules_matching("", &[APPMIX_SINK]).is_empty());
        assert_eq!(
            bt_codec_argv(60, "ldac").join(" "),
            "pw-cli set-param 60 Props { bluetoothAudioCodec: \"ldac\" }"
        );
    }

    /// pipewire-pulse's `module_args_add_props` (src/modules/module-protocol-pulse/module.c):
    /// `key=value` pairs split on spaces, a value may be quoted with `"` or `'`.
    fn pulse_args(s: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut rest = s;
        loop {
            rest = rest.trim_start();
            let Some(eq) = rest.find('=') else { break };
            let key = rest[..eq].to_string();
            rest = &rest[eq + 1..];
            let end = match rest.chars().next() {
                Some(q @ ('"' | '\'')) => {
                    rest = &rest[1..];
                    q
                }
                _ => ' ',
            };
            let stop = rest.find(end).unwrap_or(rest.len());
            out.push((key, rest[..stop].to_string()));
            rest = rest.get(stop + 1..).unwrap_or("");
        }
        out
    }

    #[test]
    fn module_arguments_survive_pactl_joining_them_with_spaces() {
        // pactl sends `argv[3..].join(" ")`; PipeWire 1.6 refuses any key not in the module's
        // valid_args (pulse-module-*.c), so every argument must come back whole.
        let check = |argv: &[String], valid: &[&str]| -> Vec<(String, String)> {
            let args = pulse_args(&argv[3..].join(" "));
            for (k, _) in &args {
                assert!(
                    valid.contains(&k.as_str()),
                    "{k} is not a module argument: {argv:?}"
                );
            }
            assert_eq!(args.len(), argv.len() - 3, "{argv:?} -> {args:?}");
            args
        };
        let ec = check(
            &echo_cancel_argv("alsa_input.mic").unwrap(),
            &[
                "source_name",
                "source_properties",
                "source_master",
                "sink_name",
                "sink_properties",
                "sink_master",
                "rate",
                "channels",
                "channel_map",
                "aec_method",
                "aec_args",
            ],
        );
        let aec = ec
            .iter()
            .find(|a| a.0 == "aec_args")
            .map(|a| pulse_args(&a.1))
            .unwrap();
        assert!(
            aec.contains(&("webrtc.gain_control".into(), "true".into())),
            "{aec:?}"
        );
        assert!(aec.contains(&("webrtc.noise_suppression".into(), "true".into())));
        let src = ec
            .iter()
            .find(|a| a.0 == "source_properties")
            .map(|a| pulse_args(&a.1))
            .unwrap();
        assert_eq!(
            src,
            [(
                "device.description".to_string(),
                "WolOS Microphone".to_string()
            )]
        );
        let sink = ec
            .iter()
            .find(|a| a.0 == "sink_properties")
            .map(|a| pulse_args(&a.1))
            .unwrap();
        assert_eq!(sink.len(), 2, "{sink:?}");
        check(
            &rnnoise_argv("alsa_input.mic").unwrap(),
            &[
                "source_name",
                "source_properties",
                "source_output_properties",
                "master",
                "source_master",
                "format",
                "rate",
                "channels",
                "channel_map",
                "plugin",
                "label",
                "control",
                "input_ladspaport_map",
                "output_ladspaport_map",
            ],
        );
        for argv in appmix_argv("alsa_input.mic") {
            let valid: &[&str] = if argv[2] == "module-null-sink" {
                &[
                    "sink_name",
                    "sink_properties",
                    "format",
                    "rate",
                    "channels",
                    "channel_map",
                ]
            } else {
                &[
                    "source",
                    "sink",
                    "latency_msec",
                    "rate",
                    "channels",
                    "channel_map",
                    "sink_input_properties",
                    "source_output_properties",
                    "source_dont_move",
                    "sink_dont_move",
                    "remix",
                ]
            };
            check(&argv, valid);
        }
    }

    /// Audit S4-22: never a relative path; `$XDG_STATE_HOME` is where WirePlumber's state is.
    #[test]
    fn reset_is_absolute_and_follows_the_state_home() {
        assert_eq!(reset_plan(Path::new(""), Path::new("/s")), None);
        assert_eq!(reset_plan(Path::new("home"), Path::new("/s")), None);
        assert_eq!(
            reset_plan(Path::new("/home/x"), Path::new(".local/state")),
            None
        );
        let p = reset_plan(Path::new("/home/x"), Path::new("/var/state/x")).unwrap();
        assert_eq!(p.remove[0], Path::new("/var/state/x/wireplumber"));
        assert_eq!(p.remove[1], Path::new("/home/x/.local/state/wireplumber"));
        assert!(p.remove.iter().all(|r| r.is_absolute()));
    }

    #[test]
    fn reset_forgets_state_and_restarts_the_three_services() {
        let p = reset_plan(Path::new("/home/x"), Path::new("/home/x/.local/state")).unwrap();
        assert_eq!(p.remove[0], Path::new("/home/x/.local/state/wireplumber"));
        assert_eq!(p.remove.len(), 3);
        assert_eq!(
            p.remove[1],
            Path::new("/home/x/.config/pipewire/pipewire.conf.d/50-wolos-pro-audio.conf")
        );
        assert_eq!(p.commands[0].join(" "), "wpctl clear-default");
        let last = p.commands.last().unwrap().join(" ");
        assert!(
            last.starts_with("systemctl --user restart pipewire.service"),
            "{last}"
        );
        assert!(last.contains("wireplumber.service"));
        let d = p.describe();
        assert!(
            d.contains("remove /home/x/.local/state/wireplumber\n")
                && d.contains("run wpctl clear-default\n")
        );
    }

    #[test]
    fn actions_have_a_stable_wire_form() {
        let a = Action::ProAudio {
            rate: 48000,
            quantum: 128,
        };
        let j = serde_json::to_string(&a).unwrap();
        assert_eq!(
            j,
            r#"{"action":"pro-audio","value":{"rate":48000,"quantum":128}}"#
        );
        assert_eq!(serde_json::from_str::<Action>(&j).unwrap(), a);
        let j = serde_json::to_string(&Action::MicVolume(80)).unwrap();
        assert_eq!(j, r#"{"action":"mic-volume","value":80}"#);
        let j = serde_json::to_string(&Action::OutputVolume(80)).unwrap();
        assert_eq!(j, r#"{"action":"output-volume","value":80}"#);
        let j = serde_json::to_string(&Action::BtCodec("AA:BB".into(), "aac".into())).unwrap();
        assert_eq!(j, r#"{"action":"bt-codec","value":["AA:BB","aac"]}"#);
        assert_eq!(
            serde_json::to_string(&Action::ResetAudio).unwrap(),
            r#"{"action":"reset-audio"}"#
        );
        let d = serde_json::to_value(mic(0.5, 1.0)).unwrap();
        assert_eq!(d["kind"], "input");
    }
}
