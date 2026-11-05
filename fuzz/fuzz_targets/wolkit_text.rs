// SPDX-License-Identifier: GPL-2.0-or-later
//! libWolkit's text buffer (`text::edit`, `text::area`) under libFuzzer. The input is a program of
//! editing, movement, undo and input-method operations on a one-line or multi-line editor; after
//! each step the buffer must be sound, the geometry of the laid-out text must answer with offsets
//! inside the text, and undoing everything must restore the empty text and the starting selection
//! (the property is in `crates/wolkit/tests/support/text_fuzz.rs`, which `tests/text_area.rs`
//! also runs over `fuzz/regressions/wolkit_text/` on stable).
#![no_main]
use libfuzzer_sys::fuzz_target;

#[path = "../../crates/wolkit/tests/support/text_fuzz.rs"]
mod text_fuzz;

fuzz_target!(|data: &[u8]| {
    text_fuzz::run(data);
});
