// SPDX-License-Identifier: BSD-2-Clause
//! Accessibility (plan section 2.7), owned by WK-W4: the tree every window has ([`tree`]), the
//! vocabulary of the desktop accessibility bus ([`roles`]).

pub mod roles;
pub mod tree;

pub use tree::{A11yNode, Context, Tree, build, dump, lint};
