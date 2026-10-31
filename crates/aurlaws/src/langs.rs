// SPDX-License-Identifier: GPL-2.0-or-later
//! Other languages' build scripts, read into the same graph.
use crate::graph::{Graph, NodeId, Stage};

/// Code given on an interpreter's command line, such as `python -c`.
pub fn inline(_g: &mut Graph, _interp: &str, _code: &str, _file: &str, _line: u32, _stage: Stage, _proc: NodeId) {}
