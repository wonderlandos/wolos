// SPDX-License-Identifier: GPL-2.0-or-later
//! wol-audio: the sound service behind Settings > Sound, Control Center and the shell's volume
//! state (docs/modules/audio.md). It replaces the 2-second `wpctl`/`pactl` polling with one
//! PipeWire registry connection and turns the frozen `wol_core::audio` actions into PipeWire
//! calls. `wol audio` (the CLI) and libwol keep the `wpctl` path in `wol_core::controls`.
//!
//! Layers, pure first:
//! - [`model`]: what the backend reports ([`model::Event`]) and asks for ([`model::Command`]);
//!   the [`model::Snapshot`] the UI draws.
//! - [`service`]: the rules. New microphones snap once to 0 dB; sliders stop at 0 dB unless
//!   boost is allowed; echo-cancel/RNNoise/App Mix are pipewire-pulse modules loaded on demand
//!   and torn down when switched off; Pro Audio writes one user conf.d file; reset forgets
//!   per-user state and restarts the three services.
//! - [`fake`]: a recording backend plus a registry fixture, so every rule is tested without a
//!   sound card ("fake-registry test mode").
//! - [`pw`] (feature `pw`, default): the real backend on pipewire-rs 0.10: a PipeWire loop thread
//!   that owns the proxies and reconnects, and a service thread that runs programs
//!   ([`service::serve`]), talked to through [`pw::Handle`].
pub mod fake;
pub mod model;
#[cfg(feature = "pw")]
pub mod pw;
pub mod service;

pub use model::{Command, Event, Snapshot};
pub use service::{Backend, Service};
pub use wol_core::audio::{Action, Device, DeviceKind, InputMeter, ProAudio};
