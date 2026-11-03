// SPDX-License-Identifier: BSD-2-Clause
//! The WolOS web-app runtime's engine adaptor (ADR 0042, track 2): WolOS's own web apps and the
//! site-as-app entries run on the Rust browser engine inside a Wolkit window, one app per process.
//!
//! Concept. A web app is a [`WebView`] made from a [`Policy`] (what the app may reach and use) and an
//! [`App`] (its id and profile directory), painting either into a wolkit `surface` node (a Wayland
//! subsurface of the app's window) or headless into memory (tests, goldens, probes).
//!
//! Why this shape. The engine is large and its embedding API still moves, so it is pinned by commit
//! and built once into `libwolwebengine.so.1` (the `webengine-wolos` package, source in `engine/`
//! and `src/engine/`), behind a small C boundary that this crate loads at run time ([`ffi`]). Every
//! decision the system owns stays here in plain Rust and is tested without the engine: the origin
//! gate ([`sandbox`]), the policy ([`policy`]), the process model ([`process`]) and the input bridge
//! from wolkit's event records ([`input`]).
//!
//! How. `WebView::new` loads the library (once per process), makes the engine with the app's
//! profile directory as its only place for files and the policy's features as its preferences, and
//! installs the gate: every navigation and resource load is asked of [`sandbox::Gate::allows`], a
//! refused load is a network error and a refused navigation is kept for the host to offer in the
//! system browser ([`WebView::take_refused`]). The host process runs in its own systemd scope named
//! like the Memory Saver's (`process::scope_unit`), so the saver and the frontmost QoS see the app
//! and its content process as one.
pub mod ffi;
pub mod input;
pub mod policy;
pub mod process;
pub mod sandbox;
mod view;

pub use policy::{Policy, TelemetryOff};
pub use view::{App, Error, Image, Target, WebView};
