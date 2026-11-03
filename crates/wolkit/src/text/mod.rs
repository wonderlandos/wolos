// SPDX-License-Identifier: BSD-2-Clause
//! Text (plan section 2.5): font index and fallback, shaping, bidi and line breaking, the shaped
//! run and glyph caches, the glyph atlas and the one-line editor. Rust throughout: font files are
//! parsed by memory-safe code. `system` is the entry point the layout and paint stages use.

pub mod atlas;
pub mod cache;
pub mod edit;
pub mod fontdb;
pub mod glyph;
pub mod glyphcache;
pub mod layout;
pub mod measure;
pub mod shape;
pub mod system;
