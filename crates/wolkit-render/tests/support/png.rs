// SPDX-License-Identifier: BSD-2-Clause
//! wolkit's golden PNG helper (`crates/wolkit/tests/golden/png.rs`), shared, not copied.

#[path = "../../../wolkit/tests/golden/png.rs"]
#[allow(dead_code)] // each test file uses part of the shared helper
mod inner;
pub use inner::*;
