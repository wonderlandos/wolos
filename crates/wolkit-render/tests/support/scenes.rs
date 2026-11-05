// SPDX-License-Identifier: BSD-2-Clause
//! wolkit's golden scenes (`crates/wolkit/tests/golden/scenes.rs`), shared, not copied. They name
//! `crate::text_support`, which the including test file provides.

#[path = "../../../wolkit/tests/golden/scenes.rs"]
#[allow(dead_code)] // each test file uses part of the shared scenes
mod inner;
pub use inner::*;
