// SPDX-License-Identifier: GPL-2.0-or-later
//! Atlas Drive (ADR 0063, docs/modules/atlas-drive.md): browser automation for Atlas, built for AI
//! agents. `atlas-drive serve` launches Atlas over `--remote-debugging-pipe` (never a TCP port) and
//! answers on `$XDG_RUNTIME_DIR/atlas/drive.sock`: raw CDP pass-through plus the `Drive.*` verbs,
//! whose results are short, deterministic and carry a token estimate. Threads, no async runtime.
#![forbid(unsafe_code)]

pub use serde_json;

pub mod agent;
pub mod batch;
pub mod browser;
pub mod cdp;
pub mod cli;
pub mod client;
pub mod cursor;
pub mod daemon;
pub mod investigate;
pub mod keys;
pub mod mcp;
pub mod mirror;
pub mod page;
pub mod pagejs;
pub mod peer;
pub mod perms;
pub mod record;
pub mod route;
pub mod server;
pub mod sourcemap;
pub mod spec;
pub mod subset;
pub mod util;
pub mod verbs;

/// The body of the `atlas_drive_wire` fuzz target (fuzz/fuzz_targets), here so the seeds can be replayed
/// as a plain test (tests/fuzz_seeds.rs): framers never exceed their limit, a batch plan, the grants
/// file, the browser's accessibility tree (rendering is deterministic) and a source map never panic.
pub fn fuzz_one(data: &[u8]) {
    use std::collections::HashMap;
    for delim in [0u8, b'\n'] {
        let mut f = cdp::Framer::new(data, delim, 4096);
        while let Ok(Some(m)) = f.next_msg() {
            assert!(m.len() <= 4096 && !m.contains(&delim));
        }
    }
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
        let _ = batch::plan(&v);
        let _ = perms::Grants::from_json(&v);
        if let Some(nodes) = v.get("nodes").and_then(|n| n.as_array()) {
            let o = agent::Options::default();
            let render = || {
                let mut st = agent::AgentState::default();
                agent::render(
                    agent::lines_of(nodes, &o),
                    &mut st,
                    "https://a.test/",
                    "t",
                    &o,
                    &HashMap::new(),
                    1,
                )
                .text
            };
            assert_eq!(render(), render());
        }
    }
    let _ = sourcemap::parse(text);
    let mut end = text.len().min(4096);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let _ = sourcemap::pretty(&text[..end]);
}
