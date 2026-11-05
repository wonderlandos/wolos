// SPDX-License-Identifier: GPL-2.0-or-later
//! Atlas Drive's decoders of outside input (ADR 0063): the framer of the browser pipe and of the
//! client socket (a message never exceeds its limit), a batch plan, the accessibility tree the
//! browser sends (rendering never panics and is deterministic), a source map and the grants file.
//! The body is `atlas_drive::fuzz_one`, which tests/fuzz_seeds.rs also replays.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| atlas_drive::fuzz_one(data));
