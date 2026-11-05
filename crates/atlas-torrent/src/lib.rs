// SPDX-License-Identifier: BSD-2-Clause
//! The Atlas browser's torrent helper (ADR 0064; ADR 0043 D4: no Rust in the browser). The
//! browser's torrent provider (chrome/browser/atlas/dl in the fork) starts it with
//! `atlas-torrent launch` (a confined user service, [`sandbox`]) and keeps one connection to
//! `$XDG_RUNTIME_DIR/atlas/torrent.sock` ([`server`]); each job becomes a row of the browser's
//! download list. Protocol: [`proto`]. Checks before the engine: [`magnet`], [`metainfo`],
//! [`confine`]. The engine: [`engine`].
#![forbid(unsafe_code)]

pub mod confine;
pub mod confined;
pub mod engine;
pub mod magnet;
pub mod metainfo;
pub mod proto;
pub mod sandbox;
pub mod server;
pub mod trackers;
