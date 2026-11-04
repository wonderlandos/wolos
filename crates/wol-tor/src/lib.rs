// SPDX-License-Identifier: GPL-2.0-or-later
//! Tor groups for Atlas Tor tabs (ADR 0053): one tor and one Atlas instance per group, the
//! browser in a network namespace with only loopback, reaching tor through [`relay`] alone.
//!
//! Invariant: nothing in this crate opens a network connection other than to tor's Unix
//! sockets. The relay has no code path to the clear net, so the kernel (no route in the browser's
//! namespace, checked by [`netns`] before Atlas starts) is the kill switch, not a setting.
//!
//! - [`group`]: group names and the paths of a group under the runtime directory.
//! - [`host`]: the command socket of a running group (open a tab, new circuit).
//! - [`torrc`]: the hardened torrc, bridge lines checked before they reach tor.
//! - [`socks`]: the SOCKS5 wire format (RFC 1928, RFC 1929), both sides.
//! - [`isolation`]: the tor credential of a stream (per site, per group, renewable).
//! - [`relay`]: the SOCKS5 relay from the browser's loopback to tor's Unix socket.
//! - [`control`]: tor's control port (cookie auth, bootstrap phase, ownership).
//! - [`netns`]: the fail-closed check that the browser's namespace has no route.
//! - [`cache`]: the bootstrap cache that makes a group start warm.
//! - [`units`]: the systemd user units of a group, as argv.
//! - [`profile`]: the Tor instance's managed policies and launch flags (Tier 0).
//! - [`ready`]: the "Keep Tor ready" switch (default off) and the cache units it installs.
#![forbid(unsafe_code)]

pub mod cache;
pub mod control;
pub mod doctor;
pub mod group;
pub mod host;
pub mod isolation;
pub mod mode;
pub mod netns;
pub mod profile;
pub mod ready;
pub mod relay;
pub mod socks;
pub mod torrc;
pub mod transparent;
pub mod units;
