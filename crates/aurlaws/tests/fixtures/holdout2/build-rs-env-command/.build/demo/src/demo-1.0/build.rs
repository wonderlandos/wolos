// SPDX-License-Identifier: GPL-2.0-or-later
use std::process::Command;
fn main() { Command::new(std::env::var("X").unwrap()).status().ok(); }
