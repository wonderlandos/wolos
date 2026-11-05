// SPDX-License-Identifier: GPL-2.0-or-later
//! atlas-bridge (ADR 0043 D4, docs/modules/atlas.md): the one program of the Atlas stack that asks
//! wold for something. The browser's host component connects to `$XDG_RUNTIME_DIR/atlas/bridge.sock`
//! and speaks the Atlas Host Protocol (`shared::atlas`); the bridge checks that the peer is the
//! browser, then hands Share, Open With and Move to Trash to wold as the portal calls of libwol 1.15
//! (`share.file`, `open.with`, `file.trash`). wold names the bridge by its executable, so the
//! person's grants `share` and `trash` for it are what let Atlas do these things.
//!
//! Threads, not an event loop: every wait is a blocking read or accept with no timer, so the bridge
//! never wakes while the browser holds an idle connection, and it exits after [`server::Config`]'s
//! idle time with no connection and nothing waiting for the person.
#![deny(unsafe_code)]

pub mod activation;
pub mod peer;
pub mod server;
pub mod verbs;
pub mod wold;
