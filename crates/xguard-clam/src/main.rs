// SPDX-License-Identifier: GPL-2.0-or-later
//! ClamAV in its own process (ADR 0028). This is the only program that links libclamav, which is
//! GPL-2.0-only; nothing else in WolOS links it, and libwol never touches it. Others connect to a
//! socket and pass an open file. Started on demand, it exits when idle, so at rest it costs nothing.
//!
//! `serve --db DIR [--socket PATH] [--idle SECS]`: answer `clean`, `found NAME` or `error WHY`.
//! Without `--socket` the listening socket comes from systemd.
//! `scan --db DIR PATH`: one load, one scan, and what it cost, for tests and measurements.
#[cfg(feature = "libclamav")]
mod engine;

#[cfg(not(feature = "libclamav"))]
fn main() {
    eprintln!("wol-xguard-clam: built without libclamav");
    std::process::exit(3);
}

#[cfg(feature = "libclamav")]
fn main() {
    std::process::exit(engine::main(&std::env::args().skip(1).collect::<Vec<_>>()));
}
