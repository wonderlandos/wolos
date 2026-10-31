// SPDX-License-Identifier: BSD-2-Clause
//! Install WolOS: what is on the disk, what the person chose, and the steps between (ADR 0023).
//! Pure: `crates/controller` runs the steps, `crates/shell` asks the questions.
pub mod disk;
pub mod layout;
pub mod plan;
