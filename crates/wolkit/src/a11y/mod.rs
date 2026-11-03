// SPDX-License-Identifier: BSD-2-Clause
//! Accessibility (plan section 2.7), owned by WK-W4: the tree every window has ([`tree`]), the
//! vocabulary of the desktop accessibility bus ([`roles`]), and the export over that bus while an
//! assistive technology is attached ([`hub`] shared with the engine, [`bridge`] on the bus).

pub mod bridge;
pub mod hub;
pub mod roles;
pub mod tree;

pub use tree::{A11yNode, Context, Tree, build, dump, lint};

/// Whether this process exports its accessibility trees now (`WOLKIT_INFO_A11Y_ACTIVE`).
pub fn active() -> bool {
    hub::current().is_some_and(|h| h.active())
}
